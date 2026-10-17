<script setup lang="ts">
import { computed } from "vue";
import { useProjectStore } from "../stores/project";

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
</script>

<template>
  <footer class="statusbar">
    <span class="sb">工程 {{ fileName }}</span>
    <span class="sb">轨道 {{ trackCount }} · 片段 {{ clipCount }}</span>
    <span class="sp" />
    <span v-if="metaText" class="sb">{{ metaText }}</span>
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
