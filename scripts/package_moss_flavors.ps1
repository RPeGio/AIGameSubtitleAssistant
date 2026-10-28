<#
package_moss_flavors.ps1 —— 把各 flavor 的 MOSS 产物打成可分发的 zip，并生成 manifest 条目

用法：
  powershell -ExecutionPolicy Bypass -File scripts\package_moss_flavors.ps1 `
    -NvidiaRuntimeDir "<含 cudart64_12.dll / cublas64_12.dll / cublasLt64_12.dll 的目录>" `
    -UrlBase "https://github.com/<owner>/<repo>/releases/download/<tag>/" `
    -SourceDir "<pin 到同一 commit 的源码树>" `
    -OutDir "<放 zip 的目录>"

产出（与 docs/RUNTIME_BOOTSTRAP_CONTRACT.md §4 的 manifest schema 对齐）：
  <OutDir>\moss-cpu.zip / moss-vulkan.zip / moss-cuda.zip
  <OutDir>\manifest.json      —— items[] 三/两条目，dest 一律 "bin"（契约 §4.2）
  <OutDir>\packages.json      —— 构建记录（非分发物）：flavor → 各文件哈希 + 上游 pin

**为什么每个 zip 都自带 MSVC 运行库**（msvcp140 / vcruntime140 / vcruntime140_1 / vcomp140）：
  `dumpbin /dependents` 实测：moss-transcribe.exe 与 ggml*.dll **全部**静态导入这 4 个 DLL
  （ggml-base/ggml-cpu 还导入 vcomp140 = OpenMP）。Rust 主程序不需要它们（Rust 静态链 CRT），
  但 MOSS 二进制是 MSVC 构建：干净的 Windows 上缺任一就是 0xC0000135，
  **表现与"缺 CUDA DLL"一模一样**，用户只看到"启动失败"。
  采用微软的 app-local 部署（把 DLL 放在 exe 同目录）：免管理员权限、免安装次序，
  也比让用户去装 VC++ 运行库更可靠。故脚本会校验这 4 个文件的 Authenticode 签名主体
  含 "Microsoft"（防止把来路不明的同名 DLL 打进分发物）。

**为什么 cuda flavor 还带 NVIDIA 运行库**：ggml-cuda.dll 静态导入 cublas64_12.dll，
  后者又导入 cublasLt64_12.dll（dumpbin 实测，Windows 上无静态 cublas 可用）。
  这 3 个 DLL 合计约 608 MB，是 cuda 包体积的大头；不带就启动不了。

退出码：0 全部成功；1 参数/前置失败；2 产物校验失败（纯净性/依赖/签名）；3 打包失败。
#>
param(
  # 逗号分隔（用 -File 调用时 PowerShell 不会把 "cpu,vulkan" 绑成数组，故收字符串再拆）
  [string]$Flavors = "cpu,vulkan,cuda",
  [string]$OutDir = "",
  [string]$NvidiaRuntimeDir = "",
  [string]$MsvcRuntimeDir = "",
  [string]$UrlBase = "",
  [string]$SourceDir = "",
  [string]$SourceVersion = ""
)

$ErrorActionPreference = "Stop"
$FlavorsRaw = [string]$Flavors
# 切分到 $FlavorList 而不是复用参数变量：param([string]$Flavors) 会把变量**永久约束为字符串**，
# 之后 $Flavors = @() 会被强制转回字符串、$Flavors += $x 变成字符串拼接（"cpu,vulkan" → "cpuvulkan"，症状只有"未知 flavor"）
# 分隔符接受逗号 / 分号 / 空格 / 制表符：-File 调用时 -Flavors "cpu,vulkan" 传入的是单个字符串
$FlavorList = @()
$cur = ""
foreach ($ch in $FlavorsRaw.ToCharArray()) {
  if (@(",", ";", " ", "`t") -contains [string]$ch) { if ($cur) { $FlavorList += $cur; $cur = "" } }
  else { $cur += $ch }
}
if ($cur) { $FlavorList += $cur }
if ($FlavorList.Count -eq 0) { throw "未指定 -Flavors" }
$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
if (-not $OutDir) { $OutDir = Join-Path $root "runtime\_flavors\_packages" }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# 每个 flavor 的后端 DLL：必须有 / 必须没有（与 verify_moss_flavors.ps1 同一套判据）
$flavorSpec = @{
  cpu    = @{ must = @();                mustNot = @("ggml-cuda.dll", "ggml-vulkan.dll") }
  cuda   = @{ must = @("ggml-cuda.dll"); mustNot = @("ggml-vulkan.dll") }
  vulkan = @{ must = @("ggml-vulkan.dll"); mustNot = @("ggml-cuda.dll") }
}

$nvidiaDlls = @("cudart64_12.dll", "cublas64_12.dll", "cublasLt64_12.dll")
$msvcDlls = @("msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll", "vcomp140.dll")

function Get-Sha256([string]$path) { (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLower() }

# 找 MSVC 运行库：优先 VS 的 redist 目录，退化到 System32（本机已装 VC++ 运行库的情形）
function Resolve-MsvcRuntimeDir([string]$explicit) {
  if ($explicit) {
    if (Test-Path (Join-Path $explicit "msvcp140.dll")) { return $explicit }
    throw "-MsvcRuntimeDir 里没有 msvcp140.dll：$explicit"
  }
  $cands = @()
  if ($env:VCToolsRedistDir) { $cands += $env:VCToolsRedistDir }
  $vsRedist = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Redist\MSVC"
  if (Test-Path $vsRedist) {
    Get-ChildItem $vsRedist -Directory | Sort-Object Name -Descending | ForEach-Object {
      $cands += (Join-Path $_.FullName "x64\Microsoft.VC143.CRT")
      $cands += (Join-Path $_.FullName "x64\Microsoft.VC143.OpenMP")
    }
  }
  foreach ($c in $cands) {
    if ((Test-Path (Join-Path $c "msvcp140.dll")) -and (Test-Path (Join-Path $c "vcomp140.dll"))) { return $c }
  }
  # 退化：本机 System32（这些就是 VC++ 运行库装出来的正本，逐个验签名）
  if (Test-Path "C:\Windows\System32\msvcp140.dll") { return "C:\Windows\System32" }
  throw "找不到 MSVC 运行库（msvcp140/vcruntime140/vcomp140）。请装 VC++ 2015-2022 x64 运行库，或用 -MsvcRuntimeDir 指定"
}

# 把文件放进载荷目录：优先硬链接（省空间，同一卷），失败则复制
function Add-PayloadFile([string]$payload, [string]$src) {
  $dst = Join-Path $payload (Split-Path $src -Leaf)
  if (Test-Path $dst) { Remove-Item $dst -Force }
  try { New-Item -ItemType HardLink -Path $dst -Target $src -ErrorAction Stop | Out-Null }
  catch { Copy-Item $src $dst -Force }
}

$msvcDir = Resolve-MsvcRuntimeDir $MsvcRuntimeDir
Write-Host "==> MSVC 运行库目录：$msvcDir"
foreach ($d in $msvcDlls) {
  $p = Join-Path $msvcDir $d
  if (-not (Test-Path $p)) { throw "MSVC 运行库缺少 $d（$msvcDir）——干净机器上会 0xC0000135" }
  $sig = Get-AuthenticodeSignature $p
  if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notmatch "Microsoft") {
    throw "$d 不是有效的微软签名（status=$($sig.Status)）——拒绝打进分发物"
  }
}

if ($FlavorList -contains "cuda") {
  if (-not $NvidiaRuntimeDir) { throw "打包 cuda flavor 必须提供 -NvidiaRuntimeDir（cudart64_12 / cublas64_12 / cublasLt64_12）" }
  foreach ($d in $nvidiaDlls) {
    if (-not (Test-Path (Join-Path $NvidiaRuntimeDir $d))) { throw "-NvidiaRuntimeDir 缺少 $d：$NvidiaRuntimeDir" }
  }
}

# 上游 pin：写进包内 SOURCE.txt，让"这个二进制是哪一版源码"可追溯（manifest 不扩字段）
$sourceNote = @()
if ($SourceDir) {
  foreach ($pair in @(@("moss-transcribe.cpp", $SourceDir), @("ggml", (Join-Path $SourceDir "third_party\ggml")))) {
    if (Test-Path (Join-Path $pair[1] ".git")) {
      $sha = (& git -C $pair[1] rev-parse HEAD).Trim()
      $sourceNote += "$($pair[0]) $sha"
    }
  }
  foreach ($lic in @(@("LICENSE-moss-transcribe.txt", (Join-Path $SourceDir "LICENSE")), @("LICENSE-ggml.txt", (Join-Path $SourceDir "third_party\ggml\LICENSE")))) {
    if (Test-Path $lic[1]) { $sourceNote += "LICENSE $($lic[0]) $(Get-Sha256 $lic[1])" }
  }
}

$entries = @()
$records = @{}
$failed = @()

foreach ($flavor in $FlavorList) {
  Write-Host ""
  Write-Host "=== $flavor ==="
  try {
    if (-not $flavorSpec.ContainsKey($flavor)) { throw "未知 flavor：$flavor（可选 cpu/cuda/vulkan）" }
    $bin = Join-Path $root "runtime\_flavors\$flavor\bin"
    if (-not (Test-Path (Join-Path $bin "moss-transcribe.exe"))) { throw "缺少产物：$bin\moss-transcribe.exe（先跑 build_moss.ps1 -Backend $flavor -OutputDir runtime\_flavors\$flavor）" }

    # 1) 纯净性：.backend 标记与后端 DLL 必须自洽
    $marker = Join-Path $bin ".backend"
    if (-not (Test-Path $marker)) { throw "缺少 .backend 标记（L1 资产探测靠它判 flavor）" }
    $markerVal = (Get-Content $marker -Raw).Trim()
    if ($markerVal -ne $flavor) { throw ".backend 写的是 '$markerVal'，与目录 flavor '$flavor' 不符" }
    foreach ($f in $flavorSpec[$flavor].must) {
      if (-not (Test-Path (Join-Path $bin $f))) { throw "缺少 $f（backend=$flavor）" }
    }
    foreach ($f in $flavorSpec[$flavor].mustNot) {
      if (Test-Path (Join-Path $bin $f)) { throw "混入其它 flavor 的 $f（backend=$flavor）：会静默降级" }
    }

    # 2) 载荷目录（cuda 额外并入 NVIDIA 运行库；所有 flavor 并入 MSVC 运行库）
    $payload = Join-Path $env:TEMP ("gsa_pack_" + $flavor + "_" + [guid]::NewGuid().ToString("N").Substring(0, 6))
    New-Item -ItemType Directory -Force -Path $payload | Out-Null
    try {
      Get-ChildItem $bin -File | ForEach-Object { Add-PayloadFile $payload $_.FullName }
      foreach ($d in $msvcDlls) { Add-PayloadFile $payload (Join-Path $msvcDir $d) }
      if ($flavor -eq "cuda") { foreach ($d in $nvidiaDlls) { Add-PayloadFile $payload (Join-Path $NvidiaRuntimeDir $d) } }
      if ($sourceNote.Count -gt 0) {
        $lines = @("MOSS flavor: $flavor", "built: " + (Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ"), "") + $sourceNote
        Set-Content -Path (Join-Path $payload "SOURCE.txt") -Value $lines -Encoding UTF8
      }

      # 3) 打包（tar.exe 是 Windows 10+ 自带的 libarchive，zip 走 _a 自动按扩展名选格式）
      $zipName = "moss-$flavor.zip"
      $zipPath = Join-Path $OutDir $zipName
      if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
      $sw = [Diagnostics.Stopwatch]::StartNew()
      & tar.exe -a -c -f $zipPath -C $payload .
      if ($LASTEXITCODE -ne 0) { throw "tar 打包失败（exit=$LASTEXITCODE）" }
      $sw.Stop()

      $size = (Get-Item $zipPath).Length
      $sha = Get-Sha256 $zipPath
      $rawMB = ((Get-ChildItem $payload -File | Measure-Object Length -Sum).Sum / 1MB)
      Write-Host ("    打包完成：{0}  {1:N1} MB（展开 {2:N1} MB，{3:N0}s）" -f $zipName, ($size / 1MB), $rawMB, $sw.Elapsed.TotalSeconds)
      Write-Host ("    sha256 = {0}" -f $sha)

      $url = if ($UrlBase) { $UrlBase.TrimEnd('/') + "/" + $zipName } else { $zipName }
      $entries += [ordered]@{
        id             = "moss"
        flavor         = $flavor
        url            = $url
        sha256         = $sha
        size           = $size
        dest           = "bin"
        backend_marker = $flavor
      }
      $records[$flavor] = [ordered]@{
        zip        = $zipName
        bytes      = $size
        sha256     = $sha
        unpacked   = [int64]((Get-ChildItem $payload -File | Measure-Object Length -Sum).Sum)
        files      = @(Get-ChildItem $payload -File | Sort-Object Name | ForEach-Object {
            [ordered]@{ name = $_.Name; size = $_.Length; sha256 = (Get-Sha256 $_.FullName) }
          })
        source     = $sourceNote
      }
    } finally {
      Remove-Item -Recurse -Force $payload -ErrorAction SilentlyContinue
    }
  } catch {
    Write-Host "    ✗ $($_.Exception.Message)" -ForegroundColor Red
    $failed += $flavor
  }
}

if ($failed.Count -gt 0) {
  Write-Host ""
  Write-Host "失败的 flavor：$($failed -join ', ')" -ForegroundColor Red
  exit 2
}

# 4) manifest（契约 §4 schema；条目顺序固定 cpu → vulkan → cuda，便于 diff）
$manifest = [ordered]@{
  version      = "1"
  generated_at = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  items        = @($entries)
}
$manifestPath = Join-Path $OutDir "manifest.json"
$manifest | ConvertTo-Json -Depth 6 | Set-Content -Path $manifestPath -Encoding UTF8
Write-Host ""
Write-Host "==> manifest：$manifestPath"

$recordPath = Join-Path $OutDir "packages.json"
([ordered]@{
    generated_at   = $manifest["generated_at"]
    source_version = $SourceVersion
    flavors        = $records
  }) | ConvertTo-Json -Depth 8 | Set-Content -Path $recordPath -Encoding UTF8
Write-Host "==> 构建记录：$recordPath"

Write-Host ""
Write-Host "=== 分发包汇总 ===" -ForegroundColor Cyan
$entries | ForEach-Object {
  "{0,-7} {1,9:N1} MB  {2}" -f $_["flavor"], ($_["size"] / 1MB), $_["sha256"]
}