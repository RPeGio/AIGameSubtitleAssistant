<script setup lang="ts">
import { computed, onActivated, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";
import {
  auditCounts,
  sortRows,
  type FuseAuditDecision,
  type FuseAuditRow,
} from "../stores/fuseAudit";
import { NInput, NModal, NVirtualList, useMessage } from "naive-ui";
import AppIcon from "../components/AppIcon.vue";

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

/// 页面套了 KeepAlive：onActivated 首次挂载也会触发，所以每次回到本页都重探一次
/// （一次 invoke，成本可忽略），免得运行时状态变了卡片还显示旧结论。
async function probeEmbedRuntime() {
  try {
    embedStatus.value = await invoke("check_embed_runtime");
  } catch (e) {
    embedStatus.value = { ready: false, message: String(e) };
  }
}
onActivated(() => void probeEmbedRuntime());
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

const fusePercent = computed(() => Math.round(projectStore.llmProgress * 100));

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
    <div class="phead">
      <h2>AI 融合</h2>
      <p>用可靠的游戏文本替换转写文本，保留其时间轴（向量召回对齐）</p>
    </div>

    <!-- 两张就绪卡片：可点击跳转对应页（无论是否就绪） -->
    <div class="engcards">
      <button type="button" class="engc" :class="{ on: corpusReady }" @click="goCorpus">
        <span class="ec">
          <span class="en">语料</span>
          <span class="es">
            {{ corpusReady ? `已收集 ${corpusCount} 条可靠文本` : "尚未收集语料（点击前往语料页）" }}
          </span>
        </span>
        <span class="est">
          <span class="pill" :class="corpusReady ? 'ok' : ''">
            {{ corpusReady ? "就绪" : "未就绪" }}
          </span>
        </span>
      </button>

      <button type="button" class="engc" :class="{ on: timelineReady }" @click="goAsr">
        <span class="ec">
          <span class="en">时间轴</span>
          <span class="es">
            {{
              timelineReady
                ? `已生成 ${gameContentCount} 段游戏内容文本（语音 + 内嵌字幕）`
                : "尚无游戏内容：运行 ASR 或对内嵌字幕做 OCR（点击前往转写页）"
            }}
          </span>
        </span>
        <span class="est">
          <span class="pill" :class="timelineReady ? 'ok' : ''">
            {{ timelineReady ? "就绪" : "未就绪" }}
          </span>
        </span>
      </button>
    </div>

    <!-- 融合操作 -->
    <button
      type="button"
      class="chk"
      role="checkbox"
      :aria-checked="unmatchFilter ? 'true' : 'false'"
      @click="unmatchFilter = !unmatchFilter"
    >
      <i :class="{ on: unmatchFilter }"></i>启用「该不配」掩码
    </button>
    <div class="enote">
      显示语音比语料语言更省内容的素材才适用；误开会把本应命中的段判为不配
    </div>

    <div class="arow">
      <button
        type="button"
        class="btn pri"
        :disabled="!canFuse || projectStore.fuseRunning"
        @click="startFuse"
      >
        <AppIcon name="fuse" :size="14" />
        {{ projectStore.fuseRunning ? "融合运行中…" : "开始融合" }}
      </button>
      <span class="sp"></span>
      <span
        v-if="!canFuse && !projectStore.fuseRunning"
        class="enote"
      >
        {{
          embedStatus && !embedStatus.ready
            ? `向量运行时未就绪：${embedStatus.message}`
            : "需语料与时间轴均就绪（点击上方卡片）"
        }}
      </span>
    </div>

    <div v-if="projectStore.fuseRunning" class="arow">
      <div class="hbar"><i :style="{ width: fusePercent + '%' }"></i></div>
      <span class="mut h11 nowrap">{{ fusePercent }}% · {{ projectStore.llmMessage }}</span>
    </div>

    <div v-else-if="fuseResult" class="resbox">
      <span class="pill ok">匹配 {{ fuseResult.matched }}/{{ fuseResult.total }} 段</span>
      <span v-if="fuseResult.masked > 0" class="enote">
        {{ fuseResult.masked }} 段被「该不配」掩码判为不配，保留转写原文
      </span>
      <span v-if="fuseResult.reused > 0" class="enote">
        {{ fuseResult.reused }} 条语料行被多段命中（候选重要提示，见审计表）
      </span>
    </div>

    <!-- 融合审计表：裁决随 .gsa 持久化，重开工程不丢 -->
    <div v-if="auditCnt.total > 0" class="aud">
      <div class="aud-h">
        <span class="t">融合审计</span>
        <span class="pill" :class="auditCnt.focused > 0 ? 'warn' : 'ok'">
          重点 {{ auditCnt.focused }}
        </span>
        <span class="pill">低分 {{ auditCnt.low }}</span>
        <span class="k">已复核 {{ auditCnt.reviewed }}/{{ auditCnt.total }}</span>
        <span class="sp"></span>
        <button
          type="button"
          class="chk"
          role="checkbox"
          :aria-checked="showNormal ? 'true' : 'false'"
          @click="showNormal = !showNormal"
        >
          <i :class="{ on: showNormal }"></i>显示已复核与正常段
        </button>
        <button type="button" class="btn ghost sm" @click="auditOpen = !auditOpen">
          {{ auditOpen ? "收起" : "展开" }}
        </button>
      </div>

      <div v-if="auditOpen" class="audlist">
        <div
          v-for="row in visibleRows"
          :key="row.event.id"
          class="arow2"
          :class="{ tall: rowExpanded(row) }"
        >
          <div class="ahd" @click="toggleRow(row.event.id, row.risk === 'A')">
            <span class="at">{{ fmtTime(row.event.start) }}</span>
            <span v-if="row.risk === 'A'" class="pill warn">重点</span>
            <span v-else-if="row.risk === 'B'" class="pill">
              低分 {{ row.meta.score.toFixed(2) }}
            </span>
            <span v-if="row.meta.reviewed" class="pill ok">已复核</span>
            <span class="ax">{{ snippet(row.event.text) }}</span>
            <span class="aa">
              <AppIcon :name="rowExpanded(row) ? 'chev-d' : 'chev-r'" :size="14" />
            </span>
          </div>

          <div v-if="rowExpanded(row)" class="abody">
            <div class="amap">
              <span class="atag">嵌字</span>
              <span class="aen">
                {{ snippet(row.meta.source_text || "（旧版融合产物，无转写原文）") }}
              </span>
              <span>→</span>
              <span class="atag">语料</span>
              <span v-if="row.meta.ocr_index > 0">
                #{{ row.meta.ocr_index }}
                {{ snippet(row.corpusText || corpusAt(row.meta.ocr_index)) }}
              </span>
              <span v-else class="pill">不配</span>
            </div>

            <template v-if="row.meta.candidates.length">
              <span class="enote">候选名单（点击替换）</span>
              <div class="acands">
                <button
                  v-for="c in row.meta.candidates"
                  :key="c.ocr_index"
                  type="button"
                  class="acand"
                  @click="decide(row.event.id, { kind: 'replace', ocrIndex: c.ocr_index })"
                >
                  #{{ c.ocr_index }} {{ snippet(corpusAt(c.ocr_index), 24) }}（{{
                    c.score.toFixed(2)
                  }}）
                </button>
              </div>
            </template>

            <div class="aacts">
              <button
                type="button"
                class="btn ghost sm"
                @click="decide(row.event.id, { kind: 'keep' })"
              >
                保留当前
              </button>
              <button
                type="button"
                class="btn ghost sm"
                @click="openPicker(row.event.id)"
              >
                从全语料选择…
              </button>
              <button
                type="button"
                class="btn ghost sm warn"
                @click="decide(row.event.id, { kind: 'unmatch' })"
              >
                不配该条
              </button>
            </div>
          </div>
        </div>

        <div v-if="!visibleRows.length" class="aempty">
          没有需要复核的段落（可勾选上方「显示已复核与正常段」）
        </div>
      </div>
    </div>

    <!-- 全语料选择弹窗：preset=card 提供卡片底；display-directive=if 惰性渲染，大语料不常驻内存 -->
    <NModal
      v-model:show="pickerShow"
      preset="card"
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
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px 18px;
  overflow: hidden;
}

/* 就绪卡片：整张可点，进对应页去准备 */
.engcards {
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.engc {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  color: var(--text);
  font-family: inherit;
  text-align: left;
  cursor: pointer;
}

.engc:hover { border-color: var(--hair-2); }
.engc.on { border-color: var(--accent); background: var(--accent-soft); }

.engc .ec { min-width: 0; }
.engc .en { display: block; font-size: 13px; font-weight: 600; letter-spacing: -.01em; }
.engc .es { display: block; margin-top: 3px; font-size: 11.5px; color: var(--dim); }
.engc .est { margin-left: auto; flex: none; }

.arow { flex: none; display: flex; align-items: center; gap: 10px; }
.arow .hbar { flex: 1; }
.nowrap { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

.resbox { flex: none; display: flex; flex-direction: column; gap: 4px; align-items: flex-start; }

/* 审计表：卡体吃满剩余高度，内部滚动 */
.aud {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  overflow: hidden;
}

.aud-h {
  flex: none;
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
  padding: 7px 10px;
  border-bottom: 1px solid var(--hair);
}

.aud-h .t { font-size: 12px; font-weight: 600; letter-spacing: .08em; color: var(--dim); }
.aud-h .k { font-size: 11px; color: var(--faint); }

.audlist { flex: 1; min-height: 0; overflow-y: auto; }

.arow2 { border-bottom: 1px solid var(--hair); }
.arow2:last-child { border-bottom: 0; }
.arow2.tall { background: var(--sel-bg); border-left: 3px solid var(--accent); }

.ahd {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  min-width: 0;
  cursor: pointer;
}

.ahd:hover { background: var(--hover); }
.ahd .at { flex: none; font: 11px/1 var(--mono); color: var(--dim); }
.ahd .ax {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.ahd .aa { flex: none; display: flex; color: var(--faint); }

.abody {
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding: 7px 10px 8px;
  border-top: 1px solid var(--hair);
}

.amap {
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
  font-size: 11.5px;
  color: var(--dim);
}

.atag {
  flex: none;
  padding: 2px 4px;
  border: 1px solid var(--hair-2);
  border-radius: 3px;
  font: 10.5px/1 var(--mono);
  color: var(--faint);
}

.aen { font: 11.5px/1.4 var(--mono); color: var(--dim); }

.acands { display: flex; flex-direction: column; gap: 5px; }

.acand {
  padding: 5px 8px;
  border: 1px solid var(--hair-2);
  border-radius: 6px;
  background: none;
  color: var(--text);
  font-family: inherit;
  font-size: 11.5px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}

.acand:hover { border-color: var(--accent); background: var(--raised); }
.aacts { display: flex; gap: 8px; flex-wrap: wrap; }
.aempty { padding: 14px 10px; font-size: 11.5px; color: var(--faint); }

/* 全语料选择弹窗（naive 负责卡片与虚拟列表，行样式对齐骨架） */
.picker-body { display: flex; flex-direction: column; gap: 10px; }
.picker-list { height: 360px; }

.picker-row {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 44px;
  padding: 0 6px;
  border-radius: 6px;
  cursor: pointer;
}

.picker-row:hover { background: var(--hover); }
.picker-idx { flex: none; font: 11px/1 var(--mono); color: var(--faint); }
.picker-text {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
