<script setup lang="ts">
import { computed } from "vue";
import { useProjectStore } from "../stores/project";
import AppSidebar from "../components/AppSidebar.vue";
import { NButton, NCard, NTag, NSpace, NText, NAlert } from "naive-ui";

const projectStore = useProjectStore();

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
  const { width, height } = meta.value;
  if (width >= 3840) return "4K";
  if (width >= 2560) return "1440p";
  if (width >= 1920) return "1080p";
  if (width >= 1280) return "720p";
  return `${width}×${height}`;
});
</script>

<template>
  <div class="editor-layout">
    <AppSidebar />
    <main class="editor-main">
      <!-- 已导入视频状态 -->
      <div v-if="hasVideo" class="video-loaded">
        <NCard title="已导入视频" class="video-card">
          <NSpace vertical>
            <div class="meta-grid">
              <div class="meta-item">
                <NText depth="3">文件</NText>
                <NText strong>{{ displayPath }}</NText>
              </div>
              <div class="meta-item">
                <NText depth="3">分辨率</NText>
                <NSpace>
                  <NText strong>{{ meta?.width }}×{{ meta?.height }}</NText>
                  <NTag size="small">{{ resolutionLabel }}</NTag>
                </NSpace>
              </div>
              <div class="meta-item">
                <NText depth="3">时长</NText>
                <NText strong>{{ durationFormatted }}</NText>
              </div>
              <div class="meta-item">
                <NText depth="3">帧率</NText>
                <NText strong>{{ meta?.fps.toFixed(2) }} fps</NText>
              </div>
              <div class="meta-item">
                <NText depth="3">视频编码</NText>
                <NText strong>{{ meta?.codec || "—" }}</NText>
              </div>
              <div class="meta-item">
                <NText depth="3">音频编码</NText>
                <NText strong>{{ meta?.audio_codec ?? "—" }}</NText>
              </div>
            </div>
            <NButton size="small" @click="projectStore.importVideo()">更换视频</NButton>
          </NSpace>
        </NCard>
      </div>

      <!-- 未导入视频状态 -->
      <div v-else class="video-empty">
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
  padding: 40px;
}

.video-empty {
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

.video-loaded {
  width: 100%;
  max-width: 520px;
}

.video-card {
  width: 100%;
}

.meta-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}

.meta-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
</style>
