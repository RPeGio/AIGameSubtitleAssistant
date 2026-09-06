# PR #25 代码审查记录（feat/corpus-ocr-options）

- 审查日期：2026-09-06
- 审查范围：PR #25（`feat/corpus-ocr-options`）相对 master merge-base `6b28b84` 的 5 个提交
- 提交构成：
  - 核心 feature（2 个）：`6ae0e88` 截图 OCR 语料、`0885751` 纯文本语料
  - 顺带进分支（3 个）：`76ec248` 默认缩放、`8279821` 暗色主题、`06dfd0b` 合并 #22 CSS 优化
- 改动文件：`src-tauri/src/{lib,ocr/project}.rs`、`src/{stores/project.ts,views/CorpusView.vue,components/timeline/Timeline.vue,App.vue}` 等
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2
- 修复分支：`feat/corpus-ocr-options`（独立 worktree `AIGameSubtitleAssistant-corpus-ocr-options`）

---

## P0（必须修复）

无致命错误。后端两条新命令有配套单测、RAII 临时目录守卫、UTF-8 校验、10MB 上限；前端复用 `pushCorpusTexts` 统一去重与撤销快照。

---

## P1（建议修复）

### P1-1 截图 OCR 与录屏 OCR 共享 `ocrRunning/ocrProgress/ocrMessage`，同页进度串显

- **位置**：`src/stores/project.ts`（`runCorpusImageOcr` 复用全局 OCR 状态）；`src/views/CorpusView.vue` 两个来源区块都绑定 `projectStore.ocrRunning`
- **问题**：语料页“从剧情录屏中截取”和“从文本截图中截取”是两个独立 OCR 来源，共用同一运行/进度状态。展开两区运行截图 OCR 时，录屏区也显示同一进度条与“识别截图 1/3”消息，显示错乱。
- **修复建议**：截图 OCR 用独立状态；CorpusView 截图区块绑独立状态。
- **状态**：✅ 已修复
- **实现说明**：截图与录屏 OCR 共用同一后端 `OcrManager`，**不可并发**，故未拆独立 running flag。改用**单一 `ocrRunning` 互斥 + 新增 `ocrSource`（"video"|"image"）区分来源**：`runOcr` 置 `"video"`、`runCorpusImageOcr` 置 `"image"`，`finally` 置回 `null`；CorpusView 两来源区块只在 `ocrRunning && ocrSource === 对应值` 时显示各自进度条。既保留互斥，又消除同页进度串显。

### P1-2 语料预览列表无虚拟化，导入大文本一次渲染数千行

- **位置**：`src/views/CorpusView.vue`（`parsedLines`/`previewStats`、`.preview-list` 的 `v-for`）
- **问题**：`read_text_file` 允许 10MB txt；`.preview-list` 无虚拟化渲染所有行，导入大文件时 DOM 数千节点阻塞渲染滚动。
- **修复建议**：预览区虚拟化或限前 N 行；`previewStats` 加行数上限短路。
- **状态**：✅ 已修复
- **实现说明**：参考 `f85fd40c`（最终字幕列表虚拟化）的 `n-virtual-list` 模式——`item-resizable` + `key-field` + 数字绑定 `:item-size` + 行间距用 padding（borderBoxSize 测量，margin 不计入）。把 `parsedLines` 包装成 `{id,text}` 对象数组以便稳定 key；容器改固定 `height:150px`（虚拟列表需确定高度容器）。

---

## P2（优化）

### P2-1 `run_ocr_images` 进度事件 `clip_index` 用 0 基，与 `run_ocr` 的 1 基不一致

- **位置**：`src-tauri/src/ocr/mod.rs`（`clip_index: i`，0 基批号）
- **问题**：`run_ocr` 的 `clip_index` 是 1 基；此处 0 基。消息文本 `识别截图 {i+1}/{total}` 已 1 基，字段值却不一致。
- **状态**：✅ 已修复（`clip_index: i + 1`，与 `run_ocr` 及消息保持 1 基）

### P2-2 `batchSize: 8` 硬编码在前端

- **位置**：`src/stores/project.ts`（`runCorpusImageOcr` 内 `batchSize: 8`）
- **问题**：后端 `run_ocr_images` 的 `batch_size` 是接口，前端写死 8，用户无法调整。
- **决定**：按要求**不暴露到前端**，仅加注释说明选择依据（截图 OCR 面向少量剧情文本图，8 已平衡批吞吐与单批耗时）。
- **状态**：✅ 已修复（注释说明，不改逻辑）

### P2-3 预览列表用 index 作 key

- **位置**：`src/views/CorpusView.vue`（`previewItems` 的 id）
- **问题**：只读预览可接受，但后续若允许预览中编辑/删除行，index key 会因重排错位。
- **状态**：✅ 已修复（id 用「行内容+序号」组合键，中间插入行不再 key 错位；同时保证预览允许的重复行 id 仍唯一）

### P2-4 `read_text_file` 未显式剥离 BOM

- **位置**：`src-tauri/src/project/mod.rs`（`read_text_file` 只做 UTF-8 校验）
- **问题**：Windows txt 常带 UTF-8 BOM（`\u{FEFF}`）。当前依赖前端 JS `trim()` 兜底，属隐式依赖；若未来 Rust 侧直接消费会残留 BOM。
- **状态**：✅ 已修复（读取后显式剥离首字符 BOM；顺带把仅在测试用的 `uuid::Uuid` import 加 `#[cfg(test)]` 消除编译警告）

---

## 顺带进分支的 3 个提交（本身干净，注意归属）

- `76ec248` 默认缩放：`initialZoomDone` 是 `<script setup>` 顶层变量，每个实例独立，校对区与迷你时间轴互不干扰，正确。小瑕疵：`Math.max(ppsFitAll, pps30)` 因 `0.3 < 1` 恒等于 `pps30`，`min(800, …)` 对极短视频显示超过 30%（可接受）。
- `8279821` 暗色主题：`NConfigProvider :theme="darkTheme"` 包裹全部，顺序正确。
- `06dfd0b` 合并 #22：CSS 优化 merge。

**建议**：这 3 个提交与“corpus OCR options”主题无关，合并 PR #25 前确认是刻意并入还是应从 master 另行合入。

---

## 审查通过的部分

- **后端安全**：`read_text_file` 10MB 上限 + UTF-8 白名单；`run_ocr_images` UUID 临时目录 + `TempDirGuard` RAII 清理（含 panic 退出路径），规避 cv2 非 ASCII 路径问题。
- **去重逻辑**：`lines_from_results` 跨图/跨批精确去重；`pushCorpusTexts` 再与现有语料去重并一次撤销快照（含 corpus），两层去重不冲突。
- **并发**：`run_ocr_images` 用 `spawn_blocking` 避免阻塞主线程；`ocrRunning` 互斥 + 按钮禁用，无并发覆盖。
- **测试**：`lines_from_results`、`read_text_file` 有单测。
- **前端安全**：无 SQL；`open` 对话框选文件；文本经 Vue 插值转义，无 XSS。

## 修复优先级建议

P1-1（同页进度串显）→ P1-2（大文本渲染）→ P2-1/P2-4（一行改动，顺手完成）→ P2-2/P2-3（注释说明 / key 稳定）。

## 修复状态汇总

- P0：无
- P1-1 ✅、P1-2 ✅
- P2-1 ✅、P2-2 ✅（注释）、P2-3 ✅、P2-4 ✅
