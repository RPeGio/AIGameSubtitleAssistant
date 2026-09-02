# PR #19 代码审查记录（feat/workflow → master）

- 审查日期：2026-09-02
- 审查范围：PR #19 全部 12 个提交（`59fd17c..feat/workflow`），25 个文件，+2660/−995
- 内容：三页工作台（语料/转写/融合）+ 数据模型扩展（双视频源/独立语料/轨道 scope-page-video）+ 最终字幕编辑页
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2

---

## P0（必须修复）

### P0-1 撤销栈不包含 corpus，但 corpus 操作会压入快照 —— 删除语料后 Ctrl+Z 是"假撤销"

- **位置**：`src/stores/project.ts:186-208`（snapshot/undo/redo）+ `src/stores/project.ts:634/653/665`（writeOcrToCorpus / addCorpusItem / removeCorpusItem 均调用了 `recordSnapshot()`）
- **问题**：`snapshot()` 和 `undo()`/`redo()` 只快照/恢复 `tracks`，不包含 `corpus`。删除语料 → `recordSnapshot()` 压入与当前完全相同的 tracks 快照 → Ctrl+Z 恢复一份一模一样的 tracks（看似撤销生效），**语料实际未恢复且丢失**。
- **修复**：把 corpus 纳入快照与恢复。
- **状态**：✅ 已修复（undo/redo 快照结构扩展为 `{ tracks, corpus }`）

```typescript
function snapshot(): { tracks: Track[]; corpus: CorpusItem[] } {
  const p = currentProject.value;
  return {
    tracks: JSON.parse(JSON.stringify(p?.tracks ?? [])) as Track[],
    corpus: JSON.parse(JSON.stringify(p?.corpus ?? [])) as CorpusItem[],
  };
}

function undo() {
  if (!currentProject.value || undoStack.value.length === 0) return;
  redoStack.value.push(snapshot());
  const snap = undoStack.value.pop()!;
  currentProject.value.tracks = snap.tracks;
  currentProject.value.corpus = snap.corpus;
}
// redo() 同理
```

---

## P1（推荐修改）

### P1-1 `updateCharacter` 每次击键压一条撤销快照，且与同行文本编辑语义不一致

- **位置**：`src/views/Editor.vue:57-63` + 模板 `Editor.vue:227`（`@update:value="updateCharacter"`）
- **问题**：`@update:value` 逐键触发 `recordSnapshot()`（整棵 tracks 深拷贝）：① 打 10 个字占掉 MAX_HISTORY=60 中 10 格，挤出有意义历史；② 每键全量深拷贝 + redo 链清空；③ 同行 `updateText`（`updateEventText`，`project.ts:520`）完全不压快照，两输入框撤销行为不一致。
- **修复建议**：改用 `@change`（失焦/回车提交），统一两字段快照策略。
- **状态**：✅ 已修复（最终方案：NInput 是受控组件，`@change` 会导致打字回退无法输入——改为保留 `@update:value` 即时生效，`@focus` 重置会话标记，聚焦会话内首次修改压一次快照，整段编辑合并为一条撤销记录；文本/角色两字段粒度一致）

### P1-2 编辑页工具栏的撤销/重做按钮没有清理悬空聚焦

- **位置**：`src/views/Editor.vue:132`、`Editor.vue:153`（对比 `ProjectLayout.vue:47-62` 键盘路径有 `cleanupFocus()`）
- **问题**：撤销可能移除当前聚焦的 clip，按钮路径不清理 `focusedClipId` → 悬空聚焦：高亮错位，后续 Delete/M 作用于失效 id。
- **修复建议**：`cleanupFocus` 提为 store 方法或 composable，两条路径共用。
- **状态**：✅ 已修复（Editor 内新增 `cleanupFocus` + `onUndo`/`onRedo` 包装，工具栏按钮接入）

### P1-3 FuseView 的 `asrCount` 把主播语音轨也算进"已生成 N 段转写"

- **位置**：`src/views/FuseView.vue:24-29`、`FuseView.vue:104`
- **问题**：`timelineReady` 只认 `track_role === "game"` 的 ASR 轨，但 `asrCount` 统计所有 asr 轨。只有主播轨时显示"已生成 N 段转写"却"未就绪"，文案矛盾。
- **修复建议**：`asrCount` 过滤 `track_role === "game"`。
- **状态**：⬜ 未修复

### P1-4 最终字幕列表无虚拟化，长视频会卡

- **位置**：`src/views/Editor.vue:208-245`（`v-for="(ev, i) in clips"`）
- **问题**：2 小时录播的融合结果可达上千行，每行 2 个 `NInput` + 3 个 `NButton`，全量渲染 DOM 数万节点，滚动与输入卡顿。
- **修复建议**：`n-virtual-list` 包裹行渲染，或分页/懒加载兜底。
- **状态**：⬜ 未修复

### P1-5 语料页 OCR 静默失败路径

- **位置**：`src/stores/project.ts`（`runOcr` 开头的守卫）
- **问题**：~~`regionClips` 为空时静默"完成"~~（**审查误报**：`regionClips.length === 0` 的 throw 在 master 上已存在，`project.ts:572`）；实际有效问题是 meta 缺失时静默 `return`，调用方无任何提示。
- **修复**：meta/项目缺失改为 throw 明确错误（唯一调用方 CorpusView.startOcr 已有 try/catch 展示）。
- **状态**：✅ 已修复（runOcr 缺项目/缺视频时 throw；空选区守卫确认已存在，无需改动）

---

## P2（锦上添花）

### P2-1 CSS 格式破损

- **位置**：`src/components/timeline/Timeline.vue:663`
- **问题**：`border-right` 与 `background` 被合并到一行（本次改动引入的编辑事故）。
- **状态**：⬜ 未修复

### P2-2 已删除组件的残留注释

- **位置**：`src/stores/project.ts:409`、`src/stores/timeline.ts:100`
- **问题**：`RegionOverlay.vue` 已删除，两处注释仍引用它。
- **状态**：⬜ 未修复

### P2-3 播放进度可能超 100%

- **位置**：`src/components/SourceVideoPreview.vue`（`progressPct` computed）
- **问题**：`time` 略超 `duration` 时 range input 的 value 超界。加 `Math.min(100, ...)`。
- **状态**：⬜ 未修复

### P2-4 导出格式未对齐后端能力

- **位置**：`src/views/Editor.vue:92-95`
- **问题**：后端 `export_track_subtitle` 支持 srt/ass/lrc/txt，编辑页只暴露 srt/ass。若是有意收窄可忽略。
- **状态**：⬜ 未修复

### P2-5 双时间轴的快捷键行为不对称（记录备忘）

- **位置**：`src/layouts/ProjectLayout.vue`（M/S 快捷键）vs `src/components/timeline/Timeline.vue`（Delete）
- **问题**：M/S 始终作用于全局 store（校对区时间轴），Delete 由各 Timeline 实例处理自己的 store。语料页聚焦迷你时间轴 clip 后按 S/M 不作用于迷你时间轴。当前 ReviewPane 常驻所有页面、行为可解释；未来若扩展迷你时间轴编辑功能，建议把 S/M 也下沉到 Timeline 组件（与 Delete 同模式）。
- **状态**：⬜ 未修复

---

## 审查通过的部分（无需改动）

- **后端 Rust**（`project/mod.rs` +109 行）：serde 缺省值设计正确，`scope/page/video` 对旧 `project.json` 的向后兼容有专门单测（legacy JSON 默认值 + roundtrip），`export/mod.rs` 测试同步更新。
- **安全性**：无 SQL/注入面；导出路径来自系统保存对话框；Vue 插值自动转义无 XSS；`convertFileSrc` 仅用于本地媒体加载，符合桌面应用场景。
- **并发**：`saveNow` 对 await 期间项目切换的竞态有守卫（`project.ts:141`）；SourceVideoPreview/TimelineClip 的 window 级拖拽监听均在 `onUnmounted` 正确清理。
- **正确性亮点**：语料 OCR 选区轨与编辑页选区轨用 `video !== "source"` 精确区分；双 Timeline 实例通过 `TIMELINE_STORE_KEY` 注入隔离 store 的方案干净且各子组件全部接入；`runFuse` 只消费 `track_role === "game"` 轨与 `timelineReady` 判定一致。

## 修复优先级建议

P0-1（数据丢失）→ P1-1/P1-2/P1-5（同一次迭代顺手完成）→ P1-3/P1-4（下一个提交）。
