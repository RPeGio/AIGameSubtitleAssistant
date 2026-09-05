# PR #23 代码审查记录（feat/ocr → master）

- 审查日期：2026-09-05
- 审查范围：PR #23（`feat/ocr` 分支）的 OCR 打轴精化两个提交 `eedcc3e`（阶段1 帧级主边界）+ `f0c836b`（阶段2 窗口内短字幕召回），相对 `06dfd0b` 共 4 文件、+325 行
- 涉及文件：`src-tauri/src/ai_runtime/dhash.rs`、`src-tauri/src/ocr/mod.rs`、`src-tauri/tests/ocr_e2e.rs`、`.gitignore`
- 验证：`cargo check` 通过；`cargo test --lib` 143 个单测全部通过（含新增 refine_window / boundary_indices / clamp_segment / merge_similar 系列）
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2

---

## P0（必须修复）

**未发现**。核心逻辑推演成立：`detect_changes` 为每个 changed 帧记录 `base_hash`（上一段代表哈希）；`boundary_indices` 相对基准累计判定（`dhash.rs:100`）；主边界取 `boundaries.last()` 而非首个 main，正确规避 A→短字幕→C 时把边界误设到短字幕开头；精化仅把 changed 帧时间前移且在 `[lo,hi]` 内，不破坏 `merge_frames` 的按 time 升序契约；`clamp_segment_times` 正确解决精化后起点前移与网格 `end` 的交叉。

---

## P1（建议修改）

### P1-1 每个 changed 帧触发一次 FFmpeg 抽帧 + 密集 dHash + 可能一次 OCR —— 性能瓶颈

- **位置**：`src-tauri/src/ocr/mod.rs`（`refine_window_changes`，已从内联块提取为 `pub` 函数）
- **问题**：对 `changes` 里**每一个** changed 帧，都调用 `extract_frames`（`video/mod.rs:272`，内部 spawn 一个 `ffmpeg` 子进程做 `-ss` seek 抽帧）计算密集窗口 dHash。对 2 小时、0.5s 网格素材，changed 帧可达数百至上千 → 数百上千次 FFmpeg 子进程 + 每窗 15~30 帧 dHash。网格变化检测原本为**减少** OCR 调用，而精化给每个变化都加了一次**昂贵的逐帧抽帧**，开销可能超过省下的 OCR。
- **修复建议**：按变化聚合抽帧（一个窗口一次连续抽取，而非逐 changed 帧重复 seek）；或限制精化只作用于"可疑"变化；或降低密集间隔。
- **状态**：✅ 已修复（整段一次性抽密帧 + 按时间切片 + 帧数上限守卫；实测精化 28.4s→10.4s，e2e 全链路 57.0s→39.6s，事件输出无回归）

### P1-2 `hashes` 用 `filter_map` 构建，与 `dense_frames` 索引错位

- **位置**：`src-tauri/src/ocr/mod.rs`（`refine_window_changes` 的窗口构建）
- **问题**：`hashes` 经 `filter_map` 过滤掉 dHash 失败的帧，长度可能小于 `dense_frames`。但 `boundaries` 的索引（来自 `hashes`）被直接用于：① `new_time = lo + last * dense_interval`；② `dense_frames.get(mid)`。任一密帧 dHash 失败即索引错位，导致边界时间算错、召回帧取错。罕见但静默产生错位字幕边界。
- **修复建议**：`filter_map` 保留原始下标（`enumerate` 收集 `(idx, hash)`），或任一帧失败时整体跳过该窗口精化（fail-closed）。
- **状态**：✅ 已修复（窗口改为 `(time, hash, path)` 元组，与 `hashes` 同步 `filter_map` 构建——失败帧同时从两者丢弃，`boundaries`/`window` 下标始终对齐；`new_time` 改用窗口内实际帧时间而非 `lo + last*interval`）

---

## P2（优化）

### P2-1 `similar_text` 前缀匹配可能过度合并

- **位置**：`src-tauri/src/ocr/mod.rs:150-152`
- **问题**：`a.starts_with(b) || b.starts_with(a)` 较激进，两条本应独立的连续字幕若一条是另一条前缀（如"派蒙" 与 "派蒙：旅行者你来了"），会被 `merge_similar_adjacent` 合并。对同句碎片合理，但可能吞掉真实两行。建议前缀匹配加长度/置信度约束。
- **状态**：✅ 已修复（仅当较短文本 ≥ 较长文本 1/3 时判定前缀，既保留"旅行者，你来了"→"旅行者，你来了。前方似乎有东西在等待。"（37%）这类渐进文本，又拒绝"派蒙"（22%）这类过短前缀；`test_merge_similar_prefix_extends` 通过）

### P2-2 `merge_similar_adjacent` 取"更长文本"但未校验置信度

- **位置**：`src-tauri/src/ocr/mod.rs:334-354`
- **问题**：合并时只按字数选更长，可能用更长但置信度更低的乱码文本覆盖干净短的文本。建议文本更长且置信度不低于原段时才替换，或复用 `vote_text` 多数投票。
- **状态**：✅ 已修复（仅当更长且置信度不低于原段时才替换文本）

### P2-3 `run_ocr` 内部重复探测视频元数据取 fps

- **位置**：`src-tauri/src/ocr/mod.rs`（`run_ocr` 命令）
- **问题**：`run_ocr` 在 `spawn_blocking` 里又调一次 `get_video_metadata(video_path)` 取 fps。前端已传 `video_w/video_h`（同样来自元数据），可顺带把 fps 作为参数传入，省一次 ffprobe 调用。
- **状态**：✅ 已修复（`run_ocr` 命令新增 `src_fps` 参数，前端 `runOcr` 传 `srcFps: meta.fps`；vue-tsc 通过）

### P2-4 逐帧 `clone` 全部 grid 帧

- **位置**：`src-tauri/src/ocr/mod.rs`（`refine_window_changes`）
- **问题**：循环里对每个 grid 帧 `clone()`（含 `frame.path` 的 String），即使非 changed 且不修改。只有 changed 帧才需 mutable 副本。
- **状态**：⬜ 已回退（尝试改为按值传入 + 原地修改，但**破坏窗口计算**：`lo = changes[idx-1].frame.time` 本应取原始网格时间，按值模式下 `changes[idx-1]` 已被上一轮改为精化后的更早时间，导致后续窗口偏移、边界检测/短字幕召回改变（实测 clip2 短字幕 4→7、耗时 5.95→9.5s）。clone 成本本就可忽略（约 205 帧），故回退为 clone 方式）

### P2-5 `boundaries.len() == 2` 硬编码恰好两次变化

- **位置**：`src-tauri/src/ocr/mod.rs`（`refine_window_changes`）
- **问题**：窗口内 3 次以上变化（连续多条短字幕）时只取前两个边界召回一段，其余忽略。注释已标注为阶段2 已知限制，后续可遍历边界对全部召回。
- **状态**：⬜ 已回退（尝试改为 `boundaries.windows(2)` 遍历全部边界对召回，但**严重回归**：每个召回都触发一次慢速 OCR IPC，实测事件数 32→63（翻倍）、e2e 耗时 39.6s→117.6s（3 倍）。故保持保守的 `len==2`，多突变/多短字幕召回留给后续更精确判定）

### P2-6 `.gitignore` 的 `.zcode` 与 OCR 功能无关

- **位置**：`.gitignore:32`
- **问题**：新增 `.zcode` 为开发工具目录，与本 PR 的 OCR 逻辑无关。建议作为独立 commit 或说明来源。
- **状态**：⬜ 未修复

---

## ✅ 通过部分

- **单元测试覆盖扎实**：`test_refine_window_*`、`test_boundary_indices_*`、`test_clamp_segment_times_*`、`test_merge_similar_adjacent_*` 覆盖主边界、无变化、短字幕、渐变累计、无重叠、同类合并、异类不合并等关键分支，全部通过。
- **降级处理稳健**：密集抽帧失败 → 保持网格时间；短字幕 OCR 失败或文本为空 → 跳过；`src_fps` 探测失败 → 关闭精化。
- **`#[allow(clippy::too_many_arguments)]` 有明确理由注释**，非无脑豁免。
- **注释质量高**：A→短字幕→C 的"误用 main 边界 bug"推理在代码注释里讲清楚。
- **向后兼容**：`FrameChange` 新增 `base_hash` 为 `Option`，对旧调用方无破坏；`ocr_e2e.rs` 已同步传入 `meta.fps`。

## 修复优先级建议

P1-1（性能，长视频实际瓶颈，需谨慎重试并实测）→ P1-2（dHash 失败静默错位，加 fail-closed 守卫成本低）→ P2 各项按需。

---

## 本轮修复实施记录（2026-09-05）

### 重构
- 把内联的精化循环提取为 `pub fn refine_window_changes(...)`（`src-tauri/src/ocr/mod.rs`），行为保持，便于集成测试/基准直接调用。
- 新增 `src-tauri/tests/bench_refinement.rs`（`#[ignore]` 基准，运行 `cargo test --release --test bench_refinement -- --ignored --nocapture`），隔离测量精化耗时。

### P1-1 性能优化
- **方案**：整段一次性抽密帧（每 clip 一次 FFmpeg 调用）替代逐 changed 帧抽帧，再按时间切片到各窗口；加 `MAX_WHOLE_FRAMES=30000` 帧数上限守卫，超限退回逐窗口抽帧（避免长视频密帧文件爆炸）。
- **数据**（`examples/test(hi-res).mp4` 4 段选区，`frame_interval=1.0`）：
  - 成本分解（clip 2）：`extract_frames` 70 次调用 10.27s（占精化 70%）vs 整段抽帧 1.67s（6 倍快），dHash 2100 帧 2.99s。**FFmpeg 抽帧是绝对瓶颈**。
  - 精化耗时：28.4s → 10.4s（**约 2.7 倍**）。
  - e2e 全链路：57.0s → 39.6s（**整体提速 30%**），事件数 32 不变，首 10 条事件时间与基线一致（无输出回归）。

### P1-2 索引错位
- 窗口改为 `Vec<(time, hash, path)>`，与 `hashes` 用同一个 `filter_map` 同步构建——任一帧 dHash 失败时该帧同时从两者丢弃，`boundaries`/`window` 下标始终对齐；`new_time` 改用窗口内实际帧时间（`window[last].0`）而非 `lo + last*interval`，整段切片时窗口首帧未必恰在 lo，更精确。

### 验证
- `cargo check` 通过；`cargo test --lib` 143 单测全部通过；e2e `test_t3_end_to_end` 通过且输出无回归。

