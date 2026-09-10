# 基准记录：pierro_questions（48min 有配音 4 说话人，语料带噪）

数据：`examples/benchmark_examples/` — 测试片 `quality_bench_test(voiced)_48min.mp4`（1080p/59.94fps）、语料片 `quality_bench_test_corpus(voiced)_48min.mp4`、参考 `quality_bench_test(voiced)_48min_reference.txt`（时间码 59.94fps 基准）、工程 `pierro_questions.gsa`。

语料片的额外录屏内容会以噪音行形式混入语料——本案例的噪音统计留作日后融合基准的容忍度验证素材（噪音不判败，考核 LLM 是否忽略无关语料行）。

## 语料收集

| 日期 | commit | 评分 | 扣分明细 | 噪音行 | 备注 |
|---|---|---|---|---|---|
| 2026-09-10 | | 95.9 | 缺失3 严重0 轻度19 | 51 | 首跑基线；缺失 [4][71][76] |

## 嵌字时间轴

| 日期 | commit | 评分 | 1:1/碎片/合并/缺失 | Δstart p95 | ≤容差 | 覆盖率 | 噪音段 | 备注 |
|---|---|---|---|---|---|---|---|---|
| 2026-09-10 | | 17.6 | 64/54/1/0 | 1.90s | 6% | 82% | 0 | 首跑基线，容差 0.3s |

## 融合

暂缓，见 [README 占位说明](README.md#融合基准占位)。
