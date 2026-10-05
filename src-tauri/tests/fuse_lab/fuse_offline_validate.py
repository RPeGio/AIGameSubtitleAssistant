# -*- coding: utf-8 -*-
"""离线验证流水线（产品形态原型）：向量召回 + 单调 DP + 人工确认清单。

输入：预校对工程（examples/benchmark_examples/*.gsa）
  语料 = 可靠文本全集；转写侧 = game 轨 ASR 段 + 无配音处 embed_ocr 段（主播轨排除）。
  时间轴一律从事件取（融合不改时间轴）。

流程：
  ① corpus / GC → split_header（**只用于编码**）
  ② E5 编码 + top-k 召回（跨语言语义对齐，无需 LLM）
  ③ monotonic_align 修正（语料与转写同序 ⇒ 严格递增）
  ④ 一对多：同一语料行被多段命中 = 同句的多次显示（语义正确）
  ⑤ 低置信 / 歧义段 → 待人工确认清单
  ⑥ 产物：命中段 = **语料原文（逐字，含名字行）**；未命中段 = 转写原文

用法：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    python src-tauri/tests/fuse_lab/fuse_offline_validate.py [project.gsa ...]
默认跑 moon_sisters.gsa 与 glupov.gsa。
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import numpy as np  # noqa: E402
from fuse_lab import (BENCH_OUT, collect_game_content, load_embedder, load_project,  # noqa: E402
                      monotonic_align, similarity_matrix, split_header, write_json, write_srt)

TOP_K = 5
DEFAULT_PROJECTS = ["moon_sisters.gsa", "glupov.gsa"]


def run(filename, tok, sess):
    proj = load_project(filename)
    corpus = [c["text"] for c in proj["corpus"]]
    gc = collect_game_content(proj)
    corpus_body = [split_header(t)[1] for t in corpus]   # 编码用正文
    gc_body = [split_header(g["text"])[1] for g in gc]
    n_head_c = sum(1 for t in corpus if split_header(t)[0])
    n_head_g = sum(1 for g in gc if split_header(g["text"])[0])

    tag = "{}（语料 {} 条 / 转写 {} 段：ASR {} + 嵌字 {}）".format(
        filename.replace(".gsa", ""), len(corpus), len(gc),
        sum(1 for g in gc if g["kind"] == "asr"), sum(1 for g in gc if g["kind"] == "embed"))
    print("=" * 92)
    print(tag)
    print("  表头剥离（仅编码）：语料 {}/{} 条、转写 {}/{} 段含表头".format(
        n_head_c, len(corpus), n_head_g, len(gc)))

    S = similarity_matrix(tok, sess, corpus_body, gc_body)
    match, _ = monotonic_align(S)

    used = {}
    for i, j in enumerate(match):
        if j >= 0:
            used.setdefault(j, []).append(i)

    rows = []
    for i, g in enumerate(gc):
        j = match[i]
        order = sorted(range(len(corpus)), key=lambda x: -S[i, x])[:TOP_K]
        top1 = float(S[i, order[0]])
        margin = top1 - float(S[i, order[1]]) if len(order) > 1 else top1
        rows.append({
            "index": i + 1, "kind": g["kind"], "start": g["start"], "end": g["end"],
            # 模型输入（已剥表头）
            "gc_body": gc_body[i], "ocr_body": corpus_body[j] if j >= 0 else "",
            # 产物文本：命中 = 语料原文逐字（含名字行）；未命中 = 转写原文
            "gc": g["text"],
            "ocr_index": j + 1 if j >= 0 else 0,
            "product_text": corpus[j] if j >= 0 else g["text"],
            "score": float(S[i, j]) if j >= 0 else 0.0,
            "top1": top1, "margin": margin,
            "cands": [(c + 1, round(float(S[i, c]), 3)) for c in order],
            "shared_with": [x + 1 for x in used.get(j, [])] if j >= 0 else [],
        })

    matched = sum(1 for r in rows if r["ocr_index"])
    seq = [r["ocr_index"] for r in rows if r["ocr_index"]]
    monotonic = all(seq[k] < seq[k + 1] for k in range(len(seq) - 1))
    verbatim = all(r["product_text"] == corpus[r["ocr_index"] - 1]
                   for r in rows if r["ocr_index"])
    print("  命中 {}/{} 段；覆盖语料 {}/{} 行；严格递增 {}；产物与语料逐字一致 {}".format(
        matched, len(rows), len(used), len(corpus), monotonic, verbatim))

    todo = [r for r in rows if not r["ocr_index"] or r["margin"] < 0.02]
    print("  待人工确认 {} 段（未命中 {} + 歧义 margin<0.02 {}）".format(
        len(todo), sum(1 for r in rows if not r["ocr_index"]),
        sum(1 for r in rows if r["ocr_index"] and r["margin"] < 0.02)))
    return {"tag": tag, "source": filename, "corpus": corpus, "rows": rows, "todo": todo}


def main():
    filenames = sys.argv[1:] or DEFAULT_PROJECTS
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    results = []
    for fn in filenames:
        try:
            results.append(run(fn, tok, sess))
        except Exception as e:
            print("{} 失败：{}: {}".format(fn, type(e).__name__, e))
    os.makedirs(BENCH_OUT, exist_ok=True)
    for r in results:
        base = r["source"].replace(".gsa", "")
        write_srt(r["rows"], os.path.join(BENCH_OUT, "fuse_vec_{}.srt".format(base)))
        write_json(os.path.join(BENCH_OUT, "fuse_vec_{}.json".format(base)), r)
        print("  已落盘 {}/fuse_vec_{}.{{srt,json}}".format(BENCH_OUT, base))

    for r in results:
        print("\n" + "=" * 92)
        print("逐段判定表 — {}".format(r["tag"]))
        print("{:>3} {:>5} {:>4} {:>7} {:>8}  {}".format("GC", "类型", "命中", "分数", "与次优差", "产物文本"))
        for row in r["rows"]:
            flag = "" if row["ocr_index"] and row["margin"] >= 0.02 else "  ← 待确认"
            text = row["product_text"].replace("\n", "⏎")
            print("{:>3} {:>5} {:>4} {:>7.3f} {:>8.3f}  {}{}".format(
                row["index"], "ASR" if row["kind"] == "asr" else "嵌字",
                row["ocr_index"] or "-", row["score"], row["margin"], text[:56], flag))


if __name__ == "__main__":
    main()
