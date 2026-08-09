<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useTimelineStore, CLIP_COLORS } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import type { TimelineEvent } from "../types";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

const focusedTrack = computed(() => projectStore.findTrack(timeline.focusedTrackId));

const isRegionTrack = computed(() => focusedTrack.value?.type === "ocr_region");
const isTextTrack = computed(() => {
  const type = focusedTrack.value?.type;
  return type === "ocr_text" || type === "asr" || type === "manual";
});

// 按开始时间升序排列的 clip（序号按此编排）
const clips = computed(() => {
  const track = focusedTrack.value;
  if (!track) return [];
  return [...track.events].sort((a, b) => a.start - b.start);
});

// 缩略图宽高比跟随视频实际比例（缺省 16:9）
const thumbAspect = computed(() => {
  const meta = projectStore.currentVideoMeta;
  if (meta && meta.width > 0 && meta.height > 0) return meta.width / meta.height;
  return 16 / 9;
});

function fmtTime(s: number): string {
  const totalMs = Math.round(s * 1000);
  const ms = totalMs % 1000;
  const sec = Math.floor(totalMs / 1000) % 60;
  const min = Math.floor(totalMs / 60000);
  return `${String(min).padStart(2, "0")}:${String(sec).padStart(2, "0")}.${String(ms).padStart(3, "0")}`;
}

function regionStyle(e: TimelineEvent) {
  if (e.type !== "ocr_region") return {};
  return {
    left: e.x1 * 100 + "%",
    top: e.y1 * 100 + "%",
    width: (e.x2 - e.x1) * 100 + "%",
    height: (e.y2 - e.y1) * 100 + "%",
  };
}

function regionLabel(e: TimelineEvent): string {
  if (e.type !== "ocr_region") return "";
  return `(${e.x1.toFixed(2)}, ${e.y1.toFixed(2)}) → (${e.x2.toFixed(2)}, ${e.y2.toFixed(2)})`;
}

function textOf(e: TimelineEvent): string {
  return e.type === "ocr_region" ? "" : e.text;
}

// 点击条目：与时间轴 clip 双向聚焦绑定（不动播放头）；
// 点击其他条目时提交当前编辑（点击正在编辑的条目自身不提交，便于继续输入）
function onEntryClicked(e: TimelineEvent) {
  if (editingId.value !== e.id) commitEdit();
  timeline.focusClip(e.id);
  timeline.focusTrack(focusedTrack.value?.id ?? null);
}

// ── 双击编辑文字 ──
const editingId = ref<string | null>(null);
const editText = ref("");
const editInput = ref<HTMLTextAreaElement | null>(null);

// textarea 行数跟随内容（最多 10 行），初始即展开多行
const editRows = computed(() => {
  const lines = editText.value.split("\n").length;
  return Math.min(10, Math.max(2, lines));
});

// 进入编辑时记录原文本：提交时文本有变化才记一个撤销步骤
let editOriginal = "";

function startEdit(e: TimelineEvent) {
  if (editingId.value === e.id) return;
  editingId.value = e.id;
  editText.value = textOf(e);
  editOriginal = editText.value;
  // 播放头跳到该 clip 起始，并滚动时间轴使其可见（便于在时间轴上定位）
  timeline.jumpTo(e.start);
  nextTick(() => {
    editInput.value?.focus();
    editInput.value?.select();
  });
}

// Enter 插入换行；Ctrl/Cmd+Enter 提交；Esc 取消
function onEditKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    commitEdit();
  }
}

function commitEdit() {
  if (editingId.value) {
    if (editText.value !== editOriginal) {
      // 文本有实际变化才记录（取消/Esc/无改动提交不产生空撤销步骤）
      projectStore.recordSnapshot();
    }
    projectStore.updateEventText(editingId.value, editText.value);
  }
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}

// 切换聚焦轨道时提交未完成的编辑：条目随轨道切换卸载，
// 元素从 DOM 移除不触发 blur，不提交会残留编辑态
watch(
  () => timeline.focusedTrackId,
  () => {
    commitEdit();
    commitRename();
  }
);

// ── 双击重命名轨道（角色标注）──
// 记录发起重命名的轨道 id：切换轨道时仍提交到原轨道
const renamingTrackId = ref<string | null>(null);
const nameDraft = ref("");
const nameInput = ref<HTMLInputElement | null>(null);

function startRename() {
  const track = focusedTrack.value;
  if (!track || renamingTrackId.value) return;
  renamingTrackId.value = track.id;
  nameDraft.value = track.name;
  nextTick(() => {
    nameInput.value?.focus();
    nameInput.value?.select();
  });
}

// 空名视为取消（防误清）；提交后轨道内 asr/manual 事件 character 跟随
function commitRename() {
  const id = renamingTrackId.value;
  renamingTrackId.value = null;
  if (id) projectStore.renameTrack(id, nameDraft.value);
}

function cancelRename() {
  renamingTrackId.value = null;
}

// 轨道名编辑时点击输入框外任意处即提交：
// 时间轴 clip 的 mousedown preventDefault/stopPropagation 会挡住 blur 与冒泡，
// 捕获阶段注册先于其执行（与时间轴标签列一致的兜底）
function onWindowMouseDown(e: MouseEvent) {
  if (!renamingTrackId.value) return;
  if ((e.target as HTMLElement).closest(".ov-name-input")) return;
  commitRename();
}

onMounted(() => window.addEventListener("mousedown", onWindowMouseDown, true));
onUnmounted(() => window.removeEventListener("mousedown", onWindowMouseDown, true));
</script>

<template>
  <div class="overview-panel">
    <div class="overview-header">
      <div v-if="focusedTrack" class="ov-track-name-wrap">
        <input
          v-if="renamingTrackId"
          ref="nameInput"
          v-model="nameDraft"
          class="ov-name-input"
          @keydown.enter="commitRename"
          @keydown.esc="cancelRename"
          @blur="commitRename"
        />
        <span v-else class="ov-track-name" title="双击重命名轨道" @dblclick="startRename">
          {{ focusedTrack.name }}
        </span>
      </div>
      <span v-else class="ov-track-name">轨道总览</span>
      <span v-if="focusedTrack" class="ov-track-type">{{ focusedTrack.type }}</span>
    </div>

    <div v-if="!focusedTrack" class="overview-empty">
      未聚焦轨道
      <br />
      点击时间轴轨道以查看 clip 总览
    </div>

    <div v-else class="overview-body">
      <div
        v-for="(clip, i) in clips"
        :key="clip.id"
        class="ov-entry"
        :class="{ focused: timeline.focusedClipId === clip.id }"
        @click="onEntryClicked(clip)"
        @dblclick="timeline.jumpTo(clip.start)"
      >
        <span class="ov-index">{{ i + 1 }}</span>

        <div
          v-if="isRegionTrack"
          class="ov-thumb"
          :style="{ aspectRatio: String(thumbAspect) }"
        >
          <div
            class="ov-region"
            :style="{ ...regionStyle(clip), borderColor: CLIP_COLORS.ocr_region }"
          />
        </div>

        <div class="ov-times">
          <span class="ov-time">{{ fmtTime(clip.start) }}</span>
          <span class="ov-time">{{ fmtTime(clip.end) }}</span>
        </div>

        <div v-if="isTextTrack" class="ov-content" @dblclick.stop="startEdit(clip)">
          <textarea
            v-if="editingId === clip.id"
            ref="editInput"
            v-model="editText"
            class="ov-edit-input"
            :rows="editRows"
            @keydown.enter="onEditKeydown"
            @keydown.esc="cancelEdit"
            @blur="commitEdit"
          />
          <span v-else class="ov-text">{{ textOf(clip) }}</span>
        </div>

        <span v-else-if="isRegionTrack" class="ov-coords">{{ regionLabel(clip) }}</span>
      </div>

      <div v-if="clips.length === 0" class="overview-empty">此轨道暂无 clip</div>
    </div>
  </div>
</template>

<style scoped>
.overview-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: 8px;
  overflow: hidden;
}

.overview-header {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--color-border);
}

.ov-track-name-wrap {
  flex: 1;
  min-width: 0;
  display: flex;
}

.ov-track-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.ov-name-input {
  width: 100%;
  font-size: 13px;
  font-weight: 600;
  font-family: inherit;
  color: var(--color-text-primary);
  background: var(--color-bg-primary);
  border: 1px solid var(--color-accent);
  border-radius: 4px;
  padding: 2px 6px;
  outline: none;
  box-sizing: border-box;
}

.ov-track-type {
  font-size: 10px;
  color: var(--color-text-secondary);
  background: var(--color-bg-tertiary);
  border-radius: 4px;
  padding: 1px 6px;
  flex-shrink: 0;
}

.overview-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

.ov-entry {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--color-border);
  cursor: pointer;
}

.ov-entry:hover {
  background: var(--color-bg-tertiary);
}

.ov-entry.focused {
  background: var(--color-bg-tertiary);
  box-shadow: inset 2px 0 0 var(--color-accent);
}

.ov-index {
  flex-shrink: 0;
  width: 20px;
  text-align: center;
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
}

.ov-thumb {
  position: relative;
  flex-shrink: 0;
  width: 96px;
  background: #000;
  border: 1px solid var(--color-border);
  border-radius: 4px;
  overflow: hidden;
}

.ov-region {
  position: absolute;
  border: 2px solid;
  box-sizing: border-box;
}

.ov-times {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  min-width: 84px;
}

.ov-time {
  font-size: 11px;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
}

.ov-content {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: flex-start;
}

.ov-text {
  font-size: 12px;
  color: var(--color-text-primary);
  line-height: 1.4;
  white-space: pre-wrap;
  word-break: break-all;
  cursor: text;
}

.ov-edit-input {
  width: 100%;
  font-size: 12px;
  font-family: inherit;
  line-height: 1.4;
  color: var(--color-text-primary);
  background: var(--color-bg-primary);
  border: 1px solid var(--color-accent);
  border-radius: 4px;
  padding: 4px 6px;
  outline: none;
  resize: vertical;
  min-height: 32px;
  box-sizing: border-box;
}

.ov-coords {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.overview-empty {
  padding: 32px 12px;
  text-align: center;
  font-size: 12px;
  color: var(--color-text-secondary);
  opacity: 0.6;
  line-height: 1.8;
}
</style>
