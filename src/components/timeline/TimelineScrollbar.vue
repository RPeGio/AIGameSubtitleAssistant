<script setup lang="ts">
import { computed, onUnmounted } from "vue";
import { useTimelineStore } from "../../stores/timeline";

const timeline = useTimelineStore();

const container = computed(() => {
  const totalW = timeline.totalWidth;
  if (totalW <= 0) return { width: "100%", left: "0%" };
  const viewW = 800;
  const leftPct = Math.max(0, Math.min(100, (timeline.scrollLeft / totalW) * 100));
  const widthPct = Math.max(1, Math.min(100, (viewW / totalW) * 100));
  return { width: widthPct + "%", left: leftPct + "%" };
});

function calcPct(e: MouseEvent, el: HTMLElement): number {
  const rect = el.getBoundingClientRect();
  return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
}

let dragging: HTMLElement | null = null;

function onScrollbarMouseDown(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  e.preventDefault();
  dragging = el;
  timeline.scrollLeft = calcPct(e, el) * timeline.totalWidth;
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!dragging) return;
  timeline.scrollLeft = calcPct(e, dragging) * timeline.totalWidth;
}

function onWindowMouseUp() {
  dragging = null;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

function onWheelScrollbar(e: WheelEvent) {
  e.preventDefault();
  const factor = e.deltaY < 0 ? 1.1 : 0.9;
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  timeline.zoom(factor, e.clientX - rect.left);
}

onUnmounted(() => {
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
});
</script>

<template>
  <div
    class="scrollbar-track"
    @mousedown="onScrollbarMouseDown"
    @wheel.prevent="onWheelScrollbar"
  >
    <div
      class="scrollbar-thumb"
      :style="{ width: container.width, left: container.left }"
    />
  </div>
</template>

<style scoped>
.scrollbar-track {
  flex: 1;
  height: 14px;
  background: var(--color-bg-tertiary);
  position: relative;
  cursor: pointer;
}

.scrollbar-thumb {
  position: absolute;
  top: 2px;
  height: 10px;
  background: var(--color-accent);
  opacity: 0.6;
  border-radius: 5px;
  min-width: 20px;
  pointer-events: none;
  transition: opacity 0.1s;
}

.scrollbar-track:hover .scrollbar-thumb {
  opacity: 1;
}
</style>
