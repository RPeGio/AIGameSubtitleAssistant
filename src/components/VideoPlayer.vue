<script setup lang="ts">
import { ref, computed, onActivated, onDeactivated, onUnmounted, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useTimelineStore } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import { useVideoContentRect } from "../composables/useVideoContentRect";
import SubtitleOverlay from "./SubtitleOverlay.vue";
import RegionOverlay from "./RegionOverlay.vue";
import AppIcon from "./AppIcon.vue";

const props = withDefaults(
  defineProps<{
    src: string;
    /// 是否叠加 OCR 选区覆盖层（校对区在转写页聚焦时显示）
    showRegion?: boolean;
  }>(),
  { showRegion: false }
);

const timeline = useTimelineStore();
const projectStore = useProjectStore();

/// 视频画面实际渲染矩形：选区覆盖层按它定位（letterbox 校正）
const { containerRef, contentRect } = useVideoContentRect("clip");

const videoRef = ref<HTMLVideoElement | null>(null);
const currentTime = ref(0);
const duration = ref(0);
const isPlaying = ref(false);
const isLoaded = ref(false);
const loadError = ref<string | null>(null);

/// 共享播放头时的身份：校对区播放器
///
/// ⚠ **前提**：共用一份 `timeline.currentTime` 只在「校对区与打轴页预览加载
/// 同一支视频」（同一 `currentVideoMeta`）时成立——这是当前的运行时巧合，
/// 不是代码保证的。若两者变成不同时长的文件，duration 各写各的、currentTime
/// 超出短视频时长被浏览器钳制 ⇒ 表现为"加载即跳时间"。届时应改为各持独立
/// 播放头，或给 timeline store 加"时长来源绑定"，不要沿用本共用机制。
const PLAYER_ID = "review";

const assetUrl = computed(() => convertFileSrc(props.src));

const progressPct = computed(() =>
  duration.value > 0 ? (currentTime.value / duration.value) * 100 : 0
);

// range 控件的已播部分（accent）用轨道渐变画出来，未播部分保持 hair-2
const progressStyle = computed(() => ({
  background: `linear-gradient(to right, var(--accent) ${progressPct.value}%, var(--hair-2) ${progressPct.value}%)`,
}));

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
    const v = videoRef.value;
    if (!v) return;
    // 拖动中，或与视频实际时间不一致时（外部跳转如轨道总览双击编辑）同步视频
    if (timeline.isSeeking || Math.abs(v.currentTime - t) > 0.02) {
      v.currentTime = t;
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
    return;
  }
  timeline.tick(v.currentTime);
}

/// 播放中每帧采样：<video> 的 timeupdate 只有约 4Hz，另一侧的播放头会一跳一跳；
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
    currentTime.value = v.currentTime;
    if (!timeline.isSeeking) timeline.tick(v.currentTime);
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

function onPlay() {
  isPlaying.value = true;
  timeline.claimPlayback(PLAYER_ID);
  startRafTicker();
}

function onPause() {
  isPlaying.value = false;
  cancelRafTicker();
}

function onError() {
  loadError.value = "视频加载失败，请检查文件是否可播放";
  isLoaded.value = false;
}

/// 另一侧（如打轴页预览）按下播放时让出播放权：只跟随时间，不再出声
watch(
  () => timeline.activePlayer,
  (id) => {
    const v = videoRef.value;
    if (v && id !== PLAYER_ID && !v.paused) v.pause();
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
  v.currentTime = (parseFloat(target.value) / 100) * duration.value;
}

function handleKeydown(e: KeyboardEvent) {
  // 编辑文字时（输入框/contentEditable）不拦截空格，避免吞掉输入
  const t = e.target as HTMLElement;
  if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;
  if (e.code === "Space") {
    e.preventDefault();
    togglePlay();
  }
}

// 组件套了 KeepAlive（校对区在语料页会被收起）：全局按键只在激活期间监听，
// 失活时摘掉并停住播放——否则在别的页面按空格会让隐藏的校对区播放器出声。
function activate() {
  window.addEventListener("keydown", handleKeydown);
}

function deactivate() {
  window.removeEventListener("keydown", handleKeydown);
  cancelRafTicker();
  const v = videoRef.value;
  if (v && !v.paused) v.pause();
}

onActivated(activate);
onDeactivated(deactivate);
onUnmounted(deactivate);
</script>

<template>
  <div class="player-wrapper">
    <div ref="containerRef" class="player-container">
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

      <SubtitleOverlay />

      <!-- OCR 选区覆盖层：焦点在转写页时显示，可直接在校对区画面里拖拽选区 -->
      <RegionOverlay
        v-if="props.showRegion"
        video-key="clip"
        :content-rect="contentRect"
        :time="timeline.currentTime"
      />

      <button
        class="cc-btn"
        :class="{ active: projectStore.subtitlePreviewOn }"
        :disabled="!isLoaded"
        title="字幕预览开关"
        @click="projectStore.subtitlePreviewOn = !projectStore.subtitlePreviewOn"
      >
        CC
      </button>

      <div v-if="loadError" class="player-overlay">
        <span class="error-text">{{ loadError }}</span>
      </div>

      <div v-else-if="!isLoaded" class="player-overlay">
        <span class="loading-text">加载中...</span>
      </div>
    </div>

    <div class="controls">
      <button class="ctrl-btn" @click="togglePlay" :disabled="!isLoaded" title="播放 / 暂停">
        <AppIcon :name="isPlaying ? 'pause' : 'play'" :size="14" />
      </button>

      <input
        type="range"
        class="progress-bar"
        min="0"
        max="100"
        step="0.1"
        :value="progressPct"
        :style="progressStyle"
        @input="seekTo"
      />

      <span class="time-text">{{ timeDisplay }}</span>
    </div>
  </div>
</template>

<style scoped>
.player-wrapper {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
}

.player-container {
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

.cc-btn {
  position: absolute;
  top: 10px;
  right: 12px;
  z-index: 20;
  height: 22px;
  padding: 0 6px;
  border: 1px solid var(--hair-2);
  border-radius: 4px;
  background: var(--v-cc-bg);
  color: var(--v-cc-fg);
  font: 11px/1 var(--mono);
  letter-spacing: 0.08em;
  cursor: pointer;
  transition: all 0.15s;
}

.cc-btn:hover {
  color: var(--v-cap-fg);
  border-color: var(--dim);
}

.cc-btn.active {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.cc-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.error-text {
  color: var(--danger);
}

/* 控件条覆盖在画面底部 32px 上（字幕会整体抬高让开，见 SubtitleOverlay） */
.controls {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: 15;
  height: 32px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 12px;
  background: var(--v-cap-bg);
  border-top: 1px solid var(--hair);
  border-radius: 0 0 8px 8px;
  color: var(--v-cap-fg);
}

.ctrl-btn {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: none;
  color: inherit;
  cursor: pointer;
  transition: background 0.15s;
}

.ctrl-btn:hover {
  background: rgba(255, 255, 255, 0.14);
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
  background: var(--hair-2);
  outline: none;
  cursor: pointer;
}

.progress-bar::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--v-cap-fg);
  cursor: pointer;
}

.progress-bar::-moz-range-thumb {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--v-cap-fg);
  cursor: pointer;
  border: none;
}

.time-text {
  flex-shrink: 0;
  min-width: 74px;
  text-align: center;
  font: 11px/1 var(--mono);
  color: inherit;
  font-variant-numeric: tabular-nums;
}
</style>
