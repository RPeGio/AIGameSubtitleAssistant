param(
  [ValidateSet("cpu", "cuda", "all")]
  [string]$Flavor = "cpu",
  [string]$OutDir = "",
  [string]$WorkDir = "",
  [string]$ModelSource = "",
  [string]$CpuBaseline = "",
  [string]$PythonVersion = "3.12.10",
  [string]$PipIndex = "",
  [string]$Proxy = "",
  [switch]$SkipSmoke,
  [switch]$KeepWork
)

# ─── package_ocr_runtime.ps1 —— 构建 OCR 运行时**分发包**（首启下载用） ──────────
#
# 与 bootstrap_ocr.ps1 / bootstrap_ocr_gpu.ps1 的分工：
#   - 那两个是**开发期**脚本：在用户机器上联网 pip install（慢、随上游漂移）；
#   - 本脚本只在**发布机**跑：按 scripts/ocr-*.lock.txt 的钉子清单构建一棵
#     自包含目录树，打成 zip 上传 Release，用户机只做「下载 + 校验 + 解压」。
#
# 产物（交给 src-tauri/resources/runtime/manifest.json 的 deps-ocr 条目）：
#   cpu  : ocr-runtime-cpu.zip   → runtime/{python,deps,models/paddleocr,worker}
#   cuda : ocr-runtime-cuda.zip  → runtime/deps_gpu（**增量**：必须叠加在 cpu 之上）
#
# 为什么 cuda 是增量而不是替代：PYTHONPATH = "deps_gpu;deps"，纯 Python 依赖
# （numpy/protobuf/pillow…）一律复用 cpu 基线的 deps，保证 CPU/GPU 精度对比可归因到
# CUDA 本身；见 bootstrap_ocr_gpu.ps1:10-17。
#
# 为什么钉清单而不是 ocr-requirements.txt（只有 2 行顶层包）：分发物必须字节可复现，
# 传递依赖（numpy/pandas/opencv…）随上游漂移会让「同样的 sha256」再也造不出来。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts/package_ocr_runtime.ps1 -Flavor all
#   ｜可选 -PipIndex https://pypi.tuna.tsinghua.edu.cn/simple  -Proxy http://127.0.0.1:7897
#   ｜-SkipSmoke 跳过冒烟（不建议；冒烟是唯一能挡住「装好了却用不了」的环节）

$ErrorActionPreference = "Stop"
# PS 5.1 默认没加载压缩程序集；ZipFile.ExtractToDirectory 依赖它（bootstrap_ocr.ps1:25 同写法）
Add-Type -AssemblyName System.IO.Compression.FileSystem

# NuGet python 包 SHA256（与 bootstrap_ocr.ps1 同一份来源，换版本必须同步改两处）
$PythonSha256 = "0EB85C2DFCCCCF1B17352DE4C397F69194035B7D37149EACC16F1147D93DE3B8"
# 分发物版本：条目内 .ocr-runtime 标记与清单 source_version 都用它；升级 OCR 时 +1
$OcrRuntimeVersion = "1"

$scriptDir = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
if (-not $OutDir) { $OutDir = Join-Path (Split-Path $repoRoot -Parent) "_gsa_staging\packages" }
# 工作根必须短：deps 里 paddle/nvidia 的路径本来就深，长前缀会撞 MAX_PATH（实测 >260 直接失败）
if (-not $WorkDir) { $WorkDir = Join-Path $env:SystemDrive "gsbocr" }
if (-not $ModelSource) { $ModelSource = Join-Path $repoRoot "runtime\models\paddleocr" }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

function Log([string]$msg) { Write-Host "==> $msg" }

# pip 会往 stderr 打通知（"A new release of pip is available" 之类），
# PS 5.1 在 Stop 偏好下会把它抛成 NativeCommandError 中断脚本（即使命令成功）。
function Invoke-Exe([string]$exe, [string[]]$argv, [string]$what) {
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  & $exe @argv
  $code = $LASTEXITCODE
  $ErrorActionPreference = $prevEap
  if ($code -ne 0) { throw "$what 失败（退出码 $code）" }
}

function Get-Sha256([string]$path) { (Get-FileHash $path -Algorithm SHA256).Hash.ToLower() }

# ─── 工作区准备 ───────────────────────────────────────────────────────────
function Reset-WorkDir([string]$dir) {
  # 只接管「自己造的」目录：父目录名必须是 gsbocr 前缀、叶子只能是 cpu/cuda，
  # 防止参数写错时误删用户目录
  $parent = Split-Path $dir -Parent
  $leaf = Split-Path $dir -Leaf
  if ((Split-Path $parent -Leaf) -notlike "gsbocr*") {
    throw "工作目录的父目录必须是 gsbocr 开头的自有目录（当前: $dir）"
  }
  if ($leaf -notin @("cpu", "cuda")) { throw "工作目录叶子只能是 cpu/cuda（当前: $dir）" }
  if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

# ─── 1. 内嵌 Python（NuGet 包解压，与 bootstrap_ocr.ps1 完全一致） ─────────
function Install-Python([string]$work) {
  $pythonDir = Join-Path $work "python"
  if (Test-Path (Join-Path $pythonDir "python.exe")) {
    Log "Python 已存在，跳过：$pythonDir"
    return (Join-Path $pythonDir "python.exe")
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
  if (-not (Test-Path (Join-Path $pythonDir "python.exe"))) { throw "解压后找不到 python.exe" }
  # 必须先把版本号收进变量：直接写进字符串会让函数的返回值变成「版本号 + 路径」数组
  $ver = (& (Join-Path $pythonDir "python.exe") --version 2>&1 | Out-String).Trim()
  Log "Python 就绪：$ver"
  return (Join-Path $pythonDir "python.exe")
}

# ─── 2a. CPU 基线：deps + 模型 + worker ───────────────────────────────────
function Build-Cpu([string]$work, [string]$python) {
  $deps = Join-Path $work "deps"
  $lock = Join-Path $scriptDir "ocr-requirements.lock.txt"
  $pinCount = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' }).Count
  New-Item -ItemType Directory -Force -Path $deps | Out-Null
  $wantPin = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' } | Sort-Object)
  $havePin = @()
  if (Test-Path $deps) {
    $prevEap0 = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $havePin = @(& $python -m pip freeze --path $deps --disable-pip-version-check 2>$null | Where-Object { $_ } | Sort-Object)
    $ErrorActionPreference = $prevEap0
  }
  if (@(Compare-Object $wantPin $havePin).Count -eq 0 -and $havePin.Count -gt 0) {
    Log "deps 已与 lock 一致（$($havePin.Count) 个包），跳过 pip install"
  } else {
  Log "pip install --target deps（$pinCount 个钉子包）"
  $argv = @(
    "-m", "pip", "install", "--disable-pip-version-check", "--no-warn-script-location",
    "--no-cache-dir", "--timeout", "60", "--retries", "10",
    "--target", $deps, "-r", $lock
  )
  if ($PipIndex) { $argv += @("-i", $PipIndex) }
  if ($Proxy) { $argv += @("--proxy", $Proxy) }
  Invoke-Exe $python $argv "pip install（CPU 依赖）"
  }

  # 钉子校验：装出来的必须与 lock 逐行一致，否则「可复现」是假的
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $frozen = & $python -m pip freeze --path $deps --disable-pip-version-check 2>$null
  $ErrorActionPreference = $prevEap
  $want = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' } | Sort-Object)
  $got = @($frozen | Where-Object { $_ } | Sort-Object)
  $diff = Compare-Object $want $got
  if ($diff) {
    $diff | Select-Object -First 10 | ForEach-Object { Log "  钉子差异 $($_.SideIndicator) $($_.InputObject)" }
    throw "构建出的 deps 与 lock 不一致（见上），拒绝打包"
  }
  Log "钉子校验通过：$($got.Count) 个包与 lock 完全一致"

  $modelDst = Join-Path $work "models\paddleocr"
  New-Item -ItemType Directory -Force -Path $modelDst | Out-Null
  if (-not (Test-Path $ModelSource)) {
    throw "找不到模型来源 $ModelSource（先在开发 runtime 里跑一次 bootstrap_ocr.ps1）"
  }
  Log "拷贝 OCR 模型：$ModelSource → models/paddleocr"
  Copy-Item (Join-Path $ModelSource "*") $modelDst -Recurse -Force

  $workerDst = Join-Path $work "worker"
  New-Item -ItemType Directory -Force -Path $workerDst | Out-Null
  Copy-Item (Join-Path $scriptDir "ocr_worker.py") (Join-Path $workerDst "ocr_worker.py") -Force
  Log "worker 就绪：worker/ocr_worker.py"
}

# ─── 2b. GPU 增量：deps_gpu（严格照 bootstrap_ocr_gpu.ps1 的三条约束） ────
function Build-Gpu([string]$work, [string]$python) {
  $gpuDeps = Join-Path $work "deps_gpu"
  $cudaLibs = @(
    "nvidia-cublas-cu12==12.6.4.1",
    "nvidia-cuda-runtime-cu12==12.6.77",
    "nvidia-cufft-cu12==11.3.0.4",
    "nvidia-curand-cu12==10.3.7.77",
    "nvidia-cusolver-cu12==11.7.1.2",
    "nvidia-cusparse-cu12==12.5.4.2",
    "nvidia-nvjitlink-cu12==12.9.86"
  )
  $cudnnPin = "nvidia-cudnn-cu12==9.9.0.52"
  $paddleIndex = "https://www.paddlepaddle.org.cn/packages/stable/cu126/"
  $pypiIndex = if ($PipIndex) { $PipIndex } else { "https://pypi.org/simple" }

  New-Item -ItemType Directory -Force -Path $gpuDeps | Out-Null
  Log "pip install --target deps_gpu paddlepaddle-gpu + $($cudaLibs.Count) 个 CUDA 运行库（约 1.5GB）"
  # --no-deps：只装 GPU 专有件，纯 Python 依赖一律复用 cpu 基线（否则会覆盖 deps 的基准版本）
  $argv = @(
    "-m", "pip", "install", "--upgrade", "--no-deps", "--no-cache-dir",
    "--no-warn-script-location", "--timeout", "60", "--retries", "10",
    "--target", $gpuDeps, "-i", $paddleIndex, "paddlepaddle-gpu==3.2.2"
  ) + $cudaLibs
  if ($Proxy) { $argv += @("--proxy", $Proxy) }
  Invoke-Exe $python $argv "pip install（paddlepaddle-gpu + CUDA 运行库）"

  # cuDNN 必须单独装到临时目录再拷入：pip 的卸载逻辑会清掉共享的 nvidia/ 命名空间
  $stage = Join-Path $env:TEMP "gsa_ocr_cudnn_stage"
  if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
  New-Item -ItemType Directory -Force -Path $stage | Out-Null
  Log "pip install $cudnnPin 到临时目录（约 770MB）"
  $argv = @(
    "-m", "pip", "install", "--no-deps", "--no-cache-dir", "--no-warn-script-location",
    "--timeout", "60", "--retries", "10", "--target", $stage, "-i", $pypiIndex, $cudnnPin
  )
  if ($Proxy) { $argv += @("--proxy", $Proxy) }
  try {
    Invoke-Exe $python $argv "pip install（cuDNN）"
    $staged = Join-Path $stage "nvidia\cudnn"
    if (-not (Test-Path $staged)) { throw "临时目录里没有 nvidia\cudnn（轮子结构异常）" }
    $nvidiaDir = Join-Path $gpuDeps "nvidia"
    New-Item -ItemType Directory -Force -Path $nvidiaDir | Out-Null
    Copy-Item $staged (Join-Path $nvidiaDir "cudnn") -Recurse -Force
    Get-ChildItem $stage -Directory -Filter "nvidia_cudnn_cu12-*" |
      ForEach-Object { Copy-Item $_.FullName $gpuDeps -Recurse -Force }
  } finally {
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
  }

  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $frozen = & $python -m pip freeze --path $gpuDeps --disable-pip-version-check 2>$null
  $ErrorActionPreference = $prevEap
  $lock = Join-Path $scriptDir "ocr-gpu-requirements.lock.txt"
  $want = @(Get-Content -Encoding UTF8 $lock | Where-Object { $_ -and $_ -notmatch '^#' } | Sort-Object)
  $got = @($frozen | Where-Object { $_ } | Sort-Object)
  $diff = Compare-Object $want $got
  if ($diff) {
    $diff | Select-Object -First 10 | ForEach-Object { Log "  钉子差异 $($_.SideIndicator) $($_.InputObject)" }
    throw "构建出的 deps_gpu 与 lock 不一致（见上），拒绝打包"
  }
  Log "钉子校验通过：$($got.Count) 个包与 GPU lock 完全一致"
}

# ─── 3. 冒烟：用真 worker 跑一张真图，挡住「装好了却用不了」 ──────────────
function Invoke-Smoke([string]$work, [string]$python, [string]$flavor, [string]$baseline = "") {
  Add-Type -AssemblyName System.Drawing
  # 合成一张带文字的白底图（不依赖仓库里的素材，任何机器都能跑）
  $bmp = New-Object System.Drawing.Bitmap 900, 220
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.Clear([System.Drawing.Color]::White)
  $font = New-Object System.Drawing.Font("Arial", 48, [System.Drawing.FontStyle]::Bold)
  $g.DrawString("GSA OCR SMOKE 2026", $font, [System.Drawing.Brushes]::Black, 20, 40)
  $g.Dispose()
  $jpg = Join-Path $env:TEMP "gsa_ocr_smoke.jpg"
  $bmp.Save($jpg, [System.Drawing.Imaging.ImageFormat]::Jpeg)
  $bmp.Dispose()
  $b64 = [Convert]::ToBase64String([System.IO.File]::ReadAllBytes($jpg))
  Remove-Item $jpg -Force

  $env:PYTHONPATH = if ($flavor -eq "cuda") {
    # cuda 是增量包，deps 基线来自 cpu 树（或 -CpuBaseline 指向的开发机 runtime）
    $depsBase = if ($baseline) { Join-Path $baseline "deps" } else { Join-Path $work "deps" }
    "$(Join-Path $work 'deps_gpu');$depsBase"
  } else { Join-Path $work "deps" }
  $env:PADDLE_PDX_CACHE_HOME = if ($baseline -and $flavor -eq "cuda") {
    Join-Path $baseline "models\paddleocr"
  } else { Join-Path $work "models\paddleocr" }
  $env:PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK = "True"
  $env:GSA_OCR_MODEL = "mobile"
  $env:GSA_OCR_DEVICE = if ($flavor -eq "cuda") { "gpu:0" } else { "cpu" }

  $req = @(
    (@{ id = 1; cmd = "ping" } | ConvertTo-Json -Compress),
    (@{ id = 2; images = @($b64) } | ConvertTo-Json -Compress),
    (@{ id = 3; cmd = "shutdown" } | ConvertTo-Json -Compress)
  ) -join "`n"
  # cuda 是增量包：work 树里只有 deps_gpu，没有 worker/ —— 退回基线 runtime 的 worker
  $worker = Join-Path $work "worker\ocr_worker.py"
  if (-not (Test-Path $worker)) { $worker = Join-Path $baseline "worker\ocr_worker.py" }
  if (-not (Test-Path $worker)) { throw "冒烟失败：找不到 ocr_worker.py（work 与 baseline 都没有）" }
  $sw = [Diagnostics.Stopwatch]::StartNew()
  # 原生 stderr 在 PS 5.1 下会被包装成 ErrorRecord（PaddleOCR 启动时必打一行
  # "Connectivity check ... skipped"），2>&1 会让脚本在 $ErrorActionPreference=Stop 下直接中断。
  # 所以 stderr 单独落文件，stdout 才是 JSONL 协议流。
  $errLog = Join-Path $work "smoke.stderr.log"
  $smokeEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $out = $req | & $python $worker 2>$errLog
  $code = $LASTEXITCODE
  $ErrorActionPreference = $smokeEap
  $sw.Stop()
  $lines = @($out | Where-Object { $_ -is [string] -and $_.TrimStart().StartsWith("{") })
  if ($code -ne 0) {
    $out | Select-Object -Last 10 | ForEach-Object { Log "  $_" }
    if (Test-Path $errLog) { Get-Content $errLog -TotalCount 20 | ForEach-Object { Log "  [stderr] $_" } }
    throw "冒烟失败：worker 退出码 $code（worker=$worker）"
  }
  $pong = $lines | Where-Object { $_ -match '"cmd"\s*:\s*"pong"' }
  if (-not $pong) { $out | Select-Object -Last 10 | ForEach-Object { Log "  $_" }; throw "冒烟失败：没有 pong" }
  $resLine = $lines | Where-Object { $_ -match '"results"' } | Select-Object -First 1
  if (-not $resLine) { $out | Select-Object -Last 10 | ForEach-Object { Log "  $_" }; throw "冒烟失败：没有 results 响应" }
  $res = $resLine | ConvertFrom-Json
  if (-not $res.ok) { throw "冒烟失败：worker 报错 $($res.error)" }
  $text = ($res.results | ForEach-Object { $_.text }) -join " "
  if (-not $text.Trim()) { throw "冒烟失败：OCR 没有识别出任何文字" }
  Log "冒烟通过（$flavor，$([math]::Round($sw.Elapsed.TotalSeconds,1))s）：识别到「$text」"
}

# ─── 4. 打包 ─────────────────────────────────────────────────────────────
# 条目数核对：磁盘写满时 tar.exe 只打一行 "Write error" 就退出，
# 但外层若没抓到（例如 7z/tar 版本差异）就会留下一个能被 sha256 计算、能上传、
# 解压时才炸的截断包 —— 所以建包后必须点一次条目数。
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
  # 列表时临时放宽 EAP 并吞掉原生 stderr 的包装错误：坏包的 stderr 会被 PS 5.1
  # 包成 NativeCommandError，那样就变成"抛异常"而不是我们能读懂的错误信息
  $eap = $ErrorActionPreference
  $ErrorActionPreference = "SilentlyContinue"
  $inZipRaw = @()
  try { $inZipRaw = @(tar.exe -tf $zip) } catch { }
  $tarCode = $LASTEXITCODE
  $ErrorActionPreference = $eap
  # 截断的 zip 未必会少条目（tar 能顺着本地头把名字读完再报错），
  # 所以还要看 tar 自己的退出码：非 0 说明包本身读不完整
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
  $srcFiles = $counts.Files
  $srcBytes = $counts.Bytes

  $bytes = $srcBytes
  $size = (Get-Item $zip).Length
  $sha = Get-Sha256 $zip
  Log ("完成：{0} / {1:N1} MB（解压 {2:N1} MB，压缩比 {3:N2}x，{4:N1}s，{5} 个文件）" -f `
    $label, ($size/1MB), ($bytes/1MB), ($bytes/$size), $sw.Elapsed.TotalSeconds, $srcFiles)
  Log "  size=$size"
  Log "  sha256=$sha"
  Log "  unpacked_size=$bytes"
  return [pscustomobject]@{ Zip = $zip; Label = $label; Size = $size; Sha256 = $sha; Unpacked = $bytes; Files = $srcFiles }
}

function New-Package([string]$work, [string]$flavor, [string[]]$topDirs) {
  $stamp = Join-Path $work ".ocr-runtime"
  $meta = @(
    "version=$OcrRuntimeVersion",
    "flavor=$flavor",
    "python=$PythonVersion",
    "built_at=$((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'))"
  ) -join "`n"
  [System.IO.File]::WriteAllText($stamp, $meta, (New-Object System.Text.UTF8Encoding($false)))

  # cpu 包：整个工作树一次打走
  if ($flavor -eq "cpu") {
    return @(New-Zip (Join-Path $OutDir "ocr-runtime-cpu.zip") $work @(".") "ocr-runtime-cpu")
  }

  # cuda 增量：GitHub 单个 release 资产硬上限 2 GiB（超了 gh 直接 422
  # "size must be less than 2147483648"），而 deps_gpu 里 nvidia/ 一家就 2.7 GB。
  # 所以按「paddle 本体」与「nvidia 运行库」拆成两个资产 —— 两者都必须在清单里列出、
  # 由安装器一起下载，缺一个 OCR 就跑不起来（缺 .pyd 或缺 .dll 都在 import 时炸）。
  $paddleDist = @(Get-ChildItem (Join-Path $work "deps_gpu") -Directory -Filter "paddlepaddle_gpu-*.dist-info")
  if ($paddleDist.Count -ne 1) {
    throw "deps_gpu 下应恰有一个 paddlepaddle_gpu-*.dist-info，实际 $($paddleDist.Count) 个"
  }
  $nvidiaDists = @(Get-ChildItem (Join-Path $work "deps_gpu") -Directory -Filter "nvidia_*.dist-info" | ForEach-Object { "deps_gpu/$($_.Name)" })

  $paddlePart = @("deps_gpu/paddle", "deps_gpu/$($paddleDist[0].Name)") +
                @(if (Test-Path (Join-Path $work "deps_gpu/bin")) { "deps_gpu/bin" }) +
                @(".ocr-runtime")
  $libsPart = @("deps_gpu/nvidia") + $nvidiaDists + @(".ocr-runtime")

  return @(
    (New-Zip (Join-Path $OutDir "ocr-runtime-cuda-paddle.zip") $work $paddlePart "ocr-runtime-cuda-paddle"),
    (New-Zip (Join-Path $OutDir "ocr-runtime-cuda-libs.zip") $work $libsPart "ocr-runtime-cuda-libs")
  )
}

# ─── 主流程 ───────────────────────────────────────────────────────────────
# 目录布局（两个 flavor 各用一个工作根，避免互相污染）：
#   <WorkDir>\cpu   python/ deps/ models/ worker/       ← cpu 包的全部内容
#   <WorkDir>\cuda  deps_gpu/                            ← 只是增量
# cuda 的冒烟需要 cpu 基线（PYTHONPATH=deps_gpu;deps）：优先用本次构建的 cpu 树，
# 没有再退回开发机 runtime/deps（-CpuBaseline）。
$flavors = if ($Flavor -eq "all") { @("cpu", "cuda") } else { @($Flavor) }
$results = @()
$cpuWork = Join-Path $WorkDir "cpu"

function Get-CpuBaseline() {
  if (Test-Path (Join-Path $cpuWork "deps")) { return $cpuWork }
  if ($CpuBaseline -and (Test-Path (Join-Path $CpuBaseline "deps"))) { return $CpuBaseline }
  throw "cuda 冒烟需要一份 deps 基线：先用 -Flavor cpu 构建，或用 -CpuBaseline 指向开发机 runtime"
}

if ($flavors -contains "cpu") {
  Log "──────── 构建 cpu ────────"
  Reset-WorkDir $cpuWork
  $python = Install-Python $cpuWork
  Build-Cpu $cpuWork $python
  if (-not $SkipSmoke) { Invoke-Smoke $cpuWork $python "cpu" }
  $results += New-Package $cpuWork "cpu" @("python", "deps", "models", "worker", ".ocr-runtime")
}

if ($flavors -contains "cuda") {
  Log "──────── 构建 cuda（增量）────────"
  $cudaWork = Join-Path $WorkDir "cuda"
  Reset-WorkDir $cudaWork
  try {
    $python = Install-Python $cudaWork
    Build-Gpu $cudaWork $python
    if (-not $SkipSmoke) {
      $baseline = Get-CpuBaseline
      Invoke-Smoke $cudaWork $python "cuda" $baseline
    }
    # cuda 返回两个包（paddle 本体 + nvidia 运行库），都进清单
    $results += New-Package $cudaWork "cuda" @("deps_gpu", ".ocr-runtime")
  } finally {
    if (-not $KeepWork -and ((Split-Path $cudaWork -Leaf) -like "cpu" -or (Split-Path $cudaWork -Leaf) -like "cuda")) {
      Remove-Item -Recurse -Force $cudaWork -ErrorAction SilentlyContinue
    }
  }
}

if (-not $KeepWork -and (Test-Path $cpuWork) -and $env:GSA_KEEP_CPU_WORK -ne "1") {
  # cpu 树留给 cuda 复用；单独构建 cpu 时删掉省空间
  if (-not ($flavors -contains "cuda")) { Remove-Item -Recurse -Force $cpuWork -ErrorAction SilentlyContinue }
}

Log "──────── 结果 ────────"
# 每项形如 { Zip, Label, Size, Sha256, Unpacked, Files }；cuda 会出两条
$results | ForEach-Object {
  "{0}: {1:N1} MB  unpacked={2:N1} MB  sha256={3}" -f $_.Label, ($_.Size/1MB), ($_.Unpacked/1MB), $_.Sha256
  "   $($_.Zip)"
}
