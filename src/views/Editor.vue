<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import type { OcrRunParams, LlmRuntimeStatus } from "../types";
import AppSidebar from "../components/AppSidebar.vue";
import VideoPlayer from "../components/VideoPlayer.vue";
import Timeline from "../components/timeline/Timeline.vue";
import TrackOverview from "../components/TrackOverview.vue";
import { useManualSave } from "../composables/useManualSave";
import {
  NButton,
  NTag,
  NSpace,
  NAlert,
  NCard,
  NModal,
  NInput,
  NInputNumber,
  NProgress,
  NText,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const timeline = useTimelineStore();
const { manualSave } = useManualSave();
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

async function startAsr() {
  try {
    await projectStore.runAsr();
    message.success("ASR 完成");
  } catch (e) {
    message.error(String(e));
  }
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

watch(
  () => timeline.duration,
  (d) => {
    if (d > 0) {
      projectStore.ensureDefaultTrack(d);
      // 默认聚焦 ocr 选区轨道，让遮罩立即可见
      const ocrTrack = projectStore.currentProject?.tracks.find(
        (t) => t.type === "ocr_region"
      );
      if (ocrTrack && !timeline.focusedTrackId) {
        timeline.focusTrack(ocrTrack.id);
      }
    }
  }
);

// 快捷键：Ctrl+S 立即保存；S 分割；Delete 删除；M 合并；
// Ctrl+Z 撤销；Ctrl+Shift+Z / Ctrl+Y 重做
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

  // Delete：删除聚焦 clip（输入框内由上面的 guard 排除）
  if (e.key === "Delete") {
    const id = timeline.focusedClipId;
    if (!id) return;
    projectStore.removeEvent(id);
    timeline.focusClip(null);
    return;
  }

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

// 撤销/重做后聚焦可能悬空（clip/轨道已被快照恢复移除），清理之
function cleanupFocus() {
  if (timeline.focusedClipId && !projectStore.findEvent(timeline.focusedClipId)) {
    timeline.focusClip(null);
  }
  if (timeline.focusedTrackId && !projectStore.findTrack(timeline.focusedTrackId)) {
    timeline.focusTrack(null);
  }
}

onMounted(() => window.addEventListener("keydown", onGlobalKeydown));
onUnmounted(() => window.removeEventListener("keydown", onGlobalKeydown));

const meta = computed(() => projectStore.currentVideoMeta);
const hasVideo = computed(() => meta.value !== null);

const displayPath = computed(() => {
  if (!meta.value) return "";
  const parts = meta.value.path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] ?? meta.value.path;
});

const durationFormatted = computed(() => {
  if (!meta.value) return "";
  const s = Math.round(meta.value.duration);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  return `${m}:${String(sec).padStart(2, "0")}`;
});

const resolutionLabel = computed(() => {
  if (!meta.value) return "";
  const { width } = meta.value;
  if (width >= 3840) return "4K";
  if (width >= 2560) return "1440p";
  if (width >= 1920) return "1080p";
  if (width >= 1280) return "720p";
  return "";
});
</script>

<template>
  <div class="editor-layout">
    <AppSidebar />
    <main class="editor-main">
      <!-- video imported -->
      <div v-if="hasVideo" class="editor-content">
        <div class="top-pane">
          <div class="preview-row">
            <div class="player-area">
              <VideoPlayer :src="meta!.path" />
            </div>

            <div class="overview-area">
              <TrackOverview />
            </div>
          </div>

          <div class="info-bar">
            <NSpace wrap size="small">
              <NTag>{{ displayPath }}</NTag>
              <NTag>{{ meta?.width }}×{{ meta?.height }}</NTag>
              <NTag v-if="resolutionLabel">{{ resolutionLabel }}</NTag>
              <NTag>{{ durationFormatted }}</NTag>
              <NTag>{{ meta?.fps.toFixed(1) }}fps</NTag>
              <NTag>{{ meta?.codec }}</NTag>
            </NSpace>

            <NButton size="small" type="primary" @click="projectStore.importVideo()">
              更换视频
            </NButton>
          </div>
        </div>

        <!-- OCR 工具栏 -->
        <div class="ocr-toolbar">
          <button
            class="history-btn"
            :disabled="!projectStore.canUndo"
            title="撤销（Ctrl+Z）"
            aria-label="撤销"
            @click="projectStore.undo(); cleanupFocus()"
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
            @click="projectStore.redo(); cleanupFocus()"
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
            @click="startAsr"
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
          </template>

          <NButton size="small" type="primary" @click="openLlmPanel">
            LLM
          </NButton>
        </div>

        <div class="timeline-pane">
          <Timeline />
        </div>
      </div>

      <!-- empty state -->
      <div v-else class="editor-empty">
        <div class="empty-icon">📹</div>
        <h2>{{ projectStore.currentProject?.name ?? "加载中..." }}</h2>
        <p class="empty-desc">导入游戏录屏以开始字幕生产</p>

        <NButton
          size="large"
          type="primary"
          @click="projectStore.importVideo()"
          class="import-btn"
        >
          导入视频
        </NButton>

        <NAlert
          v-if="projectStore.videoImportError"
          type="error"
          class="error-alert"
          closable
          @close="projectStore.videoImportError = null"
        >
          {{ projectStore.videoImportError }}
        </NAlert>

        <p class="empty-hint">支持 mp4 / mkv / webm / avi / mov / flv</p>
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
    </main>
  </div>
</template>

<style scoped>
.editor-layout {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

.editor-main {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-bg-primary);
}

.editor-content {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.top-pane {
  flex: 1 1 65%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 24px 8px;
}

.preview-row {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 8px;
}

.player-area {
  flex: 1 1 60%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.overview-area {
  flex: 1 1 40%;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.info-bar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 10px 12px;
  background: var(--color-bg-secondary);
  border-radius: 8px;
}

.timeline-pane {
  flex: 0 0 35%;
  min-height: 0;
  overflow: hidden;
  padding: 0 12px 12px;
}

.ocr-toolbar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 24px 8px;
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

.editor-empty {
  text-align: center;
  color: var(--color-text-secondary);
  max-width: 400px;
}

.empty-icon {
  font-size: 64px;
  margin-bottom: 16px;
}

.empty-desc {
  margin: 8px 0 24px;
  font-size: 14px;
}

.import-btn {
  margin-bottom: 16px;
}

.error-alert {
  margin-top: 16px;
  text-align: left;
}

.empty-hint {
  font-size: 12px;
  color: var(--color-text-secondary);
  opacity: 0.6;
}
</style>
