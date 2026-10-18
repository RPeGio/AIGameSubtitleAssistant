<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { TIMELINE_STORE_KEY, useTimelineStore } from "../../stores/timeline";

/// 时间轴音频波形行：一个"伪轨道行"（标签列 + 内容列），挂在 Timeline 轨道列表最上方。
/// 数据来自后端 get_audio_peaks（每秒 50 窗的最大振幅，0..1 线性），
/// Canvas 只画视口内可见区间（virtual render），随 scrollLeft/pps/duration 重绘。
/// 无音轨 / 加载失败 → 整行隐藏（visible 为 false，不占轨道空间）。
///
/// 注意：本组件的样式必须自包含。父组件 Timeline.vue 的 <style scoped> 只作用于
/// 其自身模板渲染的元素（子组件仅根元素带父级 scope id），内层元素拿不到
/// .tl-content / .tl-label-col 的 flex:1、position:relative 等关键布局，
/// 会导致内容区宽高塌成 0、绝对定位的 canvas 脱离本行。

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();

const props = defineProps<{
  /// 当前项目视频绝对路径；空串/undefined 表示无视频
  videoPath?: string;
  /// 标签列宽度：与 Timeline 传入的 labelWidth 一致（mini 时间轴更窄）
  labelWidth?: number;
}>();

/// 波形数据缓存（模块级）：同一视频在页面间切换/重挂载时不重复解码音频
const waveformCache = new Map<string, Float32Array>();

const waveData = ref<Float32Array | null>(null);
const failed = ref(false);

// ── 数据加载 ──

async function loadWaveform(path: string) {
  if (!path) return;
  const cached = waveformCache.get(path);
  if (cached) {
    waveData.value = cached;
    return;
  }
  try {
    const windows = await invoke<number[]>("get_audio_peaks", { videoPath: path });
    const data = Float32Array.from(windows);
    waveformCache.set(path, data);
    // await 期间视频可能又切走：仅当还是当前路径才生效
    if (props.videoPath === path) waveData.value = data;
  } catch {
    // 无音轨 / 解码失败：静默降级，整行隐藏
    if (props.videoPath === path) {
      failed.value = true;
      waveData.value = null;
    }
  }
}

watch(
  () => props.videoPath,
  (p) => {
    if (!p) {
      waveData.value = null;
      return;
    }
    // 换视频即可能成功也可能失败，复位失败标记
    failed.value = false;
    loadWaveform(p);
  },
  { immediate: true }
);

/// 整行是否渲染：有数据才显示；加载中/失败/无视频都不占轨道空间
const visible = computed(() => waveData.value !== null && !failed.value);

// ── 视口裁剪渲染 ──

const canvasRef = ref<HTMLCanvasElement | null>(null);
const bodyRef = ref<HTMLElement | null>(null);
let bodyObserver: ResizeObserver | null = null;
let rafId = 0;

/// 合并同一帧内的多次重绘请求（滚动/缩放会高频触发）
function scheduleDraw() {
  if (rafId) return;
  rafId = requestAnimationFrame(() => {
    rafId = 0;
    draw();
  });
}

function draw() {
  const data = waveData.value;
  const canvas = canvasRef.value;
  const body = bodyRef.value;
  if (!data || !canvas || !body) return;
  const cssW = body.clientWidth;
  const cssH = body.clientHeight;
  if (cssW <= 0 || cssH <= 0) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const pxW = Math.round(cssW * dpr);
  const pxH = Math.round(cssH * dpr);
  // 尺寸变化时重设位图（同时会清空画布）
  if (canvas.width !== pxW || canvas.height !== pxH) {
    canvas.width = pxW;
    canvas.height = pxH;
  }
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.clearRect(0, 0, pxW, pxH);

  const pps = timeline.pixelsPerSecond;
  if (pps <= 0) return;
  const scrollLeft = timeline.scrollLeft;
  // pxW / pxH 已是设备像素，中点与振幅高度直接用它们，勿再乘 dpr
  const mid = pxH / 2;
  const halfH = pxH / 2 - 1;
  ctx.fillStyle = "rgba(9, 132, 227, 0.55)"; // 与 asr clip 同色系、降透明度，不抢 clip 视觉
  // 逐像素列绘制：该列覆盖到的所有窗取最大值
  // （放大时一窗画多列，缩小时多窗并成一列）
  for (let col = 0; col < pxW; col++) {
    const t0 = (scrollLeft + col / dpr) / pps;
    const t1 = (scrollLeft + (col + 1) / dpr) / pps;
    let w0 = Math.floor(t0 * 50);
    let w1 = Math.ceil(t1 * 50);
    if (w0 < 0) w0 = 0;
    if (w1 > data.length) w1 = data.length;
    if (w1 <= w0) continue;
    let peak = 0;
    for (let w = w0; w < w1; w++) {
      if (data[w] > peak) peak = data[w];
    }
    if (peak <= 0) continue;
    const h = peak * halfH;
    ctx.fillRect(col, mid - h, 1, h * 2);
  }
}

/// 内容区尺寸测量（视口宽高）。行是 v-if="visible" 的，数据异步到位时才首次
/// 渲染 DOM，故 onMounted 时 bodyRef 必为 null，需在 visible 翻转后再挂 observer。
function attachObserver() {
  if (bodyObserver || !bodyRef.value) return;
  bodyObserver = new ResizeObserver(scheduleDraw);
  bodyObserver.observe(bodyRef.value);
}

onMounted(() => attachObserver());

onUnmounted(() => {
  bodyObserver?.disconnect();
  if (rafId) cancelAnimationFrame(rafId);
});

// visible 翻转（数据首次就绪）：等 DOM 渲染完，挂 observer 并立即同步画首帧
watch(visible, (v) => {
  if (!v) return;
  nextTick(() => {
    attachObserver();
    draw();
  });
});

// 影响可见区间的状态变化 → 重绘
watch(
  () => [timeline.pixelsPerSecond, timeline.scrollLeft, timeline.viewportWidth],
  scheduleDraw
);
</script>

<template>
  <div v-if="visible" class="wf-row">
    <div
      class="wf-label"
      :style="{ minWidth: (labelWidth ?? 220) + 'px', maxWidth: (labelWidth ?? 220) + 'px' }"
    >
      <div class="wf-label-name">音频波形</div>
      <div class="wf-label-type">amplitude</div>
    </div>
    <div ref="bodyRef" class="wf-body">
      <canvas ref="canvasRef" class="wf-canvas" />
    </div>
  </div>
</template>

<style scoped>
/* 全部样式自包含：不复用父组件的 scoped 类（见文件头注释） */
.wf-row {
  display: flex;
  align-items: stretch;
  min-height: 0;
  height: 48px;
  flex-shrink: 0;
  border-top: 1px solid var(--color-border);
  background: color-mix(in srgb, var(--color-bg-tertiary) 40%, transparent);
}

/* 标签列：宽度由 inline style 传入（与轨道行保持一致） */
.wf-label {
  flex-shrink: 0;
  box-sizing: border-box;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-right: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  cursor: default;
}

.wf-label-name {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  font-weight: 600;
  color: var(--color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.wf-label-type {
  flex: none;
  margin-left: auto;
  font-family: var(--mono);
  font-size: 9.5px;
  line-height: 1;
  color: var(--faint);
  border: 1px solid var(--color-border);
  border-radius: 3px;
  padding: 2px 3px;
}

/* 内容区：flex:1 撑满剩余宽度 + relative 作为 canvas 的定位基准 */
.wf-body {
  flex: 1;
  min-width: 0;
  min-height: 48px;
  height: 48px;
  position: relative;
  overflow: hidden;
}

.wf-canvas {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
</style>
