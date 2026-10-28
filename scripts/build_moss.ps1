<#
build_moss.ps1 —— 从源码构建 moss-transcribe.exe（仅开发者/CI 使用，用户设备不需要）
产物拷贝到 runtime/bin/moss-transcribe.exe，bootstrap_moss.ps1 负责模型下载与配置。

前置工具链（缺失时脚本明确报错，不做自动安装）：
  - git（clone 源码 + ggml 子模块）
  - cmake（构建系统）
  - Visual Studio 2022 Build Tools（C++ 桌面开发，MSVC 编译器）
  - nvcc（仅 -Backend cuda 需要，CUDA Toolkit）
  - glslc + Vulkan SDK（仅 -Backend vulkan 需要）：ggml-vulkan/CMakeLists.txt 强制要求
    find_package(Vulkan COMPONENTS glslc REQUIRED) 与 find_package(SPIRV-Headers CONFIG REQUIRED)。
    运行时不需要 SDK（显卡驱动提供 vulkan-1.dll 即可）。
    安装：winget install --id KhronosGroup.VulkanSDK -e

参数：
  -Force        强制重新构建（删除现有 runtime/bin/moss-transcribe.exe 后重建）
  -Backend      推理后端：cpu（默认）/ cuda / vulkan。GPU 后端由 ggml 支持
                （moss-transcribe.cpp 透传 GGML_CUDA / GGML_VULKAN，见官方 README），
                构建后 runtime/bin/ 会多出 ggml-cuda.dll / ggml-vulkan.dll
                （GGML_BACKEND_DL 开启，2026-10-19 实测 vulkan 版产出独立 ggml-vulkan.dll），
                运行时 backend 自动选 GPU（MTD_DEVICE=cpu 可强制回退；注意裸 MTD_DEVICE=vulkan
                会选中枚举序里的第一块 GPU，本机即 Intel 核显 —— 见 docs/engines_vulkan_design.md §3.3）。
  -WorkDir      构建工作目录（默认 <系统盘>\gsa-moss-build\<backend>-<PID>）。
                **有硬约束**：vulkan 构建树最深的 TryCompile 瞬态尾巴实测 208 字符，工作根
                超 ~50 字符即触 FileTracker FTK1011；勿挪进仓库（仓库路径本身已 58 字符）。
  -GitMirror    源码镜像前缀，如 https://ghproxy.com/ 或 https://gitclone.com/github.com/（网络受限时使用）
  -OutputDir    产物目录（默认 <仓库>\runtime\bin）。**分发打包必须分目录**：三个 flavor 的
                ggml-*.dll 同名互斥（cpu 版恒有 ggml-cpu.dll，GPU 版另有 ggml-cuda.dll /
                ggml-vulkan.dll），混装目录会加载到错误后端并静默降级。相对路径按仓库根解析。
  -UpstreamRef  上游源码锚点（commit sha 或 tag）。**分发必须 pin**：浅克隆 master 无法保证
                多个 flavor 同源，manifest 的 source_version 也取自它。空 = 拉当前 master。
  -SourceDir    复用已准备好的源码树（含 third_party/ggml 子模块），跳过克隆。
                用途：多个 flavor 必须同源且网络慢时，先备好一份 pin 源码，再逐个 flavor 构建。
  -CudaArch     CUDA 架构列表（仅 -Backend cuda），默认 75;86;89（Turing/Ampere/Ada）。
                单 arch 构建快 2-3 倍；本地快速迭代可传本机 arch（如 89），分发产物必须用默认值。
  -Jobs         构建并行度（默认不限核数）。CUDA 多架构构建时单个 nvcc 可占 1–3 GB，
                32 GB 机器上并行满核易爆内存，需要时按内存给（如 10）。
用法：powershell -ExecutionPolicy Bypass -File scripts/build_moss.ps1 -Backend cuda
#>
param(
  [switch]$Force,
  [ValidateSet("cpu", "cuda", "vulkan")]
  [string]$Backend = "cpu",
  [string]$WorkDir = "",
  [string]$GitMirror = "",
  [string]$OutputDir = "",
  [string]$UpstreamRef = "",
  [string]$SourceDir = "",
  [string]$CudaArch = "75;86;89",
  [int]$Jobs = 0
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
# -OutputDir：分发打包时把各 flavor 产到独立目录（runtime/_flavors/<flavor>/bin）
if ($OutputDir) {
  $binDir = if ([System.IO.Path]::IsPathRooted($OutputDir)) { $OutputDir } else { Join-Path $root $OutputDir }
} else {
  $binDir = Join-Path $runtime "bin"
}
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
if ($Backend -eq "vulkan") {
  # ggml-vulkan 的强制前置（ggml-vulkan/CMakeLists.txt）：
  #   find_package(Vulkan COMPONENTS glslc REQUIRED)
  #   find_package(SPIRV-Headers CONFIG REQUIRED)
  # 缺任一个，cmake 在**配置阶段**就失败。提前给出可执行的修法，不让用户去猜 cmake 报错。
  $glslcPath = $null
  $g = Get-Command glslc -ErrorAction SilentlyContinue
  if ($g) {
    $glslcPath = $g.Source
  } else {
    # 兜底路径：VULKAN_SDK 环境变量 > C:\VulkanSDK\<版本>（LunarG 安装器的默认位置）
    $sdkRoots = @()
    if ($env:VULKAN_SDK) { $sdkRoots += $env:VULKAN_SDK }
    $sdkRoots += @(Get-ChildItem "C:\VulkanSDK" -Directory -ErrorAction SilentlyContinue |
      Sort-Object Name -Descending | ForEach-Object { $_.FullName })
    foreach ($r in $sdkRoots) {
      $cand = Join-Path $r "Bin\glslc.exe"
      if (Test-Path $cand) {
        # 设 VULKAN_SDK：ggml 会把它追加进 CMAKE_PREFIX_PATH 供 find_package(Vulkan) 定位
        $env:VULKAN_SDK = $r
        $env:PATH = "$(Join-Path $r 'Bin');$env:PATH"
        $glslcPath = $cand
        break
      }
    }
  }
  if (-not $glslcPath) {
    $missing += "glslc（Vulkan SDK；安装：winget install --id KhronosGroup.VulkanSDK -e）"
  } else {
    Write-Host "==> 找到 glslc: $glslcPath"
    if (-not $env:VULKAN_SDK) {
      Write-Host "==> 注意：VULKAN_SDK 未设置。cmake 若报找不到 Vulkan，请把它指向 SDK 根目录"
    } else {
      # SPIRV-Headers 的 CMake CONFIG 包：LunarG SDK 1.4.363.0 实测放在 <SDK>\Lib\cmake\SPIRV-Headers\
      # （**不是** share\cmake）。ggml 会把 $VULKAN_SDK 追加进 CMAKE_PREFIX_PATH，CMake 按
      # <prefix>/[share|lib|Lib]/cmake/<name>/ 搜索，所以这几处都要探，否则对装好的 SDK 误报。
      $spirvCfg = $null
      foreach ($sub in @("Lib\cmake", "lib\cmake", "share\cmake")) {
        $hit = Get-ChildItem (Join-Path $env:VULKAN_SDK $sub) -Recurse -Filter "SPIRV-HeadersConfig.cmake" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($hit) { $spirvCfg = $hit.FullName; break }
      }
      if (-not $spirvCfg) {
        Write-Host "==> 警告：未在 $env:VULKAN_SDK 下找到 SPIRV-Headers 的 CMake CONFIG 包（SPIRV-HeadersConfig.cmake）"
        Write-Host "           ggml 的 find_package(SPIRV-Headers CONFIG REQUIRED) 会因此失败。"
        Write-Host "           修法：取 SPIRV-Headers 的 tag vulkan-sdk-<SDK 版本>，用 cmake --install 装到该前缀"
        Write-Host "           （见 docs/engines_vulkan_design.md §2.3）。"
      } else {
        Write-Host "==> 找到 SPIRV-Headers CONFIG 包: $spirvCfg"
      }
    }
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
$archStamp = Join-Path $binDir ".cudaarch"
$old = if (Test-Path $stamp) { (Get-Content $stamp -Raw).Trim() } else { "" }
$needsRebuild = $false
if (Test-Path $targetExe) {
  if ($old -eq "") {
    # 无标记：CPU 请求 → 假定旧构建即 CPU，跳过；GPU 请求 → 无对应 DLL 证据，重建
    if ($Backend -eq "cuda" -and -not (Test-Path (Join-Path $binDir "ggml-cuda.dll"))) { $needsRebuild = $true }
    elseif ($Backend -eq "vulkan" -and -not (Test-Path (Join-Path $binDir "ggml-vulkan.dll"))) { $needsRebuild = $true }
  } elseif ($old -ne $Backend) {
    $needsRebuild = $true
  } elseif ($Backend -eq "cuda") {
    # 同后端也可能换了 arch（-CudaArch）：device 代码是按 arch 编进 DLL 的，
    # 不重建就会把旧 arch 的二进制当成新 arch 的产物发出去，且外观完全无法分辨。
    $oldArch = if (Test-Path $archStamp) { (Get-Content $archStamp -Raw).Trim() } else { "" }
    if ($oldArch -ne $CudaArch) {
      Write-Host "==> CUDA arch 变更：$(if ($oldArch) { $oldArch } else { '未知（旧构建）' }) → $CudaArch，重建"
      $needsRebuild = $true
    }
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
# 工作目录有路径长度硬约束：vulkan 构建树的最深处是 ExternalProject 的 TryCompile 瞬态目录，
#   <work>\src\build-vulkan\vulkan-shaders-gen-prefix\src\vulkan-shaders-gen-build\CMakeFiles\
#   CMakeScratch\TryCompile-xxxxxx\cmTC_xxxxx.dir\Debug\cmTC_xxxxx.tlog\link-cvtres.read.1.tlog
# 其与工作根无关的尾巴实测 208 字符，Windows MAX_PATH = 260 ⇒ 工作根超 ~50 字符即报
# FileTracker error FTK1011（默认 %TEMP%\gsa_moss_build_<PID> 实测 53 字符恰好爆，2026-10-19）。
# 因此默认放系统盘根下的短目录；**不要**把构建目录挪进仓库（仓库路径本身已 58 字符）。
if ($WorkDir) {
  $work = $WorkDir
} else {
  $work = "$($env:SystemDrive)\gsa-moss-build\$Backend-$PID"
}
# 路径长度告警：留 208 字符瞬态尾巴后，工作根须 ≤ ~50 字符，否则中途 FTK1011
$deepest = $work.Length + 208
if ($deepest -gt 250) {
  Write-Host "==> 警告：工作目录 $work（$($work.Length) 字符）+ 构建瞬态尾巴 208 = $deepest，超过安全线 250 ——"
  Write-Host "           曾实测触发 FileTracker FTK1011（MSVC BuildTools 的 tlog 路径超 MAX_PATH=260）。"
  Write-Host "           若构建中途报 FTK1011/找不到文件，请用 -WorkDir 指定更短目录（≤ 50 字符）。"
}
if (Test-Path $work) {
  # 默认目录可整删重用；用户自定义 -WorkDir 只接受空目录（防止误删非本脚本创建的数据）
  if ($WorkDir) {
    if ((Get-ChildItem $work -Force -ErrorAction SilentlyContinue | Measure-Object).Count -gt 0) {
      throw "-WorkDir $work 非空：请指定一个空目录（脚本会以它为构建工作区）"
    }
  } else {
    Remove-Item -Recurse -Force $work
  }
}
try {
  New-Item -ItemType Directory -Force -Path $work | Out-Null
} catch {
  throw "无法创建工作目录 $work（$($_.Exception.Message)）；系统盘根无写权限时可用 -WorkDir 指定可写的短路径（≤ 50 字符）"
}

# 失败/被中断时保留工作目录，便于人工排查（成功才清理）
$keepWork = $false

try {
  $repoUrl = "https://github.com/mudler/moss-transcribe.cpp"
  if ($GitMirror) { $repoUrl = "$GitMirror$repoUrl" }

  $srcDir = Join-Path $work "src"
  if ($SourceDir) {
    # 复用已准备好的 pin 源码树（含 ggml 子模块）：多 flavor 同源，且不用重复下载仓库
    if (-not (Test-Path (Join-Path $SourceDir "third_party\ggml\CMakeLists.txt"))) {
      $keepWork = $true
      throw "-SourceDir $SourceDir 缺少 ggml 子模块（third_party\ggml\CMakeLists.txt），不可用"
    }
    Write-Host "==> 复用已准备好的源码树：$SourceDir（含 ggml 子模块）"
    Copy-Item -Recurse -Force $SourceDir $srcDir
  } elseif ($UpstreamRef) {
    # pin 指定 commit/tag：浅克隆 master 无法保证多 flavor 同源，分发包必须同源可复现。
    # git init + fetch --depth 1 <sha> 也能拿到任意 commit（GitHub 允许按 SHA fetch）。
    Write-Host "==> 克隆源码并 pin 到 $UpstreamRef（git init + fetch --depth 1）..."
    & git init $srcDir
    if ($LASTEXITCODE -ne 0) { $keepWork = $true; throw "git init 失败，工作目录保留在 $work" }
    & git -C $srcDir remote add origin $repoUrl
    if ($GitMirror) {
      # ggml 子模块的 URL 是绝对地址（.gitmodules: https://github.com/ggml-org/ggml），
      # 不改写的话主仓库走镜像、子模块直连 GitHub（国内常超时）→ 统一改写走镜像
      & git -C $srcDir config "url.$($GitMirror)https://github.com/.insteadOf" "https://github.com/"
    }
    & git -C $srcDir fetch --depth 1 origin $UpstreamRef
    if ($LASTEXITCODE -ne 0) { $keepWork = $true; throw "git fetch $UpstreamRef 失败（网络受限可试 -GitMirror），工作目录保留在 $work" }
    & git -C $srcDir checkout FETCH_HEAD
    if ($LASTEXITCODE -ne 0) { $keepWork = $true; throw "git checkout FETCH_HEAD 失败，工作目录保留在 $work" }
    & git -C $srcDir submodule update --init --recursive --depth 1
    if ($LASTEXITCODE -ne 0) { $keepWork = $true; throw "ggml 子模块拉取失败，工作目录保留在 $work" }
  } else {
    Write-Host "==> 克隆源码（浅克隆）..."
    & git clone --depth 1 $repoUrl $srcDir
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "git clone 失败（网络受限可试 -GitMirror 镜像参数），工作目录保留在 $work"
    }
    if ($GitMirror) {
      & git -C $srcDir config "url.$($GitMirror)https://github.com/.insteadOf" "https://github.com/"
    }
    # 子模块单独拉：先设好 insteadOf 再拉，避免子模块绕过镜像直连 GitHub
    # --depth 1：ggml 历史庞大，浅克隆子模块可显著减少下载量
    Write-Host "==> 拉取 ggml 子模块（浅克隆）..."
    & git -C $srcDir submodule update --init --recursive --depth 1
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "ggml 子模块拉取失败，工作目录保留在 $work"
    }
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
      $backendFlag = @("-DMT_GGML_CUDA=ON", "-DCMAKE_CUDA_ARCHITECTURES=$CudaArch")
      $useNinja = $true   # 自定义 CUDA 路径下 VS toolset 常缺失，Ninja 更稳
    }
    "vulkan" { $buildDir = "build-vulkan"; $backendFlag = @("-DMT_GGML_VULKAN=ON") }
  }

  Write-Host "==> cmake 配置（backend=$Backend）..."
  Push-Location (Join-Path $work "src")
  # 原生命令的成败一律看 $LASTEXITCODE，不看 stderr：本脚本是 $ErrorActionPreference="Stop"，
  # 一旦调用方把 stderr 合并进管道（如 `build_moss.ps1 ... | Tee-Object log`），
  # cmake 往 stderr 打的任意一条 warning（实测 CMakeDetermineASMCompiler 的 dev warning）
  # 都会变成 terminating error，让配置阶段假失败并误报"需装 VS Build Tools"。
  $eapBeforeCmake = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
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
    if ($Jobs -gt 0) {
      & cmake --build $buildDir --config Release --parallel $Jobs
    } else {
      & cmake --build $buildDir --config Release -j
    }
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "cmake 构建失败，详见上方日志，工作目录保留在 $work"
    }
  } finally {
    $ErrorActionPreference = $eapBeforeCmake
    Pop-Location
  }

  # MSVC 多配置生成器输出在 build/Release/，单配置（MinGW/Ninja）在 build/
  $built = Join-Path $work "src\$buildDir\Release\moss-transcribe.exe"
  if (-not (Test-Path $built)) { $built = Join-Path $work "src\$buildDir\moss-transcribe.exe" }
  if (-not (Test-Path $built)) { throw "构建完成但找不到 moss-transcribe.exe（$buildDir\Release\ 或 $buildDir\）" }

  Copy-Item $built $targetExe -Force
  # ggml 以共享库构建：DLL 在 build/bin/Release/（CMake RUNTIME_OUTPUT_DIRECTORY）。
  # Ninja 单配置布局：build-cuda/bin/*.dll（无 Release 子目录）——2026-09-30 CUDA 构建
  # 实踩：仅回退 exe 同目录导致 DLL 全缺（info 自检 0xC0000135），故先探 Ninja 的 bin/。
  $exeDir = Split-Path $built
  $dllDir = Join-Path $exeDir "..\bin\Release"
  if (-not (Test-Path (Join-Path $dllDir "ggml.dll"))) { $dllDir = Join-Path $exeDir "bin" }
  if (-not (Test-Path (Join-Path $dllDir "ggml.dll"))) { $dllDir = $exeDir }
  # 产物纯净性：三个 flavor 的 GPU 后端 DLL（ggml-cuda.dll / ggml-vulkan.dll）互斥，
  # 混装目录会按加载顺序选到错误后端并静默降级（契约 §9 要硬拦的那一类）。
  # 复制前先清掉输出目录里旧的 ggml*.dll，保证目录里只有本次 flavor 的运行时。
  Get-ChildItem -Path $binDir -Filter "ggml*.dll" -ErrorAction SilentlyContinue | Remove-Item -Force
  Get-ChildItem -Path $dllDir -Filter "*.dll" | Copy-Item -Destination $binDir -Force
  # 记录后端标记，供下次构建检测后端切换
  Set-Content -Path (Join-Path $binDir ".backend") -Value $Backend -NoNewline
  # cuda 额外记录 arch：换 arch 必须重建（见上方重建判定）
  if ($Backend -eq "cuda") {
    Set-Content -Path $archStamp -Value $CudaArch -NoNewline
  } elseif (Test-Path $archStamp) {
    Remove-Item -Force $archStamp
  }

  # 纯净性校验：目标 flavor 的后端 DLL 必须在，别的 flavor 的 GPU DLL 必须不在
  $mustNot = @()
  if ($Backend -ne "cuda") { $mustNot += "ggml-cuda.dll" }
  if ($Backend -ne "vulkan") { $mustNot += "ggml-vulkan.dll" }
  foreach ($f in $mustNot) {
    if (Test-Path (Join-Path $binDir $f)) {
      throw "产物目录混入其它 flavor 的 $f（backend=$Backend）：会加载到错误后端并静默降级。请清空 $binDir 后重建"
    }
  }
  if ($Backend -eq "cuda" -and -not (Test-Path (Join-Path $binDir "ggml-cuda.dll"))) {
    throw "构建完成但缺少 ggml-cuda.dll（backend=cuda），产物不可用"
  }
  if ($Backend -eq "vulkan" -and -not (Test-Path (Join-Path $binDir "ggml-vulkan.dll"))) {
    throw "构建完成但缺少 ggml-vulkan.dll（backend=vulkan），产物不可用"
  }
  if ($Backend -eq "cpu" -and -not (Test-Path (Join-Path $binDir "ggml-cpu.dll"))) {
    Write-Host "==> 提示：未产出 ggml-cpu.dll（CPU 后端可能静态链入），属预期内情形则可忽略"
  }
  Write-Host "==> 构建成功：$targetExe（backend=$Backend，含 ggml DLL）"
} finally {
  if (-not $keepWork) {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  }
}

Write-Host "==> 下一步：运行 scripts/bootstrap_moss.ps1 下载模型并写入配置"