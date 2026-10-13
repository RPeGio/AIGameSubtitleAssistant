// 融合审计表纯逻辑自检（R3）。
//
// fuseAudit.ts 是 TS，Node 不能直接跑：先用 tsc 转译到 temp/ 再运行本脚本：
//   node_modules\.bin\tsc.cmd src/stores/fuseAudit.ts --outDir temp --target es2022 --module esnext --moduleResolution bundler --skipLibCheck
//   node scripts/check_fuse_audit.mjs
//
// 覆盖：风险分层（A/B/normal 与 reviewed 优先级）、行构建（无 meta 事件被跳过、
// corpusText 映射与越界）、两种排序、计数条、四动作裁决（含 reused 重算、
// 候选分继承、全语料替换置 0 分、越界拒绝）、可逆性（不配 → 再替换回语料）。
import { strict as assert } from "node:assert";
import {
  auditRisk,
  buildAuditRows,
  sortRows,
  auditCounts,
  applyFusedDecision,
  refreshReused,
  LOW_SCORE_THR,
} from "../temp/fuseAudit.js";

const CORPUS = ["A句语料", "B句语料", "C句语料"];

function meta(over = {}) {
  return {
    ocr_index: 1,
    score: 0.9,
    candidates: [],
    masked: false,
    transition: "forward",
    reused: false,
    source_text: "转写原文",
    reviewed: false,
    ...over,
  };
}

function ev(id, start, metaOver = {}, text = "") {
  return { id, type: "fused", start, end: start + 1, text, fuse_meta: meta(metaOver) };
}

// ── 1. 风险分层 ──
assert.equal(auditRisk(meta()), "normal");
assert.equal(auditRisk(meta({ masked: true })), "A");
assert.equal(auditRisk(meta({ transition: "reset" })), "A");
assert.equal(auditRisk(meta({ transition: "repeat" })), "A");
assert.equal(auditRisk(meta({ ocr_index: 0, transition: "unmatched" })), "A");
assert.equal(auditRisk(meta({ reviewed: true, masked: true })), "normal", "已复核优先于结构信号");
assert.equal(auditRisk(meta({ score: 0.81 })), "B");
assert.equal(auditRisk(meta({ score: LOW_SCORE_THR })), "normal", "阈值是严格小于");
assert.equal(auditRisk(meta({ ocr_index: 0, transition: "unmatched", reviewed: true })), "normal");

// ── 2. 行构建 ──
const noMeta = { id: "x", type: "fused", start: 5, end: 6, text: "旧工程事件" };
const rows = buildAuditRows(
  [ev("e1", 2, { ocr_index: 2, score: 0.79 }), noMeta, ev("e2", 1, { ocr_index: 0, source_text: "原文" })],
  CORPUS
);
assert.equal(rows.length, 2, "无 fuse_meta 的事件不进表");
assert.deepEqual(rows.map((r) => r.event.id), ["e1", "e2"]);
assert.equal(rows[0].corpusText, "B句语料");
assert.equal(rows[1].corpusText, "", "未命中行无语料文本");
const oob = buildAuditRows([ev("e9", 0, { ocr_index: 99 })], CORPUS);
assert.equal(oob[0].corpusText, "", "越界 ocr_index 的语料文本为空串");

// ── 3. 排序（risk：A → B → normal；同层未复核在前；再按时间） ──
const mix = [
  ev("n2", 10, { reviewed: true }),               // normal 已复核
  ev("b1", 30, { ocr_index: 2, score: 0.79 }),    // B 未复核
  ev("a1", 20, { masked: true }),                 // A 未复核
  ev("n1", 40),                                   // normal 未复核
  ev("a2", 5, { masked: true, reviewed: true }),  // A 已复核 → 沉到 normal
  ev("b2", 3, { ocr_index: 1, score: 0.70, reviewed: true }), // B 已复核 → normal
];
const sorted = sortRows(buildAuditRows(mix, CORPUS), "risk").map((r) => r.event.id);
assert.deepEqual(sorted, ["a1", "b1", "n1", "b2", "a2", "n2"], "A → B → 未复核 normal → 已复核沉底");
const byTime = sortRows(buildAuditRows(mix, CORPUS), "time").map((r) => r.event.id);
assert.deepEqual(byTime, ["b2", "a2", "n2", "a1", "b1", "n1"], "time 模式纯时间轴");

// ── 4. 计数条 ──
const counts = auditCounts(buildAuditRows(mix, CORPUS));
assert.deepEqual(counts, { total: 6, focused: 1, low: 1, reviewed: 3 });

// ── 5. 四动作 ──
const work = [
  ev("w1", 0, { ocr_index: 1, candidates: [{ ocr_index: 1, score: 0.88 }, { ocr_index: 2, score: 0.79 }], source_text: "转写甲" }, "A句语料"),
  ev("w2", 1, { ocr_index: 1, candidates: [{ ocr_index: 1, score: 0.88 }, { ocr_index: 2, score: 0.79 }], source_text: "转写乙" }, "A句语料"),
  ev("w3", 2, { ocr_index: 0, transition: "unmatched", source_text: "转写丙" }, "转写丙"),
];
// keep：只置 reviewed
assert.equal(applyFusedDecision(work, "w1", CORPUS, { kind: "keep" }), null);
assert.equal(work[0].fuse_meta.reviewed, true);
assert.equal(work[0].text, "A句语料", "keep 不改文本");
// replace（候选内）：文本/ocr_index/分（继承候选实测分）/transition/reviewed，reused 重算
assert.equal(applyFusedDecision(work, "w2", CORPUS, { kind: "replace", ocrIndex: 2 }), null);
assert.equal(work[1].text, "B句语料");
assert.equal(work[1].fuse_meta.ocr_index, 2);
assert.equal(work[1].fuse_meta.score, 0.79, "候选内替换继承该行机器分");
assert.equal(work[1].fuse_meta.transition, "forward");
assert.equal(work[1].fuse_meta.reviewed, true);
assert.equal(work[0].fuse_meta.reused, false, "w1/w2 已不同行，reused 应清除");
// w3 与 w1 同行 → w1.reused=true 的重算由 refreshReused 负责；先造一次同行：
assert.equal(applyFusedDecision(work, "w2", CORPUS, { kind: "replace", ocrIndex: 1 }), null);
assert.equal(work[0].fuse_meta.reused, true, "w2 也配到 #1 ⇒ w1 的 reused 应为 true");
// replace（全语料选了候选外行）：分未知 → 0
assert.equal(applyFusedDecision(work, "w2", CORPUS, { kind: "replace", ocrIndex: 3 }), null);
assert.equal(work[2].fuse_meta.score !== undefined, true);
assert.equal(work[1].fuse_meta.score, 0, "候选外替换分数未知 → 0");
assert.equal(work[0].fuse_meta.reused, false);
// 越界替换：拒绝且不变更
const before = JSON.stringify(work[1]);
assert.notEqual(applyFusedDecision(work, "w2", CORPUS, { kind: "replace", ocrIndex: 9 }), null);
assert.equal(JSON.stringify(work[1]), before, "非法裁决不产生任何变更");
// unmatch：回退融合时转写原文 + ocr_index/score 清零
assert.equal(applyFusedDecision(work, "w3", CORPUS, { kind: "unmatch" }), null);
assert.equal(work[2].text, "转写丙");
assert.equal(work[2].fuse_meta.ocr_index, 0);
assert.equal(work[2].fuse_meta.reviewed, true);
// 可逆：不配后替换回语料
assert.equal(applyFusedDecision(work, "w3", CORPUS, { kind: "replace", ocrIndex: 3 }), null);
assert.equal(work[2].text, "C句语料");
// 旧数据保护：无 source_text 时拒绝回退；无 meta 时拒绝一切
const legacy = [ev("l1", 0, { source_text: "" })];
assert.notEqual(applyFusedDecision(legacy, "l1", CORPUS, { kind: "unmatch" }), null);
assert.notEqual(applyFusedDecision([{ id: "l2", type: "fused", start: 0, end: 1, text: "x" }], "l2", CORPUS, { kind: "keep" }), null);

// ── 6. refreshReused 独立可用 ──
const two = [
  ev("r1", 0, { ocr_index: 2 }),
  ev("r2", 1, { ocr_index: 2 }),
  ev("r3", 2, { ocr_index: 3 }),
];
refreshReused(two);
assert.equal(two[0].fuse_meta.reused, true);
assert.equal(two[1].fuse_meta.reused, true);
assert.equal(two[2].fuse_meta.reused, false);

console.log("check_fuse_audit: 全部断言通过 ✓");
