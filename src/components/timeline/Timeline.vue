<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { NModal, NPopconfirm, NSelect, NButton } from "naive-ui";
import { useTimelineStore, CLIP_COLORS } from "../../stores/timeline";
import { useProjectStore } from "../../stores/project";
import type { Track } from "../../types";
import TimelineRuler from "./TimelineRuler.vue";
import TimelineClip from "./TimelineClip.vue";
import TimelineScrollbar from "./TimelineScrollbar.vue";
import TimelineToolStrip from "./TimelineToolStrip.vue";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

// 布局常量：工具列宽度 + 轨道标签列宽度
const TOOL_WIDTH = 40;
const LABEL_WIDTH = 180;

const tracks = () => projectStore.currentProject?.tracks ?? [];
const rootRef = ref<HTMLElement | null>(null);
const viewportRef = ref<HTMLElement | null>(null);
let viewportObserver: ResizeObserver | null = null;
let scrubbingBody: HTMLElement | null = null;

// 播放头在内容区内的横向位置（相对内容区左缘）
const playheadLeft = computed(() => timeline.playheadX() + "px");

// ── 刻度区拖动：只有刻度区域能拖动红色标头 ──

function onRulerMouseDown(e: MouseEvent) {
  const target = e.currentTarget as HTMLElement;
  e.preventDefault();
  scrubbingBody = target;
  timeline.scrubbing(true);
  timeline.seek(timeline.timeAtPixel(e.clientX - target.getBoundingClientRect().left));
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!scrubbingBody) return;
  const rect = scrubbingBody.getBoundingClientRect();
  const mouseX = e.clientX - rect.left;

  if (mouseX < 0) {
    const speed = Math.min(-mouseX * 0.3, 20);
    timeline.currentTime = timeline.timeAtPixel(0);
    timeline.scrollLeft = Math.max(0, timeline.scrollLeft - speed);
  } else if (mouseX > rect.width) {
    const speed = Math.min((mouseX - rect.width) * 0.3, 20);
    timeline.currentTime = timeline.timeAtPixel(rect.width);
    timeline.scrollLeft = Math.min(timeline.scrollLeft + speed, timeline.maxScroll());
  } else {
    timeline.currentTime = timeline.timeAtPixel(mouseX);
  }
}

function onWindowMouseUp() {
  if (!scrubbingBody) return;
  scrubbingBody = null;
  timeline.scrubbing(false);
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

// ── 轨道区域：分割工具 or 只聚焦轨道（不移动播放头）──

function onTrackAreaMouseDown(e: MouseEvent) {
  const target = e.currentTarget as HTMLElement;

  // 分割工具：在 clip 上按下即分割，空白处不动作
  if (timeline.activeTool === "split") {
    const clipEl = (e.target as HTMLElement).closest(".clip") as HTMLElement | null;
    const eventId = clipEl?.dataset.eventId;
    if (!eventId) return;
    const time = timeline.timeAtPixel(e.clientX - target.getBoundingClientRect().left);
    const rightId = projectStore.splitEvent(eventId, time);
    if (rightId) {
      timeline.focusClip(rightId);
      const found = projectStore.findEvent(rightId);
      if (found) timeline.focusTrack(found.track.id);
    }
    return;
  }

  // 合并工具：点击空白取消当前第一选择
  if (timeline.activeTool === "merge") {
    mergeFirstId.value = null;
    timeline.focusClip(null);
    timeline.focusTrack(target.dataset.trackId ?? null);
    return;
  }

  // 选择模式：只聚焦轨道，不改变播放头
  timeline.focusTrack(target.dataset.trackId ?? null);
  timeline.focusClip(null);
}

// 选择模式下点击 clip：聚焦 clip + 其所在轨道；
// 合并工具下为点选合并：第一次点击记为第一选择，第二次点击相邻 clip 直接合并
const mergeFirstId = ref<string | null>(null);

function onClipClicked(track: Track, clipId: string) {
  if (timeline.activeTool === "split") return;
  if (timeline.activeTool === "merge") {
    if (!mergeFirstId.value) {
      // 第一选择：聚焦高亮，等待第二次点击
      mergeFirstId.value = clipId;
      timeline.focusClip(clipId);
      timeline.focusTrack(track.id);
      return;
    }
    const first = mergeFirstId.value;
    mergeFirstId.value = null;
    const merged = projectStore.mergeTwo(first, clipId);
    if (merged) {
      timeline.focusClip(merged);
      timeline.focusTrack(track.id);
    } else {
      // 不相邻/不同轨：失焦供用户重新选择
      timeline.focusClip(null);
      timeline.focusTrack(track.id);
    }
    return;
  }
  timeline.focusClip(clipId);
  timeline.focusTrack(track.id);
}

// 点击轨道名称标签：聚焦该轨道
function onTrackLabelClicked(track: Track) {
  timeline.focusClip(null);
  timeline.focusTrack(track.id);
}

// ── 双击标签列重命名轨道（角色标注）──
const renameTrackId = ref<string | null>(null);
const renameDraft = ref("");
const renameInput = ref<HTMLInputElement | null>(null);

function startRename(track: Track) {
  if (renameTrackId.value) return;
  renameTrackId.value = track.id;
  renameDraft.value = track.name;
  nextTick(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

// 空名视为取消（防误清）；提交后轨道内 asr/manual 事件 character 跟随
function commitRename() {
  const id = renameTrackId.value;
  renameTrackId.value = null;
  if (id) projectStore.renameTrack(id, renameDraft.value);
}

function cancelRename() {
  renameTrackId.value = null;
}

// 编辑轨道名时点击输入框外任意处即提交：
// WebView 点击不可聚焦元素可能不移焦，blur 兜底不可靠；
// 捕获阶段注册，先于 clip 拖动 mousedown 的 stopPropagation 执行
function onWindowMouseDown(e: MouseEvent) {
  if (!renameTrackId.value) return;
  if ((e.target as HTMLElement).closest(".tl-name-input")) return;
  commitRename();
}

// ── 轨道操作：上移/下移/删除/合并 ──
function onMoveTrack(track: Track, dir: "up" | "down") {
  projectStore.moveTrack(track.id, dir);
}

function onDeleteTrack(track: Track) {
  projectStore.removeTrack(track.id);
  if (timeline.focusedTrackId === track.id) {
    timeline.focusTrack(null);
    timeline.focusClip(null);
  } else if (timeline.focusedClipId && !projectStore.findEvent(timeline.focusedClipId)) {
    // 聚焦 clip 恰好位于被删轨道：清理悬空聚焦
    timeline.focusClip(null);
  }
}

// 合并：弹窗选择同类型目标轨道
const mergeModal = ref(false);
const mergeSrc = ref<Track | null>(null);
const mergeDstId = ref<string | null>(null);

const mergeOptions = computed(() => {
  if (!mergeSrc.value) return [];
  return tracks()
    .filter((t) => t.id !== mergeSrc.value!.id && t.type === mergeSrc.value!.type)
    .map((t) => ({ label: t.name, value: t.id }));
});

// 切换工具时清空点选合并的第一选择
watch(
  () => timeline.activeTool,
  () => {
    mergeFirstId.value = null;
  }
);

function onMergeOpen(track: Track) {
  mergeSrc.value = track;
  mergeDstId.value = null;
  mergeModal.value = true;
}

function onMergeConfirm() {
  if (mergeSrc.value && mergeDstId.value) {
    projectStore.mergeTrack(mergeSrc.value.id, mergeDstId.value);
    // 源轨被删除：清理悬空聚焦，聚焦指向目标轨道
    if (timeline.focusedClipId && !projectStore.findEvent(timeline.focusedClipId)) {
      timeline.focusClip(null);
    }
    if (timeline.focusedTrackId === mergeSrc.value.id) {
      timeline.focusTrack(mergeDstId.value);
    }
  }
  mergeModal.value = false;
}

// ── 滚轮：普通滚轮水平平移时间轴；Shift+滚轮垂直滚动轨道区 ──

function onWheelRoot(e: WheelEvent) {
  const t = e.target as HTMLElement;
  // 滑条自己处理缩放；工具条/标签列不响应
  if (t.closest(".scrollbar-track")) return;
  if (t.closest(".tool-strip") || t.closest(".tl-label-col")) return;
  // Shift+滚轮：垂直滚动轨道区
  if (e.shiftKey) {
    const tracksEl = rootRef.value?.querySelector<HTMLElement>(".tl-tracks") ?? null;
    if (tracksEl && tracksEl.scrollHeight > tracksEl.clientHeight) {
      // 轨道区内放行原生滚动（保留触控板惯性），轨道区外代为滚动
      // 注意 Chrome 按住 Shift 会交换 deltaX/deltaY 轴，故两轴都计入
      if (t.closest(".tl-tracks")) return;
      e.preventDefault();
      tracksEl.scrollTop += e.deltaY + e.deltaX;
      return;
    }
    // 轨道不足无法滚动：吸收输入
    e.preventDefault();
    return;
  }
  // 普通滚轮：水平平移时间轴（触控板横滑时 deltaX 非零）
  e.preventDefault();
  timeline.pan(e.deltaY + e.deltaX);
}

onMounted(() => {
  if (viewportRef.value) {
    const update = () => {
      timeline.setViewportWidth(viewportRef.value!.getBoundingClientRect().width);
    };
    update();
    viewportObserver = new ResizeObserver(update);
    viewportObserver.observe(viewportRef.value);
  }
  rootRef.value?.addEventListener("wheel", onWheelRoot, { passive: false });
  window.addEventListener("mousedown", onWindowMouseDown, true);
});

onUnmounted(() => {
  viewportObserver?.disconnect();
  rootRef.value?.removeEventListener("wheel", onWheelRoot);
  window.removeEventListener("mousedown", onWindowMouseDown, true);
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
});
</script>

<template>
  <div class="timeline-root" ref="rootRef">
    <TimelineToolStrip />

    <div class="timeline-col">
      <!-- Header: ruler（仅这里可拖动播放头） -->
      <div class="tl-row">
        <div class="tl-label-col" />
        <div
          ref="viewportRef"
          class="tl-content"
          @mousedown="onRulerMouseDown"
        >
          <TimelineRuler />
        </div>
      </div>

      <!-- Track rows（垂直滚动区：轨道多时滚动，行高固定不被压缩） -->
      <div class="tl-tracks">
        <div
          v-for="track in tracks()"
          :key="track.id"
          class="tl-row track-row"
        >
          <div
            class="tl-label-col"
            :class="{ 'track-focused': timeline.focusedTrackId === track.id }"
            @click="onTrackLabelClicked(track)"
          >
            <div class="label-name">
              <input
                v-if="renameTrackId === track.id"
                ref="renameInput"
                v-model="renameDraft"
                class="tl-name-input"
                @keydown.enter="commitRename"
                @keydown.esc="cancelRename"
                @blur="commitRename"
              />
              <span
                v-else
                class="label-name-text"
                title="双击重命名轨道"
                @dblclick="startRename(track)"
              >
                {{ track.name }}
              </span>
            </div>
            <div class="label-type">{{ track.type }}</div>
            <div v-if="renameTrackId !== track.id" class="tl-label-actions" @click.stop>
              <button class="tl-label-btn" title="上移轨道" @click="onMoveTrack(track, 'up')">↑</button>
              <button class="tl-label-btn" title="下移轨道" @click="onMoveTrack(track, 'down')">↓</button>
              <button class="tl-label-btn" title="合并到其他轨道" @click="onMergeOpen(track)">⇄</button>
              <NPopconfirm @positive-click="onDeleteTrack(track)">
                <template #trigger>
                  <button class="tl-label-btn tl-label-btn-danger" title="删除轨道">🗑</button>
                </template>
                删除轨道「{{ track.name }}」？此操作不可恢复
              </NPopconfirm>
            </div>
          </div>
          <div
            class="tl-content tl-track-body"
            :data-track-id="track.id"
            @mousedown="onTrackAreaMouseDown"
          >
            <template v-if="track.events.length > 0">
              <TimelineClip
                v-for="event in track.events"
                :key="event.id"
                :event="event"
                :siblings="track.events"
                :color="CLIP_COLORS[event.type] ?? '#666'"
                :left="timeline.clipPosition(event).left"
                :width="timeline.clipPosition(event).width"
                :focused="timeline.focusedClipId === event.id"
                @click-clip="onClipClicked(track, event.id)"
                @merged="mergeFirstId = null"
              />
            </template>
            <div v-else class="empty-hint">此轨道暂无事件</div>
          </div>
        </div>
      </div>

      <!-- Footer: scrollbar（固定贴底） -->
      <div class="tl-row timeline-footer">
        <div class="tl-label-col" />
        <div class="tl-content">
          <TimelineScrollbar />
        </div>
      </div>

      <!-- 轨道合并弹窗：选择同类型目标轨道 -->
      <NModal
        v-model:show="mergeModal"
        preset="card"
        title="合并轨道"
        style="width: 400px"
      >
        <div class="merge-form">
          <p class="merge-hint">
            将轨道「{{ mergeSrc?.name }}」并入：
          </p>
          <NSelect
            v-model:value="mergeDstId"
            :options="mergeOptions"
            placeholder="选择同类型目标轨道"
          />
        </div>
        <template #footer>
          <NButton size="small" @click="mergeModal = false">取消</NButton>
          <NButton
            size="small"
            type="primary"
            :disabled="!mergeDstId"
            @click="onMergeConfirm"
          >
            合并
          </NButton>
        </template>
      </NModal>
    </div>

    <!-- Playhead window：仅覆盖内容区，标头越界时被裁剪而不上溢到工具条/标签列 -->
    <div
      class="playhead-window"
      :style="{ left: TOOL_WIDTH + LABEL_WIDTH + 'px' }"
    >
      <div class="playhead-overlay" :style="{ left: playheadLeft }">
        <div class="playhead-head" />
        <div class="playhead-line" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline-root {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: row;
  border-top: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  flex-shrink: 0;
  overflow: hidden;
}

.timeline-col {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.tl-row {
  display: flex;
  min-height: 0;
}

.tl-tracks {
  /* 滚动容器：block 子行高度由内容决定（行高固定），溢出时垂直滚动 */
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.track-row {
  border-top: 1px solid var(--color-border);
}

.timeline-footer {
  border-top: 1px solid var(--color-border);
  flex-shrink: 0;
}

.tl-label-col {
  position: relative;
  z-index: 30;
  min-width: 180px;
  max-width: 180px;
  border-right: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 4px 10px;
  flex-shrink: 0;
  cursor: pointer;
}

/* 轨道操作按钮组：hover 显示，右缘垂直居中 */
.tl-label-actions {
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  gap: 2px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.12s;
}

.tl-label-col:hover .tl-label-actions {
  opacity: 1;
  pointer-events: auto;
}

.tl-label-btn {
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 3px;
  background: var(--color-bg-tertiary);
  color: var(--color-text-secondary);
  font-size: 10px;
  line-height: 1;
  padding: 0;
  cursor: pointer;
}

.tl-label-btn:hover {
  background: var(--color-accent);
  color: #fff;
}

.tl-label-btn-danger:hover {
  background: var(--color-error);
  color: #fff;
}

.merge-form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.merge-hint {
  margin: 0;
  font-size: 13px;
  color: var(--color-text-primary);
  word-break: break-all;
}

.tl-label-col.track-focused {
  background: var(--color-bg-tertiary);
  box-shadow: inset 2px 0 0 var(--color-accent);
}

.label-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.label-name-text {
  display: block;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tl-name-input {
  width: 100%;
  font-size: 12px;
  font-weight: 600;
  font-family: inherit;
  color: var(--color-text-primary);
  background: var(--color-bg-primary);
  border: 1px solid var(--color-accent);
  border-radius: 4px;
  padding: 1px 4px;
  outline: none;
  box-sizing: border-box;
}

.label-type {
  font-size: 10px;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
}

.tl-content {
  flex: 1;
  overflow: hidden;
  position: relative;
  min-height: 40px;
}

.empty-hint {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  font-size: 11px;
  color: var(--color-text-secondary);
  opacity: 0.4;
  white-space: nowrap;
  pointer-events: none;
}

.playhead-window {
  position: absolute;
  top: 0;
  bottom: 0;
  right: 0;
  overflow: hidden;
  pointer-events: none;
}

.playhead-overlay {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  z-index: 15;
  pointer-events: none;
}

.playhead-head {
  width: 10px;
  height: 10px;
  background: var(--color-error);
  border-radius: 3px 3px 0 0;
  margin-left: -5px;
}

.playhead-line {
  width: 1px;
  height: 100%;
  background: var(--color-error);
  margin-left: -0.5px;
}
</style>
