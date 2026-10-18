<script setup lang="ts">
defineProps<{ name: string; size?: number }>();
</script>

<template>
  <svg
    class="ic"
    :width="size ?? 15"
    :height="size ?? 15"
    :style="{ width: `${size ?? 15}px`, height: `${size ?? 15}px` }"
  >
    <use :href="`#ic-${name}`" />
  </svg>
</template>
