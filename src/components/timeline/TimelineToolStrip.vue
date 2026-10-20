<script setup lang="ts">
import { computed, inject, onUnmounted, ref, watch } from "vue";
import { NPopover } from "naive-ui";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../../stores/timeline";
import { MERGE_OPTIONS, SPLIT_OPTIONS } from "../../composables/sentenceCuts";
import type { MergeStrategy, SplitStrategy } from "../../composables/sentenceCuts";
import AppIcon from "../AppIcon.vue";

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();

// ── 策略面板（第 8 项）──
// 已选中该工具时再点一次才展开；首次点击只切换工具（避免"想切工具却弹出面板"）
const splitOpen = ref(false);
const mergeOpen = ref(false);

function closePanels() {
  splitOpen.value = false;
  mergeOpen.value = false;
}

function onSplitClick() {
  if (timeline.activeTool === "split") {
    splitOpen.value = !splitOpen.value;
    return;
  }
  closePanels();
  timeline.setTool("split");
}

function onMergeClick() {
  if (timeline.activeTool === "merge") {
    mergeOpen.value = !mergeOpen.value;
    return;
  }
  closePanels();
  timeline.setTool("merge");
}

function pickSplit(v: SplitStrategy) {
  timeline.setSplitStrategy(v);
  splitOpen.value = false;
}

function pickMerge(v: MergeStrategy) {
  timeline.setMergeStrategy(v);
  mergeOpen.value = false;
}

/// manual 触发不会自动收起：点面板与两个工具按钮之外的地方就关掉
function onWindowMouseDown(e: MouseEvent) {
  if (!splitOpen.value && !mergeOpen.value) return;
  const target = e.target as HTMLElement | null;
  if (target?.closest(".strategy-pop")) return;
  if (target?.closest(".tool-split, .tool-merge")) return;
  closePanels();
}
window.addEventListener("mousedown", onWindowMouseDown);
onUnmounted(() => window.removeEventListener("mousedown", onWindowMouseDown));

// 切到别的工具时收起对应的面板
watch(
  () => timeline.activeTool,
  (tool) => {
    if (tool !== "split") splitOpen.value = false;
    if (tool !== "merge") mergeOpen.value = false;
  }
);

const splitLabel = computed(
  () => SPLIT_OPTIONS.find((o) => o.value === timeline.splitStrategy)?.label ?? ""
);
const mergeLabel = computed(
  () => MERGE_OPTIONS.find((o) => o.value === timeline.mergeStrategy)?.label ?? ""
);
</script>

<template>
  <div class="tool-strip">
    <button
      class="tool-btn"
      :class="{ active: timeline.activeTool === 'select' }"
      title="选择工具（默认）"
      @click="timeline.setTool('select')"
    >
      <AppIcon name="cursor" :size="14" />
    </button>

    <NPopover
      trigger="manual"
      placement="bottom-start"
      :show="splitOpen"
      :show-arrow="false"
      :style="{ padding: '4px' }"
    >
      <template #trigger>
        <button
          class="tool-btn has-menu tool-split"
          :class="{ active: timeline.activeTool === 'split' }"
          :title="`分割工具（${splitLabel}）：点击 clip 在光标处分割；再点一次切换策略`"
          @click="onSplitClick"
        >
          <AppIcon name="cut" :size="14" />
        </button>
      </template>
      <div class="strategy-pop">
        <div class="sp-title">分割策略</div>
        <button
          v-for="opt in SPLIT_OPTIONS"
          :key="opt.value"
          class="sp-opt"
          :class="{ on: timeline.splitStrategy === opt.value }"
          @click="pickSplit(opt.value)"
        >
          <AppIcon name="check" :size="11" class="sp-ck" />
          <span class="sp-t">{{ opt.label }}</span>
        </button>
      </div>
    </NPopover>

    <NPopover
      trigger="manual"
      placement="bottom-start"
      :show="mergeOpen"
      :show-arrow="false"
      :style="{ padding: '4px' }"
    >
      <template #trigger>
        <button
          class="tool-btn has-menu tool-merge"
          :class="{ active: timeline.activeTool === 'merge' }"
          :title="`合并工具（${mergeLabel}）：点选两个相邻 clip 合并；再点一次切换策略`"
          @click="onMergeClick"
        >
          <AppIcon name="merge" :size="14" />
        </button>
      </template>
      <div class="strategy-pop">
        <div class="sp-title">合并策略</div>
        <button
          v-for="opt in MERGE_OPTIONS"
          :key="opt.value"
          class="sp-opt"
          :class="{ on: timeline.mergeStrategy === opt.value }"
          @click="pickMerge(opt.value)"
        >
          <AppIcon name="check" :size="11" class="sp-ck" />
          <span class="sp-t">{{ opt.label }}</span>
        </button>
      </div>
    </NPopover>

    <button
      class="tool-btn"
      :class="{ active: timeline.swapSegments }"
      :title="timeline.swapSegments ? '分段互换：开启（点轨道边界按钮交换分段内片段）' : '分段互换：关闭'"
      @click="timeline.toggleSwapSegments()"
    >
      <AppIcon name="swap" :size="14" />
    </button>
    <button
      class="tool-btn"
      :class="{ active: timeline.snapEnabled }"
      :title="timeline.snapEnabled ? '吸附：开启' : '吸附：关闭'"
      @click="timeline.toggleSnap()"
    >
      <AppIcon name="magnet" :size="14" />
    </button>
  </div>
</template>

<style scoped>
/* 工具条：卡片顶部的横条（原为左侧竖排工具列，T7 改为横条以对齐样稿） */
.tool-strip {
  flex: none;
  height: 30px;
  display: flex;
  align-items: center;
  gap: 3px;
  padding: 0 8px;
  border-bottom: 1px solid var(--color-border);
}

.tool-btn {
  width: 24px;
  height: 22px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: none;
  color: var(--dim);
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}

/* 带策略面板的工具（分割/合并）：稍宽，右下角画倒三角 */
.tool-btn.has-menu {
  position: relative;
  width: 26px;
  height: 24px;
}

.tool-btn.has-menu::after {
  content: "";
  position: absolute;
  right: 3px;
  bottom: 3px;
  border-left: 3px solid transparent;
  border-top: 3px solid currentColor;
  opacity: 0.75;
  /* 顺时针转 90°：斜面朝向按钮左上角 */
  transform: rotate(90deg);
}

.tool-btn:hover {
  background: var(--hover);
  color: var(--color-text-primary);
}

.tool-btn.active {
  background: var(--accent-soft);
  border-color: var(--color-accent);
  color: var(--accent-ink);
}

/* 策略列表：与弹层同一套控件词汇，不引额外框架样式 */
.strategy-pop {
  min-width: 132px;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.sp-title {
  padding: 2px 8px 4px;
  font-size: 11px;
  color: var(--faint);
  letter-spacing: 0.06em;
}

.sp-opt {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  padding: 0 8px;
  border: 0;
  border-radius: 5px;
  background: none;
  color: var(--text);
  font-size: 12px;
  cursor: pointer;
  text-align: left;
}

.sp-opt:hover {
  background: var(--hover);
}

.sp-ck {
  opacity: 0;
  color: var(--accent-ink);
}

.sp-opt.on {
  background: var(--accent-soft);
  color: var(--accent-ink);
}

.sp-opt.on .sp-ck {
  opacity: 1;
}

.sp-t {
  white-space: nowrap;
}
</style>
