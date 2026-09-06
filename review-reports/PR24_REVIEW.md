# PR #24 代码审查记录（feat/clip-ocr → master）

- 审查日期：2026-09-05
- 审查范围：PR #24（`feat/clip-ocr` 分支）phase 6.1 M2-M4 提交
  - `c5d4a4a` feat(ocr): embed_ocr 轨道类型 + 转写页内嵌字幕 OCR 生产区（M2）
  - `5ebd300` fix(ocr): 移除 ensureDefaultTrack 的 editor OCR 选区轨，避免重复建轨
  - `296b51d` fix(ocr): TrackOverview 文字轨判定补 embed_ocr（M3 收尾）
  - `e64ac4d` feat(fuse): 游戏内容段两源合并（game-ASR + 嵌字 OCR）接入融合（M4）
- 相对 `06dfd0b` 共 13 文件，+436/−127
- 验证：`cargo check` 通过；lib 133 单测全过（新增 `test_embed_ocr_event_roundtrip`）；`vue-tsc` 通过
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2

---

## P0（必须修复）

**未发现**。`embed_ocr` 后端复用 `OcrTextEvent` 结构（serde rename 干净、无结构重复），前端 `EmbedOcrEvent` 作为判别联合成员，所有类型守卫/switch 点（`TEXT_TRACK_TYPES`、`CLIP_COLORS`、`isTextTrack`、`clipText`、`SubtitleOverlay`）都已更新。`TimelineEvent::EmbedOcr` 变体已在 `export/mod.rs` 的 `event_times`/`event_text` 两个 match 补全（新增变体必须全处理，否则不编译）。

---

## P1（建议修改）

### P1-1 `runFuse` 移除 ocr_text 轨回退 —— 行为变更

- **位置**：`src/stores/project.ts`（`runFuse`）
- **问题**：语料为空时现在直接 `throw`，不再回退 `ocr_text` 轨。原审查担心旧项目（只有 ocr_text 数据、无 corpus）会无法融合。
- **结论（经深入分析修正）**：**回退 ocr_text 轨会引入正确性 bug**，故保留 throw。`ocr_text` 轨只可能是 mock 占位文本（"旅行者，你来了"等），或旧编辑页对**切片视频**的 OCR 结果——那是"待替换的转写文本"（plan 6.1 ②步产物），不是"可靠剧情文本"（①步产物，来自 source 视频语料）。混作可靠文本会导致错误替换。
- **状态**：✅ 已修复（保留 throw，但把错误信息写清楚，说明为何必须收集语料、以及为何不能用 ocr_text/内嵌字幕轨作为可靠文本）

```typescript
// 可靠文本：只来自 corpus 语料（剧情录屏 OCR / 截图 / 手动）。
// 不回退 ocr_text 轨：它只可能是 mock 占位文本，或旧编辑页对"切片视频"的
// OCR 结果——那是"待替换的转写文本"，不是"可靠剧情文本"（可靠文本必须来自
// source 视频的语料，见 plan 6.1 ①步）。混作可靠文本会导致错误替换。
if (project.corpus.length === 0) {
  throw new Error(
    "可靠文本语料为空：请先在语料页从剧情录屏 OCR 或手动提供文本。" +
      "（注意：OCR 文本轨 / 内嵌字幕轨是待替换的转写文本，不能作为可靠文本语料）"
  );
}
```

---

## P2（优化）

### P2-1 `collectGameContentSegments` 除零风险

- **位置**：`src/stores/project.ts`（`overlapRatio`）
- **问题**：分母 `embed.end - embed.start`，若 embed 零长度则除零 → NaN/Infinity，`NaN >= 0.5` 为 false → 该 embed 永不丢弃。零长度事件理论上不存在，但加守卫更稳。
- **状态**：✅ 已修复（`overlapRatioWithAsrUnion` 开头 `if (embed.end <= embed.start) return 0`）

### P2-2 重叠消解按单条 ASR 独立判定

- **位置**：`src/stores/project.ts`（`collectGameContentSegments`）
- **问题**：embed 与 asr1 重叠 40%、与 asr2 重叠 40%（合计 80% 覆盖）时，单条都 <50% → embed 保留，但实际已大部分冗余，融合时可能产生重复段。建议按 ASR 段**并集**算重叠。
- **状态**：✅ 已修复（改为 `overlapRatioWithAsrUnion`：累计 embed 与所有 ASR 段的并集覆盖时长，再除以 embed 时长）

### P2-3 `runOcr` 默认 clip 模式（无 regionPage）已成死路径

- **位置**：`src/stores/project.ts`（`runOcr`）
- **问题**：`ensureDefaultTrack` 不再创建 page=editor 选区轨，且 Editor.vue（PR #19 重写后）不再跑 OCR。默认 `page !== "asr"` 只会收集旧 editor 轨或空。建议把 `regionPage` 设为必填或移除该默认分支。
- **状态**：✅ 已修复（clip 模式必须显式指定 regionPage，否则 throw `"clip 模式需指定 regionPage"`；移除 `page !== "asr"` 默认分支）

### P2-4 `ensureAsrRegionTrack` 与 `ensureCorpusRegionTrack` 重复

- **位置**：`src/stores/project.ts`
- **问题**：两个函数都创建默认矩形选区控制轨（相同默认 x1/y1/x2/y2）。建议抽公共建轨助手。
- **状态**：✅ 已修复（抽公共 `pushRegionControlTrack({ name, page, video, duration })`，两个 ensure 函数复用）

### P2-5 后端 `build_prompt` 仍把输入标为 "ASR[{}]"

- **位置**：`src-tauri/src/fuse/mod.rs`
- **问题**：数据来源可以是 embed_ocr，但 prompt 前缀仍叫 "ASR"。虽然 intro 已说明来源，但 LLM 侧 "ASR" 标签对嵌字段是轻微误导。可改为中性标签。
- **状态**：✅ 已修复（标签统一改为 "GC"（Game Content），intro `ASR[{}]`/`ASR[17]` 提示与 `deser_index` 注释同步更新；`test_build_prompt_contains_all_ocr_and_batch` 断言已同步）

### P2-6 建轨配置重复

- **位置**：`src/stores/project.ts`（`ensureEmbedOcrTrack`/`ensureOcrTextTrack`/`ensureAsrRegionTrack`/`ensureCorpusRegionTrack`）
- **问题**：多个 helper 各自重复默认配置，可收敛为共享函数。
- **状态**：⬜ 已评估跳过（各 helper 的 type/name/role/page/video 差异较大，抽象一层 `pushTrack` 收益边际且新增一层抽象；最显著的选区轨重复已在 P2-4 解决）

---

## ✅ 通过部分

- 两源融合（game-ASR + embed_ocr）与 phase 6.1 计划一致，`collectGameContentSegments` 是干净纯函数、重叠启发式有注释说明。
- `runOcr` 的 `regionPage` 过滤明确正确（隔离 asr/corpus/editor 选区轨，避免串扰）。
- AsrView ② 段复用参数化的 `SourceVideoPreview`/`SourceTimeline`（videoKey="clip"），避免组件重复。
- 后端 fuse 逻辑本身未变（仅注释/prompt 措辞），风险低。
- `writeOcrSegments`/`writeEmbedOcrSegments` 都调用 `recordSnapshot()`，撤销粒度一致。

## 修复优先级建议

P1-1（已修复，保留 throw + 清晰错误信息）→ P2-2（重叠按并集算，减少冗余段，最值得）→ P2-1/P2-3（低风险守卫/清理）→ P2-4/P2-5/P2-6（维护性收敛）。
