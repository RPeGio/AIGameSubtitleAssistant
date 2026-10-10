<script setup lang="ts">
import { computed, watch } from "vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import VideoPlayer from "./VideoPlayer.vue";
import Timeline from "./timeline/Timeline.vue";
import TrackOverview from "./TrackOverview.vue";
import { NButton, NSpace, NTag } from "naive-ui";

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

// 视频加载后确保基础产物轨存在（无 ocr_text 轨时补 mock 供总览展示），
// 并聚焦产物轨让总览立即可见。OCR 选区控制轨由语料页/转写页各自建立，
// 不进全局时间轴；校对区只聚焦产物轨（scope=output）。
watch(
  () => timeline.duration,
  (d) => {
    if (d > 0) {
      projectStore.ensureDefaultTrack(d);
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
    <div class="review-header">
      <span class="review-title">校对区</span>
      <span class="review-sub">视频预览 · 轨道总览 · 时间轴</span>
    </div>

    <!-- 已导入视频：预览 + 总览 + 时间轴 -->
    <div v-if="hasVideo" class="review-fill">
      <div class="info-bar">
        <NSpace wrap size="small">
          <NTag>{{ displayPath }}</NTag>
          <NTag>{{ meta?.width }}×{{ meta?.height }}</NTag>
          <NTag v-if="resolutionLabel">{{ resolutionLabel }}</NTag>
          <NTag>{{ durationFormatted }}</NTag>
          <NTag>{{ meta?.fps.toFixed(1) }}fps</NTag>
          <NTag>{{ meta?.codec }}</NTag>
        </NSpace>

        <NButton size="small" type="primary" @click="projectStore.importVideo()">
          更换视频
        </NButton>
      </div>

      <div class="preview-row">
        <div class="player-area">
          <VideoPlayer :src="meta!.path" />
        </div>

        <div class="overview-area">
          <TrackOverview />
        </div>
      </div>

      <div class="timeline-pane">
        <Timeline />
      </div>
    </div>

    <!-- 空态：未导入视频 -->
    <div v-else class="review-empty">
      <div class="empty-icon">📹</div>
      <p class="empty-desc">导入切片视频以开始字幕生产</p>

      <NButton
        size="large"
        type="primary"
        @click="projectStore.importVideo()"
        class="import-btn"
      >
        导入视频
      </NButton>

      <p class="empty-hint">支持 mp4 / mkv / webm / avi / mov / flv</p>
    </div>
  </aside>
</template>

<style scoped>
.review-pane {
  flex: 1 1 65%;
  min-width: 520px;
  display: flex;
  flex-direction: column;
  background: var(--color-bg-secondary);
  border-left: 1px solid var(--color-border);
  overflow: hidden;
}

.review-header {
  padding: 12px 16px;
  border-bottom: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex-shrink: 0;
}

.review-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text-primary);
}

.review-sub {
  font-size: 12px;
  color: var(--color-text-secondary);
}

.review-fill {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.preview-row {
  flex: 1 1 65%;
  min-height: 0;
  display: flex;
  gap: 8px;
  padding: 0 12px 8px;
}

.info-bar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 8px 12px;
}

.player-area {
  flex: 1 1 60%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.overview-area {
  flex: 1 1 40%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.timeline-pane {
  flex: 0 0 35%;
  min-height: 0;
  overflow: hidden;
  padding: 0 12px 12px;
}

.review-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  color: var(--color-text-secondary);
  padding: 24px;
}

.empty-icon {
  font-size: 64px;
  margin-bottom: 16px;
}

.empty-desc {
  margin: 8px 0 24px;
  font-size: 14px;
}

.import-btn {
  margin-bottom: 16px;
}

.empty-hint {
  font-size: 12px;
  color: var(--color-text-secondary);
  opacity: 0.6;
}
</style>
