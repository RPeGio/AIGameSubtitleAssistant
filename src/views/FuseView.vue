<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";
import {
  NButton,
  NCard,
  NCheckbox,
  NProgress,
  NTag,
  NText,
  useMessage,
} from "naive-ui";

const projectStore = useProjectStore();
const router = useRouter();
const message = useMessage();

/// 语料就绪卡片 → 点击跳转语料页
const corpusReady = computed(() => projectStore.corpusReady);
const corpusCount = computed(() => projectStore.currentProject?.corpus.length ?? 0);

/// 时间轴就绪卡片 → 点击跳转转写页
const timelineReady = computed(() => projectStore.timelineReady);
/// 与 timelineReady 同口径：游戏内容轨（game-ASR 语音 + 嵌字 OCR 内嵌字幕）合计段数
const gameContentCount = computed(() => {
  const tracks = projectStore.currentProject?.tracks ?? [];
  return tracks
    .filter(
      (t) =>
        (t.type === "asr" || t.type === "embed_ocr") &&
        t.track_role === "game"
    )
    .reduce((n, t) => n + t.events.length, 0);
});

/// 向量融合运行时就绪（bootstrap_embed.ps1 后就绪；未就绪给引导提示）
const embedStatus = ref<{ ready: boolean; message: string } | null>(null);
onMounted(async () => {
  try {
    embedStatus.value = await invoke("check_embed_runtime");
  } catch (e) {
    embedStatus.value = { ready: false, message: String(e) };
  }
});
/// 「该不配」掩码开关（按素材，默认关；见标定 §7.6/§7.9）
const unmatchFilter = ref(false);

/// 双就绪才可融合（向量运行时也须就绪）
const canFuse = computed(
  () => corpusReady.value && timelineReady.value && (embedStatus.value?.ready ?? false)
);

const fuseResult = ref<{
  matched: number;
  total: number;
  masked: number;
  reused: number;
} | null>(null);

function goCorpus() {
  const path = router.currentRoute.value.params.path as string | undefined;
  router.push({ name: "corpus", params: { path } });
}

function goAsr() {
  const path = router.currentRoute.value.params.path as string | undefined;
  router.push({ name: "asr", params: { path } });
}

async function startFuse() {
  try {
    const result = await projectStore.runFuse(unmatchFilter.value);
    fuseResult.value = {
      matched: result.stats.matched,
      total: result.stats.total,
      masked: result.stats.masked_segments,
      reused: result.stats.reused_segments,
    };
    message.success(
      `AI 融合完成：匹配 ${result.stats.matched}/${result.stats.total} 段`
    );
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <div class="page">
    <h2 class="page-title">AI 融合</h2>
    <p class="page-desc">
      用可靠的游戏文本替换转写文本，保留其时间轴（向量召回对齐）
    </p>

    <!-- 两张就绪卡片：可点击跳转对应页（无论是否就绪） -->
    <div class="cards">
      <NCard
        class="ready-card"
        :class="{ ready: corpusReady, empty: !corpusReady }"
        hoverable
        @click="goCorpus"
      >
        <div class="card-header">
          <span class="card-title">语料</span>
          <NTag :type="corpusReady ? 'success' : 'default'" size="small">
            {{ corpusReady ? "就绪" : "未就绪" }}
          </NTag>
        </div>
        <NText depth="3" class="card-desc">
          {{ corpusReady ? `已收集 ${corpusCount} 条可靠文本` : "尚未收集语料（点击前往语料页）" }}
        </NText>
      </NCard>

      <NCard
        class="ready-card"
        :class="{ ready: timelineReady, empty: !timelineReady }"
        hoverable
        @click="goAsr"
      >
        <div class="card-header">
          <span class="card-title">时间轴</span>
          <NTag :type="timelineReady ? 'success' : 'default'" size="small">
            {{ timelineReady ? "就绪" : "未就绪" }}
          </NTag>
        </div>
        <NText depth="3" class="card-desc">
          {{
            timelineReady
              ? `已生成 ${gameContentCount} 段游戏内容文本（语音 + 内嵌字幕）`
              : "尚无游戏内容：运行 ASR 或对内嵌字幕做 OCR（点击前往转写页）"
          }}
        </NText>
      </NCard>
    </div>

    <!-- 融合操作 -->
    <div class="fuse-section">
      <NCheckbox v-model:checked="unmatchFilter" size="small">
        启用「该不配」掩码
        <NText depth="3" style="font-size: 11px">
          （显示语言比语料语言更省内容的素材才适用；误开会把本应命中的段判为不配）
        </NText>
      </NCheckbox>
      <NButton
        type="primary"
        size="large"
        :disabled="!canFuse || projectStore.fuseRunning"
        @click="startFuse"
      >
        {{ projectStore.fuseRunning ? "融合运行中..." : "开始融合" }}
      </NButton>
      <template v-if="projectStore.fuseRunning">
        <NProgress
          type="line"
          class="progress"
          :percentage="Math.round(projectStore.llmProgress * 100)"
          :show-indicator="false"
        />
        <span class="ocr-msg">{{ projectStore.llmMessage }}</span>
      </template>

      <NText v-if="!canFuse && !projectStore.fuseRunning" depth="3" style="font-size: 12px">
        {{
          embedStatus && !embedStatus.ready
            ? `向量运行时未就绪：${embedStatus.message}`
            : "需语料与时间轴均就绪后方可融合（点击上方卡片前往对应页面准备）"
        }}
      </NText>

      <div v-if="fuseResult" class="fuse-result">
        <NText strong>
          匹配 {{ fuseResult.matched }}/{{ fuseResult.total }} 段
        </NText>
        <NText v-if="fuseResult.masked > 0" depth="3" style="font-size: 12px">
          {{ fuseResult.masked }} 段被「该不配」掩码判为不配，保留转写原文
        </NText>
        <NText v-if="fuseResult.reused > 0" depth="3" style="font-size: 12px">
          {{ fuseResult.reused }} 条语料行被多段命中（候选重要提示，见审计表）
        </NText>
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
  margin-bottom: 20px;
}

.cards {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
  gap: 16px;
  margin-bottom: 24px;
}

.ready-card {
  cursor: pointer;
  transition: transform 0.15s;
}

.ready-card:hover {
  transform: translateY(-2px);
}

.ready-card.ready {
  border-color: var(--color-success, #18a058);
}

.ready-card.empty {
  opacity: 0.85;
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.card-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--color-text-primary);
}

.card-desc {
  font-size: 13px;
}

.fuse-section {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 12px;
}

.progress {
  width: 100%;
  max-width: 480px;
}

.ocr-msg {
  font-size: 12px;
  color: var(--color-text-secondary);
}

.fuse-result {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px 16px;
  background: var(--color-bg-secondary);
  border-radius: 8px;
}
</style>
