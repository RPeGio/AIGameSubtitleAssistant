<script setup lang="ts">
import { computed } from "vue";
import { useProjectStore } from "../stores/project";
import { useManualSave } from "../composables/useManualSave";

const projectStore = useProjectStore();
const { manualSave } = useManualSave();

const statusMeta = computed(() => {
  switch (projectStore.saveState) {
    case "saved":
      return { color: "var(--color-success)", text: "已保存" };
    case "pending":
      return { color: "var(--color-warning)", text: "未保存" };
    case "saving":
      return { color: "var(--color-warning)", text: "保存中…" };
    case "error":
      return { color: "var(--color-error)", text: "保存失败" };
  }
});
</script>

<template>
  <div class="save-indicator" :title="statusMeta.text + '（Ctrl+S 立即保存）'">
    <span class="save-dot" :style="{ backgroundColor: statusMeta.color }" />
    <span class="save-text">{{ statusMeta.text }}</span>
    <button
      class="save-btn"
      :title="'保存 (Ctrl+S)'"
      @click="manualSave"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
        <polyline points="17 21 17 13 7 13 7 21" />
        <polyline points="7 3 7 8 15 8" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.save-indicator {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
}

.save-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.save-text {
  font-size: 11px;
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.save-btn {
  width: 22px;
  height: 22px;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--color-text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.15s;
}

.save-btn:hover {
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
}
</style>
