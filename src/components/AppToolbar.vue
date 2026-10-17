<script setup lang="ts">
import { computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { NButton, NPopselect, useMessage } from "naive-ui";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import { useUiStore } from "../stores/ui";
import AppIcon from "./AppIcon.vue";
import SaveStatusIndicator from "./SaveStatusIndicator.vue";

const projectStore = useProjectStore();
const timeline = useTimelineStore();
const ui = useUiStore();
const message = useMessage();

/// 最终字幕轨（fused）：导出以它为准
const fusedTrack = computed(() =>
  projectStore.currentProject?.tracks.find((t) => t.type === "fused")
);
const fusedCount = computed(
  () => fusedTrack.value?.events.filter((e) => e.type === "fused").length ?? 0
);

/// 面包屑显示 .gsa 文件名（project.path 就是工程文件路径）
const projectName = computed(() => {
  const path = projectStore.currentProject?.path ?? "";
  return path.split(/[\\/]/).pop() || projectStore.currentProject?.name || "未打开工程";
});

/// 撤销/重做后聚焦可能悬空（clip/轨道已被快照恢复移除），清理之
function cleanupFocus() {
  if (timeline.focusedClipId && !projectStore.findEvent(timeline.focusedClipId)) {
    timeline.focusClip(null);
  }
  if (timeline.focusedTrackId && !projectStore.findTrack(timeline.focusedTrackId)) {
    timeline.focusTrack(null);
  }
}

function onUndo() {
  projectStore.undo();
  cleanupFocus();
}

function onRedo() {
  projectStore.redo();
  cleanupFocus();
}

// ── 导出最终字幕（复用后端 export_track_subtitle，支持 srt/ass/lrc/txt）─────────
const EXPORT_FORMATS = [
  { label: "SRT（通用字幕）", value: "srt" },
  { label: "ASS（带样式）", value: "ass" },
  { label: "LRC（歌词）", value: "lrc" },
  { label: "TXT（纯文本）", value: "txt" },
];

async function onExport(format: string) {
  const track = fusedTrack.value;
  if (!track) return;
  try {
    const path = await save({
      defaultPath: `final-subtitle.${format}`,
      filters: [{ name: `${format.toUpperCase()} 字幕`, extensions: [format] }],
    });
    if (!path) return;
    const count = await invoke<number>("export_track_subtitle", {
      track,
      format,
      destPath: path,
    });
    message.success(`已导出 ${count} 条字幕 → ${path}`);
  } catch (e) {
    message.error(`导出失败: ${e}`);
  }
}
</script>

<template>
  <header class="toolbar">
    <button
      class="tbtn"
      :title="ui.sidebarCollapsed ? '展开侧栏' : '折叠侧栏'"
      :aria-label="ui.sidebarCollapsed ? '展开侧栏' : '折叠侧栏'"
      @click="ui.toggleSidebar()"
    >
      <AppIcon :name="ui.sidebarCollapsed ? 'expand' : 'collapse'" />
    </button>

    <span class="tsep" />

    <button
      class="tbtn"
      :disabled="!projectStore.canUndo"
      title="撤销（Ctrl+Z）"
      aria-label="撤销"
      @click="onUndo()"
    >
      <AppIcon name="undo" />
    </button>
    <button
      class="tbtn"
      :disabled="!projectStore.canRedo"
      title="重做（Ctrl+Shift+Z / Ctrl+Y）"
      aria-label="重做"
      @click="onRedo()"
    >
      <AppIcon name="redo" />
    </button>

    <span class="tsep" />

    <span class="crumb">
      <span class="c1">工程</span>
      <span class="csep">/</span>
      <span class="c2">{{ projectName }}</span>
    </span>

    <div class="tbar-r">
      <button
        class="tgl"
        :title="ui.isDark ? '切换到浅色主题' : '切换到深色主题'"
        @click="ui.toggleTheme()"
      >
        <AppIcon :name="ui.isDark ? 'moon' : 'sun'" :size="12" />
        {{ ui.isDark ? "深色" : "浅色" }}
      </button>

      <SaveStatusIndicator />

      <NPopselect
        :options="EXPORT_FORMATS"
        trigger="click"
        placement="bottom-end"
        :disabled="!fusedTrack || fusedCount === 0"
        @update:value="onExport"
      >
        <NButton
          type="primary"
          size="small"
          :disabled="!fusedTrack || fusedCount === 0"
        >
          <template #icon><AppIcon name="export" :size="13" /></template>
          导出最终字幕
        </NButton>
      </NPopselect>
    </div>
  </header>
</template>

<style scoped>
.toolbar {
  flex: 0 0 36px;
  height: 36px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 12px;
  background: var(--color-bg-secondary);
  border-bottom: 1px solid var(--color-border);
}

.tbtn {
  width: 26px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
}

.tbtn:hover:not(:disabled) {
  background: var(--color-bg-tertiary);
  color: var(--color-text-primary);
}

.tbtn:disabled {
  opacity: 0.38;
  cursor: default;
}

.tsep {
  width: 1px;
  height: 18px;
  background: var(--color-border);
  margin: 0 6px;
}

.crumb {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-left: 4px;
  min-width: 0;
  font-size: 13px;
}

.c1 {
  color: var(--color-text-secondary);
}

.csep {
  color: var(--color-text-secondary);
  opacity: 0.5;
}

.c2 {
  color: var(--color-text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tbar-r {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 10px;
}

.tgl {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding: 0 9px;
  border: 1px solid var(--color-accent);
  border-radius: 999px;
  background: var(--accent-soft);
  color: var(--accent-ink);
  font-size: 11px;
  white-space: nowrap;
  cursor: pointer;
}
</style>
