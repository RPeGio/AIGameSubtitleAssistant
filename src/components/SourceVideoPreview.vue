<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import { useVideoContentRect } from "../composables/useVideoContentRect";
import RegionOverlay from "./RegionOverlay.vue";

const projectStore = useProjectStore();
/// 全局时间轴 store：仅在 syncPlayback 时用于协调"谁出声"（播放时间本身走 v-model）
const timeline = useTimelineStore();

/// 视频源独立预览（语料页剧情录屏 / 转写页切片共用）：
/// - 完全独立于全局时间轴（不写 timeline store、不注册全局空格监听）
/// - 播放时间通过 v-model 与所在页迷你时间轴共享（视频播放 ↔ 时间轴播放头同步）
/// - 选区：按当前播放头所在 clip 显示（时间轴可分段添加多条选区），可拖拽调整
/// - videoKey 决定绑定的视频源（meta/选区控制轨）；缺省 "source" 保持语料页行为

const props = withDefaults(
  defineProps<{
    /// 当前播放时间（v-model:time，父层持有共享）
    time: number;
    /// 视频总时长（v-model:duration）
    duration: number;
    /// 绑定视频源："source" 剧情录屏（默认，语料页）| "clip" 切片（转写页嵌字 OCR）
    videoKey?: "source" | "clip";
    /// 是否参与共享播放权（打轴页与校对区共用播放头时置 true）：
    /// 另一侧开始播放时本预览只跟随时间、自动暂停，避免同一文件双声道
    syncPlayback?: boolean;
  }>(),
  { videoKey: "source", syncPlayback: false }
);

const emit = defineEmits<{
  "update:time": [t: number];
  "update:duration": [d: number];
}>();

const videoRef = ref<HTMLVideoElement | null>(null);
const isPlaying = ref(false);
const isLoaded = ref(false);
const loadError = ref<string | null>(null);

const { containerRef, contentRect } = useVideoContentRect(props.videoKey);

/// 对应视频源的文件路径：clip 用切片视频（项目主视频），source 用剧情录屏
const src = computed(() => {
  const meta =
    props.videoKey === "clip"
      ? projectStore.currentVideoMeta
      : projectStore.sourceVideoMeta;
  return meta?.path ?? "";
});
const assetUrl = computed(() => (src.value ? convertFileSrc(src.value) : ""));

const progressPct = computed(() =>
  props.duration > 0 ? Math.min(100, (props.time / props.duration) * 100) : 0
);

const timeDisplay = computed(() => {
  const fmt = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return `${m}:${String(sec).padStart(2, "0")}`;
  };
  return `${fmt(props.time)} / ${fmt(props.duration)}`;
});

watch(src, () => {
  loadError.value = null;
  isLoaded.value = false;
  emit("update:time", 0);
  emit("update:duration", 0);
  isPlaying.value = false;
});

// 外部（迷你时间轴播放头）seek：视频跟随；容差避免与 timeupdate 回写互相打架
watch(
  () => props.time,
  (t) => {
    const v = videoRef.value;
    if (!v || !isLoaded.value) return;
    if (Math.abs(v.currentTime - t) > 0.02) {
      v.currentTime = t;
    }
  }
);

function onLoadedMeta() {
  const v = videoRef.value;
  if (!v) return;
  emit("update:duration", v.duration);
  isLoaded.value = true;
}

function onTimeUpdate() {
  const v = videoRef.value;
  if (!v) return;
  emit("update:time", v.currentTime);
}

/// 播放中每帧上报时间：timeupdate 只有约 4Hz，另一侧的播放头会一跳一跳；
/// 用 rAF 直接读 currentTime 才能线性推进。onTimeUpdate 保留作后台标签页兜底。
let rafId = 0;

function startRafTicker() {
  cancelRafTicker();
  const step = () => {
    const v = videoRef.value;
    if (!v || v.paused || v.ended) {
      rafId = 0;
      return;
    }
    emit("update:time", v.currentTime);
    rafId = requestAnimationFrame(step);
  };
  rafId = requestAnimationFrame(step);
}

function cancelRafTicker() {
  if (rafId) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
}

function onError() {
  loadError.value = "视频加载失败，请检查文件是否可播放";
  isLoaded.value = false;
}

function onPlay() {
  isPlaying.value = true;
  if (props.syncPlayback) timeline.claimPlayback(props.videoKey);
  startRafTicker();
}

function onPause() {
  isPlaying.value = false;
  cancelRafTicker();
}

/// 另一侧（校对区播放器）按下播放时让出播放权：只跟随时间，不再出声
watch(
  () => timeline.activePlayer,
  (id) => {
    if (!props.syncPlayback) return;
    const v = videoRef.value;
    if (v && id !== props.videoKey && !v.paused) v.pause();
  }
);

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
  v.currentTime = (parseFloat(target.value) / 100) * props.duration;
}

onUnmounted(() => {
  cancelRafTicker();
});
</script>

<template>
  <div class="source-preview">
    <div ref="containerRef" class="preview-container">
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

      <!-- 选区遮罩：显示当前播放头所在选区的框与遮罩，可拖拽调整 -->
      <RegionOverlay
        v-if="isLoaded"
        :video-key="props.videoKey"
        :content-rect="contentRect"
        :time="props.time"
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
.source-preview {
  display: flex;
  flex-direction: column;
  gap: 8px;
  height: 100%;
}

.preview-container {
  position: relative;
  flex: 1;
  min-height: 0;
  background: #000;
  border-radius: 8px;
  overflow: hidden;
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

.time-text {
  font-size: 12px;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
  min-width: 80px;
  text-align: right;
  flex-shrink: 0;
}
</style>
