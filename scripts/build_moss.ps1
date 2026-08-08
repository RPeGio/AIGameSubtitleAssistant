<#
build_moss.ps1 —— 从源码构建 moss-transcribe.exe（仅开发者/CI 使用，用户设备不需要）
产物拷贝到 runtime/bin/moss-transcribe.exe，bootstrap_moss.ps1 负责模型下载与配置。

前置工具链（缺失时脚本明确报错，不做自动安装）：
  - git（clone 源码 + ggml 子模块）
  - cmake（构建系统）
  - Visual Studio 2022 Build Tools（C++ 桌面开发，MSVC 编译器）

参数：
  -Force        强制重新构建（删除现有 runtime/bin/moss-transcribe.exe 后重建）
  -GitMirror    源码镜像前缀，如 https://ghproxy.com/ 或 https://gitclone.com/github.com/（网络受限时使用）
用法：powershell -ExecutionPolicy Bypass -File scripts/build_moss.ps1
#>
param(
  [switch]$Force,
  [string]$GitMirror = ""
)

$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$binDir = Join-Path $runtime "bin"
$targetExe = Join-Path $binDir "moss-transcribe.exe"

# ── 1. 工具链检查 ──
$missing = @()
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { $missing += "git" }
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) { $missing += "cmake" }
if ($missing.Count -gt 0) {
  throw "缺少工具链: $($missing -join '、')。请安装后重试（git: https://git-scm.com，cmake: https://cmake.org）"
}

# ── 2. 清理旧产物 ──
if ((Test-Path $targetExe) -and $Force) {
  Write-Host "==> -Force：删除旧二进制 $targetExe"
  Remove-Item -Force $targetExe
}
if (Test-Path $targetExe) {
  Write-Host "==> $targetExe 已存在，跳过构建（-Force 重建）"
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

  Write-Host "==> cmake 配置..."
  Push-Location (Join-Path $work "src")
  try {
    & cmake -B build -DMT_BUILD_TESTS=OFF
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "cmake 配置失败（需 Visual Studio 2022 Build Tools C++ 工作负载），工作目录保留在 $work"
    }

    Write-Host "==> 编译（Release，耗时数分钟）..."
    & cmake --build build --config Release -j
    if ($LASTEXITCODE -ne 0) {
      $keepWork = $true
      throw "cmake 构建失败，详见上方日志，工作目录保留在 $work"
    }
  } finally {
    Pop-Location
  }

  # MSVC 多配置生成器输出在 build/Release/，单配置（MinGW/Ninja）在 build/
  $built = Join-Path $work "src\build\Release\moss-transcribe.exe"
  if (-not (Test-Path $built)) { $built = Join-Path $work "src\build\moss-transcribe.exe" }
  if (-not (Test-Path $built)) { throw "构建完成但找不到 moss-transcribe.exe（build\Release\ 或 build\）" }

  Copy-Item $built $targetExe -Force
  # ggml 以共享库构建：DLL 在 build/bin/Release/（CMake RUNTIME_OUTPUT_DIRECTORY），
  # 单配置生成器可能无该布局，回退到 exe 同目录
  $exeDir = Split-Path $built
  $dllDir = Join-Path $exeDir "..\bin\Release"
  if (-not (Test-Path (Join-Path $dllDir "ggml.dll"))) { $dllDir = $exeDir }
  Get-ChildItem -Path $dllDir -Filter "*.dll" | Copy-Item -Destination $binDir -Force
  Write-Host "==> 构建成功：$targetExe（含 ggml DLL）"
} finally {
  if (-not $keepWork) {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
  }
}

Write-Host "==> 下一步：运行 scripts/bootstrap_moss.ps1 下载模型并写入配置"
