#!/usr/bin/env python3
"""T2 阶段 C：告警口径实测（score / margin 阈值）——**§7.9 ② 重标**。

产品形态的告警（`scripts/fuse_align_srt.py`）：`未命中` 或 `score < THR_SCORE` ⇒ 进"待人工确认"清单。
本脚本**只实测、不发明阈值**，但在**产品口径**下测（与产物逐段一致）：

    语料/段编码 → `mask_empty_body`（吸引子防护）→ 按素材的「该不配」行掩码
    → 统一转移 DP（skip=0.02 / unmatched=0.25 / repeat=0.25 / reset=0.05）

输出四件事：
  1. 现行阈值的告警体积 / 精确率 / 召回率（四案例 + vesna 单列）；
  2. `score` 阈值扫描表（标记率 / 精确率 / 召回率）；
  3. **口径对比**：「未命中 或 score<T」（产品现行组合）vs「仅 score<T」（单一阈值）；
  4. `margin` 的区分度**复检**（§4.2 的旧结论是否仍成立）。

**为什么必须重标**：§4.1~§4.3 的历史数字是在 **3 案例 / 严格递增 `align_fast` / 无掩码 / 旧真值**
下测的，当时工作点只有 1~4 个错误、`score` 告警"全为误报"。现在 vesna 已是四案例一等公民、
真值与素材修订过，错误样本充足 ⇒ 结论必须重测（本脚本输出即重测结果；历史记录不改写）。

**只有一种输入口径**（消费管线产出，不做输入卫生）——"互为前缀的相邻段"的合并属
OCR 合并层的 raw 缺陷，应在管线层修；曾按文本判据合并，已被 benchmark/OCR_PIPELINE_DEFECTS.md
**D12** 证否并删除。

用法：python scripts/fuse_alarm.py
"""
import io
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "tests"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.dont_write_bytecode = True  # 不写 __pycache__（与 fuse_calib / fuse_unmatch_calib 同源）
import numpy as np  # noqa: E402
from fuse_lab import BENCH_OUT, load_embedder  # noqa: E402
from fuse_calib import (DEFAULT, REPEAT_DEFAULT, RESET_DEFAULT,  # noqa: E402
                        align_v2, build_matrices, load_truth, mask_should_unmatched)
from fuse_align_srt import UNMATCH_PER_CASE  # noqa: E402  （掩码的**按素材开关**，单一来源）

# 告警阈值：**与 `fuse_align_srt.THR_SCORE` 必须一致**（§7.9 ② 重标：0.78 → 0.86）。
# 0.86 = 实测**召回饱和点**（最高分的错误 = 0.854）⇒ 召回 10/10；代价是标记率 89.4%
# （精确率 ≈ 错误基准率 4.2% ⇒ score 不是正确性信号，该清单等价于"全量复核"）。
# 旧值 0.78 见 HIST_THR（§4.2 的历史初值，只作对照标注）。
THR_SCORE = 0.86
HIST_THR = 0.78          # §4.2 的历史初值（仅用于对照标注）
THR_MARGIN = 0.02        # §4.2 已否决，仅复检
KEYS = ["moon", "glupov", "pierro", "vesna"]
THR_GRID = [0.60, 0.65, 0.70, 0.72, 0.74, 0.76, 0.78, 0.80, 0.82, 0.84,
            0.86, 0.88, 0.90, 0.92, 0.95, 1.01]
MARGIN_GRID = [0.001, 0.005, 0.01, 0.02, 0.03, 0.05]


def analyze(key, data):
    """→ 逐段记录（产品口径：掩码按 UNMATCH_PER_CASE 施加）。"""
    d = data[key]
    base = d["S"]                      # 已含 mask_empty_body 的吸引子防护
    S = base.copy()
    nmask = 0
    if UNMATCH_PER_CASE.get(key, False):
        nmask = mask_should_unmatched(S, d["seg_texts"], d["corpus"])
    m = align_v2(S, *DEFAULT, repeat_penalty=REPEAT_DEFAULT, reset_penalty=RESET_DEFAULT)
    pos = {k: n for n, k in enumerate(d["idxs"])}
    recs = []
    for k in d["scored"]:
        n = pos[k]
        pred = m[n] + 1
        ok = d["truth_ok"][k]
        if not ok:
            correct = pred <= 0
        else:
            correct = pred > 0 and any(d["cls"][pred - 1] == d["cls"][t - 1] for t in ok)
        row = base[n]
        valid = row > -1e8
        vr = row[valid]
        order = np.argsort(-vr)
        top1 = float(vr[order[0]]) if len(vr) else 0.0
        top2 = float(vr[order[1]]) if len(vr) > 1 else top1
        recs.append({
            "index": k, "start": d["starts"][k],
            "text": d["seg_texts"][k], "truth_ok": ok, "empty": not ok,
            "pred": pred, "correct": bool(correct),
            "score": float(row[pred - 1]) if pred > 0 else 0.0,
            "margin": top1 - top2,
        })
    return {"key": key, "nmask": nmask, "recs": recs}


def counts(recs, thr, rule="combo"):
    """→ (标记数, 标记中的真错数, 错误总数, 段数)

    rule="combo"：产品现行口径 = `未命中` 或 `已配 且 score<thr`；
    rule="score"：只看 score（`已配 且 score<thr`，未命中不计）。
    """
    errs = [r for r in recs if not r["correct"]]
    n = len(recs)
    flag = []
    for r in recs:
        f = r["pred"] > 0 and r["score"] < thr
        if rule == "combo" and r["pred"] <= 0:
            f = True
        if f:
            flag.append(r)
    hit = sum(1 for r in flag if not r["correct"])
    return len(flag), hit, len(errs), n


def sweep_table(title, recs, rule="combo"):
    print("   ── {} ──".format(title))
    print("      {:>7} {:>8} {:>8} {:>8} {:>9} {:>8}  {}".format(
        "阈值T", "标记数", "标记率", "其中真错", "精确率", "召回率", "备注"))
    rows = []
    for t in THR_GRID:
        nf, nh, ne, n = counts(recs, t, rule)
        prec = nh / nf * 100 if nf else float("nan")
        rec = nh / ne * 100 if ne else float("nan")
        note = ""
        if abs(t - HIST_THR) < 1e-9:
            note += "§4.2 历史初值 "
        if abs(t - THR_SCORE) < 1e-9:
            note += "★ 落地"
        print("      {:>7.2f} {:>8} {:>7.1f}% {:>8} {:>8.1f}% {:>7.1f}%  {}".format(
            t, nf, nf / n * 100, nh, prec, rec, note))
        rows.append({"thr": t, "flag": nf, "flag_rate": nf / n,
                     "flag_hit": nh, "errors": ne, "precision": prec, "recall": rec})
    return rows


def sweep_margin(recs):
    errs = [r for r in recs if not r["correct"]]
    n = len(recs)
    print("   ── margin 复检（口径 = 未命中 或 margin<T；§4.2 曾判「无区分度」）──")
    print("      {:>7} {:>8} {:>8} {:>8} {:>9} {:>8}".format(
        "阈值M", "标记数", "标记率", "其中真错", "精确率", "召回率"))
    for t in MARGIN_GRID:
        flag = [r for r in recs if r["pred"] <= 0 or r["margin"] < t]
        hit = sum(1 for r in flag if not r["correct"])
        print("      {:>7.3f} {:>8} {:>7.1f}% {:>8} {:>8.1f}% {:>7.1f}%".format(
            t, len(flag), len(flag) / n * 100, hit,
            hit / len(flag) * 100 if flag else float("nan"),
            hit / len(errs) * 100 if errs else float("nan")))
    if errs:
        srt = sorted(recs, key=lambda r: r["margin"])
        for pct in (0.05, 0.10, 0.20):
            kk = max(1, int(n * pct))
            got = sum(1 for r in srt[:kk] if not r["correct"])
            print("      按 margin 升序复核前 {:4.1f}%（{:3} 段）→ 捞回错误 {:2}/{:2} ({:5.1f}%)".format(
                pct * 100, kk, got, len(errs), got / len(errs) * 100))


def dist(recs):
    ok = sorted(r["score"] for r in recs if r["correct"])
    bad = sorted(r["score"] for r in recs if not r["correct"])
    def q(v, p):
        return v[min(len(v) - 1, int(len(v) * p))] if v else float("nan")
    return ("对 n={} p10 {:.3f} 中位 {:.3f} p90 {:.3f} | 错 n={} p10 {:.3f} 中位 {:.3f} p90 {:.3f}"
            .format(len(ok), q(ok, 0.1), q(ok, 0.5), q(ok, 0.9),
                    len(bad), q(bad, 0.1), q(bad, 0.5), q(bad, 0.9)))


def main():
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = build_matrices(KEYS, tok, sess)
    for k in KEYS:
        data[k]["starts"] = {i: r["start"] for i, r in enumerate(load_truth(k)["rows"])}
    res = {k: analyze(k, data) for k in KEYS}
    print("=" * 108)
    print("阶段 C：告警口径重标（§7.9 ②）——产品口径 = mask_empty_body + 统一转移 DP + 按素材掩码")
    print("=" * 108)
    print("  掩码开关（fuse_align_srt.UNMATCH_PER_CASE）：{}".format(UNMATCH_PER_CASE))
    print("  落地阈值 score<{:.2f}（与 fuse_align_srt.THR_SCORE 同值）；§4.2 历史初值 {:.2f}".format(
        THR_SCORE, HIST_THR))
    print()
    print("   {:<8} {:>5} {:>5} {:>6} {:>6}  现行口径：标记 / 其中真错 / 精确率".format(
        "案例", "段数", "错误", "未命中", "掩行"))
    tot = []
    for k in KEYS:
        recs = res[k]["recs"]
        tot += recs
        nf, nh, ne, n = counts(recs, THR_SCORE, "combo")
        print("   {:<8} {:>5} {:>5} {:>6} {:>6}  {:>4} / {:>3} / {:>5.1f}%".format(
            k, n, ne, sum(1 for r in recs if r["pred"] <= 0), res[k]["nmask"],
            nf, nh, nh / nf * 100 if nf else float("nan")))
        print("            score 分布（对 | 错）：{}".format(dist(recs)))
    nf, nh, ne, n = counts(tot, THR_SCORE, "combo")
    print("   {:<8} {:>5} {:>5} {:>6} {:>6}  {:>4} / {:>3} / {:>5.1f}%".format(
        "合计", n, ne, sum(1 for r in tot if r["pred"] <= 0),
        sum(res[k]["nmask"] for k in KEYS), nf, nh,
        nh / nf * 100 if nf else float("nan")))
    print("            score 分布（对 | 错）：{}".format(dist(tot)))
    print()
    print("   ⇒ 落地 score<{:.2f} 召回 {}/{} = {:.1f}%（精确率 {:.1f}%，错误基准率 {:.1f}%）；".format(
        THR_SCORE, nh, ne, nh / ne * 100 if ne else float("nan"),
        nh / nf * 100 if nf else float("nan"), ne / n * 100))
    miss = [r["score"] for r in tot if not r["correct"] and r["pred"] > 0
            and r["score"] >= HIST_THR]
    print("      §4.2 初值 score<{:.2f} 只召回 {}/{} = {:.1f}%：漏掉的 {} 个错误 score ∈ [{:.3f}, {:.3f}]".format(
        HIST_THR, counts(tot, HIST_THR, "combo")[1], ne,
        counts(tot, HIST_THR, "combo")[1] / ne * 100 if ne else float("nan"),
        len(miss), min(miss, default=float("nan")), max(miss, default=float("nan"))))
    print("      ⇒ 精确率在任何阈值下都 ≈ 基准率 ⇒ **score 不是正确性信号**；0.86 的清单实际是「全量复核」。")
    print()
    rows_tot = sweep_table("A. 合计（四案例 {} 计分段）：口径 = 未命中 或 score<T".format(len(tot)), tot, "combo")
    print()
    rows_v = sweep_table("B. vesna 单列（{} 段）：口径 = 未命中 或 score<T".format(
        len(res["vesna"]["recs"])), res["vesna"]["recs"], "combo")
    print()
    rows_s = sweep_table("C. 合计：口径 = 仅 score<T（未命中不计）——「单一阈值够不够」", tot, "score")
    print()
    print("   ── D. 口径对比（同一 T 下：组合口径 vs 仅 score）──")
    print("      {:>7} {:>22} {:>22}".format("T", "组合(未命中或score<T)", "仅 score<T"))
    for t in (0.78, 0.82, 0.84, 0.86, 0.88, 0.90):
        a = counts(tot, t, "combo")
        b = counts(tot, t, "score")
        print("      {:>7.2f} 标记{:>4} 真错{:>3} 精确{:>5.1f}% 召回{:>5.1f}%   "
              "标记{:>4} 真错{:>3} 精确{:>5.1f}% 召回{:>5.1f}%".format(
                  t, a[0], a[1], a[1] / a[0] * 100 if a[0] else float("nan"),
                  a[1] / a[2] * 100 if a[2] else float("nan"),
                  b[0], b[1], b[1] / b[0] * 100 if b[0] else float("nan"),
                  b[1] / b[2] * 100 if b[2] else float("nan")))
    print()
    print("   ── E. 逐案例（落地阈值 T={:.2f}，口径 = 未命中 或 score<T）──".format(THR_SCORE))
    for k in KEYS:
        nf, nh, ne, n = counts(res[k]["recs"], THR_SCORE, "combo")
        print("      {:<8} 段{:>4} 错误{:>3} → 标记{:>4}（{:5.1f}%）真错{:>3} 精确{:>5.1f}% 召回{:>5.1f}%".format(
            k, n, ne, nf, nf / n * 100, nh, nh / nf * 100 if nf else float("nan"),
            nh / ne * 100 if ne else float("nan")))
    print()
    sweep_margin(tot)
    print()
    print("   ── G. 按 score 升序的「复核预算」曲线（口径 = 仅已配段按 score 排序）──")
    matched = [r for r in tot if r["pred"] > 0]
    merrs = [r for r in matched if not r["correct"]]
    for pct in (0.05, 0.10, 0.20, 0.30, 0.50):
        kk = max(1, int(len(matched) * pct))
        got = sum(1 for r in sorted(matched, key=lambda r: r["score"])[:kk] if not r["correct"])
        print("      前 {:4.1f}%（{:3} 段）→ 捞回错误 {:2}/{:2} ({:5.1f}%)".format(
            pct * 100, kk, got, len(merrs), got / len(merrs) * 100 if merrs else float("nan")))
    print()
    print("   ── H. 现行阈值漏掉的错误（score ≥ {:.2f}，清单看不到）──".format(HIST_THR))
    print("      {:<8} {:>9} {:>7} {:>5} {:>6}  {}".format(
        "案例", "t(s)", "score", "pred", "空集", "转写文本"))
    for k in KEYS:
        for r in res[k]["recs"]:
            if not r["correct"] and (r["pred"] <= 0 or r["score"] >= HIST_THR):
                print("      {:<8} {:>9.2f} {:>7.3f} {:>5} {:>6}  {}".format(
                    k, r["start"], r["score"], r["pred"], "是" if r["empty"] else "否",
                    r["text"].replace("\n", " / ")[:44]))
    print()
    print("   ── I. vesna 敏感性：掩码关（默认版产物）下的同一扫描 ──")
    d = data["vesna"]
    base = d["S"]
    m = align_v2(base.copy(), *DEFAULT, repeat_penalty=REPEAT_DEFAULT,
                 reset_penalty=RESET_DEFAULT)
    pos = {k: n for n, k in enumerate(d["idxs"])}
    recs_nomask = []
    for k in d["scored"]:
        n = pos[k]
        pred = m[n] + 1
        ok = d["truth_ok"][k]
        correct = (pred <= 0) if not ok else (
            pred > 0 and any(d["cls"][pred - 1] == d["cls"][t - 1] for t in ok))
        recs_nomask.append({"pred": pred, "correct": bool(correct),
                            "score": float(base[n][pred - 1]) if pred > 0 else 0.0})
    rows_nm = sweep_table("vesna（掩码关，{} 段，错误 {}）".format(
        len(recs_nomask), sum(1 for r in recs_nomask if not r["correct"])),
        recs_nomask, "combo")
    print()
    p = os.path.join(BENCH_OUT, "alarm_report.json")
    with io.open(p, "w", encoding="utf-8", newline="") as f:
        json.dump({"config": {"thr_score": THR_SCORE, "hist_thr": HIST_THR,
                              "unmatch_per_case": UNMATCH_PER_CASE,
                              "dp": {"skip": DEFAULT[0], "unmatched": DEFAULT[1],
                                     "repeat": REPEAT_DEFAULT, "reset": RESET_DEFAULT}},
                   "per_case": {k: {"n": len(res[k]["recs"]),
                                    "errors": sum(1 for r in res[k]["recs"] if not r["correct"]),
                                    "nmask": res[k]["nmask"]} for k in KEYS},
                   "sweep_total_combo": rows_tot, "sweep_vesna_combo": rows_v,
                   "sweep_total_score_only": rows_s, "sweep_vesna_nomask": rows_nm},
                  f, ensure_ascii=False, indent=1)
    print("已落盘 {}".format(p))
    return 0


if __name__ == "__main__":
    sys.exit(main())
