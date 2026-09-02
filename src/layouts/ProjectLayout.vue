<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import AppSidebar from "../components/AppSidebar.vue";
import ReviewPane from "../components/ReviewPane.vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import { useManualSave } from "../composables/useManualSave";

const route = useRoute();
const router = useRouter();
const projectStore = useProjectStore();
const timeline = useTimelineStore();
const { manualSave } = useManualSave();

// 直接访问 /project/:path/... 时按路径打开项目；失败则回欢迎页
onMounted(async () => {
  if (!projectStore.currentProject) {
    const path = route.params.path as string;
    if (path) {
      try {
        await projectStore.openProject(path);
        return;
      } catch {
        // 项目不存在或损坏 → 回欢迎页
      }
    }
    router.replace("/");
  }
});

// ── 全局快捷键：仅 Ctrl+S 保存；Ctrl+Z 撤销；Ctrl+Shift+Z / Ctrl+Y 重做 ──
// S 分割 / Delete 删除 / M 合并 已下沉到各 Timeline 实例（处理自己的 store）
function cleanupFocus() {
  if (timeline.focusedClipId && !projectStore.findEvent(timeline.focusedClipId)) {
    timeline.focusClip(null);
  }
  if (timeline.focusedTrackId && !projectStore.findTrack(timeline.focusedTrackId)) {
    timeline.focusTrack(null);
  }
}

function onGlobalKeydown(e: KeyboardEvent) {
  const t = e.target as HTMLElement;
  if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;

  // 撤销/重做（含聚焦悬空清理）
  if (e.code === "KeyZ" && e.ctrlKey) {
    e.preventDefault();
    if (e.shiftKey) {
      projectStore.redo();
    } else {
      projectStore.undo();
    }
    cleanupFocus();
    return;
  }
  if (e.code === "KeyY" && e.ctrlKey) {
    e.preventDefault();
    projectStore.redo();
    cleanupFocus();
    return;
  }

  if (e.code === "KeyS" && e.ctrlKey) {
    e.preventDefault(); // 挡住浏览器默认保存对话框
    manualSave();
    return;
  }

  // Delete/S/M 已下沉到各 Timeline 实例处理自己的 store（校对区全局 + 语料页独立），
  // 这里不重复处理，避免双重触发作用于错误的时间轴。
}

onMounted(() => window.addEventListener("keydown", onGlobalKeydown));
onUnmounted(() => window.removeEventListener("keydown", onGlobalKeydown));
</script>

<template>
  <div class="project-layout">
    <AppSidebar />
    <main class="workspace-main">
      <router-view />
    </main>
    <ReviewPane />
  </div>
</template>

<style scoped>
.project-layout {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

.workspace-main {
  flex: 0 1 35%;
  min-width: 360px;
  display: flex;
  background: var(--color-bg-primary);
  overflow: hidden;
}
</style>
