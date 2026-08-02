<script setup lang="ts">
import { useRouter } from "vue-router";
import { useProjectStore } from "../stores/project";
import { NButton } from "naive-ui";
import SaveStatusIndicator from "./SaveStatusIndicator.vue";

const router = useRouter();
const projectStore = useProjectStore();

function handleBack() {
  projectStore.closeProject();
  router.push("/");
}
</script>

<template>
  <aside class="sidebar">
    <div class="sidebar-header">
      <span class="sidebar-logo">GSA</span>
      <span class="sidebar-title">GameSubtitleAI</span>
    </div>

    <div class="sidebar-nav">
      <div class="nav-item active">
        <span class="nav-icon">📹</span>
        <span class="nav-label">视频工作台</span>
      </div>
      <div class="nav-item">
        <span class="nav-icon">📝</span>
        <span class="nav-label">字幕编辑</span>
      </div>
      <div class="nav-item">
        <span class="nav-icon">⚙️</span>
        <span class="nav-label">项目设置</span>
      </div>
    </div>

    <div class="sidebar-footer">
      <NButton size="small" quaternary @click="handleBack">
        ← 返回
      </NButton>
      <SaveStatusIndicator />
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: 220px;
  background: var(--color-bg-secondary);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.sidebar-header {
  padding: 20px 16px;
  border-bottom: 1px solid var(--color-border);
  display: flex;
  align-items: center;
  gap: 10px;
}

.sidebar-logo {
  width: 32px;
  height: 32px;
  background: linear-gradient(135deg, var(--color-accent), #a29bfe);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 800;
  color: white;
  flex-shrink: 0;
}

.sidebar-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text-primary);
}

.sidebar-nav {
  flex: 1;
  padding: 12px 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 8px;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.15s;
}

.nav-item:hover {
  background: var(--color-bg-tertiary);
  color: var(--color-text-primary);
}

.nav-item.active {
  background: var(--color-accent);
  color: white;
}

.nav-icon {
  font-size: 16px;
  width: 20px;
  text-align: center;
}

.nav-label {
  font-size: 13px;
  font-weight: 500;
}

.sidebar-footer {
  padding: 12px 8px;
  border-top: 1px solid var(--color-border);
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}
</style>
