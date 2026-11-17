<script setup lang="ts">
import { useRoute, useRouter } from "vue-router";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import AppIcon from "./AppIcon.vue";

const route = useRoute();
const router = useRouter();
const projectStore = useProjectStore();
const ui = useUiStore();

/// 工作流导航项：name = 路由名，segment = 子路由路径
const NAV_ITEMS = [
  { name: "corpus", segment: "corpus", icon: "text", label: "文本语料" },
  { name: "asr", segment: "asr", icon: "wave", label: "打轴+转写" },
  { name: "fuse", segment: "fuse", icon: "fuse", label: "AI 融合" },
  { name: "editor", segment: "editor", icon: "track", label: "时间轴编辑" },
];

function isActive(name: string): boolean {
  return route.name === name;
}

function navigate(name: string) {
  const path = route.params.path as string | undefined;
  router.push({ name, params: { path } });
}

function handleBack() {
  projectStore.closeProject();
  router.push("/");
}
</script>

<template>
  <aside class="sidebar" :class="{ collapsed: ui.sidebarCollapsed }">
    <div class="sbrand">
      <span class="logo">GSA</span>
      <span v-if="!ui.sidebarCollapsed" class="bn">GameSubtitleAssistant</span>
    </div>

    <nav class="nav">
      <button
        v-for="item in NAV_ITEMS"
        :key="item.name"
        type="button"
        class="nitem"
        :class="{ on: isActive(item.name) }"
        :title="ui.sidebarCollapsed ? item.label : undefined"
        @click="navigate(item.name)"
      >
        <AppIcon :name="item.icon" />
        <span v-if="!ui.sidebarCollapsed" class="nl">{{ item.label }}</span>
      </button>
    </nav>

    <div class="sf">
      <button type="button" class="sfrow" title="返回工程列表" @click="handleBack">
        <AppIcon name="back" />
        <span v-if="!ui.sidebarCollapsed">返回</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  flex: 0 0 208px;
  width: 208px;
  display: flex;
  flex-direction: column;
  padding: 8px 6px;
  min-height: 0;
  background: var(--color-bg-secondary);
  border-right: 1px solid var(--color-border);
  transition: flex-basis 0.12s, width 0.12s, padding 0.12s;
}

/* 折叠态只用 padding 定位（44 = 5 + 34 + 5）。这里若用 align-items:center，
   它会与宽度动画错拍：宽度还没收完就先把内容居中，logo 先右跳再随宽度左移 */
.sidebar.collapsed {
  flex: 0 0 44px;
  width: 44px;
  padding: 8px 5px;
}

.sbrand {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 40px;
  padding: 0 4px;
  margin-bottom: 12px;
  min-width: 0;
  transition: padding 0.12s;
}

.logo {
  width: 40px;
  height: 40px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background: var(--color-accent);
  color: #fff;
  font-family: var(--mono);
  font-size: 15px;
  font-weight: 600;
  transition: width 0.12s, height 0.12s, font-size 0.12s;
}

/* 折叠态：logo 收成与导航图标同宽的 34px，左缘与图标列对齐，位置不动 */
.sidebar.collapsed .sbrand {
  padding: 0;
}

.sidebar.collapsed .logo {
  width: 34px;
  height: 34px;
  font-size: 13px;
}

.bn {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: -0.03em;
  color: var(--color-text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
}

.nitem {
  position: relative;
  height: 34px;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 0 9px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--color-text-secondary);
  font-family: inherit;
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
}

.nitem:hover {
  background: var(--color-bg-tertiary);
  color: var(--color-text-primary);
}

.nitem.on {
  background: var(--accent-soft);
  color: var(--color-text-primary);
}

.nitem.on::before {
  content: "";
  position: absolute;
  left: 0;
  top: 7px;
  bottom: 7px;
  width: 3px;
  border-radius: 0 2px 2px 0;
  background: var(--color-accent);
}

.sidebar.collapsed .nitem {
  width: 34px;
  height: 34px;
  padding: 0;
  justify-content: center;
  gap: 0;
}

.nl {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sf {
  margin-top: auto;
  width: 100%;
  padding-top: 8px;
  border-top: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.sfrow {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 26px;
  padding: 0 9px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--color-text-secondary);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
}

.sfrow:hover {
  background: var(--color-bg-tertiary);
  color: var(--color-text-primary);
}

.sidebar.collapsed .sfrow {
  width: 34px;
  padding: 0;
  justify-content: center;
}
</style>
