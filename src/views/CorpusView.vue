<script setup lang="ts">
import { computed, ref } from "vue";
import { useProjectStore } from "../stores/project";
import SourceVideoPreview from "../components/SourceVideoPreview.vue";
import SourceTimeline from "../components/SourceTimeline.vue";
import ParamStepper from "../components/ParamStepper.vue";
import AppIcon from "../components/AppIcon.vue";
import type { Diff, OcrRegionEvent } from "../types";
import { usePersistedOcrParams } from "../composables/ocrDefaults";
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
const activeRegion = computed(
  () =>
    regions.value.find((e) => e.start <= sourceTime.value && sourceTime.value < e.end) ??
    null
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
                <span class="fname" :title="sourceMeta?.path">{{ sourceFileName }}</span>
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
                  <div class="preview-item">{{ item.text }}</div>
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
        <div class="rh"><span class="t">语料列表 · {{ corpus.length }} 条</span></div>

        <div v-if="pendingDiffs.length > 0" class="appr">
          <div class="apprh">
            <AppIcon name="fuse" :size="13" />待审批纠正 {{ pendingDiffs.length }}
            <span class="sp"></span>
            <span class="mut h11">点「采纳」写入语料</span>
          </div>
          <div v-for="(d, di) in pendingDiffs" :key="`${di}-${d.new}`" class="ccard">
            <span class="cn">{{ di + 1 }}</span>
            <span class="cm">
              <span class="co" :title="d.old.join(' / ')">{{ d.old.join(" / ") }}</span>
              <span class="cnw" :title="d.new">{{ d.new }}</span>
            </span>
            <span class="ca">
              <button class="btn pri sm" @click="approveDiff(d)">采纳</button>
              <button class="btn ghost sm" @click="discardDiff(d)">放弃</button>
            </span>
          </div>
        </div>

        <div class="clist">
          <div v-if="corpus.length === 0" class="cempty">
            暂无语料——展开左侧「从剧情录屏中截取」跑一次 OCR，文本会自动进入此列表
          </div>
          <div v-for="(item, ci) in corpus" :key="item.id" class="crow">
            <span class="cn">{{ ci + 1 }}</span>
            <span class="ct" :title="item.text">{{ item.text }}</span>
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
