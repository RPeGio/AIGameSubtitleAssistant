<#
bootstrap_ocr.ps1 —— 搭建开发期 OCR 运行环境（项目内嵌固定版本 Python，不再依赖系统 Python）
1. 下载 Python 3.12.10 安装器并校验 SHA256，静默安装到 runtime/python
2. 用内嵌解释器 pip install --target runtime/deps -r scripts/ocr-requirements.txt
3. 拷贝 scripts/ocr_worker.py -> runtime/worker/
4. 写 runtime/config.json（python_path 相对 runtime，机器无关）

参数：
  -Force            强制重装 Python（删除现有 runtime/python 后重装）
  -Mirror <前缀>    下载镜像前缀，如 https://registry.npmmirror.com/-/binary
                    默认 https://www.python.org/ftp（镜像下载后仍校验 SHA256）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr.ps1
#>
param(
  [switch]$Force,
  [string]$Mirror = ""
)

$ErrorActionPreference = "Stop"

# 固定内嵌版本：3.12 最后一个带官方 Windows 安装器的补丁版（3.12.11+ 仅源码包）
$PythonVersion = "3.12.10"
# 安装器 SHA256（python.org 官方发布，换镜像仍按此校验，防篡改）
$PythonSha256  = "67B5635E80EA51072B87941312D00EC8927C4DB9BA18938F7AD2D27B328B95FB"

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$pythonDir = Join-Path $runtime "python"
$pythonExe = Join-Path $pythonDir "python.exe"

$dirs = @(
  (Join-Path $runtime "deps"),
  (Join-Path $runtime "worker"),
  (Join-Path $runtime "models\paddleocr")
)
foreach ($d in $dirs) {
  New-Item -ItemType Directory -Force -Path $d | Out-Null
}

# ── 1. 内嵌 Python ──
$installedPython = $false
if ((Test-Path $pythonExe) -and -not $Force) {
  Write-Host "==> Python $PythonVersion 已存在，跳过安装（-Force 重装）"
} else {
  $installedPython = $true
  if ($Force -and (Test-Path $pythonDir)) {
    Remove-Item -Recurse -Force $pythonDir
  }
  $base = if ($Mirror) { $Mirror } else { "https://www.python.org/ftp" }
  $url = "$base/python/$PythonVersion/python-$PythonVersion-amd64.exe"
  $installer = Join-Path $env:TEMP "python-$PythonVersion-amd64.exe"

  Write-Host "==> 下载 Python $PythonVersion ..."
  Invoke-WebRequest -Uri $url -OutFile $installer -UseBasicParsing
  $hash = (Get-FileHash $installer -Algorithm SHA256).Hash
  if ($hash -ne $PythonSha256) {
    throw "安装器 SHA256 校验失败: $hash（期望 $PythonSha256）"
  }

  Write-Host "==> 静默安装到 $pythonDir ..."
  $p = Start-Process -FilePath $installer -ArgumentList @(
    "/quiet", "InstallAllUsers=0", "TargetDir=$pythonDir",
    "PrependPath=0", "Include_launcher=0",
    "Include_test=0", "Include_doc=0", "Include_tcltk=0",
    "Include_pip=1", "Include_dev=0"
  ) -Wait -PassThru
  if ($p.ExitCode -ne 0) { throw "Python 安装失败（exit=$($p.ExitCode)）" }
  Remove-Item -Force $installer
  if (-not (Test-Path $pythonExe)) { throw "安装完成但找不到 $pythonExe" }
}

$ver = & $pythonExe --version 2>&1
Write-Host "==> 使用内嵌解释器: $ver"

# ── 2. 依赖（用内嵌解释器）──
# 换了解释器（或强制重装）时清掉旧 deps：可能混有其它 Python 版本的 wheel，白占空间
if ($installedPython -or $Force) {
  $deps = Join-Path $runtime "deps"
  if (Test-Path $deps) { Remove-Item -Recurse -Force $deps }
  New-Item -ItemType Directory -Force -Path $deps | Out-Null
}
Write-Host "==> 安装 OCR 依赖到 runtime\deps ..."
& $pythonExe -m pip install --disable-pip-version-check --no-warn-script-location `
  --target (Join-Path $runtime "deps") -r (Join-Path $scriptDir "ocr-requirements.txt")
if ($LASTEXITCODE -ne 0) { throw "pip install 失败" }

# ── 3. 拷贝 worker 脚本 ──
Write-Host "==> 拷贝 worker 脚本 ..."
Copy-Item (Join-Path $scriptDir "ocr_worker.py") (Join-Path $runtime "worker\ocr_worker.py") -Force

# ── 4. 配置 ──
$config = [ordered]@{
  python_path   = "python/python.exe"   # 相对 runtime，机器无关（Rust 侧按 runtime_dir 解析）
  worker_script = "worker/ocr_worker.py"
  deps_dir      = "deps"
  model_dir     = "models/paddleocr"
  language      = "ch"
  ocr_model     = "mobile"
  dev_debug     = $true
}
# 无 BOM 写入（Set-Content -Encoding UTF8 会带 BOM，Rust 端 serde_json 解析会失败）
[System.IO.File]::WriteAllText(
  (Join-Path $runtime "config.json"),
  ($config | ConvertTo-Json),
  (New-Object System.Text.UTF8Encoding $false)
)

Write-Host "==> OCR 运行环境就绪：$runtime"
