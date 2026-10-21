/// 快捷键注册表与分发。
///
/// 键位只在这里定义：设置面板（第 19 项）与帮助面板将来同源读取这张表，
/// 避免出现「文案与实现不符」。分发走单一 window 监听（ProjectLayout 挂载），
/// 组件按 id 注册处理器，不再各自 addEventListener——多个时间轴实例在场时，
/// 用「鼠标最后进入的那个」判定活动时间轴，避免两条时间轴同时响应一次按键。
export type ShortcutScope = "global" | "timeline" | "player";

export interface ShortcutDef {
  id: string;
  group: string;
  label: string;
  /// 规范写法，见 shortcutKey()；一个动作可绑多个键位
  keys: string[];
  scope: ShortcutScope;
}

export const SHORTCUT_GROUPS = ["全局", "播放", "工具", "时间轴", "编辑"];

/// 键位取舍见 MISC_TODO.md 第 10 项：吸附用 N（避让 S），S 保留为播放头处分割；
/// J/K/L 只做单速（倒放 / 停止 / 正放），不做连按加速。
export const SHORTCUTS: ShortcutDef[] = [
  { id: "save", group: "全局", label: "保存工程", keys: ["Ctrl+S"], scope: "global" },
  { id: "undo", group: "全局", label: "撤销", keys: ["Ctrl+Z"], scope: "global" },
  { id: "redo", group: "全局", label: "重做", keys: ["Ctrl+Y", "Ctrl+Shift+Z"], scope: "global" },

  { id: "playPause", group: "播放", label: "播放 / 暂停", keys: ["Space"], scope: "player" },
  { id: "reverse", group: "播放", label: "倒放（1×）", keys: ["J"], scope: "player" },
  { id: "stopPlay", group: "播放", label: "停止播放并恢复正放", keys: ["K"], scope: "player" },
  { id: "forward", group: "播放", label: "正放（1×）", keys: ["L"], scope: "player" },

  { id: "toolSelect", group: "工具", label: "选择工具", keys: ["V"], scope: "timeline" },
  { id: "toolSplit", group: "工具", label: "分割工具", keys: ["C"], scope: "timeline" },
  { id: "toggleSnap", group: "工具", label: "吸附开关", keys: ["N"], scope: "timeline" },

  { id: "frameStep", group: "时间轴", label: "逐帧步进（被分割态改为搬句界）", keys: ["ArrowLeft", "ArrowRight"], scope: "timeline" },
  { id: "secondStep", group: "时间轴", label: "前后 1 秒", keys: ["Shift+ArrowLeft", "Shift+ArrowRight"], scope: "timeline" },
  { id: "docStartEnd", group: "时间轴", label: "片头 / 片尾", keys: ["Home", "End"], scope: "timeline" },
  { id: "prevNextClip", group: "时间轴", label: "上 / 下一个片段", keys: [",", "."], scope: "timeline" },
  { id: "zoom", group: "时间轴", label: "放大 / 缩小", keys: ["=", "-"], scope: "timeline" },
  { id: "zoomFit", group: "时间轴", label: "缩放至全部", keys: ["\\"], scope: "timeline" },

  { id: "splitAtPlayhead", group: "编辑", label: "在播放头处分割", keys: ["S"], scope: "timeline" },
  { id: "mergeNext", group: "编辑", label: "合并相邻片段", keys: ["M"], scope: "timeline" },
  { id: "delete", group: "编辑", label: "删除选中片段（多选时整批）", keys: ["Delete"], scope: "timeline" },
  { id: "selectAll", group: "编辑", label: "全选活动时间轴的片段", keys: ["Ctrl+A"], scope: "timeline" },
  { id: "exitSplitPair", group: "编辑", label: "退出被分割态", keys: ["Escape"], scope: "timeline" },
];

/// 事件 → 规范键位串：Ctrl+Shift+Z / Space / ArrowLeft / S / = / , / \
export function shortcutKey(e: KeyboardEvent): string {
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  let key = e.key;
  if (key === " " || key === "Spacebar") key = "Space";
  else if (key.length === 1) key = key.toUpperCase();
  parts.push(key);
  return parts.join("+");
}

const DEF_BY_KEY = new Map<string, ShortcutDef>();
for (const def of SHORTCUTS) {
  for (const k of def.keys) {
    const prev = DEF_BY_KEY.get(k);
    if (prev && prev.id !== def.id) {
      // 注册表自身冲突：只警告，先注册者生效（实现期发现，别让它静默）
      console.warn(`[shortcuts] 键位冲突：${k}（${prev.id} / ${def.id}）`);
    } else {
      DEF_BY_KEY.set(k, def);
    }
  }
}

interface Registration {
  id: string;
  run: (e: KeyboardEvent) => void;
  active?: () => boolean;
}

const registrations: Registration[] = [];

/// 注册某动作的处理器；返回注销函数（组件在 onUnmounted 调用）
export function onShortcut(
  id: string,
  run: (e: KeyboardEvent) => void,
  active?: () => boolean
): () => void {
  const reg: Registration = { id, run, active };
  registrations.push(reg);
  return () => {
    const i = registrations.indexOf(reg);
    if (i >= 0) registrations.splice(i, 1);
  };
}

// 活动时间轴：鼠标最后进入的那个；从未进入过则默认「主时间轴」（校对区）
let hoveredTimeline: object | null = null;

export function markHoveredTimeline(owner: object | null) {
  hoveredTimeline = owner;
}

export function isActiveTimeline(owner: object, primary: boolean): boolean {
  return hoveredTimeline ? hoveredTimeline === owner : primary;
}

/// 分发一次按键；返回是否命中（调用方据此 preventDefault）
export function dispatchShortcut(e: KeyboardEvent): boolean {
  const def = DEF_BY_KEY.get(shortcutKey(e));
  if (!def) return false;
  let hit = false;
  for (const reg of registrations) {
    if (reg.id !== def.id) continue;
    if (reg.active && !reg.active()) continue;
    reg.run(e);
    hit = true;
  }
  return hit;
}

/// 输入目标判定：输入框/文本域/contentEditable 里不劫持按键
export function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable;
}
