<#
verify_moss_flavors.ps1 —— 校验各 flavor 的 MOSS 产物（分发入库前的检查）

用法：
  powershell -ExecutionPolicy Bypass -File scripts\verify_moss_flavors.ps1 `
    -ModelPath "<moss gguf 路径>" `
    -NvidiaRuntimeDir "<NVIDIA 运行库目录：含 cudart64_12.dll / cublas64_12.dll / cublasLt64_12.dll>"

检查项（每个 flavor：cpu / cuda / vulkan）：
  1. 产物清单：文件名 + 字节数 + SHA-256（manifest 的 sha256 就是从这里取）
  2. 后端纯净性：目标 flavor 的后端 DLL 必须在，其它 flavor 的 GPU DLL 必须不在
     （三个 flavor 的 ggml*.dll 同名互斥，混装会加载到错误后端并**静默降级**）
  3. 运行时自检：把该 flavor 复制到临时目录（干净 PATH，不受本机环境影响），
     跑 `moss-transcribe info <model>`，从 stderr 解析真实后端（`[mt I] backend:`）。
     ggml 在设备缺失时是静默降级（exit code 仍为 0），所以必须看这行而不是退出码。
  4. 依赖体检：纯净环境跑不起来时识别缺依赖（如 cuda 版缺 cublas/cudart，
     需要随包捆绑 NVIDIA 运行库）

输出：控制台人类可读摘要；-JsonOut <path> 另存 JSON（供生成 manifest / 分发清单用）。
#>
param(
  [string]$FlavorRoot = "",
  [string]$ModelPath = "",
  [string]$NvidiaRuntimeDir = "",
  [string]$JsonOut = ""
)

$ErrorActionPreference = "Stop"
$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
if (-not $FlavorRoot) { $FlavorRoot = Join-Path $root "runtime\_flavors" }
if (-not (Test-Path $FlavorRoot)) { throw "flavor 根目录不存在：$FlavorRoot" }

# 每个 flavor 的后端 DLL：必须有 / 必须没有
$flavorSpec = @{
  cpu    = @{ must = @();           mustNot = @("ggml-cuda.dll", "ggml-vulkan.dll") }
  cuda   = @{ must = @("ggml-cuda.dll");   mustNot = @("ggml-vulkan.dll") }
  vulkan = @{ must = @("ggml-vulkan.dll"); mustNot = @("ggml-cuda.dll") }
}

function Get-Sha256([string]$path) {
  (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLower()
}

# 在干净目录里跑一次 info，解析真实后端
function Test-FlavorRuntime([string]$binDir, [string]$model, [string[]]$extraDlls) {
  $tmp = Join-Path $env:TEMP ("gsa_verify_" + [guid]::NewGuid().ToString("N").Substring(0, 8))
  New-Item -ItemType Directory -Force -Path $tmp | Out-Null
  try {
    Get-ChildItem $binDir -File | ForEach-Object { Copy-Item $_.FullName (Join-Path $tmp $_.Name) -Force }
    foreach ($d in $extraDlls) {
      if (Test-Path $d) { Copy-Item $d (Join-Path $tmp (Split-Path $d -Leaf)) -Force }
    }
    $exe = Join-Path $tmp "moss-transcribe.exe"
    if (-not (Test-Path $exe)) { return @{ ok = $false; backend = ""; exit_code = -1; detail = "缺少 moss-transcribe.exe" } }
    $outFile = Join-Path $tmp "stdout.txt"
    $errFile = Join-Path $tmp "stderr.txt"
    # 必须用 Start-Process 重定向：PowerShell 5.1 把原生命令的 stderr 当 error record，
    # 在 ErrorActionPreference=Stop 下会直接抛出，捕获不到 ggml 日志行
    $argList = @("info")
    if ($model) { $argList += $model }
    $prevEA = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $code = -1
    try {
      $proc = Start-Process -FilePath $exe -ArgumentList $argList -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $outFile -RedirectStandardError $errFile
      $code = $proc.ExitCode
    } catch {
      $code = -1
    } finally {
      $ErrorActionPreference = $prevEA
    }
    $err = ""
    if (Test-Path $errFile) { $err = [System.IO.File]::ReadAllText($errFile) }
    $backend = ""
    if ($err -cmatch "\[mt I\] backend:\s*([^\r\n]+)") { $backend = $Matches[1].Trim() }
    # DLL 缺失：Windows 装载器返回 0xC0000135（-1073741515）
    $missingDll = ($code -eq -1073741515)
    $detail = ""
    if ($missingDll) {
      $detail = "进程无法启动（0xC0000135）：缺少依赖 DLL"
      if ($err) { $detail += "；stderr: " + (($err -split "`r?`n" | Where-Object { $_ -match '\S' } | Select-Object -First 2) -join " | ") }
    } elseif ($err) {
      $detail = (($err -split "`r?`n" | Where-Object { $_ -match '\S' } | Select-Object -First 3) -join " | ")
    }
    return @{ ok = ($code -eq 0 -and $backend -ne ""); backend = $backend; exit_code = $code; detail = $detail; missing_dll = $missingDll }
  } finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
  }
}

$report = @{ generated_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ"); flavor_root = $FlavorRoot; flavors = @() }
$problems = @()

foreach ($f in @("cpu", "cuda", "vulkan")) {
  $bin = Join-Path $FlavorRoot "$f\bin"
  Write-Host ""
  Write-Host "=== flavor: $f  ($bin) ==="
  $entry = @{ flavor = $f; dir = $bin; files = @(); purity = @(); runtime = $null }
  if (-not (Test-Path $bin)) {
    Write-Host "  [缺失] 目录不存在"
    $problems += "$f：目录不存在 $bin"
    $entry.purity += "目录不存在"
    $report.flavors += $entry
    continue
  }
  # 1) 文件清单 + 哈希
  foreach ($file in (Get-ChildItem $bin -File | Sort-Object Name)) {
    $hash = Get-Sha256 $file.FullName
    $entry.files += @{ name = $file.Name; size = $file.Length; sha256 = $hash }
    Write-Host ("  {0,-24} {1,12:N0} B  sha256={2}" -f $file.Name, $file.Length, $hash)
  }
  # 2) 纯净性
  $spec = $flavorSpec[$f]
  foreach ($must in $spec.must) {
    if (-not (Test-Path (Join-Path $bin $must))) {
      $msg = "缺少本 flavor 的后端 DLL：$must"
      Write-Host "  [纯净性] $msg"
      $entry.purity += $msg
      $problems += "$f：$msg"
    }
  }
  foreach ($mustNot in $spec.mustNot) {
    if (Test-Path (Join-Path $bin $mustNot)) {
      $msg = "混入其它 flavor 的后端 DLL：$mustNot（会加载到错误后端并静默降级）"
      Write-Host "  [纯净性] $msg"
      $entry.purity += $msg
      $problems += "$f：$msg"
    }
  }
  if ($entry.purity.Count -eq 0) { Write-Host "  [纯净性] OK" }
  # 3) 运行时自检（纯净环境）
  $rt = Test-FlavorRuntime $bin $ModelPath @()
  Write-Host ("  [自检/纯净] exit={0} backend={1} {2}" -f $rt.exit_code, $rt.backend, $rt.detail)
  if (-not $rt.ok) {
    if ($rt.missing_dll -and $f -eq "cuda" -and $NvidiaRuntimeDir) {
      # 4) 补上 NVIDIA 运行库再测（随包捆绑的情形）
      $extra = @()
      foreach ($n in @("cudart64_12.dll", "cublas64_12.dll", "cublasLt64_12.dll")) {
        $p = Join-Path $NvidiaRuntimeDir $n
        if (Test-Path $p) { $extra += $p } else { Write-Host "  [依赖] NVIDIA 运行库缺少 $n" }
      }
      $rt2 = Test-FlavorRuntime $bin $ModelPath $extra
      Write-Host ("  [自检/补依赖] exit={0} backend={1} {2}" -f $rt2.exit_code, $rt2.backend, $rt2.detail)
      $entry.runtime = $rt2
      $entry.runtime_bundled_deps = @($extra | ForEach-Object { Split-Path $_ -Leaf })
    } else {
      $entry.runtime = $rt
      $problems += "$f：运行时自检失败（exit=$($rt.exit_code)）"
    }
  } else {
    $entry.runtime = $rt
  }
  $report.flavors += $entry
}

Write-Host ""
if ($problems.Count -eq 0) {
  Write-Host "==> 全部 flavor 校验通过"
} else {
  Write-Host "==> 发现 $($problems.Count) 个问题："
  $problems | ForEach-Object { Write-Host "    - $_" }
}

if ($JsonOut) {
  $json = $report | ConvertTo-Json -Depth 8
  [System.IO.File]::WriteAllText($JsonOut, $json, (New-Object System.Text.UTF8Encoding $false))
  Write-Host "==> JSON 已写入 $JsonOut"
}

if ($problems.Count -gt 0) { exit 1 }
exit 0
