import { defineStore } from "pinia";
import { computed, ref } from "vue";

export type ThemeName = "dark" | "light";

const STORAGE_KEY = "gsa.theme";

function readTheme(): ThemeName {
  return localStorage.getItem(STORAGE_KEY) === "light" ? "light" : "dark";
}

/** 主题状态：把 data-theme 写到 <html>，令牌层随属性翻转 */
export const useUiStore = defineStore("ui", () => {
  const theme = ref<ThemeName>(readTheme());
  const isDark = computed(() => theme.value === "dark");

  function setTheme(next: ThemeName) {
    theme.value = next;
    localStorage.setItem(STORAGE_KEY, next);
    document.documentElement.dataset.theme = next;
  }

  function toggleTheme() {
    setTheme(isDark.value ? "light" : "dark");
  }

  document.documentElement.dataset.theme = theme.value;

  return { theme, isDark, setTheme, toggleTheme };
});
