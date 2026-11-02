/// 界面偏好（localStorage）：设置面板负责写，其它 store 只读。
/// 单独成模块，避免 project / timeline / ui 三个 store 为了读一个开关互相 import。
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

/// WebView2 是 Chromium，`zoom` 属性可用；它连 fixed 遮罩一起缩放，
/// 比逐个改根字号省事（本项目样式全是 px，改字号没用）
export function applyZoom(zoom: number) {
  document.documentElement.style.zoom = zoom === 1 ? "" : String(zoom);
}

export function setZoom(zoom: number) {
  localStorage.setItem(PREF_KEYS.zoom, String(zoom));
  applyZoom(zoom);
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
