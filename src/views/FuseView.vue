<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";
import {
  auditCounts,
  sortRows,
  type FuseAuditDecision,
  type FuseAuditRow,
} from "../stores/fuseAudit";
import {
  NButton,
  NCard,
  NCheckbox,
  NInput,
  NModal,
  NProgress,
  NTag,
  NText,
  NVirtualList,
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
    // 融合成功自动展开审计表（重置展开覆盖，回到「A 展开/B 折叠」默认态）
    auditOpen.value = true;
    expanded.value = {};
  } catch (e) {
    message.error(String(e));
  }
}

// ── 融合审计表（R3）────────────────────────────
/// 数据源 = store 计算属性（fused 轨 fuse_meta × 语料），裁决后自动重算并落盘
const auditCnt = computed(() => auditCounts(projectStore.fusedAuditRows));
const auditOpen = ref(false);
/// 展开状态覆盖表：未写入时按分层默认（A 展开，其余折叠）
const expanded = ref<Record<string, boolean>>({});
const showNormal = ref(false); // 显示已复核/正常段
const pickerShow = ref(false); // 全语料惰性弹窗（display-directive=if，打开才渲染）
const pickerEventId = ref<string | null>(null);
const pickerQuery = ref("");

const visibleRows = computed(() => {
  const rows = sortRows(projectStore.fusedAuditRows, "risk");
  return showNormal.value ? rows : rows.filter((r) => r.risk !== "normal");
});

function rowExpanded(row: FuseAuditRow): boolean {
  return expanded.value[row.event.id] ?? row.risk === "A";
}

function toggleRow(id: string, def: boolean) {
  expanded.value[id] = !(expanded.value[id] ?? def);
}

function snippet(s: string, n = 42): string {
  return s.length > n ? s.slice(0, n) + "…" : s;
}

function corpusAt(ocrIndex: number): string {
  return projectStore.currentProject?.corpus[ocrIndex - 1]?.text ?? "";
}

const pickerItems = computed(() => {
  const all = projectStore.currentProject?.corpus ?? [];
  const q = pickerQuery.value.trim();
  return all
    .map((c, i) => ({ index: i + 1, text: c.text }))
    .filter((c) => !q || c.text.includes(q));
});

function openPicker(eventId: string) {
  pickerEventId.value = eventId;
  pickerQuery.value = "";
  pickerShow.value = true;
}

/// 审计四动作统一入口：失败给 toast（零副作用），成功计数条自动清减
function decide(eventId: string, decision: FuseAuditDecision) {
  const err = projectStore.applyFusedAuditDecision(eventId, decision);
  if (err) {
    message.error(err);
    return;
  }
  message.success("已记录裁决");
}

function pickCorpus(ocrIndex: number) {
  if (!pickerEventId.value) return;
  decide(pickerEventId.value, { kind: "replace", ocrIndex });
  pickerShow.value = false;
}

function fmtTime(s: number): string {
  const totalMs = Math.round(s * 1000);
  const ms = totalMs % 1000;
  const sec = Math.floor(totalMs / 1000) % 60;
  const min = Math.floor(totalMs / 60000);
  return `${String(min).padStart(2, "0")}:${String(sec).padStart(2, "0")}.${String(ms).padStart(3, "0")}`;
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

    <!-- 融合审计表：裁决随 .gsa 持久化，重开工程不丢 -->
    <div v-if="auditCnt.total > 0" class="audit-section">
      <div class="audit-head">
        <NText strong>融合审计</NText>
        <NTag :type="auditCnt.focused > 0 ? 'warning' : 'success'" size="small">
          重点 {{ auditCnt.focused }}
        </NTag>
        <NTag size="small" :bordered="auditCnt.low > 0">低分 {{ auditCnt.low }}</NTag>
        <NText depth="3" style="font-size: 12px">
          已复核 {{ auditCnt.reviewed }}/{{ auditCnt.total }}
        </NText>
        <NButton size="tiny" quaternary @click="auditOpen = !auditOpen">
          {{ auditOpen ? "收起" : "展开" }}
        </NButton>
      </div>
      <div v-if="auditOpen" class="audit-list">
        <NCheckbox v-model:checked="showNormal" size="small" class="audit-show-normal">
          显示已复核与正常段
        </NCheckbox>
        <div
          v-for="row in visibleRows"
          :key="row.event.id"
          class="audit-row"
          :class="{ 'risk-a': row.risk === 'A' }"
        >
          <div
            class="audit-line"
            @click="toggleRow(row.event.id, row.risk === 'A')"
          >
            <span class="audit-time">{{ fmtTime(row.event.start) }}</span>
            <NTag v-if="row.risk === 'A'" type="warning" size="small">重点</NTag>
            <NTag v-else-if="row.risk === 'B'" size="small">
              低分 {{ row.meta.score.toFixed(2) }}
            </NTag>
            <NTag v-if="row.meta.reviewed" type="success" size="small">已复核</NTag>
            <span class="audit-text">{{ snippet(row.event.text) }}</span>
            <span class="audit-caret">{{ rowExpanded(row) ? "▾" : "▸" }}</span>
          </div>
          <div v-if="rowExpanded(row)" class="audit-detail">
            <div class="audit-cur">
              <NText depth="3" style="font-size: 12px">当前命中</NText>
              <span>
                {{
                  row.meta.ocr_index > 0
                    ? `#${row.meta.ocr_index} ${snippet(row.corpusText || corpusAt(row.meta.ocr_index))}`
                    : "不配（保留转写原文）"
                }}
              </span>
            </div>
            <div v-if="row.meta.candidates.length" class="audit-cands">
              <NText depth="3" style="font-size: 12px">候选名单（点击替换）</NText>
              <div class="audit-cand-btns">
                <NButton
                  v-for="c in row.meta.candidates"
                  :key="c.ocr_index"
                  size="tiny"
                  secondary
                  @click="decide(row.event.id, { kind: 'replace', ocrIndex: c.ocr_index })"
                >
                  #{{ c.ocr_index }} {{ snippet(corpusAt(c.ocr_index), 24) }}（{{ c.score.toFixed(2) }}）
                </NButton>
              </div>
            </div>
            <div class="audit-actions">
              <NButton size="tiny" @click="decide(row.event.id, { kind: 'keep' })">
                保留当前
              </NButton>
              <NButton size="tiny" @click="openPicker(row.event.id)">
                从全语料选择…
              </NButton>
              <NButton size="tiny" quaternary type="warning" @click="decide(row.event.id, { kind: 'unmatch' })">
                不配该条
              </NButton>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 全语料选择弹窗：display-directive=if 惰性渲染，大语料不常驻内存 -->
    <NModal
      v-model:show="pickerShow"
      display-directive="if"
      title="从语料中选择替换文本"
      style="width: 640px"
    >
      <div class="picker-body">
        <NInput v-model:value="pickerQuery" size="small" clearable placeholder="筛选语料" />
        <n-virtual-list
          class="picker-list"
          :items="pickerItems"
          :item-size="44"
          item-resizable
          key-field="index"
        >
          <template #default="{ item }">
            <div class="picker-row" @click="pickCorpus(item.index)">
              <span class="picker-idx">#{{ item.index }}</span>
              <span class="picker-text">{{ item.text }}</span>
            </div>
          </template>
        </n-virtual-list>
      </div>
    </NModal>
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

/* ── 融合审计表 ── */
.audit-section {
  margin-top: 20px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.audit-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.audit-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.audit-show-normal {
  font-size: 12px;
}

.audit-row {
  border: 1px solid var(--color-border, #efeff5);
  border-radius: 6px;
}

.audit-row.risk-a {
  border-color: var(--color-warning, #f0a020);
}

.audit-line {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  cursor: pointer;
  min-width: 0;
}

.audit-time {
  font-size: 12px;
  color: var(--color-text-secondary);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

.audit-text {
  font-size: 12px;
  color: var(--color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
  flex: 1;
}

.audit-caret {
  font-size: 11px;
  color: var(--color-text-secondary);
  flex-shrink: 0;
}

.audit-detail {
  padding: 8px 12px 10px;
  border-top: 1px dashed var(--color-border, #efeff5);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.audit-cur {
  display: flex;
  gap: 8px;
  align-items: baseline;
  font-size: 13px;
}

.audit-cands {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.audit-cand-btns {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.audit-actions {
  display: flex;
  gap: 8px;
}

.picker-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.picker-list {
  height: 420px;
}

.picker-row {
  display: flex;
  gap: 10px;
  align-items: baseline;
  padding: 8px 12px;
  cursor: pointer;
  border-radius: 4px;
}

.picker-row:hover {
  background: var(--color-bg-secondary);
}

.picker-idx {
  font-size: 12px;
  color: var(--color-text-secondary);
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}

.picker-text {
  font-size: 13px;
  min-width: 0;
}
</style>
