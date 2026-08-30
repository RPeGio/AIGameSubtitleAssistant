<script setup lang="ts">
// 文本语料页：可靠游戏文本来源管理（剧情录屏 OCR / 截图 OCR / 纯文本粘贴）
// 控制轨：ocr_region（绑定剧情录屏视频 A）；产物：corpus 列表（独立字段，不入 tracks）
</script>

<template>
  <div class="page">
    <h2 class="page-title">文本语料</h2>
    <p class="page-desc">获取可靠的游戏内文本（步骤 2 实现：剧情录屏 OCR → 语料、粘贴文本、截图 OCR）</p>
    <div class="placeholder">语料获取与校对功能将在后续步骤实现</div>
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
