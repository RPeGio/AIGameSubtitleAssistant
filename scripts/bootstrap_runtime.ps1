<#
bootstrap_runtime.ps1 —— 按 manifest 安装 runtime（多条目 / 多 flavor 的通用安装器）

本期条目：
  - moss         按 flavor 拆成 cpu / vulkan / cuda 三条（二进制 + ggml 后端）
  - models-moss  不带 flavor 的一条（gguf 模型，三种后端共用同一份）
脚本按「多条目 + 多 flavor」写：后续 deps-ocr / deps-embed 只加 manifest 条目，
不改本脚本逻辑（无 flavor 的条目直接选中，不参与设备探测）。

与前端/配置侧的接口见 docs/RUNTIME_BOOTSTRAP_CONTRACT.md：
  - stdout 只输出 JSONL 事件（start / progress / item_done / item_skip / item_fail /
    error / done），最后一行必为 done；人类可读诊断走 stderr；
  - 退出码：0 全成功 / 1 预检失败 / 2 校验失败 / 3 部分条目失败 / 4 未知错误。

参数：
  -Items <ids>          逗号分隔的条目 id，默认 moss,models-moss（二进制 + 模型缺一不可）
  -Flavor <auto|cpu|cuda|vulkan>  默认 auto（L0 探测：nvidia-smi → vulkaninfo → cpu）
  -Mirror <前缀>        代理前缀（默认 gh-proxy），传空串 "" 表示直连；见契约 §4.1
  -Force                忽略已安装标记，强制重装
  -Manifest <path>      覆盖内置清单（离线/自测用）
  -RuntimeDir <path>    落地根目录，默认 <仓库>/runtime；Rust 侧会传绝对路径

用法：
  powershell -ExecutionPolicy Bypass -File scripts/bootstrap_runtime.ps1
  powershell -ExecutionPolicy Bypass -File scripts/bootstrap_runtime.ps1 -Flavor vulkan -Force
#>
param(
  [string]$Items = "moss,models-moss",
  [string]$Flavor = "auto",
  [string]$Mirror = "https://gh-proxy.com/",
  [switch]$Force,
  [string]$Manifest = "",
  [string]$RuntimeDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# stdout 必须是纯 JSONL：先统一成 UTF-8，避免中文 detail 在重定向时被 GBK 编码
try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false) } catch { }
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.ServicePointManager]::SecurityProtocol } catch { }

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
if (-not $RuntimeDir) { $RuntimeDir = Join-Path $root "runtime" }
$runtime = [System.IO.Path]::GetFullPath($RuntimeDir)

# ─── 事件输出（唯一允许写 stdout 的通道）────────────────────────────────
$script:emittedDone = $false

function Emit([hashtable]$evt) {
  $json = $evt | ConvertTo-Json -Compress -Depth 4
  [Console]::Out.WriteLine($json)
  [Console]::Out.Flush()
  if ($evt.event -eq "done") { $script:emittedDone = $true }
}

function Log([string]$msg) { [Console]::Error.WriteLine($msg) }

function Emit-Done([int]$exitCode, [string[]]$installed, [string[]]$skipped, [string[]]$failed) {
  # 注意：空的 [string[]] 参数会绑成 $null，@($null) 会成 [null]，必须过滤
  Emit @{
    event     = "done"
    ok        = ($exitCode -eq 0)
    installed = @($installed | Where-Object { $_ })
    skipped   = @($skipped | Where-Object { $_ })
    failed    = @($failed | Where-Object { $_ })
  }
}

# 预检/致命错误：先发 error，再发 done，由调用处 exit
function Stop-Fatal([string]$detail, [int]$exitCode, [string]$item) {
  $evt = @{ event = "error"; detail = $detail }
  if ($item) { $evt.item = $item }
  Emit $evt
  Emit-Done $exitCode @() @() @($detail)
  exit $exitCode
}

# ─── 参数校验（白名单，防注入）──────────────────────────────────────────
$Flavor = $Flavor.ToLower()
if ($Flavor -notmatch "^(auto|cpu|cuda|vulkan)$") {
  Stop-Fatal "无效的 -Flavor: $Flavor（可选 auto/cpu/cuda/vulkan）" 1
}

$itemIds = @($Items -split "[,;\s]+" | Where-Object { $_ })
if ($itemIds.Count -eq 0) { Stop-Fatal "未指定 -Items" 1 }

# ─── 清单 ───────────────────────────────────────────────────────────────
$manifestPath = $Manifest
if (-not $manifestPath) {
  $envManifest = $env:GSA_RUNTIME_MANIFEST
  $shipped = Join-Path $root "src-tauri\resources\runtime\manifest.json"
  $manifestPath = if ($envManifest) { $envManifest } elseif (Test-Path $shipped) { $shipped } else { "" }
}
if (-not $manifestPath -or -not (Test-Path $manifestPath)) {
  Stop-Fatal "找不到 manifest（可用 -Manifest 指定；内置位置：src-tauri/resources/runtime/manifest.json）" 1
}

try {
  $manifestJson = Get-Content $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
} catch {
  Stop-Fatal "manifest 解析失败：$($_.Exception.Message)" 1
}
$allItems = @($manifestJson.items)
if ($allItems.Count -eq 0) { Stop-Fatal "manifest 里没有任何条目" 1 }

$manifestVersion = if ($manifestJson.version) { [string]$manifestJson.version } else { "" }
Log "manifest: $manifestPath (version=$manifestVersion, items=$($allItems.Count))"

# ─── L0 探测：决定 auto 用哪个 flavor ───────────────────────────────────
function Get-CudaComputeCap {
  # 需要驱动 >= 510 才支持 --query-gpu=compute_cap；老驱动退化为 $null（不阻塞）
  try {
    $out = & nvidia-smi --query-gpu=compute_cap --format=csv,noheader 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $out) { return $null }
    $first = @($out)[0]
    if (-not $first) { return $null }
    $v = [double]($first.ToString().Trim().Trim('"'))
    return [int][math]::Round($v * 10)
  } catch { return $null }
}

function Test-VulkanPresent {
  # 驱动层的 vulkan-1.dll 存在即认为有 Vulkan 运行时（与 backend_probe 的 L0 同口径）
  try {
    if (Test-Path (Join-Path $env:SystemRoot "System32\vulkan-1.dll")) { return $true }
  } catch { }
  return $false
}

function Resolve-AutoFlavor([string[]]$available) {
  $hasCuda = $available -contains "cuda"
  $hasVulkan = $available -contains "vulkan"
  $hasCpu = $available -contains "cpu"
  if ($hasCuda) {
    $cc = Get-CudaComputeCap
    if ($null -eq $cc) {
      Log "auto: nvidia-smi 未给出算力（驱动 < 510？），按可用处理 → cuda"
      return "cuda"
    }
    if ($cc -ge 75) { return "cuda" }
    Log "auto: 算力 $([math]::Round($cc / 10, 1)) < 7.5，跳过 cuda（预构建 PTX 不覆盖老架构）"
  }
  if ($hasVulkan -and (Test-VulkanPresent)) { return "vulkan" }
  if ($hasCpu) { return "cpu" }
  if ($hasVulkan) { return "vulkan" }
  return $null
}

# ─── 选条目 ─────────────────────────────────────────────────────────────
# 可用 flavor 取「本次要装的条目」的并集，而不是只看第一条：
# 条目可能不带 flavor（models-moss 与后端无关），也可能同一 id 多个 flavor（moss）。
# 全都无 flavor ⇒ 不需要定 flavor（显式清空，避免后面按 "" 过滤把有 flavor 的条目漏掉）。
$flavored = @($allItems | Where-Object { $itemIds -contains $_.id -and $_.flavor })
$availableFlavors = @($flavored | ForEach-Object { ([string]$_.flavor).ToLower() } | Select-Object -Unique)
$effectiveFlavor = $Flavor
if ($Flavor -eq "auto") {
  if ($availableFlavors.Count -eq 0) {
    $effectiveFlavor = ""
    Log "auto: 所选条目与 flavor 无关，跳过设备探测"
  }
  else {
    $effectiveFlavor = Resolve-AutoFlavor $availableFlavors
    if (-not $effectiveFlavor) {
      Stop-Fatal "清单里没有可用的 flavor（可用：$($availableFlavors -join ', ')）" 1
    }
    Log "auto 选择: $effectiveFlavor"
  }
}

$selected = @()
$failedEarly = @()
foreach ($id in $itemIds) {
  $cands = @($allItems | Where-Object { $_.id -eq $id })
  if ($cands.Count -eq 0) {
    Emit @{ event = "item_fail"; item = $id; detail = "清单里没有条目 $id" }
    $failedEarly += $id
    continue
  }
  # 条目本身不带 flavor（与 flavor 无关）→ 直接选；否则按请求的 flavor 过滤
  $pick = @($cands | Where-Object { -not $_.flavor -or ([string]$_.flavor).ToLower() -eq $effectiveFlavor })
  if ($pick.Count -eq 0) {
    $known = @($cands | ForEach-Object { ([string]$_.flavor).ToLower() }) -join ", "
    Emit @{ event = "item_fail"; item = $id; flavor = $effectiveFlavor; detail = "清单里没有 $id 的 $effectiveFlavor 版本（可用：$known）" }
    $failedEarly += $id
    continue
  }
  $selected += $pick[0]
}

if ($selected.Count -eq 0) {
  Emit-Done 3 @() @() @($failedEarly)
  exit 3
}

# ─── 磁盘预检（在任何下载之前）──────────────────────────────────────────
function Get-FreeBytes([string]$path) {
  $probe = $path
  while ($probe -and -not (Test-Path $probe)) {
    $parent = Split-Path $probe -Parent
    if (-not $parent -or $parent -eq $probe) { break }
    $probe = $parent
  }
  if (-not $probe) { return $null }
  try {
    $root_ = [System.IO.Path]::GetPathRoot((Resolve-Path $probe).Path)
    $drive = (Get-PSDrive -Name $root_.TrimEnd(':', '\') -ErrorAction Stop)
    if ($null -ne $drive.Free) { return [int64]$drive.Free }
  } catch { }
  return $null
}

$downloadBytes = [int64]0
foreach ($it in $selected) { if ($it.size) { $downloadBytes += [int64]$it.size } }
# 余量：下载临时文件 + 解压（zip 展开经验约 2 倍）+ 固定余量
$requiredBytes = [int64]($downloadBytes * 3) + 100MB
$freeBytes = Get-FreeBytes $runtime
if ($null -ne $freeBytes -and $freeBytes -lt $requiredBytes) {
  $gb = { param($b) [math]::Round($b / 1GB, 2) }
  Stop-Fatal "磁盘空间不足：需要约 $(& $gb $requiredBytes) GB（下载 $( & $gb $downloadBytes ) GB + 解压余量），可用 $(& $gb $freeBytes) GB" 1
}
Log "磁盘预检通过：需要约 $([math]::Round($requiredBytes / 1GB, 2)) GB，可用 $([math]::Round($freeBytes / 1GB, 2)) GB"

# ─── 已安装状态 ─────────────────────────────────────────────────────────
$stateFile = Join-Path $runtime ".installed.json"
$stateItems = @{}
if (Test-Path $stateFile) {
  try {
    $stateRaw = Get-Content $stateFile -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($stateRaw.items) {
      foreach ($p in $stateRaw.items.PSObject.Properties) { $stateItems[$p.Name] = $p.Value }
    }
  } catch {
    Log "警告：.installed.json 解析失败，按未安装处理（$($_.Exception.Message)）"
  }
}

function Save-State {
  $ordered = [ordered]@{ version = 1; items = ([ordered]@{}) }
  foreach ($k in ($stateItems.Keys | Sort-Object)) {
    $v = $stateItems[$k]
    $ordered.items[$k] = [ordered]@{
      flavor        = [string]$v.flavor
      sha256        = [string]$v.sha256
      source_version = [string]$v.source_version
      installed_at  = [string]$v.installed_at
    }
  }
  $json = $ordered | ConvertTo-Json -Depth 6
  [System.IO.File]::WriteAllText($stateFile, $json, (New-Object System.Text.UTF8Encoding($false)))
}

# ─── 下载（WebClient 异步 + 轮询进度，PS 5.1 可用）──────────────────────
function Resolve-Url([string]$url) {
  if (-not $Mirror) { return $url }
  if ($url -match "^https?://") { return "$Mirror$url" }   # 代理前缀拼接，不插分隔符
  return "$($Mirror.TrimEnd('/'))/$($url.TrimStart('/'))"
}

function Invoke-Download([string]$url, [string]$dest, $item, $flavor, [int64]$expectedSize) {
  $wc = New-Object System.Net.WebClient
  $wc.Headers.Add("User-Agent", "gsa-bootstrap/1")
  $task = $null
  try {
    $task = $wc.DownloadFileTaskAsync([uri]$url, $dest)
    $lastPct = -1
    $lastTick = Get-Date
    while (-not $task.IsCompleted) {
      Start-Sleep -Milliseconds 200
      # DownloadFileTaskAsync 下 $wc.BytesReceived 常一直为 0（它只对内存流下载更新），
      # 直接用它进度条会永远停在 0%。改为以落地文件大小为准，拿不到再退回 BytesReceived。
      $got = [int64]0
      try { if (Test-Path $dest) { $got = (Get-Item $dest).Length } } catch { }
      if ($got -le 0) { $got = [int64]$wc.BytesReceived }
      $total = $expectedSize
      try {
        if ($wc.ResponseHeaders -and $wc.ResponseHeaders["Content-Length"]) {
          $total = [int64]$wc.ResponseHeaders["Content-Length"]
        }
      } catch { }
      if ($total -gt 0) {
        $pct = [math]::Round(($got / $total) * 100, 1)
        if ($pct -gt $lastPct -or ((Get-Date) - $lastTick).TotalSeconds -ge 2) {
          $evt = @{ event = "progress"; item = $item; phase = "download"; percent = $pct; detail = "下载中 $([math]::Round($got / 1MB, 1))/$([math]::Round($total / 1MB, 1)) MB" }
          if ($flavor) { $evt.flavor = $flavor }
          Emit $evt
          $lastPct = $pct
          $lastTick = Get-Date
        }
      }
    }
    $task.GetAwaiter().GetResult()
  } finally {
    $wc.Dispose()
  }
}

function Get-Sha256([string]$path) {
  return (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLower()
}

# ─── 主循环 ─────────────────────────────────────────────────────────────
$installed = @()
$skipped = @()
$failed = @()
$hashFailed = $false

foreach ($it in $selected) {
  $id = [string]$it.id
  $flavor = if ($it.flavor) { ([string]$it.flavor).ToLower() } else { "" }
  $destRel = if ($it.dest) { [string]$it.dest } else { "bin" }
  $destDir = Join-Path $runtime $destRel
  $expectedSha = ([string]$it.sha256).ToLower()
  $label = if ($flavor) { "$id（$flavor）" } else { $id }

  $startEvt = @{ event = "start"; item = $id; detail = "开始安装 $label" }
  if ($flavor) { $startEvt.flavor = $flavor }
  Emit $startEvt

  # 已安装且哈希一致 → 跳过（-Force 时重装）
  $prev = $stateItems[$id]
  if (-not $Force -and $prev -and ([string]$prev.sha256).ToLower() -eq $expectedSha -and (Test-Path $destDir)) {
    $hasFiles = @(Get-ChildItem $destDir -File -ErrorAction SilentlyContinue).Count -gt 0
    if ($hasFiles) {
      Emit @{ event = "item_skip"; item = $id; detail = "已安装且版本一致" }
      $skipped += $id
      continue
    }
  }

  $tmpZip = Join-Path ([System.IO.Path]::GetTempPath()) ("gsa_rt_" + $id + "_" + [guid]::NewGuid().ToString("N").Substring(0, 8) + ".zip")
  $tmpDir = [System.IO.Path]::GetDirectoryName($tmpZip) | Join-Path -ChildPath ([System.IO.Path]::GetFileNameWithoutExtension($tmpZip))
  $ok = $true
  try {
    # 1) 下载
    $absoluteUrl = [string]$it.url
    $url = Resolve-Url $absoluteUrl
    Log "下载 $label ← $url"
    try {
      Invoke-Download $url $tmpZip $id $flavor ([int64]$it.size)
    } catch {
      if ($Mirror -and $absoluteUrl -match "^https?://") {
        Log "镜像失败（$($_.Exception.Message)），回退直连"
        Emit @{ event = "progress"; item = $id; phase = "download"; percent = 0; detail = "镜像不可用，回退直连" }
        Invoke-Download $absoluteUrl $tmpZip $id $flavor ([int64]$it.size)
      } else { throw }
    }

    # 2) 校验
    $verifyEvt = @{ event = "progress"; item = $id; phase = "verify"; percent = 100; detail = "校验 SHA256" }
    if ($flavor) { $verifyEvt.flavor = $flavor }
    Emit $verifyEvt
    $actualSha = Get-Sha256 $tmpZip
    if ($actualSha -ne $expectedSha) {
      throw [System.InvalidOperationException]::new("HASH_MISMATCH 期望 $expectedSha 实际 $actualSha")
    }

    # 3) 解压（先解到临时目录再平铺搬入，规避 zip 条目的 ./ 前导）
    $extractEvt = @{ event = "progress"; item = $id; phase = "extract"; percent = 0; detail = "解压" }
    if ($flavor) { $extractEvt.flavor = $flavor }
    Emit $extractEvt
    if (Test-Path $tmpDir) { Remove-Item $tmpDir -Recurse -Force }
    Expand-Archive -Path $tmpZip -DestinationPath $tmpDir -Force
    if (-not (Test-Path $destDir)) { New-Item -ItemType Directory -Path $destDir -Force | Out-Null }
    $moved = 0
    foreach ($f in @(Get-ChildItem $tmpDir -Recurse -File)) {
      $target = Join-Path $destDir $f.Name
      Move-Item -Path $f.FullName -Destination $target -Force
      $moved++
    }
    if ($moved -eq 0) { throw [System.InvalidOperationException]::new("压缩包内没有文件") }

    # 4) 落地标记（.backend 只是缓存提示，判据永远是 L2 实测）
    if ($it.backend_marker) {
      $markerPath = Join-Path $destDir ".backend"
      [System.IO.File]::WriteAllText($markerPath, [string]$it.backend_marker, (New-Object System.Text.UTF8Encoding($false)))
    }

    # 5) 记状态
    $stateItems[$id] = [pscustomobject]@{
      flavor         = $flavor
      sha256         = $actualSha
      source_version = $manifestVersion
      installed_at   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    }
    Save-State

    $doneEvt = @{ event = "item_done"; item = $id; phase = "extract"; detail = "已安装 $label（$moved 个文件）" }
    if ($flavor) { $doneEvt.flavor = $flavor }
    Emit $doneEvt
    $installed += $id
  } catch {
    $ok = $false
    $msg = $_.Exception.Message
    if ($msg -like "HASH_MISMATCH*") { $hashFailed = $true }
    $failEvt = @{ event = "item_fail"; item = $id; detail = $msg }
    if ($flavor) { $failEvt.flavor = $flavor }
    Emit $failEvt
    $failed += $id
    Log "安装 $label 失败：$msg"
  } finally {
    Remove-Item $tmpZip -Force -ErrorAction SilentlyContinue
    Remove-Item $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
  }
}

# ─── 收尾 ───────────────────────────────────────────────────────────────
$exitCode = 0
if ($hashFailed) { $exitCode = 2 }
elseif ($failed.Count -gt 0) { $exitCode = 3 }
elseif ($failedEarly.Count -gt 0) { $exitCode = 3 }

if ($exitCode -eq 0) { Log "完成：安装 $($installed.Count) 项、跳过 $($skipped.Count) 项" }
else { Log "结束（退出码 $exitCode）：安装 $($installed.Count) 项、跳过 $($skipped.Count) 项、失败 $($failed.Count) 项" }

Emit-Done $exitCode $installed $skipped (@($failed) + @($failedEarly))
exit $exitCode
