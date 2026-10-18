import { defineStore } from "pinia";
import { computed, ref } from "vue";

export type ThemeName = "dark" | "light";

const THEME_KEY = "gsa.theme";
const SIDEBAR_KEY = "gsa.sidebar";

function readTheme(): ThemeName {
  return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
}

/** 外壳状态：主题写 <html data-theme>（令牌层随属性翻转）、侧栏折叠状态 */
export const useUiStore = defineStore("ui", () => {
  const theme = ref<ThemeName>(readTheme());
  const isDark = computed(() => theme.value === "dark");
  const sidebarCollapsed = ref(localStorage.getItem(SIDEBAR_KEY) === "1");
  /// 当前焦点窗格：main = 左侧主列（页面本体），review = 右侧校对区。
  /// 转写页的 OCR 选区只在校对区画一次，靠它决定"谁在操作选区"。
  const focusedPane = ref<"main" | "review">("main");

  function setFocusedPane(pane: "main" | "review") {
    focusedPane.value = pane;
  }

  function setTheme(next: ThemeName) {
    theme.value = next;
    localStorage.setItem(THEME_KEY, next);
    document.documentElement.dataset.theme = next;
  }

  function toggleTheme() {
    setTheme(isDark.value ? "light" : "dark");
  }

  function toggleSidebar() {
    sidebarCollapsed.value = !sidebarCollapsed.value;
    localStorage.setItem(SIDEBAR_KEY, sidebarCollapsed.value ? "1" : "0");
  }

  document.documentElement.dataset.theme = theme.value;

  return {
    theme,
    isDark,
    sidebarCollapsed,
    focusedPane,
    setTheme,
    toggleTheme,
    toggleSidebar,
    setFocusedPane,
  };
});
