<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { useMessage } from "naive-ui";
import type { SetupItem } from "../types";
import { useProjectStore } from "../stores/project";
import { useSetupStore } from "../stores/setup";

const setup = useSetupStore();
const project = useProjectStore();
const message = useMessage();

const PHASE_TEXT: Record<string, string> = {
  download: "正在下载",
  verify: "正在校验",
  extract: "正在解压",
  configure: "正在配置",
};

/// 总进度：已完成条目数 + 当前条目百分比，除以本次要装的条目总数
const overallPercent = computed(() => {
  const total = project.installItemTotal;
  if (!total) return 0;
  const finished = Math.max(0, project.installItemIndex - 1);
  const current = Math.min(1, Math.max(0, project.installPercent / 100));
  return Math.min(100, Math.round(((finished + current) / total) * 100));
});

const progressLine = computed(() => {
  const total = project.installItemTotal;
  if (!total) return "正在准备…";
  const index = Math.min(project.installItemIndex || 1, total);
  const phase = PHASE_TEXT[project.installPhase] ?? "正在安装";
  return `第 ${index}/${total} 项 · ${phase} ${Math.round(project.installPercent)}%`;
});

const logHint = computed(() => {
  const dir = setup.status?.runtime_dir ?? "";
  return dir ? `${dir}\\logs\\install.log` : "运行时目录下的 logs\\install.log";
});

function formatSize(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(bytes / 1024 ** 2))} MB`;
}

function markOf(item: SetupItem): string {
  if (item.state === "ready") return "✓";
  if (item.state === "update") return "↑";
  return "↓";
}

function metaOf(item: SetupItem): string {
  const parts: string[] = [];
  if (item.flavor) parts.push(item.flavor.toUpperCase());
  if (item.state === "ready") parts.push("已就绪");
  else parts.push(formatSize(item.size));
  if (!item.required) parts.push("可选");
  if (item.note) parts.push(item.note);
  return parts.join(" · ");
}

async function runInstall(force = false): Promise<void> {
  await setup.install(force);
}

async function onCancel(): Promise<void> {
  const cancelled = await project.cancelRuntimeInstall();
  if (!cancelled) message.info("没有正在运行的安装任务");
}

// 装完停留一下再放行，避免进度条一闪而过
watch(
  () => setup.stage,
  (stage) => {
    if (stage === "done") setTimeout(() => (setup.stage = "ready"), 800);
  },
);

onMounted(async () => {
  await setup.refresh();
  // D1：首次进准备页直接开跑，不等点击
  if (setup.stage === "needs") void runInstall();
});
</script>

<template>
  <div v-if="!setup.passable" class="gate">
    <div class="card">
      <div class="spin" />
      <h1>首次启动准备</h1>
      <p class="sub">运行时装好才能开始工作。只需要一次，之后启动会直接进入。</p>

      <div v-if="setup.stage === 'checking'" class="hint">正在检查运行时…</div>

      <template v-else-if="setup.stage === 'needs' || setup.stage === 'installing'">
        <ul class="items">
          <li v-for="item in setup.status?.items ?? []" :key="item.id">
            <span class="mark" :class="item.state">{{ markOf(item) }}</span>
            <span class="name">{{ item.label }}</span>
            <span class="meta">{{ metaOf(item) }}</span>
          </li>
        </ul>

        <div v-if="setup.stage === 'installing'" class="prog">
          <div class="bar"><i :style="{ width: `${overallPercent}%` }" /></div>
          <div class="line">{{ progressLine }}</div>
          <div v-if="project.installDetail" class="detail" :title="project.installDetail">
            {{ project.installDetail }}
          </div>
        </div>

        <div class="acts">
          <button v-if="setup.stage === 'needs'" class="btn pri" @click="runInstall(false)">
            开始准备
          </button>
          <button v-else class="btn ghost" @click="onCancel">取消</button>
        </div>
      </template>

      <template v-else-if="setup.stage === 'failed'">
        <div class="err">{{ setup.error }}</div>
        <div class="hint">详细日志：{{ logHint }}</div>
        <div class="acts">
          <button class="btn pri" @click="runInstall(true)">重试</button>
          <button class="btn ghost" @click="setup.skip()">先跳过（受限模式）</button>
        </div>
      </template>

      <div v-else class="hint">准备完成，正在进入…</div>
    </div>
  </div>
</template>

<style scoped>
.gate {
  position: fixed;
  inset: 0;
  z-index: 300;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg);
}

.card {
  width: min(520px, calc(100vw - 48px));
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  text-align: center;
}

.spin {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 2px solid var(--hair);
  border-top-color: var(--accent);
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

h1 {
  margin: 2px 0 0;
  font-size: 17px;
  font-weight: 600;
  color: var(--text);
}

.sub {
  margin: 0;
  font-size: 12px;
  color: var(--dim);
  line-height: 1.6;
}

.hint {
  font-size: 12px;
  color: var(--faint);
  word-break: break-all;
}

.items {
  width: 100%;
  margin: 6px 0 0;
  padding: 10px 12px;
  list-style: none;
  border: 1px solid var(--hair);
  border-radius: 8px;
  background: var(--panel);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.items li {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 12px;
  text-align: left;
}

.mark {
  width: 12px;
  flex: none;
  font-weight: 600;
  color: var(--faint);
}

.mark.ready {
  color: var(--ok);
}

.mark.update {
  color: var(--warn);
}

.name {
  color: var(--text);
}

.meta {
  margin-left: auto;
  color: var(--faint);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}

.prog {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.bar {
  height: 4px;
  border-radius: 2px;
  background: var(--hover);
  overflow: hidden;
}

.bar i {
  display: block;
  height: 100%;
  background: var(--accent);
  transition: width 0.2s ease;
}

.line {
  font-size: 12px;
  color: var(--dim);
  font-variant-numeric: tabular-nums;
}

.detail {
  font-size: 11px;
  color: var(--faint);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.err {
  width: 100%;
  font-size: 12px;
  line-height: 1.6;
  color: var(--danger);
  text-align: left;
  word-break: break-word;
}

.acts {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 4px;
}
</style>
