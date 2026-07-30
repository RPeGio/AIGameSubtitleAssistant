<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useTimelineStore } from "../stores/timeline";

const props = defineProps<{ src: string }>();

const timeline = useTimelineStore();

const videoRef = ref<HTMLVideoElement | null>(null);
const currentTime = ref(0);
const duration = ref(0);
const isPlaying = ref(false);
const isLoaded = ref(false);
const loadError = ref<string | null>(null);

const assetUrl = computed(() => convertFileSrc(props.src));

const progressPct = computed(() =>
  duration.value > 0 ? (currentTime.value / duration.value) * 100 : 0
);

const timeDisplay = computed(() => {
  const fmt = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return `${m}:${String(sec).padStart(2, "0")}`;
  };
  return `${fmt(currentTime.value)} / ${fmt(duration.value)}`;
});

watch(
  () => props.src,
  () => {
    loadError.value = null;
    isLoaded.value = false;
    currentTime.value = 0;
    duration.value = 0;
    isPlaying.value = false;
    timeline.setDuration(0);
  }
);

watch(
  () => timeline.currentTime,
  (t) => {
    if (timeline.isSeeking && videoRef.value) {
      videoRef.value.currentTime = t;
    }
  }
);

function onLoadedMeta() {
  const v = videoRef.value;
  if (!v) return;
  duration.value = v.duration;
  isLoaded.value = true;
  timeline.setDuration(v.duration);
}

function onTimeUpdate() {
  const v = videoRef.value;
  if (!v) return;
  currentTime.value = v.currentTime;
  if (timeline.isSeeking) {
    timeline.seekDone();
    return;
  }
  timeline.tick(v.currentTime);
}

function onPlay() {
  isPlaying.value = true;
}

function onPause() {
  isPlaying.value = false;
}

function onError() {
  loadError.value = "视频加载失败，请检查文件是否可播放";
  isLoaded.value = false;
}

function togglePlay() {
  const v = videoRef.value;
  if (!v) return;
  if (v.paused) v.play();
  else v.pause();
}

function seekTo(e: Event) {
  const v = videoRef.value;
  if (!v) return;
  const target = e.target as HTMLInputElement;
  v.currentTime = (parseFloat(target.value) / 100) * duration.value;
}

function handleKeydown(e: KeyboardEvent) {
  if (e.code === "Space") {
    e.preventDefault();
    togglePlay();
  }
}

onMounted(() => window.addEventListener("keydown", handleKeydown));
onUnmounted(() => window.removeEventListener("keydown", handleKeydown));
</script>

<template>
  <div class="player-wrapper">
    <div class="player-container">
      <video
        ref="videoRef"
        :src="assetUrl"
        class="video-el"
        preload="metadata"
        @loadedmetadata="onLoadedMeta"
        @timeupdate="onTimeUpdate"
        @play="onPlay"
        @pause="onPause"
        @error="onError"
      />

      <div v-if="loadError" class="player-overlay">
        <span class="error-text">{{ loadError }}</span>
      </div>

      <div v-else-if="!isLoaded" class="player-overlay">
        <span class="loading-text">加载中...</span>
      </div>
    </div>

    <div class="controls">
      <button class="ctrl-btn" @click="togglePlay" :disabled="!isLoaded">
        {{ isPlaying ? "⏸" : "▶" }}
      </button>

      <input
        type="range"
        class="progress-bar"
        min="0"
        max="100"
        step="0.1"
        :value="progressPct"
        @input="seekTo"
      />

      <span class="time-text">{{ timeDisplay }}</span>
    </div>
  </div>
</template>

<style scoped>
.player-wrapper {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.player-container {
  position: relative;
  background: #000;
  border-radius: 8px;
  overflow: hidden;
  aspect-ratio: 16 / 9;
  max-height: 60vh;
}

.video-el {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.player-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.6);
  font-size: 14px;
  color: var(--color-text-secondary);
}

.error-text {
  color: var(--color-error);
}

.controls {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 4px;
}

.ctrl-btn {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 6px;
  background: var(--color-bg-tertiary);
  color: var(--color-text-primary);
  font-size: 14px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: background 0.15s;
}

.ctrl-btn:hover {
  background: var(--color-accent);
}

.ctrl-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.progress-bar {
  flex: 1;
  -webkit-appearance: none;
  appearance: none;
  height: 4px;
  border-radius: 2px;
  background: var(--color-bg-tertiary);
  outline: none;
}

.progress-bar::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--color-accent);
  cursor: pointer;
}

.progress-bar::-moz-range-thumb {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--color-accent);
  cursor: pointer;
  border: none;
}

.time-text {
  font-size: 12px;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
  min-width: 80px;
  text-align: right;
  flex-shrink: 0;
}
</style>
