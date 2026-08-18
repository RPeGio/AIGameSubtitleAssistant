<#
bootstrap_llm.ps1 —— 搭建本地 LLM 运行环境（llama.cpp + Qwen2.5-3B）
1. 下载 llama.cpp 官方预编译 release zip 到 runtime/bin/llm/（与 moss 分目录：
   两者的 ggml.dll 等共享库同名且互相不兼容，DLL 从 exe 所在目录优先加载）
   -Backend cpu  → llama-<ver>-bin-win-cpu-x64.zip
   -Backend cuda → llama-<ver>-bin-win-cuda-12.4-x64.zip（NVIDIA 显卡）
   -Backend vulkan → llama-<ver>-bin-win-vulkan-x64.zip（AMD/Intel 显卡）
2. 下载 Qwen2.5-3B-Instruct GGUF 到 runtime/models/qwen/，
   SHA256 从 HF API 取官方 LFS hash 校验（API 不可达时告警跳过）
3. 合并写 runtime/config.json（保留 OCR/MOSS 等既有字段）

参数：
  -Backend <cpu|cuda|vulkan>  推理后端，默认 cpu
  -Version <bXXXX>            llama.cpp release 版本号，默认 b10333
  -BinarySha256 <hex>         预编译 zip 的 SHA256（提供时校验）
  -GitMirror <前缀>           二进制下载镜像（完整 GitHub URL 前缀），如 https://gh-proxy.com/（GitHub 直连慢时使用）
  -HfMirror <前缀>            模型下载镜像，如 https://hf-mirror.com（网络受限时使用）
  -Force                      重新下载二进制与模型（覆盖已有）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_llm.ps1 -Backend cuda
#>
param(
  [string]$Backend = "cpu",
  [string]$Version = "b10333",
  [string]$BinarySha256 = "",
  [string]$GitMirror = "",
  [string]$HfMirror = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$llmDir = Join-Path $runtime "bin\llm"
$targetExe = Join-Path $llmDir "llama-cli.exe"
$modelDir = Join-Path $runtime "models\qwen"
$modelFile = "qwen2.5-3b-instruct-q4_k_m.gguf"
$modelPath = Join-Path $modelDir $modelFile

# 白名单校验，防路径/URL 注入
$Backend = $Backend.ToLower()
if ($Backend -notmatch "^(cpu|cuda|vulkan)$") {
  throw "无效的 -Backend: $Backend（可选 cpu/cuda/vulkan）"
}
if ($Version -notmatch "^b\d+$") {
  throw "无效的 -Version: $Version（格式如 b10333）"
}

$asset = switch ($Backend) {
  "cpu"    { "llama-$Version-bin-win-cpu-x64.zip" }
  "cuda"   { "llama-$Version-bin-win-cuda-12.4-x64.zip" }
  "vulkan" { "llama-$Version-bin-win-vulkan-x64.zip" }
}

New-Item -ItemType Directory -Force -Path $llmDir | Out-Null
New-Item -ItemType Directory -Force -Path $modelDir | Out-Null

# ── 1. llama-cli.exe 就位 ──
$haveBinary = $false
if (Test-Path $targetExe) {
  if (-not $Force) {
    Write-Host "==> 已有 $targetExe，跳过下载（-Force 重新下载；切换后端时请先删除或 -Force）"
    $haveBinary = $true
  } else {
    Write-Host "==> -Force：覆盖已有二进制"
  }
}

if (-not $haveBinary) {
  # GitMirror 是完整 URL 前缀（如 gh-proxy.com/ 需拼 https://github.com/ 完整地址）
  $fullUrl = "https://github.com/ggml-org/llama.cpp/releases/download/$Version/$asset"
  $zipUrl = if ($GitMirror) { "$GitMirror$fullUrl" } else { $fullUrl }
  Write-Host "==> 下载 llama.cpp 预编译包（Backend=$Backend）: $zipUrl"
  $zip = Join-Path $env:TEMP "llama-$Version-$Backend.zip"
  # --no-progress-meter：隐藏进度条刷屏，出错时仍输出错误
  # 不加 -C -：部分代理镜像（如 gh-proxy.com）对 Range 请求返回 403，且 --retry 已覆盖网络抖动
  curl.exe -sS --no-progress-meter -L --fail --retry 3 --retry-delay 5 --max-time 3600 -o $zip $zipUrl
  if ($LASTEXITCODE -ne 0) { throw "llama.cpp 下载失败（GitHub 直连慢可试 -GitMirror 镜像参数）" }

  if ($BinarySha256) {
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash
    if ($hash -ne $BinarySha256.ToUpper()) { throw "二进制 SHA256 校验失败: $hash（期望 $BinarySha256）" }
    Write-Host "==> 二进制 SHA256 校验通过"
  } else {
    Write-Host "==> 未提供 -BinarySha256，跳过校验"
  }

  # 解压到临时目录，避免部分失败污染 bin/llm
  $exDir = Join-Path $env:TEMP "gsa_llm_bin_$PID"
  if (Test-Path $exDir) { Remove-Item -Recurse -Force $exDir }
  Expand-Archive $zip $exDir -Force
  $found = Get-ChildItem -Path $exDir -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
  if (-not $found) { throw "预编译包中找不到 llama-cli.exe" }
  # 拷贝 exe 所在目录的全部文件：ggml/llama 共享库必须与 exe 同目录
  Get-ChildItem -Path $found.Directory -File | Copy-Item -Destination $llmDir -Force
  Remove-Item -Recurse -Force $exDir
  Remove-Item -Force $zip
  Write-Host "==> 二进制就位：$targetExe（含同目录 DLL）"
}

# ── 1.5 CUDA runtime（仅 cuda 后端）──
# cuda 版预编译包是 lean 版：不含 cudart64_12.dll / cublas64_12.dll / cublasLt64_12.dll，
# 缺任一都会导致 ggml-cuda.dll 加载失败（--list-devices 显示 none）
$cudartDlls = @("cudart64_12.dll", "cublas64_12.dll", "cublasLt64_12.dll")
$cudartMissing = @($cudartDlls | Where-Object { -not (Test-Path (Join-Path $llmDir $_)) })
if ($Backend -eq "cuda" -and ($cudartMissing.Count -gt 0 -or $Force)) {
  $cudartAsset = "cudart-llama-bin-win-cuda-12.4-x64.zip"
  $cudartUrl = "https://github.com/ggml-org/llama.cpp/releases/download/$Version/$cudartAsset"
  if ($GitMirror) { $cudartUrl = "$GitMirror$cudartUrl" }
  Write-Host "==> 下载 CUDA runtime（cudart + cuBLAS，约 580MB）: $cudartUrl"
  $czip = Join-Path $env:TEMP "cudart-$Version.zip"
  curl.exe -sS --no-progress-meter -L --fail --retry 3 --retry-delay 5 --max-time 3600 -o $czip $cudartUrl
  if ($LASTEXITCODE -ne 0) { throw "CUDA runtime 下载失败（缺 cudart/cuBLAS 时 CUDA 后端无法工作）" }
  $cexDir = Join-Path $env:TEMP "gsa_cudart_$PID"
  if (Test-Path $cexDir) { Remove-Item -Recurse -Force $cexDir }
  Expand-Archive $czip $cexDir -Force
  Get-ChildItem -Path $cexDir -Filter "*.dll" | Copy-Item -Destination $llmDir -Force
  Remove-Item -Recurse -Force $cexDir
  Remove-Item -Force $czip
  Write-Host "==> CUDA runtime 就位（cudart64_12.dll / cublas64_12.dll / cublasLt64_12.dll）"
} elseif ($Backend -eq "cuda") {
  Write-Host "==> CUDA runtime 已存在，跳过下载"
}

# ── 2. 下载 GGUF 模型 ──
if ((Test-Path $modelPath) -and -not $Force) {
  Write-Host "==> 模型已存在，跳过下载（-Force 重新下载）"
} else {
  $base = if ($HfMirror) { $HfMirror } else { "https://huggingface.co" }
  $url = "$base/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/$modelFile"

  Write-Host "==> 下载模型 $modelFile（约 1.9GB，请耐心等待）..."
  Write-Host "    来源: $url"
  curl.exe -L --fail --retry 3 --retry-delay 5 --max-time 3600 -o $modelPath $url
  if ($LASTEXITCODE -ne 0) {
    # 删除残留的半截文件，避免下次运行时被"已存在"跳过
    Remove-Item -Force $modelPath -ErrorAction SilentlyContinue
    throw "模型下载失败（网络受限可试 -HfMirror https://hf-mirror.com）"
  }

  # 从 HF API 取官方 LFS OID（Git-LFS OID 即文件 SHA256）校验；API 不可达时告警跳过
  $sha256 = $null
  try {
    $files = Invoke-RestMethod -Uri "$base/api/models/Qwen/Qwen2.5-3B-Instruct-GGUF/tree/main" -TimeoutSec 30
    foreach ($f in $files) {
      if ($f.path -eq $modelFile -and $f.lfs.oid) { $sha256 = $f.lfs.oid; break }
    }
  } catch {
    Write-Host "==> 警告：无法访问 HF API 获取官方 SHA256，跳过校验"
  }
  if ($sha256) {
    $hash = (Get-FileHash $modelPath -Algorithm SHA256).Hash
    if ($hash -ne $sha256.ToUpper()) {
      Remove-Item -Force $modelPath
      throw "模型 SHA256 校验失败: $hash（期望 $sha256），已删除损坏文件"
    }
    Write-Host "==> 模型 SHA256 校验通过"
  }
}

# ── 3. 合并写 config.json ──
$cfgPath = Join-Path $runtime "config.json"
if (Test-Path $cfgPath) {
  $cfg = Get-Content $cfgPath -Raw | ConvertFrom-Json
} else {
  $cfg = [pscustomobject]@{}
}
# 幂等：先删旧属性再加，避免 Add-Member 重复添加报错
foreach ($prop in @("llm_binary", "llm_model", "llm_threads")) {
  if ($cfg.PSObject.Properties.Name -contains $prop) {
    $cfg.PSObject.Properties.Remove($prop)
  }
}
$cfg | Add-Member -NotePropertyName "llm_binary"  -NotePropertyValue "bin/llm/llama-cli.exe" -Force
$cfg | Add-Member -NotePropertyName "llm_model"    -NotePropertyValue "models/qwen/$modelFile" -Force
$cfg | Add-Member -NotePropertyName "llm_threads"  -NotePropertyValue 0 -Force

# 无 BOM 写入（Rust 端 serde_json 解析需要）
[System.IO.File]::WriteAllText(
  $cfgPath,
  ($cfg | ConvertTo-Json -Depth 5),
  (New-Object System.Text.UTF8Encoding $false)
)
Write-Host "==> config.json 已更新（OCR/MOSS 字段保留）"

# ── 4. 就绪自检 ──
# 日志走 stderr；PS 5.1 在 ErrorActionPreference=Stop 时会把原生命令 stderr 抛成
# NativeCommandError（即使已重定向），故自检段临时降级
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "Continue"

if ($Backend -eq "cpu") {
  $out = & $targetExe --version 2>&1
  $code = $LASTEXITCODE
  $out | Select-Object -First 3
  if ($code -eq 0) {
    Write-Host "==> LLM 运行环境就绪：$targetExe + $modelPath（Backend=cpu）"
  } else {
    Write-Host "==> 警告：llama-cli --version 自检失败（exit=$code）"
  }
} else {
  # GPU 后端：列出设备确认对应后端已加载
  $out = & $targetExe --list-devices 2>&1
  $code = $LASTEXITCODE
  $joined = ($out -join "`n")
  $device = if ($Backend -eq "cuda") { "CUDA0" } else { "Vulkan0" }
  if ($code -eq 0 -and $joined -match $device) {
    Write-Host "==> LLM 运行环境就绪：$targetExe + $modelPath（Backend=$Backend，检测到 $device）"
  } else {
    Write-Host "==> 警告：--list-devices 未见 $device（exit=$code）"
    if ($Backend -eq "cuda") {
      Write-Host "    排查：1) 若此前用其他 -Backend 装过，请加 -Force 重新下载（当前 exe 可能仍是旧后端构建）"
      Write-Host "         2) 确认 bin/llm 下存在 cudart64_12.dll / cublas64_12.dll / cublasLt64_12.dll"
      Write-Host "         3) 确认 NVIDIA 驱动已安装（nvidia-smi 可查）"
    } else {
      Write-Host "    排查：若此前用其他 -Backend 装过，请加 -Force 重新下载（当前 exe 可能仍是旧后端构建）；否则检查显卡驱动"
      Write-Host "    --list-devices 输出："
    }
    $out | Select-Object -First 6
  }
}

$ErrorActionPreference = $prevEap
