<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import { useProjectStore } from "../stores/project";
import SourceVideoPreview from "../components/SourceVideoPreview.vue";
import SourceTimeline from "../components/SourceTimeline.vue";
import ParamStepper from "../components/ParamStepper.vue";
import AppIcon from "../components/AppIcon.vue";
import type { CorpusItem, Diff, OcrRegionEvent } from "../types";
import { usePersistedOcrParams } from "../composables/ocrDefaults";
import { frameDurOf, hitsFrame } from "../composables/useFrameTolerance";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { NVirtualList, useMessage } from "naive-ui";

const projectStore = useProjectStore();
const message = useMessage();

// ── 三个语料来源折叠卡：一次只展开一张，默认全部收起 ──────────
type CardName = "recordings" | "screenshot" | "manual";
const openCard = ref<CardName | null>(null);

function toggleCard(name: CardName) {
  openCard.value = openCard.value === name ? null : name;
}

// ── "从剧情录屏中截取" ────────────────────────────────
// 跨栏目切换保留（此前是组件局部 ref ⇒ 切栏目即静默回到默认，见 ocrDefaults.ts）
const ocrParams = usePersistedOcrParams("corpus");

/// 术语表条目操作（可增删的 input-text 列表）
function addGlossaryTerm() {
  ocrParams.value.glossary.push("");
}

function removeGlossaryTerm(i: number) {
  ocrParams.value.glossary.splice(i, 1);
}

const sourceMeta = computed(() => projectStore.sourceVideoMeta);
const hasSourceVideo = computed(() => sourceMeta.value !== null);

const sourceFileName = computed(
  () => sourceMeta.value?.path.split(/[\\/]/).pop() ?? ""
);

function fmtDur(s: number): string {
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${String(sec).padStart(2, "0")}`;
}

/// 共享播放状态：视频预览 ↔ 迷你时间轴同步
const sourceTime = ref(0);
/// 剧情录屏时长取工程元数据（唯一来源）。此前靠预览组件的 loadedmetadata 回填，
/// 一旦预览不在渲染树里，迷你时间轴就没有时长（滚动/缩放/双击加选区全失效）。
const sourceDuration = computed(() => projectStore.sourceVideoMeta?.duration ?? 0);

/// 当前视频源的 OCR 选区控制轨（语料页加工的是剧情录屏，故 video === "source"）
const regionTrack = computed(() =>
  projectStore.currentProject?.tracks.find(
    (t) => t.type === "ocr_region" && t.video === "source"
  )
);

const regions = computed<OcrRegionEvent[]>(() =>
  (regionTrack.value?.events ?? []).filter(
    (e): e is OcrRegionEvent => e.type === "ocr_region"
  )
);

/// 播放头所在时间段对应的选区——OCR 实际识别的就是它
/// （命中判定留一帧容差，理由见 useFrameTolerance.ts）
const activeRegion = computed(
  () =>
    regions.value.find((e) =>
      hitsFrame(e.start, e.end, sourceTime.value, frameDurOf(projectStore.sourceVideoMeta))
    ) ?? null
);

const regionSize = computed(() =>
  activeRegion.value
    ? `${Math.round(activeRegion.value.x2 - activeRegion.value.x1)} × ${Math.round(activeRegion.value.y2 - activeRegion.value.y1)}`
    : "未设置"
);

const regionPos = computed(() =>
  activeRegion.value
    ? `(${Math.round(activeRegion.value.x1)}, ${Math.round(activeRegion.value.y1)})`
    : "—"
);

async function startOcr() {
  try {
    await projectStore.runOcr(ocrParams.value, "source");
    message.success("OCR 完成，文本已提取到语料");
  } catch (e) {
    message.error(String(e));
  }
}

// ── "从文本截图中截取" ────────────────────────────────
const selectedImages = ref<string[]>([]);

async function pickImages() {
  const selected = await open({
    multiple: true,
    title: "选择剧情文本截图",
    filters: [
      { name: "图片文件", extensions: ["png", "jpg", "jpeg", "webp", "bmp"] },
    ],
  });
  if (!selected) return;
  selectedImages.value = Array.isArray(selected) ? selected : [selected];
}

async function startImageOcr() {
  try {
    await projectStore.runCorpusImageOcr(selectedImages.value);
    message.success("OCR 完成，文本已提取到语料");
  } catch (e) {
    message.error(String(e));
  }
}

// ── "手动提供文本" ────────────────────────────────────
const manualText = ref("");

/// 实时解析预览：拆行 → trim → 丢空行（\r 随 trim 去除，兼容 Windows 换行）
const parsedLines = computed(() =>
  manualText.value
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
);

/// 预览行对象：供 n-virtual-list 使用（:items + key-field）。
/// id 用「行内容+序号」组合保证唯一：预览允许显示重复行（pushCorpusTexts 才去重），
/// 纯序号在中间插入行时会因 index 平移导致 key 错位，组合键能稳定匹配同一行。
const previewItems = computed(() =>
  parsedLines.value.map((text, i) => ({ id: `${i}-${text}`, text }))
);

/// 与 pushCorpusTexts 相同的去重规则预计数：新增条数 / 跳过重复条数
const previewStats = computed(() => {
  const existing = new Set(
    (projectStore.currentProject?.corpus ?? []).map((c) => c.text)
  );
  let fresh = 0;
  let dup = 0;
  for (const line of parsedLines.value) {
    if (existing.has(line)) {
      dup++;
    } else {
      existing.add(line);
      fresh++;
    }
  }
  return { fresh, dup };
});

async function importTxt() {
  const selected = await open({
    multiple: false,
    title: "选择文本文件",
    filters: [{ name: "文本文件", extensions: ["txt"] }],
  });
  if (!selected) return;
  try {
    manualText.value = await invoke<string>("read_text_file", {
      path: selected,
    });
  } catch (e) {
    message.error(String(e));
  }
}

function addManualCorpus() {
  const added = projectStore.pushCorpusTexts(parsedLines.value, "paste");
  message.success(`已添加 ${added} 条语料`);
  manualText.value = "";
}

// ── 语料列表 ──────────────────────────────────────────
const corpus = computed(() => projectStore.currentProject?.corpus ?? []);

const SOURCE_LABELS: Record<string, string> = {
  paste: "手动",
  image_ocr: "截图 OCR",
  ocr_track: "OCR 轨",
};

function sourceLabel(s: string): string {
  return SOURCE_LABELS[s] ?? s;
}

function removeItem(id: string) {
  projectStore.removeCorpusItem(id);
}

// ── 语料列表：行内编辑 / 手动新增 / 拖动排序 ────────────
const editingId = ref<string | null>(null);
const editText = ref("");
let editInputEl: HTMLInputElement | null = null;

/// 输入框挂上即聚焦并全选（编辑与新增共用）
function setEditInput(el: unknown) {
  editInputEl = (el as HTMLInputElement) ?? null;
  if (editInputEl) {
    editInputEl.focus();
    editInputEl.select();
  }
}

function startEdit(item: CorpusItem) {
  editingId.value = item.id;
  editText.value = item.text;
}

/// Ctrl/Cmd+Enter 或失焦提交；内容未变或 trim 后为空时 store 内部直接返回（不产生空撤销步骤）
function commitEdit() {
  if (!editingId.value) return;
  projectStore.updateCorpusItem(editingId.value, editText.value);
  editingId.value = null;
}

function cancelEdit() {
  editingId.value = null;
}

const drafting = ref(false);
const draftText = ref("");

function startDraft() {
  drafting.value = true;
  draftText.value = "";
}

function commitDraft() {
  const text = draftText.value.trim();
  if (text) projectStore.addCorpusItem(text);
  drafting.value = false;
  draftText.value = "";
}

function cancelDraft() {
  drafting.value = false;
  draftText.value = "";
}

// 拖动排序：用指针事件手写，不用 HTML5 拖放。
// 实测（临时工具 temp/css-preview/drag.mjs 用 CDP 可信鼠标事件驱动）：原生拖放里
// dragstart 正常触发、源行也能拿到 .dragging，但 dragover 从不下发给目标行、drop 永不触发
// ⇒ 拖完没有任何变化。指针事件同时能精确控制插入位置与指示线。
// 只有左侧点阵手柄按下才进入排序态（否则与文本选择冲突）。
const draggingId = ref<string | null>(null);
const dropAt = ref<{ index: number; before: boolean } | null>(null);
let reorderRows: { top: number; bottom: number; index: number }[] = [];

function rowGeom() {
  return Array.from(document.querySelectorAll<HTMLElement>(".clist .crow:not(.draft)")).map(
    (el, index) => {
      const r = el.getBoundingClientRect();
      return { top: r.top, bottom: r.bottom, index };
    }
  );
}

function onReorderMove(e: MouseEvent) {
  if (!draggingId.value) return;
  if (!reorderRows.length) reorderRows = rowGeom();
  const y = e.clientY;
  const last = reorderRows[reorderRows.length - 1];
  let index = last ? last.index : 0;
  let before = false;
  for (const r of reorderRows) {
    if (y < r.bottom) {
      index = r.index;
      before = y < (r.top + r.bottom) / 2;
      break;
    }
  }
  dropAt.value = { index, before };
}

/// 指针落点 → moveCorpusItem 需要的 to（「移除被拖项之后」的插入下标）
function resolveInsert(at: { index: number; before: boolean }): number | null {
  const from = corpus.value.findIndex((c) => c.id === draggingId.value);
  if (from < 0) return null;
  return at.before ? (at.index > from ? at.index - 1 : at.index) : at.index >= from ? at.index : at.index + 1;
}

function onReorderUp() {
  const id = draggingId.value;
  const at = dropAt.value;
  if (id && at) {
    const to = resolveInsert(at);
    if (to !== null) projectStore.moveCorpusItem(id, to);
  }
  endReorder();
}

function endReorder() {
  draggingId.value = null;
  dropAt.value = null;
  reorderRows = [];
  window.removeEventListener("mousemove", onReorderMove);
  window.removeEventListener("mouseup", onReorderUp);
}

function startReorder(e: MouseEvent, index: number) {
  e.preventDefault();
  const item = corpus.value[index];
  if (!item) return;
  draggingId.value = item.id;
  dropAt.value = { index, before: true };
  reorderRows = rowGeom();
  window.addEventListener("mousemove", onReorderMove);
  window.addEventListener("mouseup", onReorderUp);
}

onUnmounted(endReorder);

/// 行上的插入指示线类名（拖到自己身上时不显示）
function dropClass(ci: number, id: string): string {
  const at = dropAt.value;
  if (!at || draggingId.value === null || draggingId.value === id) return "";
  if (at.index !== ci) return "";
  return at.before ? "drop-before" : "drop-after";
}

// ── 待审批纠正 ────────────────────────────────────────
// 术语表纠错在 Rust 侧只标记不改写语料文本，命中项在此逐条由用户裁决
const pendingDiffs = computed(
  () => projectStore.currentProject?.corpus_ocr_diffs ?? []
);

async function approveDiff(d: Diff) {
  try {
    await projectStore.approveCorpusOcrDiff(d);
    message.success(`已采纳：${d.old.join(" / ")} → ${d.new}`);
  } catch (e) {
    message.error(`采纳失败：${e}`);
  }
}

function discardDiff(d: Diff) {
  projectStore.discardCorpusOcrDiff(d);
}
</script>

<template>
  <div class="page">
    <div class="phead">
      <h2>文本语料</h2>
      <p>从剧情录屏截取字幕区域 · 逐条核对后再交给后续步骤</p>
    </div>

    <div class="g2">
      <!-- ── 左列：三个语料来源 ─────────────────────────── -->
      <div class="l2">
        <!-- ① 从剧情录屏中截取 -->
        <div class="acc" :class="openCard === 'recordings' ? 'open' : 'off'">
          <div class="acch" @click="toggleCard('recordings')">
            从剧情录屏中截取
            <span class="ad">
              <AppIcon :name="openCard === 'recordings' ? 'chev-d' : 'chev-r'" :size="14" />
            </span>
          </div>
          <div v-if="openCard === 'recordings'" class="accb">
            <template v-if="!hasSourceVideo">
              <div class="frow">
                <button class="btn pri" @click="projectStore.importSourceVideo()">
                  导入剧情录屏
                </button>
                <span class="mut h11">支持 mp4 / mkv / webm / avi / mov / flv</span>
              </div>
            </template>

            <template v-else>
              <div class="frow">
                <button class="btn ghost" @click="projectStore.importSourceVideo()">
                  <AppIcon name="folder" :size="14" />更换剧情录屏
                </button>
                <span class="fname selectable" :title="sourceMeta?.path">{{ sourceFileName }}</span>
                <span class="sp"></span>
                <span class="mut h11" v-if="sourceMeta">
                  {{ sourceMeta.width }}×{{ sourceMeta.height }} · {{ fmtDur(sourceMeta.duration) }}
                </span>
              </div>

              <!-- 视频选区预览（独立于全局时间轴，播放时间与下方迷你时间轴同步） -->
              <div class="vrow">
                <div class="vid">
                  <SourceVideoPreview
                    v-model:time="sourceTime"
                    :duration="sourceDuration"
                    :show-header="false"
                  />
                </div>
                <div class="vside">
                  <div class="sblk">
                    <span class="bt">选区 · 播放头所在时间段</span>
                    <span class="kv"><span>尺寸</span><span>{{ regionSize }}</span></span>
                    <span class="kv"><span>左上角</span><span>{{ regionPos }}</span></span>
                    <span class="kv"><span>选区数</span><span>{{ regions.length }}</span></span>
                  </div>
                  <div class="sblk">
                    <span class="bt">术语表 · {{ ocrParams.glossary.length }} 条</span>
                    <span v-if="ocrParams.glossary.length === 0" class="gempty">
                      未配置。留空即关闭；填入正确词条（角色名/专有名词）后，识别结果中与之相近的文本会被自动纠正
                    </span>
                    <div
                      v-for="(_, gi) in ocrParams.glossary"
                      :key="gi"
                      class="glo"
                    >
                      <input
                        v-model="ocrParams.glossary[gi]"
                        class="glo-inp"
                        placeholder="正确词条，如 缟玛瑙"
                      />
                      <span class="gdel" @click="removeGlossaryTerm(gi)">
                        <AppIcon name="x" :size="11" />
                      </span>
                    </div>
                    <button class="btn ghost gadd" @click="addGlossaryTerm">
                      <AppIcon name="plus" :size="13" />添加
                    </button>
                  </div>
                </div>
              </div>

              <!-- 迷你时间轴：按时间段管理多条选区 -->
              <SourceTimeline
                v-model:time="sourceTime"
                :duration="sourceDuration"
                class="mini-timeline"
              />

              <!-- OCR 参数（无框步进：单击数值才出现输入框） -->
              <div class="agrid">
                <ParamStepper
                  v-model="ocrParams.frame_interval"
                  label="帧间隔（秒）"
                  :min="0.1"
                  :step="0.5"
                  :precision="1"
                />
                <ParamStepper
                  v-model="ocrParams.dhash_threshold"
                  label="变化检测阈值"
                  :min="0"
                  :max="64"
                  :step="1"
                />
                <ParamStepper
                  v-model="ocrParams.batch_size"
                  label="批大小"
                  :min="1"
                  :max="128"
                  :step="1"
                />
                <ParamStepper
                  v-model="ocrParams.merge_similarity"
                  label="合并相似度"
                  :min="0"
                  :max="1"
                  :step="0.05"
                  :precision="2"
                />
                <ParamStepper
                  v-model="ocrParams.min_subtitle_sec"
                  label="字幕预估最短长度（秒）"
                  :min="0"
                  :max="5"
                  :step="0.1"
                  :precision="1"
                />
                <ParamStepper
                  v-model="ocrParams.min_text_length"
                  label="单段最少实质字符数"
                  :min="0"
                  :max="10"
                  :step="1"
                />
              </div>

              <div class="arow">
                <button
                  class="btn pri"
                  :disabled="projectStore.ocrRunning"
                  @click="startOcr"
                >
                  {{ projectStore.ocrRunning && projectStore.ocrSource === "video" ? "OCR 运行中…" : "开始 OCR" }}
                </button>
                <template v-if="projectStore.ocrRunning && projectStore.ocrSource === 'video'">
                  <span class="hbar">
                    <i :style="{ width: Math.round(projectStore.ocrProgress * 100) + '%' }"></i>
                  </span>
                  <span class="mut h11">{{ Math.round(projectStore.ocrProgress * 100) }}% · {{ projectStore.ocrMessage }}</span>
                </template>
              </div>
            </template>
          </div>
        </div>

        <!-- ② 从文本截图中截取 -->
        <div class="acc" :class="openCard === 'screenshot' ? 'open' : 'off'">
          <div class="acch" @click="toggleCard('screenshot')">
            从文本截图中截取
            <span class="ad">
              <AppIcon :name="openCard === 'screenshot' ? 'chev-d' : 'chev-r'" :size="14" />
            </span>
          </div>
          <div v-if="openCard === 'screenshot'" class="accb">
            <div class="frow">
              <button class="btn ghost" @click="pickImages">
                <AppIcon name="folder" :size="14" />选择截图
              </button>
              <span class="mut h11">
                {{ selectedImages.length > 0 ? `已选 ${selectedImages.length} 张` : "支持 png / jpg / jpeg / webp / bmp" }}
              </span>
            </div>
            <div class="arow">
              <button
                class="btn pri"
                :disabled="selectedImages.length === 0 || projectStore.ocrRunning"
                @click="startImageOcr"
              >
                {{ projectStore.ocrRunning && projectStore.ocrSource === "image" ? "OCR 运行中…" : "开始 OCR" }}
              </button>
              <template v-if="projectStore.ocrRunning && projectStore.ocrSource === 'image'">
                <span class="hbar">
                  <i :style="{ width: Math.round(projectStore.ocrProgress * 100) + '%' }"></i>
                </span>
                <span class="mut h11">{{ Math.round(projectStore.ocrProgress * 100) }}% · {{ projectStore.ocrMessage }}</span>
              </template>
            </div>
          </div>
        </div>

        <!-- ③ 手动提供文本 -->
        <div class="acc" :class="openCard === 'manual' ? 'open' : 'off'">
          <div class="acch" @click="toggleCard('manual')">
            手动提供文本
            <span class="ad">
              <AppIcon :name="openCard === 'manual' ? 'chev-d' : 'chev-r'" :size="14" />
            </span>
          </div>
          <div v-if="openCard === 'manual'" class="accb">
            <textarea
              v-model="manualText"
              class="inp wta"
              rows="6"
              placeholder="粘贴游戏文本，每行一条；空行自动忽略，重复行自动跳过"
            ></textarea>

            <div class="frow">
              <button class="btn ghost" @click="importTxt">
                <AppIcon name="folder" :size="14" />导入 .txt
              </button>
              <span class="mut h11">仅支持 UTF-8 编码的 txt</span>
            </div>

            <template v-if="parsedLines.length > 0">
              <span class="mut h11">
                将新增 {{ previewStats.fresh }} 条<template v-if="previewStats.dup > 0">，跳过重复 {{ previewStats.dup }} 条</template>
              </span>
              <n-virtual-list
                class="preview-list"
                :items="previewItems"
                :item-size="28"
                item-resizable
                key-field="id"
              >
                <template #default="{ item }">
                  <div class="preview-item selectable">{{ item.text }}</div>
                </template>
              </n-virtual-list>
            </template>

            <div class="arow">
              <button
                class="btn pri"
                :disabled="previewStats.fresh === 0"
                @click="addManualCorpus"
              >
                添加到语料
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- ── 右列：语料列表 + 待审批纠正 ─────────────────── -->
      <div class="r2">
        <div class="rh">
          <span class="t">语料列表 · {{ corpus.length }} 条</span>
          <span class="sp"></span>
          <button class="btn ghost sm" @click="startDraft" title="手动添加一条语料">
            <AppIcon name="plus" :size="12" />添加
          </button>
        </div>

        <div v-if="pendingDiffs.length > 0" class="appr">
          <div class="apprh">
            <AppIcon name="fuse" :size="13" />待审批纠正 {{ pendingDiffs.length }}
            <span class="sp"></span>
            <span class="mut h11">点「采纳」写入语料</span>
          </div>
          <div v-for="(d, di) in pendingDiffs" :key="`${di}-${d.new}`" class="ccard">
            <span class="cn">{{ di + 1 }}</span>
            <span class="cm">
              <span class="co selectable" :title="d.old.join(' / ')">{{ d.old.join(" / ") }}</span>
              <span class="cnw selectable" :title="d.new">{{ d.new }}</span>
            </span>
            <span class="ca">
              <button class="btn pri sm" @click="approveDiff(d)">采纳</button>
              <button class="btn ghost sm" @click="discardDiff(d)">放弃</button>
            </span>
          </div>
        </div>

        <div class="clist" :class="{ reordering: draggingId !== null }">
          <div v-if="corpus.length === 0 && !drafting" class="cempty">
            暂无语料——展开左侧「从剧情录屏中截取」跑一次 OCR，文本会自动进入此列表
          </div>
          <div v-if="drafting" class="crow draft">
            <span class="cn">+</span>
            <input
              :ref="setEditInput"
              v-model="draftText"
              class="inp ct-edit"
              placeholder="输入文本，Ctrl+Enter 或失焦提交"
              @keydown.ctrl.enter.prevent="commitDraft"
              @keydown.meta.enter.prevent="commitDraft"
              @keydown.esc.prevent="cancelDraft"
              @blur="commitDraft"
            />
          </div>
          <div
            v-for="(item, ci) in corpus"
            :key="item.id"
            class="crow"
            :class="[dropClass(ci, item.id), { dragging: draggingId === item.id }]"
          >
            <span
              class="grip"
              title="按住拖动调整顺序（语料顺序影响融合取舍）"
              @mousedown="startReorder($event, ci)"
            >
              <svg class="grip-ic" viewBox="0 0 16 16" aria-hidden="true">
                <circle cx="6" cy="4" r="1.4" />
                <circle cx="10" cy="4" r="1.4" />
                <circle cx="6" cy="8" r="1.4" />
                <circle cx="10" cy="8" r="1.4" />
                <circle cx="6" cy="12" r="1.4" />
                <circle cx="10" cy="12" r="1.4" />
              </svg>
            </span>
            <span class="cn">{{ ci + 1 }}</span>
            <input
              v-if="editingId === item.id"
              :ref="setEditInput"
              v-model="editText"
              class="inp ct-edit"
              @keydown.ctrl.enter.prevent="commitEdit"
              @keydown.meta.enter.prevent="commitEdit"
              @keydown.esc.prevent="cancelEdit"
              @blur="commitEdit"
            />
            <span v-else class="ct selectable" :title="item.text" @dblclick="startEdit(item)">
              {{ item.text }}
            </span>
            <button
              v-if="editingId !== item.id"
              class="btn icb sm ghost"
              @click="startEdit(item)"
              title="编辑该条"
            >
              <AppIcon name="pencil" :size="12" />
            </button>
            <span class="psrc">{{ sourceLabel(item.source) }}</span>
            <button class="btn icb sm ghost" @click="removeItem(item.id)" title="删除该条">
              <AppIcon name="x" :size="12" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <div class="wfoot">素材需人工预校对，管线 raw 产物不等于最终字幕</div>
  </div>
</template>

<style scoped>
.page {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px 18px;
  overflow: hidden;
}

.g2 {
  flex: 1;
  min-height: 0;
  display: flex;
}

.l2 {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-right: 20px;
}

.r2 {
  flex: 0 0 38%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-left: 20px;
  border-left: 1px solid var(--hair);
}

.frow {
  flex: none;
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 32px;
}

.fname {
  min-width: 0;
  font: 11px/1 var(--mono);
  color: var(--dim);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.vrow {
  flex: none;
  display: flex;
  gap: 14px;
}

.vid {
  flex: none;
  height: 224px;
  aspect-ratio: 16 / 9;
  border: 1px solid var(--hair-2);
  border-radius: 8px;
  background: #000;
  overflow: hidden;
}

.vside {
  flex: 1;
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
  align-content: start;
}

.gempty {
  font-size: 11px;
  line-height: 1.6;
  color: var(--dim);
}

.glo {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--hair);
  border-radius: 6px;
  min-width: 0;
}

.glo-inp {
  flex: 1;
  min-width: 0;
  border: 0;
  background: none;
  color: var(--text);
  font: 11.5px/1 var(--sans);
  outline: none;
}

.glo-inp::placeholder {
  color: var(--faint);
}

.gdel {
  flex: none;
  display: flex;
  color: var(--faint);
  cursor: pointer;
}

.gdel:hover {
  color: var(--danger);
}

.gadd {
  align-self: flex-start;
}

.mini-timeline {
  flex: none;
}

.agrid {
  flex: none;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px 16px;
}

.arow {
  flex: none;
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 36px;
}

.arow .hbar {
  width: 170px;
}

/* 右列 */
.clist {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  overflow-x: hidden;
}

.cempty {
  padding: 20px 14px;
  font-size: 12px;
  line-height: 1.7;
  color: var(--dim);
}

.crow {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 30px;
  padding: 5px 10px;
  border-bottom: 1px solid var(--hair);
}

.crow:last-child {
  border-bottom: 0;
}

.crow:hover {
  background: var(--hover);
}

.crow .cn {
  width: 20px;
  flex: none;
  font: 11px/1 var(--mono);
  color: var(--faint);
}

.crow .ct {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 左侧点阵拖动手柄（2 列 × 3 行） */
.crow .grip {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  color: var(--faint);
  cursor: grab;
}

.crow .grip:hover {
  color: var(--dim);
}

.crow .grip:active {
  cursor: grabbing;
}

.grip-ic {
  width: 14px;
  height: 14px;
  fill: currentColor;
}

/* 行内编辑 / 新增输入框：占满文本位 */
.crow .ct-edit {
  flex: 1;
  min-width: 0;
  height: 24px;
  font-size: 12.5px;
}

.crow.draft {
  background: var(--sel-bg);
}

.crow.dragging {
  opacity: 0.45;
}

/* 插入指示线：用 inset 阴影画线，避免布局跳动 */
.crow.drop-before {
  box-shadow: inset 0 2px 0 var(--accent);
}

.crow.drop-after {
  box-shadow: inset 0 -2px 0 var(--accent);
}

/* 排序进行中：整列显示抓取光标（指针可能已离开手柄） */
.clist.reordering .crow,
.clist.reordering .grip {
  cursor: grabbing;
}

/* 待审批纠正 */
.appr {
  flex: none;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  overflow: hidden;
}

.apprh {
  display: flex;
  align-items: center;
  gap: 7px;
  height: 30px;
  padding: 0 10px;
  border-bottom: 1px solid var(--hair);
  font-size: 11.5px;
  color: var(--dim);
}

.ccard {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px;
  border-bottom: 1px solid var(--hair);
}

.ccard:last-child {
  border-bottom: 0;
}

.ccard .cn {
  flex: none;
  font: 11px/1 var(--mono);
  color: var(--faint);
}

.ccard .cm {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.ccard .co {
  font-size: 11.5px;
  color: var(--faint);
  text-decoration: line-through;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ccard .cnw {
  font-size: 11.5px;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ccard .ca {
  flex: none;
  display: flex;
  gap: 6px;
}

.wfoot {
  flex: none;
  margin-top: auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--hair);
  font-size: 11.5px;
  color: var(--faint);
}

/* 手动输入的文本域与预览列表（虚拟列表需确定高度的滚动容器） */
.wta {
  width: 100%;
}

.preview-list {
  height: 150px;
  overflow: auto;
  padding: 8px 12px;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
}

.preview-item {
  font-size: 12px;
  color: var(--text);
  word-break: break-all;
  /* 行间距用 padding：虚拟列表用 borderBoxSize 测量行高，margin 不计入会破坏定位 */
  padding: 4px 0;
  box-sizing: border-box;
}
</style>
