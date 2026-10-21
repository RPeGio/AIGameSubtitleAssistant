<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import AppSidebar from "../components/AppSidebar.vue";
import AppStatusBar from "../components/AppStatusBar.vue";
import AppToolbar from "../components/AppToolbar.vue";
import ReviewPane from "../components/ReviewPane.vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import { useUiStore } from "../stores/ui";
import { useManualSave } from "../composables/useManualSave";
import { dispatchShortcut, isEditableTarget, onShortcut } from "../composables/shortcuts";

const route = useRoute();
const router = useRouter();
const projectStore = useProjectStore();
const timeline = useTimelineStore();
const ui = useUiStore();
const { manualSave } = useManualSave();

// 语料页加工的是它自己的剧情录屏（时间轴与校对区不相干），故整页全宽、不挂校对区
const showReviewPane = computed(() => route.name !== "corpus");

// 页面与校对区都套了 KeepAlive（切页不重建：保住折叠卡/草稿/滚动/时间轴缩放，
// 也避免校对区波形 canvas 每次从语料页回来都整幅重绘）。缓存键取工程路径：
// 同一工程内切页继续复用，换工程才整棵重建。
const projectKey = computed(
  () => projectStore.currentProject?.path ?? (route.params.path as string) ?? "none"
);

// 焦点窗格：点左列 → focus=main，点校对区 → focus=review。
// 转写页的 OCR 选区"焦点在 main 时才在校对区显示"，所以拖选区本身不算
// 把焦点交给校对区（否则选区会在拖拽起点就消失，没法拖）。
function onReviewMouseDown(e: MouseEvent) {
  const el = e.target as HTMLElement | null;
  if (el?.closest(".region-overlay")) return;
  ui.setFocusedPane("review");
}

// 切页＝换工作对象：默认焦点回到页面主列（转写页一进来就能看到选区）
watch(
  () => route.name,
  () => ui.setFocusedPane("main")
);

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

// ── 全局快捷键：统一由 composables/shortcuts.ts 分发（键位表只在那里定义）──
// 这里只登记全局动作；S 分割 / Delete 删除 / M 合并等由各 Timeline 实例登记。
function onGlobalKeydown(e: KeyboardEvent) {
  if (isEditableTarget(e.target)) return;
  if (dispatchShortcut(e)) e.preventDefault();
}

const offShortcuts: Array<() => void> = [];

onMounted(() => {
  window.addEventListener("keydown", onGlobalKeydown);
  offShortcuts.push(
    onShortcut("save", () => manualSave()),
    onShortcut("undo", () => {
      projectStore.undo();
      cleanupFocus();
    }),
    onShortcut("redo", () => {
      projectStore.redo();
      cleanupFocus();
    })
  );
});

onUnmounted(() => {
  window.removeEventListener("keydown", onGlobalKeydown);
  offShortcuts.splice(0).forEach((off) => off());
});
</script>

<template>
  <div class="project-layout">
    <AppToolbar />
    <div class="shell">
      <AppSidebar />
      <main
        class="workspace-main"
        :class="{ full: !showReviewPane, focused: showReviewPane && ui.focusedPane === 'main' }"
        @mousedown.capture="ui.setFocusedPane('main')"
      >
        <!-- KeepAlive 只接受单个子节点，注释放在外面。
             key 必须「页面 + 工程」都在：只用工程路径的话四个页面会共用同一个
             缓存键，KeepAlive 会把它们当成同一个实例复用（切页不换内容）。 -->
        <router-view v-slot="{ Component }">
          <KeepAlive>
            <component :is="Component" :key="projectKey + ':' + String(route.name)" />
          </KeepAlive>
        </router-view>
      </main>
      <KeepAlive>
        <ReviewPane
          v-if="showReviewPane"
          :key="projectKey"
          :class="{ focused: ui.focusedPane === 'review' }"
          @mousedown.capture="onReviewMouseDown"
        />
      </KeepAlive>
    </div>
    <AppStatusBar />
  </div>
</template>

<style scoped>
.project-layout {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
}

.shell {
  flex: 1;
  min-height: 0;
  display: flex;
}

.workspace-main {
  flex: 0 1 35%;
  min-width: 360px;
  display: flex;
  background: var(--color-bg-primary);
  overflow: hidden;
}

.workspace-main.full {
  flex: 1 1 auto;
  min-width: 0;
}

/* 焦点窗格的高亮描边：主列 / 校对区成对出现，改动请同步两处。
   用 outline 而不是 inset box-shadow——两侧内容都是不透明背景，
   inset 阴影会被子元素盖住；outline-offset:-1px 让线画在边缘内侧。 */
.workspace-main.focused,
.review-pane.focused {
  outline: 1px solid color-mix(in srgb, var(--color-accent) 45%, transparent);
  outline-offset: -1px;
}
</style>
