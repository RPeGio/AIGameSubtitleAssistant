<script setup lang="ts">
import { ref } from "vue";
import { useProjectStore } from "../stores/project";
import type { LlmRuntimeStatus } from "../types";
import {
  NAlert,
  NButton,
  NCard,
  NInput,
  NModal,
  NProgress,
  NSpace,
  NText,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const message = useMessage();

// ── LLM 测试台（保留在编辑页，供调试）───────────────────
const showLlmPanel = ref(false);
const llmPrompt = ref("");
const llmResult = ref("");
const llmRuntimeStatus = ref<LlmRuntimeStatus | null>(null);

/// 打开面板：重置上次结果并探测运行时状态
async function openLlmPanel() {
  showLlmPanel.value = true;
  llmResult.value = "";
  try {
    llmRuntimeStatus.value = await projectStore.checkLlmRuntime();
  } catch {
    // 探测失败也要给用户可见的反馈，不能静默
    llmRuntimeStatus.value = {
      provider: "",
      ready: false,
      message: "无法探测 LLM 运行环境状态",
    };
  }
}

async function runLlm() {
  llmResult.value = "";
  try {
    llmResult.value = await projectStore.runLlm(llmPrompt.value);
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <div class="workbench">
    <!-- 编辑页：撤销/重做 + LLM 测试台 -->
    <div class="toolbar">
      <button
        class="history-btn"
        :disabled="!projectStore.canUndo"
        title="撤销（Ctrl+Z）"
        aria-label="撤销"
        @click="projectStore.undo()"
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
        @click="projectStore.redo()"
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

      <NButton size="small" type="primary" @click="openLlmPanel">
        LLM 测试台
      </NButton>
    </div>

    <!-- 工作区说明 -->
    <div class="workbench-body">
      <div class="empty-icon">🧰</div>
      <h2>{{ projectStore.currentProject?.name ?? "加载中..." }}</h2>
      <p class="empty-desc">
        在左侧对应页面运行 OCR / ASR / AI 融合任务，结果实时出现在右侧校对区的时间轴上，本页可撤销/重做并微调轨道。
      </p>
    </div>

    <!-- LLM 测试台弹窗 -->
    <NModal v-model:show="showLlmPanel" title="LLM 测试台" :mask-closable="false">
      <NCard title="LLM 测试台" style="width: 560px">
        <NSpace vertical size="large">
          <NAlert
            v-if="llmRuntimeStatus && !llmRuntimeStatus.ready"
            type="warning"
            :show-icon="true"
          >
            {{ llmRuntimeStatus.message }}
          </NAlert>

          <NInput
            v-model:value="llmPrompt"
            type="textarea"
            :rows="5"
            placeholder="输入 prompt，例如：2+2=?"
          />

          <div class="llm-run-row">
            <template v-if="projectStore.llmRunning">
              <NProgress
                type="line"
                class="llm-progress"
                :percentage="Math.round(projectStore.llmProgress * 100)"
                :show-indicator="false"
              />
              <NText depth="3">{{ projectStore.llmMessage }}</NText>
            </template>
            <NButton
              size="small"
              type="primary"
              :disabled="projectStore.llmRunning"
              @click="runLlm"
            >
              {{ projectStore.llmRunning ? "推理中..." : "运行" }}
            </NButton>
          </div>

          <NCard v-if="llmResult" title="结果" size="small">
            <pre class="llm-result">{{ llmResult }}</pre>
          </NCard>
        </NSpace>
      </NCard>
    </NModal>
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

.llm-run-row {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
}

.llm-progress {
  flex: 1;
  min-width: 200px;
}

.llm-result {
  white-space: pre-wrap;
  word-break: break-all;
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--color-text-primary);
}
</style>
