<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { useTimelineStore, CLIP_COLORS } from "../../stores/timeline";
import { useProjectStore } from "../../stores/project";
import TimelineRuler from "./TimelineRuler.vue";
import TimelineClip from "./TimelineClip.vue";
import TimelineScrollbar from "./TimelineScrollbar.vue";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

const tracks = () => projectStore.currentProject?.tracks ?? [];
const tracksBodyRef = ref<HTMLElement | null>(null);
let scrubbingTrackBody: HTMLElement | null = null;

// ── 轨道区域 mousedown/mousemove/mouseup ──

function onTrackBodyMouseDown(e: MouseEvent) {
  const target = e.currentTarget as HTMLElement;
  const rect = target.getBoundingClientRect();
  e.preventDefault();
  scrubbingTrackBody = target;
  timeline.scrubbing(true);
  timeline.seek(timeline.timeAtPixel(e.clientX - rect.left));
  timeline.focusClip(null);
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!scrubbingTrackBody) return;
  const rect = scrubbingTrackBody.getBoundingClientRect();
  timeline.currentTime = timeline.timeAtPixel(e.clientX - rect.left);
}

function onWindowMouseUp() {
  if (!scrubbingTrackBody) return;
  scrubbingTrackBody = null;
  timeline.scrubbing(false);
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
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
  if (tracksBodyRef.value) {
    const bodies = tracksBodyRef.value.querySelectorAll(".tl-track-body");
    bodies.forEach((el) => setupWheelListeners(el as HTMLElement));
  }
});

onUnmounted(() => {
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
    <!-- Header: ruler -->
    <div class="tl-row">
      <div class="tl-label-col" />
      <div class="tl-content">
        <TimelineRuler />
      </div>
    </div>

    <!-- Track rows -->
    <div
      v-for="track in tracks()"
      :key="track.id"
      class="tl-row track-row"
    >
      <div class="tl-label-col">
        <div class="label-name">{{ track.name }}</div>
        <div class="label-type">{{ track.type }}</div>
      </div>
      <div
        class="tl-content tl-track-body"
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
            @click-clip="timeline.focusClip(event.id)"
          />
        </template>
        <div v-else class="empty-hint">点击左侧时间轴跳转，在此轨道暂无事件</div>
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
</template>

<style scoped>
.timeline-root {
  border-top: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.tl-row {
  display: flex;
  min-height: 0;
}

.track-row {
  border-top: 1px solid var(--color-border);
}

.tl-label-col {
  min-width: 180px;
  max-width: 180px;
  border-right: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 4px 10px;
  flex-shrink: 0;
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
</style>
