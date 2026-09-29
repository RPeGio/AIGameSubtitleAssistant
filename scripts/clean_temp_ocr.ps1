# 清理 temp/ocr 下的 OCR 帧抽取产物（dev_debug 落盘的诊断帧，单次基准跑可增数百 MB~GB）
#
# 用法：
#   pwsh -File scripts\clean_temp_ocr.ps1              # 清理全部 temp\ocr\* 目录
#   pwsh -File scripts\clean_temp_ocr.ps1 -KeepDays 2  # 只清理 2 天前的
#   pwsh -File scripts\clean_temp_ocr.ps1 -WhatIf      # 只报告不删除
#
# 背景：runtime/config.json 的 dev_debug=true 时，每次 OCR 跑测都会把抽帧落到
# temp/ocr/<uuid>/clip_N/ 供人工检查（保留是刻意设计，便于诊断）；但连续跑基准会
# 堆积数 GB，需定期清理。该目录在 .gitignore 内。
[CmdletBinding()]
param(
    [int]$KeepDays = 0,
    [switch]$WhatIf
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$ocrDir = Join-Path $root 'temp\ocr'

if (-not (Test-Path $ocrDir)) {
    Write-Host "temp\ocr 不存在，无需清理：$ocrDir"
    return
}

$cutoff = (Get-Date).AddDays(-$KeepDays)
$dirs = Get-ChildItem $ocrDir -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.LastWriteTime -lt $cutoff }

if (-not $dirs) {
    Write-Host "没有符合条件（早于 $($cutoff.ToString('yyyy-MM-dd HH:mm'))）的目录。"
    return
}

$totalBytes = 0
foreach ($d in $dirs) {
    $totalBytes += (Get-ChildItem $d.FullName -Recurse -File -ErrorAction SilentlyContinue |
        Measure-Object Length -Sum).Sum
}

if ($WhatIf) {
    Write-Host ("[WhatIf] 将删除 {0} 个目录，释放约 {1:N2} GB" -f $dirs.Count, ($totalBytes / 1GB))
    return
}

foreach ($d in $dirs) { Remove-Item $d.FullName -Recurse -Force -ErrorAction SilentlyContinue }

$remain = Get-ChildItem $ocrDir -Directory -ErrorAction SilentlyContinue
Write-Host ("已清理 {0} 个目录，释放约 {1:N2} GB；剩余 {2} 个。" -f $dirs.Count, ($totalBytes / 1GB), $remain.Count)
