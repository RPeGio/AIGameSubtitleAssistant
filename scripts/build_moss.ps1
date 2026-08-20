<#
build_moss.ps1 —— 从源码构建 moss-transcribe.exe（仅开发者/CI 使用，用户设备不需要）
产物拷贝到 runtime/bin/moss-transcribe.exe，bootstrap_moss.ps1 负责模型下载与配置。

前置工具链（缺失时脚本明确报错，不做自动安装）：
  - git（clone 源码 + ggml 子模块）
  - cmake（构建系统）
  - Visual Studio 2022 Build Tools（C++ 桌面开发，MSVC 编译器）
  - nvcc（仅 -Backend cuda 需要，CUDA Toolkit；Vulkan 需要 glslc + Vulkan SDK）

参数：
  -Force        强制重新构建（删除现有 runtime/bin/moss-transcribe.exe 后重建）
  -Backend      推理后端：cpu（默认）/ cuda / vulkan。GPU 后端由 ggml 支持
                （moss-transcribe.cpp 透传 GGML_CUDA / GGML_VULKAN，见官方 README），
                构建后 runtime/bin/ 会多出 ggml-cuda.dll / ggml-vulkan.dll，
                运行时 backend 自动选 GPU（MTD_DEVICE=cpu 可强制回退）。
  -GitMirror    源码镜像前缀，如 https://ghproxy.com/ 或 https://gitclone.com/github.com/（网络受限时使用）
用法：powershell -ExecutionPolicy Bypass -File scripts/build_moss.ps1 -Backend cuda
#>
param(
  [switch]$Force,
  [string]$Backend = "cpu",
  [string]$GitMirror = ""
)

$ErrorActionPreference = "Stop"

# 后端白名单校验，防参数注入
$Backend = $Backend.ToLower()
if ($Backend -notmatch "^(cpu|cuda|vulkan)$") {
  throw "无效的 -Backend: $Backend（可选 cpu/cuda/vulkan）"
}

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$binDir = Join-Path $runtime "bin"
$targetExe = Join-Path $binDir "moss-transcribe.exe"

# ── 1. 工具链检查 ──
$missing = @()
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { $missing += "git" }
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) { $missing += "cmake" }
if ($Backend -eq "cuda") {
  $nvcc = Get-Command nvcc -ErrorAction SilentlyContinue
  if (-not $nvcc) {
    # 兜底路径：CUDA_PATH 环境变量（安装器设置）> 标准安装目录 > 常见自定义目录
    $candidates = @()
    if ($env:CUDA_PATH) { $candidates += $env:CUDA_PATH }
    $candidates += "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA"
    $candidates += "D:\CUDA-Toolkit"
    $cudaRoot = $candidates |
      ForEach-Object { Get-ChildItem $_ -Directory -ErrorAction SilentlyContinue } |
      Sort-Object Name -Descending | Select-Object -First 1
    if (-not $cudaRoot) {
      $cudaRoot = $candidates | ForEach-Object {
        if (Test-Path (Join-Path $_ "bin\nvcc.exe")) { Get-Item $_ } else { $null }
      } | Select-Object -First 1
    }
    if ($cudaRoot -and (Test-Path (Join-Path $cudaRoot.FullName "bin\nvcc.exe"))) {
      Write-Host "==> 找到 nvcc: $(Join-Path $cudaRoot.FullName 'bin\nvcc.exe')"
      # 加入 PATH 供 cmake 的 CUDA 编译器探测使用
      $env:PATH = "$(Join-Path $cudaRoot.FullName 'bin');$env:PATH"
    } else {
      $missing += "nvcc（CUDA Toolkit 12+，含 CUBLAS/CUDART 开发组件）"
    }
  } else {
    Write-Host "==> 找到 nvcc（PATH）: $($nvcc.Source)"
  }
}
if ($missing.Count -gt 0) {
  throw "缺少工具链: $($missing -join '、')。请安装后重试（git: https://git-scm.com，cmake: https://cmake.org）"
}

# ── 2. 清理旧产物 ──
# GPU 后端产物与 CPU 版不兼容（DLL 布局不同），切后端时强制删旧 exe 防误用。
# .backend 标记缺失 = 旧构建（该标记是新版才写入）→ 一律视为未知后端，
# 若请求后端与现有 DLL 布局不符则重建（GPU 请求 + 无 ggml-cuda/vulkan.dll → 必须重建）
$stamp = Join-Path $binDir ".backend"
$old = if (Test-Path $stamp) { (Get-Content $stamp -Raw).Trim() } else { "" }
$needsRebuild = $false
if (Test-Path $targetExe) {
  if ($old -eq "") {
    # 无标记：CPU 请求 → 假定旧构建即 CPU，跳过；GPU 请求 → 无对应 DLL 证据，重建
    if ($Backend -eq "cuda" -and -not (Test-Path (Join-Path $binDir "ggml-cuda.dll"))) { $needsRebuild = $true }
    elseif ($Backend -eq "vulkan" -and -not (Test-Path (Join-Path $binDir "ggml-vulkan.dll"))) { $needsRebuild = $true }
  } elseif ($old -ne $Backend) {
    $needsRebuild = $true
  }
}
if ($needsRebuild) {
  $from = if ($old -eq "") { "未知（旧构建）" } else { $old }
  Write-Host "==> 检测到后端 $from → $Backend，删除旧二进制 $targetExe 重建"
  Remove-Item -Force $targetExe
  Remove-Item -Force $stamp -ErrorAction SilentlyContinue
}
if ((Test-Path $targetExe) -and $Force) {
  Write-Host "==> -Force：删除旧二进制 $targetExe"
  Remove-Item -Force $targetExe
}
if (Test-Path $targetExe) {
  Write-Host "==> $targetExe 已存在（backend=$Backend），跳过构建（-Force 重建）"
  exit 0
}

New-Item -ItemType Directory -Force -Path $binDir | Out-Null

# ── 3. clone + 构建 ──
$work = Join-Path $env:TEMP "gsa_moss_build_$PID"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force -Path $work | Out-Null

# 失败/被中断时保留工作目录，便于人工排查（成功才清理）
$keepWork = $false

try {
  $repoUrl = "https://github.com/mudler/moss-transcribe.cpp"
  if ($GitMirror) { $repoUrl = "$GitMirror$repoUrl" }

  Write-Host "==> 克隆源码（含 ggml 子模块，浅克隆）..."
  # --shallow-submodules：ggml 历史庞大，浅克隆子模块可显著减少下载量
  & git clone --recursive --depth 1 --shallow-submodules $repoUrl (Join-Path $work "src")
  if ($LASTEXITCODE -ne 0) {
    $keepWork = $true
    throw "git clone 失败（网络受限可试 -GitMirror 镜像参数），工作目录保留在 $work"
  }
  $ggmlDir = Join-Path $work "src\third_party\ggml"
  if (-not (Test-Path (Join-Path $ggmlDir "CMakeLists.txt"))) {
    $keepWork = $true
    throw "ggml 子模块缺失（$ggmlDir），工作目录保留在 $work"
  }

  # 后端 cmake flag：moss-transcribe.cpp 的 CMakeLists 把 MT_GGML_* 透传给 ggml
  $buildDir = "build"
  $backendFlag = @()
  $useNinja = $false
  switch ($Backend) {
    "cuda"   {
      $buildDir = "build-cuda"
      $backendFlag = @("-DMT_GGML_CUDA=ON", "-DCMAKE_CUDA_ARCHITECTURES=89")
      $useNinja = $true   # 自定义 CUDA 路径下 VS toolset 常缺失，Ninja 更稳
    }
    "vulkan" { $buildDir = "build-vulkan"; $backendFlag = @("-DMT_GGML_VULKAN=ON") }
  }

  Write-Host "==> cmake 配置（backend=$Backend）..."
  Push-Location (Join-Path $work "src")
  try {
    if ($useNinja) {
      # CUDA 后端：清 PATH 剔除 mingw/Anaconda 污染 → vcvars 导入 MSVC 工具 → cmake -G Ninja
      $vcvars = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
      if (Test-Path $vcvars) {
        $dump = cmd /c "set PATH=C:\WINDOWS\system32;C:\WINDOWS && call `"$vcvars`" >nul 2>&1 && set" 2>$null
        foreach ($line in $dump) {
          if ($line -match '^([^=]+)=(.*)$') {
            [System.Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
          }
        }
        $env:PATH = "D:\CUDA-Toolkit\bin;C:\Program Files\CMake\bin;$env:PATH"
        & cmake -B $buildDir -G Ninja -DCMAKE_BUILD_TYPE=Release -DMT_BUILD_TESTS=OFF @backendFlag
      } else {
        & cmake -B $buildDir -G Ninja -DCMAKE_BUILD_TYPE=Release -DMT_BUILD_TESTS=OFF @backendFlag
      }
    } else {
      & cmake -B $buildDir -DMT_BUILD_TESTS=OFF @backendFlag
    }
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "cmake 配置失败（需 Visual Studio 2022 Build Tools C++ 工作负载；CUDA 后端还需 CUDA Toolkit + Ninja），工作目录保留在 $work"
    }

    Write-Host "==> 编译（Release，CPU 版数分钟 / CUDA 版首次可能 10-30 分钟）..."
    & cmake --build $buildDir --config Release -j
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "cmake 构建失败，详见上方日志，工作目录保留在 $work"
    }
  } finally {
    Pop-Location
  }

  # MSVC 多配置生成器输出在 build/Release/，单配置（MinGW/Ninja）在 build/
  $built = Join-Path $work "src\$buildDir\Release\moss-transcribe.exe"
  if (-not (Test-Path $built)) { $built = Join-Path $work "src\$buildDir\moss-transcribe.exe" }
  if (-not (Test-Path $built)) { throw "构建完成但找不到 moss-transcribe.exe（$buildDir\Release\ 或 $buildDir\）" }

  Copy-Item $built $targetExe -Force
  # ggml 以共享库构建：DLL 在 build/bin/Release/（CMake RUNTIME_OUTPUT_DIRECTORY），
  # 单配置生成器可能无该布局，回退到 exe 同目录
  $exeDir = Split-Path $built
  $dllDir = Join-Path $exeDir "..\bin\Release"
  if (-not (Test-Path (Join-Path $dllDir "ggml.dll"))) { $dllDir = $exeDir }
  Get-ChildItem -Path $dllDir -Filter "*.dll" | Copy-Item -Destination $binDir -Force
  # 记录后端标记，供下次构建检测后端切换
  Set-Content -Path (Join-Path $binDir ".backend") -Value $Backend -NoNewline
  Write-Host "==> 构建成功：$targetExe（backend=$Backend，含 ggml DLL）"
} finally {
  if (-not $keepWork) {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  }
}

Write-Host "==> 下一步：运行 scripts/bootstrap_moss.ps1 下载模型并写入配置"
