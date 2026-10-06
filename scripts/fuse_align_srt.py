#!/usr/bin/env python3
"""产品形态的融合对齐 → SRT（供人工检验效果）。

这是 [benchmark/FUSE_ALIGNMENT_SCALE_VALIDATION.md](../benchmark/FUSE_ALIGNMENT_SCALE_VALIDATION.md)
与 [benchmark/FUSE_THRESHOLD_CALIBRATION.md](../benchmark/FUSE_THRESHOLD_CALIBRATION.md) 所验证路线的
**可执行产品形态**：多语言向量召回 + 单调 DP 做跨语言对齐，把语料文本替换到转写侧的时间轴上。

流程（对应 FUSE_VECTOR_RECALL_VALIDATION.md §三 的产品形态）：

    ① 语料 corpus → **剥表头（仅编码用）** → E5 编码
    ② 转写侧段 → **输入卫生**（合并"互为前缀的相邻段"，即打字机首帧）→ 剥表头 → 编码 → 召回
    ③ 单调 DP 修正（语料与转写同序 ⇒ 匹配下标严格递增；skip=0.02 / unmatched=0.25）
    ④ 一对多：同一语料行被多段命中 = 同句的多次显示，语义正确，不合并时间轴
    ⑤ 低置信（score<0.78）或未命中 → 待人工确认清单
    ⑥ 产物：命中段 = **语料原文逐字（含名字行）**；未命中段 = 转写原文；时间轴沿用转写侧

两条不可动摇的约定（见 src-tauri/tests/fuse_lab/README.md）：

  · **表头剥离只用于模型输入**：产物文本必须是语料原文逐字（含名字行）。
    表头在段间完全同形会淹没正文语义——glupov 带表头时召回 3/22，剥离后 22/22。
  · **单调性必须真正强制**：曾因"无对应"分支沿用 argmax 前驱导致下标回退/回绕。

阈值说明（T2 实测结论）：`skip_penalty=0.02` / `unmatched_penalty=0.25` 已在最优平台上，**不需调整**；
告警只用 `score<0.78`（`margin<0.02` 已实测无区分度，标记率 63.5% 而精确率 ≤12%，故**不采用**）。

用法：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    python scripts/fuse_align_srt.py                 # 三个案例
    python scripts/fuse_align_srt.py pierro          # 指定案例

产物（默认 temp/bench_output/）：
    fuse_srt_<case>.srt        —— 最终字幕（BOM + CRLF，与产品 export 一致）
    fuse_srt_<case>.todo.txt   —— 待人工确认清单
    fuse_srt_<case>.json       —— 逐段判定表（含分数、候选、命中语料下标）
"""
import io
import json
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(_HERE, "..", "src-tauri", "tests"))
sys.path.insert(0, _HERE)

import numpy as np  # noqa: E402
from fuse_lab import (BENCH_OUT, collect_game_content, load_embedder,  # noqa: E402
                      load_project, similarity_matrix, split_header, write_srt)
from fuse_calib import DEFAULT, align_fast  # noqa: E402

# 案例 → 工程（转写侧由 collect_game_content 按产品口径收集：game 轨 ASR + 嵌字）
CASES = ["moon", "glupov", "pierro"]
PROJECTS = {
    "moon": "moon_sisters.gsa",
    "glupov": "glupov.gsa",
    "pierro": "pierro_questions.gsa",
}
# 告警阈值：只保留 score（T2 实测 margin 无区分度，故不使用）
THR_SCORE = 0.78
TOP_K = 5


def merge_prefix_dups(segs):
    """输入卫生：合并"互为前缀的相邻段"（打字机逐字显示的首帧）。

    同一句被 OCR 抓两次（先短后长），两者属**同一条字幕**。这里**合并而非丢弃**：
    时间取并集（保住起点，不丢时间轴覆盖），文本取最长者。

    不合并会怎样：严格递增 DP 无法让两段复用同一语料行 ⇒ 后一段被挤到下一行，
    此后整条链顺移。pierro 实测 1 处未合并 ⇒ 13 段连续错位（88.3% vs 99.2%）。
    """
    def nz(s):
        return "".join(s.split())

    out, i = [], 0
    while i < len(segs):
        j = i
        while j + 1 < len(segs):
            a, b = nz(segs[i]["text"]), nz(segs[j + 1]["text"])
            if a.startswith(b) or b.startswith(a):
                j += 1
            else:
                break
        grp = segs[i:j + 1]
        longest = max(grp, key=lambda g: len(nz(g["text"])))
        out.append({
            "start": min(g["start"] for g in grp),
            "end": max(g["end"] for g in grp),
            "text": longest["text"],
            "merged": len(grp),
        })
        i = j + 1
    return out


def run(case, tok, sess, out_dir):
    proj = load_project(PROJECTS[case])
    corpus = [c["text"] for c in proj["corpus"]]
    gc_raw = collect_game_content(proj)
    if not gc_raw:
        print("  {}：无转写侧段，跳过".format(case))
        return None

    gc = merge_prefix_dups(gc_raw)
    n_merged = sum(1 for g in gc if g["merged"] > 1)

    # ① / ② 编码：表头剥离**只作用于编码**（产物用原文）
    cb = [split_header(t)[1] for t in corpus]
    sb = [split_header(g["text"])[1] for g in gc]
    S = similarity_matrix(tok, sess, cb, sb)

    # ③ 单调 DP
    match = align_fast(S, *DEFAULT)

    # ④⑤⑥ 组装产物
    rows, todo = [], []
    matched = 0
    for i, g in enumerate(gc):
        j = match[i]
        order = sorted(range(len(corpus)), key=lambda x: -S[i, x])[:TOP_K]
        score = float(S[i, j]) if j >= 0 else 0.0
        hit = j >= 0
        if hit:
            matched += 1
        text = corpus[j] if hit else g["text"]
        rows.append({
            "start": g["start"], "end": g["end"],
            "product_text": text,
            "ocr_index": j + 1 if hit else 0,
            "score": round(score, 4),
            "gc": g["text"],
            "merged": g["merged"],
            "cands": [(c + 1, round(float(S[i, c]), 4)) for c in order],
        })
        if not hit or score < THR_SCORE:
            todo.append({
                "start": g["start"], "end": g["end"],
                "reason": "未命中" if not hit else "低置信(score={:.3f})".format(score),
                "gc": g["text"], "product_text": text,
                "cands": [(c + 1, round(float(S[i, c]), 4)) for c in order],
            })

    # 产物落盘
    os.makedirs(out_dir, exist_ok=True)
    srt = os.path.join(out_dir, "fuse_srt_{}.srt".format(case))
    write_srt(rows, srt, text_key="product_text")
    with io.open(os.path.join(out_dir, "fuse_srt_{}.json".format(case)), "w",
                 encoding="utf-8", newline="") as f:
        json.dump({"case": case, "project": PROJECTS[case], "corpus": corpus,
                   "segments": len(gc), "merged_from_dups": n_merged,
                   "matched": matched, "todo": len(todo),
                   "rows": rows, "todo_rows": todo}, f, ensure_ascii=False, indent=1)
    tp = os.path.join(out_dir, "fuse_srt_{}.todo.txt".format(case))
    with io.open(tp, "w", encoding="utf-8", newline="") as f:
        f.write("待人工确认清单 —— {}\n".format(case))
        f.write("（低置信 score<{:.2f} 或未命中；阈值见 benchmark/FUSE_THRESHOLD_CALIBRATION.md）\n\n".format(THR_SCORE))
        for t in todo:
            f.write("[{:8.2f} -> {:8.2f}] {}\n".format(t["start"], t["end"], t["reason"]))
            f.write("   转写: {}\n".format(t["gc"].replace("\n", " / ")))
            f.write("   产物: {}\n".format(t["product_text"].replace("\n", " / ")))
            f.write("   候选: {}\n\n".format(
                ", ".join("OCR[{}]={:.3f}".format(c, s) for c, s in t["cands"])))

    print("  {:<8} 语料 {:3} 条 | 转写 {:3} 段（合并 {:2} 处前缀重复）| 命中 {:3}/{:3} ({:5.1f}%) | 待确认 {:2}".format(
        case, len(corpus), len(gc), n_merged, matched, len(gc), matched / len(gc) * 100, len(todo)))
    print("           → {}".format(srt))
    return {"case": case, "corpus": len(corpus), "segments": len(gc),
            "merged": n_merged, "matched": matched, "todo": len(todo)}


def main():
    want = [a for a in sys.argv[1:] if not a.startswith("-")] or CASES
    out_dir = os.environ.get("GSA_BENCH_OUT_DIR", BENCH_OUT)
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    print("=" * 100)
    print("融合对齐 → SRT（向量召回 + 单调 DP；表头剥离仅用于编码，产物用语料原文）")
    res = []
    for case in want:
        r = run(case, tok, sess, out_dir)
        if r:
            res.append(r)
    if res:
        print()
        print("── 合计 ──")
        print("  段 {} | 命中 {} ({:.1f}%) | 待确认 {}".format(
            sum(r["segments"] for r in res), sum(r["matched"] for r in res),
            sum(r["matched"] for r in res) / sum(r["segments"] for r in res) * 100,
            sum(r["todo"] for r in res)))
    print()
    print("产物目录：{}".format(out_dir))
    return 0


if __name__ == "__main__":
    sys.exit(main())
