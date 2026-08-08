<script setup lang="ts">
import { ref } from "vue";
import type { TimelineEvent } from "../../types";
import { useTimelineStore } from "../../stores/timeline";
import { useProjectStore } from "../../stores/project";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

const props = defineProps<{
  event: TimelineEvent;
  /// 同轨全部事件（含自身）：拖动时计算相邻约束
  siblings: TimelineEvent[];
  color: string;
  left: number;
  width: number;
  focused: boolean;
}>();

const emit = defineEmits<{
  "click-clip": [];
  /// 拖动合并完成（供时间轴清空点选合并的第一选择）
  merged: [];
}>();

// ── 拖动：整段移动 / 左右边界改起止 ──
const MIN_DUR = 0.05; // 与 splitEvent 的 EPS 一致，避免零长度 sliver
const DRAG_THRESHOLD = 3; // 位移超过该像素才视为拖动（区分点击）

type DragMode = "move" | "l" | "r";

const drag = ref<{
  mode: DragMode;
  startX: number;
  origStart: number;
  origEnd: number;
  moved: boolean;
  /// 拖动中内容区屏幕矩形：鼠标越出可视区边缘时自动滚动
  bodyRect: DOMRect | null;
} | null>(null);
let suppressClick = false;

// 相邻约束：只考虑与当前事件完全不重叠的最近前驱/后继
// （合并轨道可能存在同时段重叠事件，重叠邻居不参与 clamp）；
// duration 未知（≤0，视频元数据未加载）时不限制右界。
// prev/next 供合并工具拖动过界时作为合并对象
function bounds(): {
  minStart: number;
  maxEnd: number;
  prev: TimelineEvent | null;
  next: TimelineEvent | null;
} {
  const sorted = [...props.siblings].sort((a, b) => a.start - b.start);
  const self = props.event;
  let prev: TimelineEvent | null = null;
  let next: TimelineEvent | null = null;
  for (const e of sorted) {
    if (e.id === self.id) continue;
    if (e.end <= self.start) {
      if (!prev || e.end > prev.end) prev = e;
    } else if (e.start >= self.end) {
      if (!next || e.start < next.start) next = e;
    }
  }
  return {
    minStart: prev ? prev.end : 0,
    maxEnd: next
      ? next.start
      : timeline.duration > 0
        ? timeline.duration
        : Number.MAX_SAFE_INTEGER,
    prev,
    next,
  };
}

// 合并工具：resize 边缘拖过相邻边界该距离（px）即合并
const MERGE_DRAG_PX = 30;

function onClipMouseDown(e: MouseEvent, mode: DragMode) {
  // 分割工具下不拖动（点击即分割是既有行为，不拦截冒泡）
  if (timeline.activeTool === "split") return;
  e.preventDefault();
  e.stopPropagation();
  const body = (e.currentTarget as HTMLElement).closest(".tl-content") as HTMLElement | null;
  drag.value = {
    mode,
    startX: e.clientX,
    origStart: props.event.start,
    origEnd: props.event.end,
    moved: false,
    bodyRect: body ? body.getBoundingClientRect() : null,
  };
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  const d = drag.value;
  if (!d) return;
  // 拖出可视区边缘自动滚动（补偿 scrollLeft 变化，保持鼠标与 clip 相对位置）
  let scrollDelta = 0;
  if (d.bodyRect) {
    const before = timeline.scrollLeft;
    if (e.clientX < d.bodyRect.left) {
      timeline.pan((e.clientX - d.bodyRect.left) * 0.3);
    } else if (e.clientX > d.bodyRect.right) {
      timeline.pan((e.clientX - d.bodyRect.right) * 0.3);
    }
    scrollDelta = timeline.scrollLeft - before;
  }
  const dx = e.clientX - d.startX;
  if (!d.moved && Math.abs(dx) > DRAG_THRESHOLD) d.moved = true;
  const dt = (dx + scrollDelta) / timeline.pixelsPerSecond;
  const { minStart, maxEnd, prev, next } = bounds();
  const dur0 = d.origEnd - d.origStart;
  // 合并工具：resize 边缘拖过相邻边界超过阈值即直接合并相邻 clip
  if (timeline.activeTool === "merge" && d.moved) {
    if (d.mode === "l" && prev && d.origStart + dt < prev.end - MERGE_DRAG_PX) {
      if (tryMergeWith(prev)) return;
    } else if (d.mode === "r" && next && d.origEnd + dt > next.start + MERGE_DRAG_PX) {
      if (tryMergeWith(next)) return;
    }
  }
  let start = d.origStart;
  let end = d.origEnd;
  if (d.mode === "move") {
    start = Math.max(minStart, Math.min(d.origStart + dt, maxEnd - dur0));
    end = start + dur0;
  } else if (d.mode === "l") {
    start = Math.max(minStart, Math.min(d.origStart + dt, d.origEnd - MIN_DUR));
  } else {
    end = Math.max(d.origStart + MIN_DUR, Math.min(d.origEnd + dt, maxEnd));
  }
  projectStore.updateEventTime(props.event.id, start, end);
}

// 合并工具拖动：合并当前 clip 与相邻 clip 并聚焦合并结果，结束拖动
function tryMergeWith(other: TimelineEvent): boolean {
  const merged = projectStore.mergeTwo(props.event.id, other.id);
  if (!merged) return false;
  timeline.focusClip(merged);
  emit("merged");
  onWindowMouseUp();
  return true;
}

function onWindowMouseUp() {
  const d = drag.value;
  drag.value = null;
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
  // 拖动结束后的 click（与 mouseup 同任务派发）不触发聚焦逻辑；
  // 若鼠标已离开 clip（无 click 派发）也复位，不吞掉下一次点击
  if (d?.moved) {
    suppressClick = true;
    window.setTimeout(() => (suppressClick = false), 0);
  }
}

function onClick() {
  if (suppressClick) {
    suppressClick = false;
    return;
  }
  emit("click-clip");
}

function clipText(): string {
  const e = props.event;
  switch (e.type) {
    case "ocr_region":
      return `(${e.x1.toFixed(2)}, ${e.y1.toFixed(2)}) → (${e.x2.toFixed(2)}, ${e.y2.toFixed(2)})`;
    case "ocr_text":
    case "asr":
    case "manual":
      return e.text.length > 24 ? e.text.slice(0, 24) + "…" : e.text;
  }
}
</script>

<template>
  <div
    class="clip"
    :class="{ focused, dragging: drag !== null, 'split-tool': timeline.activeTool === 'split' }"
    :data-event-id="event.id"
    :style="{
      left: left + 'px',
      width: width + 'px',
      backgroundColor: color,
      borderColor: focused ? '#fff' : color,
    }"
    @mousedown="onClipMouseDown($event, 'move')"
    @click.stop="onClick"
  >
    <span
      v-if="timeline.activeTool !== 'split'"
      class="resize-handle resize-l"
      @mousedown="onClipMouseDown($event, 'l')"
    />
    <span
      v-if="timeline.activeTool !== 'split'"
      class="resize-handle resize-r"
      @mousedown="onClipMouseDown($event, 'r')"
    />
    <span class="clip-label">{{ clipText() }}</span>
  </div>
</template>

<style scoped>
.clip {
  position: absolute;
  top: 4px;
  height: 32px;
  border-radius: 4px;
  border: 2px solid;
  box-sizing: border-box;
  overflow: hidden;
  cursor: pointer;
  transition: box-shadow 0.1s;
}

.clip.focused {
  z-index: 5;
  box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.5);
}

.clip:hover {
  opacity: 0.9;
}

.clip.split-tool {
  cursor: crosshair;
}

.clip.dragging {
  cursor: grabbing;
}

/* 左右边界手柄：拖动改起止时间 */
.resize-handle {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 6px;
  cursor: ew-resize;
  z-index: 2;
}

.resize-l {
  left: 0;
}

.resize-r {
  right: 0;
}

.resize-handle:hover {
  background: rgba(255, 255, 255, 0.35);
}

.clip-label {
  display: block;
  padding: 0 6px;
  font-size: 11px;
  line-height: 28px;
  color: #fff;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
