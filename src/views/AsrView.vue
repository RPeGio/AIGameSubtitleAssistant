<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useProjectStore } from "../stores/project";
import { useTimelineStore } from "../stores/timeline";
import SourceVideoPreview from "../components/SourceVideoPreview.vue";
import SourceTimeline from "../components/SourceTimeline.vue";
import AppIcon from "../components/AppIcon.vue";
import ParamStepper from "../components/ParamStepper.vue";
import type { AsrRunParams, AsrEngineStatus } from "../types";
import { usePersistedOcrParams } from "../composables/ocrDefaults";
import { useMessage } from "naive-ui";

const projectStore = useProjectStore();
/// 全局时间轴 store（校对区同一个实例）：打轴页的播放头挂在它上面
const timeline = useTimelineStore();
const message = useMessage();

// ── 两张折叠卡的展开态（默认全部收起，一次只开一张最大利用高度）──
type CardName = "visual" | "audio";
const openCard = ref<CardName | null>(null);

function toggleCard(name: CardName) {
  openCard.value = openCard.value === name ? null : name;
}

// ── "音频获取时间轴"（ASR）─────────────────────────────
const asrParams = ref<AsrRunParams>({
  engine: "moss",
  max_speakers: null,
  language: null,
});

/// 引擎展示顺序：MOSS 为主力（GPU 更快、说话人分离更好），FunASR 仅作 fallback
const ENGINE_ORDER = ["moss", "funasr"] as const;

const ENGINE_INFO: Record<string, { title: string; sub: string }> = {
  moss: {
    title: "MOSS-Transcribe-Diarize",
    sub: "推荐 · GPU 更快 · 说话人分离更好",
  },
  funasr: {
    title: "FunASR + diarize",
    sub: "备用 fallback · 较慢、说话人分离较弱",
  },
};

/// 两引擎运行环境（展开时探测，禁用不可用引擎）
const asrEngines = ref<AsrEngineStatus[]>([]);

/// 探测失败（asrEngines 为空）时直通：不阻塞用户，运行失败由错误提示兜底
function probeFailed(): boolean {
  return asrEngines.value.length === 0;
}

function engineStatus(engine: string): AsrEngineStatus | undefined {
  return asrEngines.value.find((e) => e.engine === engine);
}

function engineReady(engine: string): boolean {
  if (probeFailed()) return true;
  return engineStatus(engine)?.ready ?? false;
}

function engineStateText(engine: string): string {
  const s = engineStatus(engine);
  if (!s) return probeFailed() ? "状态未知" : "探测中";
  return s.ready ? "就绪" : "未就绪";
}

function engineStateTitle(engine: string): string {
  const s = engineStatus(engine);
  if (!s) {
    return probeFailed() ? "探测失败，可直接尝试运行" : "正在探测运行环境";
  }
  return s.ready ? "运行环境就绪" : s.message || "未配置";
}

const maxSpeakerOptions = [2, 3, 4, 5, 6];

const languageOptions = [
  { label: "自动检测", value: "auto" },
  { label: "中文", value: "zh" },
  { label: "English", value: "en" },
  { label: "日本語", value: "ja" },
];

/// MOSS 自动估计说话人 / 自动识别多语言，两个参数对它无意义
const mossOnly = computed(() => asrParams.value.engine === "moss");

function onMaxSpeakers(e: Event) {
  const raw = (e.target as HTMLSelectElement).value;
  asrParams.value.max_speakers = raw === "auto" ? null : Number(raw);
}

function onLanguage(e: Event) {
  const raw = (e.target as HTMLSelectElement).value;
  asrParams.value.language = raw === "auto" ? null : raw;
}

async function openAsrPanel() {
  try {
    asrEngines.value = await projectStore.getAsrEngines();
  } catch {
    asrEngines.value = [];
  }
}

// 展开 ASR 卡时探测一次引擎状态
watch(openCard, (v) => {
  if (v === "audio") void openAsrPanel();
});

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

// ── ASR 结果说话人标记 ────────────────────────────────
/// 全部 asr 轨（仅显示非空轨道，按说话人名排序）
const asrTracks = computed(() => {
  const tracks = projectStore.currentProject?.tracks ?? [];
  return tracks
    .filter((t) => t.type === "asr" && t.events.length > 0)
    .sort((a, b) => a.name.localeCompare(b.name));
});

/// 标记轨道角色（主播语音 / 游戏内容），"选择即生效"：
/// 融合只消费游戏内容轨，主播轨仅供字幕轴参考
function setTrackRole(trackId: string, role: string) {
  projectStore.updateTrackRole(trackId, role);
}

// ── "检测画面变化获取时间轴"（内嵌字幕 OCR）────────────
/// 针对没有配音的游戏任务：OCR 切片视频画面里的游戏内嵌字幕，
/// 产出带时间轴的 embed_ocr 轨（嵌字轴），作为"游戏内容"源供融合。
// 术语表是**语料 OCR 独有**功能：嵌字产物进 embed_ocr 轨（供融合做游戏内容段），
// 不落 corpus，也就没有"待审批纠正"的去处，故保持共享工厂的空数组缺省（后端 serde default 同义）
// 跨栏目切换保留（此前是组件局部 ref ⇒ 切栏目即静默回到默认，见 ocrDefaults.ts）
const ocrParams = usePersistedOcrParams("asr");

const hasClipVideo = computed(() => projectStore.currentVideoMeta !== null);

/// 播放头与校对区共用：预览与迷你时间轴直接读写全局时间轴时间（双向），
/// 不再各持一份进度——否则同一支切片视频两处播放头会各走各的。
/// 注意用直接赋值而非 timeline.seek()：seek() 会置 isSeeking，校对区播放器的
/// tick 会被它挡掉，而迷你时间轴没有配对的 seekDone()。
const clipTime = computed({
  get: () => timeline.currentTime,
  set: (t: number) => {
    timeline.currentTime = t;
  },
});
const clipDuration = ref(0);

const clipFileName = computed(() => {
  const p = projectStore.currentProject?.video ?? "";
  return p.split(/[\\/]/).pop() || p;
});

const clipMeta = computed(() => {
  const m = projectStore.currentVideoMeta;
  if (!m) return "";
  return `${m.width}×${m.height} · ${fmtDur(m.duration)}`;
});

/// 切片视频时长就绪后确保嵌字选区控制轨存在（page=asr，与编辑页选区并存）
watch(
  () => projectStore.currentVideoMeta?.duration ?? 0,
  (d) => {
    if (d > 0) projectStore.ensureAsrRegionTrack(d);
  },
  { immediate: true }
);

function fmtDur(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function pct(v: number): string {
  return `${Math.round(v * 100)}%`;
}

async function startEmbedOcr() {
  try {
    await projectStore.runOcr(ocrParams.value, "clip", { regionPage: "asr" });
    message.success("嵌字 OCR 完成，文本已写入内嵌字幕轨道");
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <div class="page">
    <div class="phead">
      <h2>打轴 + 转写</h2>
      <p>用 ASR 生成说话人轨道；没有配音的任务改用画面变化 + 内嵌字幕 OCR 取轴</p>
    </div>

    <!-- ① 检测画面变化获取时间轴（内嵌字幕 OCR） -->
    <div class="acc" :class="openCard === 'visual' ? 'open' : 'off'">
      <div class="acch" @click="toggleCard('visual')">
        检测画面变化获取时间轴（内嵌字幕 OCR）
        <span class="ad">
          <AppIcon :name="openCard === 'visual' ? 'chev-d' : 'chev-r'" :size="14" />
        </span>
      </div>
      <div v-if="openCard === 'visual'" class="accb">
        <span class="enote">
          针对没有配音的游戏任务：框选切片视频画面中的游戏字幕区域，OCR 识别内嵌字幕，
          产出带时间轴的嵌字轨道，供 AI 融合作为游戏内容
        </span>

        <template v-if="!hasClipVideo">
          <div class="arow">
            <button class="btn pri" @click="projectStore.importVideo()">导入切片视频</button>
            <span class="enote">支持 mp4 / mkv / webm / avi / mov / flv</span>
          </div>
        </template>

        <template v-else>
          <div class="frow">
            <button class="btn ghost" @click="projectStore.importVideo()">
              <AppIcon name="folder" :size="14" />选择切片视频
            </button>
            <span class="fname">{{ clipFileName }}</span>
            <span class="sp"></span>
            <span class="enote">{{ clipMeta }}</span>
          </div>

          <div class="vrow">
            <div class="vid">
              <SourceVideoPreview
                video-key="clip"
                v-model:time="clipTime"
                v-model:duration="clipDuration"
                :show-header="false"
                sync-playback
              />
            </div>
          </div>

          <SourceTimeline
            video-key="clip"
            v-model:time="clipTime"
            :duration="clipDuration"
            class="mini-timeline"
          />

          <div class="agrid">
            <ParamStepper
              v-model="ocrParams.frame_interval"
              label="帧间隔（秒）"
              :step="0.5"
              :min="0.1"
              :precision="1"
            />
            <ParamStepper
              v-model="ocrParams.dhash_threshold"
              label="变化检测阈值"
              :min="0"
              :max="64"
            />
            <ParamStepper v-model="ocrParams.batch_size" label="批大小" :min="1" :max="128" />
            <ParamStepper
              v-model="ocrParams.merge_similarity"
              label="合并相似度"
              :step="0.05"
              :min="0"
              :max="1"
              :precision="2"
            />
            <ParamStepper
              v-model="ocrParams.min_subtitle_sec"
              label="字幕预估最短长度（秒）"
              :step="0.1"
              :min="0"
              :max="5"
              :precision="1"
            />
            <ParamStepper
              v-model="ocrParams.min_text_length"
              label="单段最少实质字符数"
              :min="0"
              :max="10"
            />
          </div>

          <span class="enote">
            合并相似度越大越易合并同句；单段最少实质字符数 0 关闭过滤。OCR 按播放头所在时间段对应的选区识别
          </span>

          <div class="arow arow-bottom">
            <button class="btn pri" :disabled="projectStore.ocrRunning" @click="startEmbedOcr">
              {{ projectStore.ocrRunning ? "OCR 运行中…" : "开始识别" }}
            </button>
            <template v-if="projectStore.ocrRunning">
              <span class="hbar"><i :style="{ width: pct(projectStore.ocrProgress) }"></i></span>
              <span class="enote">
                {{ pct(projectStore.ocrProgress) }} · {{ projectStore.ocrMessage }}
              </span>
            </template>
          </div>
        </template>
      </div>
    </div>

    <!-- ② 音频获取时间轴（ASR） -->
    <div class="acc" :class="openCard === 'audio' ? 'open' : 'off'">
      <div class="acch" @click="toggleCard('audio')">
        音频获取时间轴
        <span class="ad">
          <AppIcon :name="openCard === 'audio' ? 'chev-d' : 'chev-r'" :size="14" />
        </span>
      </div>
      <div v-if="openCard === 'audio'" class="accb">
        <span class="enote">
          对切片视频做语音识别 + 说话人分离，产出带说话人标签的时间轴与转录文本
        </span>

        <div class="sblk">
          <span class="bt">转写引擎</span>
          <div class="eng">
            <div
              v-for="key in ENGINE_ORDER"
              :key="key"
              class="erow"
              :class="{ on: asrParams.engine === key, dis: !engineReady(key) }"
              :title="engineStateTitle(key)"
              @click="engineReady(key) && (asrParams.engine = key)"
            >
              <span class="erdot"></span>
              <span class="en">
                <span class="t">{{ ENGINE_INFO[key].title }}</span>
                <span class="s">{{ ENGINE_INFO[key].sub }}</span>
              </span>
              <span class="psrc es">{{ engineStateText(key) }}</span>
            </div>
          </div>
        </div>

        <div class="sblk">
          <span class="bt">转写参数</span>
          <div class="prow">
            <div class="pcell">
              <span class="pl">最大说话人数量</span>
              <span class="selwrap">
                <select
                  class="selbox"
                  :value="asrParams.max_speakers ?? 'auto'"
                  :disabled="mossOnly"
                  @change="onMaxSpeakers"
                >
                  <option value="auto">自动估计</option>
                  <option v-for="n in maxSpeakerOptions" :key="n" :value="n">
                    {{ n }} 人
                  </option>
                </select>
                <AppIcon name="chev-d" :size="11" />
              </span>
            </div>
            <div class="pcell">
              <span class="pl">识别语言</span>
              <span class="selwrap">
                <select
                  class="selbox"
                  :value="asrParams.language ?? 'auto'"
                  :disabled="mossOnly"
                  @change="onLanguage"
                >
                  <option v-for="o in languageOptions" :key="o.value" :value="o.value">
                    {{ o.label }}
                  </option>
                </select>
                <AppIcon name="chev-d" :size="11" />
              </span>
            </div>
          </div>
          <span v-if="mossOnly" class="enote">
            MOSS 自动估计说话人并自动识别多语言，无需指定；切到 FunASR 才需要手动设
          </span>
        </div>

        <!-- 说话人标记：每条 ASR 轨标记为主播语音 / 游戏内容（"选择即生效"）。
             融合只消费游戏内容轨（嵌字轴），主播轨仅供字幕轴参考 -->
        <div v-if="asrTracks.length > 0" class="sblk">
          <span class="bt">说话人标记 · 标记为「游戏内容」的轨道参与 AI 融合</span>
          <div v-for="track in asrTracks" :key="track.id" class="trow">
            <span class="ln">{{ track.name }}</span>
            <span class="enote">{{ track.events.length }} 段</span>
            <span class="sp"></span>
            <button
              class="btn sm"
              :class="{ on: track.track_role === 'game' }"
              @click="setTrackRole(track.id, 'game')"
            >
              游戏内容
            </button>
            <button
              class="btn sm"
              :class="{ on: track.track_role === 'streamer' }"
              @click="setTrackRole(track.id, 'streamer')"
            >
              主播语音
            </button>
          </div>
        </div>

        <div class="arow arow-bottom">
          <button
            class="btn pri"
            :disabled="projectStore.asrRunning || !engineReady(asrParams.engine)"
            @click="startAsr"
          >
            {{ projectStore.asrRunning ? "ASR 运行中…" : "开始 ASR" }}
          </button>
          <template v-if="projectStore.asrRunning">
            <span class="hbar"><i :style="{ width: pct(projectStore.asrProgress) }"></i></span>
            <span class="enote">
              {{ pct(projectStore.asrProgress) }} · {{ projectStore.asrMessage }}
            </span>
            <button class="btn ghost sm" @click="cancelAsr">取消</button>
          </template>
          <span class="sp"></span>
          <button class="btn ghost sm" @click="openAsrPanel">重新探测引擎状态</button>
        </div>
      </div>
    </div>

    <div class="wfoot">
      输出轨道：S01 游戏内容 · S02 主播语音（嵌字 OCR 产物进「内嵌字幕」轨）
    </div>
  </div>
</template>

<style scoped>
.page {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 16px 18px;
  overflow: hidden;
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

/* 文件行右侧的尺寸/时长不许折行——窄列里折行会把文件行撑成两行 */
.frow .enote {
  flex: none;
  white-space: nowrap;
}

.vrow {
  flex: none;
  display: flex;
  gap: 14px;
}

.vid {
  flex: none;
  height: 208px;
  aspect-ratio: 16 / 9;
  border: 1px solid var(--hair-2);
  border-radius: 8px;
  background: #000;
  overflow: hidden;
}

.mini-timeline {
  flex: none;
  margin-top: 2px;
}

.agrid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px 16px;
  align-content: start;
}

.prow {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px 14px;
}

.trow {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 26px;
}

.trow .ln {
  font-size: 12.5px;
  font-weight: 600;
}

.arow {
  flex: none;
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 34px;
}

.arow .hbar {
  width: 170px;
}

/* 卡体撑满时把主操作压到卡底，避免动作行悬在中间留一片空白 */
.arow-bottom {
  margin-top: auto;
}

.wfoot {
  flex: none;
  padding-top: 2px;
  font-size: 11px;
  color: var(--faint);
}
</style>
