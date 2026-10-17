<script setup lang="ts">
import { computed, watch } from "vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import VideoPlayer from "./VideoPlayer.vue";
import Timeline from "./timeline/Timeline.vue";
import TrackOverview from "./TrackOverview.vue";
import AppIcon from "./AppIcon.vue";

const projectStore = useProjectStore();
const timeline = useTimelineStore();

const meta = computed(() => projectStore.currentVideoMeta);
const hasVideo = computed(() => meta.value !== null);

const displayPath = computed(() => {
  if (!meta.value) return "";
  const parts = meta.value.path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] ?? meta.value.path;
});

const durationFormatted = computed(() => {
  if (!meta.value) return "";
  const s = Math.round(meta.value.duration);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  return `${m}:${String(sec).padStart(2, "0")}`;
});

const resolutionLabel = computed(() => {
  if (!meta.value) return "";
  const { width } = meta.value;
  if (width >= 3840) return "4K";
  if (width >= 2560) return "1440p";
  if (width >= 1920) return "1080p";
  if (width >= 1280) return "720p";
  return "";
});

// 视频加载后聚焦产物轨让总览立即可见。OCR 选区控制轨由语料页/转写页各自建立，
// 不进全局时间轴；校对区只聚焦产物轨（scope=output）。
watch(
  () => timeline.duration,
  (d) => {
    if (d > 0) {
      const outputTrack = projectStore.currentProject?.tracks.find(
        (t) => t.scope !== "control"
      );
      if (outputTrack && !timeline.focusedTrackId) {
        timeline.focusTrack(outputTrack.id);
      }
    }
  }
);
</script>

<template>
  <aside class="review-pane">
    <div class="rvhead">
      <span class="t">校对区</span>
      <span class="s">视频预览 · 轨道总览 · 时间轴</span>
    </div>

    <template v-if="hasVideo">
      <div class="rvinfo">
        <span class="fname" :title="meta!.path">{{ displayPath }}</span>
        <span class="sp" />
        <span class="pill">{{ meta!.width }}×{{ meta!.height }}</span>
        <span v-if="resolutionLabel" class="pill">{{ resolutionLabel }}</span>
        <span class="pill">{{ durationFormatted }}</span>
        <span class="pill">{{ meta!.fps.toFixed(1) }}fps</span>
        <span class="pill">{{ meta!.codec }}</span>
        <button class="btn ghost sm" @click="projectStore.importVideo()">
          <AppIcon name="refresh" :size="12" />更换视频
        </button>
      </div>

      <div class="rvbody">
        <div class="rvtop">
          <div class="player-area">
            <VideoPlayer :src="meta!.path" />
          </div>

          <div class="overview-area">
            <TrackOverview />
          </div>
        </div>

        <div class="rvbot">
          <Timeline show-waveform />
        </div>
      </div>
    </template>

    <!-- 空态：未导入视频 -->
    <div v-else class="review-empty">
      <AppIcon name="track" :size="40" class="empty-ic" />
      <p class="empty-desc">导入切片视频以开始字幕生产</p>

      <button class="btn pri lg" @click="projectStore.importVideo()">导入视频</button>

      <p class="empty-hint">支持 mp4 / mkv / webm / avi / mov / flv</p>
    </div>
  </aside>
</template>

<style scoped>
.review-pane {
  /* 与主列 35% 相对：flex-basis 决定 35/65 的分割比例，勿改成 auto */
  flex: 1 1 65%;
  min-width: 520px;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  border-left: 1px solid var(--hair);
  overflow: hidden;
}

.rvhead {
  flex: none;
  height: 32px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px;
  background: var(--panel);
  border-bottom: 1px solid var(--hair);
}

.rvhead .t {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--dim);
}

.rvhead .s {
  font-size: 11px;
  color: var(--faint);
}

.rvinfo {
  flex: none;
  height: 30px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  background: var(--panel);
  border-bottom: 1px solid var(--hair);
  font: 11px/1 var(--mono);
  color: var(--dim);
  min-width: 0;
}

.rvinfo .fname {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--text);
}

.rvbody {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px 12px;
}

.rvtop {
  flex: 1.1;
  min-height: 0;
  display: flex;
  gap: 12px;
  align-items: center;
}

/* 预览按 16:9 定尺（宽由 62% 决定），窗口矮时收缩并让视频自己加黑边 */
.player-area {
  flex: 0 0 62%;
  width: auto;
  height: auto;
  aspect-ratio: 16 / 9;
  max-height: 100%;
  min-width: 0;
  align-self: center;
}

.overview-area {
  flex: 1 1 38%;
  min-width: 0;
  min-height: 0;
  align-self: stretch;
  display: flex;
}

/* 时间轴自己的外层卡片（内部样式见 timeline/，T7 处理） */
.rvbot {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  overflow: hidden;
}

.review-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  text-align: center;
  color: var(--dim);
  padding: 24px;
}

.empty-ic {
  color: var(--faint);
}

.empty-desc {
  font-size: 13px;
}

.empty-hint {
  font-size: 11px;
  color: var(--faint);
}
</style>
