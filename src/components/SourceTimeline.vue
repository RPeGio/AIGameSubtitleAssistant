<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useProjectStore } from "../stores/project";
import type { OcrRegionEvent } from "../types";

/// 语料页迷你时间轴：
/// - 完全独立于全局 timeline store（不读写它，避免与校对区时间轴串扰）
/// - 只显示剧情录屏（source）的 OCR 选区控制轨，单轨高度
/// - 播放头与 SourceVideoPreview 通过 v-model:time 共享
/// - 完整视口模型：缩放（滚轮）、水平平移（滚轮/滚动条）、按 scrollLeft 换算时间

const props = defineProps<{
  /// 当前播放时间（v-model:time）
  time: number;
  /// 视频总时长
  duration: number;
}>();

const emit = defineEmits<{
  "update:time": [t: number];
}>();

const projectStore = useProjectStore();

const LABEL_WIDTH = 100;
const MIN_PPS = 1;
const MAX_PPS = 800;

// ── 视口状态（本地，独立于全局 timeline store）──────────
const pps = ref(60); // 初始 60px/s（视频较长时挂载后自动 fit）
const scrollLeft = ref(0);
const viewportWidth = ref(0);

const viewportRef = ref<HTMLElement | null>(null);
const rootRef = ref<HTMLElement | null>(null);
let viewportObserver: ResizeObserver | null = null;
/// 用户是否已手动缩放（此后视口尺寸变化不再强制 fit，避免覆盖用户缩放）
let userZoomed = false;

const sourceTrack = computed(() =>
  projectStore.currentProject?.tracks.find(
    (t) => t.type === "ocr_region" && t.video === "source"
  )
);

const regions = computed<OcrRegionEvent[]>(() =>
  (sourceTrack.value?.events ?? []).filter(
    (e): e is OcrRegionEvent => e.type === "ocr_region"
  )
);

const totalWidth = computed(() => props.duration * pps.value);

/// 内容区宽度：内容总宽，至少铺满视口（防止 fit 后内容被裁）
const areaWidth = computed(() => Math.max(totalWidth.value, viewportWidth.value));

function maxScroll(): number {
  return Math.max(0, areaWidth.value - viewportWidth.value);
}

/// 内容区坐标换算（原生滚动：`.viewport` overflow-x:auto 已把 `.area` 平移 scrollLeft，
/// 因此 content 坐标不再手工减 scrollLeft，避免二次位移）。
/// px 传入的是相对 `.area` 左缘（内容原点）的偏移，无需再叠加 scrollLeft。
function timeAtPixel(px: number): number {
  return Math.max(0, Math.min(props.duration, px / pps.value));
}

function clipPos(e: OcrRegionEvent) {
  return {
    left: e.start * pps.value,
    width: Math.max(4, (e.end - e.start) * pps.value),
  };
}

const playheadLeft = computed(() => props.time * pps.value + "px");

function fitView() {
  if (props.duration > 0 && viewportWidth.value > 0) {
    pps.value = Math.max(MIN_PPS, Math.min(MAX_PPS, viewportWidth.value / props.duration));
  }
}

// 视频加载/时长变化时 fit 视口（重置用户缩放状态）
watch(
  () => props.duration,
  () => {
    userZoomed = false;
    fitView();
    scrollLeft.value = 0;
    if (viewportRef.value) viewportRef.value.scrollLeft = 0;
  }
);

// 视口宽度变化时仅当未手动缩放才 fit（用户缩放后尊重其选择）
watch(viewportWidth, () => {
  if (!userZoomed) fitView();
});

function zoom(factor: number, cursorX: number) {
  const oldPps = pps.value;
  const minPps = props.duration > 0 ? viewportWidth.value / props.duration : 1;
  const newPps = Math.max(minPps, Math.min(MAX_PPS, oldPps * factor));
  if (newPps === oldPps) return;
  const timeAtCursor = (scrollLeft.value + cursorX) / oldPps;
  pps.value = newPps;
  scrollLeft.value = Math.max(0, Math.min(timeAtCursor * newPps - cursorX, maxScroll()));
  if (viewportRef.value) viewportRef.value.scrollLeft = scrollLeft.value;
  userZoomed = true;
}

function pan(delta: number) {
  scrollLeft.value = Math.max(0, Math.min(scrollLeft.value + delta, maxScroll()));
  if (viewportRef.value) viewportRef.value.scrollLeft = scrollLeft.value;
}

// ── 滚轮：普通滚轮水平平移；Ctrl+滚轮缩放 ──
function onWheel(e: WheelEvent) {
  e.preventDefault();
  if (e.ctrlKey || e.metaKey) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    zoom(e.deltaY < 0 ? 1.1 : 0.9, e.clientX - rect.left - LABEL_WIDTH);
  } else {
    pan(e.deltaY + e.deltaX);
  }
}

// ── 播放头拖动（ruler/内容区）──
let scrubbing = false;
let scrubBody: HTMLElement | null = null;

function onContentMouseDown(e: MouseEvent) {
  const target = e.currentTarget as HTMLElement;
  // 不 preventDefault：否则浏览器不合成 dblclick，双击添加选区失效。
  // 拖动播放头靠 mousemove 更新，无需阻止默认行为。
  scrubBody = target;
  scrubbing = true;
  emit("update:time", timeAtPixel(e.clientX - target.getBoundingClientRect().left));
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  if (!scrubbing || !scrubBody) return;
  emit(
    "update:time",
    timeAtPixel(e.clientX - scrubBody.getBoundingClientRect().left)
  );
}

function onWindowMouseUp() {
  scrubbing = false;
  scrubBody = null;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
}

// ── clip 拖动：改起止时间（按像素增量换算，与 scrollLeft 无关）──
let clipDrag: null | {
  id: string;
  mode: "move" | "l" | "r";
  startX: number;
  origStart: number;
  origEnd: number;
} = null;
const MIN_DUR = 0.05;

function onClipMouseDown(e: MouseEvent, ev: OcrRegionEvent, mode: "move" | "l" | "r") {
  e.stopPropagation();
  e.preventDefault();
  clipDrag = { id: ev.id, mode, startX: e.clientX, origStart: ev.start, origEnd: ev.end };
  window.addEventListener("mousemove", onClipMove);
  window.addEventListener("mouseup", onClipUp);
}

function onClipMove(e: MouseEvent) {
  if (!clipDrag) return;
  const dt = (e.clientX - clipDrag.startX) / pps.value;
  const { origStart, origEnd } = clipDrag;
  let start = origStart;
  let end = origEnd;
  if (clipDrag.mode === "move") {
    const len = origEnd - origStart;
    start = Math.max(0, Math.min(props.duration - len, origStart + dt));
    end = start + len;
  } else if (clipDrag.mode === "l") {
    start = Math.max(0, Math.min(origEnd - MIN_DUR, origStart + dt));
  } else {
    end = Math.min(props.duration, Math.max(origStart + MIN_DUR, origEnd + dt));
  }
  projectStore.updateEventTime(clipDrag.id, start, end);
}

function onClipUp() {
  clipDrag = null;
  window.removeEventListener("mousemove", onClipMove);
  window.removeEventListener("mouseup", onClipUp);
}

// ── 双击轨道空白添加选区（以点击处为中心 ±1.5s）──
function onDblClick(e: MouseEvent) {
  if (!sourceTrack.value) return;
  const target = e.currentTarget as HTMLElement;
  const t = timeAtPixel(e.clientX - target.getBoundingClientRect().left);
  const start = Math.max(0, t - 1.5);
  const end = Math.min(props.duration, t + 1.5);
  projectStore.addOcrRegionEvent(sourceTrack.value.id, start, end);
  // 播放头定位到点击处（不是 clip 起点，避免视觉上"左跳"）
  emit("update:time", t);
}

// ── 点击 clip：跳转播放头到该选区起点（同步视频）──
function onClipClick(ev: OcrRegionEvent) {
  emit("update:time", ev.start);
}

onMounted(() => {
  if (viewportRef.value) {
    const update = () => {
      viewportWidth.value = viewportRef.value!.getBoundingClientRect().width;
    };
    update();
    viewportObserver = new ResizeObserver(update);
    viewportObserver.observe(viewportRef.value);
    // 原生横向滚动 → 同步到 scrollLeft ref（仅供 zoom/pan 使用，内容坐标不再手工减）
    viewportRef.value.addEventListener("scroll", () => {
      scrollLeft.value = viewportRef.value!.scrollLeft;
    });
  }
  rootRef.value?.addEventListener("wheel", onWheel, { passive: false });
});

onUnmounted(() => {
  viewportObserver?.disconnect();
  rootRef.value?.removeEventListener("wheel", onWheel);
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
  window.removeEventListener("mousemove", onClipMove);
  window.removeEventListener("mouseup", onClipUp);
});
</script>

<template>
  <div ref="rootRef" class="source-timeline">
    <!-- 工具条（横向排版节省空间） -->
    <div class="toolbar">
      <span class="hint">
        双击空白加选区 · 拖动 clip 调起止 · 点击 clip 跳转 · 滚轮平移 · Ctrl+滚轮缩放
      </span>
    </div>

    <!-- 滚动容器：label 固定在外，内容区（.area）自身为滚动体 -->
    <div class="scroll-wrap">
      <div class="label-cell">剧情录屏选区</div>
      <div ref="viewportRef" class="viewport">
        <div
          class="area"
          :style="{ width: areaWidth + 'px' }"
          @mousedown="onContentMouseDown"
          @dblclick="onDblClick"
        >
          <div class="track" :style="{ width: totalWidth + 'px' }">
            <div
              v-for="ev in regions"
              :key="ev.id"
              class="region-clip"
              :class="{ active: props.time >= ev.start && props.time < ev.end }"
              :style="clipPos(ev)"
              @mousedown.stop="onClipMouseDown($event, ev, 'move')"
              @click.stop="onClipClick(ev)"
            >
              <span
                class="resize-handle l"
                @mousedown.stop="onClipMouseDown($event, ev, 'l')"
              />
              <span
                class="resize-handle r"
                @mousedown.stop="onClipMouseDown($event, ev, 'r')"
              />
            </div>
          </div>
          <!-- 播放头（相对内容区左缘，含 label 后的内容起点） -->
          <div class="playhead" :style="{ left: playheadLeft }" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.source-timeline {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  background: var(--color-bg-secondary);
  overflow: hidden;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.hint {
  font-size: 11px;
  color: var(--color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 滚动容器：label 固定在外，viewport 横向滚动内容 */
.scroll-wrap {
  display: flex;
  min-height: 0;
}

.label-cell {
  width: 100px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  padding: 0 8px;
  font-size: 11px;
  color: var(--color-text-secondary);
  background: var(--color-bg-tertiary);
  border-right: 1px solid var(--color-border);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  position: relative;
  z-index: 2;
}

/* 视口：横向滚动容器（只滚内容区） */
.viewport {
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  position: relative;
}

/* 内容区：宽度 = 内容总宽（fit 后至少铺满视口），坐标基准 = 内容左缘 */
.area {
  position: relative;
  height: 48px;
  cursor: crosshair;
  user-select: none;
  background: repeating-linear-gradient(
    90deg,
    transparent 0,
    transparent 59px,
    var(--color-border) 59px,
    var(--color-border) 60px
  );
}

.track {
  position: relative;
  height: 30px;
  top: 18px;
}

.region-clip {
  position: absolute;
  top: 4px;
  height: 22px;
  background: var(--color-accent);
  opacity: 0.55;
  border-radius: 3px;
  cursor: move;
}

.region-clip.active {
  opacity: 0.95;
}

.resize-handle {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 6px;
  cursor: ew-resize;
}

.resize-handle.l {
  left: 0;
}

.resize-handle.r {
  right: 0;
}

.playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--color-error);
  pointer-events: none;
}
</style>
