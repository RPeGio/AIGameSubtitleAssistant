<#
bootstrap_ocr.ps1 —— 搭建开发期 OCR 运行环境（复用系统 Python）
1. 建 runtime/{deps, worker, models/paddleocr}
2. pip install --target runtime/deps -r scripts/ocr-requirements.txt
3. 拷贝 scripts/ocr_worker.py -> runtime/worker/
4. 写 runtime/config.json（相对路径，由 Rust 解析）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr.ps1
#>
$ErrorActionPreference = "Stop"
$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"

$dirs = @(
  (Join-Path $runtime "deps"),
  (Join-Path $runtime "worker"),
  (Join-Path $runtime "models\paddleocr")
)
foreach ($d in $dirs) {
  New-Item -ItemType Directory -Force -Path $d | Out-Null
}

Write-Host "==> 安装 OCR 依赖到 runtime\deps ..."
python -m pip install --target (Join-Path $runtime "deps") -r (Join-Path $scriptDir "ocr-requirements.txt")
if ($LASTEXITCODE -ne 0) { throw "pip install 失败" }

Write-Host "==> 拷贝 worker 脚本 ..."
Copy-Item (Join-Path $scriptDir "ocr_worker.py") (Join-Path $runtime "worker\ocr_worker.py") -Force

$pythonPath = (Get-Command python).Source
$config = [ordered]@{
  python_path   = $pythonPath
  worker_script = "worker/ocr_worker.py"
  deps_dir      = "deps"
  model_dir     = "models/paddleocr"
  language      = "ch"
}
$config | ConvertTo-Json | Set-Content -Path (Join-Path $runtime "config.json") -Encoding UTF8

Write-Host "==> OCR 运行环境就绪：$runtime"
