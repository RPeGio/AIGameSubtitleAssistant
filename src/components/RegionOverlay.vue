<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useTimelineStore } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import type { OcrRegionEvent } from "../types";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

const rootRef = ref<HTMLElement | null>(null);
const containerSize = ref({ width: 0, height: 0 });
let resizeObserver: ResizeObserver | null = null;

// 拖拽状态：mode=move 平移整个选区；mode=resize 拉伸某条边/角
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

onMounted(() => {
  const el = rootRef.value;
  if (!el) return;
  const update = () => {
    const r = el.getBoundingClientRect();
    containerSize.value = { width: r.width, height: r.height };
  };
  update();
  resizeObserver = new ResizeObserver(update);
  resizeObserver.observe(el);
});

onUnmounted(() => {
  resizeObserver?.disconnect();
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
});

// ── 聚焦轨道 + 播放头所在 clip ──
// 焦点挂在轨道上：仅当聚焦的是 ocr_region 轨道时才显示遮罩，
// 选区矩形始终跟随"红色标头当前所在的 clip"，随时间动态变化。

const focusedTrack = computed(() => projectStore.findTrack(timeline.focusedTrackId));

const region = computed<OcrRegionEvent | null>(() => {
  const track = focusedTrack.value;
  if (!track || track.type !== "ocr_region") return null;
  const t = timeline.currentTime;
  const current = track.events.find(
    (e) => e.type === "ocr_region" && e.start <= t && e.end > t
  ) as OcrRegionEvent | undefined;
  if (current) return current;
  // 播放头恰好停在视频结尾时，选中最后一个 ocr_region clip
  const regions = track.events.filter(
    (e) => e.type === "ocr_region"
  ) as OcrRegionEvent[];
  return regions.length > 0 ? regions[regions.length - 1] : null;
});

// ── 视频实际渲染矩形（letterbox 校正）──
// 视频 object-fit:contain，若画面比例与容器不同会出现黑边。
// 用 ffprobe 提供的视频宽高 + 容器实测尺寸，算出画面真实占据的矩形，
// 选区坐标（归一化 0~1）都相对这个矩形换算。

const contentRect = computed(() => {
  const cw = containerSize.value.width;
  const ch = containerSize.value.height;
  if (cw <= 0 || ch <= 0) return { left: 0, top: 0, width: cw, height: ch };
  const meta = projectStore.currentVideoMeta;
  if (!meta || meta.width <= 0 || meta.height <= 0) {
    return { left: 0, top: 0, width: cw, height: ch };
  }
  const scale = Math.min(cw / meta.width, ch / meta.height);
  const w = meta.width * scale;
  const h = meta.height * scale;
  return { left: (cw - w) / 2, top: (ch - h) / 2, width: w, height: h };
});

const contentStyle = computed(() => ({
  left: contentRect.value.left + "px",
  top: contentRect.value.top + "px",
  width: contentRect.value.width + "px",
  height: contentRect.value.height + "px",
}));

// ── 选区矩形 + 四块遮罩的定位（相对 content div 的百分比）──

const MIN = 0.02;

function pct(v: number) {
  return v * 100 + "%";
}

const rectStyle = computed(() => {
  const r = region.value;
  if (!r) return {};
  return {
    left: pct(r.x1),
    top: pct(r.y1),
    width: pct(r.x2 - r.x1),
    height: pct(r.y2 - r.y1),
  };
});

const dims = computed(() => {
  const r = region.value;
  if (!r) return [];
  const { x1, y1, x2, y2 } = r;
  return [
    { key: "top", style: { left: "0%", top: "0%", width: "100%", height: pct(y1) } },
    { key: "bottom", style: { left: "0%", top: pct(y2), width: "100%", height: pct(1 - y2) } },
    { key: "left", style: { left: "0%", top: pct(y1), width: pct(x1), height: pct(y2 - y1) } },
    { key: "right", style: { left: pct(x2), top: pct(y1), width: pct(1 - x2), height: pct(y2 - y1) } },
  ];
});

// ── 8 个拉伸 handle ──

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

// ── 拖拽交互 ──

function startDrag(e: MouseEvent, mode: "move" | "resize", handle = "") {
  const r = region.value;
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
  const r = region.value;
  if (!r) return;

  const dx = (e.clientX - drag.startX) / cr.width;
  const dy = (e.clientY - drag.startY) / cr.height;
  const { x1, y1, x2, y2 } = drag;

  if (drag.mode === "move") {
    // 整体平移，平移后整体夹在 [0,1] 内
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
    // 按 handle 决定移动哪条边
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
</script>

<template>
  <div ref="rootRef" class="region-overlay">
    <div v-if="region" class="region-content" :style="contentStyle">
      <!-- 区域外 4 块 50% 黑遮罩 -->
      <div v-for="d in dims" :key="d.key" class="dim" :style="d.style" />
      <!-- 选区边框 + 8 个拉伸 handle -->
      <div
        class="region-rect"
        :style="rectStyle"
        @mousedown="startDrag($event, 'move')"
      >
        <div
          v-for="h in HANDLES"
          :key="h.name"
          class="handle"
          :style="{ left: h.left, top: h.top, cursor: h.cursor }"
          @mousedown.stop="startDrag($event, 'resize', h.name)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.region-overlay {
  position: absolute;
  inset: 0;
  z-index: 5;
  pointer-events: none;
}

.region-content {
  position: absolute;
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
</style>
