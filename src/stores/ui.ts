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

  return { theme, isDark, sidebarCollapsed, setTheme, toggleTheme, toggleSidebar };
});
