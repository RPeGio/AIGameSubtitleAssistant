<script setup lang="ts">
import { onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import AppSidebar from "../components/AppSidebar.vue";
import ReviewPane from "../components/ReviewPane.vue";
import { useProjectStore } from "../stores/project";

const route = useRoute();
const router = useRouter();
const projectStore = useProjectStore();

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
  flex: 1;
  min-width: 0;
  display: flex;
  background: var(--color-bg-primary);
  overflow: hidden;
}
</style>
