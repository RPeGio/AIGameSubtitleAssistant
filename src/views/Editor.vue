<script setup lang="ts">
import { ref } from "vue";
import { useProjectStore } from "../stores/project";
import type { OcrRunParams, AsrRunParams, AsrEngineStatus, LlmRuntimeStatus } from "../types";
import {
  NButton,
  NSpace,
  NAlert,
  NCard,
  NModal,
  NInput,
  NInputNumber,
  NProgress,
  NRadio,
  NRadioGroup,
  NSelect,
  NText,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const message = useMessage();

// ── OCR 控制 ────────────────────────────────────────────
const showOcrConfig = ref(false);
const ocrParams = ref<OcrRunParams>({
  frame_interval: 0.5,
  dhash_threshold: 3,
  batch_size: 16,
  merge_similarity: 0.3,
});

async function startOcr() {
  showOcrConfig.value = false;
  try {
    await projectStore.runOcr(ocrParams.value);
    message.success("OCR 完成");
  } catch (e) {
    message.error(String(e));
  }
}

// ── ASR 控制 ────────────────────────────────────────────
const showAsrConfig = ref(false);
const asrParams = ref<AsrRunParams>({
  engine: "funasr",
  max_speakers: null,
  language: null,
});
/// 两引擎运行环境（打开面板时探测，禁用不可用引擎）
const asrEngines = ref<AsrEngineStatus[]>([]);

const asrLanguageOptions = [
  { label: "中文", value: "zh" },
  { label: "English", value: "en" },
  { label: "日本語", value: "ja" },
];

function engineStatus(engine: string): AsrEngineStatus | undefined {
  return asrEngines.value.find((e) => e.engine === engine);
}

/// 探测失败（asrEngines 为空）时直通：不阻塞用户，运行失败由错误提示兜底
function probeFailed(): boolean {
  return asrEngines.value.length === 0;
}

function engineReady(engine: string): boolean {
  if (probeFailed()) return true;
  return engineStatus(engine)?.ready ?? false;
}

function engineDetail(engine: string): string {
  const s = engineStatus(engine);
  if (!s) return probeFailed() ? "状态未知（探测失败，可直接尝试）" : "正在探测…";
  return s.ready ? "就绪" : s.message || "未配置";
}

async function openAsrConfig() {
  try {
    asrEngines.value = await projectStore.getAsrEngines();
  } catch {
    asrEngines.value = [];
  }
  showAsrConfig.value = true;
}

async function startAsr() {
  showAsrConfig.value = false;
  try {
    await projectStore.runAsr(asrParams.value);
    message.success("ASR 完成");
  } catch (e) {
    // 用户主动取消是预期行为，用中性提示而非错误
    if (String(e).includes("已取消")) {
      message.info("ASR 已取消");
    } else {
      message.error(String(e));
    }
  }
}

function cancelAsr() {
  projectStore.cancelAsr();
}

// ── LLM 控制 ────────────────────────────────────────────
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

// ── AI 融合控制 ─────────────────────────────────────────
async function startFuse() {
  try {
    const result = await projectStore.runFuse();
    const failed = result.stats.failed_batches;
    const suffix = failed > 0 ? `，${failed} 批解析失败已保留原文本` : "";
    message.success(
      `AI 融合完成：匹配 ${result.stats.matched}/${result.stats.total} 段${suffix}`
    );
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <div class="workbench">
    <!-- 操作台：AI 任务运行入口（结果实时出现在右侧校对区时间轴） -->
    <div class="ocr-toolbar">
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
      <NButton
        size="small"
        type="primary"
        :disabled="projectStore.ocrRunning"
        @click="showOcrConfig = true"
      >
        运行 OCR
      </NButton>
      <template v-if="projectStore.ocrRunning">
        <NProgress
          type="line"
          class="ocr-progress"
          :percentage="Math.round(projectStore.ocrProgress * 100)"
          :show-indicator="false"
        />
        <span class="ocr-msg">{{ projectStore.ocrMessage }}</span>
      </template>

      <NButton
        size="small"
        type="primary"
        :disabled="projectStore.asrRunning"
        @click="openAsrConfig"
      >
        运行 ASR
      </NButton>
      <template v-if="projectStore.asrRunning">
        <NProgress
          type="line"
          class="ocr-progress"
          :percentage="Math.round(projectStore.asrProgress * 100)"
          :show-indicator="false"
        />
        <span class="ocr-msg">{{ projectStore.asrMessage }}</span>
        <NButton size="tiny" quaternary type="error" @click="cancelAsr">
          取消
        </NButton>
      </template>

      <NButton size="small" type="primary" @click="openLlmPanel">
        LLM
      </NButton>

      <NButton
        size="small"
        type="primary"
        :disabled="projectStore.fuseRunning"
        @click="startFuse"
      >
        AI 融合
      </NButton>
      <template v-if="projectStore.fuseRunning">
        <NProgress
          type="line"
          class="ocr-progress"
          :percentage="Math.round(projectStore.llmProgress * 100)"
          :show-indicator="false"
        />
        <span class="ocr-msg">{{ projectStore.llmMessage }}</span>
      </template>
    </div>

    <!-- 工作区说明 -->
    <div class="workbench-body">
      <div class="empty-icon">🧰</div>
      <h2>{{ projectStore.currentProject?.name ?? "加载中..." }}</h2>
      <p class="empty-desc">
        在此运行 OCR / ASR / LLM / AI 融合任务，结果实时出现在右侧校对区的时间轴上。
      </p>
    </div>

    <!-- OCR 参数弹窗 -->
    <NModal v-model:show="showOcrConfig" :mask-closable="false">
        <NCard title="OCR 参数设置" style="width: 440px">
          <NSpace vertical size="large">
            <div class="cfg-field">
              <NText depth="2">帧间隔（秒）</NText>
              <NInputNumber
                v-model:value="ocrParams.frame_interval"
                :min="0.1"
                :step="0.5"
                style="width: 100%"
              />
            </div>
            <div class="cfg-field">
              <NText depth="2">变化检测阈值</NText>
              <NInputNumber
                v-model:value="ocrParams.dhash_threshold"
                :min="0"
                :max="64"
                :precision="0"
                :step="1"
                style="width: 100%"
              />
            </div>
            <div class="cfg-field">
              <NText depth="2">批大小</NText>
              <NInputNumber
                v-model:value="ocrParams.batch_size"
                :min="1"
                :max="128"
                :precision="0"
                :step="1"
                style="width: 100%"
              />
            </div>
            <div class="cfg-field">
              <NText depth="2">合并相似度（0~1，越大越易合并）</NText>
              <NInputNumber
                v-model:value="ocrParams.merge_similarity"
                :min="0"
                :max="1"
                :step="0.05"
                style="width: 100%"
              />
            </div>
            <NText depth="3" style="font-size: 12px">
              提示：若发现有漏识别，可降低帧间隔后重新运行。
            </NText>
            <NSpace justify="end">
              <NButton size="small" @click="showOcrConfig = false">取消</NButton>
              <NButton size="small" type="primary" @click="startOcr">开始</NButton>
            </NSpace>
          </NSpace>
        </NCard>
      </NModal>

      <!-- ASR 参数弹窗 -->
      <NModal v-model:show="showAsrConfig" :mask-closable="false">
        <NCard title="ASR 参数设置" style="width: 480px">
          <NSpace vertical size="large">
            <div class="cfg-field">
              <NText depth="2">引擎（必选）</NText>
              <NRadioGroup v-model:value="asrParams.engine">
                <NSpace vertical>
                  <NRadio value="funasr" :disabled="!engineReady('funasr')">
                    <div class="engine-option">
                      <div>FunASR + diarize（推荐，本地 GPU 快）</div>
                      <NText depth="3" style="font-size: 12px">
                        {{ engineDetail("funasr") }}
                      </NText>
                    </div>
                  </NRadio>
                  <NRadio value="moss" :disabled="!engineReady('moss')">
                    <div class="engine-option">
                      <div>MOSS-Transcribe-Diarize（慢但更准）</div>
                      <NText depth="3" style="font-size: 12px">
                        {{ engineDetail("moss") }}
                      </NText>
                    </div>
                  </NRadio>
                </NSpace>
              </NRadioGroup>
            </div>
            <div class="cfg-field">
              <NText depth="2">最大说话人数量（选填，默认自动估计）</NText>
              <NInputNumber
                v-model:value="asrParams.max_speakers"
                :min="1"
                :precision="0"
                :step="1"
                placeholder="自动估计"
                :disabled="asrParams.engine === 'moss'"
                style="width: 100%"
              />
              <NText v-if="asrParams.engine === 'moss'" depth="3" style="font-size: 12px">
                MOSS 自动估计说话人，不支持手动限制
              </NText>
            </div>
            <div class="cfg-field">
              <NText depth="2">识别语言（选填，默认自动检测）</NText>
              <NSelect
                v-model:value="asrParams.language"
                :options="asrLanguageOptions"
                clearable
                placeholder="自动检测"
                :disabled="asrParams.engine === 'moss'"
                style="width: 100%"
              />
              <NText v-if="asrParams.engine === 'moss'" depth="3" style="font-size: 12px">
                MOSS 自动识别多语言，无需指定
              </NText>
            </div>
            <NText depth="3" style="font-size: 12px">
              提示：识别语言不准确时可在面板指定语言后重新运行。
            </NText>
            <NSpace justify="end">
              <NButton size="small" @click="showAsrConfig = false">取消</NButton>
              <NButton
                size="small"
                type="primary"
                :disabled="!engineReady(asrParams.engine)"
                @click="startAsr"
              >
                开始
              </NButton>
            </NSpace>
          </NSpace>
        </NCard>
      </NModal>

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

.ocr-toolbar {
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

.ocr-progress {
  flex: 1;
  max-width: 320px;
}

.ocr-msg {
  font-size: 12px;
  color: var(--color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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

.cfg-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.engine-option {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
</style>
