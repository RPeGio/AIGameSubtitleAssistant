<script setup lang="ts">
import { useTimelineStore } from "../../stores/timeline";
import TimelineRuler from "./TimelineRuler.vue";
import TimelineScrollbar from "./TimelineScrollbar.vue";

const timeline = useTimelineStore();

function onWheelTracks(e: WheelEvent) {
  e.preventDefault();
  timeline.pan(e.deltaY);
}

function onWheelScrollbar(e: WheelEvent) {
  e.preventDefault();
  const factor = e.deltaY < 0 ? 1.1 : 0.9;
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  timeline.zoom(factor, e.clientX - rect.left);
}

function onClickTracks(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const x = e.clientX - rect.left;
  timeline.seek(timeline.timeAtPixel(x));
}
</script>

<template>
  <div class="timeline-root">
    <div class="timeline-header">
      <div class="track-label-col"></div>
      <div
        class="timeline-body"
        @wheel.prevent="onWheelTracks"
        @click="onClickTracks"
      >
        <TimelineRuler />
        <div class="tracks-area">
          <!-- 1.4b 轨道渲染占位 -->
        </div>
      </div>
    </div>
    <div class="timeline-footer">
      <div class="track-label-col"></div>
      <TimelineScrollbar @wheel.prevent="onWheelScrollbar" />
    </div>
  </div>
</template>

<style scoped>
.timeline-root {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  min-height: 0;
  flex-shrink: 0;
}

.timeline-header {
  display: flex;
}

.track-label-col {
  min-width: 180px;
  max-width: 180px;
  border-right: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
}

.timeline-body {
  flex: 1;
  overflow: hidden;
  position: relative;
  cursor: pointer;
}

.tracks-area {
  position: relative;
  height: 40px;
}

.timeline-footer {
  display: flex;
  border-top: 1px solid var(--color-border);
  height: 14px;
}
</style>
