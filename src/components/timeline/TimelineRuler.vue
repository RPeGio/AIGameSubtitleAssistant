<script setup lang="ts">
import { computed, inject } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../../stores/timeline";

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();

/// 刻度间隔随缩放自适应（像素/秒越小，刻度越疏）
const ticks = computed(() => {
  const pps = timeline.pixelsPerSecond;
  if (pps <= 0 || timeline.duration <= 0) return [];

  let interval: number;
  if (pps >= 200) interval = 1;
  else if (pps >= 100) interval = 2;
  else if (pps >= 50) interval = 5;
  else if (pps >= 25) interval = 10;
  else interval = 30;

  const result: { x: number; label: string }[] = [];
  const startTime = Math.floor(timeline.scrollLeft / pps / interval) * interval;
  const endTime = Math.min(
    timeline.duration,
    (timeline.scrollLeft + 2000) / pps + interval
  );

  for (let t = startTime; t <= endTime; t += interval) {
    const x = t * pps - timeline.scrollLeft;
    if (x < -50 || x > 2500) continue;

    const h = Math.floor(t / 3600);
    const m = Math.floor((t % 3600) / 60);
    const s = t % 60;
    const label = h > 0
      ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
      : `${m}:${String(s).padStart(2, "0")}`;

    result.push({ x, label });
  }
  return result;
});
</script>

<template>
  <div class="ruler">
    <span
      v-for="tick in ticks"
      :key="tick.x"
      class="tick"
      :style="{ left: tick.x + 'px' }"
    >
      {{ tick.label }}
    </span>
  </div>
</template>

<style scoped>
/* 刻度尺：20px 高，刻度线 + 等宽小字标签（样稿 .tl-ruler 做法） */
.ruler {
  position: relative;
  height: 20px;
  border-bottom: 1px solid var(--color-border);
  font-family: var(--mono);
  font-size: 10px;
  line-height: 1;
  color: var(--faint);
  overflow: hidden;
}

.tick {
  position: absolute;
  top: 6px;
  padding-left: 4px;
  border-left: 1px solid var(--color-border);
  white-space: nowrap;
  pointer-events: none;
}
</style>
