# 基准记录：moon_sisters（5min 有配音 4 说话人，语料无噪）

数据：`examples/benchmark_examples/` — 测试片 `quality_bench_test(voiced)_5min.mp4`（720p/60fps）、语料片 `quality_bench_test_corpus(voiced)_5min.mp4`、参考 `quality_bench_test(voiced)_5min_reference.txt`（时间码 30fps 基准）、工程 `moon_sisters.gsa`。

## 语料收集

| 日期 | commit | 评分 | 扣分明细 | 噪音行 | 备注 |
|---|---|---|---|---|---|
| 2026-09-10 | | 99.7 | 缺失0 严重0 轻度1 | 2 | 首跑基线（旧参考 17 条） |
| 2026-09-10 | | 89.2 | 缺失2 严重0 轻度1 | 2 | 参考修正（17→19 条）后基线；缺失 [12][14]（被 OCR 并入相邻段） |

## 嵌字时间轴

| 日期 | commit | 评分 | 1:1/碎片/合并/缺失 | Δstart p95 | ≤容差 | 覆盖率 | 噪音段 | 备注 |
|---|---|---|---|---|---|---|---|---|
| 2026-09-10 | | 2.6 | 10/9/0/0 | 1.17s | 0% | 91% | 0 | 参考时间轴修正 + 嵌字选区微调后基线，容差 0.3s（旧行 0.0 分系对错误时间轴测得，已移除） |

## 融合

暂缓，见 [README 占位说明](README.md#融合基准占位)。
