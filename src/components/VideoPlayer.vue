<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useRoute } from "vue-router";
import { useTimelineStore } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import { useVideoContentRect } from "../composables/useVideoContentRect";
import { onShortcut } from "../composables/shortcuts";
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

// ── 播放类快捷键（Space / J / K / L）：登记到统一分发器（键位表在 shortcuts.ts）──
// 活动闸门用「DOM 是否仍在文档里」：校对区被 KeepAlive 暂存时 isConnected=false，
// 孙组件收不到 activated/deactivated 钩子（见 Timeline.vue 同款注释）。
function isPlayerActive(): boolean {
  return containerRef.value?.isConnected ?? false;
}

// 倒放：浏览器不支持负 playbackRate，用 rAF 按真实耗时把 currentTime 递减
const reverseActive = ref(false);
let reverseRaf = 0;
let reverseLast = 0;

function stopReverse() {
  reverseActive.value = false;
  if (reverseRaf) cancelAnimationFrame(reverseRaf);
  reverseRaf = 0;
}

function startReverse() {
  const v = videoRef.value;
  if (!v || reverseRaf) return;
  v.pause();
  reverseActive.value = true;
  reverseLast = performance.now();
  const step = (now: number) => {
    const el = videoRef.value;
    if (!el || !reverseActive.value || !isPlayerActive()) {
      reverseRaf = 0;
      reverseActive.value = false;
      return;
    }
    const dt = Math.min(0.1, (now - reverseLast) / 1000);
    reverseLast = now;
    const next = el.currentTime - dt;
    if (next <= 0) {
      el.currentTime = 0;
      currentTime.value = 0;
      timeline.setTime(0);
      stopReverse();
      return;
    }
    el.currentTime = next;
    currentTime.value = next;
    timeline.setTime(next);
    reverseRaf = requestAnimationFrame(step);
  };
  reverseRaf = requestAnimationFrame(step);
}

const shortcutOffs: Array<() => void> = [];


// KeepAlive 收起（切到语料页）时停住播放：失活钩子收不到（见 handleKeydown 注释），
// 用路由当信号——校对区仅在非语料页显示（ProjectLayout 的 showReviewPane），
// 故切到语料页即视为被收起，摘掉 rAF 采样并暂停出声。
const route = useRoute();

watch(
  () => route.name,
  (n) => {
    if (n === "corpus") {
      cancelRafTicker();
      const v = videoRef.value;
      if (v && !v.paused) v.pause();
    }
  }
);

onMounted(() => {
  shortcutOffs.push(
    onShortcut(
      "playPause",
      () => {
        stopReverse();
        togglePlay();
      },
      isPlayerActive
    ),
    onShortcut("reverse", () => startReverse(), isPlayerActive),
    onShortcut(
      "stopPlay",
      () => {
        stopReverse();
        const v = videoRef.value;
        if (v && !v.paused) v.pause();
      },
      isPlayerActive
    ),
    onShortcut(
      "forward",
      () => {
        stopReverse();
        const v = videoRef.value;
        if (v && v.paused) togglePlay();
      },
      isPlayerActive
    )
  );
});

onUnmounted(() => {
  shortcutOffs.splice(0).forEach((off) => off());
  stopReverse();
});
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
/* 网格两行：画面（占剩余高度） + 控件条 32px，行距 6px */
.player-wrapper {
  position: relative;
  display: grid;
  grid-template-rows: minmax(0, 1fr) 32px;
  row-gap: 6px;
  height: 100%;
  min-height: 0;
}

/* 画面：在「剩余高度 × 可用宽度」内取最大的 16:9 框（两轴都 auto + 双 max 约束，
   由 aspect-ratio 保形）⇒ 窗口变矮/变宽都只缩小画面，不会溢出盖住控件条或信息栏 */
.player-container {
  position: relative;
  justify-self: center;
  align-self: center;
  aspect-ratio: 16 / 9;
  max-width: 100%;
  max-height: 100%;
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

/* 控件条在画面之下（不遮挡画面、不遮挡画面底部的选区手柄，见 docs/MISC_TODO 第 5 项） */
.controls {
  height: 32px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 4px;
  color: var(--dim);
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
  background: var(--hover);
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
  background: var(--accent);
  cursor: pointer;
}

.progress-bar::-moz-range-thumb {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--accent);
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
