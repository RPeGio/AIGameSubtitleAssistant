# 基准测试记录

本目录存档质量基准的跑测数据（语料收集 / 嵌字时间轴），按案例一页，随时间追加，作为跨版本的回归对比档案。

## 基准与跑法

| 基准 | 测试 | 输入 | 期望 |
|---|---|---|---|
| ① 语料收集 | `bench_corpus` | 语料片（剧情/游戏录屏） | 参考文本全部条目**去重后**的文本 |
| ② 嵌字时间轴 | `bench_hardsub` | 测试片（实况切片）内嵌字幕 | 参考文本内的**时间轴** |
| ③ LLM 融合 | 暂缓 | 预校对工程 | 见下方占位说明 |

```powershell
cargo test --release --test bench_corpus  -- --ignored --nocapture --test-threads=1
cargo test --release --test bench_hardsub -- --ignored --nocapture --test-threads=1
```

环境变量：`GSA_BENCH_TOLERANCE_SEC`（嵌字基准时间容差，默认 0.3s——1s 级偏差对字幕生产已是严重偏离）。
数据在 `examples/benchmark_examples/`（**视频与 `.gsa` 工程不入库**——二者均为本地素材；仅参考文本 `*_reference.txt` 随 git 分发）。案例选区坐标已硬编码在 `src-tauri/tests/common/mod.rs`，以该文件为受控副本，`.gsa` 仅作本地留档。

素材或 OCR 环境缺失时测试打印 `[跳过]` 并正常结束；**素材与环境齐备后流水线自身报错则测试失败**（不再以 `[跳过]` 掩盖真实回归）；参考文本的时间码若非法（帧号越界 / 分隔或段数有误）会带行号直接报错。

## 评分（扣分制，0–100）

总分为均值式：**得分 = 100 − 100 × Σ扣分 ÷ 期望条目数**——按条目数归一，错误率相同则得分相同，与视频长短无关（10 条缺 1 与 200 条缺 20 同为 90 分）。

**语料收集**：参考条目与 OCR 语料做顺序保持的文本对齐（宽松归一化：去空白 + 全角折叠 + 去标点）。期望侧在**对齐前按 trim 后全等文本去重**（保留首现）——参考按实况片校对，主播切页回放会让同一对话出现两条时间轴，与产物侧 `pushCorpusTexts` 的去重语义镜像（见 [review-reports/BENCH_SCORING_DUPLICATE_EXPECTED.md](../review-reports/BENCH_SCORING_DUPLICATE_EXPECTED.md)）；折叠数在跑测摘要中单列。

| 判定 | 扣分 |
|---|---|
| 正确（相似度 ≥ 0.98） | 0 |
| 轻度错误（0.7 ≤ 相似度 < 0.98） | 1 − 相似度 |
| 严重错误（相似度 < 0.7） | 1 |
| 缺失（无配对） | 1 |

**嵌字时间轴**：期望条目与产出段做时间重叠对齐。d = max(|Δstart|, |Δend|)。

| 判定 | 扣分 |
|---|---|
| d ≤ 0.5×容差 | 0 |
| 0.5×容差 < d ≤ 容差 | 线性 0..1 |
| d > 容差 | 1 |
| 碎片化（一条期望被 N 条产出覆盖） | (N−1)×0.5 |
| 被吞并 / 缺失 | 1 |

**不计分的统计项**：语料噪音行（产物多余，案例3 的噪音留作融合容忍度验证素材）、嵌字噪音段、选区时间窗未覆盖的期望条目、对齐对文本相似度（仅报告）。

## 记录规范

跑完基准后，把终端打印的 markdown 行粘贴到对应案例页对应小节的表尾，并补齐「日期」「commit」「备注」三列。每行代表一次完整跑测；改动流水线后重跑即可对比回归。

## 案例页

- [moon_sisters.md](moon_sisters.md) — 5min 有配音 4 说话人（语料无噪）
- [glupov.md](glupov.md) — 11min 无配音 1 说话人
- [pierro_questions.md](pierro_questions.md) — 48min 有配音 4 说话人（语料带噪）

## 管线缺陷报告

基准暴露的 OCR 管线问题汇总（碎片化、段尾延伸、语料缺失、字符精度等，含证据与修复方向）：[OCR_PIPELINE_DEFECTS.md](OCR_PIPELINE_DEFECTS.md)。跑测日志存于 `log/`。

## 融合基准（占位）

LLM 融合基准暂缓：待语料与转写链路调试稳定后，在预校对工程（.gsa，语料准确、各轨角色与时间轴对齐）上启用。已定稿的设计：逐段索引对齐（融合输出时间轴恒等于输入段），按段判类 correct_replaced / correct_kept / missed_replacement / wrong_line / character_error，指标为替换准确率 + 角色名准确率 + failed_batches。
