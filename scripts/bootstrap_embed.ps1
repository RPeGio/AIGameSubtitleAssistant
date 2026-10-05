<#
bootstrap_embed.ps1 —— 搭建多语言向量召回运行环境（onnxruntime CPU + E5 模型）

用于「向量召回替代 LLM 做跨语言对齐」路线的离线验证与后续产品化
（背景与实测见 benchmark/FUSE_VECTOR_RECALL_VALIDATION.md）。

1. 安装 Python 依赖到 runtime/deps_embed/（onnxruntime CPU + tokenizers，pip --target）
   只走 CPU：GPU 算力留给 OCR/ASR；短文本编码 CPU 已足够
2. 下载 multilingual-e5-small 的 ONNX 量化件到 runtime/models/embed/multilingual-e5-small/
   （qint8 约 113MB；SHA256 从 HF API 取官方 LFS hash 校验，API 不可达时告警跳过）
3. 合并写 runtime/config.json 的 embed_* 字段（保留既有字段）

参数：
  -Model <名>                模型目录名，默认 multilingual-e5-small
  -Repo <HF repo>            模型仓库，默认 intfloat/multilingual-e5-small
  -Variant <文件名>          ONNX 变体，默认 model_qint8_avx512_vnni.onnx
                             （可选 model_O4.onnx 224MB / model.onnx 448MB，精度略高）
  -HfMirror <前缀>           模型下载镜像，如 https://hf-mirror.com（网络受限时使用）
  -IndexUrl <URL>            PyPI 镜像，默认清华源
  -Force                     重新下载模型（覆盖已有）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_embed.ps1 -HfMirror https://hf-mirror.com
#>
param(
  [string]$Model = "multilingual-e5-small",
  [string]$Repo = "intfloat/multilingual-e5-small",
  [string]$Variant = "model_qint8_avx512_vnni.onnx",
  [string]$HfMirror = "https://hf-mirror.com",
  [string]$IndexUrl = "https://pypi.tuna.tsinghua.edu.cn/simple",
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$py = Join-Path $root "runtime\python\python.exe"
$deps = Join-Path $root "runtime\deps_embed"
$modelDir = Join-Path $root "runtime\models\embed\$Model"
$cfgPath = Join-Path $root "runtime\config.json"

if (-not (Test-Path $py)) { throw "未找到嵌入式 Python：$py" }

# ── 1. Python 依赖 ──
New-Item -ItemType Directory -Force -Path $deps | Out-Null
# %TEMP% 常在 C 盘且余量小，pip 解包大文件会 Errno 28 → 重定向到大容量盘
$tmp = Join-Path $root "runtime\_tmp_embed"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$env:TEMP = $tmp; $env:TMP = $tmp; $env:PYTHONUTF8 = "1"

$pkgs = @("onnxruntime==1.20.1", "numpy==2.5.3", "huggingface_hub==1.32.0", "tokenizers")
Write-Host "==> 安装 Python 依赖到 runtime/deps_embed（镜像 $IndexUrl）"
& $py -m pip install --no-warn-script-location --disable-pip-version-check `
  --target $deps --index-url $IndexUrl @pkgs
if ($LASTEXITCODE -ne 0) { throw "pip 安装失败（退出码 $LASTEXITCODE）" }

# ── 2. 模型文件 ──
New-Item -ItemType Directory -Force -Path $modelDir | Out-Null
$files = @($Variant, "tokenizer.json", "config.json",
           "special_tokens_map.json", "tokenizer_config.json", "sentencepiece.bpe.model")
$expect = @{}
try {
  $tree = Invoke-RestMethod -Uri "$HfMirror/api/models/$Repo/tree/main/onnx" -TimeoutSec 40
  foreach ($f in $tree) { if ($f.lfs.oid) { $expect[$f.path] = $f.lfs.oid } }
} catch {
  Write-Host "==> 警告：无法访问 HF API 获取官方 SHA256，跳过校验"
}
foreach ($f in $files) {
  $out = Join-Path $modelDir $f
  if ((Test-Path $out) -and ((Get-Item $out).Length -gt 0) -and -not $Force) {
    Write-Host "==> 已存在，跳过：$f"
  } else {
    Write-Host "==> 下载 $f"
    curl.exe -sL --fail --retry 3 --retry-delay 5 --max-time 3600 -o $out "$HfMirror/$Repo/resolve/main/onnx/$f"
    if ($LASTEXITCODE -ne 0) {
      Remove-Item -Force $out -ErrorAction SilentlyContinue
      throw "下载失败：$f"
    }
  }
  if ($expect.ContainsKey($f)) {
    $h = (Get-FileHash $out -Algorithm SHA256).Hash
    if ($h -ne $expect[$f].ToUpper()) { throw "SHA256 不匹配：$f`n  期望 $($expect[$f])`n  实测 $h" }
    Write-Host ("    SHA256 校验通过（{0:N1}MB）" -f ((Get-Item $out).Length / 1MB))
  }
}
"$Model|$Variant" | Set-Content -Encoding utf8 (Join-Path $modelDir "VARIANT.txt")

# ── 3. 合并写 runtime/config.json（保留既有字段） ──
if (Test-Path $cfgPath) {
  $cfg = Get-Content $cfgPath -Raw -Encoding UTF8 | ConvertFrom-Json
} else {
  $cfg = New-Object psobject
}
$fields = @{
  embed_model = "models/embed/$Model/$Variant"
  embed_tokenizer = "models/embed/$Model/tokenizer.json"
  embed_deps_dir = "deps_embed"
}
foreach ($k in $fields.Keys) {
  if ($cfg.PSObject.Properties.Name -contains $k) { $cfg.$k = $fields[$k] }
  else { $cfg | Add-Member -NotePropertyName $k -NotePropertyValue $fields[$k] }
}
$cfg | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $cfgPath
Write-Host "==> 已更新 runtime/config.json（embed_model / embed_tokenizer / embed_deps_dir）"
Write-Host "==> 完成。验证："
$env:PYTHONPATH = $deps
& $py -c "import onnxruntime, tokenizers, numpy; print('onnxruntime', onnxruntime.__version__); print('tokenizers', tokenizers.__version__)"
