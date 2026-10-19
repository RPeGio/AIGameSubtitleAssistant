<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import AppIcon from "./AppIcon.vue";
import InfoDot from "./InfoDot.vue";
import { OCR_PARAM_DOCS } from "../composables/ocrParamDocs";

/// 无框数值步进：默认只显示数值（左减右加），单击数值才出现输入框。
/// 样式词汇（.pcell/.step/.pl/.val）在 src/style.css 的全局控件层。
const props = withDefaults(
  defineProps<{
    label: string;
    modelValue: number;
    /// 对应 OcrRunParams 的字段名：给了就在标签后挂 info 圆点（文案见 ocrParamDocs.ts）
    paramKey?: string;
    step?: number;
    min?: number;
    max?: number;
    /// 显示/输入时保留的小数位（0 = 整数）
    precision?: number;
  }>(),
  { paramKey: "", step: 1, min: Number.NEGATIVE_INFINITY, max: Number.POSITIVE_INFINITY, precision: 0 }
);

const doc = computed(() => (props.paramKey ? OCR_PARAM_DOCS[props.paramKey] ?? "" : ""));

const emit = defineEmits<{ "update:modelValue": [value: number] }>();

const editing = ref(false);
const draft = ref("");
const inputEl = ref<HTMLInputElement | null>(null);

const display = computed(() => {
  const v = props.modelValue;
  if (!Number.isFinite(v)) return "—";
  if (props.precision <= 0 || Number.isInteger(v)) return String(v);
  return v.toFixed(props.precision).replace(/0+$/, "").replace(/\.$/, "");
});

function commit(v: number) {
  const next = Math.min(props.max, Math.max(props.min, v));
  if (next !== props.modelValue) emit("update:modelValue", next);
}

function bump(dir: 1 | -1) {
  commit(Number((props.modelValue + dir * props.step).toFixed(4)));
}

async function startEdit() {
  draft.value = String(props.modelValue);
  editing.value = true;
  await nextTick();
  inputEl.value?.focus();
  inputEl.value?.select();
}

function endEdit(ok: boolean) {
  if (ok) {
    const v = Number(draft.value);
    if (Number.isFinite(v)) commit(v);
  }
  editing.value = false;
}
</script>

<template>
  <div class="pcell">
    <span class="pl">
      <span class="pl-t">{{ label }}</span>
      <InfoDot v-if="doc" :text="doc" :title="label" />
    </span>
    <span class="step">
      <b @click="bump(-1)"><AppIcon name="minus" :size="12" /></b>
      <input
        v-if="editing"
        ref="inputEl"
        v-model="draft"
        class="val ed"
        @keydown.enter="endEdit(true)"
        @keydown.esc="endEdit(false)"
        @blur="endEdit(true)"
      />
      <span v-else class="val" @click="startEdit">{{ display }}</span>
      <b @click="bump(1)"><AppIcon name="plus" :size="12" /></b>
    </span>
  </div>
</template>

<style scoped>
.val.ed {
  height: 26px;
  border: 1px solid var(--accent);
  border-radius: 6px;
  background: none;
  font-family: var(--mono);
  font-size: 12.5px;
  color: var(--text);
  outline: none;
}
</style>
