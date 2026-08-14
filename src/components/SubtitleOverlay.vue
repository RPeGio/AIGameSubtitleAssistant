<script setup lang="ts">
import { computed } from "vue";
import { useTimelineStore, CLIP_COLORS, TEXT_TRACK_TYPES } from "../stores/timeline";
import { useProjectStore } from "../stores/project";
import { useVideoContentRect } from "../composables/useVideoContentRect";
import type { TimelineEvent } from "../types";

const timeline = useTimelineStore();
const projectStore = useProjectStore();
const { containerRef, contentRect } = useVideoContentRect();

interface PreviewLine {
  color: string;
  prefix: string;
  text: string;
}

/// 角色/说话人前缀：asr 优先 character 再 speaker；manual 用 character。
/// 注：fused 类型在 llm 分支合并后启用（当前分支 types 尚无该事件）
function prefixOf(e: TimelineEvent): string {
  if (e.type === "asr") {
    if (e.character) return e.character;
    if (e.speaker) return e.speaker;
    return "";
  }
  if (e.type === "manual") {
    return e.character ?? "";
  }
  return "";
}

// 当前时间命中的文字事件：按轨道列表顺序，跳过 preview_visible=false 的轨道
const activeLines = computed<PreviewLine[]>(() => {
  if (!projectStore.subtitlePreviewOn) return [];
  const t = timeline.currentTime;
  const tracks = projectStore.currentProject?.tracks ?? [];
  const lines: PreviewLine[] = [];
  for (const track of tracks) {
    if (track.preview_visible === false) continue;
    if (!TEXT_TRACK_TYPES.includes(track.type)) continue;
    for (const e of track.events) {
      // ocr_region 无 text，排除后其余事件类型均含 text（TS 窄化）
      if (e.start <= t && e.end > t && e.type !== "ocr_region") {
        lines.push({
          color: CLIP_COLORS[e.type] ?? "#ffffff",
          prefix: prefixOf(e),
          text: e.text,
        });
      }
    }
  }
  return lines;
});

// 字幕堆叠贴视频画面底部（letterbox 校正后），translateY(-100%) 向上生长；
// max-height 限画面高度，多轨同时命中溢出时裁掉顶部（保留贴底行）
const stackStyle = computed(() => ({
  left: contentRect.value.left + "px",
  top: contentRect.value.top + contentRect.value.height + "px",
  width: contentRect.value.width + "px",
  maxHeight: contentRect.value.height + "px",
}));
</script>

<template>
  <div ref="containerRef" class="subtitle-layer">
    <div
      v-if="activeLines.length > 0 && contentRect.width > 0"
      class="subtitle-stack"
      :style="stackStyle"
    >
      <div v-for="(line, i) in activeLines" :key="i" class="subtitle-line">
        <span v-if="line.prefix" class="subtitle-prefix" :style="{ color: line.color }">
          {{ line.prefix }}：
        </span>
        <span class="subtitle-text">{{ line.text }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.subtitle-layer {
  position: absolute;
  inset: 0;
  pointer-events: none;
  /* 高于 RegionOverlay(z-index:5)：编辑 OCR 选区时字幕不被半透明遮罩压暗 */
  z-index: 6;
}

.subtitle-stack {
  position: absolute;
  transform: translateY(-100%);
  display: flex;
  flex-direction: column-reverse; /* 轨道列表第一条最贴底 */
  align-items: center;
  gap: 4px;
  padding: 0 4%;
  box-sizing: border-box;
  overflow: hidden; /* 超出画面高度时裁掉顶部溢出行 */
}

.subtitle-line {
  display: flex;
  align-items: baseline;
  gap: 4px;
  max-width: 100%;
  background: rgba(0, 0, 0, 0.55);
  border-radius: 4px;
  padding: 2px 10px;
  font-size: 14px;
  font-weight: 500;
  color: #fff;
  line-height: 1.5;
  text-align: center;
}

.subtitle-prefix {
  font-weight: 700;
  flex-shrink: 0;
}

.subtitle-text {
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
