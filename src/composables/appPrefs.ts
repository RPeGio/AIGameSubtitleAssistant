/// 界面偏好（localStorage）：设置面板负责写，其它 store 只读。
/// 单独成模块，避免 project / timeline / ui 三个 store 为了读一个开关互相 import。
import { invoke } from "@tauri-apps/api/core";

export const PREF_KEYS = {
  theme: "gsa.theme",
  sidebar: "gsa.sidebar",
  /// 侧栏默认折叠（仅在还没有用户手动折叠记录时生效）
  sidebarDefault: "gsa.sidebarDefault",
  zoom: "gsa.zoom",
  autosave: "gsa.autosave",
  autosaveDelay: "gsa.autosaveDelay",
  snap: "gsa.timeline.snap",
  splitStrategy: "gsa.timeline.splitStrategy",
  mergeStrategy: "gsa.timeline.mergeStrategy",
} as const;

export const ZOOM_VALUES = [0.9, 1, 1.1, 1.25] as const;
export const AUTOSAVE_DELAYS = [300, 1000, 3000] as const;

function readNumber(key: string, fallback: number): number {
  const raw = localStorage.getItem(key);
  const n = raw === null ? Number.NaN : Number(raw);
  return Number.isFinite(n) ? n : fallback;
}

function readBool(key: string, fallback: boolean): boolean {
  const raw = localStorage.getItem(key);
  if (raw === null) return fallback;
  return raw === "1" || raw === "true";
}

// ── 界面缩放 ────────────────────────────────────────────
export function readZoom(): number {
  const z = readNumber(PREF_KEYS.zoom, 1);
  return (ZOOM_VALUES as readonly number[]).includes(z) ? z : 1;
}

/// 界面缩放走 WebView 自己的 zoom（Rust 命令 `set_ui_zoom`），不用 CSS `zoom`。
///
/// CSS zoom 只缩放内容，`100vh` 这类视口单位不跟着变：缩到 90% 时应用底部空出
/// 一条、放大到 125% 时内容溢出把文字挤着换行。WebView 缩放改的是 CSS 像素换算比，
/// 视口单位随之正确。
export async function applyZoom(zoom: number) {
  try {
    await invoke("set_ui_zoom", { scale: zoom });
  } catch (e) {
    // 非 Tauri 环境（vite preview / 单测）没有这个命令，忽略即可
    console.warn("设置界面缩放失败:", e);
  }
}

export async function setZoom(zoom: number) {
  localStorage.setItem(PREF_KEYS.zoom, String(zoom));
  await applyZoom(zoom);
}

// ── 自动保存 ────────────────────────────────────────────
export function readAutosaveEnabled(): boolean {
  return readBool(PREF_KEYS.autosave, true);
}

export function setAutosaveEnabled(on: boolean) {
  localStorage.setItem(PREF_KEYS.autosave, on ? "1" : "0");
}

export function readAutosaveDelay(): number {
  const d = readNumber(PREF_KEYS.autosaveDelay, 1000);
  return (AUTOSAVE_DELAYS as readonly number[]).includes(d) ? d : 1000;
}

export function setAutosaveDelay(delay: number) {
  localStorage.setItem(PREF_KEYS.autosaveDelay, String(delay));
}

// ── 侧栏默认折叠 ────────────────────────────────────────
export function readSidebarDefault(): boolean {
  return readBool(PREF_KEYS.sidebarDefault, false);
}

export function setSidebarDefault(collapsed: boolean) {
  localStorage.setItem(PREF_KEYS.sidebarDefault, collapsed ? "1" : "0");
}

// ── 时间轴吸附默认 ──────────────────────────────────────
export function readSnapDefault(): boolean {
  return readBool(PREF_KEYS.snap, true);
}

export function setSnapDefault(on: boolean) {
  localStorage.setItem(PREF_KEYS.snap, on ? "1" : "0");
}
