<script setup lang="ts">
import { computed } from "vue";
import { darkTheme, NConfigProvider, NMessageProvider } from "naive-ui";
import AppIconSprite from "./components/AppIconSprite.vue";
import SetupGate from "./components/SetupGate.vue";
import { useSetupStore } from "./stores/setup";
import { useUiStore } from "./stores/ui";

const ui = useUiStore();
const setup = useSetupStore();

const themeOverrides = computed(() => ({
  common: {
    primaryColor: ui.isDark ? "#5e6ad2" : "#4f5bc4",
    primaryColorHover: ui.isDark ? "#6f7ade" : "#5a66cf",
    primaryColorPressed: ui.isDark ? "#4f5bc4" : "#3b46a8",
    primaryColorSuppl: ui.isDark ? "#6f7ade" : "#5a66cf",
    borderRadius: "6px",
    fontSize: "13px",
  },
}));
</script>

<template>
  <n-config-provider :theme="ui.isDark ? darkTheme : null" :theme-overrides="themeOverrides">
    <n-message-provider placement="top">
      <AppIconSprite />
      <!-- 首启门控：运行时装齐之前不渲染主界面，避免进到一半到处报未就绪 -->
      <SetupGate />
      <router-view v-if="setup.passable" />
    </n-message-provider>
  </n-config-provider>
</template>
