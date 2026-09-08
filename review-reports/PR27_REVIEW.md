# PR #27 代码审查记录（feat/ocr）

- 审查日期：2026-09-04
- 审查范围：PR #27（`feat/ocr`）相对 master merge-base `8e952a1` 的提交
- 提交构成：
  - `4b96cd5` perf(ocr): zero-disk frame scanning — eliminate dense_whole JPEG dumps
  - `24c333c` perf(ocr): base64 image IPC — zero disk writes across the OCR chain
- 改动文件：`src-tauri/src/{video/mod.rs,ai_runtime/{dhash,config,paddle}.rs,ocr/mod.rs}`、`scripts/ocr_worker.py`、`src-tauri/{Cargo.toml,Cargo.lock}`、`src-tauri/tests/ocr_bench_refinement.rs`
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2

---

## P0（必须修复）

### P0-1 网格帧数 `ceil()` 与 ffmpeg 实际输出帧数不一致 —— 幻影变化帧被静默丢弃（尾部字幕切换漏 OCR）

- **位置**：`src-tauri/src/ocr/mod.rs`（网格装配段）；`src-tauri/src/video/mod.rs`（`extract_frames_bytes` 的 ffmpeg `fps=1/interval_secs`）
- **问题**：`grid_frame_count` 用 `ceil(duration/frame_interval)` 估算网格帧数，而 ffmpeg `fps` 滤镜实际输出按 round 舍入。

  **实测证据**（真实 h264 mp4，`-ss 0.5 -t 5 -fps=1/interval -f mjpeg pipe:1`）：

  | interval | `ceil(5/interval)` | ffmpeg 实际输出帧数 |
  |---|---|---|
  | 0.5 | 10 | 10 |
  | 0.3 | 17 | 17 |
  | 0.7 | **8** | **7** |
  | 1.0 | 5 | 5 |

  **审查更正（初版误报了报错路径）**：初版认为超界的 keep 号被丢弃会导致 `ocr_pass` 报"图像数据与变化帧数量不符"。深入推演后确认**该报错路径不成立**——`changes` 由 `(0..grid.total)`（实际帧数）+ `flags.get(k)` 兜底构建，`changed_total` 也来自它；而 `grid.kept` = `keep ∩ [0, grid.total)` 恰好等于 `changes` 中的变化帧集合，两者数量恒等，不会触发校验错误。

  **真实后果**：多出的"幻影"网格帧（号 ≥ 实际帧数，因 round ≤ ceil 至多 1 个）若被判为变化帧，其字节被静默丢弃——即**片段尾部最后不足一个 interval 内的字幕切换，扫描哈希已检测到、却没有帧可 OCR**，旧文本顺延到片段结束。该盲区与旧实现相同（非回归），但零落盘扫描架构本可消除它。
- **修复**：装配逻辑抽成纯函数 `assemble_grid_frames`（`src-tauri/src/ocr/mod.rs`）：
  1. 实际帧（`0..grid.total`）直接装配，`kept_jpegs` 与变化帧数量显式校验（违反即报错，不再可能静默错位）；
  2. 幻影网格帧判为变化时，按网格时间经 `extract_single_frame_bytes` 补抽单帧（追加在末尾保持升序对齐）；补抽失败则该帧同步跳过，对齐不变。
- **状态**：✅ 已修复（新增 6 个单测覆盖回收/跳过/失败对齐/total 一致/错位报错/ocr_pass 契约；`cargo test --lib` 157 通过）

---

## P1（建议修复）

### P1-1 `extract_single_frame_bytes` 未校验 EOI 尾标记

- **位置**：`src-tauri/src/video/mod.rs`（`extract_single_frame_bytes` 末尾）
- **问题**：只校验了 `jpeg.starts_with(&[0xFF, 0xD8])`（SOI 头），未校验 JPEG 是否完整（EOI `FFD9` 结尾）。若 ffmpeg 单帧输出被截断/无效，会产出**无 EOI 的残缺 JPEG**，交给 OCR worker 后 `cv2.imdecode` 返回 `None` → worker 抛 "图像解码失败"，表现为偶发 OCR 失败但报错原因与真实问题不符。
- **修复建议**：校验 `jpeg.ends_with(&[0xFF, 0xD9])`，或复用 `try_take_jpeg` 思路确认完整帧。

  ```rust
  if !jpeg.starts_with(&[0xFF, 0xD8]) || !jpeg.ends_with(&[0xFF, 0xD9]) {
      return Err("ffmpeg 单帧抽取输出不是完整 JPEG".into());
  }
  ```
- **状态**：✅ 已修复（抽纯函数 `is_complete_jpeg`（SOI 开头且 EOI 结尾）供 `extract_single_frame_bytes` 校验 + 1 个单测；残缺帧在 Rust 侧即报错，不再到 worker 的 cv2.imdecode 才误报"图像解码失败"）

### P1-2 `dev_debug` 默认值从 true 改为 false 的兼容性

- **位置**：`src-tauri/src/ai_runtime/config.rs`（`default_dev_debug` 由 `true` → `false`）
- **问题**：这是行为变更。已有用户若依赖 `dev_debug=true`（调试时帧落盘 + 详细日志），升级后默认关闭；而旧 `runtime/config.json` 若显式写了 `dev_debug` 则不受影响，但**未写该字段的旧配置**会从"开启"变"关闭"。属预期的性能/隐私权衡，但应在文档/迁移说明中提及。
- **修复建议**：确认这是有意为之并写进 README/release note；若担心旧行为，可仅在 release 构建默认 false。
- **状态**：⬜ 未修复（建议确认意图）

### P1-3 `run_ocr_images` 一次性读全部图片字节进内存

- **位置**：`src-tauri/src/ocr/mod.rs`（`run_ocr_images`）
- **问题**：图片列表改为逐个 `std::fs::read` 并 base64。若用户一次选几十张高分辨率截图，每张 RGB 几十 MB，base64 后更大，`images` 向量一次性驻留内存。`recognize_batch` 又是真批处理。虽比临时目录落盘省了磁盘 I/O，但峰值内存可能很高。
- **修复建议**：分批读取 + base64，或限制单批图片数量（已有 `batch_size`，但 `images` 在循环前已全部构建）。可改为逐批构建 `images`。
- **状态**：⬜ 未修复

---

## P2（优化）

### P2-1 `scan_frame_hashes` 用 `read_exact` 读定长 72 字节，末帧不完整时静默 break

- **位置**：`src-tauri/src/video/mod.rs`（`scan_frame_hashes`）
- **问题**：`read_exact(&mut buf)` 在流结束时 `UnexpectedEof` 即 `break`，未校验是否刚好整帧对齐。若 ffmpeg 因故在帧中途终止（非正常结束），最后 72 字节不完整会被**静默丢弃**，靠退出码兜底。正常结束时应整帧对齐，但退出码非 0 时已报错——此处逻辑尚可，只是未显式说明"应整帧对齐"，建议加注释或校验 `buf` 是否归零。

- **状态**：🟢 可选优化

### P2-2 `try_take_jpeg` 多次 `drain` 导致 O(n²) 拷贝

- **位置**：`src-tauri/src/video/mod.rs`（`try_take_jpeg`）
- **问题**：`buf.drain(..soi)` 和 `buf.drain(..eoi+2)` 每次把剩余字节前移。若一行里连续数百帧，每次 drain 是 O(剩余长度)，总复杂度近 O(n²)。实际每帧 JPEG 数十 KB、流式读取，通常 `buf` 只保留一帧余量，影响小；但连续帧较多且 `keep` 稀疏时可能累积。
- **修复建议**：改用游标 `start_pos` + 惰性截断，或每次 drain 后 `buf` 保持小容量（已有 `with_capacity(256KB)`）。当前影响可忽略，记录备查。

- **状态**：🟢 可选优化

### P2-3 `dev_debug` 分支保留的帧写盘仍会落盘

- **位置**：`src-tauri/src/ocr/mod.rs`（dev_debug 时 `std::fs::write(dump, &fb.jpeg)`）
- **问题**：生产模式零落盘已达成，但 `dev_debug=true` 时仍写盘。这是设计意图（人工检查帧），非缺陷；仅用于提醒——若追求"绝对零落盘"，需确保 dev_debug 关闭。

- **状态**：🟢 设计如此，记录不用改

---

## 审查通过的部分（无需改动）

- **视频抽取管道**：`extract_frames_bytes` 用 `BufReader` + 分块 `read`，`stderr` 独立线程排空避免 ffmpeg 阻塞；`try_take_jpeg` 的 SOI/EOI 标记切分有单测（`test_try_take_jpeg_splits_marker_stream`、`test_try_take_jpeg_skips_leading_noise`），注释正确说明 JPEG 熵编码段 FFD9 只作结束标记。
- **dhash 重构**：`dhash_gray9x8` 直接消费 rawvideo 定长字节，`change_flags` 纯内存化，测试覆盖平坦/行布局/镜像翻转/汉明距离边界。与 ffmpeg `format=gray,scale=9:8:flags=area` 的缩放质量匹配。
- **worker base64 解码**：`base64.b64decode` + `cv2.imdecode` + `np.frombuffer` 正确，`img is None` 检查到位，并根除 cv2 非 ASCII 路径问题。
- **内存管理**：抽帧仅在 `grid.kept` 保留变化帧字节，`jpegs` 与 `changes` 对齐；每 clip 结束后 `remove_dir_all` 清理（dev_debug 保留）。
- **测试**：151 个 lib 测试全过；`ocr_bench_refinement.rs` 基准脚本同步更新为扫描+抽帧+精化三段计时。

---

## 修复优先级建议

1. ~~**P0-1（帧数错位导致 OCR 失败）必须修**~~ → ✅ 已修复（更正：报错路径为初版误报；实际修复"幻影变化帧被静默丢弃"——`assemble_grid_frames` 回收 + 对齐显式校验）。
2. **P1-1（JPEG EOI 校验）**——单帧召回路径加尾标记校验，避免偶发解码失败误报。
3. **P1-2/P1-3**——确认 `dev_debug` 默认变更意图、优化 `run_ocr_images` 内存。
4. **P2 各项**——可选，记录备查。
