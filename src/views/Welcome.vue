<script setup lang="ts">
import { ref, computed, onMounted, nextTick } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "../stores/project";
import { open } from "@tauri-apps/plugin-dialog";
import type { RecentProject } from "../types";
import { NButton, NCard, NInput, NModal, NPopconfirm, NSpace, NText, useMessage } from "naive-ui";

const router = useRouter();
const projectStore = useProjectStore();
const message = useMessage();

const showCreateModal = ref(false);
const newProjectName = ref("");
const newProjectPath = ref("");

async function handleSelectFolder() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择项目文件夹",
  });
  if (selected) {
    newProjectPath.value = selected;
  }
}

async function handleCreateProject() {
  if (!newProjectName.value || !newProjectPath.value) return;
  const project = await projectStore.createProject(
    newProjectName.value,
    newProjectPath.value
  );
  showCreateModal.value = false;
  resetForm();
  router.push(`/project/${encodeURIComponent(project.path)}/editor`);
}

function resetForm() {
  newProjectName.value = "";
  newProjectPath.value = "";
}

async function handleOpenProject(path: string) {
  try {
    await projectStore.openProject(path);
  } catch (e) {
    message.error(`打开项目失败：${e}`);
    return;
  }
  router.push(`/project/${encodeURIComponent(path)}/editor`);
}

/// 打开已有项目：选择 .gsa 项目文件（项目身份 = 文件，同目录可有多个项目）
async function handleOpenProjectDialog() {
  const selected = await open({
    multiple: false,
    title: "打开 GSA 项目",
    filters: [{ name: "GSA 项目", extensions: ["gsa"] }],
  });
  if (!selected) return;
  await handleOpenProject(selected);
}

function isValidPath(input: string) {
  return input.length > 0;
}

// 项目名即项目文件名（<项目名>.gsa），Windows 非法文件名字符直接拦在前端
const ILLEGAL_NAME_RE = /[\\/:*?"<>|\x00-\x1f]/;
const nameIllegal = computed(() => ILLEGAL_NAME_RE.test(newProjectName.value));

// 后端 updated_at 是 Unix 秒字符串 → "YYYY-MM-DD HH:mm"
function formatUpdatedAt(unixSecs: string): string {
  const date = new Date(Number(unixSecs) * 1000);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

// ── 行内重命名（复用轨道重命名交互：按钮触发，Enter/失焦提交、Esc 取消）──
const renamingPath = ref<string | null>(null);
const renameDraft = ref("");
const renameInput = ref<HTMLInputElement | null>(null);

/// v-for 内的模板 ref 会被收集为数组，focus 会失败；用函数 ref 绑定当前渲染的输入框
function setRenameInput(el: unknown) {
  renameInput.value = (el as HTMLInputElement) ?? null;
}

function startRename(proj: RecentProject) {
  if (renamingPath.value) return;
  renamingPath.value = proj.path;
  renameDraft.value = proj.name;
  nextTick(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

// 失焦提交必然伴随一次落点点击：记录刚提交的行，该次点击不触发"打开项目"
let justCommittedPath: string | null = null;

async function commitRename() {
  const path = renamingPath.value;
  if (!path) return;
  const name = renameDraft.value.trim();
  const proj = projectStore.recentProjects.find((p) => p.path === path);
  if (name && name !== proj?.name && ILLEGAL_NAME_RE.test(name)) {
    message.error("项目名含非法字符（\\ / : * ? \" < > |），无法用作项目文件名");
    renameInput.value?.focus();
    return;
  }
  renamingPath.value = null;
  // 名字未变或为空：视为取消，不发请求
  if (!name || name === proj?.name) return;
  try {
    await projectStore.renameProject(path, name);
  } catch (e) {
    message.error(`重命名失败：${e}`);
  }
}

function cancelRename() {
  renamingPath.value = null;
}

/// 重命名中的行：点击行内非输入框区域即提交，且该次点击不触发打开项目
function onRowMouseDown(proj: RecentProject, e: MouseEvent) {
  if (renamingPath.value !== proj.path) return;
  if ((e.target as HTMLElement).closest(".rename-input")) return;
  justCommittedPath = proj.path;
  commitRename();
}

function onRowClick(proj: RecentProject) {
  if (renamingPath.value === proj.path) return;
  if (justCommittedPath === proj.path) {
    justCommittedPath = null;
    return;
  }
  handleOpenProject(proj.path);
}

async function handleRemoveProject(proj: RecentProject) {
  try {
    await projectStore.removeRecentProject(proj.path);
  } catch (e) {
    message.error(`移除失败：${e}`);
  }
}

onMounted(() => {
  projectStore.refreshRecentProjects();
});
</script>

<template>
  <div class="welcome">
    <div class="welcome-left">
      <div class="logo-icon">GSA</div>
      <h1 class="title">GameSubtitleAssistant</h1>
      <p class="subtitle">AI 游戏剧情字幕生产工作站</p>

      <div class="actions">
        <NButton size="large" type="primary" @click="showCreateModal = true">
          创建新项目
        </NButton>
        <NButton size="large" @click="handleOpenProjectDialog">
          打开已有项目
        </NButton>
      </div>
    </div>

    <div class="welcome-right">
      <h2 class="recent-title">最近项目</h2>

      <div v-if="projectStore.recentProjects.length > 0" class="recent-list">
        <div
          v-for="proj in projectStore.recentProjects"
          :key="proj.path"
          class="recent-item"
          @mousedown="onRowMouseDown(proj, $event)"
          @click="onRowClick(proj)"
        >
          <div class="recent-item-main">
            <input
              v-if="renamingPath === proj.path"
              :ref="setRenameInput"
              v-model="renameDraft"
              class="rename-input"
              @keydown.enter="commitRename"
              @keydown.esc="cancelRename"
              @blur="commitRename"
            />
            <template v-else>
              <NText strong>{{ proj.name }}</NText>
              <NText depth="3" class="recent-path">{{ proj.path }}</NText>
              <NText depth="3" class="recent-time">
                最后打开：{{ formatUpdatedAt(proj.updated_at) }}
              </NText>
            </template>
          </div>
          <div v-if="renamingPath !== proj.path" class="recent-item-actions" @click.stop>
            <NButton size="tiny" quaternary @click="startRename(proj)">
              重命名
            </NButton>
            <NPopconfirm @positive-click="handleRemoveProject(proj)">
              <template #trigger>
                <NButton size="tiny" quaternary type="error">
                  删除
                </NButton>
              </template>
              从最近项目列表移除「{{ proj.name }}」？项目文件不会被删除
            </NPopconfirm>
          </div>
        </div>
      </div>

      <div v-else class="recent-empty">
        <p class="recent-empty-title">暂无最近项目</p>
        <p class="recent-empty-hint">创建或打开一个项目后，将显示在这里</p>
      </div>
    </div>

    <NModal v-model:show="showCreateModal" title="创建新项目">
      <NCard title="创建新项目" style="width: 480px">
        <NSpace vertical>
          <NInput
            v-model:value="newProjectName"
            placeholder="项目名称"
            clearable
          />
          <NText v-if="nameIllegal" type="error" style="font-size: 12px">
            项目名含非法字符（\ / : * ? " &lt; &gt; |），无法用作项目文件名
          </NText>
          <NInput
            v-model:value="newProjectPath"
            placeholder="选择项目保存位置"
            readonly
          />
          <NButton @click="handleSelectFolder">选择文件夹</NButton>
          <NButton
            type="primary"
            :disabled="
              !isValidPath(newProjectPath) || !newProjectName || nameIllegal
            "
            @click="handleCreateProject"
          >
            创建
          </NButton>
        </NSpace>
      </NCard>
    </NModal>
  </div>
</template>

<style scoped>
.welcome {
  height: 100vh;
  display: flex;
  background: var(--color-bg-primary);
}

/* ── 左半边：logo 信息区，水平垂直居中 ── */
.welcome-left {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 48px 32px;
}

.logo-icon {
  width: 96px;
  height: 96px;
  margin-bottom: 8px;
  background: linear-gradient(135deg, var(--color-accent), #a29bfe);
  border-radius: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 32px;
  font-weight: 800;
  color: white;
}

.title {
  font-size: 32px;
  font-weight: 700;
  color: var(--color-text-primary);
}

.subtitle {
  font-size: 15px;
  color: var(--color-text-secondary);
}

.actions {
  display: flex;
  gap: 12px;
  margin-top: 24px;
}

/* ── 右半边：最近项目列表 ── */
.welcome-right {
  flex: 1;
  display: flex;
  flex-direction: column;
  background: var(--color-bg-secondary);
  border-left: 1px solid var(--color-border);
  padding: 32px 40px;
  min-width: 0;
}

.recent-title {
  font-size: 14px;
  color: var(--color-text-secondary);
  text-transform: uppercase;
  letter-spacing: 1px;
  margin-bottom: 16px;
  flex-shrink: 0;
}

.recent-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.recent-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 16px;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.15s;
  flex-shrink: 0;
}

.recent-item:hover {
  background: var(--color-bg-tertiary);
}

.recent-item-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.rename-input {
  width: 100%;
  font-family: inherit;
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text-primary);
  background: var(--color-bg-primary);
  border: 1px solid var(--color-accent);
  border-radius: 4px;
  padding: 2px 6px;
  outline: none;
  box-sizing: border-box;
}

.recent-path {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.recent-time {
  font-size: 11px;
}

.recent-item-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}

/* ── 空状态 ── */
.recent-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1px dashed var(--color-border);
  border-radius: 12px;
}

.recent-empty-title {
  font-size: 15px;
  color: var(--color-text-secondary);
}

.recent-empty-hint {
  font-size: 12px;
  color: var(--color-text-secondary);
  opacity: 0.7;
}
</style>
