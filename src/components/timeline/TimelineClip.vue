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

const emit = defineEmits<{ "click-clip": [] }>();

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
} | null>(null);
let suppressClick = false;

// 相邻约束：同轨按 start 排序后，前一个事件的 end / 后一个事件的 start
function bounds() {
  const sorted = [...props.siblings].sort((a, b) => a.start - b.start);
  const idx = sorted.findIndex((e) => e.id === props.event.id);
  const prev = idx > 0 ? sorted[idx - 1] : null;
  const next = idx >= 0 && idx < sorted.length - 1 ? sorted[idx + 1] : null;
  return {
    minStart: prev ? prev.end : 0,
    maxEnd: next ? next.start : timeline.duration,
  };
}

function onClipMouseDown(e: MouseEvent, mode: DragMode) {
  // 分割工具下不拖动（点击即分割是既有行为，不拦截冒泡）
  if (timeline.activeTool === "split") return;
  e.preventDefault();
  e.stopPropagation();
  drag.value = {
    mode,
    startX: e.clientX,
    origStart: props.event.start,
    origEnd: props.event.end,
    moved: false,
  };
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

function onWindowMouseMove(e: MouseEvent) {
  const d = drag.value;
  if (!d) return;
  const dx = e.clientX - d.startX;
  if (!d.moved && Math.abs(dx) > DRAG_THRESHOLD) d.moved = true;
  const dt = dx / timeline.pixelsPerSecond;
  const { minStart, maxEnd } = bounds();
  const dur0 = d.origEnd - d.origStart;
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
