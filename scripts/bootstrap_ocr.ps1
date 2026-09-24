<#
bootstrap_ocr.ps1 —— 搭建开发期 OCR 运行环境（项目内嵌固定版本 Python，不再依赖系统 Python）
1. 下载 Python 3.12.10（NuGet python 包，完整 CPython + 自带 pip），校验 SHA256，解压到 runtime/python
2. 用内嵌解释器 pip install --target runtime/deps -r scripts/ocr-requirements.txt
3. 拷贝 scripts/ocr_worker.py -> runtime/worker/
4. 写 runtime/config.json（python_path 相对 runtime，机器无关）

为什么用 NuGet 包而不是 python.org 安装器：
  安装器是 WiX bundle，按 per-user 注册 "CPython-<ver> 已安装"；同一机器上第一个项目装完后，
  其它项目再静默安装会被判定"已安装"直接退出码 0，什么都不装（WixBundleInstalled=1）。
  NuGet 包是纯 zip 解压，无注册表/bundle 状态，任意目录可靠解压即用，适合每个项目内嵌一份。

参数：
  -Force            强制重装 Python（删除现有 runtime/python 后重新解压）
  -Mirror <前缀>    下载镜像前缀（默认 https://www.nuget.org/api/v2/package，
                    镜像需同样支持 /python/<版本> 的包下载路径）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr.ps1
#>
param(
  [switch]$Force,
  [string]$Mirror = ""
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression.FileSystem

$PythonVersion = "3.12.10"
# NuGet python 包 SHA256（nuget.org 官方发布，换镜像仍按此校验，防篡改）
$PythonSha256  = "0EB85C2DFCCCCF1B17352DE4C397F69194035B7D37149EACC16F1147D93DE3B8"

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

# ── 1. 内嵌 Python（解压 NuGet 包）──
$installedPython = $false
if ((Test-Path $pythonExe) -and -not $Force) {
  Write-Host "==> Python $PythonVersion 已存在，跳过安装（-Force 重装）"
} else {
  $installedPython = $true
  if ($Force -and (Test-Path $pythonDir)) {
    Remove-Item -Recurse -Force $pythonDir
  }
  $base = if ($Mirror) { $Mirror } else { "https://www.nuget.org/api/v2/package" }
  $url = "$base/python/$PythonVersion"
  $nupkg = Join-Path $env:TEMP "python-$PythonVersion.nupkg"
  $staging = Join-Path $env:TEMP "python-$PythonVersion.extract"

  Write-Host "==> 下载 Python $PythonVersion（NuGet 包）... $url"
  Invoke-WebRequest -Uri $url -OutFile $nupkg -UseBasicParsing
  $hash = (Get-FileHash $nupkg -Algorithm SHA256).Hash
  if ($hash -ne $PythonSha256) {
    throw "安装包 SHA256 校验失败: $hash（期望 $PythonSha256）"
  }

  Write-Host "==> 解压到 $pythonDir ..."
  if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
  [System.IO.Compression.ZipFile]::ExtractToDirectory($nupkg, $staging)
  Move-Item (Join-Path $staging "tools") $pythonDir
  Remove-Item -Recurse -Force $staging
  Remove-Item -Force $nupkg
  if (-not (Test-Path $pythonExe)) { throw "解压完成但找不到 $pythonExe" }
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
# 局部放宽错误偏好：pip 在目标目录已存在时向 stderr 打 WARNING，
# 外层 $ErrorActionPreference="Stop" 会把它当错误中断（重跑 bootstrap 时必现）。
# 这里只看退出码，stderr 交给控制台显示；失败仍由下面的 $LASTEXITCODE 判定。
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "Continue"
& $pythonExe -m pip install --disable-pip-version-check --no-warn-script-location `
  --target (Join-Path $runtime "deps") -r (Join-Path $scriptDir "ocr-requirements.txt")
$pipExit = $LASTEXITCODE
$ErrorActionPreference = $prevEap
if ($pipExit -ne 0) { throw "pip install 失败" }

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

# ── 5. 预置 OCR 模型 ──
# 为什么需要这一步：worker 按 config.json 的 model_dir（models/paddleocr）查找模型，
# 该目录为空时 worker 会尝试联网下载；首次下载慢或网络受限时会超时，导致基准测试
# 直接报 OCR 环境未就绪并静默跳过（排查成本高）。这里优先从用户级缓存复制，缓存缺失时才联网。
#
# 注意：缓存位置是 paddlex 的默认落盘目录（用户主目录下的 .paddlex/official_models），
# 与 worker 使用的项目内 model_dir 不同，故需显式复制。
$modelRoot = Join-Path $runtime "models\paddleocr\official_models"
New-Item -ItemType Directory -Force -Path $modelRoot | Out-Null
$userCache = Join-Path $env:USERPROFILE ".paddlex\official_models"
# 与 config.json 的 ocr_model=mobile 对应；换档位时同步改这里
$models = @("PP-OCRv5_mobile_det", "PP-OCRv5_mobile_rec")
foreach ($m in $models) {
  $dst = Join-Path $modelRoot $m
  if (Test-Path $dst) {
    Write-Host "==> 模型 $m 已存在，跳过"
    continue
  }
  $src = Join-Path $userCache $m
  if (Test-Path $src) {
    Write-Host "==> 从用户缓存复制模型 $m ..."
    Copy-Item -Recurse -Force $src $dst
  } else {
    # 缓存缺失：用内嵌解释器触发 paddlex 下载（会落到 ~/.paddlex，再复制进来）
    Write-Host "==> 缓存缺少 $m，联网下载（首次较慢）..."
    $env:PYTHONPATH = Join-Path $runtime "deps"
    $dlScript = Join-Path $env:TEMP "gsa_ocr_dl_model.py"
    @(
      "from paddleocr import PaddleOCR"
      "PaddleOCR(lang='ch', text_detection_model_name='$m',"
      "          use_doc_orientation_classify=False, use_doc_unwarping=False,"
      "          use_textline_orientation=False)"
    ) | Set-Content -Path $dlScript -Encoding UTF8
    # 同 pip：paddlex 会往 stderr 打 INFO/WARNING，局部放宽错误偏好，只看退出码
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $pythonExe $dlScript
    $dlExit = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    Remove-Item -Force $dlScript -ErrorAction SilentlyContinue
    if ($dlExit -ne 0) { throw "模型 $m 下载失败" }
    Copy-Item -Recurse -Force $src $dst
  }
}

Write-Host "==> OCR 运行环境就绪：$runtime"
