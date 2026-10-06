#!/usr/bin/env python3
"""T2 阶段 C：告警口径实测（现行 margin / score 阈值是否可用）。

现行告警（来自 benchmark/FUSE_VECTOR_RECALL_VALIDATION.md §三 ⑤ 的产品形态）：
    score < 0.78 或 margin(=top1−top2) < 0.02  ⇒ 进"待人工确认"清单
报告 §五-3 已指出这两值是**未校准初值**、歧义告警偏多。

本脚本不发明新阈值，只**实测**三件事：
  1. 告警**体积**：按现行阈值会标记多少段（回答"告警偏多"到底多多少）；
  2. 告警**精确率**：被标记的段里真正错的有几段（标签来自独立真值）；
  3. 告警**区分度**：margin / score 在"对/错"两组上的分布差异，以及按 margin 升序
     排序时"复核预算 5%/10%/20% 能捞回多少错误"。

注意：等价类口径评分（见 fuse_calib.equiv_classes）。错误样本很少，故精确率/召回的数字
**置信度低**，脚本会显式标注样本量。

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
import numpy as np  # noqa: E402
from fuse_lab import BENCH_OUT, load_embedder, similarity_matrix, split_header  # noqa: E402
from fuse_calib import (DEFAULT, align_fast,  # noqa: E402
                        equiv_classes, load_truth)

THR_SCORE = 0.78
THR_MARGIN = 0.02


def analyze(key, tok, sess, verbose=True):
    T = load_truth(key)
    corpus, rows = T["corpus"], T["rows"]
    truth = [r["truth"] for r in rows]
    cls = equiv_classes(corpus)
    idxs = list(range(len(rows)))
    cb = [split_header(t)[1] for t in corpus]
    sb = [split_header(rows[k]["text"])[1] for k in idxs]
    S = similarity_matrix(tok, sess, cb, sb)
    m = align_fast(S, *DEFAULT)

    recs = []
    for n, k in enumerate(idxs):
        pred = m[n] + 1
        t = truth[k]
        correct = pred > 0 and t > 0 and cls[pred - 1] == cls[t - 1]
        row = S[n]
        order = np.argsort(-row)
        top1 = float(row[order[0]])
        top2 = float(row[order[1]]) if len(order) > 1 else 0.0
        margin = top1 - top2
        recs.append({
            "index": rows[k]["index"], "correct": bool(correct),
            "pred": pred, "truth": t,
            "score": float(row[pred - 1]) if pred > 0 else 0.0,
            "top1": top1, "margin": margin,
            "flag_score": (float(row[pred - 1]) if pred > 0 else 0.0) < THR_SCORE,
            "flag_margin": margin < THR_MARGIN,
        })
    return {"key": key, "n": len(recs), "recs": recs}


def report(res, verbose=True):
    recs = res["recs"]
    n = len(recs)
    errs = [r for r in recs if not r["correct"]]
    f_m = [r for r in recs if r["flag_margin"]]
    f_s = [r for r in recs if r["flag_score"]]
    f_any = [r for r in recs if r["flag_margin"] or r["flag_score"]]
    print("  {:<8} n={:3}  错误={:2}".format(res["key"], n, len(errs)))
    print("      margin<{:.2f} 标记 {:3} 段（{:4.1f}%）其中真错 {:2} → 精确率 {:5.1f}% 召回 {:5.1f}%".format(
        THR_MARGIN, len(f_m), len(f_m) / n * 100,
        sum(1 for r in f_m if not r["correct"]),
        (sum(1 for r in f_m if not r["correct"]) / len(f_m) * 100) if f_m else float("nan"),
        (sum(1 for r in f_m if not r["correct"]) / len(errs) * 100) if errs else float("nan")))
    print("      score <{:.2f} 标记 {:3} 段（{:4.1f}%）其中真错 {:2} → 精确率 {:5.1f}%".format(
        THR_SCORE, len(f_s), len(f_s) / n * 100,
        sum(1 for r in f_s if not r["correct"]),
        (sum(1 for r in f_s if not r["correct"]) / len(f_s) * 100) if f_s else float("nan")))
    print("      任一标记      {:3} 段（{:4.1f}%）".format(len(f_any), len(f_any) / n * 100))

    # 分布：对 / 错
    def stat(rs, f):
        if not rs:
            return "-"
        v = sorted(f(r) for r in rs)
        return "中位{:.4f} p10 {:.4f} p90 {:.4f}".format(
            v[len(v) // 2], v[max(0, int(len(v) * 0.1))], v[min(len(v) - 1, int(len(v) * 0.9))])
    ok = [r for r in recs if r["correct"]]
    print("      margin 分布：对 {} | 错 {}".format(stat(ok, lambda r: r["margin"]),
                                                   stat(errs, lambda r: r["margin"])))
    print("      score  分布：对 {} | 错 {}".format(stat(ok, lambda r: r["score"]),
                                                   stat(errs, lambda r: r["score"])))

    # 复核预算：按 margin 升序取前 k%
    if errs:
        srt = sorted(recs, key=lambda r: r["margin"])
        for pct in (0.05, 0.10, 0.20):
            kk = max(1, int(n * pct))
            got = sum(1 for r in srt[:kk] if not r["correct"])
            print("      按 margin 升序复核前 {:4.1f}%（{:3} 段）→ 捞回错误 {:2}/{:2} ({:5.1f}%)".format(
                pct * 100, kk, got, len(errs), got / len(errs) * 100))
    return {
        "key": res["key"], "n": n, "errors": len(errs),
        "flag_margin": len(f_m), "flag_score": len(f_s), "flag_any": len(f_any),
        "flag_margin_hit": sum(1 for r in f_m if not r["correct"]),
        "flag_score_hit": sum(1 for r in f_s if not r["correct"]),
    }


def sweep(recs, field, thresholds, label):
    """阈值扫描：给出各阈值下的标记率 / 精确率 / 召回率"""
    errs = [r for r in recs if not r["correct"]]
    n = len(recs)
    print("      {} 阈值扫描（n={}，错误={}）".format(label, n, len(errs)))
    print("        {:>7} {:>7} {:>8} {:>8} {:>8}".format("阈值", "标记率", "标记数", "精确率", "召回率"))
    for th in thresholds:
        f = [r for r in recs if r[field] < th]
        hit = sum(1 for r in f if not r["correct"])
        prec = hit / len(f) * 100 if f else float("nan")
        rec = hit / len(errs) * 100 if errs else float("nan")
        print("        {:>7.3f} {:>6.1f}% {:>8} {:>7.1f}% {:>7.1f}%".format(
            th, len(f) / n * 100, len(f), prec, rec))


def main():
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    keys = ["moon", "glupov", "pierro"]
    print("=" * 100)
    print("阶段 C：告警口径实测（阈值 score<{:.2f} / margin<{:.2f}，均为未校准初值）".format(
        THR_SCORE, THR_MARGIN))
    out = {}
    for key in keys:
        res = analyze(key, tok, sess)
        out[key] = report(res)
        if key == "pierro":
            print()
            sweep(res["recs"], "score", [0.70, 0.75, 0.78, 0.79, 0.80, 0.81, 0.82], "score")
            sweep(res["recs"], "margin", [0.001, 0.005, 0.01, 0.02, 0.03, 0.05], "margin")
    # 合计
    print()
    print("── 合计 ──")
    tot = {"n": 0, "errors": 0, "flag_margin": 0, "flag_margin_hit": 0,
           "flag_score": 0, "flag_score_hit": 0, "flag_any": 0}
    for key in keys:
        r = out[key]
        for f in tot:
            tot[f] += r[f]
    print("  段数 {}  错误 {}  margin 标记 {}（{:.1f}%）其中真错 {}  score 标记 {} 任一 {}（{:.1f}%）".format(
        tot["n"], tot["errors"], tot["flag_margin"], tot["flag_margin"] / tot["n"] * 100,
        tot["flag_margin_hit"], tot["flag_score"], tot["flag_any"], tot["flag_any"] / tot["n"] * 100))
    print("  ⇒ 现行 margin 告警的精确率 = {}/{} = {:.1f}%".format(
        tot["flag_margin_hit"], tot["flag_margin"],
        tot["flag_margin_hit"] / tot["flag_margin"] * 100 if tot["flag_margin"] else float("nan")))

    p = os.path.join(BENCH_OUT, "alarm_report.json")
    with io.open(p, "w", encoding="utf-8", newline="") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
    print()
    print("已落盘 {}".format(p))
    return 0


if __name__ == "__main__":
    sys.exit(main())
