<script setup lang="ts">
// AI 融合页：可靠文本 ↔ 转写文本对应替换（LLM 融合）
// 输入：corpus 语料 / OCR 轨文本 + game 轨 ASR 段；产物：fused 轨
</script>

<template>
  <div class="page">
    <h2 class="page-title">AI 融合</h2>
    <p class="page-desc">可靠文本与转写文本对应替换（步骤 4 实现）</p>
    <div class="placeholder">融合参数与运行功能将在后续步骤实现</div>
  </div>
</template>

<style scoped>
.page {
  flex: 1;
  min-width: 0;
  padding: 20px 24px;
  overflow: auto;
}

.page-title {
  font-size: 18px;
  font-weight: 700;
  color: var(--color-text-primary);
  margin-bottom: 8px;
}

.page-desc {
  font-size: 13px;
  color: var(--color-text-secondary);
  margin-bottom: 16px;
}

.placeholder {
  padding: 32px;
  border: 1px dashed var(--color-border);
  border-radius: 8px;
  text-align: center;
  font-size: 13px;
  color: var(--color-text-secondary);
}
</style>
