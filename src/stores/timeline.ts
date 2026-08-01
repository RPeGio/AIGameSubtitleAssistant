import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { TimelineEvent } from "../types";

export const CLIP_COLORS: Record<string, string> = {
  ocr_region: "#00b894",
  ocr_text: "#fdcb6e",
  asr: "#0984e3",
  manual: "#a29bfe",
};

export const useTimelineStore = defineStore("timeline", () => {
  const pixelsPerSecond = ref(100);
  const scrollLeft = ref(0);
  const currentTime = ref(0);
  const duration = ref(0);
  const isSeeking = ref(false);
  const focusedClipId = ref<string | null>(null);
  const viewportWidth = ref(0);

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

  function clipPosition(event: TimelineEvent) {
    const pps = pixelsPerSecond.value;
    return {
      left: event.start * pps - scrollLeft.value,
      width: Math.max(4, (event.end - event.start) * pps),
    };
  }

  return {
    pixelsPerSecond,
    scrollLeft,
    currentTime,
    duration,
    isSeeking,
    focusedClipId,
    viewportWidth,
    totalWidth,
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
    clipPosition,
  };
});
