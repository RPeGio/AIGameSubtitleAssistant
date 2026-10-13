# -*- coding: utf-8 -*-
"""判别器 B：多语言向量召回的召回率（判定"召回 + 复核"路线是否可行）。

对每组用例：语料编码为向量库、每条 GC 编码为查询、取 top-k，统计 recall@k 与 MRR。
判别器（错序用例）的恒等基线为 0 ⇒ 高 recall 只能来自内容匹配。

用法：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    python src-tauri/tests/fuse_lab/bench_fusion_recall.py
环境变量：GSA_EMBED_MODEL、GSA_EMBED_ONNX、GSA_BENCH_OUT_DIR
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import numpy as np  # noqa: E402
from fuse_lab import (BENCH_OUT, build_cases, identity_baseline,  # noqa: E402
                      is_deranged_case, load_embedder, similarity_matrix, write_json)


def evaluate(name, corpus, gc_texts, truth, tok, sess):
    S = similarity_matrix(tok, sess, corpus, gc_texts)
    order = np.argsort(-S, axis=1)
    hits = {1: 0, 3: 0, 5: 0}
    rr = 0.0
    ident = 0
    tot = 0
    for i, want in enumerate(truth):
        if want == 0:
            continue
        tot += 1
        rank = int(np.where(order[i] == want - 1)[0][0]) + 1
        for k in hits:
            if rank <= k:
                hits[k] += 1
        rr += 1.0 / rank
        ident += (order[i][0] == i)
    rnd = {k: min(k, len(corpus)) / len(corpus) * 100 for k in hits}
    print("[{:28}] n={:2} recall@1 {:2}/{:2} ({:4.0f}%)  @3 {:2}/{:2} ({:4.0f}%)  "
          "@5 {:2}/{:2} ({:4.0f}%)  MRR {:.3f}".format(
              name, tot, hits[1], tot, hits[1] / max(tot, 1) * 100,
              hits[3], tot, hits[3] / max(tot, 1) * 100,
              hits[5], tot, hits[5] / max(tot, 1) * 100, rr / max(tot, 1)))
    print("      随机基线 @1 {:.0f}%  @3 {:.0f}%  @5 {:.0f}%".format(rnd[1], rnd[3], rnd[5]))
    return {"case": name, "n": tot, "recall1": hits[1], "recall3": hits[3], "recall5": hits[5],
            "mrr": rr / max(tot, 1), "identity_top1": ident,
            "random1_pct": rnd[1], "random3_pct": rnd[3], "random5_pct": rnd[5]}


def main():
    out_dir = os.environ.get("GSA_BENCH_OUT_DIR", BENCH_OUT)
    model = os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small")
    print("=== 向量召回判别器（模型 {}，CPU onnxruntime）===".format(model))
    tok, sess = load_embedder(model)
    rows = []
    for name, corpus, gc_texts, truth in build_cases():
        if is_deranged_case(name):
            assert identity_baseline(truth) == 0, "{}：判别器失效".format(name)
        rows.append(evaluate(name, corpus, gc_texts, truth, tok, sess))
    der = [r for r in rows if is_deranged_case(r["case"])]
    n = sum(r["n"] for r in der)
    print("\n=== 汇总（错序用例；恒等映射正确率恒为 0）===")
    for k in (1, 3, 5):
        h = sum(r["recall{}".format(k)] for r in der)
        print("  recall@{} = {}/{} ({:.0f}%)".format(k, h, n, h / max(n, 1) * 100))
    print("  MRR = {:.3f}".format(sum(r["mrr"] * r["n"] for r in der) / max(n, 1)))
    write_json(os.path.join(out_dir, "fuse_lab_recall_{}.json".format(model)), rows)
    print("结果落盘：{}/fuse_lab_recall_{}.json".format(out_dir, model))


if __name__ == "__main__":
    main()
