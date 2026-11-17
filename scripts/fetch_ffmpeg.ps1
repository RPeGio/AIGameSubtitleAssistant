<#
fetch_ffmpeg.ps1 —— 取固定版本的 ffmpeg（LGPL 共享构建），落进 src-tauri/resources/ffmpeg/

为什么是「共享构建」：官方与各三方打包站默认给的多是 GPL 静态构建（含 libx264 等 GPL 组件）。
把它随 app 分发等于让整个 app 受 GPL 约束。共享构建（-lgpl-shared）只链接 LGPL 部分，
且以「可替换的 DLL」形态出现，满足 LGPL 第 4 条最省事的那条路（见 THIRD_PARTY.md）。

为什么固定 tag 而不是 latest：BtbN 的 latest 是滚动 tag，同一天再拉可能就是另一个二进制；
这里 pin 到某个 autobuild 日期 tag + asset 名 + SHA256 三元组，任何人任何时候跑都得到同一份文件。

产物（都在 -Dest 目录下，平铺）：
  ffmpeg.exe  ffprobe.exe
  avcodec-*.dll avformat-*.dll avutil-*.dll avfilter-*.dll avdevice-*.dll swscale-*.dll swresample-*.dll
  LICENSE.txt（LGPL 全文，必须随包）  SOURCE.txt（来源留痕：URL / SHA256 / 许可）
打包时整个目录进 $RESOURCES/resources/ffmpeg/（见 tauri.conf.json 的 bundle.resources），
Rust 侧由 video::ffmpeg_locator 优先从这里找，找不到才退回系统 PATH。

参数：
  -Dest   落地目录，默认 <仓库>/src-tauri/resources/ffmpeg
  -Mirror 代理前缀，默认 gh-proxy；传空串 "" 表示直连（GitHub 直连不稳时用前缀）
  -Cache  下载缓存目录，默认 <仓库>/temp/ffmpeg-cache（命中哈希就复用，不重下）
  -Force  即使目标目录已就绪也重新解压

用法：
  powershell -ExecutionPolicy Bypass -File scripts/fetch_ffmpeg.ps1
  powershell -ExecutionPolicy Bypass -File scripts/fetch_ffmpeg.ps1 -Mirror "" -Force
#>
param(
  [string]$Dest = "",
  [string]$Mirror = "https://gh-proxy.com/",
  [string]$Cache = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.ServicePointManager]::SecurityProtocol } catch { }

# ─── 版本钉子（升级 = 同时改这五行，并复核 LICENSE 仍是 LGPL）──────────────
$PinTag = "autobuild-2026-10-28-19-47"
$PinAsset = "ffmpeg-n8.1.3-25-g611cec792a-win64-lgpl-shared-8.1.zip"
$PinUrl = "https://github.com/BtbN/FFmpeg-Builds/releases/download/$PinTag/$PinAsset"
$PinSha256 = "0448c3f3b9bdc724cc0189d5ce6e6bfa5b8937e7ec408e1e97fb08719bdb3b92"
$PinSize = 81649021

# 我们只用 ffmpeg/ffprobe；ffplay 要 SDL、体积 18 MB，不进包
$SkipEntries = @("ffplay.exe")

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
if (-not $Dest) { $Dest = Join-Path $root "src-tauri\resources\ffmpeg" }
if (-not $Cache) { $Cache = Join-Path $root "temp\ffmpeg-cache" }
$dest = [System.IO.Path]::GetFullPath($Dest)
$cache = [System.IO.Path]::GetFullPath($Cache)

function Log([string]$msg) { [Console]::Error.WriteLine($msg) }

function Resolve-Url([string]$url) {
  # 与 bootstrap_runtime.ps1 同一口径：gh-proxy 是「前缀」不是基址
  if (-not $Mirror) { return $url }
  if ($url -match "^https?://") { return "$Mirror$url" }
  return "$($Mirror.TrimEnd('/'))/$($url.TrimStart('/'))"
}

function Get-Sha256([string]$path) {
  return (Get-FileHash -Path $path -Algorithm SHA256).Hash.ToLower()
}

# ─── 已就绪就跳过（-Force 除外）：比较来源留痕里的哈希，能发现「换了版本但没重解压」
$sourceFile = Join-Path $dest "SOURCE.txt"
$ready = (Test-Path (Join-Path $dest "ffmpeg.exe")) -and (Test-Path (Join-Path $dest "ffprobe.exe"))
if ($ready -and -not $Force -and (Test-Path $sourceFile)) {
  $stamped = (Get-Content $sourceFile -Raw -Encoding UTF8) -match [regex]::Escape($PinSha256)
  if ($stamped) {
    Log "已就绪：$dest（SHA256 与钉子一致，跳过；要重解压加 -Force）"
    exit 0
  }
  Log "目标目录已有旧版本（SOURCE.txt 里的哈希不是 $PinSha256），将重新解压"
}

New-Item -ItemType Directory -Force -Path $dest, $cache | Out-Null

# ─── 下载（缓存命中则复用一个字节都不重下）────────────────────────────────
$zipPath = Join-Path $cache $PinAsset
$reuse = $false
if (Test-Path $zipPath) {
  if ((Get-Sha256 $zipPath) -eq $PinSha256) {
    $reuse = $true
    Log "缓存命中：$zipPath（$([math]::Round((Get-Item $zipPath).Length / 1MB, 1)) MB）"
  } else {
    Log "缓存文件哈希不符，丢弃重下：$zipPath"
    Remove-Item $zipPath -Force
  }
}

if (-not $reuse) {
  $url = Resolve-Url $PinUrl
  Log "下载 $url"
  $wc = New-Object System.Net.WebClient
  $wc.Headers.Add("User-Agent", "gsa-fetch-ffmpeg/1")
  try {
    $task = $wc.DownloadFileTaskAsync([uri]$url, $zipPath)
    $lastPct = -1
    while (-not $task.IsCompleted) {
      Start-Sleep -Milliseconds 300
      $got = 0
      try { if (Test-Path $zipPath) { $got = (Get-Item $zipPath).Length } } catch { }
      if ($got -gt 0) {
        $pct = [math]::Round(($got / $PinSize) * 100, 1)
        if ($pct -gt $lastPct) {
          Log ("  下载中 {0}/{1} MB（{2}%）" -f [math]::Round($got / 1MB, 1), [math]::Round($PinSize / 1MB, 1), $pct)
          $lastPct = $pct
        }
      }
    }
    $task.GetAwaiter().GetResult()
  } finally {
    $wc.Dispose()
  }
}

$actual = Get-Sha256 $zipPath
if ($actual -ne $PinSha256) {
  Log "校验失败：期望 $PinSha256，实际 $actual"
  exit 2
}
Log "SHA256 校验通过"

# ─── 解压：只取 bin/ 下的文件（平铺）+ LICENSE.txt ─────────────────────────
Add-Type -AssemblyName System.IO.Compression.FileSystem
$z = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
$copied = 0
try {
  foreach ($e in $z.Entries) {
    if ($e.Length -eq 0 -and $e.FullName.EndsWith("/")) { continue }
    $parts = $e.FullName.Split("/")
    $name = $parts[-1]
    $isBin = ($parts.Length -ge 2) -and ($parts[-2] -eq "bin")
    $isLicense = ($parts.Length -eq 2) -and ($name -eq "LICENSE.txt")
    if (-not ($isBin -or $isLicense)) { continue }
    if ($SkipEntries -contains $name) { continue }
    $target = Join-Path $dest $name
    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($e, $target, $true)
    $copied++
  }
} finally {
  $z.Dispose()
}
Log "解压完成：$copied 个文件 → $dest"

# ─── 来源留痕（同时也是「已就绪」判据）──────────────────────────────────
$stamp = @"
source   : $PinUrl
asset    : $PinAsset
tag      : $PinTag
sha256   : $PinSha256
size     : $PinSize
license  : LGPL v2.1 or later（BtbN -lgpl-shared 构建的 DLL 可替换，见 THIRD_PARTY.md）
fetched  : $(Get-Date -Format "yyyy-MM-ddTHH:mm:ssZ")
note     : 由 scripts/fetch_ffmpeg.ps1 生成；重新运行会覆盖本文件
"@
[System.IO.File]::WriteAllText($sourceFile, $stamp, (New-Object System.Text.UTF8Encoding($true)))

# ─── 真跑一次（解压不完整的 DLL 在这儿就会暴露）───────────────────────────
foreach ($exe in @("ffmpeg.exe", "ffprobe.exe")) {
  $p = Join-Path $dest $exe
  # 不能写成 `& $p -version | Select-Object -First 1`：管道提前关闭会把进程杀掉，
  # $LASTEXITCODE 变成非 0，把「明明跑起来了」误判成跑不起来。先整段收进变量再取首行。
  $out = & $p -version 2>&1
  $code = $LASTEXITCODE
  $line = @($out)[0]
  if ($code -ne 0 -or -not $line) {
    Log "自检失败：$exe 跑不起来（exit=$code，$line）"
    exit 1
  }
  Log "自检通过：$line"
}

$total = (Get-ChildItem $dest -File | Measure-Object -Property Length -Sum).Sum
Log ("完成：{0} 个文件，共 {1} MB → {2}" -f (Get-ChildItem $dest -File).Count, [math]::Round($total / 1MB, 1), $dest)
exit 0
