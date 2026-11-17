<#
.SYNOPSIS
  构建「向量融合（embed）运行时」分发包 embed-runtime.zip。

.DESCRIPTION
  产物是一个 zip，解到 runtime 根（清单里 dest = "."）后布局为：

      runtime/deps_embed/             onnxruntime + numpy + tokenizers（26 个钉子包）
      runtime/models/embed/multilingual-e5-small/   ONNX + tokenizer（约 134 MB）
      runtime/worker/embed_worker.py  一次性子进程入口
      runtime/worker/fuse_core.py     融合核心（与实验台共用同一份源码）
      runtime/.embed-runtime          版本/模型标记

  设计取舍：
  - **单包**（不按 flavor 拆）：这 26 个包是纯 CPU 的 onnxruntime，没有 CUDA/DirectML 变体，
    也就没有「按显卡选包」这回事。唯一的前置依赖是 deps-ocr 带来的 python/ —— 本包装不下
    python（那是 47 MB 且与 OCR 共用一份），所以 embed 的安装前置 = deps-ocr 已装。
  - 不内嵌 Python：与 OCR 包共用 `runtime/python/python.exe`（省 47 MB，也避免两份解释器）。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\package_embed_runtime.ps1
  powershell -ExecutionPolicy Bypass -File scripts\package_embed_runtime.ps1 -SkipSmoke -KeepWork
#>
param(
  [string]$OutDir = "",
  [string]$WorkDir = "",
  [string]$ModelSource = "",
  [string]$WorkerSource = "",
  [string]$PythonVersion = "3.12.10",
  [string]$PipIndex = "",
  [string]$Proxy = "",
  [switch]$SkipSmoke,
  [switch]$KeepWork
)

# PS 5.1 默认不加载 ZipFile 类型（解 nupkg 要用）
Add-Type -AssemblyName System.IO.Compression.FileSystem
$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

if (-not $OutDir) { $OutDir = Join-Path (Split-Path -Parent $repoRoot) "_gsa_staging\packages" }
$drive = if (Test-Path "F:\") { "F:\" } else { "$($env:SystemDrive)\" }
if (-not $WorkDir) { $WorkDir = Join-Path $drive "gsbembed" }
if (-not $ModelSource) { $ModelSource = Join-Path $repoRoot "runtime\models\embed" }
if (-not $WorkerSource) { $WorkerSource = Join-Path $repoRoot "scripts" }

# python 3.12.10 的 NuGet 包摘要（与 package_ocr_runtime.ps1 同一份，固定版本才能字节可复现）
$PythonSha256 = "0EB85C2DFCCCCF1B17352DE4C397F69194035B7D37149EACC16F1147D93DE3B8"
$EmbedRuntimeVersion = "1"
$EmbedModel = "multilingual-e5-small"
$EmbedOnnx = "model_qint8_avx512_vnni.onnx"

function Log([string]$msg) { Write-Host "==> $msg" }

# pip 会往 stderr 打通知；PS 5.1 在 Stop 偏好下会把它抛成 NativeCommandError（即使成功）
function Invoke-Exe([string]$exe, [string[]]$argv, [string]$what) {
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  & $exe @argv
  $code = $LASTEXITCODE
  $ErrorActionPreference = $prevEap
  if ($code -ne 0) { throw "$what 失败（退出码 $code）" }
}

function Get-Sha256([string]$path) { (Get-FileHash $path -Algorithm SHA256).Hash.ToLower() }

# 只接管「自己造的」目录，防参数写错时误删
function Reset-WorkDir([string]$dir) {
  $leaf = Split-Path $dir -Leaf
  $parentLeaf = Split-Path (Split-Path $dir -Parent) -Leaf
  if (($leaf -notlike "gsbembed*") -and ($parentLeaf -notlike "gsbembed*")) {
    throw "只接管 gsbembed 开头的自有目录（当前: $dir）"
  }
  # 复用上次的工作树（python + deps_embed 已有且与 lock 一致）能省掉几分钟 pip
  if ((Test-Path (Join-Path $dir ".embed-runtime")) -and (Test-Path (Join-Path $dir "deps_embed"))) {
    Log "复用已有工作树：$dir"
    return
  }
  if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

# ─── 1. 构建工具用的 Python（NuGet 解压，与 bootstrap_ocr.ps1 一致）────────
function Install-Python([string]$work) {
  $pythonDir = Join-Path $work "python"
  $exe = Join-Path $pythonDir "python.exe"
  if (Test-Path $exe) {
    Log "Python 已存在，跳过：$pythonDir"
    return $exe
  }
  $nupkg = Join-Path $env:TEMP "python-$PythonVersion.nupkg"
  $staging = Join-Path $env:TEMP "python-$PythonVersion.extract"
  $url = "https://www.nuget.org/api/v2/package/python/$PythonVersion"
  Log "下载 Python $PythonVersion（NuGet）：$url"
  Invoke-WebRequest -Uri $url -OutFile $nupkg -UseBasicParsing
  $hash = (Get-FileHash $nupkg -Algorithm SHA256).Hash
  if ($hash -ne $PythonSha256) { throw "Python 包 SHA256 不符：$hash（期望 $PythonSha256）" }
  if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
  [System.IO.Compression.ZipFile]::ExtractToDirectory($nupkg, $staging)
  Move-Item (Join-Path $staging "tools") $pythonDir
  Remove-Item -Recurse -Force $staging
  Remove-Item -Force $nupkg
  if (-not (Test-Path $exe)) { throw "解压后找不到 python.exe" }
  $ver = (& $exe --version 2>&1 | Out-String).Trim()
  Log "Python 就绪：$ver"
  return $exe
}

# ─── 2. deps_embed（钉子清单）+ 模型 + worker ─────────────────────────────
function Build-Embed([string]$work, [string]$python) {
  $deps = Join-Path $work "deps_embed"
  $lock = Join-Path $scriptDir "embed-requirements.lock.txt"
  $pinCount = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' }).Count
  Log "依赖钉子清单：$pinCount 个包（$lock）"

  # 已有 deps_embed 且与 lock 一致就从简（重跑不必等 pip）
  $needPip = $true
  if (Test-Path $deps) {
    $want = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' } | Sort-Object)
    $got = @(& $python -m pip freeze --path $deps --disable-pip-version-check 2>$null | Sort-Object)
    if ((Compare-Object $want $got).Count -eq 0) { $needPip = $false; Log "deps_embed 与 lock 一致，跳过 pip" }
  }
  if ($needPip) {
    if (Test-Path $deps) { Remove-Item -Recurse -Force $deps }
    $argv = @("-m", "pip", "install", "--target", $deps, "-r", $lock,
      "--disable-pip-version-check", "--no-input", "--progress-bar", "off")
    if ($PipIndex) { $argv += @("--index-url", $PipIndex) }
    if ($Proxy) { $argv += @("--proxy", $Proxy) }
    Log "pip install --target deps_embed（$pinCount 个钉子包）"
    Invoke-Exe $python $argv "pip install deps_embed"
    $want = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' } | Sort-Object)
    $got = @(& $python -m pip freeze --path $deps --disable-pip-version-check 2>$null | Sort-Object)
    $diff = @(Compare-Object $want $got)
    if ($diff.Count -ne 0) {
      $diff | Select-Object -First 10 | ForEach-Object { Log "  $($_.SideIndicator) $($_.InputObject)" }
      throw "deps_embed 与 lock 不一致（$($diff.Count) 处），拒绝打包"
    }
    Log "依赖校验通过：$($got.Count) 个包与 lock 逐行一致"
  }

  # 模型：只允许一个模型目录（与 config 的 embed_model 约定一致）
  $modelDst = Join-Path $work "models\embed"
  if (Test-Path $modelDst) { Remove-Item -Recurse -Force $modelDst }
  New-Item -ItemType Directory -Force -Path $modelDst | Out-Null
  $srcModel = Join-Path $ModelSource $EmbedModel
  if (-not (Test-Path $srcModel)) { throw "模型目录不存在：$srcModel" }
  Copy-Item $srcModel (Join-Path $modelDst $EmbedModel) -Recurse -Force
  $onnxPath = Join-Path $modelDst "$EmbedModel\$EmbedOnnx"
  if (-not (Test-Path $onnxPath)) { throw "模型缺少 $EmbedOnnx：$onnxPath" }
  $tokPath = Join-Path $modelDst "$EmbedModel\tokenizer.json"
  if (-not (Test-Path $tokPath)) { throw "模型缺少 tokenizer.json：$tokPath" }
  $mb = (Get-ChildItem $modelDst -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
  Log ("模型就绪：{0}（{1:N1} MB）" -f $EmbedModel, $mb)

  # worker：两个脚本都必须来自仓库 scripts/（与实验台共用同一份源码）
  $workerDst = Join-Path $work "worker"
  New-Item -ItemType Directory -Force -Path $workerDst | Out-Null
  foreach ($n in @("embed_worker.py", "fuse_core.py")) {
    $src = Join-Path $WorkerSource $n
    if (-not (Test-Path $src)) { throw "worker 源缺失：$src" }
    Copy-Item $src (Join-Path $workerDst $n) -Force
  }
  Log "worker 就绪：embed_worker.py / fuse_core.py"

  $marker = @(
    "version=$EmbedRuntimeVersion",
    "model=$EmbedModel",
    "onnx=$EmbedOnnx",
    "python=$PythonVersion",
    "built_at=$((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
  ) -join "`n"
  [System.IO.File]::WriteAllText((Join-Path $work ".embed-runtime"), $marker,
    (New-Object System.Text.UTF8Encoding($false)))
}

# ─── 3. 冒烟：真跑一次 worker，断言输出 ok 且段数对 ────────────────────────
function Invoke-Smoke([string]$work, [string]$python) {
  $tmp = Join-Path $env:TEMP ("gsa_embed_smoke_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
  New-Item -ItemType Directory -Force -Path $tmp | Out-Null
  $inJson = Join-Path $tmp "input.json"
  $outJson = Join-Path $tmp "output.json"
  $req = @{
    corpus   = @("你好，世界。", "这是一句测试台词。", "第三行语料。")
    segments = @(
      @{ index = 1; start = 0.0; end = 1.0; text = "你好世界" },
      @{ index = 2; start = 1.0; end = 2.0; text = "这是一句测试台词" }
    )
    top_k    = 3
    model    = $EmbedModel
    onnx     = $EmbedOnnx
  }
  [System.IO.File]::WriteAllText($inJson, ($req | ConvertTo-Json -Depth 6),
    (New-Object System.Text.UTF8Encoding($false)))

  $worker = Join-Path $work "worker\embed_worker.py"
  $deps = Join-Path $work "deps_embed"
  $env:PYTHONPATH = $deps
  $env:PYTHONIOENCODING = "utf-8"
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $errLog = Join-Path $tmp "stderr.log"
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  & $python $worker $inJson $outJson 2>$errLog
  $code = $LASTEXITCODE
  $ErrorActionPreference = $prevEap
  $sw.Stop()

  if ($code -ne 0) {
    if (Test-Path $errLog) { Get-Content -Encoding UTF8 $errLog -TotalCount 20 | ForEach-Object { Log "  [stderr] $_" } }
    throw "冒烟失败：worker 退出码 $code"
  }
  if (-not (Test-Path $outJson)) { throw "冒烟失败：没有输出文件" }
  $res = Get-Content -Encoding UTF8 $outJson -Raw | ConvertFrom-Json
  if (-not $res.ok) { throw "冒烟失败：worker 报错 $($res.error)" }
  $n = @($res.segments).Count
  if ($n -ne 2) { throw "冒烟失败：段数 $n，期望 2" }
  Log ("冒烟通过（{0:N1}s）：model={1} corpus={2} segment={3} matched={4}" -f `
      $sw.Elapsed.TotalSeconds, $res.model, $res.corpus_size, $res.segment_count, $res.stats.matched)

  # 清理临时目录（含 stderr 日志，不进包）
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# ─── 4. 打包（tar 建包 + 条目数校验，防磁盘写满留下截断包）────────────────
function Assert-ZipEntries([string]$zip, [string[]]$paths, [string]$work, [string]$label) {
  $srcFiles = 0
  $srcBytes = [int64]0
  foreach ($p in $paths) {
    $full = Join-Path $work $p
    if (-not (Test-Path $full)) { throw "打包源不存在：$full" }
    $m = Get-ChildItem $full -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum
    $srcFiles += [int]$m.Count
    $srcBytes += [int64]$m.Sum
  }
  $eap = $ErrorActionPreference
  $ErrorActionPreference = "SilentlyContinue"
  $inZipRaw = @()
  try { $inZipRaw = @(tar.exe -tf $zip) } catch { }
  $tarCode = $LASTEXITCODE
  $ErrorActionPreference = $eap
  if ($tarCode -ne 0) { throw "包 $label 读不完整（tar -tf 退出码 $tarCode），可能被截断" }
  $inZip = @($inZipRaw | Where-Object { $_ -and -not $_.EndsWith("/") })
  if ($inZip.Count -ne $srcFiles) {
    throw "包 $label 条目数不符：zip 内 $($inZip.Count) 个文件，源 $srcFiles 个（打包可能被磁盘写满截断）"
  }
  return [pscustomobject]@{ Files = $srcFiles; Bytes = $srcBytes }
}

function New-Zip([string]$zip, [string]$work, [string[]]$paths, [string]$label) {
  if (Test-Path $zip) { Remove-Item $zip -Force }
  Log "打包 $label → $zip"
  $sw = [Diagnostics.Stopwatch]::StartNew()
  Invoke-Exe "tar.exe" (@("-a", "-c", "-f", $zip, "-C", $work) + $paths) "tar 打包（$label）"
  $sw.Stop()
  $counts = Assert-ZipEntries $zip $paths $work $label
  $size = (Get-Item $zip).Length
  $sha = Get-Sha256 $zip
  Log ("完成：{0} / {1:N1} MB（解压 {2:N1} MB，压缩比 {3:N2}x，{4:N1}s，{5} 个文件）" -f `
    $label, ($size/1MB), ($counts.Bytes/1MB), ($counts.Bytes/$size), $sw.Elapsed.TotalSeconds, $counts.Files)
  Log "  size=$size"
  Log "  sha256=$sha"
  Log "  unpacked_size=$($counts.Bytes)"
  return [pscustomobject]@{ Zip = $zip; Label = $label; Size = $size; Sha256 = $sha; Unpacked = $counts.Bytes; Files = $counts.Files }
}

# ─── 主流程 ───────────────────────────────────────────────────────────────
Log "工作目录：$WorkDir"
Log "产物目录：$OutDir"
Reset-WorkDir $WorkDir
if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Force -Path $OutDir | Out-Null }

$python = Install-Python $WorkDir
Build-Embed $WorkDir $python
if (-not $SkipSmoke) { Invoke-Smoke $WorkDir $python }

# 冒烟日志/临时文件一律不进包（实测教训：OCR 包曾把 smoke.stderr.log 发出去）
Get-ChildItem -LiteralPath $WorkDir -Filter "smoke.*.log" -File -ErrorAction SilentlyContinue |
  Remove-Item -Force -ErrorAction SilentlyContinue

$results = @()
$results += New-Zip (Join-Path $OutDir "embed-runtime.zip") $WorkDir `
  @("deps_embed", "models", "worker", ".embed-runtime") "embed-runtime"

Log ""
Log "产物清单（贴进 runtime manifest 用）："
foreach ($r in $results) {
  Log ("  {0}: size={1} sha256={2} unpacked_size={3}" -f $r.Label, $r.Size, $r.Sha256, $r.Unpacked)
}
Log "  dest = `".`"（包内自带 deps_embed/ models/ worker/ 一级目录）"
Log "  前置依赖：deps-ocr（提供 python/python.exe）"

if (-not $KeepWork) {
  Log "清理工作目录（-KeepWork 可保留）：$WorkDir"
  Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
}
