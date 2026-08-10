<#
bootstrap_moss.ps1 —— 搭建 MOSS ASR 运行环境
1. 就位 moss-transcribe.exe：
   -BinaryUrl 提供预编译 zip（发布期）→ 下载、解压、可选 SHA256 校验
   否则使用 runtime/bin/moss-transcribe.exe（先跑 scripts/build_moss.ps1 构建）
   两者皆无 → 打印指引并退出（用户设备不需要 cmake）
2. 下载 GGUF 模型到 runtime/models/moss/，SHA256 从 HF API 取官方 LFS hash 校验
   （API 不可达时告警跳过校验）
3. 合并写 runtime/config.json（保留 OCR 等既有字段）

参数：
  -Backend <cpu|cuda|vulkan>  推理后端，默认 cpu。GPU 后端需要自构建
                              （官方暂未发布 Windows GPU 预编译包），本参数为预埋接口
  -Quant <q5_k|q8_0|q6_k|q5_0|q4_k|q4_0|f16>  模型档位，默认 q5_k（HF 官方推荐：byte-identical 且仅 619MB）
  -BinaryUrl <url>    预编译 moss-transcribe.zip 下载地址（可选）
  -BinarySha256 <hex> 预编译 zip 的 SHA256（提供时校验）
  -HfMirror <前缀>    模型下载镜像，如 https://hf-mirror.com（网络受限时使用）
  -Force              重新下载模型（覆盖已有）
用法：powershell -ExecutionPolicy Bypass -File scripts/bootstrap_moss.ps1
#>
param(
  [string]$Backend = "cpu",
  [string]$Quant = "q5_k",
  [string]$BinaryUrl = "",
  [string]$BinarySha256 = "",
  [string]$HfMirror = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$binDir = Join-Path $runtime "bin"
$modelDir = Join-Path $runtime "models\moss"
$targetExe = Join-Path $binDir "moss-transcribe.exe"

# 白名单校验，防路径/URL 注入
$Backend = $Backend.ToLower()
if ($Backend -notmatch "^(cpu|cuda|vulkan)$") {
  throw "无效的 -Backend: $Backend（可选 cpu/cuda/vulkan）"
}
# GPU 后端预埋：官方暂无 Windows GPU 预编译包，仅提示构建方式，行为不变
if ($Backend -ne "cpu") {
  Write-Host "==> 注意：-Backend $Backend 需自构建 GPU 版（scripts/build_moss.ps1 加 -DGGML_CUDA=ON / -DGGML_VULKAN=ON），或提供 -BinaryUrl 指向 GPU 版 zip"
}

# 白名单档位，防路径注入
$Quant = $Quant.ToLower()
if ($Quant -notmatch "^(q5_k|q8_0|q6_k|q5_0|q4_k|q4_0|f16)$") {
  throw "无效的 -Quant: $Quant（可选 q5_k/q8_0/q6_k/q5_0/q4_k/q4_0/f16）"
}
$modelFile = "moss-transcribe-$Quant.gguf"

New-Item -ItemType Directory -Force -Path $binDir | Out-Null
New-Item -ItemType Directory -Force -Path $modelDir | Out-Null

# ── 1. moss-transcribe.exe 就位 ──
if ($BinaryUrl) {
  Write-Host "==> 下载预编译二进制: $BinaryUrl"
  $zip = Join-Path $env:TEMP "moss-transcribe.zip"
  curl.exe -L --fail --max-time 600 -o $zip $BinaryUrl
  if ($LASTEXITCODE -ne 0) { throw "二进制下载失败（HTTP 错误）" }

  if ($BinarySha256) {
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash
    if ($hash -ne $BinarySha256.ToUpper()) { throw "二进制 SHA256 校验失败: $hash（期望 $BinarySha256）" }
    Write-Host "==> 二进制 SHA256 校验通过"
  } else {
    Write-Host "==> 未提供 -BinarySha256，跳过校验"
  }

  $exDir = Join-Path $env:TEMP "gsa_moss_bin_$PID"
  if (Test-Path $exDir) { Remove-Item -Recurse -Force $exDir }
  Expand-Archive $zip $exDir -Force
  $found = Get-ChildItem -Path $exDir -Recurse -Filter "moss-transcribe.exe" | Select-Object -First 1
  if (-not $found) { throw "预编译包中找不到 moss-transcribe.exe" }
  # 拷贝 exe 所在目录的全部文件：ggml.dll 等共享库必须与 exe 同目录
  Get-ChildItem -Path $found.Directory -File | Copy-Item -Destination $binDir -Force
  Remove-Item -Recurse -Force $exDir
  Remove-Item -Force $zip
  Write-Host "==> 二进制就位：$targetExe（含同目录 DLL）"
} elseif (-not (Test-Path $targetExe)) {
  Write-Host @"

==> 未找到 moss-transcribe.exe（$targetExe）
    用户设备无需工具链：发布后使用 -BinaryUrl 下载预编译包即可。
    当前开发阶段请先构建一次：powershell -ExecutionPolicy Bypass -File scripts/build_moss.ps1
"@
  exit 1
} else {
  Write-Host "==> 使用已有二进制：$targetExe"
}

# ── 2. 下载 GGUF 模型 ──
$modelPath = Join-Path $modelDir $modelFile
if ((Test-Path $modelPath) -and -not $Force) {
  Write-Host "==> 模型已存在，跳过下载（-Force 重新下载）"
} else {
  $base = if ($HfMirror) { $HfMirror } else { "https://huggingface.co" }
  $url = "$base/mudler/moss-transcribe.cpp-gguf/resolve/main/$modelFile"

  Write-Host "==> 下载模型 $modelFile（约 $(if ($Quant -match '^f16$') {'1.8GB'} elseif ($Quant -match '^q8_0$') {'940MB'} elseif ($Quant -match '^q6_k$') {'730MB'} elseif ($Quant -match '^q5') {'620MB'} else {'510MB'})，请耐心等待）..."
  Write-Host "    来源: $url"
  curl.exe -L --fail --retry 3 --retry-delay 5 --max-time 3600 -o $modelPath $url
  if ($LASTEXITCODE -ne 0) {
    # 删除残留的半截文件，避免下次运行时被"已存在"跳过
    Remove-Item -Force $modelPath -ErrorAction SilentlyContinue
    throw "模型下载失败（网络受限可试 -HfMirror https://hf-mirror.com）"
  }

  # 从 HF API 取官方 LFS OID（Git-LFS OID 即文件 SHA256）校验；API 不可达时告警跳过（模型已落盘）
  $sha256 = $null
  try {
    $files = Invoke-RestMethod -Uri "$base/api/models/mudler/moss-transcribe.cpp-gguf/tree/main" -TimeoutSec 30
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
foreach ($prop in @("moss_binary", "moss_model", "moss_threads")) {
  if ($cfg.PSObject.Properties.Name -contains $prop) {
    $cfg.PSObject.Properties.Remove($prop)
  }
}
$cfg | Add-Member -NotePropertyName "moss_binary"  -NotePropertyValue "bin/moss-transcribe.exe" -Force
$cfg | Add-Member -NotePropertyName "moss_model"    -NotePropertyValue "models/moss/$modelFile" -Force
$cfg | Add-Member -NotePropertyName "moss_threads"  -NotePropertyValue 0 -Force

# 无 BOM 写入（Rust 端 serde_json 解析需要）
[System.IO.File]::WriteAllText(
  $cfgPath,
  ($cfg | ConvertTo-Json -Depth 5),
  (New-Object System.Text.UTF8Encoding $false)
)
Write-Host "==> config.json 已更新（OCR 字段保留）"

# ── 4. 就绪自检 ──
# 日志走 stderr；PS 5.1 在 ErrorActionPreference=Stop 时会把原生命令 stderr 抛成
# NativeCommandError（即使已重定向），故自检段临时降级
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$infoOut = & $targetExe info $modelPath 2>$null
$code = $LASTEXITCODE
$ErrorActionPreference = $prevEap
$infoOut | Select-Object -First 8
if ($code -ne 0) {
  Write-Host "==> 警告：moss-transcribe info 自检失败（exit=$code），请检查模型与二进制"
} else {
  Write-Host "==> MOSS 运行环境就绪：$targetExe + $modelPath"
}
