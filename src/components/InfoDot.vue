<script setup lang="ts">
import { computed, ref } from "vue";
import { NTooltip } from "naive-ui";
import AppIcon from "./AppIcon.vue";

/// 参数标签后的 info 圆点：hover / 键盘聚焦显示解释。
///
/// 用 naive `NTooltip` 而不是自绘或原生 `title`：参数卡片体是 `overflow-y: auto`
/// 的滚动容器，自绘 tooltip 会被裁掉；naive 的 popper 挂在 body，且已随
/// `n-config-provider` 的主题翻转；原生 `title` 有约 1 秒延迟且样式不可控。
/// naive 的 `trigger` 只能取单个值（`'click' | 'hover' | 'focus' | 'manual'`），
/// 所以用 `manual` + 自己接管 mouseenter/leave 与 focus/blur，兼顾鼠标与键盘。
const props = withDefaults(
  defineProps<{
    text: string;
    /// 悬浮标题（通常传标签名）
    title?: string;
    maxWidth?: number;
    placement?: "top" | "bottom" | "left" | "right";
  }>(),
  { title: "", maxWidth: 320, placement: "top" }
);

const show = ref(false);
const open = () => (show.value = true);
const close = () => (show.value = false);

/// 文字按 \n 分段渲染：tooltip 内容不换行是 naive 的默认行为，必须显式覆盖
const tooltipStyle = computed(() => ({
  maxWidth: `${props.maxWidth}px`,
  padding: "8px 10px",
  fontSize: "12px",
  lineHeight: "1.6",
  whiteSpace: "pre-line" as const,
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--hair-2)",
  borderRadius: "6px",
  boxShadow: "0 8px 22px rgba(0, 0, 0, 0.38)",
}));
</script>

<template>
  <NTooltip v-model:show="show" trigger="manual" :placement="placement" :style="tooltipStyle">
    <template #trigger>
      <span
        class="infodot"
        tabindex="0"
        role="img"
        :aria-label="title || '参数说明'"
        @mouseenter="open"
        @mouseleave="close"
        @focus="open"
        @blur="close"
      >
        <AppIcon name="info" :size="12" />
      </span>
    </template>
    <div v-if="title" class="tip-t">{{ title }}</div>
    <div>{{ text }}</div>
  </NTooltip>
</template>

<style scoped>
.infodot {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 15px;
  height: 15px;
  margin-left: 3px;
  border-radius: 3px;
  color: var(--faint);
  cursor: help;
  vertical-align: -3px;
}
.infodot:hover,
.infodot:focus-visible {
  color: var(--accent-ink);
  background: var(--accent-soft);
  outline: none;
}
.tip-t {
  font-weight: 600;
  margin-bottom: 2px;
}
</style>
