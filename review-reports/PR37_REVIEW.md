# PR #37 代码审查记录（feat/fuse-recall → dev）

- 审查日期：2026-10-10
- 审查范围：PR #37（`feat/fuse-recall` → `dev`），merge-base `4eaaea1` 之后 **27 个提交**、38 个文件（+14413/−105）
- 主题分组：
  - **融合召回工具链**（scripts/ 新增 ~4700 行离线 Python）：`fuse_calib.py`（1091 行，统一转移 DP + 掩码/置信/分段三个默认关闭的判据开关）、`fuse_truth.py`（可接受集合真值）、`fuse_alarm.py`（告警重标）、`fuse_align_srt.py`、`fuse_count_calib.py`、`fuse_unmatch_calib.py`、`fuse_seg_calib{,2}.py`、`gsa_import_embed.py`
  - **OCR 修复**（产品代码）：亚帧过渡帧碎片并前条（D14 盲点）、打字机链内短态不被行边界门切断（D12 收口，`run_text_growing`）
  - **bench 扩展**：vesna 案例（第 4 案例）、语料轴专用排除、期望侧去重（方案 A）、`---` 拆分、新基准 `bench_pierro_embed_ocr.rs`（为 pierro 工程补跑嵌字转写侧）
  - **基准数据入库**：4 份 `.gsa` 工程入库（路径字段清空）+ `.gitignore` negation 规则 + 归档文档（FUSE_THRESHOLD_CALIBRATION 等 1215/227 行）
  - **前端**：OCR 参数跨栏目持久化（修"切栏目静默重置"）、校对区 mock 剧情轨移除
- 审查深度：产品代码（merge.rs 全 diff + pipeline + 新单测）、前端、bench diff 为精读；7 个 fuse_* Python 工具为设计级走查（入口/口径/核心算法抽读，含 `align_v2` DP 全文）。
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2
- 自动验证：`cargo test --lib` **257 通过**；bench 自测 14×2、e2e 按设计 ignored；`vue-tsc --noEmit` 干净

---

## P0（必须修复）

**无**。

---

## P1（建议修复）

### P1-1 `bench_pierro_embed_ocr` 在当前仓库状态下必然 panic —— "可复现"承诺 broken

- **位置**：`src-tauri/tests/bench_pierro_embed_ocr.rs:66-68`（`let video = proj["video"]…; assert!(PathBuf::from(&video).is_file(), "切片视频不存在: {video}")`）与入库的 `examples/benchmark_examples/pierro_questions.gsa`
- **问题**：`6626bdf`（"基准工程与参考文本入库——融合基准可被他人复现"）把 `.gsa` 工程入库时**清空了素材路径字段**（`video`/`source_video`/`path` 均为 `""`，实测确认；清空本身是对的——否则入库本机绝对路径）。但 `bench_pierro_embed_ocr` 从 `proj["video"]` 读切片视频路径且**没有任何回退**（无环境变量覆盖、无 `bench_data_dir` 拼接）——空串 → `is_file()` 恒 false → **该测试一跑就 panic "切片视频不存在: "**。入库素材路径之前作者跑通过（产物已落档），入库清空之后**包括作者在内的任何人都跑不了**。
- **影响**：与 commit 意图（"可被他人复现"）直接冲突；`fuse_lab` 离线验证链的 pierro 规模化数据（`pierro_embed_ocr.json`）无法再生成。
- **修复建议**：支持环境变量覆盖 + 本地回退：

  ```rust
  let video = std::env::var("GSA_PIERRO_VIDEO").unwrap_or_else(|_| {
      // 入库 .gsa 已清空素材路径；本地跑测经 env 指定，或回退到基准素材约定名
      let fallback = root.join("examples/benchmark_examples/quality_bench_test(voiced)_48min.mp4");
      fallback.to_string_lossy().into_owned()
  });
  assert!(PathBuf::from(&video).is_file(), "切片视频不存在: {video}（可设 GSA_PIERRO_VIDEO 指定）");
  ```
  并在头部用法注释补 `GSA_PIERRO_VIDEO`。
- **状态**：⬜ 未修复

---

## P2（优化）

### P2-1 `benchmark/README.md` 与数据入库现状矛盾（两处）

- **位置**：`benchmark/README.md:19`
- **问题**：① "**视频与 `.gsa` 工程不入库**……以该文件为受控副本，`.gsa` 仅作本地留档"——`6626bdf` 已把 4 份 `.gsa` 入库（`.gitignore` 加了 `!examples/benchmark_examples/*.gsa` negation 并带理由注释，另显式忽略已弃用的 `pierro_fixed.gsa`）；"硬编码坐标为受控副本"的表述也未涵盖新入口 `bench_pierro_embed_ocr`（它从 `.gsa` 读选区）。② README 的"基准与跑法"表未列 `bench_pierro_embed_ocr` / `fuse_lab` 三个入口（后者有自己的 README，可链接说明）。
- **建议**：README 该段改写为现状——".gsa 工程入库（素材路径字段清空；视频仍本地）；语料/嵌字两轴选区坐标硬编码于 `tests/common/mod.rs`，pierro 嵌字补跑基准从 `.gsa` 读选区"；补一行新基准的运行方式。
- **状态**：⬜ 未修复

### P2-2 `usePersistedOcrParams` 的持久化键不含项目维度（参数跨项目共享）

- **位置**：`src/composables/ocrDefaults.ts`（`gsa.ocrParams.corpus` / `gsa.ocrParams.asr` 两个全局键）
- **问题**：所有项目共享同一份 OCR 参数。设计取向本身合理（同一用户的工作流参数跨项目延续，vesna 的教训是"参数考据不出来"，持久化已根治）；但切换到素材差异大的项目时，用户可能忘了 `min_subtitle_sec` 是上个项目调的。
- **建议**：可保持现状；若要更稳，可按项目路径做键（或仅在状态栏显示当前生效值）。记录备查。
- **状态**：🟢 保持现状

### P2-3 标定脚本的迭代版本并存（`fuse_seg_calib.py` 577 行 + `fuse_seg_calib2.py` 745 行）

- **位置**：`scripts/`
- **问题**：§7.7 的两代标定脚本并存，功能高度重叠；后续读者不知以哪个为准。
- **建议**：在文件头 docstring 相互标注（"已被 fuse_seg_calib2 取代/保留作历史对照"），或在 `benchmark/FUSE_THRESHOLD_CALIBRATION.md` 里写明两份的关系。
- **状态**：⬜ 未修复（低成本文档项）

### P2-4 素材命名与实际时长不符（vesna 语料片）

- **位置**：`tests/common/mod.rs` 的 `VESNA` 配置注释 vs 文件名
- **问题**：`corpus_video = "pv_reaction_vesna_corpus(voiced)_12min.mp4"`，但注释写"语料片 191.0s"（≈3.2min）且选区 end=190.997——文件名"12min"疑似沿用了切片（clip）的命名口径，易误导。
- **建议**：素材文件重命名或注释里写明"文件名为 12min 系沿用切片命名，语料片实际 191s"。
- **状态**：🟢 记录备查

---

## 审查通过的部分（亮点，无需改动）

- **`ocr/merge.rs` 亚帧过渡帧吸收（方向 1，D14 盲点修复）**：判据（残留头 + 其余行与后条**同序号行**逐行弱关联、行数防护）与**吸收方向**的论证质量极高——① 参考真值包含关系（碎片属前条尾段，改并后条会让 Δend 从 −0.114s 恶化到 −0.58s 超容差，净 −0.4 分属回归）；② 用户在 GUI 手动合并正是并前条；③ "为什么不是亚帧就无条件并"给了 4 条**同为亚帧但不得并**的语料反例（保护"语料产出逐字节不变"的长期硬门）。`comparable_line`（D9 + 长度 ≥2 门）与 `line_weakly_related`（子序列/模糊前缀/重叠率三判据）抽成公共实现，两条吸收路径同源同优先级——正确的重构方向。
- **D12 收口 `run_text_growing`**：以"首帧→末帧**净**增长"判打字机链（OCR 抖动只 ±1~2 字符不构成净增量），方向保守（false 时与引入前**逐位相同**）；`merge_frames` 的 stable_boundary 门与它组合后 pierro 实测 1167.99 双段问题消除。仅 `merge_frames` 可用的定位（run 内轨迹）在注释里写明。
- **前端参数持久化**：修复了一个**真实的高害缺陷**（组件局部 ref → 切栏目即静默回默认 → 用户以为在用调好的参数，产出不可信且无从复现——vesna 案例的阈值就是因此考据不出来）。修法讲究：按**栏目分别**持久化（语料/嵌字参数本就该分开）、逐字段合并（新参数自动补默认值）、损坏退默认、存储不可用静默降级。
- **bench 口径成熟化**：期望侧去重方案 A（PR31 报告 P1-1 的正解落地：参考按实况片校对会有同文双时间轴，期望侧 trim 后全等去重、折叠数单列）；**语料轴专用排除**（`corpus_excluded_refs` 与嵌字轴的 `excluded_refs` 分开——"中文 PV 结构上产不出英文语气词"，判据是"不含 CJK 字符"而非"是英文"，为将来多语种留了正确扩展点）；`---` 块内拆分（中英非 1:1，与 `fuse_truth.py` 的 PART_SEP 同约定）；`drop_refs_by_list` 按下标降序删除防左移。
- **`fuse_calib.py` 的 `align_v2`（统一转移 DP）**：四种转移并列（前进/复用/回退/不配）把"序"与"重数"假设分离；**`--selfcheck` 断言 `repeat=reset=inf` 时与严格递增 DP 逐段等价**——"新结构不破坏旧行为"的长期护栏；复杂度保持 O(n·m)（前缀/后缀一趟最大）；B 的有界性用线性累积罚分实现（无需额外状态）；C 的"已知语义边界"（拖进度条回退）如实声明，未过度声称。这是融合管线重构（F13 证明现行 LLM 融合只是按下标对齐）的判据来源，方向正确。
- **`fuse_truth.py` "可接受集合"**：真值从"函数"改"关系"（多块均可接受、归位结果并入原集合），支持中英非 1:1 素材——与 `---` 拆分同一认知，两侧一致。
- **`fuse_alarm.py`**：明确"只有一种输入口径（消费管线产出，不做输入卫生）——互为前缀合并属 OCR 合并层 raw 缺陷，应在管线层修"——**符合 AGENTS.md 的素材责任边界**；历史数字不精确时重测而不改写历史记录。
- **`.gsa` 入库卫生**：三个路径字段清空（无本机路径泄露）；`.gitignore` negation 规则带"为什么必须入库"的注释，且 `*.gsa` 例外规则写在 `*.gsa` 之后（git 生效顺序）并显式忽略弃用工程。
- **提交纪律**：27 个提交继续按 §7.x/T4c/D14/D17 编号推进；默认关闭的特性开关（UNMATCH_FILTER/SEGMENTED_FILTER/置信计分/重播分段）风格统一，每个都附标定依据；`a02d189` 把素材责任边界写进 AGENTS.md。

---

## 结论

**建议合并（P1-1 修复后合并更佳，也可合并后立即修）**。产品代码改动小而扎实（两个 OCR 合并判据修复 + 316 行新单测），主体是融合召回的判据工具链与基准数据工程化。27 个提交延续了 PR34 的实验纪律：判据默认关闭 + 标定依据 + selfcheck 等价护栏 + 反例论证。验证：`cargo test --lib` 257 通过、bench 自测 14×2、`vue-tsc` 干净。

待办优先级：P1-1（bench_pierro_embed_ocr 视频回退，约 5 行）→ P2-1（README 现状化）→ P2-3（标定脚本互标）→ P2-2/P2-4 记录备查。
