<script setup lang="ts">
import { computed, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { NButton, NModal, NPopconfirm, NSelect, NSwitch, useMessage } from "naive-ui";
import AppIcon from "./AppIcon.vue";
import InfoDot from "./InfoDot.vue";
import { useSettingsStore } from "../stores/settings";
import { useUiStore } from "../stores/ui";
import { useTimelineStore } from "../stores/timeline";
import { useSetupStore } from "../stores/setup";
import { SHORTCUTS, SHORTCUT_GROUPS } from "../composables/shortcuts";
import type { MergeStrategy, SplitStrategy } from "../composables/sentenceCuts";
import { AUTOSAVE_DELAYS, ZOOM_VALUES } from "../composables/appPrefs";

const show = defineModel<boolean>("show", { required: true });

const settings = useSettingsStore();
const ui = useUiStore();
const timeline = useTimelineStore();
const setup = useSetupStore();
const message = useMessage();

/// 打开时才拉运行时快照（命令要读盘 + 查可用空间，不必常驻）
watch(show, (on) => {
  if (on) void settings.load();
});

const snap = computed(() => settings.snapshot);

const ZOOM_OPTIONS = ZOOM_VALUES.map((z) => ({
  label: `${Math.round(z * 100)}%`,
  value: z,
}));

const SOURCE_OPTIONS = [
  { label: "自动（镜像 → GitHub）", value: "auto" },
  { label: "强制镜像", value: "mirror" },
  { label: "直连 GitHub", value: "direct" },
];

const SEGMENT_OPTIONS = [
  { label: "关闭分段（单流下载）", value: 0 },
  { label: "16 MB", value: 16 },
  { label: "64 MB（默认）", value: 64 },
  { label: "256 MB", value: 256 },
];

const OCR_DEVICE_OPTIONS = [
  { label: "自动（跟随运行时自检）", value: "" },
  { label: "CPU", value: "cpu" },
  { label: "GPU:0", value: "gpu:0" },
];

const AUTOSAVE_DELAY_OPTIONS = AUTOSAVE_DELAYS.map((d) => ({
  label: d < 1000 ? `${d} 毫秒` : `${d / 1000} 秒`,
  value: d,
}));

const SPLIT_OPTIONS: { label: string; value: SplitStrategy }[] = [
  { label: "按标点（默认）", value: "punct" },
  { label: "按空格", value: "space" },
  { label: "按原始断句", value: "raw" },
];

const MERGE_OPTIONS: { label: string; value: MergeStrategy }[] = [
  { label: "保留后一句（默认）", value: "latter" },
  { label: "保留前一句", value: "former" },
  { label: "上下堆叠", value: "stack" },
];

/// 快捷键只读表：分组顺序按 SHORTCUT_GROUPS，组内保持注册表顺序
const shortcutGroups = computed(() =>
  SHORTCUT_GROUPS.map((group) => ({
    group,
    rows: SHORTCUTS.filter((s) => s.group === group),
  })).filter((g) => g.rows.length > 0)
);

function fmtBytes(n: number | null): string {
  if (n === null) return "未知";
  const gb = n / 1024 ** 3;
  if (gb >= 1) return `${gb.toFixed(2)} GB`;
  return `${(n / 1024 ** 2).toFixed(0)} MB`;
}

/// 只提交改动的字段：后端 patch 是 Option，少传即不改
async function saveRuntime(patch: Parameters<typeof settings.patch>[0]) {
  const ok = await settings.patch(patch);
  if (!ok) message.error(`保存失败：${settings.error ?? "未知错误"}`);
}

async function onMove() {
  const target = await open({ directory: true, title: "选择新的 runtime 目录" });
  if (!target || typeof target !== "string") return;
  try {
    const res = await settings.moveRuntimeDir(target);
    if (res.ok) message.success(res.message);
    else message.error(res.message);
  } catch (e) {
    message.error(`移动失败：${e}`);
  }
}

async function onDeleteOld() {
  try {
    message.success(await settings.deleteOldRuntimeDir());
  } catch (e) {
    message.error(`删除失败：${e}`);
  }
}

async function onOpenLogs() {
  try {
    await settings.openLogDir();
  } catch (e) {
    message.error(`打开日志目录失败：${e}`);
  }
}

/// 「重新检查运行时」= 回到准备页重新判定（不强制重装）
function onRecheckRuntime() {
  setup.reopen();
  show.value = false;
}

function onZoom(next: number) {
  ui.setZoom(next);
}
</script>

<template>
  <NModal
    v-model:show="show"
    preset="card"
    display-directive="if"
    title="设置"
    style="width: 720px"
    :bordered="false"
    class="settings-modal"
  >
    <div class="set-body">
      <p v-if="settings.loading" class="set-loading">正在读取运行时设置…</p>

      <!-- ── A 外观 ─────────────────────────────────── -->
      <section class="set-group">
        <h4>外观</h4>
        <div class="set-row">
          <span class="sl">主题</span>
          <span class="sv">
            <button class="seg" :class="{ on: ui.isDark }" @click="ui.setTheme('dark')">
              <AppIcon name="moon" :size="12" /> 深色
            </button>
            <button class="seg" :class="{ on: !ui.isDark }" @click="ui.setTheme('light')">
              <AppIcon name="sun" :size="12" /> 浅色
            </button>
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            侧栏默认折叠
            <InfoDot text="下次启动时的初始状态；本次已展开/折叠的状态不受影响。" />
          </span>
          <span class="sv">
            <NSwitch
              :value="ui.sidebarCollapsed"
              size="small"
              @update:value="ui.setSidebarCollapsed"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">界面缩放</span>
          <span class="sv">
            <NSelect
              :value="ui.zoom"
              :options="ZOOM_OPTIONS"
              size="small"
              style="width: 132px"
              @update:value="onZoom"
            />
          </span>
        </div>
      </section>

      <!-- ── B 运行时 ───────────────────────────────── -->
      <section class="set-group">
        <h4>运行时</h4>
        <div class="set-row col">
          <span class="sl">runtime 目录</span>
          <span class="sv">
            <code class="path">{{ snap?.runtime_dir ?? "—" }}</code>
            <span class="hint">可用 {{ fmtBytes(snap?.free_bytes ?? null) }}</span>
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            搬家
            <InfoDot
              text="复制到新目录后重启应用生效。旧目录会保留，确认新位置一切正常后再删。"
            />
          </span>
          <span class="sv">
            <NButton size="small" :loading="settings.saving" @click="onMove()">
              移动到…
            </NButton>
            <NPopconfirm
              v-if="snap?.previous_runtime_dir"
              @positive-click="onDeleteOld"
            >
              <template #trigger>
                <NButton size="small" type="error" ghost :loading="settings.saving">
                  删除旧目录
                </NButton>
              </template>
              删除 {{ snap.previous_runtime_dir }}？此操作不可撤销。
            </NPopconfirm>
          </span>
        </div>
        <div v-if="snap?.previous_runtime_dir" class="set-row col">
          <span class="sl">旧目录（待删）</span>
          <span class="sv"><code class="path">{{ snap.previous_runtime_dir }}</code></span>
        </div>
        <div class="set-row">
          <span class="sl">下载源</span>
          <span class="sv">
            <NSelect
              :value="snap?.download_source ?? 'auto'"
              :options="SOURCE_OPTIONS"
              size="small"
              style="width: 260px"
              :disabled="!snap || settings.saving"
              @update:value="(v: string) => saveRuntime({ download_source: v })"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            分段下载
            <InfoDot
              text="大文件分段续传；镜像源分段会整包重取，脚本会自动改走单流。关闭后一律单流。"
            />
          </span>
          <span class="sv">
            <NSelect
              :value="snap?.segment_mb ?? 64"
              :options="SEGMENT_OPTIONS"
              size="small"
              style="width: 200px"
              :disabled="!snap || settings.saving"
              @update:value="(v: number) => saveRuntime({ segment_mb: v })"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">启动时自动准备运行时</span>
          <span class="sv">
            <NSwitch
              :value="snap?.setup_auto ?? true"
              size="small"
              :disabled="!snap || settings.saving"
              @update:value="(v: boolean) => saveRuntime({ setup_auto: v })"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">校验与日志</span>
          <span class="sv">
            <NButton size="small" @click="onRecheckRuntime">重新检查运行时</NButton>
            <NButton size="small" @click="onOpenLogs">打开日志目录</NButton>
          </span>
        </div>
        <div class="set-row">
          <span class="sl">安装清单</span>
          <span class="sv">
            <span :class="['hint', snap && !snap.manifest_found ? 'bad' : '']">
              {{ snap?.manifest_found ? "已找到 manifest.json" : "未找到 manifest.json" }}
            </span>
          </span>
        </div>
      </section>

      <!-- ── C 引擎 ─────────────────────────────────── -->
      <section class="set-group">
        <h4>引擎</h4>
        <div class="set-row">
          <span class="sl">
            OCR 设备
            <InfoDot text="留空 = 跟随运行时自检；显式指定后 OCR worker 启动时按此选择。" />
          </span>
          <span class="sv">
            <NSelect
              :value="snap?.ocr_device ?? ''"
              :options="OCR_DEVICE_OPTIONS"
              size="small"
              style="width: 200px"
              :disabled="!snap || settings.saving"
              @update:value="(v: string) => saveRuntime({ ocr_device: v })"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            MOSS 线程数
            <InfoDot text="0 = 由后端按 CPU 核数自动决定；过大反而抢显存/内存带宽。" />
          </span>
          <span class="sv">
            <NSelect
              :value="snap?.moss_threads ?? 0"
              :options="[0, 4, 8, 12, 16, 24, 32].map((n) => ({ label: n === 0 ? '自动' : `${n}`, value: n }))"
              size="small"
              style="width: 132px"
              :disabled="!snap || settings.saving"
              @update:value="(v: number) => saveRuntime({ moss_threads: v })"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">MOSS 超时（分钟）</span>
          <span class="sv">
            <NSelect
              :value="snap?.moss_timeout_minutes ?? 0"
              :options="[0, 15, 30, 45, 60, 120].map((n) => ({ label: n === 0 ? '不限制' : `${n} 分钟`, value: n }))"
              size="small"
              style="width: 148px"
              :disabled="!snap || settings.saving"
              @update:value="(v: number) => saveRuntime({ moss_timeout_minutes: v })"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            MOSS 推理设备
            <InfoDot text="只读：改这里要重启才生效，请到打轴页「转写参数 → 推理设备」修改（即时生效）。" />
          </span>
          <span class="sv">
            <code class="path">{{ snap?.moss_device || "auto" }}</code>
            <span class="hint">在打轴页修改</span>
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            保留中间产物
            <InfoDot text="dev_debug：把 OCR 逐帧图、MOSS 原始输出等留在临时目录，排查问题用。" />
          </span>
          <span class="sv">
            <NSwitch
              :value="snap?.dev_debug ?? false"
              size="small"
              :disabled="!snap || settings.saving"
              @update:value="(v: boolean) => saveRuntime({ dev_debug: v })"
            />
          </span>
        </div>
      </section>

      <!-- ── E 时间轴 ───────────────────────────────── -->
      <section class="set-group">
        <h4>时间轴与保存</h4>
        <div class="set-row">
          <span class="sl">默认开启吸附</span>
          <span class="sv">
            <NSwitch :value="settings.snapDefault" size="small" @update:value="settings.setSnapDefault" />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">分割策略</span>
          <span class="sv">
            <NSelect
              :value="timeline.splitStrategy"
              :options="SPLIT_OPTIONS"
              size="small"
              style="width: 168px"
              @update:value="timeline.setSplitStrategy"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">合并策略</span>
          <span class="sv">
            <NSelect
              :value="timeline.mergeStrategy"
              :options="MERGE_OPTIONS"
              size="small"
              style="width: 168px"
              @update:value="timeline.setMergeStrategy"
            />
          </span>
        </div>
        <div class="set-row">
          <span class="sl">
            自动保存
            <InfoDot text="关闭后只有 Ctrl+S 与关窗前兜底会写盘；改动多时容易忘记保存。" />
          </span>
          <span class="sv">
            <NSwitch :value="settings.autosave" size="small" @update:value="settings.setAutosave" />
          </span>
        </div>
        <div v-if="settings.autosave" class="set-row">
          <span class="sl">自动保存延迟</span>
          <span class="sv">
            <NSelect
              :value="settings.autosaveDelay"
              :options="AUTOSAVE_DELAY_OPTIONS"
              size="small"
              style="width: 132px"
              @update:value="settings.setAutosaveDelay"
            />
          </span>
        </div>
      </section>

      <!-- ── F 快捷键（只读） ───────────────────────── -->
      <section class="set-group">
        <h4>快捷键（只读）</h4>
        <div v-for="g in shortcutGroups" :key="g.group" class="sc-group">
          <div class="sc-title">{{ g.group }}</div>
          <div v-for="s in g.rows" :key="s.id" class="sc-row">
            <span class="sc-label">{{ s.label }}</span>
            <span class="sc-keys">
              <kbd v-for="k in s.keys" :key="k">{{ k }}</kbd>
            </span>
          </div>
        </div>
      </section>
    </div>
  </NModal>
</template>

<style scoped>
.set-body {
  max-height: min(70vh, 640px);
  overflow-y: auto;
  padding-right: 4px;
}

.set-loading {
  margin: 0 0 8px;
  font-size: 11px;
  color: var(--color-text-secondary);
}

.set-group {
  padding: 8px 0 10px;
  border-top: 1px solid var(--color-border);
}

.set-group:first-of-type {
  border-top: 0;
  padding-top: 0;
}

.set-group h4 {
  margin: 0 0 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--color-text-secondary);
}

.set-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 30px;
  padding: 2px 0;
}

.set-row.col {
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
}

.sl {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  color: var(--color-text-primary);
}

.sv {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--color-text-secondary);
}

.path {
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  word-break: break-all;
  color: var(--color-text-secondary);
}

.hint {
  font-size: 11px;
  color: var(--color-text-secondary);
}

.hint.bad {
  color: var(--warn);
}

.seg {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 9px;
  font-size: 11px;
  border: 1px solid var(--color-border);
  border-radius: 5px;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
}

.seg.on {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-soft);
}

.sc-group {
  margin-bottom: 6px;
}

.sc-title {
  font-size: 11px;
  color: var(--color-text-secondary);
  margin: 4px 0 2px;
}

.sc-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 2px 0;
  font-size: 12px;
}

.sc-label {
  color: var(--color-text-primary);
}

.sc-keys {
  display: flex;
  gap: 4px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.sc-keys kbd {
  font-family: var(--font-mono, monospace);
  font-size: 10.5px;
  padding: 1px 5px;
  border: 1px solid var(--color-border);
  border-bottom-width: 2px;
  border-radius: 4px;
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
}
</style>
