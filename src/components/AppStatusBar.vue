<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/project";
import type { AsrEngineStatus } from "../types";

const projectStore = useProjectStore();

const fileName = computed(() => {
  const path = projectStore.currentProject?.path ?? "";
  return path.split(/[\\/]/).pop() || projectStore.currentProject?.name || "—";
});

const trackCount = computed(() => projectStore.currentProject?.tracks.length ?? 0);

const clipCount = computed(() =>
  (projectStore.currentProject?.tracks ?? []).reduce(
    (n, t) => n + t.events.length,
    0
  )
);

const metaText = computed(() => {
  const m = projectStore.currentVideoMeta;
  if (!m) return "";
  return `${m.fps.toFixed(1)}fps · ${m.codec} · ${m.width}×${m.height}`;
});

// ── 引擎段：显示"当前实际可用的引擎"，只做只读浅探测 ──────────
/// 只剩 MOSS 一个引擎（FunASR 已弃用、不再分发）；后端仍会把它一并报上来，这里按白名单过滤
const ENGINE_PRIORITY = ["moss"] as const;
const ENGINE_LABEL: Record<string, string> = { moss: "MOSS" };

const asrEngines = ref<AsrEngineStatus[] | null>(null);
const embedReady = ref<boolean | null>(null);

/// 探测失败（null）如实显示 "—"，不阻塞任何操作；真正运行时由各页探针与后端报错兜底
async function probe() {
  try {
    asrEngines.value = await projectStore.getAsrEngines();
  } catch {
    asrEngines.value = null;
  }
  try {
    embedReady.value = (await invoke<{ ready: boolean }>("check_embed_runtime")).ready;
  } catch {
    embedReady.value = null;
  }
}

onMounted(() => void probe());

const engineText = computed(() => {
  const list = asrEngines.value;
  if (list === null) return "ASR: —";
  const key = ENGINE_PRIORITY.find((k) =>
    list.some((e) => e.engine === k && e.ready)
  );
  return `ASR: ${key ? ENGINE_LABEL[key] : "未就绪"}`;
});

const vectorText = computed(() =>
  embedReady.value === null ? "向量: —" : `向量: ${embedReady.value ? "e5-small" : "未就绪"}`
);
</script>

<template>
  <footer class="statusbar">
    <span class="sb">工程 {{ fileName }}</span>
    <span class="sb">轨道 {{ trackCount }} · 片段 {{ clipCount }}</span>
    <span class="sp" />
    <span v-if="metaText" class="sb">{{ metaText }}</span>
    <span class="sb">{{ engineText }} · {{ vectorText }}</span>
  </footer>
</template>

<style scoped>
.statusbar {
  flex: 0 0 26px;
  height: 26px;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 12px;
  background: var(--color-bg-secondary);
  border-top: 1px solid var(--color-border);
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1;
  color: var(--faint);
  white-space: nowrap;
  overflow: hidden;
}

.sp {
  flex: 1;
  min-width: 0;
}
</style>
