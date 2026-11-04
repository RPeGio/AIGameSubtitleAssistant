<#
.SYNOPSIS
  验证一个下载 URL 是否适合做运行时资产源：Range 支持、TTFB、持续速率、缓存迹象。

.DESCRIPTION
  为什么需要它：分发链路的可靠性只能靠反复实测，而"单次测速没有意义"
  （同一源相邻两次能差 70 倍）。本脚本固定做三件事：

    1. Range 探测 —— bootstrap_runtime.ps1 的分段下载依赖 206。
       返回 200 的源会被降级为单流（不续传），必须让运维知道。
    2. 多次采样取中位 —— TTFB 与持续速率各采 N 次，报中位数与极值。
    3. 缓存迹象 —— Age / X-Cache / Server 头，用来判断是"命中边缘"还是"每次回源"。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify_cdn_mirror.ps1 -Url https://dl.rpegio.top/moss-runtime-v1/moss-vulkan.zip
  powershell -ExecutionPolicy Bypass -File scripts\verify_cdn_mirror.ps1 -Url https://... -Samples 5 -Seconds 15
#>
param(
  [Parameter(Mandatory = $true)][string]$Url,
  [int]$Samples = 3,
  [int]$Seconds = 10,
  [int64]$RangeBytes = 1048576
)

$ErrorActionPreference = "Stop"
if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) { throw "找不到 curl.exe（Win10 1803+ 自带）" }

function Median([double[]]$xs) {
  if ($xs.Count -eq 0) { return 0 }
  $s = $xs | Sort-Object
  $n = $s.Count
  if ($n % 2 -eq 1) { return $s[[int](($n - 1) / 2)] }
  return ($s[$n / 2 - 1] + $s[$n / 2]) / 2
}

function Fmt([double]$bytes) {
  if ($bytes -ge 1GB) { return "{0:N2} GB" -f ($bytes / 1GB) }
  if ($bytes -ge 1MB) { return "{0:N1} MB" -f ($bytes / 1MB) }
  if ($bytes -ge 1KB) { return "{0:N1} KB" -f ($bytes / 1KB) }
  return "$([int]$bytes) B"
}

$uri = [uri]$Url
Write-Host ("目标: {0}" -f $Url)
Write-Host ("主机: {0}" -f $uri.Host)

# ─── 1. DNS ───────────────────────────────────────────────
try {
  $ips = @(Resolve-DnsName $uri.Host -Type A -ErrorAction Stop | Where-Object { $_.IPAddress } | ForEach-Object { $_.IPAddress })
  Write-Host ("解析: {0}" -f ($ips -join ", "))
} catch {
  Write-Warning ("DNS 解析失败: {0}" -f $_.Exception.Message)
}

# 原生程序的 stderr 在 PS 5.1 下会被包成 ErrorRecord，$ErrorActionPreference=Stop 时
# 直接终止脚本；而 curl 用 --max-time 截断大文件时**必然**返回 28（正常现象）。
# 所以调原生命令一律临时切回 Continue，并只用 -s（不要 -S，否则超时提示会刷屏）。
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "Continue"

# ─── 2. 响应头（HEAD 拿不到就退回 GET 0 字节）──────────────
Write-Host ""
Write-Host "── 响应头 ──"
$hdr = @(curl.exe -s -L --max-time 20 -r 0-0 -D - -o NUL $Url 2>&1)
$status = ($hdr | Select-String -Pattern '^HTTP/' | Select-Object -Last 1).Line
if (-not $status) {
  Write-Warning "  拿不到响应头（源可能不可达或超时），下面的采样只作参考"
  $status = ""
}
Write-Host ("状态行: {0}" -f ($status -replace "`r", ""))

$interesting = 'Content-Length|Content-Range|Accept-Ranges|Age|Cache-Control|X-Cache|X-Swift|Server|Via|ETag|Last-Modified'
$fields = @{}
foreach ($line in $hdr) {
  $l = $line -replace "`r", ""
  if ($l -match "^($interesting):\s*(.+)$") { $fields[$Matches[1]] = $Matches[2].Trim() }
}
foreach ($k in @('Content-Length', 'Content-Range', 'Accept-Ranges', 'Age', 'Cache-Control', 'X-Cache', 'Server', 'Via', 'ETag')) {
  if ($fields.ContainsKey($k)) { Write-Host ("  {0,-16} {1}" -f $k, $fields[$k]) }
}

# ─── 3. Range 探测 ─────────────────────────────────────────
Write-Host ""
Write-Host "── Range 探测（分段下载的前提）──"
$rangeOk = $false
$totalSize = [int64]0
if ($status -match ' 206 ') {
  $rangeOk = $true
  if ($fields.ContainsKey('Content-Range') -and $fields['Content-Range'] -match 'bytes \d+-\d+/(\d+)') {
    $totalSize = [int64]$Matches[1]
  }
  Write-Host ("  [OK] 返回 206，可分段/续传（总长 {0}）" -f (Fmt $totalSize))
} else {
  Write-Warning "  返回的不是 206 —— 下载器会降级为单流，断流只能从头再来"
}

# ─── 4. 采样 ───────────────────────────────────────────────
Write-Host ""
Write-Host ("── 采样 {0} 次，每次最多 {1} 秒 ──" -f $Samples, $Seconds)
$ttfbs = @()
$speeds = @()
for ($i = 1; $i -le $Samples; $i++) {
  # 用 Range 只取一段，避免把整个文件拖下来；不支持 Range 时 curl 会自己按 --max-time 截断
  $args = @("-s", "-L", "--max-time", "$Seconds", "-o", "NUL",
    "-w", "%{http_code} %{time_starttransfer} %{speed_download} %{size_download}")
  if ($rangeOk) { $args += @("-r", "0-$($RangeBytes * 8 - 1)") }
  $args += $Url
  $out = (& curl.exe @args 2>$null) | Select-Object -Last 1
  $parts = ($out -split '\s+')
  if ($parts.Count -lt 4) { Write-Warning ("  第 {0} 次采样异常: {1}" -f $i, $out); continue }
  $code = $parts[0]
  $ttfb = [double]$parts[1]
  $speed = [double]$parts[2] / 1MB
  $got = [int64]$parts[3]
  $ttfbs += $ttfb
  if ($speed -gt 0) { $speeds += $speed }
  Write-Host ("  #{0}: HTTP {1}  TTFB {2:N2}s  速率 {3:N2} MB/s  实收 {4}" -f $i, $code, $ttfb, $speed, (Fmt $got))
}
$ErrorActionPreference = $prevEap

# ─── 5. 结论 ───────────────────────────────────────────────
Write-Host ""
Write-Host "── 结论 ──"
if ($ttfbs.Count -gt 0) {
  $t = Median $ttfbs
  Write-Host ("  TTFB 中位 {0:N2}s（范围 {1:N2}–{2:N2}s）" -f $t, ($ttfbs | Measure-Object -Minimum).Minimum, ($ttfbs | Measure-Object -Maximum).Maximum)
}
if ($speeds.Count -gt 0) {
  $m = Median $speeds
  Write-Host ("  速率 中位 {0:N2} MB/s（范围 {1:N2}–{2:N2}）" -f $m, ($speeds | Measure-Object -Minimum).Minimum, ($speeds | Measure-Object -Maximum).Maximum)
  if ($totalSize -gt 0) {
    $secs = $totalSize / 1MB / [Math]::Max($m, 0.01)
    Write-Host ("  按中位速率下完 {0}（整包）约需 {1:N1} 分钟" -f (Fmt $totalSize), ($secs / 60))
  }
  if ($m -ge 5) { Write-Host "  [好] 够快，可作为首选源" }
  elseif ($m -ge 1.5) { Write-Host "  [可用] 比现状（GitHub 直连 ~0.1–0.3 MB/s）好，但不算理想" }
  else { Write-Host "  [差] 与现状差别不大，先别切" }
}
Write-Host ("  Range: {0}" -f $(if ($rangeOk) { "支持 206" } else { "不支持（只能单流）" }))
if ($fields.ContainsKey('Age')) { Write-Host ("  缓存: Age={0}（>0 说明命中了边缘缓存）" -f $fields['Age']) }
