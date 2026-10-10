# 基准测试记录

本目录存档质量基准的跑测数据（语料收集 / 嵌字时间轴），按案例一页，随时间追加，作为跨版本的回归对比档案。

## 基准与跑法

| 基准 | 测试 | 输入 | 期望 |
|---|---|---|---|
| ① 语料收集 | `bench_corpus` | 语料片（剧情/游戏录屏） | 参考文本全部条目**去重后**的文本 |
| ② 嵌字时间轴 | `bench_hardsub` | 测试片（实况切片）内嵌字幕 | 参考文本内的**时间轴** |
| ③ 嵌字补跑 | `bench_pierro_embed_ocr` | pierro 工程切片（`page=asr` 选区） | 补出 `embed_ocr` 转写侧（供融合验证消费） |
| ④ 融合对齐 | `fuse_lab`（Python，非 cargo） | 预校对工程的 `.gsa` + 参考文本 | 跨语言对齐正确率，见 [FUSE_THRESHOLD_CALIBRATION.md](FUSE_THRESHOLD_CALIBRATION.md) |

```powershell
cargo test --release --test bench_corpus  -- --ignored --nocapture --test-threads=1
cargo test --release --test bench_hardsub -- --ignored --nocapture --test-threads=1
cargo test --release --test bench_pierro_embed_ocr -- --ignored --nocapture
```

环境变量：`GSA_BENCH_TOLERANCE_SEC`（嵌字基准时间容差，默认 **0.6s** = 2.4×采样量子；1s 级偏差对字幕生产已是严重偏离）；`GSA_PIERRO_VIDEO`（pierro 切片视频路径，未设时回退约定素材名）。
数据在 `examples/benchmark_examples/`：**`.gsa` 工程与参考文本 `*_reference.txt` 随 git 分发**（`.gsa` 已清洗本机绝对路径，顶层 `path`/`video`/`source_video` 置空串）；**视频不入库**（合计约 2.9 GB），用 `powershell -ExecutionPolicy Bypass -File scripts/fetch_bench_materials.ps1` 从 release 资产下载并按 `benchmark/materials.sha256` 校验。案例选区真值 = 各案例 `.gsa` 内 `type=ocr_region` 且 page 匹配的控制轨事件（`src-tauri/tests/common/mod.rs::regions_from_gsa`，单一来源；PR31 时代的硬编码副本随 `.gsa` 入库退役）。
**融合基准不需要视频**——它只读 `.gsa` 的 tracks 与参考文本，故 clone 即可复现（见 `src-tauri/tests/fuse_lab/README.md`）。

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

## 嵌字跑测报告的三轴结构（阅读顺序）

嵌字基准的终端输出分三段，**必须先读 ① ② 再看 ③**：

```
── ① 结构轴 ──
1:1 10｜碎片 9 条（多出 15 段：2段 6 / 3段 1 / ≥4段 2）｜被吞并 0｜缺失 0｜噪音段 0
── ② 时间轴（仅 1:1 配对，n=10；Δ = 产出 − 参考，+ 表示偏晚/过伸）──
Δstart：中位 +0.203s｜p95(|·|) 1.236s｜最大(|·|) 1.236s｜偏晚 6 / 偏早 4｜容差内 70%
Δend  ：中位 +0.103s｜p95(|·|) 3.499s｜最大(|·|) 3.499s｜过伸 7 / 欠伸 3｜容差内 50%
复合 d=max(|Δstart|,|Δend|)：容差内 50%｜覆盖率 89%｜文本相似度均值 0.413
── ③ 复合评分（沿用旧口径，供跨版本可比）──
21.5/100
```

- **① 结构轴**：碎片条目数 + **多出段数 Σ(N−1)** + 段数直方图 + 被吞并/缺失/噪音。碎片化是嵌字链路的最大结构缺陷（一条长显示被拆成 N 段）。
- **② 时间轴**：只统计 1:1 配对。**带符号**给出中位数（正 = 偏晚/过伸）与偏晚/偏早、过伸/欠伸计数——用于区分"边界偏晚"与"边界偏早/欠伸"两类完全不同的成因；同时给出 **max|·|**（防止小样本下 p95=max 被误读为普遍现象）与**分轴达标率**（d=max 会掩盖是哪一轴不达标）。
- **③ 复合评分**：沿用旧扣分口径，仅用于跨版本可比。

**为什么必须并列读**：复合评分把两个轴压成一个数，且口径不对等——**碎片罚 (N−1)×0.5 低于时间罚 1.0**。因此纯粹的**结构修复（如碎片拼接）可能反而拉低复合评分**：拼接把此前被"碎片罚"掩盖的时间误差暴露为满额时间罚。判断一次改动是否值得，应看 ① 结构是否改善、② 时间中位/分轴达标率是否改善，再决定是否接受 ③ 的短期波动（详见 [OCR_PIPELINE_DEFECTS.md](OCR_PIPELINE_DEFECTS.md) D1/D2/D5）。

## 记录规范

跑完基准后，把终端打印的 markdown 行粘贴到对应案例页对应小节的表尾，并补齐「日期」「commit」「备注」三列。每行代表一次完整跑测；改动流水线后重跑即可对比回归。**备注列应记录 ①/② 的关键变化**（如"碎片 54→39、多出段 71→50；Δend 中位 +0.9→+0.4"），避免只留复合评分而丢失结构/时间信息。

## 案例页

- [moon_sisters.md](moon_sisters.md) — 5min 有配音 4 说话人（语料无噪）
- [glupov.md](glupov.md) — 11min 无配音 1 说话人
- [pierro_questions.md](pierro_questions.md) — 48min 有配音 4 说话人（语料带噪）

## 管线缺陷报告

基准暴露的 OCR 管线问题汇总（碎片化、段尾延伸、语料缺失、字符精度等，含证据与修复方向）：[OCR_PIPELINE_DEFECTS.md](OCR_PIPELINE_DEFECTS.md)。跑测日志存于 `log/`。

## 融合基准（下一个阶段）

> **暂缓（2026-09-30）**：经调研，融合管线**鲁棒性不足**且与现 OCR/ASR 工作流产出**不兼容**——
> 定稿设计的输出契约**强制要求说话人**，而现管线存在无说话人的产出路径（无配音处嵌字段无姓名
> 前缀、ASR 未分离出说话人等），"无说话人"分支未被设计覆盖；管线对真实产出的鲁棒性也未达基准门槛。
> 故**在重新评审设计前提（说话人契约 + 鲁棒性）之前不实施**。暂缓理由、已就绪环境（可复用资产）
> 与恢复前待办见 [OCR_TIMELINE_CLOSURE.md](OCR_TIMELINE_CLOSURE.md) §三「状态」。

**OCR 时间轴已封盘（2026-09-29）**：语料 97.1/100.0/99.6、嵌字 71.5/71.4/71.2（0.5s 网格、容差 0.6s、
逐案例时基）；起止两侧帧级真值归因完毕，0.5s 网格内无通用可修项——封盘声明、分支全部优化清单与
融合基准落地计划见 **[OCR_TIMELINE_CLOSURE.md](OCR_TIMELINE_CLOSURE.md)**。

LLM 融合基准已定稿的设计：在预校对工程（.gsa，语料准确、各轨角色与时间轴对齐）上启用；
**逐段索引对齐**（融合输出时间轴恒等于输入段）、按段判类 correct_replaced / correct_kept /
missed_replacement / wrong_line / character_error，指标为替换准确率 + 角色名准确率 + failed_batches；
展开成可执行步骤的分步计划见 closure 文档 §三）。

### 重评审进展（2026-10-05）

上述暂缓的两条前提已被量化复核，替代路线亦已验证：

- [FUSE_PIPELINE_DEFECTS.md](FUSE_PIPELINE_DEFECTS.md) — F1–F13 缺陷台账（含真实 `.gsa` + 真实 llama-cli 实测）；F13 证明 3B 主行为是「按下标对齐」，跨语言语义对齐未发生。
- [FUSE_VECTOR_RECALL_VALIDATION.md](FUSE_VECTOR_RECALL_VALIDATION.md) — 向量召回（E5-small，CPU）+ 单调 DP 替代 LLM 的离线验证（moon 15/15、glupov 22/22）。
- [FUSE_ALIGNMENT_SCALE_VALIDATION.md](FUSE_ALIGNMENT_SCALE_VALIDATION.md) — **规模化验证**（pierro 48min/148 语料/120 段）：88.3% → **99.2%**，主因是输入卫生（打字机前缀重复段引发 13 段漂移链），而非对齐算法。
- [FUSE_THRESHOLD_CALIBRATION.md](FUSE_THRESHOLD_CALIBRATION.md) — **阈值校准**（三案例独立真值）：DP 参数**无需改**（现行值已在最优平台）；`margin<0.02` 告警建议**删除**（标记率 63.5%、精确率 ≤12%）；`score<0.78` 保留；护栏应移到输入侧。另发现语料近重复行（glupov 12 对）⇒ 评分须用等价类口径。**最新（§7.9，2026-10-10）**：「该不配」top-K 并列簇判据落地（vesna 空集段 12/13、其余三案例假阳性 0）⇒ 四案例端到端 **226/236 (95.8%)**；`score` 阈值 0.78 → **0.86**（召回 30% → 100%，代价是标记率 89.4% ⇒ 清单等价"全量复核"）。

判据与工具：`src-tauri/tests/fuse_lab/`（判别器 + 离线验证流水线）；含「内部句点名字」缺陷的回归素材见 `src-tauri/tests/fuse_lab/fixtures/`。
