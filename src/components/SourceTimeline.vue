<script setup lang="ts">
import { createPinia } from "pinia";
import { computed, provide, watch } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import Timeline from "./timeline/Timeline.vue";
import type { Track } from "../types";

/// 视频源迷你时间轴：直接复用校对区的 Timeline 组件（同款式/交互），
/// 但注入独立 timeline store 实例，与全局校对时间轴完全隔离（互不串扰）。
/// 只显示指定视频源（videoKey）的 OCR 选区控制轨。
/// videoKey 缺省 "source" 保持语料页行为。

const props = withDefaults(
  defineProps<{
    /// 当前播放时间（v-model:time）
    time: number;
    /// 视频总时长
    duration: number;
    /// 绑定视频源："source" 剧情录屏（默认，语料页）| "clip" 切片（转写页嵌字 OCR）
    videoKey?: "source" | "clip";
  }>(),
  { videoKey: "source" }
);

const emit = defineEmits<{
  "update:time": [t: number];
}>();

const projectStore = useProjectStore();

// 独立 store 实例：本地 pinia 上注册 timeline store，供 Timeline 及其子组件注入
const localPinia = createPinia();
const timeline = useTimelineStore(localPinia);
provide(TIMELINE_STORE_KEY, timeline);

// 只显示该视频源的 OCR 选区控制轨
const trackFilter = (t: Track) => t.type === "ocr_region" && t.video === props.videoKey;

const regionTrack = computed(() =>
  projectStore.currentProject?.tracks.find(
    (t) => t.type === "ocr_region" && t.video === props.videoKey
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
// 外部大跳（>0.5s）走 jumpTo：它会在必要时做最小滚动让播放头重新可见；
// 播放中的小步进仍走 tick，避免逐帧触发滚动。
watch(
  () => props.time,
  (t) => {
    if (!timeline.isSeeking && Math.abs(t - timeline.currentTime) > 0.5) {
      timeline.jumpTo(t);
    } else {
      timeline.tick(t);
    }
  }
);

// 时间轴交互（拖动刻度/点击 clip）→ 同步视频（父级 v-model:time）
watch(
  () => timeline.currentTime,
  (t) => emit("update:time", t)
);

/// 双击轨道空白：以点击处为中心 ±1.5s 添加选区（快捷方式，保留）
function onDblClickTrack(time: number) {
  if (!regionTrack.value) return;
  const start = Math.max(0, time - 1.5);
  const end = Math.min(props.duration, time + 1.5);
  projectStore.addOcrRegionEvent(regionTrack.value.id, start, end);
  emit("update:time", time);
}

/// 加选区工具拖出的草稿：落库为正式选区，并把播放头放到选区起点
function onCreateTrackRegion(payload: { trackId: string; start: number; end: number }) {
  projectStore.addOcrRegionEvent(payload.trackId, payload.start, payload.end);
  timeline.setTime(payload.start);
  emit("update:time", payload.start);
}
</script>

<template>
  <div class="source-timeline">
    <div class="tl-shell">
      <Timeline
        :track-filter="trackFilter"
        :show-tool-strip="true"
        tool-variant="mini"
        timeline-role="mini"
        :video-source="props.videoKey"
        :click-seeks="true"
        :label-width="132"
        @dblclick-track="onDblClickTrack"
        @create-track-region="onCreateTrackRegion"
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

/* Timeline 根节点是 height:100%，需给固定高度的容器
   110 = 工具条 30 + 标尺 20 + 车道 48 + 滑条 8 + 边框余量 */
.tl-shell {
  height: 110px;
  min-height: 0;
}
</style>
