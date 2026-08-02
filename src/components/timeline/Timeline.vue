<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
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
const tracksBodyRef = ref<HTMLElement | null>(null);
const viewportRef = ref<HTMLElement | null>(null);
let viewportObserver: ResizeObserver | null = null;
let scrubbingTrackBody: HTMLElement | null = null;

const playheadLeft = computed(
  () => TOOL_WIDTH + LABEL_WIDTH + timeline.playheadX() + "px"
);

// ── 轨道区域 mousedown/mousemove/mouseup ──

function onTrackBodyMouseDown(e: MouseEvent) {
  const target = e.currentTarget as HTMLElement;
  const rect = target.getBoundingClientRect();

  // 分割工具：在 clip 上按下即分割，空白处不动作
  if (timeline.activeTool === "split") {
    const clipEl = (e.target as HTMLElement).closest(".clip") as HTMLElement | null;
    const eventId = clipEl?.dataset.eventId;
    if (!eventId) return;
    const time = timeline.timeAtPixel(e.clientX - rect.left);
    const rightId = projectStore.splitEvent(eventId, time);
    if (rightId) {
      timeline.focusClip(rightId);
      const found = projectStore.findEvent(rightId);
      if (found) timeline.focusTrack(found.track.id);
    }
    return;
  }

  // 选择工具：擦动时间轴，并聚焦当前轨道
  e.preventDefault();
  scrubbingTrackBody = target;
  timeline.scrubbing(true);
  timeline.seek(timeline.timeAtPixel(e.clientX - rect.left));
  timeline.focusTrack(target.dataset.trackId ?? null);
  timeline.focusClip(null);
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!scrubbingTrackBody) return;
  const rect = scrubbingTrackBody.getBoundingClientRect();
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
  if (!scrubbingTrackBody) return;
  scrubbingTrackBody = null;
  timeline.scrubbing(false);
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

// 选择模式下点击 clip：聚焦 clip + 其所在轨道
function onClipClicked(track: Track, clipId: string) {
  if (timeline.activeTool === "split") return;
  timeline.focusClip(clipId);
  timeline.focusTrack(track.id);
}

// 点击轨道名称标签：聚焦该轨道
function onTrackLabelClicked(track: Track) {
  timeline.focusClip(null);
  timeline.focusTrack(track.id);
}

// ── 滚轮 ──

function onWheelTracks(e: WheelEvent) {
  e.preventDefault();
  timeline.pan(e.deltaY);
}

function setupWheelListeners(el: HTMLElement) {
  el.addEventListener("wheel", onWheelTracks, { passive: false });
}

function teardownWheelListeners(el: HTMLElement) {
  el.removeEventListener("wheel", onWheelTracks);
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
  if (tracksBodyRef.value) {
    const bodies = tracksBodyRef.value.querySelectorAll(".tl-track-body");
    bodies.forEach((el) => setupWheelListeners(el as HTMLElement));
  }
});

onUnmounted(() => {
  viewportObserver?.disconnect();
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
  if (tracksBodyRef.value) {
    const bodies = tracksBodyRef.value.querySelectorAll(".tl-track-body");
    bodies.forEach((el) => teardownWheelListeners(el as HTMLElement));
  }
});
</script>

<template>
  <div class="timeline-root" ref="tracksBodyRef">
    <TimelineToolStrip />

    <div class="timeline-col">
      <!-- Header: ruler -->
      <div class="tl-row">
        <div class="tl-label-col" />
        <div
          ref="viewportRef"
          class="tl-content"
          @mousedown="onTrackBodyMouseDown"
        >
          <TimelineRuler />
        </div>
      </div>

      <!-- Track rows -->
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
          <div class="label-name">{{ track.name }}</div>
          <div class="label-type">{{ track.type }}</div>
        </div>
        <div
          class="tl-content tl-track-body"
          :data-track-id="track.id"
          @mousedown="onTrackBodyMouseDown"
        >
          <template v-if="track.events.length > 0">
            <TimelineClip
              v-for="event in track.events"
              :key="event.id"
              :event="event"
              :color="CLIP_COLORS[event.type] ?? '#666'"
              :left="timeline.clipPosition(event).left"
              :width="timeline.clipPosition(event).width"
              :focused="timeline.focusedClipId === event.id"
              @click-clip="onClipClicked(track, event.id)"
            />
          </template>
          <div v-else class="empty-hint">此轨道暂无事件</div>
        </div>
      </div>

      <!-- Footer: scrollbar -->
      <div class="tl-row">
        <div class="tl-label-col" />
        <div class="tl-content">
          <TimelineScrollbar />
        </div>
      </div>
    </div>

    <!-- Playhead overlay — spans full height across all rows -->
    <div class="playhead-overlay" :style="{ left: playheadLeft }">
      <div class="playhead-head" />
      <div class="playhead-line" />
    </div>
  </div>
</template>

<style scoped>
.timeline-root {
  position: relative;
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

.track-row {
  border-top: 1px solid var(--color-border);
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

.playhead-overlay {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  z-index: 20;
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
