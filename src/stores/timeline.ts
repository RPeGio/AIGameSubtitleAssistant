import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { InjectionKey } from "vue";
import type { TimelineEvent } from "../types";

export const CLIP_COLORS: Record<string, string> = {
  ocr_region: "#00b894",
  ocr_text: "#fdcb6e",
  asr: "#0984e3",
  fused: "#00cec9",
  manual: "#a29bfe",
};

/// 参与字幕预览/文字编辑的轨道类型（眼睛开关与预览渲染共用，防两处漂移）
export const TEXT_TRACK_TYPES = ["ocr_text", "asr", "fused", "manual"];

/// 统一吸附阈值（像素）：clip 边缘与吸附点距离小于该值即对齐
export const SNAP_THRESHOLD_PX = 8;

export const useTimelineStore = defineStore("timeline", () => {
  const pixelsPerSecond = ref(100);
  const scrollLeft = ref(0);
  const currentTime = ref(0);
  const duration = ref(0);
  const isSeeking = ref(false);
  const focusedClipId = ref<string | null>(null);
  const focusedTrackId = ref<string | null>(null);
  const activeTool = ref<"select" | "split" | "merge">("select");
  const viewportWidth = ref(0);
  /// 吸附开关（默认开启）
  const snapEnabled = ref(true);
  /// 分段垂直互换模式（默认关闭）：开启后时间轴显示分段遮罩与
  /// 轨道边界互换按钮，用于手动修正跨段说话人归属
  const swapSegments = ref(false);
  /// 吸附标记线位置（内容区像素坐标；null = 不显示）
  const snapGuideX = ref<number | null>(null);

  const totalWidth = computed(() => duration.value * pixelsPerSecond.value);

  /// 时间轴内容区域的可见宽度（由 Timeline 组件测量后写入）
  function setViewportWidth(w: number) {
    viewportWidth.value = w;
  }

  /// 内容区可滚动的最大偏移：保证右边缘恰好显示视频结尾
  function maxScroll(): number {
    return Math.max(0, totalWidth.value - viewportWidth.value);
  }

  function tick(t: number) {
    if (isSeeking.value) return;
    currentTime.value = t;
  }

  function scrubbing(value: boolean) {
    isSeeking.value = value;
  }

  function seek(t: number) {
    scrubbing(true);
    currentTime.value = t;
  }

  function seekDone() {
    isSeeking.value = false;
  }

  function setDuration(d: number) {
    duration.value = d;
  }

  function playheadX(): number {
    return currentTime.value * pixelsPerSecond.value - scrollLeft.value;
  }

  function zoom(factor: number, cursorX: number) {
    const oldPps = pixelsPerSecond.value;
    // 最小 pps 允许缩到整段视频刚好铺满可视区（即缩略块=100% 宽）
    const minPps = duration.value > 0 ? viewportWidth.value / duration.value : 1;
    const newPps = Math.max(minPps, Math.min(800, oldPps * factor));
    if (newPps === oldPps) return;
    const timeAtCursor = (scrollLeft.value + cursorX) / oldPps;
    pixelsPerSecond.value = newPps;
    scrollLeft.value = Math.max(0, Math.min(timeAtCursor * newPps - cursorX, maxScroll()));
  }

  function pan(delta: number) {
    scrollLeft.value = Math.max(0, Math.min(scrollLeft.value + delta, maxScroll()));
  }

  function timeAtPixel(px: number): number {
    const t = (scrollLeft.value + px) / pixelsPerSecond.value;
    return Math.max(0, Math.min(duration.value, t));
  }

  function focusClip(id: string | null) {
    focusedClipId.value = id;
  }

  /// 聚焦某条轨道（决定 RegionOverlay 遮罩是否显示）
  function focusTrack(id: string | null) {
    focusedTrackId.value = id;
  }

  /// 切换时间轴工具：select 选择/擦动 | split 分割 | merge 合并
  function setTool(tool: "select" | "split" | "merge") {
    activeTool.value = tool;
  }

  function toggleSnap() {
    snapEnabled.value = !snapEnabled.value;
    snapGuideX.value = null;
  }

  /// 切换分段互换模式
  function toggleSwapSegments() {
    swapSegments.value = !swapSegments.value;
  }

  /// 设置吸附标记线位置（内容区像素坐标）；null 清除
  function setSnapGuide(x: number | null) {
    snapGuideX.value = x;
  }

  function clipPosition(event: TimelineEvent) {
    const pps = pixelsPerSecond.value;
    return {
      left: event.start * pps - scrollLeft.value,
      width: Math.max(4, (event.end - event.start) * pps),
    };
  }

  /// 跳转播放头到指定时间，并滚动时间轴确保其位于可视区内
  /// （scrollLeft 钳制在 [0, maxScroll()]，播放头距视口边缘保留 24px 余量）
  function jumpTo(t: number) {
    const MARGIN = 24;
    const x = t * pixelsPerSecond.value;
    if (x < scrollLeft.value + MARGIN) {
      scrollLeft.value = Math.max(0, x - MARGIN);
    } else if (x > scrollLeft.value + viewportWidth.value - MARGIN) {
      scrollLeft.value = Math.min(x - viewportWidth.value + MARGIN, maxScroll());
    }
    currentTime.value = t;
  }

  return {
    pixelsPerSecond,
    scrollLeft,
    currentTime,
    duration,
    isSeeking,
    focusedClipId,
    focusedTrackId,
    activeTool,
    viewportWidth,
    totalWidth,
    snapEnabled,
    snapGuideX,
    swapSegments,
    tick,
    scrubbing,
    seek,
    seekDone,
    setDuration,
    setViewportWidth,
    maxScroll,
    playheadX,
    zoom,
    pan,
    timeAtPixel,
    focusClip,
    focusTrack,
    setTool,
    toggleSnap,
    toggleSwapSegments,
    setSnapGuide,
    clipPosition,
    jumpTo,
  };
});

export type TimelineStore = ReturnType<typeof useTimelineStore>;

/// 注入 key：允许页面级独立时间轴（如语料页迷你时间轴）注入自己的 store 实例，
/// 子组件优先读取注入的实例，未注入时回退全局 store（保持校对区行为不变）。
export const TIMELINE_STORE_KEY: InjectionKey<TimelineStore> = Symbol("timeline-store");
