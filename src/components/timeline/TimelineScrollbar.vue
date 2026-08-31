<script setup lang="ts">
import { computed, inject, onMounted, onUnmounted, ref } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../../stores/timeline";

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();
const trackRef = ref<HTMLElement | null>(null);

const trackWidth = ref(0);
let resizeObserver: ResizeObserver | null = null;

// 可视宽度（即滑条轨道宽度）随窗口变化自动测量
onMounted(() => {
  const el = trackRef.value;
  if (!el) return;
  const update = () => {
    trackWidth.value = el.getBoundingClientRect().width;
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

// ── 缩略块位置换算 ──
// 内容总宽 = timeline.totalWidth(px)；可视宽 = trackWidth(px)
// 缩略块宽度 = 可视宽占总内容宽的比例，映射到轨道像素
// 缩略块左边缘(轨道px) = scrollLeft / totalWidth * trackWidth

const thumbWidthPx = computed(() => {
  const totalW = timeline.totalWidth;
  if (totalW <= 0 || trackWidth.value <= 0) return 0;
  return Math.max(20, Math.min(trackWidth.value, (trackWidth.value / totalW) * trackWidth.value));
});

const thumbLeftPx = computed(() => {
  const totalW = timeline.totalWidth;
  if (totalW <= 0 || trackWidth.value <= 0) return 0;
  return (timeline.scrollLeft / totalW) * trackWidth.value;
});

const container = computed(() => {
  const tw = trackWidth.value;
  if (tw <= 0) return { width: "100%", left: "0%" };
  return {
    width: (thumbWidthPx.value / tw) * 100 + "%",
    left: (thumbLeftPx.value / tw) * 100 + "%",
  };
});

// 由"缩略块左边缘的轨道像素坐标"反推 scrollLeft，并夹在边界内
function scrollLeftFromThumbLeft(thumbLeftTrackPx: number): number {
  const totalW = timeline.totalWidth;
  const tw = trackWidth.value;
  if (totalW <= 0 || tw <= 0) return 0;
  const maxScroll = Math.max(0, totalW - tw);
  const raw = (thumbLeftTrackPx / tw) * totalW;
  return Math.max(0, Math.min(maxScroll, raw));
}

// ── 拖拽状态 ──
let dragging = false;
let dragOffsetPx = 0; // 鼠标X与缩略块左边缘之间的固定偏移(轨道px)

function onScrollbarMouseDown(e: MouseEvent) {
  const el = trackRef.value;
  if (!el || timeline.totalWidth <= 0 || trackWidth.value <= 0) return;
  e.preventDefault();

  const rect = el.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const tw = thumbWidthPx.value;
  const tl = thumbLeftPx.value;

  if (x >= tl && x <= tl + tw) {
    // 按下的是缩略块本体：不瞬移，记录偏移，拖动跟随
    dragOffsetPx = x - tl;
  } else {
    // 按下的是缩略块外的轨道区域：缩略块中心瞬移至鼠标位置
    const newScrollLeft = scrollLeftFromThumbLeft(x - tw / 2);
    timeline.scrollLeft = newScrollLeft;
    dragOffsetPx = tw / 2;
  }

  dragging = true;
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!dragging || !trackRef.value) return;
  const rect = trackRef.value.getBoundingClientRect();
  const x = e.clientX - rect.left;
  timeline.scrollLeft = scrollLeftFromThumbLeft(x - dragOffsetPx);
}

function onWindowMouseUp() {
  dragging = false;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

function onWheelScrollbar(e: WheelEvent) {
  e.preventDefault();
  const factor = e.deltaY < 0 ? 1.1 : 0.9;
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  timeline.zoom(factor, e.clientX - rect.left);
}
</script>

<template>
  <div
    ref="trackRef"
    class="scrollbar-track"
    @mousedown="onScrollbarMouseDown"
    @wheel.prevent="onWheelScrollbar"
  >
    <div
      class="scrollbar-thumb"
      :style="{ width: container.width, left: container.left }"
    />
  </div>
</template>

<style scoped>
.scrollbar-track {
  flex: 1;
  height: 14px;
  background: var(--color-bg-tertiary);
  position: relative;
  cursor: pointer;
}

.scrollbar-thumb {
  position: absolute;
  top: 2px;
  height: 10px;
  background: var(--color-accent);
  opacity: 0.6;
  border-radius: 5px;
  min-width: 20px;
  pointer-events: none;
  transition: opacity 0.1s;
}

.scrollbar-track:hover .scrollbar-thumb {
  opacity: 1;
}
</style>
