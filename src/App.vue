<script setup lang="ts">
import { NMessageProvider } from "naive-ui";
</script>

<template>
  <n-message-provider placement="top">
    <router-view />
  </n-message-provider>
</template>
