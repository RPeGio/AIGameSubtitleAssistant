<script setup lang="ts">
// 语音转写页：切片视频 ASR 说话人分离 + 主播/游戏角色标记拆轨
// 控制轨：ocr_region（绑定切片视频 B，为 OCR 切片功能占位）；产物：asr 轨（streamer/game）
</script>

<template>
  <div class="page">
    <h2 class="page-title">语音转写</h2>
    <p class="page-desc">切片视频 ASR 转写 + 说话人标记拆轨（步骤 4/5 实现）</p>
    <div class="placeholder">转写与说话人标记功能将在后续步骤实现</div>
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
