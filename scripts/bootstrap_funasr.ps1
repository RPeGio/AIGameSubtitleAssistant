<#
bootstrap_funasr.ps1 —— 搭建 FunASR ASR 运行环境（Fun-ASR-Nano + VAD + cam++ + 标点）

1. 复用 runtime/python（OCR 的内嵌解释器；缺失时提示先跑 bootstrap_ocr.ps1）
2. pip install --target runtime/deps_funasr：
   - -Device cuda：先装 CUDA 版 torch/torchaudio（pytorch 官方 index），再装 funasr/modelscope
   - -Device cpu：直接装全量（PyPI 默认 CPU 版 torch）
   - deps_funasr 独立于 OCR 的 runtime/deps，避免 torch 与 paddleocr 依赖版本冲突
3. 用 modelscope snapshot_download 预下载 4 个模型到 runtime/models/funasr
   （MODELSCOPE_CACHE 指向该目录，与 worker 运行时同缓存，避免重复下载）：
   - FunAudioLLM/Fun-ASR-Nano-2512（识别主模型，~1.6GB）
   - iic/speech_fsmn_vad_zh-cn-16k-common-pytorch（VAD 分段）
   - iic/speech_campplus_sv_zh-cn_16k-common（说话人）
   - iic/punc_ct-transformer_cn-en-common-vocab471067-large（标点，中英）
4. 拷贝 scripts/funasr_worker.py -> runtime/worker/
5. 合并写 runtime/config.json（保留 OCR/MOSS/LLM 等既有字段），asr_provider=funasr

参数：
  -Device <cuda|cpu>    推理设备，默认 cuda（torch CUDA 版体积大 ~2.5GB，无 GPU 用 cpu）
  -TorchIndexUrl <url>  CUDA 版 torch 下载源，默认 https://download.pytorch.org/whl/cu124
                        （驱动不符可换 cu121/cu126 等；国内可试 https://mirrors.aliyun.com/pytorch-wheels/cu124/）
  -PipMirror <url>      非 torch 包的 pip 镜像，如 https://mirrors.aliyun.com/pypi/simple
  -Force                重装依赖（删除 deps_funasr）并重新下载模型（覆盖已有）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_funasr.ps1
#>
param(
  [string]$Device = "cuda",
  [string]$TorchIndexUrl = "https://download.pytorch.org/whl/cu124",
  [string]$PipMirror = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$pythonExe = Join-Path $runtime "python\python.exe"
$depsDir = Join-Path $runtime "deps_funasr"
$modelCache = Join-Path $runtime "models\funasr"

# 白名单校验
$Device = $Device.ToLower()
if ($Device -notmatch "^(cuda|cpu)$") {
  throw "无效的 -Device: $Device（可选 cuda/cpu）"
}

# ── 1. 解释器检查（复用 OCR 的内嵌 Python）──
if (-not (Test-Path $pythonExe)) {
  Write-Host @"

==> 未找到内嵌 Python（$pythonExe）
    FunASR 复用 OCR 的 runtime/python 解释器，请先执行：
    powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr.ps1
"@
  exit 1
}
$ver = & $pythonExe --version 2>&1
Write-Host "==> 使用内嵌解释器: $ver"

# ── 2. 依赖（deps_funasr，与 OCR 的 deps 隔离）──
New-Item -ItemType Directory -Force -Path $depsDir | Out-Null
New-Item -ItemType Directory -Force -Path $modelCache | Out-Null

$funasrInstalled = Test-Path (Join-Path $depsDir "funasr")
if ($Force -or -not $funasrInstalled) {
  if ($Force -and (Test-Path $depsDir)) {
    Remove-Item -Recurse -Force $depsDir
    New-Item -ItemType Directory -Force -Path $depsDir | Out-Null
  }
  $basePipArgs = @("--disable-pip-version-check", "--no-warn-script-location", "--no-cache-dir", "--target", $depsDir)
  if ($PipMirror) {
    $basePipArgs += @("--index-url", $PipMirror)
  }

  if ($Device -eq "cuda") {
    # CUDA 版 torch 必须走 pytorch 官方 index（先装 torch，避免与 PyPI 的 CPU 版竞争）
    Write-Host "==> 安装 torch（CUDA）到 $depsDir ..."
    & $pythonExe -m pip install $basePipArgs `
      --index-url $TorchIndexUrl torch torchaudio
    if ($LASTEXITCODE -ne 0) { throw "torch(CUDA) 安装失败" }
  }

  Write-Host "==> 安装 funasr/modelscope 到 $depsDir ..."
  # setuptools 固定 75.x：84+ 移除了 distutils shim（Python 3.12 无 distutils，
  # funasr 部分模型模块仍依赖；worker 手动激活 _distutils_hack）
  & $pythonExe -m pip install $basePipArgs funasr modelscope "setuptools==75.3.0"
  if ($LASTEXITCODE -ne 0) { throw "funasr/modelscope 安装失败" }

  if ($Device -eq "cpu") {
    # CPU 模式：确认 torch（CPU）就位（funasr 安装可能已带上，幂等）
    Write-Host "==> 安装 torch（CPU）到 $depsDir ..."
    & $pythonExe -m pip install $basePipArgs torch torchaudio
    if ($LASTEXITCODE -ne 0) { throw "torch(CPU) 安装失败" }
  }
} else {
  Write-Host "==> deps_funasr 已存在且含 funasr，跳过安装（-Force 重装）"
}

# ── 3. 预下载模型（MODELSCOPE_CACHE 与 worker 运行时同目录，避免重复下载）──
$models = @(
  "FunAudioLLM/Fun-ASR-Nano-2512",
  "iic/speech_fsmn_vad_zh-cn-16k-common-pytorch",
  "iic/speech_campplus_sv_zh-cn_16k-common",
  "iic/punc_ct-transformer_cn-en-common-vocab471067-large"
)
$env:MODELSCOPE_CACHE = $modelCache
# modelscope 装在 deps_funasr（--target），模型下载段需把 deps 加入模块搜索路径
$env:PYTHONPATH = $depsDir
foreach ($m in $models) {
  # modelscope 1.39 缓存布局：<cache>/models/<org>--<name>/snapshots/<hash>
  $marker = Join-Path $modelCache ("models\" + ($m -replace "/", "--"))
  if ((Test-Path $marker) -and -not $Force) {
    Write-Host "==> 模型已存在，跳过下载: $m"
    continue
  }
  Write-Host "==> 下载模型 $m ..."
  & $pythonExe -c "from modelscope import snapshot_download; import sys; snapshot_download(sys.argv[1])" $m
  if ($LASTEXITCODE -ne 0) { throw "模型下载失败: $m" }
}
Remove-Item Env:\MODELSCOPE_CACHE -ErrorAction SilentlyContinue
Remove-Item Env:\PYTHONPATH -ErrorAction SilentlyContinue

# ── 4. 拷贝 worker 脚本 ──
Write-Host "==> 拷贝 worker 脚本 ..."
Copy-Item (Join-Path $scriptDir "funasr_worker.py") (Join-Path $runtime "worker\funasr_worker.py") -Force

# ── 5. 合并写 config.json（保留既有字段）──
$cfgPath = Join-Path $runtime "config.json"
if (Test-Path $cfgPath) {
  $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
} else {
  $cfg = [pscustomobject]@{}
}
foreach ($prop in @(
  "asr_provider", "funasr_worker", "funasr_deps", "funasr_model_dir",
  "funasr_device", "funasr_language", "funasr_timeout_minutes")) {
  if ($cfg.PSObject.Properties.Name -contains $prop) {
    $cfg.PSObject.Properties.Remove($prop)
  }
}
$cfg | Add-Member -NotePropertyName "asr_provider"           -NotePropertyValue "funasr" -Force
$cfg | Add-Member -NotePropertyName "funasr_worker"          -NotePropertyValue "worker/funasr_worker.py" -Force
$cfg | Add-Member -NotePropertyName "funasr_deps"            -NotePropertyValue "deps_funasr" -Force
$cfg | Add-Member -NotePropertyName "funasr_model_dir"       -NotePropertyValue "models/funasr" -Force
$cfg | Add-Member -NotePropertyName "funasr_device"          -NotePropertyValue $Device -Force
$cfg | Add-Member -NotePropertyName "funasr_language"        -NotePropertyValue "中文" -Force
$cfg | Add-Member -NotePropertyName "funasr_timeout_minutes" -NotePropertyValue 0 -Force

# 无 BOM 写入（Rust 端 serde_json 解析需要）
[System.IO.File]::WriteAllText(
  $cfgPath,
  ($cfg | ConvertTo-Json -Depth 5),
  (New-Object System.Text.UTF8Encoding $false)
)
Write-Host "==> config.json 已更新（asr_provider=funasr，OCR/MOSS/LLM 字段保留）"

Write-Host "==> FunASR 运行环境就绪：$depsDir + $modelCache"
