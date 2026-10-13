// ─── 融合审计表：纯逻辑（无 Vue 依赖，可被脚本导入自检）──────
// 数据源 = fused 轨 `events[].fuse_meta`（已持久化，工程重开可见）。
// 风险分层依据（benchmark/FUSE_THRESHOLD_CALIBRATION.md §7.9）：
// - score 不是正确性信号（任何阈值精确率 ≈ 基准率 4.2%）⇒ score<0.82 只作
//   折叠标记与排序辅助（B 层），**不是告警判据**；
// - 结构信号（masked / reset / repeat / 未命中）才是重点罗列（A 层）；
//   `reused` 在长句多段素材（vesna 68/82）是常态，只作信息标记、不参与分层。
// 分层含义：A = 需要立刻裁决（默认展开）；B = 值得复核（折叠单行）；已复核
// （reviewed）视为已处理（normal），计数随裁决清零，形成"剩余工作量"进度条。

import type { FuseMeta, FusedEvent } from "../types";

/// 「低分」折叠标记阈值（§7.9⑥：T=0.82 标记 50.4%/召回 60%；替代值 0.80 → 22.5%/40%。
/// 仅展示与排序辅助，不是告警，勿当门限用）
export const LOW_SCORE_THR = 0.82;

/// 审计风险分层：A = 重点（默认展开）| B = 低分（折叠标记）| normal = 正常/已复核
export type AuditRisk = "A" | "B" | "normal";

/// 单行审计数据（meta 必须存在才构成一行；无 fuse_meta 的事件不进表）
export interface FuseAuditRow {
  event: FusedEvent;
  meta: FuseMeta;
  /// 当前命中的语料原文（ocr_index 越界或未命中时为空串）
  corpusText: string;
  risk: AuditRisk;
}

/// 审计计数（风险模式下的"剩余工作量"：未复核的重点/低分）
export interface AuditCounts {
  total: number;
  focused: number;
  low: number;
  reviewed: number;
}

export type AuditSortMode = "risk" | "time";

/// 风险分层：已复核 → normal；结构信号 → A；低分命中 → B；其余 → normal
export function auditRisk(meta: FuseMeta): AuditRisk {
  if (meta.reviewed) return "normal";
  if (meta.masked) return "A";
  if (meta.transition === "reset" || meta.transition === "repeat") return "A";
  if (meta.ocr_index <= 0) return "A";
  if (meta.score < LOW_SCORE_THR) return "B";
  return "normal";
}

/// 从 fused 轨事件构建审计行（只收有 fuse_meta 的事件，旧工程融合产物为空表）
export function buildAuditRows(
  events: readonly FusedEvent[],
  corpusTexts: readonly string[]
): FuseAuditRow[] {
  const rows: FuseAuditRow[] = [];
  for (const event of events) {
    const meta = event.fuse_meta;
    if (!meta) continue;
    const corpusText =
      meta.ocr_index >= 1 && meta.ocr_index <= corpusTexts.length
        ? corpusTexts[meta.ocr_index - 1]
        : "";
    rows.push({ event, meta, corpusText, risk: auditRisk(meta) });
  }
  return rows;
}

/// 排序。risk 模式：A → B → normal（已复核在 normal 内沉底）；
/// 同层内未复核在前、再按时间轴；time 模式：纯时间轴。
export function sortRows(rows: FuseAuditRow[], mode: AuditSortMode): FuseAuditRow[] {
  const start = (r: FuseAuditRow) => r.event.start;
  const reviewed = (r: FuseAuditRow) => (r.meta.reviewed ? 1 : 0);
  const tier = (r: FuseAuditRow) =>
    r.risk === "A" ? 0 : r.risk === "B" ? 1 : 2;
  if (mode === "time") {
    return [...rows].sort((x, y) => start(x) - start(y));
  }
  return [...rows].sort(
    (x, y) =>
      tier(x) - tier(y) || reviewed(x) - reviewed(y) || start(x) - start(y)
  );
}

/// 计数条：total = 全部审计行；focused/low = 未复核的 A/B；reviewed = 已复核数
export function auditCounts(rows: readonly FuseAuditRow[]): AuditCounts {
  let focused = 0;
  let low = 0;
  let reviewed = 0;
  for (const r of rows) {
    if (r.risk === "A") focused += 1;
    else if (r.risk === "B") low += 1;
    if (r.meta.reviewed) reviewed += 1;
  }
  return { total: rows.length, focused, low, reviewed };
}

export type FuseAuditDecision =
  | { kind: "keep" }
  | { kind: "replace"; ocrIndex: number }
  | { kind: "unmatch" };

/// 命中的语料行在候选名单里的实测分（人工替换时继承该行机器分，而非置 0；
/// 候选名单没有该行 = 从全语料选择，分数未知 → 0）
function candidateScore(meta: FuseMeta, ocrIndex: number): number {
  const hit = meta.candidates.find((c) => c.ocr_index === ocrIndex);
  return hit ? hit.score : 0;
}

/// 重算 reused：它是"当前 ocr_index 分布"的纯函数，人工改判后必须刷新（否则展示撒谎）
export function refreshReused(events: readonly FusedEvent[]): void {
  const counts = new Map<number, number>();
  for (const e of events) {
    const oi = e.fuse_meta?.ocr_index ?? 0;
    if (oi >= 1) counts.set(oi, (counts.get(oi) ?? 0) + 1);
  }
  for (const e of events) {
    const meta = e.fuse_meta;
    if (!meta) continue;
    meta.reused = meta.ocr_index >= 1 && (counts.get(meta.ocr_index) ?? 0) >= 2;
  }
}

/// 应用一次审计裁决（就地修改事件；store 侧负责快照与落盘）。
/// 返回 null = 成功；否则返回给 UI 的提示文案（未找到 / 非法裁决 / 数据不支持）。
export function applyFusedDecision(
  events: readonly FusedEvent[],
  eventId: string,
  corpusTexts: readonly string[],
  decision: FuseAuditDecision
): string | null {
  const event = events.find((e) => e.id === eventId);
  const meta = event?.fuse_meta;
  if (!event || !meta) return "该事件无审计元数据（请重跑一次融合）";
  if (decision.kind === "keep") {
    meta.reviewed = true;
    return null;
  }
  if (decision.kind === "replace") {
    const oi = decision.ocrIndex;
    if (!Number.isInteger(oi) || oi < 1 || oi > corpusTexts.length) {
      return "替换目标语料行越界";
    }
    event.text = corpusTexts[oi - 1];
    meta.ocr_index = oi;
    meta.score = candidateScore(meta, oi);
    meta.transition = "forward";
    meta.masked = false;
    meta.reviewed = true;
    refreshReused(events);
    return null;
  }
  // unmatch：回退到融合时刻的转写原文（source_text 为空 = R2 时代数据，无法回退）
  if (!meta.source_text) {
    return "该工程无转写原文记录（旧版融合产出），无法回退——请重跑一次融合";
  }
  event.text = meta.source_text;
  meta.ocr_index = 0;
  meta.score = 0;
  meta.transition = "unmatched";
  meta.masked = false;
  meta.reviewed = true;
  refreshReused(events);
  return null;
}
