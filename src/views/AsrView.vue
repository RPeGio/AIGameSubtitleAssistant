<script setup lang="ts">
import { ref } from "vue";
import { useProjectStore } from "../stores/project";
import type { AsrRunParams, AsrEngineStatus } from "../types";
import {
  NButton,
  NCollapse,
  NCollapseItem,
  NInputNumber,
  NProgress,
  NRadio,
  NRadioGroup,
  NSelect,
  NSpace,
  NText,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const message = useMessage();

// ── 两个时间轴来源折叠标签（默认展开第一个）────────────
const activeKeys = ref<string[]>(["audio"]);

// ── "音频获取时间轴"（ASR）─────────────────────────────
const asrParams = ref<AsrRunParams>({
  engine: "funasr",
  max_speakers: null,
  language: null,
});
/// 两引擎运行环境（展开时探测，禁用不可用引擎）
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

async function openAsrPanel() {
  try {
    asrEngines.value = await projectStore.getAsrEngines();
  } catch {
    asrEngines.value = [];
  }
}

async function startAsr() {
  try {
    await projectStore.runAsr(asrParams.value);
    message.success("ASR 完成，时间轴已生成");
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

// ── 占位标签 ──────────────────────────────────────────
const PLACEHOLDER_KEY = "visual";
</script>

<template>
  <div class="page">
    <h2 class="page-title">语音转写</h2>
    <p class="page-desc">从切片视频生成精确的字幕轴 / 视频内嵌字轴</p>

    <NCollapse v-model:expanded-names="activeKeys" class="sources" arrow-placement="right">
      <!-- ① 音频获取时间轴（ASR） -->
      <NCollapseItem name="audio" title="音频获取时间轴">
        <div class="source-body">
          <NText depth="3" style="font-size: 12px">
            对切片视频做语音识别 + 说话人分离，产出带说话人标签的时间轴与转录文本
          </NText>

          <!-- ASR 参数（内联，点击立即执行） -->
          <div class="params">
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
          </div>

          <!-- 仅"开始 ASR"按钮，点击立即执行 -->
          <div class="run-row">
            <NButton
              type="primary"
              :disabled="projectStore.asrRunning || !engineReady(asrParams.engine)"
              @click="startAsr"
            >
              {{ projectStore.asrRunning ? "ASR 运行中..." : "开始 ASR" }}
            </NButton>
            <template v-if="projectStore.asrRunning">
              <NProgress
                type="line"
                class="progress"
                :percentage="Math.round(projectStore.asrProgress * 100)"
                :show-indicator="false"
              />
              <span class="ocr-msg">{{ projectStore.asrMessage }}</span>
              <NButton size="tiny" quaternary type="error" @click="cancelAsr">
                取消
              </NButton>
            </template>
          </div>

          <NButton size="small" quaternary @click="openAsrPanel">
            重新探测引擎状态
          </NButton>
        </div>
      </NCollapseItem>

      <!-- ② 检测画面变化获取时间轴 -->
      <NCollapseItem :name="PLACEHOLDER_KEY" title="检测画面变化获取时间轴">
        <div class="source-body">
          <div class="placeholder">功能开发中，敬请期待</div>
        </div>
      </NCollapseItem>
    </NCollapse>
  </div>
</template>

<style scoped>
.page {
  flex: 1;
  min-width: 0;
  padding: 20px 24px;
  overflow: auto;
}

.page-title {
  font-size: 18px;
  font-weight: 700;
  color: var(--color-text-primary);
  margin-bottom: 8px;
}

.page-desc {
  font-size: 13px;
  color: var(--color-text-secondary);
  margin-bottom: 16px;
}

.sources {
  margin-bottom: 24px;
}

.source-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 4px 0;
}

.params {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 12px;
  padding: 12px;
  background: var(--color-bg-secondary);
  border-radius: 8px;
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

.run-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.progress {
  flex: 1;
  max-width: 320px;
}

.ocr-msg {
  font-size: 12px;
  color: var(--color-text-secondary);
}

.placeholder {
  padding: 32px;
  border: 1px dashed var(--color-border);
  border-radius: 8px;
  text-align: center;
  font-size: 13px;
  color: var(--color-text-secondary);
}
</style>
