<#
bootstrap_runtime.ps1 —— 按 manifest 安装 runtime（多条目 / 多 flavor 的通用安装器）

本期条目：
  - moss         按 flavor 拆成 cpu / vulkan / cuda 三条（二进制 + ggml 后端）
  - models-moss  不带 flavor 的一条（gguf 模型，三种后端共用同一份）
  - deps-ocr     OCR 运行时基线，flavor cpu（python + deps + models/paddleocr + worker）
  - deps-ocr-gpu OCR 的 CUDA 增量包，flavor cuda + optional（只有 NVIDIA 机器才装，
                 deps_gpu 叠加在 deps 之上，运行时 PYTHONPATH=deps_gpu;deps）
脚本按「多条目 + 多 flavor」写：后续 deps-embed 只加 manifest 条目，不改本脚本逻辑
（无 flavor 的条目直接选中，不参与设备探测）。

CUDA/cuDNN 运行库不再打包分发，改为安装时用 pip 现拉（见下）：
  - 资产本身是 NVIDIA 的 cudart/cublas/cudnn 轮子。CUDA EULA 1.2.2 禁止「作为独立产品分发」，
    1.1.2 要求「仅你的应用可访问」；把它们打成 zip 挂公开直链给任意人下载同时踩这两条。
  - 改为让用户机器从 PyPI 镜像按钉子清单直拉：许可上由用户自己获取，我们不参与再分发；
    顺带把最大的 1.86 GB 资产从发布里去掉（越过了 GitHub 单资产 2 GiB 上限，也少一个断流点）。
  - 触发条件：flavor 解析为 cuda 且 OCR 基线就位（pip 用的是 runtime/python/python.exe）。

flavor 解析规则（每个条目独立匹配）：
  1) 有与请求 flavor 相同的条目 → 用它；
  2) 否则该条目有 cpu 版 → 回退 cpu（例：AMD 机 auto 选到 vulkan，OCR 没有 vulkan 版）；
  3) 否则可选条目（optional）→ 静默跳过；非可选 → 记为失败（退出码 3）。

与前端/配置侧的接口见 docs/RUNTIME_BOOTSTRAP_CONTRACT.md：
  - stdout 只输出 JSONL 事件（start / progress / item_done / item_skip / item_fail /
    error / done），最后一行必为 done；人类可读诊断走 stderr；
  - 退出码：0 全成功 / 1 预检失败 / 2 校验失败 / 3 部分条目失败 / 4 未知错误。

参数：
  -Items <ids>          逗号分隔的条目 id，默认 moss,models-moss,deps-ocr,deps-ocr-gpu,deps-embed
                        （首启一次性装齐：ASR 二进制 + 模型 + OCR 运行时；GPU 那条约
                        在非 NVIDIA 机器上会被静默跳过。老调用方仍可显式带上
                        deps-ocr-cuda-libs 表示"要 CUDA 运行库"，它由 pip 阶段处理）
  -Flavor <auto|cpu|cuda|vulkan>  默认 auto（L0 探测：nvidia-smi → vulkaninfo → cpu）
  -Mirror <前缀>        代理前缀（默认 gh-proxy），传空串 "" 表示直连；见契约 §4.1
  -PipMirror <index-url>  CUDA 运行库的 PyPI 索引，默认华为云（实测 10 MB/s；
                        传空串 = 官方 PyPI；清华 TUNA 目录接口会 403，别用）
  -SkipGpuLibs          跳过 CUDA 运行库的 pip 阶段（排障/离线用）
  -PipUseProxy          pip 阶段沿用系统代理（默认临时绕过：国内镜像直连更快）
  -Force                忽略已安装标记，强制重装
  -SegmentMB <int>      分段大小（MB），默认 64：按此把每个资产切成若干段下载，
                        断流只损失当前段，已下的段跨进程保留（见契约 §4.5）
  -NoResume             丢弃 .downloads 里已下的分段，从头开始（排障用）
  -MaxAttempts <int>    单段最多尝试次数，默认 3
  -Manifest <path>      覆盖内置清单（离线/自测用）
  -RuntimeDir <path>    落地根目录，默认 <仓库>/runtime；Rust 侧会传绝对路径

用法：
  powershell -ExecutionPolicy Bypass -File scripts/bootstrap_runtime.ps1
  powershell -ExecutionPolicy Bypass -File scripts/bootstrap_runtime.ps1 -Flavor vulkan -Force
#>
param(
  [string]$Items = "moss,models-moss,deps-ocr,deps-ocr-gpu,deps-embed",
  [string]$Flavor = "auto",
  [string]$Mirror = "https://gh-proxy.com/",
  [string]$PipMirror = "https://repo.huaweicloud.com/repository/pypi/simple",
  [switch]$PipUseProxy,
  [switch]$SkipGpuLibs,
  [switch]$Force,
  [int]$SegmentMB = 64,
  [switch]$NoResume,
  [int]$MaxAttempts = 3,
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

# CUDA 运行库的钉子清单（源：scripts/ocr-gpu-requirements.lock.txt 去掉 paddlepaddle-gpu，
# 那个随 deps-ocr-gpu 包分发）。--require-hashes 要求逐条带 sha256，这里是 win_amd64 轮子的官方摘要。
$GpuLibs = @(
  @{ name = "nvidia-cublas-cu12";     ver = "12.6.4.1"; size = 434535301; sha256 = "9e4fa264f4d8a4eb0cdbd34beadc029f453b3bafae02401e999cf3d5a5af75f8" },
  @{ name = "nvidia-cuda-runtime-cu12"; ver = "12.6.77"; size = 891773;   sha256 = "86c58044c824bf3c173c49a2dbc7a6c8b53cb4e4dca50068be0bf64e9dab3f7f" },
  @{ name = "nvidia-cudnn-cu12";      ver = "9.9.0.52"; size = 767785830; sha256 = "d53036b7edad1a85b5d59580defc91e30326746fde21ffc701eb8b4d4695eca1" },
  @{ name = "nvidia-cufft-cu12";      ver = "11.3.0.4"; size = 199356881; sha256 = "6048ebddfb90d09d2707efb1fd78d4e3a77cb3ae4dc60e19aab6be0ece2ae464" },
  @{ name = "nvidia-curand-cu12";     ver = "10.3.7.77"; size = 55783873; sha256 = "6d6d935ffba0f3d439b7cd968192ff068fafd9018dbf1b85b37261b13cfc9905" },
  @{ name = "nvidia-cusolver-cu12";   ver = "11.7.1.2"; size = 149287877; sha256 = "6813f9d8073f555444a8705f3ab0296d3e1cb37a16d694c5fc8b862a0d8706d7" },
  @{ name = "nvidia-cusparse-cu12";   ver = "12.5.4.2"; size = 213712630; sha256 = "4acb8c08855a26d737398cba8fb6f8f5045d93f82612b4cfd84645a2332ccf20" },
  @{ name = "nvidia-nvjitlink-cu12";  ver = "12.9.86"; size = 35584936; sha256 = "cc6fcec260ca843c10e34c936921a1c426b351753587fdd638e8cff7b16bb9db" }
)
$GpuLibsDownloadBytes = [int64]0
foreach ($g in $GpuLibs) { $GpuLibsDownloadBytes += [int64]$g.size }
$GpuLibsUnpackedBytes = [int64]2693178035   # 实测 deps_gpu/nvidia 落地大小

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
# deps-ocr-cuda-libs 是"虚拟条目"：清单里没有对应的下载包（CUDA 运行库改为安装时 pip 现拉），
# 出现在 -Items 里只表示"要 CUDA 运行库"，由后面的 pip 阶段满足。
$GpuLibsId = "deps-ocr-cuda-libs"
$wantGpuLibs = $false
foreach ($id in $itemIds) {
  if ($id -eq $GpuLibsId) { $wantGpuLibs = $true; continue }
  $cands = @($allItems | Where-Object { $_.id -eq $id })
  if ($cands.Count -eq 0) {
    Emit @{ event = "item_fail"; item = $id; detail = "清单里没有条目 $id" }
    $failedEarly += $id
    continue
  }
  # 条目本身不带 flavor（与 flavor 无关）→ 直接选；否则按请求的 flavor 过滤
  $pick = @($cands | Where-Object { -not $_.flavor -or ([string]$_.flavor).ToLower() -eq $effectiveFlavor })
  # 没有精确匹配时回退该条目的 cpu 版本：deps-ocr 只有 cpu/cuda 两版，而 AMD 机上
  # auto 会落到 vulkan（moss 有、OCR 没有）——这时应装 OCR 的 cpu 基线，而不是整轮失败。
  if ($pick.Count -eq 0 -and $effectiveFlavor) {
    $pick = @($cands | Where-Object { ([string]$_.flavor).ToLower() -eq "cpu" })
    if ($pick.Count -gt 0) { Log "条目 $id 没有 $effectiveFlavor 版本，回退 cpu" }
  }
  if ($pick.Count -eq 0) {
    # 可选条目（如 GPU 增量包）：本机设备用不上就跳过，不算失败
    if (@($cands | Where-Object { $_.optional }).Count -gt 0) {
      Log "跳过可选条目 $id（没有 $effectiveFlavor 版本）"
      continue
    }
    $known = @($cands | ForEach-Object { ([string]$_.flavor).ToLower() }) -join ", "
    Emit @{ event = "item_fail"; item = $id; flavor = $effectiveFlavor; detail = "清单里没有 $id 的 $effectiveFlavor 版本（可用：$known）" }
    $failedEarly += $id
    continue
  }
  $selected += $pick[0]
}

# 是否要现拉 CUDA 运行库：flavor 落到 cuda 且本次装了 OCR 的 GPU 增量包（或调用方显式点了 id）
$willFetchGpuLibs = (-not $SkipGpuLibs) -and ($effectiveFlavor -eq "cuda") -and
  ($wantGpuLibs -or (@($selected | Where-Object { $_.id -eq "deps-ocr-gpu" }).Count -gt 0))

if ($selected.Count -eq 0 -and -not $willFetchGpuLibs) {
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
$unpackedBytes = [int64]0
$maxUnpacked = [int64]0
foreach ($it in $selected) {
  if ($it.size) { $downloadBytes += [int64]$it.size }
  if ($it.unpacked_size) {
    $unpackedBytes += [int64]$it.unpacked_size
    # 峰值出现在"解到临时目录 + 再拷进 dest"的那一刻：当前这一项会出现两份
    if ([int64]$it.unpacked_size -gt $maxUnpacked) { $maxUnpacked = [int64]$it.unpacked_size }
  }
}
# 有 unpacked_size 就用真实值（OCR 的 deps 24k 个 .py/.pyd，zip 只压到 ~2.5 倍）；
# 没有就退回粗糙的 3 倍估算。
# 峰值 = Σ下载 + Σ解压 + 最大单包解压（临时目录与 dest 同时存在的那一份）+ 余量。
# 早先漏了最后一项，cuda 的 2.7 GB 包正好卡在这个差额上（真机上吃到"磁盘空间不足"）。
if ($unpackedBytes -gt 0) {
  $requiredBytes = [int64]($downloadBytes + $unpackedBytes + $maxUnpacked) + 100MB
}
else {
  $requiredBytes = [int64]($downloadBytes * 3) + 100MB
}
# CUDA 运行库走 pip，不在清单里，但同样吃盘：8 个轮子 1.73 GiB + 解包后 2.51 GiB。
if ($willFetchGpuLibs) {
  $requiredBytes += $GpuLibsDownloadBytes + $GpuLibsUnpackedBytes
}
$freeBytes = Get-FreeBytes $runtime
if ($null -ne $freeBytes -and $freeBytes -lt $requiredBytes) {
  $gb = { param($b) [math]::Round($b / 1GB, 2) }
  $extra = if ($willFetchGpuLibs) { "（含 CUDA 运行库 $( & $gb ($GpuLibsDownloadBytes + $GpuLibsUnpackedBytes) ) GB）" } else { "" }
  Stop-Fatal "磁盘空间不足：需要约 $(& $gb $requiredBytes) GB$extra（下载 $( & $gb $downloadBytes ) GB + 解压余量），可用 $(& $gb $freeBytes) GB" 1
}
Log "磁盘预检通过：需要约 $([math]::Round($requiredBytes / 1GB, 2)) GB$(if ($willFetchGpuLibs) { '（含 CUDA 运行库）' })，可用 $([math]::Round($freeBytes / 1GB, 2)) GB"

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

# ─── 下载（curl 分段 + 断点续传 + 多源回退）─────────────────────────────
# 为什么不用 WebClient：它没有续传，断流就从 0 开始（实测一次 1.77 GB 的包在 57%
# 断掉后整包重下）；也没有分段，单流在跨境链路上往往只有 0.1–0.5 MB/s。
# 改用系统自带的 curl.exe（Win10 1803+ 均有）：
#   · 按 -SegmentMB 切成若干段，每段一个文件，失败只重下这一段；
#   · 段文件落在 <runtime>\.downloads\<id>-<sha8>\，跨进程保留 ⇒ 下次运行接着下；
#   · curl -C 依赖服务端支持 Range，所以**先探测 206**：不支持就退回单流，
#     否则对忽略 Range 的镜像用续传会把文件越下越大（实测 curl -C - 把 1770 MB
#     下成 2036 MB）。
function Resolve-Url([string]$url) {
  if (-not $Mirror) { return $url }
  if ($url -match "^https?://") { return "$Mirror$url" }   # 代理前缀拼接，不插分隔符
  return "$($Mirror.TrimEnd('/'))/$($url.TrimStart('/'))"
}

# 候选源：清单的 urls[]（顺序即优先级）→ 镜像前缀 → 直连；去重保序
function Get-UrlCandidates($it) {
  $list = New-Object System.Collections.ArrayList
  foreach ($u in @($it.urls)) { if ($u) { [void]$list.Add([string]$u) } }
  if ($it.url) {
    if ($Mirror) { [void]$list.Add((Resolve-Url ([string]$it.url))) }
    [void]$list.Add([string]$it.url)
  }
  $seen = @{}
  $out = @()
  foreach ($u in $list) { if (-not $seen.ContainsKey($u)) { $seen[$u] = $true; $out += $u } }
  return $out
}

function Get-FileLength([string]$p) {
  if (Test-Path -LiteralPath $p) { return [int64](Get-Item -LiteralPath $p).Length }
  return [int64]0
}

# 人类可读体积（只在 stderr/事件 detail 里用，不参与协议）
function Format-MB([int64]$bytes) {
  if ($bytes -ge 1GB) { return ("{0:N2} GB" -f ($bytes / 1GB)) }
  return ("{0:N1} MB" -f ($bytes / 1MB))
}

# 探测 Range 支持：返回 @{ ok; length }（length=0 表示服务端没给总长）
function Test-RangeSupport([string]$url) {
  try {
    $out = & curl.exe -sS -L --max-time 25 -r 0-0 -D - -o NUL $url 2>$null
    $text = ($out | Out-String)
    if ($text -match "(?m)^HTTP/[0-9.]+ 206") {
      $len = [int64]0
      if ($text -match "(?i)Content-Range:\s*bytes\s+0-0/(\d+)") { $len = [int64]$Matches[1] }
      return @{ ok = $true; length = $len }
    }
  } catch { }
  return @{ ok = $false; length = [int64]0 }
}

# 下载一段（含段内续传：已落地的字节不再重下）。失败抛异常，由调用方重试。
# $progress 是 scriptblock，收一个参数 = 本段已拿到的字节数（用于总进度）。
function Invoke-CurlSegment([string]$url, [string]$segFile, [int64]$offset, [int64]$length, [scriptblock]$progress) {
  $piece = "$segFile.curlpart"

  # 上次被中断时留下的 .curlpart 是本段的**连续前缀**（每次尝试都从 offset+已下 开始
  # 写一个全新文件），先并进段文件 ⇒ 跨进程续传真的从断点继续，而不是重下整段。
  if ((Test-Path -LiteralPath $piece) -and (Get-FileLength $piece) -gt 0) {
    $fin0 = [System.IO.File]::Open($segFile, [System.IO.FileMode]::Append)
    $inp0 = [System.IO.File]::OpenRead($piece)
    try { $inp0.CopyTo($fin0) } finally { $inp0.Dispose(); $fin0.Dispose() }
    Remove-Item -LiteralPath $piece -Force -ErrorAction SilentlyContinue
  }

  $have = Get-FileLength $segFile
  if ($have -gt $length) { Remove-Item -LiteralPath $segFile -Force; $have = 0 }
  if ($have -eq $length -and $length -gt 0) { return }

  Remove-Item -LiteralPath $piece -Force -ErrorAction SilentlyContinue

  $from = $offset + $have
  $to = $offset + $length - 1
  $curlArgs = @(
    "-sS", "-L", "--fail", "--retry", "2", "--retry-delay", "2",
    "--connect-timeout", "25", "--speed-limit", "1024", "--speed-time", "90",
    "-r", "$from-$to", "-o", $piece, $url
  )

  # 直接造 Process：Start-Process -PassThru 拿到的对象在进程退出后 ExitCode 常为
  # 空（本机实测每次都拿到空串，导致"下完了却判失败"），.NET API 才稳。
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = "curl.exe"
  $psi.Arguments = (($curlArgs | ForEach-Object { '"' + ([string]$_).Replace('"', '\"') + '"' }) -join ' ')
  $psi.UseShellExecute = $false
  $psi.CreateNoWindow = $true
  $psi.RedirectStandardError = $true
  $proc = [System.Diagnostics.Process]::Start($psi)
  $errTask = $proc.StandardError.ReadToEndAsync()
  $lastPct = -1
  $lastTick = Get-Date
  while (-not $proc.HasExited) {
    Start-Sleep -Milliseconds 250
    if ($progress) {
      $got = [int64]$have + (Get-FileLength $piece)
      $pct = if ($length -gt 0) { [math]::Floor(([double]$got / [double]$length) * 100) } else { 0 }
      if ($pct -ne $lastPct -or ((Get-Date) - $lastTick).TotalSeconds -ge 3) {
        & $progress $got
        $lastPct = $pct
        $lastTick = Get-Date
      }
    }
  }
  $proc.WaitForExit()
  $code = $proc.ExitCode
  $err = ""
  try { $err = ((@($errTask.Result) -join " ")).Trim() } catch { }
  $proc.Dispose()

  $got = Get-FileLength $piece
  if ($code -ne 0 -or $got -le 0) {
    if ($err) { $err = ($err -split "`r?`n" | Select-Object -Last 2) -join " / " }
    throw "SEGMENT_FAILED 段 $from-$to（curl 退出码 $code，$got 字节）：$err"
  }
  # 追加进段文件（curl 的 --append 只用于上传，下载续传只能自己拼）
  $fin = [System.IO.File]::Open($segFile, [System.IO.FileMode]::Append)
  $inp = [System.IO.File]::OpenRead($piece)
  try { $inp.CopyTo($fin) } finally { $inp.Dispose(); $fin.Dispose() }
  Remove-Item -LiteralPath $piece -Force -ErrorAction SilentlyContinue

  $now = Get-FileLength $segFile
  if ($now -gt $length) { throw "SEGMENT_OVERRUN 段 $offset 有 $now 字节，超过 $length（服务端没按 Range 返回？）" }
  if ($now -lt $length) { throw "SEGMENT_SHORT 段 $offset 只有 $now/$length 字节" }
}

# 单流兜底（源不支持 Range 时用；不做续传，避免对忽略 Range 的镜像越下越大）
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

# 按顺序拼接分段并核对总长
function Join-Segments([string[]]$files, [string]$dest, [int64]$expectedSize) {
  $tmp = "$dest.joining"
  Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
  $out = [System.IO.File]::Create($tmp)
  try {
    foreach ($f in $files) {
      $inp = [System.IO.File]::OpenRead($f)
      try { $inp.CopyTo($out) } finally { $inp.Dispose() }
    }
  } finally { $out.Dispose() }
  $len = Get-FileLength $tmp
  if ($expectedSize -gt 0 -and $len -ne $expectedSize) {
    throw "JOIN_MISMATCH 拼装后 $len 字节，期望 $expectedSize"
  }
  Move-Item -LiteralPath $tmp -Destination $dest -Force
}

# 入口：选源 → 分段下载（可续传）→ 拼接；源不支持 Range 时退回单流
function Invoke-SegmentedDownload($it, [string]$dest, [string]$cacheDir, $id, $flavor) {
  # 分段依赖系统自带 curl（Windows 10 1803+ 都有）；老系统上别硬失败，退回单流
  if ($SegmentMB -gt 0 -and -not (Get-Command curl.exe -ErrorAction SilentlyContinue)) {
    Log "未找到 curl.exe（Win10 1803+ 自带），退回单流下载（无分段、无续传）"
    $SegmentMB = 0
  }

  $expectedSize = [int64]0
  if ($it.size) { $expectedSize = [int64]$it.size }
  $candidates = @(Get-UrlCandidates $it)
  if ($candidates.Count -eq 0) { throw "清单条目 $id 没有任何 url" }

  $rangeUrl = $null
  $plainUrl = $null
  foreach ($u in $candidates) {
    if ($SegmentMB -le 0) { $plainUrl = $u; break }
    $probe = Test-RangeSupport $u
    if ($probe.ok) {
      if ($probe.length -gt 0) { $expectedSize = $probe.length }
      $rangeUrl = $u
      Log "分段下载源：$u（支持 Range，总长 $(Format-MB $expectedSize)）"
      break
    }
    if (-not $plainUrl) { $plainUrl = $u }
    Log "跳过 $u（不支持 Range，或探测失败）"
  }

  # 不支持 Range / 未知长度 / 关掉分段 ⇒ 单流，且逐个候选源回退
  # （镜像前缀会把 localhost 之类也拼成死的，必须靠回退兜住）
  if (-not $rangeUrl -or $expectedSize -le 0) {
    $lastErr = $null
    foreach ($u in $candidates) {
      try {
        Log "单流下载：$u"
        Remove-Item -LiteralPath $dest -Force -ErrorAction SilentlyContinue
        Invoke-Download $u $dest $id $flavor $expectedSize
        return
      } catch {
        $lastErr = $_.Exception.Message
        Log "单流源失败（$u）：$lastErr"
      }
    }
    throw "所有候选源都失败：$lastErr"
  }

  $segBytes = [int64]$SegmentMB * 1MB
  $count = [int]([math]::Ceiling([double]$expectedSize / [double]$segBytes))
  if ($count -lt 1) { $count = 1 }
  if (-not (Test-Path -LiteralPath $cacheDir)) { New-Item -ItemType Directory -Path $cacheDir -Force | Out-Null }

  $resumeBytes = 0
  for ($k = 0; $k -lt $count; $k++) {
    $off = [int64]$k * $segBytes
    $len = [math]::Min($segBytes, $expectedSize - $off)
    $resumeBytes += [int64][math]::Min((Get-FileLength (Join-Path $cacheDir ("seg{0:d3}.bin" -f $k))), $len)
  }
  if ($resumeBytes -gt 0) { Log "续传：缓存里已有 $(Format-MB $resumeBytes)，只需补 $(Format-MB ($expectedSize - $resumeBytes))" }
  else { Log "分段下载：$count 段 × $(Format-MB $segBytes)，缓存 $cacheDir" }

  $files = @()
  $doneBytes = [int64]0
  for ($i = 0; $i -lt $count; $i++) {
    $off = [int64]$i * $segBytes
    $len = [math]::Min($segBytes, $expectedSize - $off)
    $seg = Join-Path $cacheDir ("seg{0:d3}.bin" -f $i)
    $files += $seg
    if ($NoResume) { Remove-Item -LiteralPath $seg -Force -ErrorAction SilentlyContinue }

    $baseBytes = $doneBytes
    $idx = $i
    $emit = {
      param($sofar)
      $total = $baseBytes + $sofar
      $pct = [math]::Round(([double]$total / [double]$expectedSize) * 100, 1)
      $evt = @{ event = "progress"; item = $id; phase = "download"; percent = $pct; detail = "下载中 $(Format-MB $total)/$(Format-MB $expectedSize)（第 $($idx + 1)/$count 段）" }
      if ($flavor) { $evt.flavor = $flavor }
      Emit $evt
    }

    if ((Get-FileLength $seg) -eq $len -and $len -gt 0) { $doneBytes += $len; continue }
    $attempt = 0
    while ($true) {
      $attempt++
      try {
        Invoke-CurlSegment $rangeUrl $seg $off $len $emit
        break
      } catch {
        if ($attempt -ge $MaxAttempts) { throw }
        Log "第 $($i + 1)/$count 段失败（$($_.Exception.Message)），第 $attempt/$MaxAttempts 次，2 秒后重试"
        Start-Sleep -Seconds 2
      }
    }
    $doneBytes += $len
  }

  Join-Segments $files $dest $expectedSize
  Remove-Item -LiteralPath $cacheDir -Recurse -Force -ErrorAction SilentlyContinue
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
    # 1) 下载（分段 + 断点续传 + 多源回退；候选源在函数内部按顺序探测）
    $cacheDir = Join-Path $runtime (".downloads\" + $id + "-" + $expectedSha.Substring(0, 8))
    try {
      Invoke-SegmentedDownload $it $tmpZip $cacheDir $id $flavor
    } catch {
      throw [System.InvalidOperationException]::new("DOWNLOAD_FAILED $($_.Exception.Message)")
    }

    # 2) 校验
    $verifyEvt = @{ event = "progress"; item = $id; phase = "verify"; percent = 100; detail = "校验 SHA256" }
    if ($flavor) { $verifyEvt.flavor = $flavor }
    Emit $verifyEvt
    $actualSha = Get-Sha256 $tmpZip
    if ($actualSha -ne $expectedSha) {
      throw [System.InvalidOperationException]::new("HASH_MISMATCH 期望 $expectedSha 实际 $actualSha")
    }

    # 3) 解压（保留目录结构：moss 包是平铺的，等价于原来的逐文件搬入；
    #    OCR 包带 python/、deps/、models/paddleocr/ 子目录，平铺会把 24k 个文件
    #    压成一堆同名文件。Copy -Recurse 也比逐个 Move 快一个数量级。）
    $extractEvt = @{ event = "progress"; item = $id; phase = "extract"; percent = 0; detail = "解压" }
    if ($flavor) { $extractEvt.flavor = $flavor }
    Emit $extractEvt
    if (Test-Path $tmpDir) { Remove-Item $tmpDir -Recurse -Force }
    Expand-Archive -Path $tmpZip -DestinationPath $tmpDir -Force
    if (-not (Test-Path $destDir)) { New-Item -ItemType Directory -Path $destDir -Force | Out-Null }
    Copy-Item -Path (Join-Path $tmpDir "*") -Destination $destDir -Recurse -Force
    $moved = @(Get-ChildItem $tmpDir -Recurse -File).Count
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

# ─── CUDA 运行库：pip 现拉（不打包、不再分发）────────────────────────────
# 为什么不用 zip：CUDA EULA 禁止把 SDK 当独立产品分发，并要求"仅你的应用可访问"。
# 让用户机器自己从 PyPI 镜像按钉子清单拉取，我们只发版本号 + 官方摘要。
function Install-GpuLibs {
  $id = $GpuLibsId
  $depsGpu = Join-Path $runtime "deps_gpu"
  $pythonExe = Join-Path $runtime "python\python.exe"
  if (-not (Test-Path $pythonExe)) {
    Emit @{ event = "item_fail"; item = $id; flavor = "cuda"; detail = "找不到 $pythonExe —— CUDA 运行库用 OCR 基线里的 pip 安装，请先装 deps-ocr" }
    return "fail"
  }

  # 指纹 = 清单内容摘要：版本或任一 sha256 变了就要重装
  $fpText = (@($GpuLibs | ForEach-Object { "$($_.name)==$($_.ver):$($_.sha256)" }) -join "`n")
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $lockHash = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($fpText))) -replace "-", "").ToLower()
  } finally { $sha.Dispose() }

  $nvidiaDir = Join-Path $depsGpu "nvidia"
  $prev = $stateItems[$id]
  if (-not $Force -and $prev -and ([string]$prev.sha256).ToLower() -eq $lockHash -and (Test-Path $nvidiaDir)) {
    if (@(Get-ChildItem $nvidiaDir -Directory -ErrorAction SilentlyContinue).Count -ge 4) {
      Emit @{ event = "item_skip"; item = $id; detail = "CUDA 运行库已就绪（版本一致）" }
      return "skip"
    }
  }

  Emit @{ event = "start"; item = $id; flavor = "cuda"; detail = "开始拉取 CUDA 运行库（$( [math]::Round($GpuLibsDownloadBytes / 1MB) ) MB，$( $GpuLibs.Count ) 个轮子）" }
  Log "CUDA 运行库：pip 从 $PipMirror 拉取 $($GpuLibs.Count) 个轮子（$( [math]::Round($GpuLibsDownloadBytes / 1MB) ) MB）"

  $reqFile = Join-Path ([System.IO.Path]::GetTempPath()) ("gsa_gpulibs_" + [guid]::NewGuid().ToString("N").Substring(0, 8) + ".txt")
  $errFile = [System.IO.Path]::ChangeExtension($reqFile, ".err.log")
  $lines = @($GpuLibs | ForEach-Object { "$($_.name)==$($_.ver) --hash=sha256:$($_.sha256)" })
  [System.IO.File]::WriteAllLines($reqFile, $lines, (New-Object System.Text.UTF8Encoding($false)))

  $pipArgs = @(
    "-m", "pip", "install",
    "--no-deps", "--only-binary=:all:", "--require-hashes",
    "--no-cache-dir", "--disable-pip-version-check", "--no-input",
    "--progress-bar", "off", "--upgrade",
    "--target", $depsGpu,
    "-r", $reqFile
  )
  if ($PipMirror) { $pipArgs += @("--index-url", $PipMirror) }

  # 国内镜像直连最快；系统代理（尤其高延迟节点）会把 10 MB/s 拖到 1 MB/s 以下
  $savedProxy = @{ HTTP_PROXY = $env:HTTP_PROXY; HTTPS_PROXY = $env:HTTPS_PROXY; ALL_PROXY = $env:ALL_PROXY }
  $restoreProxy = $false
  if (-not $PipUseProxy) {
    $env:HTTP_PROXY = $null; $env:HTTPS_PROXY = $null; $env:ALL_PROXY = $null
    if ($savedProxy.HTTP_PROXY -or $savedProxy.HTTPS_PROXY) {
      Log "检测到系统代理，pip 阶段已临时绕过（国内镜像不需要代理）"
      $restoreProxy = $true
    }
  }

  $ok = $false
  $detail = ""
  try {
    $seen = 0
    $total = $GpuLibs.Count
    & $pythonExe @pipArgs 2> $errFile | ForEach-Object {
      $line = [string]$_
      if ($line -match "^\s*Downloading\s+(\S+)") {
        $seen++
        $pct = [math]::Round(($seen / $total) * 100, 1)
        Emit @{ event = "progress"; item = $id; flavor = "cuda"; phase = "download"; percent = $pct; detail = "拉取 $($Matches[1])（$seen/$total）" }
      }
    }
    $code = $LASTEXITCODE
    if ($code -ne 0) {
      $tail = @()
      if (Test-Path $errFile) { $tail = @(Get-Content $errFile -Encoding UTF8 | Where-Object { $_.Trim() } | Select-Object -Last 5) }
      throw [System.InvalidOperationException]::new("pip 退出码 $code" + $(if ($tail.Count) { "：" + ($tail -join " / ") } else { "" }))
    }
    $files = @(Get-ChildItem $depsGpu -Recurse -File -ErrorAction SilentlyContinue).Count
    if ($files -eq 0) { throw [System.InvalidOperationException]::new("pip 成功但 $depsGpu 为空") }
    $detail = "CUDA 运行库就绪（$files 个文件）"
    $ok = $true
  } catch {
    $detail = $_.Exception.Message
    $ok = $false
  } finally {
    if ($restoreProxy) {
      $env:HTTP_PROXY = $savedProxy.HTTP_PROXY; $env:HTTPS_PROXY = $savedProxy.HTTPS_PROXY; $env:ALL_PROXY = $savedProxy.ALL_PROXY
    }
    Remove-Item $reqFile -Force -ErrorAction SilentlyContinue
    Remove-Item $errFile -Force -ErrorAction SilentlyContinue
  }

  if ($ok) {
    $stateItems[$id] = [pscustomobject]@{
      flavor         = "cuda"
      sha256         = $lockHash
      source_version = "pip:" + $(($GpuLibs | ForEach-Object { "$($_.name)=$($_.ver)" }) -join ",")
      installed_at   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    }
    Save-State
    Emit @{ event = "item_done"; item = $id; flavor = "cuda"; phase = "extract"; detail = $detail }
    return "ok"
  }
  Emit @{ event = "item_fail"; item = $id; flavor = "cuda"; detail = $detail }
  Log "CUDA 运行库安装失败：$detail"
  return "fail"
}

if ($willFetchGpuLibs) {
  switch (Install-GpuLibs) {
    "ok"   { $installed += $GpuLibsId }
    "skip" { $skipped += $GpuLibsId }
    default { $failed += $GpuLibsId }
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
