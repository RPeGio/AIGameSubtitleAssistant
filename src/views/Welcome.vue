<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "../stores/project";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NCard, NInput, NModal, NSpace, NText, useMessage } from "naive-ui";

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

onMounted(() => {
  projectStore.refreshRecentProjects();
});
</script>

<template>
  <div class="welcome">
    <div class="welcome-content">
      <div class="logo-area">
        <div class="logo-icon">GSA</div>
        <h1 class="title">GameSubtitleAssistant</h1>
        <p class="subtitle">AI 游戏剧情字幕生产工作站</p>
      </div>

      <div class="actions">
        <NButton size="large" type="primary" @click="showCreateModal = true">
          创建新项目
        </NButton>
        <NButton size="large" @click="handleOpenProjectDialog">
          打开已有项目
        </NButton>
      </div>

      <div v-if="projectStore.recentProjects.length > 0" class="recent-section">
        <h2 class="recent-title">最近项目</h2>
        <div class="recent-list">
          <NCard
            v-for="proj in projectStore.recentProjects"
            :key="proj.path"
            class="recent-card"
            hoverable
            @click="handleOpenProject(proj.path)"
          >
            <div class="recent-card-body">
              <NText strong>{{ proj.name }}</NText>
              <NText depth="3" class="recent-path">{{ proj.path }}</NText>
            </div>
          </NCard>
        </div>
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
  align-items: center;
  justify-content: center;
  background: var(--color-bg-primary);
}

.welcome-content {
  max-width: 520px;
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 32px;
  padding: 48px 32px;
}

.logo-area {
  text-align: center;
}

.logo-icon {
  width: 80px;
  height: 80px;
  margin: 0 auto 16px;
  background: linear-gradient(135deg, var(--color-accent), #a29bfe);
  border-radius: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 28px;
  font-weight: 800;
  color: white;
}

.title {
  font-size: 32px;
  font-weight: 700;
  color: var(--color-text-primary);
  margin-bottom: 8px;
}

.subtitle {
  font-size: 15px;
  color: var(--color-text-secondary);
}

.actions {
  display: flex;
  gap: 12px;
}

.recent-section {
  width: 100%;
}

.recent-title {
  font-size: 14px;
  color: var(--color-text-secondary);
  margin-bottom: 12px;
  text-transform: uppercase;
  letter-spacing: 1px;
}

.recent-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.recent-card {
  cursor: pointer;
}

.recent-card-body {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.recent-path {
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
