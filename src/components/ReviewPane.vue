<script setup lang="ts">
import { useProjectStore } from "../stores/project";

const projectStore = useProjectStore();
</script>

<template>
  <aside class="review-pane">
    <div class="review-header">
      <span class="review-title">校对区</span>
      <span class="review-sub">视频预览 · 轨道总览 · 时间轴</span>
    </div>
    <div class="review-body">
      <p class="placeholder">校对区组件将在后续步骤迁移至此（全局常驻，跟随切片视频时间轴）</p>
      <p v-if="projectStore.currentProject?.video" class="video-path">
        {{ projectStore.currentProject.video }}
      </p>
      <p v-else class="placeholder">尚未导入切片视频</p>
    </div>
  </aside>
</template>

<style scoped>
.review-pane {
  width: 340px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: var(--color-bg-secondary);
  border-left: 1px solid var(--color-border);
}

.review-header {
  padding: 14px 16px;
  border-bottom: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.review-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text-primary);
}

.review-sub {
  font-size: 12px;
  color: var(--color-text-secondary);
}

.review-body {
  flex: 1;
  overflow: hidden;
  padding: 16px;
}

.placeholder {
  font-size: 13px;
  color: var(--color-text-secondary);
  line-height: 1.6;
}

.video-path {
  margin-top: 12px;
  font-size: 12px;
  color: var(--color-text-secondary);
  word-break: break-all;
}
</style>
