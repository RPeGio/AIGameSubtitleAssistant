<script setup lang="ts">
import { computed, ref } from "vue";
import type { TimelineEvent } from "../../types";
import { useTimelineStore, SNAP_THRESHOLD_PX } from "../../stores/timeline";
import { useProjectStore } from "../../stores/project";

const timeline = useTimelineStore();
const projectStore = useProjectStore();

const props = defineProps<{
  event: TimelineEvent;
  /// 所在轨道 id（垂直换轨时区分源轨）
  trackId: string;
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
  /// 垂直拖动悬停目标轨道变化（null = 无有效目标）
  "v-drag-target": [trackId: string | null];
}>();

// ── 拖动：整段移动 / 左右边界改起止 / 垂直换轨 ──
const MIN_DUR = 0.05; // 与 splitEvent 的 EPS 一致，避免零长度 sliver
const DRAG_THRESHOLD = 3; // 位移超过该像素才视为拖动（区分点击）
const V_DRAG_THRESHOLD = 12; // 垂直位移超过该像素才进入换轨模式（asr 事件专属）

type DragMode = "move" | "l" | "r";

const drag = ref<{
  mode: DragMode;
  startX: number;
  startY: number;
  origStart: number;
  origEnd: number;
  moved: boolean;
  /// 垂直换轨模式：鼠标已跨过垂直阈值，只换轨道不改时间
  vMode: boolean;
  /// 当前悬停的目标轨道 id（有效目标，非源轨）
  vTarget: string | null;
  /// 拖动中内容区屏幕矩形：鼠标越出可视区边缘时自动滚动
  bodyRect: DOMRect | null;
  /// 候选吸附边界（所有其他事件 start/end 时间秒），拖动开始收集一次（静态）
  snapEdges: number[];
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

// ── 吸附 ──

/// 收集所有轨道所有事件的 start/end 时间 + 自身原位置边界（移走后可吸回原位）：
/// 拖动中候选边界集合静态，首次进入拖动时收集一次
function collectSnapEdges(): number[] {
  const edges: number[] = [];
  const d = drag.value;
  // 自身原边界：clip 移走后仍能吸附回原位
  edges.push(d?.origStart ?? props.event.start, d?.origEnd ?? props.event.end);
  const tracks = projectStore.currentProject?.tracks ?? [];
  for (const track of tracks) {
    for (const e of track.events) {
      if (e.id === props.event.id) continue;
      edges.push(e.start, e.end);
    }
  }
  return edges;
}

/// 边界吸附：拖动关键边与候选边界像素距离 < 阈值时对齐。
/// move 模式 start/end 双边缘都试（取更近命中）；l 只吸 start；r 只吸 end。
/// 返回吸附后的 start/end 与标记线像素位置；无命中返回 null。
function snapToEdges(
  mode: DragMode,
  start: number,
  end: number,
  edges: number[],
  pps: number,
  minStart: number,
  maxEnd: number,
): { start: number; end: number; guidePx: number } | null {
  const dur = end - start;
  let best: { start: number; end: number; guidePx: number } | null = null;
  let bestDist = SNAP_THRESHOLD_PX;
  for (const cand of edges) {
    if (mode === "move" || mode === "l") {
      const dist = Math.abs(start - cand) * pps;
      if (dist < bestDist) {
        const s = cand;
        const e = mode === "move" ? s + dur : end;
        // 同 clamp 语义：不得越过相邻边界、时长不得小于 MIN_DUR
        if (s >= minStart && e <= maxEnd && e - s >= MIN_DUR) {
          best = { start: s, end: e, guidePx: cand * pps };
          bestDist = dist;
        }
      }
    }
    if (mode === "move" || mode === "r") {
      const dist = Math.abs(end - cand) * pps;
      if (dist < bestDist) {
        const e = cand;
        const s = mode === "move" ? e - dur : start;
        if (s >= minStart && e <= maxEnd && e - s >= MIN_DUR) {
          best = { start: s, end: e, guidePx: cand * pps };
          bestDist = dist;
        }
      }
    }
  }
  return best;
}

// ── 原位虚影 ──
// 拖动中在原始位置渲染虚线框（内容区绝对坐标，随滚动移动）；
// mouseup 后 drag=null 自动销毁
const ghostStyle = computed(() => {
  const d = drag.value;
  if (!d) return {};
  const pps = timeline.pixelsPerSecond;
  const dur = d.origEnd - d.origStart;
  return {
    left: d.origStart * pps - timeline.scrollLeft + "px",
    width: Math.max(4, dur * pps) + "px",
  };
});

function onClipMouseDown(e: MouseEvent, mode: DragMode) {
  // 分割工具下不拖动（点击即分割是既有行为，不拦截冒泡）
  if (timeline.activeTool === "split") return;
  e.preventDefault();
  e.stopPropagation();
  const body = (e.currentTarget as HTMLElement).closest(".tl-content") as HTMLElement | null;
  drag.value = {
    mode,
    startX: e.clientX,
    startY: e.clientY,
    origStart: props.event.start,
    origEnd: props.event.end,
    moved: false,
    vMode: false,
    vTarget: null,
    bodyRect: body ? body.getBoundingClientRect() : null,
    snapEdges: [],
  };
  window.addEventListener("mousemove", onWindowMouseMove);
  window.addEventListener("mouseup", onWindowMouseUp);
}

// 垂直换轨：鼠标所在位置悬停的 asr 轨道（非源轨）为目标轨道
function vDragTargetAt(clientX: number, clientY: number): string | null {
  const el = document.elementFromPoint(clientX, clientY);
  const body = el?.closest<HTMLElement>(".tl-track-body[data-track-id]");
  if (!body) return null;
  // 只允许 asr → asr 换轨；高亮与 drop 行为一致，避免误导
  if (body.dataset.trackType !== "asr") return null;
  const trackId = body.dataset.trackId ?? null;
  if (!trackId || trackId === props.trackId) return null;
  return trackId;
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
  const dy = e.clientY - d.startY;
  // 拖动判定阈值：吸附开启（select 工具）时用吸附阈值——位移未到阈值
  // 保持原位且不记快照（避免亚阈值拖动产生空撤销步骤）；
  // 其余情况沿用点击判定阈值 3px
  const dragThreshold =
    timeline.snapEnabled && timeline.activeTool === "select" ? SNAP_THRESHOLD_PX : DRAG_THRESHOLD;
  if (!d.moved && Math.abs(dx) > dragThreshold) {
    d.moved = true;
    // 一次拖动 = 一个撤销步骤（首次超过阈值才记录，点击不产生空步骤；
    // 拖动中的 updateEventTime 不重复记录）
    projectStore.recordSnapshot();
    // 候选吸附边界静态，进入拖动时收集一次
    d.snapEdges = collectSnapEdges();
  }

  // 垂直换轨：asr 事件在 move 模式下垂直位移超过阈值 → 冻结时间，只换轨道
  // 合并工具有自己的拖动语义（resize 过界合并），不进入换轨
  if (d.mode === "move" && props.event.type === "asr" && timeline.activeTool !== "merge") {
    if (!d.vMode && Math.abs(dy) > V_DRAG_THRESHOLD) {
      d.vMode = true;
      timeline.setSnapGuide(null);
    }
    if (d.vMode) {
      const target = vDragTargetAt(e.clientX, e.clientY);
      if (target !== d.vTarget) {
        d.vTarget = target;
        emit("v-drag-target", target);
      }
      return;
    }
  }

  // 位移未达拖动阈值（点击 or 原位吸附区间）→ 不更新事件：
  // 保持原位，且未记快照无需撤销
  if (!d.moved) return;

  const dt = (dx + scrollDelta) / timeline.pixelsPerSecond;

  const { minStart, maxEnd, prev, next } = bounds();
  const dur0 = d.origEnd - d.origStart;
  // 合并工具：resize 边缘拖过相邻边界超过阈值即直接合并相邻 clip
  // （阈值以像素声明，比较前换算为秒：prev.end - MERGE_DRAG_PX / pps）
  if (timeline.activeTool === "merge" && d.moved) {
    const threshold = MERGE_DRAG_PX / timeline.pixelsPerSecond;
    if (d.mode === "l" && prev && d.origStart + dt < prev.end - threshold) {
      if (tryMergeWith(prev)) return;
    } else if (d.mode === "r" && next && d.origEnd + dt > next.start + threshold) {
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

  // 边界吸附：拖动关键边接近其他 clip 的 start/end 时对齐，并显示标记线
  if (d.moved && timeline.snapEnabled && timeline.activeTool === "select" && d.snapEdges.length > 0) {
    const snapped = snapToEdges(
      d.mode,
      start,
      end,
      d.snapEdges,
      timeline.pixelsPerSecond,
      minStart,
      maxEnd,
    );
    if (snapped) {
      start = snapped.start;
      end = snapped.end;
      timeline.setSnapGuide(snapped.guidePx);
    } else {
      timeline.setSnapGuide(null);
    }
  }

  projectStore.updateEventTime(props.event.id, start, end);
}

// 合并工具拖动：合并当前 clip 与相邻 clip 并聚焦合并结果，结束拖动。
// 拖动场景的相邻关系已由 bounds() 保证（最近非重叠邻居），
// 中间可能有重叠事件打断排序相邻，故不要求严格相邻
function tryMergeWith(other: TimelineEvent): boolean {
  const merged = projectStore.mergeTwo(props.event.id, other.id, false);
  if (!merged) return false;
  timeline.focusClip(merged);
  emit("merged");
  onWindowMouseUp();
  return true;
}

function onWindowMouseUp() {
  const d = drag.value;
  drag.value = null;
  // 拖动结束：吸附标记线立即消失
  timeline.setSnapGuide(null);
  window.removeEventListener("mousemove", onWindowMouseMove);
  window.removeEventListener("mouseup", onWindowMouseUp);
  // 垂直换轨：释放时若有有效目标轨道则移动事件，并清除悬停高亮
  if (d?.vMode) {
    emit("v-drag-target", null);
    if (d.vTarget) {
      // 水平未移动（moved=false）时由 store 记录快照；已移动过则复用水平拖动的快照
      projectStore.moveEventToTrack(props.event.id, d.vTarget, !d.moved);
      timeline.focusClip(props.event.id);
      timeline.focusTrack(d.vTarget);
    }
  }
  // 拖动结束后的 click（与 mouseup 同任务派发）不触发聚焦逻辑；
  // 若鼠标已离开 clip（无 click 派发）也复位，不吞掉下一次点击
  // vMode 也需抑制：纯垂直拖动后 click 会落在目标轨道的其他 clip 上
  if (d?.moved || d?.vMode) {
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
    case "fused":
    case "manual":
      return e.text.length > 24 ? e.text.slice(0, 24) + "…" : e.text;
  }
}
</script>

<template>
  <div
    v-if="drag && drag.moved"
    class="clip-ghost"
    :style="{ ...ghostStyle, borderColor: color, color }"
  />
  <div
    class="clip"
    :class="{ focused, dragging: drag !== null, 'v-dragging': drag?.vMode, 'split-tool': timeline.activeTool === 'split' }"
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

/* 原位虚影：拖动中显示在原始位置的虚线框 */
.clip-ghost {
  position: absolute;
  top: 4px;
  height: 32px;
  border: 2px dashed;
  border-radius: 4px;
  box-sizing: border-box;
  background: color-mix(in srgb, currentColor 20%, transparent);
  pointer-events: none;
  z-index: 1;
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

/* 垂直换轨模式：半透明示意"等待放到目标轨道"，并隐藏文字避免遮挡 */
.clip.v-dragging {
  opacity: 0.55;
}

.clip.v-dragging .clip-label {
  opacity: 0;
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
