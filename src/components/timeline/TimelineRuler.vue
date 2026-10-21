<script setup lang="ts">
import { computed, inject } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../../stores/timeline";

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();

/// 刻度间隔候选（秒）：1 秒 → 1 天，取第一个够宽的
const STEPS = [
  1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1200, 1800,
  3600, 7200, 14400, 28800, 43200, 86400,
];
/// 相邻刻度至少留这么多像素，标签才不互相压住（10px 等宽最长标签约 46px + 左距 4px）
const MIN_TICK_PX = 56;
/// 低于这个间距连标签也不画，只留刻度线（超长素材 + 极窄视口）
const MIN_LABEL_PX = 44;

function timeLabel(t: number): string {
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = t % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${m}:${String(s).padStart(2, "0")}`;
}

/// 刻度间隔按像素密度自适应（旧实现按 pps 分档、30s 封顶 ⇒ 长素材最小缩放下标签糊成一片）
const ticks = computed(() => {
  const pps = timeline.pixelsPerSecond;
  if (pps <= 0 || timeline.duration <= 0) return [];

  const interval = STEPS.find((s) => s * pps >= MIN_TICK_PX) ?? STEPS[STEPS.length - 1];
  const showLabel = interval * pps >= MIN_LABEL_PX;

  const result: { x: number; label: string }[] = [];
  const viewport = timeline.viewportWidth > 0 ? timeline.viewportWidth : 2000;
  const startTime = Math.floor(timeline.scrollLeft / pps / interval) * interval;
  const endTime = Math.min(
    timeline.duration,
    (timeline.scrollLeft + viewport) / pps + interval
  );

  for (let t = startTime; t <= endTime; t += interval) {
    const x = t * pps - timeline.scrollLeft;
    if (x < -50 || x > viewport + 100) continue;
    result.push({ x, label: showLabel ? timeLabel(t) : "" });
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
