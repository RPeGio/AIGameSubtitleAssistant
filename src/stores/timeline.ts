import { defineStore } from "pinia";
import { ref, computed } from "vue";

export const useTimelineStore = defineStore("timeline", () => {
  const pixelsPerSecond = ref(100);
  const scrollLeft = ref(0);
  const currentTime = ref(0);
  const duration = ref(0);
  const isSeeking = ref(false);

  const totalWidth = computed(() => duration.value * pixelsPerSecond.value);

  function tick(t: number) {
    if (isSeeking.value) return;
    currentTime.value = t;
  }

  function seek(t: number) {
    isSeeking.value = true;
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
    const newPps = Math.max(25, Math.min(800, oldPps * factor));
    if (newPps === oldPps) return;
    const timeAtCursor = (scrollLeft.value + cursorX) / oldPps;
    pixelsPerSecond.value = newPps;
    scrollLeft.value = Math.max(0, timeAtCursor * newPps - cursorX);
  }

  function pan(delta: number) {
    const maxScroll = Math.max(0, totalWidth.value - 100);
    scrollLeft.value = Math.max(0, Math.min(scrollLeft.value + delta, maxScroll));
  }

  function timeAtPixel(px: number): number {
    return (scrollLeft.value + px) / pixelsPerSecond.value;
  }

  return {
    pixelsPerSecond,
    scrollLeft,
    currentTime,
    duration,
    isSeeking,
    totalWidth,
    tick,
    seek,
    seekDone,
    setDuration,
    playheadX,
    zoom,
    pan,
    timeAtPixel,
  };
});
