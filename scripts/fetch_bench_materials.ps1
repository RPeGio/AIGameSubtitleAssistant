<#
.SYNOPSIS
    下载基准素材（OCR 基准所需的 8 个 mp4，合计约 2.79 GB）。

.DESCRIPTION
    体积原因这些视频不入 git，改由 GitHub Release 资产分发。本脚本把它们下载到
    examples/benchmark_examples/，并按 benchmark/materials.sha256 校验 SHA256 与字节数。

    · 已存在且校验通过的文件**跳过**（幂等，可反复运行）
    · 下载到 *.part 再改名，中断不会留下被误认为完整的文件
    · 校验失败会删除该文件并以非零码退出

    **融合基准不需要这些视频**——它只读 .gsa 的 tracks 与参考文本，二者已随 git 分发。
    只有 OCR 基准（bench_corpus / bench_hardsub / bench_pierro_embed_ocr）需要。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/fetch_bench_materials.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/fetch_bench_materials.ps1 -Force

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/fetch_bench_materials.ps1 -Only "*vesna*"
#>
[CmdletBinding()]
param(
    # Release 标签（资产 URL 的一部分）
    [string]$Tag = 'bench-materials-v1',
    # 仓库（owner/name）
    [string]$Repo = 'RPeGio/AIGameSubtitleAssistant',
    # 目标目录，默认 <repo>/examples/benchmark_examples
    [string]$Dest,
    # 只取名字匹配该通配符的文件（可多次传入）
    [string[]]$Only,
    # 忽略已存在且校验通过的文件，强制重新下载
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'Continue'

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Dest) { $Dest = Join-Path $repoRoot 'examples/benchmark_examples' }
$manifest = Join-Path $repoRoot 'benchmark/materials.sha256'

if (-not (Test-Path $manifest)) { throw "找不到清单：$manifest" }
if (-not (Test-Path $Dest)) { New-Item -ItemType Directory -Path $Dest -Force | Out-Null }

# 清单格式：<sha256>  <字节数>  <release 资产名>  <落地文件名>（# 开头为注释）
# 两列名字通常不同：GitHub 会改写 release 资产名（实测括号 () 被替换为 . ），
# 故必须按「资产名」拼下载 URL，按「落地文件名」保存——否则 404。
$entries = foreach ($line in Get-Content $manifest) {
    $t = $line.Trim()
    if (-not $t -or $t.StartsWith('#')) { continue }
    $parts = $t -split '\s+', 4
    if ($parts.Count -lt 4) { throw "清单行格式错误（应为 4 列）：$line" }
    [pscustomobject]@{
        Sha256 = $parts[0].ToLower(); Size = [int64]$parts[1]
        Asset  = $parts[2]; Name = $parts[3]
    }
}
if ($Only) {
    $entries = $entries | Where-Object { $n = $_.Name; @($Only | Where-Object { $n -like $_ }).Count -gt 0 }
}
if (-not $entries) { throw '清单里没有匹配的条目（检查 -Only）' }

Write-Host ''
Write-Host '基准素材（OCR 基准）' -ForegroundColor Cyan
Write-Host "  来源  https://github.com/$Repo/releases/tag/$Tag"
Write-Host "  目标  $Dest"
Write-Host "  条目  $($entries.Count) 个，合计 $([math]::Round((($entries | Measure-Object Size -Sum).Sum / 1GB), 2)) GB"
Write-Host ''

$failed = @()
foreach ($e in $entries) {
    $target = Join-Path $Dest $e.Name
    $label = '{0,-46}' -f $e.Name

    # 已存在且校验通过 ⇒ 跳过
    if ((Test-Path $target) -and -not $Force) {
        $fi = Get-Item $target
        if ($fi.Length -eq $e.Size) {
            $h = (Get-FileHash -Algorithm SHA256 -Path $target).Hash.ToLower()
            if ($h -eq $e.Sha256) {
                Write-Host "  $label 已就绪（校验通过）" -ForegroundColor DarkGray
                continue
            }
            Write-Host "  $label 校验不符，重新下载" -ForegroundColor Yellow
        }
        else {
            Write-Host "  $label 体积不符（$($fi.Length) ≠ $($e.Size)），重新下载" -ForegroundColor Yellow
        }
    }

    # 资产名含括号等字符，URL 路径段直接使用原名（RFC 3986 允许括号）
    $url = "https://github.com/$Repo/releases/download/$Tag/$($e.Asset)"
    $part = "$target.part"
    if (Test-Path $part) { Remove-Item $part -Force }

    Write-Host "  $label 下载中（$([math]::Round($e.Size / 1MB, 1)) MB）…"
    try {
        Invoke-WebRequest -Uri $url -OutFile $part -UseBasicParsing
    }
    catch {
        Write-Host "  $label 下载失败：$($_.Exception.Message)" -ForegroundColor Red
        if (Test-Path $part) { Remove-Item $part -Force }
        $failed += $e.Name
        continue
    }

    $got = (Get-Item $part).Length
    $hash = (Get-FileHash -Algorithm SHA256 -Path $part).Hash.ToLower()
    if ($got -ne $e.Size -or $hash -ne $e.Sha256) {
        Write-Host "  $label 校验失败（体积 $got/$($e.Size)，sha256 $($hash.Substring(0,12))/$($e.Sha256.Substring(0,12))）" -ForegroundColor Red
        Remove-Item $part -Force
        $failed += $e.Name
        continue
    }
    Move-Item -Path $part -Destination $target -Force
    Write-Host "  $label 完成并校验通过" -ForegroundColor Green
}

Write-Host ''
if ($failed.Count) {
    Write-Host "失败 $($failed.Count) 个：" -ForegroundColor Red
    $failed | ForEach-Object { Write-Host "  · $_" }
    Write-Host ''
    Write-Host '可重跑本脚本（已成功的会跳过）；若资产缺失请检查 tag：' -NoNewline
    Write-Host " gh release view $Tag --repo $Repo" -ForegroundColor DarkGray
    exit 1
}

Write-Host '全部就绪。运行 OCR 基准：' -ForegroundColor Cyan
Write-Host '  cargo test --release --test bench_corpus  -- --ignored --nocapture --test-threads=1'
Write-Host '  cargo test --release --test bench_hardsub -- --ignored --nocapture --test-threads=1'
Write-Host '  cargo test --release --test bench_pierro_embed_ocr -- --ignored --nocapture'
Write-Host ''
