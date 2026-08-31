<script setup lang="ts">
import { createPinia } from "pinia";
import { computed, provide, watch } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import Timeline from "./timeline/Timeline.vue";
import type { Track } from "../types";

/// 语料页迷你时间轴：直接复用校对区的 Timeline 组件（同款式/交互），
/// 但注入独立 timeline store 实例，与全局校对时间轴完全隔离（互不串扰）。
/// 只显示剧情录屏（source）的 OCR 选区控制轨。

const props = defineProps<{
  /// 当前播放时间（v-model:time）
  time: number;
  /// 视频总时长
  duration: number;
}>();

const emit = defineEmits<{
  "update:time": [t: number];
}>();

const projectStore = useProjectStore();

// 独立 store 实例：本地 pinia 上注册 timeline store，供 Timeline 及其子组件注入
const localPinia = createPinia();
const timeline = useTimelineStore(localPinia);
provide(TIMELINE_STORE_KEY, timeline);

// 只显示剧情录屏的 OCR 选区控制轨
const trackFilter = (t: Track) => t.type === "ocr_region" && t.video === "source";

const sourceTrack = computed(() =>
  projectStore.currentProject?.tracks.find(
    (t) => t.type === "ocr_region" && t.video === "source"
  )
);

// 视频时长 → 独立 store（刻度/滚动/吸附都依赖 duration）
watch(
  () => props.duration,
  (d) => {
    if (d > 0) timeline.setDuration(d);
  },
  { immediate: true }
);

// 视频播放推进 → 时间轴播放头（tick 在拖动时被 isSeeking 挡掉）
watch(
  () => props.time,
  (t) => timeline.tick(t)
);

// 时间轴交互（拖动刻度/点击 clip）→ 同步视频（父级 v-model:time）
watch(
  () => timeline.currentTime,
  (t) => emit("update:time", t)
);

/// 双击轨道空白：以点击处为中心 ±1.5s 添加选区
function onDblClickTrack(time: number) {
  if (!sourceTrack.value) return;
  const start = Math.max(0, time - 1.5);
  const end = Math.min(props.duration, time + 1.5);
  projectStore.addOcrRegionEvent(sourceTrack.value.id, start, end);
  emit("update:time", time);
}
</script>

<template>
  <div class="source-timeline">
    <div class="hint">
      双击空白加选区 · 拖动 clip 调起止 · 点击 clip 跳转 · 滚轮平移 · Ctrl+滚轮缩放
    </div>
    <div class="tl-shell">
      <Timeline
        :track-filter="trackFilter"
        :show-tool-strip="false"
        :click-seeks="true"
        @dblclick-track="onDblClickTrack"
      />
    </div>
  </div>
</template>

<style scoped>
.source-timeline {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  background: var(--color-bg-secondary);
  overflow: hidden;
}

.hint {
  padding: 4px 8px;
  font-size: 11px;
  color: var(--color-text-secondary);
  border-bottom: 1px solid var(--color-border);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
}

/* Timeline 根节点是 height:100%，需给固定高度的容器 */
.tl-shell {
  height: 200px;
  min-height: 0;
}
</style>
