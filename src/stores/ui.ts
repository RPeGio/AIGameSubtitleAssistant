import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  PREF_KEYS,
  applyZoom,
  readSidebarDefault,
  readZoom,
  setSidebarDefault,
  setZoom as persistZoom,
} from "../composables/appPrefs";

export type ThemeName = "dark" | "light";

const THEME_KEY = PREF_KEYS.theme;
const SIDEBAR_KEY = PREF_KEYS.sidebar;

function readTheme(): ThemeName {
  return localStorage.getItem(THEME_KEY) === "light" ? "light" : "dark";
}

function readSidebarCollapsed(): boolean {
  // 没有手动折叠记录时，用设置面板里的「默认折叠」
  const raw = localStorage.getItem(SIDEBAR_KEY);
  return raw === null ? readSidebarDefault() : raw === "1";
}

/** 外壳状态：主题写 <html data-theme>（令牌层随属性翻转）、侧栏折叠状态、界面缩放 */
export const useUiStore = defineStore("ui", () => {
  const theme = ref<ThemeName>(readTheme());
  const isDark = computed(() => theme.value === "dark");
  const sidebarCollapsed = ref(readSidebarCollapsed());
  /// 界面缩放（0.9 / 1 / 1.1 / 1.25）
  const zoom = ref(readZoom());
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

  function setSidebarCollapsed(collapsed: boolean) {
    sidebarCollapsed.value = collapsed;
    localStorage.setItem(SIDEBAR_KEY, collapsed ? "1" : "0");
  }

  function setZoom(next: number) {
    zoom.value = next;
    void persistZoom(next);
  }

  /// 侧栏「默认折叠」是给下次启动用的，当前状态不动（避免用户点一下就跳）
  function setSidebarDefaultCollapsed(collapsed: boolean) {
    setSidebarDefault(collapsed);
  }

  document.documentElement.dataset.theme = theme.value;
  void applyZoom(zoom.value);

  return {
    theme,
    isDark,
    sidebarCollapsed,
    zoom,
    focusedPane,
    setTheme,
    toggleTheme,
    toggleSidebar,
    setSidebarCollapsed,
    setZoom,
    setSidebarDefaultCollapsed,
    setFocusedPane,
  };
});
