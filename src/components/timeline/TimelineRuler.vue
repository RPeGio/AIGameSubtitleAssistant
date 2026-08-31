<script setup lang="ts">
import { computed, inject } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../../stores/timeline";
import { NText } from "naive-ui";

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();

const TICK_HEIGHT_MAJOR = 13;
const TICK_HEIGHT_MINOR = 7;

const ticks = computed(() => {
  const pps = timeline.pixelsPerSecond;
  if (pps <= 0 || timeline.duration <= 0) return [];

  let interval: number;
  if (pps >= 200) interval = 1;
  else if (pps >= 100) interval = 2;
  else if (pps >= 50) interval = 5;
  else if (pps >= 25) interval = 10;
  else interval = 30;

  const result: { x: number; label: string; isMajor: boolean }[] = [];
  const startTime = Math.floor(timeline.scrollLeft / pps / interval) * interval;
  const endTime = Math.min(
    timeline.duration,
    (timeline.scrollLeft + 2000) / pps + interval
  );

  for (let t = startTime; t <= endTime; t += interval) {
    const x = t * pps - timeline.scrollLeft;
    if (x < -50 || x > 2500) continue;

    const isMajor = (pps < 200 && t % 10 === 0) || (pps >= 200);
    const h = Math.floor(t / 3600);
    const m = Math.floor((t % 3600) / 60);
    const s = t % 60;
    const label = h > 0
      ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
      : `${m}:${String(s).padStart(2, "0")}`;

    result.push({ x, label, isMajor });
  }
  return result;
});
</script>

<template>
  <div class="ruler">
    <div
      v-for="tick in ticks"
      :key="tick.x"
      class="tick"
      :class="{ major: tick.isMajor }"
      :style="{ left: tick.x + 'px' }"
    >
      <div
        class="tick-line"
        :style="{
          height: (tick.isMajor ? TICK_HEIGHT_MAJOR : TICK_HEIGHT_MINOR) + 'px',
        }"
      />
      <NText v-if="tick.isMajor" class="tick-label" depth="3">
        {{ tick.label }}
      </NText>
    </div>
  </div>
</template>

<style scoped>
.ruler {
  position: relative;
  height: 28px;
  background: var(--color-bg-tertiary);
  border-bottom: 1px solid var(--color-border);
  overflow: hidden;
}

.tick {
  position: absolute;
  top: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  pointer-events: none;
}

.tick-line {
  width: 1px;
  background: var(--color-text-secondary);
  opacity: 0.4;
}

.tick.major .tick-line {
  opacity: 0.7;
}

.tick-label {
  font-size: 10px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  margin-top: 1px;
}
</style>
