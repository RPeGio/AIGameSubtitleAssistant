<script setup lang="ts">
import { computed } from "vue";
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

function onScrollbarClick(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  const rect = el.getBoundingClientRect();
  const pct = (e.clientX - rect.left) / rect.width;
  timeline.scrollLeft = pct * timeline.totalWidth;
}
</script>

<template>
  <div class="scrollbar-track" @click="onScrollbarClick">
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
  transition: opacity 0.1s;
}

.scrollbar-track:hover .scrollbar-thumb {
  opacity: 1;
}
</style>
