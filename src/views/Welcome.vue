<script setup lang="ts">
import { ref, computed, onMounted, nextTick } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import type { RecentProject, MossInstallStatus } from "../types";
import AppIcon from "../components/AppIcon.vue";
import { NCard, NCheckbox, NInput, NModal, NPopconfirm, useMessage } from "naive-ui";

const router = useRouter();
const projectStore = useProjectStore();
const ui = useUiStore();
const message = useMessage();

const showCreateModal = ref(false);
const newProjectName = ref("");
const newProjectPath = ref("");

async function handleSelectFolder() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择项目文件夹",
  });
  if (selected) {
    newProjectPath.value = selected;
  }
}

async function handleCreateProject() {
  if (!newProjectName.value || !newProjectPath.value) return;
  const project = await projectStore.createProject(
    newProjectName.value,
    newProjectPath.value
  );
  showCreateModal.value = false;
  resetForm();
  router.push(`/project/${encodeURIComponent(project.path)}/editor`);
}

function resetForm() {
  newProjectName.value = "";
  newProjectPath.value = "";
}

async function handleOpenProject(path: string) {
  try {
    await projectStore.openProject(path);
  } catch (e) {
    message.error(`打开项目失败：${e}`);
    return;
  }
  router.push(`/project/${encodeURIComponent(path)}/editor`);
}

/// 打开已有项目：选择 .gsa 项目文件（项目身份 = 文件，同目录可有多个项目）
async function handleOpenProjectDialog() {
  const selected = await open({
    multiple: false,
    title: "打开 GSA 项目",
    filters: [{ name: "GSA 项目", extensions: ["gsa"] }],
  });
  if (!selected) return;
  await handleOpenProject(selected);
}

function isValidPath(input: string) {
  return input.length > 0;
}

// 项目名即项目文件名（<项目名>.gsa），Windows 非法文件名字符直接拦在前端
const ILLEGAL_NAME_RE = /[\\/:*?"<>|\x00-\x1f]/;
const nameIllegal = computed(() => ILLEGAL_NAME_RE.test(newProjectName.value));

// 后端 updated_at 是 Unix 秒字符串 → "YYYY-MM-DD HH:mm"
function formatUpdatedAt(unixSecs: string): string {
  const date = new Date(Number(unixSecs) * 1000);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

// ── 行内重命名（复用轨道重命名交互：按钮触发，Enter/失焦提交、Esc 取消）──
const renamingPath = ref<string | null>(null);
const renameDraft = ref("");
const renameInput = ref<HTMLInputElement | null>(null);

/// v-for 内的模板 ref 会被收集为数组，focus 会失败；用函数 ref 绑定当前渲染的输入框
function setRenameInput(el: unknown) {
  renameInput.value = (el as HTMLInputElement) ?? null;
}

function startRename(proj: RecentProject) {
  if (renamingPath.value) return;
  renamingPath.value = proj.path;
  renameDraft.value = proj.name;
  nextTick(() => {
    renameInput.value?.focus();
    renameInput.value?.select();
  });
}

// 失焦提交必然伴随一次落点点击：记录刚提交的行，该次点击不触发"打开项目"
let justCommittedPath: string | null = null;

async function commitRename() {
  const path = renamingPath.value;
  if (!path) return;
  const name = renameDraft.value.trim();
  const proj = projectStore.recentProjects.find((p) => p.path === path);
  if (name && name !== proj?.name && ILLEGAL_NAME_RE.test(name)) {
    message.error("项目名含非法字符（\\ / : * ? \" < > |），无法用作项目文件名");
    renameInput.value?.focus();
    return;
  }
  renamingPath.value = null;
  // 名字未变或为空：视为取消，不发请求
  if (!name || name === proj?.name) return;
  try {
    await projectStore.renameProject(path, name);
  } catch (e) {
    message.error(`重命名失败：${e}`);
  }
}

function cancelRename() {
  renamingPath.value = null;
}

/// 重命名中的行：点击行内非输入框区域即提交，且该次点击不触发打开项目
function onRowMouseDown(proj: RecentProject, e: MouseEvent) {
  if (renamingPath.value !== proj.path) return;
  if ((e.target as HTMLElement).closest(".rename-input")) return;
  justCommittedPath = proj.path;
  commitRename();
}

function onRowClick(proj: RecentProject) {
  if (renamingPath.value === proj.path) return;
  if (justCommittedPath === proj.path) {
    justCommittedPath = null;
    return;
  }
  handleOpenProject(proj.path);
}

async function handleRemoveProject(proj: RecentProject) {
  try {
    await projectStore.removeRecentProject(proj.path, deleteWithFile.value);
  } catch (e) {
    message.error(`删除失败：${e}`);
  }
}

// 删除确认弹窗里的复选框：勾选后确认会连带删除项目文件；每次打开弹窗重置为不勾选
const deleteWithFile = ref(false);

// ── 环境自检：只做探针 ──
// 未就绪/探测失败都只显示状态，不阻塞任何操作（真正运行该引擎时才会提示），
// 这里也不做自动 bootstrap。三个探针互不等待：谁先回来谁先显示。
type ProbeState = "probing" | "ready" | "notready" | "unknown";

interface Probe {
  state: ProbeState;
  message: string;
  /// 配置的推理设备（仅 ASR 卡返回）："" = auto（ggml 自动选优）/ cpu / cuda / vulkan
  device?: string;
  /// 实测生效后端（仅 moss，来自 spawn 时 info 的 [mt I] backend 行），
  /// 如 "CUDA0"/"Vulkan1"/"CPU"；是硬判据——device=vulkan 静默降级后实测可能仍是 CUDA0
  backend?: string | null;
  /// 设备判定说明（仅 ASR 卡）：不可用时给出原因/错误原文
  deviceNote?: string;
  /// 设备判定为硬错误（显式指定了本机不具备的后端）
  deviceBad?: boolean;
  /// 已装运行时 vs 清单的结论（仅 ASR 卡，来自 probe_moss_install）
  installNote?: string;
  /// 该结论需要用户动作（更新 / 文件缺失 / 后端已下线）
  installWarn?: boolean;
  /// 原始状态码与已装后端（决定补齐命令装哪个版本）
  installState?: string;
  installFlavor?: string;
  /// 推荐安装的后端版本（cuda / vulkan / cpu），用于拼手动补齐命令
  recommend?: string;
}

const ENV_CARDS = [
  { key: "ocr", name: "PaddleOCR", desc: "OCR 引擎 · PP-OCR 本地 Python worker" },
  { key: "asr", name: "MOSS", desc: "转写 + 说话人分离（S01/S02）· ggml，有 CUDA/Vulkan 即 GPU" },
  { key: "embed", name: "e5-small", desc: "融合向量召回 · multilingual-e5-small ONNX qint8" },
] as const;

type EnvKey = (typeof ENV_CARDS)[number]["key"];

const probes = ref<Record<EnvKey, Probe>>({
  ocr: { state: "probing", message: "" },
  asr: { state: "probing", message: "" },
  embed: { state: "probing", message: "" },
});

const STATE_TEXT: Record<ProbeState, string> = {
  probing: "探测中…",
  ready: "就绪",
  notready: "未就绪",
  unknown: "—",
};

function stateText(key: EnvKey) {
  return STATE_TEXT[probes.value[key].state];
}

/// 探测失败与未就绪都用灰点，探测中给琥珀点
function dotClass(key: EnvKey) {
  const state = probes.value[key].state;
  if (state === "ready") return "";
  return state === "probing" ? "warn" : "off";
}

function setProbe(key: EnvKey, ready: boolean | null, msg: string, device = "", backend: string | null = null) {
  probes.value[key] = {
    state: ready === null ? "unknown" : ready ? "ready" : "notready",
    message: msg,
    device,
    backend,
  };
}

/// 把设备判定结论并进 ASR 卡（显式指定了本机不具备的后端 = 硬错误，醒目显示）
function setAsrDeviceNote(note: string, bad: boolean) {
  probes.value.asr = { ...probes.value.asr, deviceNote: note, deviceBad: bad };
}

/// 这些状态需要用户动作（更新 / 文件缺失 / 后端已下线）；ok / unknown / not_installed 只陈述
const INSTALL_ACTION_STATES = ["update_available", "file_missing", "flavor_not_in_manifest"];

/// 把「已装运行时 vs 清单」的结论并进 ASR 卡
function setAsrInstall(install: MossInstallStatus) {
  probes.value.asr = {
    ...probes.value.asr,
    installNote: install.message,
    installWarn: INSTALL_ACTION_STATES.includes(install.state),
    installState: install.state,
    installFlavor: install.installed?.flavor,
  };
}

async function probeOcr() {
  try {
    const status = await invoke<{ ready: boolean; message: string }>("check_ocr_runtime");
    setProbe("ocr", status.ready, status.message);
  } catch (e) {
    setProbe("ocr", null, String(e));
  }
}

async function probeAsr() {
  try {
    const engines = await projectStore.getAsrEngines();
    const moss = engines.find((e) => e.engine === "moss");
    if (!moss) {
      setProbe("asr", null, "探针未返回 MOSS 引擎信息");
    } else {
      setProbe("asr", moss.ready, moss.message, moss.device, moss.effective_backend ?? null);
    }
  } catch (e) {
    setProbe("asr", null, String(e));
  }

  // 设备判定与运行时比对互不等待、互不拖累：任一失败只丢那一行信息
  const [report, install] = await Promise.all([
    projectStore.probeMossBackend().catch(() => null),
    projectStore.probeMossInstall().catch(() => null),
  ]);

  if (report) {
    probes.value.asr = { ...probes.value.asr, recommend: report.recommendation };
    if (!report.available) {
      // 显式指定了本机不具备的后端 = 硬错误，卡片上直接摊开说清楚
      setAsrDeviceNote(report.error || report.reason || "当前推理设备不可用", report.requested !== "auto");
    } else if (install?.installed && report.recommendation && report.recommendation !== install.installed.flavor) {
      // 装的是 CPU 版但本机能上 GPU：把「为什么没吃到 GPU / 该装哪个版本」说清楚
      setAsrDeviceNote(`建议改用 ${report.recommendation} 版运行时：${report.recommendation_reason}`, false);
    }
  }

  if (install) {
    setAsrInstall(install);
  }
}

/// 手动补齐命令：源码运行时用 scripts/ 下的脚本；安装版由安装器负责（第 11c 项）
function cmdOf(key: EnvKey): string {
  if (key === "asr") {
    const p = probes.value.asr;
    // 更新既有安装 → 装回原后端；尚未安装 / 该后端已下线 → 装推荐后端
    const flavor = p.installState === "update_available" && p.installFlavor ? p.installFlavor : p.recommend;
    return `scripts/bootstrap_moss.ps1${flavor ? ` -Backend ${flavor}` : ""}`;
  }
  return key === "ocr" ? "scripts/bootstrap_ocr.ps1" : "scripts/bootstrap_embed.ps1";
}

/// 需要手动补齐/更新时才给命令：未就绪，或运行时与清单不一致（仅 ASR 有此判据）
function showCmd(key: EnvKey): boolean {
  if (probes.value[key].state === "notready") return true;
  return key === "asr" && !!probes.value.asr.installWarn;
}

async function copyCmd(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    message.success("已复制命令");
  } catch {
    message.warning("复制失败，请手动选中命令");
  }
}

/// 设备展示文案："" = auto（ggml 自动选优，实测偏好独显）
function deviceText(key: EnvKey): string {
  const d = probes.value[key].device;
  if (d === undefined) return "";
  return d === "" ? "auto（ggml 自动选优）" : d;
}

/// 实测生效后端（moss 专属）：有值时附在设备文案后
function backendText(key: EnvKey): string {
  const b = probes.value[key].backend;
  return b ? `实测 ${b}` : "";
}

async function probeEmbed() {
  try {
    const status = await invoke<{ ready: boolean; message: string }>("check_embed_runtime");
    setProbe("embed", status.ready, status.message);
  } catch (e) {
    setProbe("embed", null, String(e));
  }
}

onMounted(() => {
  projectStore.refreshRecentProjects();
  probeOcr();
  probeAsr();
  probeEmbed();
});
</script>

<template>
  <div class="welcome">
    <div class="wtop">
      <button
        class="tgl"
        :title="ui.isDark ? '切换到浅色主题' : '切换到深色主题'"
        @click="ui.toggleTheme()"
      >
        <AppIcon :name="ui.isDark ? 'moon' : 'sun'" :size="12" />
        {{ ui.isDark ? "深色" : "浅色" }}
      </button>
    </div>

    <div class="main">
      <div class="wgrid">
        <div class="wl">
          <div class="wbrand">
            <span class="wlogo">GSA</span>
            <span>
              <span class="wt">GameSubtitleAssistant</span>
              <span class="wtag">离线 · 本地推理 · 游戏剧情字幕工作站</span>
            </span>
          </div>
          <p class="wdesc">
            导入游戏录屏与直播切片，自动提取可信文本、转写语音并对齐成时间轴字幕；全部推理在本机完成。
          </p>
          <div class="wbtns">
            <button class="btn pri lg" @click="showCreateModal = true">
              <AppIcon name="plus" :size="14" />创建新项目
            </button>
            <button class="btn ghost lg" @click="handleOpenProjectDialog">
              <AppIcon name="folder" :size="14" />打开已有项目
            </button>
          </div>
          <span class="wtip">全部推理在本机运行，素材不出本机</span>
        </div>

        <div class="wr">
          <div class="rh">
            <span class="t">最近项目</span>
          </div>

          <div v-if="projectStore.recentProjects.length > 0" class="rlist">
            <div
              v-for="proj in projectStore.recentProjects"
              :key="proj.path"
              class="rrow"
              @mousedown="onRowMouseDown(proj, $event)"
              @click="onRowClick(proj)"
            >
              <span class="ric"><AppIcon name="folder" :size="16" /></span>
              <span class="rmain">
                <input
                  v-if="renamingPath === proj.path"
                  :ref="setRenameInput"
                  v-model="renameDraft"
                  class="rename-input"
                  @keydown.enter="commitRename"
                  @keydown.esc="cancelRename"
                  @blur="commitRename"
                />
                <template v-else>
                  <b class="rn">{{ proj.name }}</b>
                  <span class="rp">{{ proj.path }}</span>
                </template>
              </span>
              <span class="rt" title="最后打开">{{ formatUpdatedAt(proj.updated_at) }}</span>
              <span v-if="renamingPath !== proj.path" class="ract" @click.stop>
                <button class="btn ghost sm" @click="startRename(proj)">重命名</button>
                <NPopconfirm
                  negative-text="取消"
                  positive-text="确认"
                  @update:show="(show) => show && (deleteWithFile = false)"
                  @positive-click="handleRemoveProject(proj)"
                >
                  <template #trigger>
                    <button class="btn ghost sm">删除</button>
                  </template>
                  <div class="pop-delete">
                    <div>从最近项目中删除此项吗？</div>
                    <NCheckbox v-model:checked="deleteWithFile">
                      同时删除工程文件
                    </NCheckbox>
                  </div>
                </NPopconfirm>
              </span>
            </div>
          </div>

          <div v-else class="wempty">
            <p class="wempty-title">暂无最近项目</p>
            <p class="wempty-hint">创建或打开一个项目后，将显示在这里</p>
          </div>
        </div>
      </div>

      <div class="envsec">
        <div class="envh">
          <span class="t">环境自检</span>
          <span class="nh">只做探针自检 · 未就绪不会阻塞其它步骤，只有真正运行该引擎时才会提示</span>
          <span class="lg">
            <i class="dot" />就绪<i class="dot warn" />探测中<i class="dot off" />未就绪
          </span>
        </div>
        <div class="env">
          <div
            v-for="card in ENV_CARDS"
            :key="card.key"
            class="ecard"
            :title="probes[card.key].message"
          >
            <span class="et">{{ card.name }}</span>
            <span class="ev">
              <i class="dot" :class="dotClass(card.key)" />{{ stateText(card.key) }}
            </span>
            <span class="ex">{{ card.desc }}</span>
            <span v-if="card.key === 'asr' && deviceText(card.key)" class="ex">
              设备 · {{ deviceText(card.key) }}{{ backendText(card.key) ? `（${backendText(card.key)}）` : "" }}
            </span>
            <span
              v-if="card.key === 'asr' && probes.asr.deviceNote"
              class="ex"
              :class="{ danger: probes.asr.deviceBad, warn: !probes.asr.deviceBad }"
            >
              {{ probes.asr.deviceNote }}
            </span>
            <span v-if="card.key === 'asr' && probes.asr.installNote" class="ex" :class="{ warn: probes.asr.installWarn }">
              {{ probes.asr.installNote }}
            </span>
            <!-- 未就绪原因原文直接摊在卡面上（ASR 的原因已由上面两行覆盖，不重复） -->
            <span
              v-if="card.key !== 'asr' && probes[card.key].state === 'notready' && probes[card.key].message"
              class="ex"
            >
              {{ probes[card.key].message }}
            </span>
            <!-- 手动补齐命令（点击复制）；安装版没有 scripts/，由安装器负责 -->
            <code v-if="showCmd(card.key)" class="cmd" title="点击复制" @click="copyCmd(cmdOf(card.key))">
              {{ cmdOf(card.key) }}
            </code>
            <span v-if="card.key === 'asr' && probes.asr.state === 'notready'" class="ex">
              推理设备可在打轴页「转写参数 → 推理设备」中设置
            </span>
          </div>
        </div>
      </div>

      <div class="wfoot">素材需人工预校对，管线 raw 产物不等于最终字幕</div>
    </div>

    <NModal v-model:show="showCreateModal" title="创建新项目">
      <NCard title="创建新项目" style="width: 480px">
        <div class="mform">
          <NInput v-model:value="newProjectName" placeholder="项目名称" clearable />
          <span v-if="nameIllegal" class="ferr">
            项目名含非法字符（\ / : * ? " &lt; &gt; |），无法用作项目文件名
          </span>
          <NInput
            v-model:value="newProjectPath"
            placeholder="选择项目保存位置"
            readonly
          />
          <div class="mbtns">
            <button class="btn ghost" @click="handleSelectFolder">选择文件夹</button>
            <button
              class="btn pri"
              :disabled="!isValidPath(newProjectPath) || !newProjectName || nameIllegal"
              @click="handleCreateProject"
            >
              创建
            </button>
          </div>
        </div>
      </NCard>
    </NModal>
  </div>
</template>

<style scoped>
.welcome {
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--bg);
}

/* 欢迎页没有工程，因此只有一行顶栏位：右侧主题开关 */
.wtop {
  flex: none;
  height: 46px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 0 34px;
}

.main {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 4px 34px 0;
}

.wgrid {
  flex: 1;
  min-height: 0;
  display: flex;
  position: relative;
}

/* 贯穿全高的分隔线：两半各自居中，线不随内容高度跳动 */
.wgrid::after {
  content: "";
  position: absolute;
  left: 46%;
  top: 4px;
  bottom: 4px;
  width: 1px;
  background: var(--hair);
}

.wl {
  flex: 0 0 46%;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 16px;
  padding-right: 40px;
}

.wr {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding-left: 40px;
}

.wbrand {
  display: flex;
  align-items: center;
  gap: 16px;
}

.wlogo {
  width: 48px;
  height: 48px;
  flex: none;
  border-radius: 12px;
  background: var(--accent);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font: 600 17px/1 var(--mono);
}

.wt {
  display: block;
  font-size: 20px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.wtag {
  display: block;
  font-size: 12px;
  color: var(--dim);
  margin-top: 4px;
}

.wdesc {
  font-size: 12.5px;
  color: var(--dim);
  line-height: 1.7;
  max-width: 470px;
}

.wbtns {
  display: flex;
  gap: 10px;
}

.wtip {
  font-size: 11.5px;
  color: var(--faint);
  display: flex;
  align-items: center;
  gap: 8px;
}

.wtip::before {
  content: "";
  width: 3px;
  height: 14px;
  border-radius: 2px;
  background: var(--accent);
}

.rlist {
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  overflow-y: auto;
  max-height: 100%;
}

.rrow {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 64px;
  padding: 0 12px;
  border-bottom: 1px solid var(--hair);
  cursor: pointer;
}

.rrow:last-child {
  border-bottom: 0;
}

.rrow:hover {
  background: var(--hover);
}

.ric {
  width: 30px;
  height: 30px;
  flex: none;
  border-radius: 7px;
  border: 1px solid var(--hair-2);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--dim);
}

.rmain {
  flex: 1;
  min-width: 0;
}

.rn {
  display: block;
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rp {
  display: block;
  font: 11px/1.5 var(--mono);
  color: var(--faint);
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rt {
  font: 11px/1 var(--mono);
  color: var(--dim);
  flex: none;
}

.ract {
  display: flex;
  gap: 6px;
  flex: none;
}

.rename-input {
  width: 100%;
  font-family: inherit;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  background: var(--raised);
  border: 1px solid var(--accent);
  border-radius: 4px;
  padding: 2px 6px;
  outline: none;
  box-sizing: border-box;
}

.wempty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1px dashed var(--hair);
  border-radius: 12px;
}

.wempty-title {
  font-size: 13px;
  color: var(--dim);
}

.wempty-hint {
  font-size: 11.5px;
  color: var(--faint);
}

.envsec {
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 20px;
}

.envh {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.envh .t {
  flex: none;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--dim);
}

.envh .nh {
  font-size: 11.5px;
  color: var(--faint);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.envh .lg {
  margin-left: auto;
  flex: none;
  display: flex;
  align-items: center;
  font-size: 11px;
  color: var(--faint);
}

.envh .lg i {
  margin-left: 8px;
}

.envh .lg i:first-child {
  margin-left: 0;
}

.env {
  flex: none;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}

.ecard {
  border: 1px solid var(--hair);
  border-radius: 8px;
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  background: var(--panel);
  min-width: 0;
}

.ecard .et {
  font-size: 11px;
  color: var(--faint);
  letter-spacing: 0.06em;
}

.ecard .ev {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
}

.ecard .ex {
  font-size: 11px;
  color: var(--dim);
}

.ecard .ex.danger {
  color: var(--danger);
}

.ecard .ex.warn {
  color: var(--warn);
}

.ecard .cmd {
  font-family: var(--mono);
  font-size: 10.5px;
  color: var(--text);
  background: var(--hover);
  border: 1px solid var(--hair);
  border-radius: 4px;
  padding: 4px 7px;
  cursor: pointer;
  overflow-wrap: anywhere;
  user-select: all;
}

.ecard .cmd:hover {
  border-color: var(--dim);
}

.wfoot {
  margin-top: auto;
  flex: none;
  font-size: 11.5px;
  color: var(--faint);
  display: flex;
  align-items: center;
  gap: 8px;
  border-top: 1px solid var(--hair);
  padding-top: 10px;
  margin-bottom: 18px;
}

.wfoot::before {
  content: "";
  width: 3px;
  height: 14px;
  border-radius: 2px;
  background: var(--accent);
}

.mform {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.mbtns {
  display: flex;
  gap: 8px;
}

.ferr {
  font-size: 12px;
  color: var(--danger);
}

.pop-delete {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
</style>
