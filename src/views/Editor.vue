<script setup lang="ts">
import { useProjectStore } from "../stores/project";
import AppSidebar from "../components/AppSidebar.vue";

const projectStore = useProjectStore();
</script>

<template>
  <div class="editor-layout">
    <AppSidebar />
    <main class="editor-main">
      <div class="editor-placeholder">
        <div class="placeholder-icon">🎬</div>
        <h2>{{ projectStore.currentProject?.name ?? "加载中..." }}</h2>
        <p>视频工作台正在开发中...</p>
      </div>
    </main>
  </div>
</template>

<style scoped>
.editor-layout {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

.editor-main {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-bg-primary);
}

.editor-placeholder {
  text-align: center;
  color: var(--color-text-secondary);
}

.placeholder-icon {
  font-size: 64px;
  margin-bottom: 16px;
}
</style>
