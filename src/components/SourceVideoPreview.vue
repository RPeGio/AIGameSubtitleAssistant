<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";
import { useVideoContentRect } from "../composables/useVideoContentRect";
import { NButton } from "naive-ui";

const projectStore = useProjectStore();

/// 语料页剧情录屏独立预览：
/// - 完全独立于全局时间轴（不写 timeline store、不注册全局空格监听）
/// - 播放时间通过 v-model 与语料页迷你时间轴共享（视频播放 ↔ 时间轴播放头同步）
/// - 选区：按当前播放头所在 clip 显示（时间轴可分段添加多条选区），可拖拽调整

const props = defineProps<{
  /// 当前播放时间（v-model:time，父层持有共享）
  time: number;
  /// 视频总时长（v-model:duration）
  duration: number;
}>();

const emit = defineEmits<{
  "update:time": [t: number];
  "update:duration": [d: number];
}>();

const videoRef = ref<HTMLVideoElement | null>(null);
const isPlaying = ref(false);
const isLoaded = ref(false);
const loadError = ref<string | null>(null);

const { containerRef, contentRect } = useVideoContentRect();

const src = computed(() => projectStore.sourceVideoMeta?.path ?? "");
const assetUrl = computed(() => (src.value ? convertFileSrc(src.value) : ""));

/// source 控制轨（可能存在多条选区事件）
const sourceTrack = computed(() =>
  projectStore.currentProject?.tracks.find(
    (t) => t.type === "ocr_region" && t.video === "source"
  )
);

/// 当前播放头所在的选区事件；播放头不在任何 clip 内时取最后一个
const regionEvent = computed(() => {
  const track = sourceTrack.value;
  if (!track) return null;
  const t = props.time;
  const regions = track.events.filter(
    (e): e is Extract<typeof e, { type: "ocr_region" }> => e.type === "ocr_region"
  );
  const current = regions.find((e) => e.start <= t && e.end > t);
  return current ?? (regions.length > 0 ? regions[regions.length - 1] : null);
});

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
    if (Math.abs(v.currentTime - t) > 0.05) {
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
  v.currentTime = (parseFloat(target.value) / 100) * props.duration;
}

// ── 选区拖拽（不依赖 timeline store）────────────────────
let drag: null | {
  mode: "move" | "resize";
  handle: string;
  startX: number;
  startY: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
} = null;

function pct(v: number) {
  return v * 100 + "%";
}

const MIN = 0.02;

const rectStyle = computed(() => {
  const r = regionEvent.value;
  if (!r) return {};
  return {
    left: pct(r.x1),
    top: pct(r.y1),
    width: pct(r.x2 - r.x1),
    height: pct(r.y2 - r.y1),
  };
});

const dims = computed(() => {
  const r = regionEvent.value;
  if (!r) return [];
  return [
    { key: "top", style: { left: "0%", top: "0%", width: "100%", height: pct(r.y1) } },
    { key: "bottom", style: { left: "0%", top: pct(r.y2), width: "100%", height: pct(1 - r.y2) } },
    { key: "left", style: { left: "0%", top: pct(r.y1), width: pct(r.x1), height: pct(r.y2 - r.y1) } },
    { key: "right", style: { left: pct(r.x2), top: pct(r.y1), width: pct(1 - r.x2), height: pct(r.y2 - r.y1) } },
  ];
});

const HANDLES = [
  { name: "nw", left: "0%", top: "0%", cursor: "nwse-resize" },
  { name: "n", left: "50%", top: "0%", cursor: "ns-resize" },
  { name: "ne", left: "100%", top: "0%", cursor: "nesw-resize" },
  { name: "e", left: "100%", top: "50%", cursor: "ew-resize" },
  { name: "se", left: "100%", top: "100%", cursor: "nwse-resize" },
  { name: "s", left: "50%", top: "100%", cursor: "ns-resize" },
  { name: "sw", left: "0%", top: "100%", cursor: "nesw-resize" },
  { name: "w", left: "0%", top: "50%", cursor: "ew-resize" },
];

function clamp(v: number, lo: number, hi: number) {
  return Math.max(lo, Math.min(hi, v));
}

function startDrag(e: MouseEvent, mode: "move" | "resize", handle = "") {
  const r = regionEvent.value;
  if (!r) return;
  e.preventDefault();
  e.stopPropagation();
  drag = {
    mode,
    handle,
    startX: e.clientX,
    startY: e.clientY,
    x1: r.x1,
    y1: r.y1,
    x2: r.x2,
    y2: r.y2,
  };
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!drag) return;
  const cr = contentRect.value;
  if (cr.width <= 0 || cr.height <= 0) return;
  const r = regionEvent.value;
  if (!r) return;

  const dx = (e.clientX - drag.startX) / cr.width;
  const dy = (e.clientY - drag.startY) / cr.height;
  const { x1, y1, x2, y2 } = drag;

  if (drag.mode === "move") {
    let nx1 = x1 + dx;
    let nx2 = x2 + dx;
    if (nx1 < 0) {
      nx2 -= nx1;
      nx1 = 0;
    } else if (nx2 > 1) {
      nx1 -= nx2 - 1;
      nx2 = 1;
    }
    let ny1 = y1 + dy;
    let ny2 = y2 + dy;
    if (ny1 < 0) {
      ny2 -= ny1;
      ny1 = 0;
    } else if (ny2 > 1) {
      ny1 -= ny2 - 1;
      ny2 = 1;
    }
    projectStore.updateOcrRegion(r.id, { x1: nx1, y1: ny1, x2: nx2, y2: ny2 });
  } else {
    const west = drag.handle.includes("w");
    const east = drag.handle.includes("e");
    const north = drag.handle.includes("n");
    const south = drag.handle.includes("s");
    projectStore.updateOcrRegion(r.id, {
      x1: west ? clamp(x1 + dx, 0, x2 - MIN) : x1,
      x2: east ? clamp(x2 + dx, x1 + MIN, 1) : x2,
      y1: north ? clamp(y1 + dy, 0, y2 - MIN) : y1,
      y2: south ? clamp(y2 + dy, y1 + MIN, 1) : y2,
    });
  }
}

function onWindowMouseUp() {
  drag = null;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

onUnmounted(() => {
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
});
</script>

<template>
  <div class="source-preview">
    <div class="preview-header">
      <NButton size="small" type="primary" @click="projectStore.importSourceVideo()">
        更换剧情录屏
      </NButton>
    </div>

    <div ref="containerRef" class="preview-container">
      <video
        ref="videoRef"
        :src="assetUrl"
        class="video-el"
        preload="metadata"
        @loadedmetadata="onLoadedMeta"
        @timeupdate="onTimeUpdate"
        @play="isPlaying = true"
        @pause="isPlaying = false"
        @error="onError"
      />

      <!-- 选区遮罩：固定显示 source 控制轨的选区，可拖拽调整 -->
      <div v-if="regionEvent && isLoaded" class="region-overlay">
        <div
          v-for="d in dims"
          :key="d.key"
          class="dim"
          :style="d.style"
        />
        <div class="region-rect" :style="rectStyle" @mousedown="startDrag($event, 'move')">
          <div
            v-for="h in HANDLES"
            :key="h.name"
            class="handle"
            :style="{ left: h.left, top: h.top, cursor: h.cursor }"
            @mousedown.stop="startDrag($event, 'resize', h.name)"
          />
        </div>
      </div>

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

.preview-header {
  display: flex;
  justify-content: flex-end;
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

.region-overlay {
  position: absolute;
  inset: 0;
  z-index: 5;
  pointer-events: none;
}

.dim {
  position: absolute;
  background: rgba(0, 0, 0, 0.5);
  pointer-events: none;
}

.region-rect {
  position: absolute;
  box-sizing: border-box;
  border: 2px solid var(--color-accent);
  pointer-events: auto;
  cursor: move;
}

.handle {
  position: absolute;
  width: 10px;
  height: 10px;
  background: var(--color-accent);
  border: 1px solid #fff;
  border-radius: 2px;
  transform: translate(-50%, -50%);
  pointer-events: auto;
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
