<script setup lang="ts">
import { computed, onUnmounted } from "vue";
import { useProjectStore } from "../stores/project";

/// OCR 选区覆盖层：把选区的归一化坐标按视频真实渲染矩形（contentRect）
/// 换算成容器像素，画出遮罩 + 选区框 + 8 个缩放手柄。
/// 语料页（source 视频）与校对区（clip 视频）共用同一套几何与拖拽逻辑，
/// 避免两处实现漂移。
const props = withDefaults(
  defineProps<{
    videoKey: "source" | "clip";
    /// useVideoContentRect 算出的视频画面矩形（容器像素）
    contentRect: { left: number; top: number; width: number; height: number };
    /// 当前播放头时间：用于挑选"播放头所在"的选区事件
    time: number;
    /// 是否可拖拽/缩放；false 时只显示
    interactive?: boolean;
  }>(),
  { interactive: true }
);

const projectStore = useProjectStore();

/// 该视频源绑定的 OCR 选区控制轨（可能存在多条选区事件）
const regionTrack = computed(() =>
  projectStore.currentProject?.tracks.find(
    (t) => t.type === "ocr_region" && t.video === props.videoKey
  )
);

/// 当前播放头所在的选区事件；播放头不在任何 clip 内时取最后一个
const regionEvent = computed(() => {
  const track = regionTrack.value;
  if (!track) return null;
  const t = props.time;
  const regions = track.events.filter(
    (e): e is Extract<typeof e, { type: "ocr_region" }> => e.type === "ocr_region"
  );
  const current = regions.find((e) => e.start <= t && e.end > t);
  return current ?? (regions.length > 0 ? regions[regions.length - 1] : null);
});

// ── 选区拖拽 ────────────────────────────────────────────
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

const MIN = 0.02;

/// 选区矩形按视频渲染矩形绝对像素定位：存储的归一化坐标是相对视频画面的，
/// 直接用容器百分比会在 object-fit: contain 的 letterbox 下系统性偏移
/// （语料页 2.39:1 视频尤为明显）
const rectStyle = computed(() => {
  const r = regionEvent.value;
  if (!r) return {};
  const cr = props.contentRect;
  return {
    left: cr.left + r.x1 * cr.width + "px",
    top: cr.top + r.y1 * cr.height + "px",
    width: (r.x2 - r.x1) * cr.width + "px",
    height: (r.y2 - r.y1) * cr.height + "px",
  };
});

/// 画面矩形之外的区域压暗（让选区范围一眼可见）
const dims = computed(() => {
  const r = regionEvent.value;
  if (!r) return [];
  const cr = props.contentRect;
  const bottomTop = cr.top + cr.height;
  const rightLeft = cr.left + cr.width;
  return [
    { key: "top", style: { left: "0px", top: "0px", width: "100%", height: cr.top + "px" } },
    {
      key: "bottom",
      style: { left: "0px", top: bottomTop + "px", width: "100%", height: `calc(100% - ${bottomTop}px)` },
    },
    { key: "left", style: { left: "0px", top: cr.top + "px", width: cr.left + "px", height: cr.height + "px" } },
    {
      key: "right",
      style: { left: rightLeft + "px", top: cr.top + "px", width: `calc(100% - ${rightLeft}px)`, height: cr.height + "px" },
    },
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
  if (!props.interactive) return;
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
  const cr = props.contentRect;
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
  <div v-if="regionEvent" class="region-overlay" :class="{ static: !props.interactive }">
    <div v-for="d in dims" :key="d.key" class="dim" :style="d.style" />
    <div
      class="region-rect"
      :style="rectStyle"
      @mousedown="startDrag($event, 'move')"
    >
      <template v-if="props.interactive">
        <div
          v-for="h in HANDLES"
          :key="h.name"
          class="handle"
          :style="{ left: h.left, top: h.top, cursor: h.cursor }"
          @mousedown.stop="startDrag($event, 'resize', h.name)"
        />
      </template>
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

.dim {
  position: absolute;
  background: rgba(0, 0, 0, 0.5);
  pointer-events: none;
}

.region-rect {
  position: absolute;
  box-sizing: border-box;
  border: 2px solid var(--accent);
  pointer-events: auto;
  cursor: move;
}

.region-overlay.static .region-rect {
  cursor: default;
}

.handle {
  position: absolute;
  width: 10px;
  height: 10px;
  background: var(--accent);
  border: 1px solid #fff;
  border-radius: 2px;
  transform: translate(-50%, -50%);
  pointer-events: auto;
}
</style>
