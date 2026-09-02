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

// ── 全局快捷键：Ctrl+S 保存；S 分割；Delete 删除；M 合并；Ctrl+Z 撤销；Ctrl+Shift+Z / Ctrl+Y 重做
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

  // Delete：删除聚焦 clip 已下沉到 Timeline 组件内（每个实例处理自己的 store，
  // 覆盖校对区全局时间轴与语料页独立时间轴），这里不再重复处理
  // M：聚焦 clip 与同轨下一个事件合并（输入框内由 guard 排除）
  if (e.code === "KeyM" && !e.ctrlKey && !e.metaKey && !e.altKey) {
    const id = timeline.focusedClipId;
    if (id) projectStore.mergeAdjacent(id);
    return;
  }

  if (e.code === "KeyS" && !e.metaKey && !e.altKey) {
    const id = timeline.focusedClipId;
    if (!id) return;
    const rightId = projectStore.splitEvent(id, timeline.currentTime);
    if (rightId) {
      timeline.focusClip(rightId);
      const found = projectStore.findEvent(rightId);
      if (found) timeline.focusTrack(found.track.id);
    }
  }
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
