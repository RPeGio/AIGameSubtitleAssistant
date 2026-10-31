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
type ProbeState = "probing" | "installing" | "ready" | "notready" | "unknown";

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
  /// 原始状态串（`probe_moss_install.state`）：用于区分「运行时已就绪但引擎仍缺料」
  installState?: string;
  /// 版本号（仅 ffmpeg 卡）：形如 "n8.1.3-25-g611cec792a"
  version?: string;
  /// 二进制来源（仅 ffmpeg 卡）："bundled" / "path" / "missing"
  origin?: string;
}

const ENV_CARDS = [
  { key: "ocr", name: "PaddleOCR", desc: "OCR 引擎 · PP-OCR 本地 Python worker" },
  { key: "asr", name: "MOSS", desc: "转写 + 说话人分离（S01/S02）· ggml，有 CUDA/Vulkan 即 GPU" },
  { key: "embed", name: "e5-small", desc: "融合向量召回 · multilingual-e5-small ONNX qint8" },
  { key: "ffmpeg", name: "FFmpeg", desc: "视频解码 / 抽帧 / 抽音 / 波形 · 随安装包内嵌" },
] as const;

type EnvKey = (typeof ENV_CARDS)[number]["key"];

const probes = ref<Record<EnvKey, Probe>>({
  ocr: { state: "probing", message: "" },
  asr: { state: "probing", message: "" },
  embed: { state: "probing", message: "" },
  ffmpeg: { state: "probing", message: "" },
});

const STATE_TEXT: Record<ProbeState, string> = {
  probing: "探测中…",
  installing: "安装中…",
  ready: "就绪",
  notready: "未就绪",
  unknown: "—",
};

function stateText(key: EnvKey) {
  return STATE_TEXT[probes.value[key].state];
}

/// 探测失败与未就绪都用灰点，探测中 / 安装中给琥珀点
function dotClass(key: EnvKey) {
  const state = probes.value[key].state;
  if (state === "ready") return "";
  return state === "probing" || state === "installing" ? "warn" : "off";
}

function setProbe(
  key: EnvKey,
  ready: boolean | null,
  msg: string,
  device = "",
  backend: string | null = null,
  warming = false,
) {
  probes.value[key] = {
    // 后台预热（provider 还在握手）≠ 未就绪：给「探测中…」。否则首帧显示"未就绪"、
    // 几秒后自己变"就绪"，看起来像故障（真机反馈 2026-10-31 就是这条）
    state: warming ? "probing" : ready === null ? "unknown" : ready ? "ready" : "notready",
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
const INSTALL_ACTION_STATES = ["update_available", "file_missing", "flavor_not_in_manifest", "model_missing"];

/// 把「已装运行时 vs 清单」的结论并进 ASR 卡
function setAsrInstall(install: MossInstallStatus) {
  probes.value.asr = {
    ...probes.value.asr,
    installNote: install.message,
    installWarn: INSTALL_ACTION_STATES.includes(install.state),
    installState: install.state,
  };
}

/// 后台预热期间的文案（provider 构造不再同步握手，见契约 §8.6.1）
const WARMING_HINTS = ["正在启动", "正在探测"];

function isWarming(message: string): boolean {
  return WARMING_HINTS.some((h) => message.includes(h));
}

async function probeOcr(retries = 8) {
  try {
    const status = await invoke<{ ready: boolean; message: string }>("check_ocr_runtime");
    const warming = !status.ready && isWarming(status.message);
    setProbe("ocr", status.ready, status.message, "", null, warming);
    // 模型加载在后台线程跑（约 2–7 s）：打到「正在启动」就隔一会儿再看一眼，
    // 别让用户停在「未就绪」上
    if (warming && retries > 0) {
      window.setTimeout(() => void probeOcr(retries - 1), 1200);
    }
  } catch (e) {
    setProbe("ocr", null, String(e));
  }
}

async function probeAsr(retries = 20) {
  try {
    const engines = await projectStore.getAsrEngines();
    const moss = engines.find((e) => e.engine === "moss");
    if (!moss) {
      setProbe("asr", null, "探针未返回 MOSS 引擎信息");
    } else {
      setProbe(
        "asr",
        moss.ready,
        moss.message,
        moss.device,
        moss.effective_backend ?? null,
        !moss.ready && isWarming(moss.message),
      );
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

  // MOSS 后端探测在后台线程跑：打到「正在探测」就隔一会儿再看一眼
  const p = probes.value.asr;
  if ((p.state === "notready" || p.state === "probing") && retries > 0 && isWarming(p.message)) {
    window.setTimeout(() => void probeAsr(retries - 1), 1500);
  }
}

/// 一键安装/更新的入口条件：运行时与清单不一致，或引擎未就绪。
/// 清单里现在也含模型（models-moss）与 OCR/向量运行时，所以「装全部」是缺省口径。
const canInstall = computed(() => {
  const p = probes.value.asr;
  if (projectStore.installRunning) return false;
  if (p.installWarn) return true;
  return p.state === "notready";
});

/// 运行时与清单一致（installState === "ok"）却仍未就绪：多半是文件被挪动/损坏，
/// 按钮改成「重装运行时」（带 -Force 覆盖已装标记）
const forceReinstall = computed(() => {
  const p = probes.value.asr;
  return p.state === "notready" && p.installState === "ok" && !p.installWarn;
});

const installLabel = computed(() => {
  if (forceReinstall.value) return "重装运行时";
  return probes.value.asr.state === "notready" ? "安装运行时" : "更新运行时";
});


const PHASE_TEXT: Record<string, string> = {
  download: "下载",
  verify: "校验",
  extract: "解压",
  configure: "配置",
};

/// 进度行文案：「下载 42%」；脚本还没报阶段时退回「准备中…」
const installLine = computed(() => {
  if (!projectStore.installRunning) return "";
  const phase = PHASE_TEXT[projectStore.installPhase] || "";
  const pct = Math.round(projectStore.installPercent);
  if (phase) return `${phase} ${pct}%`;
  return pct > 0 ? `${pct}%` : "准备中…";
});

/// 装 MOSS 运行时：先磁盘预检（cuda 包 622 MB，别下到一半才发现没空间），
/// 再交给 installer（flavor=auto = 按本机能力选 cuda / vulkan / cpu）
async function startInstall() {
  if (projectStore.installRunning) return;
  let pre;
  try {
    pre = await projectStore.precheckRuntimeDisk();
  } catch (e) {
    message.error(`磁盘预检失败：${e}`);
    return;
  }
  if (!pre.ok) {
    message.error(pre.message);
    return;
  }
  probes.value.asr = { ...probes.value.asr, state: "installing" };
  const summary = await projectStore.startRuntimeInstall("auto", forceReinstall.value).catch((e) => {
    message.error(`安装失败：${e}`);
    return null;
  });
  if (summary) {
    if (summary.cancelled) message.warning(summary.message);
    else if (summary.ok) message.success(summary.message);
    else message.error(summary.message);
  }
  // 装好的二进制与模型只落在磁盘上：进程内的 provider 还是启动时按「空目录」
  // 构造的占位对象，必须重建才能生效 —— 否则就是"装完不重启永远未就绪"
  if (summary?.ok) {
    try {
      const reload = await projectStore.reloadRuntimes();
      const bad = [...reload.asr, reload.ocr].filter((line) => line.includes("未就绪"));
      if (bad.length) message.info(bad.join("；"));
    } catch (e) {
      message.warning(`运行时已装好，但热加载失败，重启应用后生效：${e}`);
    }
  }
  // 装完或取消都重探：运行时状态、设备判定、实测后端都可能变
  await refreshAll();
}

async function cancelInstall() {
  try {
    if (!(await projectStore.cancelRuntimeInstall())) message.info("没有正在运行的安装任务");
  } catch (e) {
    message.error(`取消失败：${e}`);
  }
}

/// 手动补齐命令（只给安装器覆盖不到的条目：OCR / e5-small / 内嵌 ffmpeg）；
/// MOSS 由上面的安装按钮负责，不再重复给命令
function cmdOf(key: EnvKey): string {
  if (key === "ocr") return "scripts/bootstrap_ocr.ps1";
  if (key === "ffmpeg") return "scripts/fetch_ffmpeg.ps1";
  return "scripts/bootstrap_embed.ps1";
}

function showCmd(key: EnvKey): boolean {
  return key !== "asr" && probes.value[key].state === "notready";
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

/// ffmpeg 自检：内嵌优先、回退 PATH，只读探测
async function probeFfmpeg() {
  try {
    const status = await projectStore.checkFfmpeg();
    probes.value.ffmpeg = {
      state: status.ready ? "ready" : "notready",
      message: status.message,
      version: status.version,
      origin: status.source,
    };
  } catch (e) {
    setProbe("ffmpeg", null, String(e));
  }
}

/// 二进制来源文案：内嵌 / 系统 PATH（都没找到时不显示这行）
function originText(key: EnvKey): string {
  const o = probes.value[key].origin;
  if (o === "bundled") return "内嵌";
  if (o === "path") return "系统 PATH";
  return "";
}

/// 四张卡一起重探（一键安装完成后 / 手动点「重新探测」）。
/// 单张卡各自重试内部已有节流，这里只负责同时触发。
async function refreshAll() {
  await Promise.all([probeOcr(), probeAsr(), probeEmbed(), probeFfmpeg()]);
}

onMounted(() => {
  projectStore.refreshRecentProjects();
  void refreshAll();
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
            <span v-if="card.key === 'ffmpeg' && probes.ffmpeg.state === 'ready'" class="ex">
              版本 · {{ probes.ffmpeg.version || "未知" }}（{{ originText("ffmpeg") }}）
            </span>
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
            <!-- 运行时与清单一致、引擎却仍未就绪：真实原因在 installNote/卡面原因里说，
                 这里不再猜「多半是模型没下」（那是 loader 口径未回退时的假结论） -->
            <!-- 一键安装/更新：装清单里的全部条目（运行时 + 模型 + OCR/向量运行时），装完自动重探 -->
            <span v-if="card.key === 'asr' && !projectStore.installRunning" class="ex act">
              <button v-if="canInstall" class="btn pri sm" @click="startInstall">{{ installLabel }}</button>
              <span v-if="canInstall" class="hint">按本机能力选后端</span>
              <button class="btn ghost sm" @click="refreshAll">重新探测</button>
            </span>
            <template v-if="card.key === 'asr' && projectStore.installRunning">
              <span class="pbar"><i :style="{ width: `${projectStore.installPercent}%` }" /></span>
              <span class="ex act">
                <span class="inst">{{ installLine }}</span>
                <button class="btn ghost sm" @click="cancelInstall">取消</button>
              </span>
              <span v-if="projectStore.installDetail" class="ex ell" :title="projectStore.installDetail">
                {{ projectStore.installDetail }}
              </span>
            </template>
            <!-- ASR 未就绪的原因原文：别只藏在 tooltip 里。真机上"未就绪 + 没有原因"
                 根本没法报障（2026-10-31 反馈），这里直接摊开 -->
            <span
              v-if="card.key === 'asr' && probes.asr.state === 'notready' && probes.asr.message"
              class="ex"
              :class="{ danger: !isWarming(probes.asr.message) }"
            >
              {{ probes.asr.message }}
            </span>
            <!-- 未就绪原因原文直接摊在卡面上（ASR 的已由上面一行/两行覆盖，不重复） -->
            <span
              v-if="card.key !== 'asr' && probes[card.key].state === 'notready' && probes[card.key].message"
              class="ex"
            >
              {{ probes[card.key].message }}
            </span>
            <!-- 手动补齐命令（点击复制）：只给安装器覆盖不到的条目 -->
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
  /* 4 张卡在宽屏一行放下，窄屏自动折行（固定 3 列会让第 4 张独占一行） */
  grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
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

/* 安装按钮行与进度行：与文案行同样式底，只多一层横排 */
.ecard .ex.act {
  display: flex;
  align-items: center;
  gap: 8px;
}

.ecard .act .hint {
  font-size: 10.5px;
  color: var(--faint);
}

.ecard .inst {
  font-size: 11px;
  color: var(--warn);
  font-variant-numeric: tabular-nums;
}

.ecard .pbar {
  display: block;
  height: 3px;
  border-radius: 2px;
  background: var(--hover);
  overflow: hidden;
}

.ecard .pbar i {
  display: block;
  height: 100%;
  background: var(--accent);
  transition: width 0.2s;
}

/* detail 是脚本原文的短诊断，长了就省略，全文走 title */
.ecard .ex.ell {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
