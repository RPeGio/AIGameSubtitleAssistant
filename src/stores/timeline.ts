import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { InjectionKey } from "vue";
import type { TimelineEvent } from "../types";
import type { MergeStrategy, SplitStrategy } from "../composables/sentenceCuts";

/// 轨道片段配色：bg 底 / fg 文字 / line 描边，三值都指向主题令牌
export interface ClipTone {
  bg: string;
  fg: string;
  line: string;
}

/// 未知事件类型的中性回落配色
const FALLBACK_TONE: ClipTone = {
  bg: "var(--hover)",
  fg: "var(--dim)",
  line: "var(--hair-2)",
};

export const CLIP_COLORS: Record<string, ClipTone> = {
  ocr_region: { bg: "var(--reg-bg)", fg: "var(--reg-fg)", line: "var(--reg-line)" },
  ocr_text: { bg: "var(--otx-bg)", fg: "var(--otx-fg)", line: "var(--otx-line)" },
  embed_ocr: { bg: "var(--ocr-bg)", fg: "var(--ocr-fg)", line: "var(--ocr-line)" },
  asr: { bg: "var(--asr-bg)", fg: "var(--asr-fg)", line: "var(--asr-line)" },
  fused: { bg: "var(--fus-bg)", fg: "var(--fus-fg)", line: "var(--fus-line)" },
  manual: { bg: "var(--man-bg)", fg: "var(--man-fg)", line: "var(--man-line)" },
};

/// 取某事件类型的片段配色（未知类型回落中性灰）
export function clipTone(type: string): ClipTone {
  return CLIP_COLORS[type] ?? FALLBACK_TONE;
}

/// 参与字幕预览/文字编辑的轨道类型（眼睛开关与预览渲染共用，防两处漂移）
export const TEXT_TRACK_TYPES = ["ocr_text", "embed_ocr", "asr", "fused", "manual"];

/// 统一吸附阈值（像素）：clip 边缘与吸附点距离小于该值即对齐
export const SNAP_THRESHOLD_PX = 8;

/// 分割/合并策略的持久化键与合法值（第 8 项）；不合法或缺失时回落默认档
const SPLIT_KEY = "gsa.timeline.splitStrategy";
const MERGE_KEY = "gsa.timeline.mergeStrategy";
const SPLIT_VALUES: SplitStrategy[] = ["punct", "space", "raw"];
const MERGE_VALUES: MergeStrategy[] = ["latter", "former", "stack"];

function readStrategy<T extends string>(key: string, allowed: T[], fallback: T): T {
  const saved = localStorage.getItem(key);
  return saved && (allowed as string[]).includes(saved) ? (saved as T) : fallback;
}

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
  /// 当前持有播放权的播放器 id（"review" 校对区 / "clip" 切片预览 / "source" 剧情录屏预览）。
  /// 打轴页与校对区共用同一条时间轴时间，若两侧 <video> 同时播放会双声道、双解码，
  /// 故约定：谁按下播放谁持有播放权，另一侧只跟随时间、保持静音暂停。
  const activePlayer = ref<string | null>(null);
  /// 分割策略（面板可切，持久化）：punct 按断句标点 | space 按空格 | raw 保留原文
  const splitStrategy = ref<SplitStrategy>(readStrategy(SPLIT_KEY, SPLIT_VALUES, "punct"));
  /// 合并策略：latter 保留后者文本 | former 保留前者文本 | stack 前后堆叠（现状）
  const mergeStrategy = ref<MergeStrategy>(readStrategy(MERGE_KEY, MERGE_VALUES, "latter"));

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

  /// 拖动擦洗开始：期间由鼠标位置驱动时间，视频的 tick 不得回写
  function beginScrub() {
    isSeeking.value = true;
  }

  /// 拖动擦洗结束：恢复「视频驱动时间」
  function endScrub() {
    isSeeking.value = false;
  }

  /// 瞬时跳转（点 clip、双击总览条目等）：只写时间，不进入擦洗态。
  /// 若这里置 isSeeking，视频的 tick 会被挡掉 ⇒ 该时间轴的播放头永久停住。
  function setTime(t: number) {
    currentTime.value = t;
  }

  function setDuration(d: number) {
    duration.value = d;
  }

  /// 抢占播放权：调用方开始播放时自报身份，其他播放器监听到后自行暂停（只跟随时间）
  function claimPlayback(id: string) {
    activePlayer.value = id;
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

  /// 聚焦某条轨道（决定选区遮罩是否显示）
  function focusTrack(id: string | null) {
    focusedTrackId.value = id;
  }

  /// 切换时间轴工具：select 选择/擦动 | split 分割 | merge 合并
  function setTool(tool: "select" | "split" | "merge") {
    activeTool.value = tool;
  }

  /// 切换分割策略（面板选择后持久化）
  function setSplitStrategy(next: SplitStrategy) {
    splitStrategy.value = next;
    localStorage.setItem(SPLIT_KEY, next);
  }

  /// 切换合并策略（面板选择后持久化）
  function setMergeStrategy(next: MergeStrategy) {
    mergeStrategy.value = next;
    localStorage.setItem(MERGE_KEY, next);
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
    activePlayer,
    splitStrategy,
    mergeStrategy,
    tick,
    beginScrub,
    endScrub,
    setTime,
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
    setSplitStrategy,
    setMergeStrategy,
    toggleSnap,
    toggleSwapSegments,
    setSnapGuide,
    clipPosition,
    jumpTo,
    claimPlayback,
  };
});

export type TimelineStore = ReturnType<typeof useTimelineStore>;

/// 注入 key：允许页面级独立时间轴（如语料页迷你时间轴）注入自己的 store 实例，
/// 子组件优先读取注入的实例，未注入时回退全局 store（保持校对区行为不变）。
export const TIMELINE_STORE_KEY: InjectionKey<TimelineStore> = Symbol("timeline-store");
