<script setup lang="ts">
import { computed, ref } from "vue";
import { useProjectStore } from "../stores/project";
import SourceVideoPreview from "../components/SourceVideoPreview.vue";
import SourceTimeline from "../components/SourceTimeline.vue";
import type { OcrRunParams } from "../types";
import {
  NButton,
  NCollapse,
  NCollapseItem,
  NInputNumber,
  NProgress,
  NText,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const message = useMessage();

// ── 三个语料来源折叠标签（默认展开第一个）──────────────
const activeKeys = ref<string[]>(["recordings"]);

// ── "从剧情录屏中截取" ────────────────────────────────
const ocrParams = ref<OcrRunParams>({
  frame_interval: 0.5,
  dhash_threshold: 3,
  batch_size: 16,
  merge_similarity: 0.3,
});

const sourceMeta = computed(() => projectStore.sourceVideoMeta);
const hasSourceVideo = computed(() => sourceMeta.value !== null);

/// 共享播放状态：视频预览 ↔ 迷你时间轴同步
const sourceTime = ref(0);
const sourceDuration = ref(0);

async function startOcr() {
  try {
    await projectStore.runOcr(ocrParams.value, "source");
    message.success("OCR 完成，文本已提取到语料");
  } catch (e) {
    message.error(String(e));
  }
}

// ── 语料列表 ──────────────────────────────────────────
const corpus = computed(() => projectStore.currentProject?.corpus ?? []);

function removeItem(id: string) {
  projectStore.removeCorpusItem(id);
}

// ── 占位标签 ──────────────────────────────────────────
const PLACEHOLDER_KEYS = ["screenshot", "manual"];
</script>

<template>
  <div class="page">
    <h2 class="page-title">文本语料</h2>
    <p class="page-desc">获取可靠的游戏内文本，作为 AI 融合的语料来源</p>

    <NCollapse v-model:expanded-names="activeKeys" class="sources" arrow-placement="right">
      <!-- ① 从剧情录屏中截取 -->
      <NCollapseItem name="recordings" title="从剧情录屏中截取">
        <div class="source-body">
          <template v-if="!hasSourceVideo">
            <NButton type="primary" @click="projectStore.importSourceVideo()">
              导入剧情录屏
            </NButton>
            <NText depth="3" style="font-size: 12px">
              支持 mp4 / mkv / webm / avi / mov / flv
            </NText>
          </template>

          <template v-else>
            <!-- 视频选区预览（独立于全局时间轴与全局空格，播放时间与下方迷你时间轴同步） -->
            <div class="preview-box">
              <SourceVideoPreview
                v-model:time="sourceTime"
                v-model:duration="sourceDuration"
              />
            </div>

            <!-- 迷你时间轴：按时间段管理多条选区 -->
            <SourceTimeline
              v-model:time="sourceTime"
              :duration="sourceDuration"
              class="mini-timeline"
            />
            <NText depth="3" style="font-size: 12px">
              在预览中拖拽选框框定字幕区域；双击迷你时间轴空白添加不同时间段的选区，
              拖动 clip 调整起止、点击跳转。OCR 按播放头所在时间段对应的选区识别。
            </NText>

            <!-- OCR 参数（内联，点击立即执行） -->
            <div class="params">
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
            </div>

            <!-- 仅"开始 OCR"按钮，点击立即执行 -->
            <div class="run-row">
              <NButton
                type="primary"
                :disabled="projectStore.ocrRunning"
                @click="startOcr"
              >
                {{ projectStore.ocrRunning ? "OCR 运行中..." : "开始 OCR" }}
              </NButton>
              <template v-if="projectStore.ocrRunning">
                <NProgress
                  type="line"
                  class="progress"
                  :percentage="Math.round(projectStore.ocrProgress * 100)"
                  :show-indicator="false"
                />
                <span class="ocr-msg">{{ projectStore.ocrMessage }}</span>
              </template>
            </div>
          </template>
        </div>
      </NCollapseItem>

      <!-- ② 从文本截图中截取 -->
      <NCollapseItem
        v-for="key in PLACEHOLDER_KEYS"
        :key="key"
        :name="key"
        :title="key === 'screenshot' ? '从文本截图中截取' : '手动提供文本'"
      >
        <div class="source-body">
          <div class="placeholder">功能开发中，敬请期待</div>
        </div>
      </NCollapseItem>
    </NCollapse>

    <!-- 语料列表 -->
    <div class="corpus-section">
      <h3 class="corpus-title">语料列表（{{ corpus.length }}）</h3>
      <div v-if="corpus.length === 0" class="corpus-empty">
        暂无语料——运行上方"从剧情录屏中截取"后，OCR 文本将自动进入此列表
      </div>
      <div v-else class="corpus-list">
        <div v-for="item in corpus" :key="item.id" class="corpus-item">
          <span class="corpus-text">{{ item.text }}</span>
          <span class="corpus-source">{{ item.source }}</span>
          <NButton size="tiny" quaternary type="error" @click="removeItem(item.id)">
            删除
          </NButton>
        </div>
      </div>
    </div>
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

.preview-box {
  height: 300px;
  background: #000;
  border-radius: 8px;
  overflow: hidden;
}

.mini-timeline {
  flex-shrink: 0;
}

.params {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
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

.corpus-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text-primary);
  margin-bottom: 12px;
}

.corpus-empty {
  padding: 24px;
  border: 1px dashed var(--color-border);
  border-radius: 8px;
  text-align: center;
  font-size: 13px;
  color: var(--color-text-secondary);
}

.corpus-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.corpus-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  background: var(--color-bg-secondary);
  border-radius: 8px;
}

.corpus-text {
  flex: 1;
  font-size: 13px;
  color: var(--color-text-primary);
  word-break: break-all;
}

.corpus-source {
  font-size: 11px;
  color: var(--color-text-secondary);
  flex-shrink: 0;
}
</style>
