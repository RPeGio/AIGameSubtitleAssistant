<script setup lang="ts">
import type { TimelineEvent } from "../../types";
import { useTimelineStore } from "../../stores/timeline";

const timeline = useTimelineStore();

const props = defineProps<{
  event: TimelineEvent;
  color: string;
  left: number;
  width: number;
  focused: boolean;
}>();

const emit = defineEmits<{ "click-clip": [] }>();

function clipText(): string {
  const e = props.event;
  switch (e.type) {
    case "ocr_region":
      return `(${e.x1.toFixed(2)}, ${e.y1.toFixed(2)}) → (${e.x2.toFixed(2)}, ${e.y2.toFixed(2)})`;
    case "ocr_text":
    case "asr":
    case "manual":
      return e.text.length > 24 ? e.text.slice(0, 24) + "…" : e.text;
  }
}
</script>

<template>
  <div
    class="clip"
    :class="{ focused, 'split-tool': timeline.activeTool === 'split' }"
    :data-event-id="event.id"
    :style="{
      left: left + 'px',
      width: width + 'px',
      backgroundColor: color,
      borderColor: focused ? '#fff' : color,
    }"
    @click.stop="emit('click-clip')"
  >
    <span class="clip-label">{{ clipText() }}</span>
  </div>
</template>

<style scoped>
.clip {
  position: absolute;
  top: 4px;
  height: 32px;
  border-radius: 4px;
  border: 2px solid;
  box-sizing: border-box;
  overflow: hidden;
  cursor: pointer;
  transition: box-shadow 0.1s;
}

.clip.focused {
  z-index: 5;
  box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.5);
}

.clip:hover {
  opacity: 0.9;
}

.clip.split-tool {
  cursor: crosshair;
}

.clip-label {
  display: block;
  padding: 0 6px;
  font-size: 11px;
  line-height: 28px;
  color: #fff;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
