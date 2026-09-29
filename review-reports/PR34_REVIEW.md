# PR #34 代码审查记录（feat/ocr → dev）

- 审查日期：2026-09-29
- 审查范围：PR #34（`feat/ocr` → `dev`），merge-base `978c56f` 之后 **61 个提交**、39 个文件（+7787/−999）
- 主题分组（按工作流编号）：
  - **D1–D17**：OCR 碎片治理与时序精度（相邻段拼接 → 精化/条带 → 短碎片门 → 时序归因）
  - **A4/A5**：参考时基校准（探针 + 产物 + SHA256 守卫）与受控 A/B 实验
  - **R1–R4**：语料纠错产品化（只标记不改动 → 落盘 → 前端审批 → 双实现同步）
  - **S1/S2**：标点归一化 + 术语表纠错
  - **CUDA**：可选 GPU paddle（bootstrap_ocr_gpu + TF32 决策 + det 批 16）
  - **D5 时序归因收口**：终点侧锚点、封盘声明
- 审查深度说明：核心流水线（`ocr/mod.rs` 全部治理遍 + `run_ocr_pipeline` + 命令）、`dhash.rs`、`project/export/paddle` diff、前端纠错链路、`bench_hardsub.rs`、时基基础设施为全文/定点精读；两个 Python 时基脚本与 bootstrap 脚本为设计级走查（离线审计工具，非产品路径）。
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2
- 自动验证：`cargo test --lib` **245 通过**；4 个测试目标编译/自测通过（e2e 按设计 ignored）；`vue-tsc --noEmit` 干净；clippy 较基线**新增 2 处** `unnecessary_map_or`（见 P2-3）

---

## P0（必须修复）

**无**。未发现需要合并前修复的正确性缺陷。

值得专门说明的两个"高危类"均被证实是安全的：

1. **`run_ocr` 返回值改为两元组**（`(segments, diffs)`）：Tauri 把元组序列化为 JSON 数组，前端 `invoke<[OcrSegment[], Diff[]]>` 同步解构（commit `dacfc8f` 正是修复 R1 改签名后的连锁报错），两侧契约一致。
2. **`Project` 新增 `corpus_ocr_diffs`**：`#[serde(default)]` 兼容旧 `.gsa`，并有专门单测守住"save_project 反序列化→写盘"路径不丢该字段（前端审批列表跨会话依赖它）。

---

## P1（建议修复）

**无**。以下 P2 各项均不阻塞合并。

---

## P2（优化）

### P2-1 "采纳替换"规则是前端/基准的**双实现**，存在漂移风险（已有护栏，长期应收敛）

- **位置**：`src/stores/project.ts`（`approveCorpusOcrDiff`）与 `src-tauri/tests/common/mod.rs`（`apply_diffs_to_segments`）
- **问题**：同一条规则（对全部条目做 `old→new` 全量替换，`split/join` ≡ `str::replace`）有两份实现，分属 TS 与 Rust。代码已自标注"**任一侧改动必须同步另一侧，否则基准分数不再代表产品行为**"，且 R3 已用真实数据逐位对齐验证——属**已知的、有护栏的**维护性风险，故定级 P2。
- **建议**：长期把替换逻辑下沉为后端命令（前端审批后调 `apply_corpus_diff`），基准直接调用同一 Rust 实现，消除双实现。

### P2-2 "默认 0.7"陈旧文案（3 处，用户可见）

- **位置**：`src/types/index.ts`（`min_subtitle_sec` 注释）、`CorpusView.vue` 与 `AsrView.vue` 的控件提示（"默认 0.7；实测 1.5 能收掉…"）
- **问题**：两视图的实际默认值与 Rust `DEFAULT_MIN_SUBTITLE_SEC` 均为 **1.5**，输入框显示 1.5，提示却写"默认 0.7"——文案与所见值矛盾（0.7 应为早期默认的残留）。
- **建议**：统一改为"默认 1.5（实测可收掉长打字机/遮挡碎片；调小减少误并，0 关闭）"。
- **状态**：✅ 已修复（types 注释 + 两视图提示统一为"默认 1.5"，措辞与实测依据一致）

### P2-3 clippy 新增 2 处 `unnecessary_map_or`

- **位置**：`src-tauri/src/ocr/mod.rs:474`（`vote_text_with` 的 `winner.map_or(true, …)`）、`:897`（`short_fragment_related` 的 `fl.last().map_or(true, …)`）
- **问题**：新版 clippy 建议 `is_none_or`。与本仓库既有的 6 处同类风格警告一致，非功能问题。
- **建议**：顺手改为 `is_none_or`（各 1 行），或与既有警告一起统一处理。
- **状态**：✅ 已修复（两处改 `is_none_or`，clippy 回到既有基线，无新增警告）

### P2-4 术语表 UI 允许空词条进参数

- **位置**：`src/views/CorpusView.vue`（`addGlossaryTerm` push `""`）
- **问题**：空词条随参数发送，后端 `filter(|t| t.len() >= 2)` 静默忽略——无实际危害（有兜底），但 UI 不提示，用户可能以为"空行也生效"。
- **建议**：提交前过滤空/过短条目或在输入框 blur 时 trim。

### P2-5 两视图各维护一份相同的 `OcrRunParams` 默认值

- **位置**：`CorpusView.vue` 与 `AsrView.vue`（`punctuation`/`glossary` 等字段三处重复）
- **建议**：抽共享工厂（如 `defaultOcrParams(minSubtitleSec?)`），后续加参数只改一处。ASR 页 glossary 固定空的差异用参数表达即可。
- **状态**：✅ 已修复（新增 `src/composables/ocrDefaults.ts`：`createDefaultOcrParams()` + `defaultPunctuation()`，两视图改用；ASR 页保留"术语表固定为空"的说明注释，沿用工厂的空数组缺省）

### P2-6 遗留观察（非本 PR 引入）——本次已顺手修复

- ~~`run_ocr_images` 一次性全量读图进内存~~ → ✅ 已修复：改为**逐 IPC 批读图**（读取→base64→识别→下一批），峰值内存从"全部图片 + base64 放大 1.33×"降为单批用量；进度消息与批语义不变。
- ~~`OcrRunParams.min_subtitle_sec` 无 `#[serde(default)]`~~ → ✅ 已修复：补 `#[serde(default = "default_min_subtitle_sec")]`，与产品默认 `DEFAULT_MIN_SUBTITLE_SEC` 同源，防旧参数缺字段反序列化失败。

---

## 审查通过的部分（亮点，无需改动）

**实验纪律是本 PR 最突出的质量**：几乎每个阈值/判据都有实测依据注释（取值区间、反例、余量倍数）；失效假设被受控实验证伪后**公开更正**（A5 的通道 2 假设、A5-P3"按同构建 A/B 实测修正失效结论"）；有专门的 revert 提交（`214468a` "实证回滚"）与负结果归档（D13 B-iii、D4 负结果）。这让 61 个提交的历史可审计。

按主题：

- **D8 变化检测补漏**（`dhash.rs::change_flags_rescued`）：帧级差分注入（切换点映射其后首个网格样本、多切换取最早、有效时刻覆写与 `sample_time` 分离）+ 静态超时保险丝；4 个单测复刻真实漏检场景（pierro [75]），`stale_timeout≤0` 可关。PR27 审查引入的 `assemble_grid_frames` 对齐守卫与幻影回收被完整保留并正确扩展。
- **碎片治理六遍流水线**（`run_ocr_pipeline` 内顺序：merge_frames → 召回并排序 → merge_contained_adjacent → merge_similar_adjacent → refine_segment_ends → clamp_segment_times → merge_short_fragments_into_next → 标点归一/术语表标记）：**顺序依赖有专述**（P2 注释论证"短碎片吸收必须在段尾精化与 clamp 之后"，含 pierro SRT #111 的 3.63→3.38s 实测反例）；各 pass 判据分层清晰（严格判据+时长门 vs 弱关联+`min_subtitle_sec` 门），护栏齐备（D9 纯标点条绝不并入、前置行一致性防"换说话人误并"、稳定姓名框态不并）。
- **渐进补全偏好**：三道门（`MIN_GAIN=3` 承重并有真实素材正反单测锁死、`CONF_MARGIN` 实测幻觉尾巴低置信、support≥2/末态兜底）+ `GSA_OCR_PROGRESSIVE_COMPLETE` 开关支持同构建 A/B；默认 0.5s 网格下被证明惰性并如实记录。
- **语料纠错产品化（R1–R4）**：后端**只标记不改写**（Diff 结构，`old` 集合语义跨条目全局生效）；`corpus_ocr_diffs` 落盘有 serde 兼容与"保存不丢字段"双测试；前端审批用对象身份定位（防连点误删相邻条）、采纳后按入库同口径去重清理替换产生的重复、撤销快照扩展到三者；`runOcr` 一次 OCR 合成一个撤销步骤（`record=false` 参数化避免双入栈），项目中途关闭的竞态有同口径丢弃说明。
- **时基校准（A4）**：`benchmark/timebase/<case>_timebase.json` 产物 + **SHA256/字节数守卫**（素材一换立即报错，错误信息含重跑指引）+ **质量门**（残差中位、k 的 95%CI 半宽）+ **fit/applied 分离**（重跑探针不会悄悄改分，`--apply` 显式切换）+ 离群点按先验规则（实质文本变化比率 >0.35）而非残差剔除。产物放 `benchmark/timebase/`（入库目录）而非 gitignore 的 `examples/`——正确。
- **嵌字基准报告升级**：三轴并列（结构/时间/复合）、碎片直方图与明细（定性定位合并层缺口）、Δstart/Δend 带符号中位数与偏晚/偏早计数、容差自洽论述（0.6s=2.4×量化）、**分数不写死在代码**（明确指向权威记录文件）、SRT 主观评估产物、pierro 人工 transition 排除计分。
- **CUDA/GPU（可选启用，默认路径零变化）**：`GSA_OCR_DEVICE` 空时不传 device（行为与改前一致）；显式 gpu 但 CUDA 不可用回退 cpu + stderr 提示；**TF32 默认关闭**（换取与 CPU 逐字节一致的产出，量化代价 3.8%，`GSA_OCR_TF32=1` 开关且尊重 `NVIDIA_TF32_OVERRIDE`）；det 批 16（1.62×，对齐 IPC 批）且写明 paddlex 配置替换语义的坑（须 load 官方配置再改）。
- **工程细节**：`PADDLE_PDX_CACHE_HOME`/PYTHONPATH 拼接用 `join_paths`（跨平台分隔符正确）；临时目录 unix 下 0o700；长选区精化加进度聚合上报（消除 546s 静默）；`clean_temp_ocr.ps1` 支持 `-KeepDays/-WhatIf`；bootstrap 脚本仅 pip 下载（官方源 + 可选镜像/代理），无危险执行。
- **提交卫生**：61 个提交按工作流编号清晰分组、revert 有实证记录、文档与代码同步演进（封盘声明、融合基准交接计划）。

---

## 结论

**建议合并**。未发现 P0/P1 级问题；P2 共 6 项均为文案/风格/维护性改进，不阻塞。该 PR 在 61 个提交的规模下保持了罕见的质量水准——尤其"阈值必有实测依据、假设必须受控验证、负结果也归档"的工程纪律，以及基准基础设施的 SHA256 守卫直接把此前审查（PR31）指出的"基线可信度"类问题从机制上关闭。

后续可另行处理：~~P2-2 三处陈旧文案~~、~~P2-3 clippy 两处~~、~~P2-5 默认值工厂~~、~~P2-6 两条遗留~~ 均已修复；剩余 P2-1 双实现收敛（把采纳替换下沉为后端命令，让基准与产品共用一份规则）为长期项；P2-4（术语表空词条）经评估弃置。
