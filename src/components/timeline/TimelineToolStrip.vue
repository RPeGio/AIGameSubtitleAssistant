<script setup lang="ts">
import { useTimelineStore } from "../../stores/timeline";

const timeline = useTimelineStore();
</script>

<template>
  <div class="tool-strip">
    <button
      class="tool-btn"
      :class="{ active: timeline.activeTool === 'select' }"
      :title="'选择工具（默认）'"
      @click="timeline.setTool('select')"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="20"
        height="20"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="m3 3 7.07 16.97 2.51-7.39 7.39-2.51L3 3z" />
        <path d="m13 13 6 6" />
      </svg>
    </button>
    <button
      class="tool-btn"
      :class="{ active: timeline.activeTool === 'split' }"
      :title="'分割工具：点击 clip 在光标处分割'"
      @click="timeline.setTool('split')"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="20"
        height="20"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="6" cy="6" r="3"></circle>
        <circle cx="6" cy="18" r="3"></circle>
        <line x1="20" y1="4" x2="8.12" y2="15.88"></line>
        <line x1="14.47" y1="14.48" x2="20" y2="20"></line>
        <line x1="8.12" y1="8.12" x2="12" y2="12"></line>
      </svg>
    </button>
    <button
      class="tool-btn"
      :class="{ active: timeline.activeTool === 'merge' }"
      :title="'合并工具：点选两个相邻 clip 合并；或按住 clip 边缘拖过相邻边界合并'"
      @click="timeline.setTool('merge')"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="20"
        height="20"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <line x1="12" y1="3.5" x2="12" y2="20.5" />
        <rect x="2" y="6" width="6" height="12" rx="1" />
        <rect x="16" y="6" width="6" height="12" rx="1" />
        <polyline points="3.5 10 7.5 12 3.5 14" />
        <polyline points="20.5 10 16.5 12 20.5 14" />
      </svg>
    </button>
    <button
      class="tool-btn"
      :class="{ active: timeline.snapEnabled }"
      :title="timeline.snapEnabled ? '吸附：开启' : '吸附：关闭'"
      @click="timeline.toggleSnap()"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="20"
        height="20"
        viewBox="0 0 24 24"
        fill="currentColor"
        stroke="none"
      >
        <path d="M14,12a2,2,0,0,1-4,0V2H2V12a10,10,0,0,0,20,0V2H14ZM8,4V6H4V4ZM20,4V6H16V4Zm-4,8V8h4v4A8,8,0,0,1,4,12V8H8v4a4,4,0,0,0,8,0Z" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.tool-strip {
  width: 40px;
  flex-shrink: 0;
  border-right: 1px solid var(--color-border);
  background: var(--color-bg-tertiary);
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 4px 0;
  gap: 4px;
}

.tool-btn {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--color-text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.15s;
}

.tool-btn:hover {
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
}

.tool-btn.active {
  background: var(--color-accent);
  color: #fff;
}
</style>
