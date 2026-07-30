<script setup lang="ts">
import { computed, watch } from "vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import AppSidebar from "../components/AppSidebar.vue";
import VideoPlayer from "../components/VideoPlayer.vue";
import Timeline from "../components/timeline/Timeline.vue";
import { NButton, NTag, NSpace, NAlert } from "naive-ui";

const projectStore = useProjectStore();
const timeline = useTimelineStore();

watch(
  () => timeline.duration,
  (d) => {
    if (d > 0) projectStore.ensureDefaultTrack(d);
  }
);

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
</script>

<template>
  <div class="editor-layout">
    <AppSidebar />
    <main class="editor-main">
      <!-- video imported -->
      <div v-if="hasVideo" class="editor-content">
        <div class="player-area">
          <VideoPlayer :src="meta!.path" />
        </div>

        <div class="info-bar">
          <NSpace wrap size="small">
            <NTag>{{ displayPath }}</NTag>
            <NTag>{{ meta?.width }}×{{ meta?.height }}</NTag>
            <NTag v-if="resolutionLabel">{{ resolutionLabel }}</NTag>
            <NTag>{{ durationFormatted }}</NTag>
            <NTag>{{ meta?.fps.toFixed(1) }}fps</NTag>
            <NTag>{{ meta?.codec }}</NTag>
          </NSpace>

          <NButton size="small" @click="projectStore.importVideo()">
            更换视频
          </NButton>
        </div>

        <Timeline />
      </div>

      <!-- empty state -->
      <div v-else class="editor-empty">
        <div class="empty-icon">📹</div>
        <h2>{{ projectStore.currentProject?.name ?? "加载中..." }}</h2>
        <p class="empty-desc">导入游戏录屏以开始字幕生产</p>

        <NButton
          size="large"
          type="primary"
          @click="projectStore.importVideo()"
          class="import-btn"
        >
          导入视频
        </NButton>

        <NAlert
          v-if="projectStore.videoImportError"
          type="error"
          class="error-alert"
          closable
          @close="projectStore.videoImportError = null"
        >
          {{ projectStore.videoImportError }}
        </NAlert>

        <p class="empty-hint">支持 mp4 / mkv / webm / avi / mov / flv</p>
      </div>
    </main>
  </div>
</template>

<style scoped>
.editor-layout {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

.editor-main {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-bg-primary);
}

.editor-content {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: 24px 24px 0;
  gap: 12px;
}

.player-area {
  flex-shrink: 0;
}

.info-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 10px 12px;
  background: var(--color-bg-secondary);
  border-radius: 8px;
}

.editor-empty {
  text-align: center;
  color: var(--color-text-secondary);
  max-width: 400px;
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

.error-alert {
  margin-top: 16px;
  text-align: left;
}

.empty-hint {
  font-size: 12px;
  color: var(--color-text-secondary);
  opacity: 0.6;
}
</style>
