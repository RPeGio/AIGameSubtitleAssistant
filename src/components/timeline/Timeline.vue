<script setup lang="ts">
import { useTimelineStore, CLIP_COLORS } from "../../stores/timeline";
import { useProjectStore } from "../../stores/project";
import TimelineRuler from "./TimelineRuler.vue";
import TimelineClip from "./TimelineClip.vue";
import TimelineScrollbar from "./TimelineScrollbar.vue";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

const tracks = () => projectStore.currentProject?.tracks ?? [];

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
  timeline.focusClip(null);
}
</script>

<template>
  <div class="timeline-root">
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
        class="tl-content"
        @wheel.prevent="onWheelTracks"
        @click="onClickTracks"
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
        <TimelineScrollbar @wheel.prevent="onWheelScrollbar" />
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
}
</style>
