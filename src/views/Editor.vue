<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import type { FusedEvent } from "../types";
import {
  NAlert,
  NButton,
  NInput,
  NPopselect,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const timeline = useTimelineStore();
const router = useRouter();
const message = useMessage();

/// 最终字幕轨（fused）：编辑页只做融合产物，避免与校对区时间轴职责重叠
const fusedTrack = computed(() =>
  projectStore.currentProject?.tracks.find((t) => t.type === "fused")
);

/// 按时间排序的最终字幕事件
const clips = computed<FusedEvent[]>(() => {
  const track = fusedTrack.value;
  if (!track) return [];
  return [...track.events]
    .filter((e): e is FusedEvent => e.type === "fused")
    .sort((a, b) => a.start - b.start);
});

function fmtTime(s: number): string {
  const totalMs = Math.round(s * 1000);
  const ms = totalMs % 1000;
  const sec = Math.floor(totalMs / 1000) % 60;
  const min = Math.floor(totalMs / 60000);
  return `${String(min).padStart(2, "0")}:${String(sec).padStart(2, "0")}.${String(ms).padStart(3, "0")}`;
}

/// 点击行：跳转校对区视频/时间轴并聚焦该 clip
function onRowClick(ev: FusedEvent) {
  timeline.jumpTo(ev.start);
  timeline.focusClip(ev.id);
  if (fusedTrack.value) timeline.focusTrack(fusedTrack.value.id);
}

/// 就地编辑文本（即时生效）
function updateText(ev: FusedEvent, text: string) {
  projectStore.updateEventText(ev.id, text);
}

/// 就地编辑角色（change 触发：失焦/回车提交，避免逐键压撤销快照）
function updateCharacter(ev: FusedEvent, character: string) {
  const found = projectStore.findEvent(ev.id);
  if (found && found.event.type === "fused") {
    projectStore.recordSnapshot();
    found.event.character = character.trim() || undefined;
  }
}

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

function removeClip(ev: FusedEvent) {
  projectStore.removeEvent(ev.id);
  if (timeline.focusedClipId === ev.id) timeline.focusClip(null);
}

/// 分割当前 clip：在播放头位置切开（与校对区 S 键语义一致）
function splitClip(ev: FusedEvent) {
  const rightId = projectStore.splitEvent(ev.id, timeline.currentTime);
  if (rightId) {
    timeline.focusClip(rightId);
    timeline.focusTrack(fusedTrack.value?.id ?? null);
  } else {
    message.info("播放头不在该字幕区间内，无法分割");
  }
}

/// 合并当前 clip 与同轨下一事件（与校对区 M 键语义一致）
function mergeNext(ev: FusedEvent) {
  const merged = projectStore.mergeAdjacent(ev.id);
  if (merged) {
    timeline.focusClip(merged);
  } else {
    message.info("已到最后一条，或下一条不在同一轨道");
  }
}

// ── 导出最终字幕（复用后端 export_track_subtitle）─────────
const EXPORT_FORMATS = [
  { label: "SRT（通用字幕）", value: "srt" },
  { label: "ASS（带样式）", value: "ass" },
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

function goFuse() {
  const path = router.currentRoute.value.params.path as string | undefined;
  router.push({ name: "fuse", params: { path } });
}
</script>

<template>
  <div class="workbench">
    <!-- 工具条：撤销/重做 + 导出最终字幕 -->
    <div class="toolbar">
      <button
        class="history-btn"
        :disabled="!projectStore.canUndo"
        title="撤销（Ctrl+Z）"
        aria-label="撤销"
        @click="onUndo()"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M2.5 2v6h6M2.66 15.57a10 10 0 1 0 .57-8.38" />
        </svg>
      </button>
      <button
        class="history-btn"
        :disabled="!projectStore.canRedo"
        title="重做（Ctrl+Shift+Z / Ctrl+Y）"
        aria-label="重做"
        @click="onRedo()"
      >
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38" />
        </svg>
      </button>

      <div class="toolbar-spacer" />

      <NPopselect
        :options="EXPORT_FORMATS"
        trigger="click"
        placement="bottom-end"
        :disabled="!fusedTrack || clips.length === 0"
        @update:value="onExport"
      >
        <NButton type="primary" size="small" secondary :disabled="!fusedTrack || clips.length === 0">
          导出最终字幕
        </NButton>
      </NPopselect>
    </div>

    <!-- 无 fused 轨：引导去融合页 -->
    <div v-if="!fusedTrack" class="workbench-body">
      <div class="empty-icon">🎬</div>
      <h2>{{ projectStore.currentProject?.name ?? "加载中..." }}</h2>
      <p class="empty-desc">
        尚无最终字幕。请先在「AI 融合」页运行融合，生成最终字幕轨后，再回到本页逐条校对与导出。
      </p>
      <NButton type="primary" size="small" @click="goFuse">前往 AI 融合</NButton>
    </div>

    <!-- 有 fused 轨但无事件 -->
    <div v-else-if="clips.length === 0" class="workbench-body">
      <div class="empty-icon">📝</div>
      <p class="empty-desc">最终字幕轨已创建，但还没有内容——请前往「AI 融合」页运行融合。</p>
      <NButton type="primary" size="small" @click="goFuse">前往 AI 融合</NButton>
    </div>

    <!-- 最终字幕列表：逐条校对 -->
    <div v-else class="editor-body">
      <NAlert type="info" :show-icon="true" class="editor-hint">
        点击行可跳转视频对应时间；文本/角色就地编辑即时生效；行内按钮分割 / 合并 / 删除。
      </NAlert>

      <div class="subtitle-list">
        <div
          v-for="(ev, i) in clips"
          :key="ev.id"
          class="subtitle-row"
          :class="{ active: timeline.focusedClipId === ev.id }"
          @click="onRowClick(ev)"
        >
          <span class="row-index">{{ i + 1 }}</span>
          <div class="row-time">
            <span class="time-start">{{ fmtTime(ev.start) }}</span>
            <span class="time-end">{{ fmtTime(ev.end) }}</span>
          </div>
          <div class="row-fields" @click.stop>
            <NInput
              :value="ev.character ?? ''"
              size="tiny"
              placeholder="角色"
              class="row-character"
              @change="updateCharacter(ev, $event)"
            />
            <NInput
              :value="ev.text"
              type="textarea"
              :autosize="{ minRows: 1, maxRows: 6 }"
              class="row-text"
              placeholder="字幕文本"
              @update:value="updateText(ev, $event)"
            />
          </div>
          <div class="row-actions" @click.stop>
            <NButton size="tiny" quaternary title="在播放头处分割" @click="splitClip(ev)">
              分割
            </NButton>
            <NButton size="tiny" quaternary title="与下一条合并" @click="mergeNext(ev)">
              合并
            </NButton>
            <NButton size="tiny" quaternary type="error" title="删除本条" @click="removeClip(ev)">
              删除
            </NButton>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.workbench {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.toolbar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 24px 8px;
  flex-wrap: wrap;
}

.toolbar-spacer {
  flex: 1;
}

.workbench-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  color: var(--color-text-secondary);
  max-width: 460px;
  margin: 0 auto;
  padding: 24px;
}

.empty-icon {
  font-size: 64px;
  margin-bottom: 16px;
}

.empty-desc {
  margin: 8px 0 24px;
  font-size: 14px;
  line-height: 1.7;
}

/* 撤销/重做：透明底色，可用时亮色图标，不可用时浅灰 */
.history-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--color-text-primary);
  cursor: pointer;
}

.history-btn:not(:disabled):hover {
  background: rgba(255, 255, 255, 0.12);
}

.history-btn:disabled {
  color: var(--color-text-secondary);
  cursor: default;
}

.editor-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 8px 24px 20px;
  overflow: auto;
}

.editor-hint {
  flex-shrink: 0;
}

.subtitle-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.subtitle-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 8px 10px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.12s, box-shadow 0.12s;
}

.subtitle-row:hover {
  border-color: var(--color-accent);
}

.subtitle-row.active {
  border-color: var(--color-accent);
  box-shadow: inset 3px 0 0 var(--color-accent);
}

.row-index {
  flex-shrink: 0;
  width: 22px;
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary);
  line-height: 28px;
  text-align: center;
}

.row-time {
  flex-shrink: 0;
  min-width: 64px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  line-height: 1.25;
  padding-top: 4px;
}

.time-start {
  color: var(--color-text-secondary);
}

.time-end {
  color: var(--color-text-primary);
}

.row-fields {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.row-character {
  max-width: 180px;
}

.row-actions {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 2px;
}
</style>
