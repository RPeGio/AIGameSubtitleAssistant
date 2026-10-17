<script setup lang="ts">
import { inject } from "vue";
import { useTimelineStore, TIMELINE_STORE_KEY } from "../../stores/timeline";
import AppIcon from "../AppIcon.vue";

const timeline = inject(TIMELINE_STORE_KEY, null) ?? useTimelineStore();
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
    <button
      class="tool-btn"
      :class="{ active: timeline.activeTool === 'split' }"
      title="分割工具：点击 clip 在光标处分割"
      @click="timeline.setTool('split')"
    >
      <AppIcon name="cut" :size="14" />
    </button>
    <button
      class="tool-btn"
      :class="{ active: timeline.activeTool === 'merge' }"
      title="合并工具：点选两个相邻 clip 合并；或按住 clip 边缘拖过相邻边界合并"
      @click="timeline.setTool('merge')"
    >
      <AppIcon name="merge" :size="14" />
    </button>
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

.tool-btn:hover {
  background: var(--hover);
  color: var(--color-text-primary);
}

.tool-btn.active {
  background: var(--accent-soft);
  border-color: var(--color-accent);
  color: var(--accent-ink);
}
</style>
