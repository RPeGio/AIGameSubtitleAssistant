#!/usr/bin/env python3
"""T2 阶段 B：统一评估台 + DP 参数网格搜索。

真值来自 scripts/fuse_truth.py（参考文本为中介，独立于对齐算法）。

设计要点：
  · 相似度矩阵 S 只依赖编码，**与 DP 参数无关** ⇒ 每案例只算一次，网格内只跑 DP；
  · DP 用 O(n·m) 的前缀最大值形式（等价于 O(n·m²) 朴素式，脚本内自带等价性自检）；
  · 准确率同时给出**相对天花板**：严格递增 DP 无法表达真值里的一对多
    （同一语料行被多段复用）⇒ 天花板 = n − Σ(组内段数−1)；
  · 两种输入口径：raw（原样）与 dedup（去掉"互为前缀的相邻段"，即打字机首帧）。

用法：
    python scripts/fuse_calib.py                # 全案例网格搜索
    python scripts/fuse_calib.py --selfcheck    # 只做 DP 等价性自检
    python scripts/fuse_calib.py --cases pierro # 限定案例
"""
import argparse
import difflib
import io
import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "tests"))
import numpy as np  # noqa: E402
from fuse_lab import BENCH_OUT, load_embedder, similarity_matrix, split_header  # noqa: E402

NEG = -1e9

GRID_SKIP = [0.0, 0.002, 0.005, 0.01, 0.02, 0.04, 0.08, 0.16]
GRID_UNMATCHED = [0.02, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5]
DEFAULT = (0.02, 0.25)   # 现行未校准初值


# ────────────────────────── DP：朴素式（参考实现）──────────────────────────

def align_naive(S, skip_penalty, unmatched_penalty):
    n, m = S.shape
    dp = np.full((n + 1, m + 1), NEG)
    bk = np.zeros((n + 1, m + 1, 2), dtype=np.int32)
    dp[0, 0] = 0.0
    for i in range(1, n + 1):
        for j in range(m + 1):
            if dp[i - 1, j] <= NEG / 2:
                continue
            v = dp[i - 1, j] - unmatched_penalty
            if v > dp[i, j]:
                dp[i, j] = v
                bk[i, j] = (j, 0)
        for j in range(m + 1):
            if dp[i - 1, j] <= NEG / 2:
                continue
            base = dp[i - 1, j]
            for k in range(j + 1, m + 1):
                v = base + S[i - 1, k - 1] - skip_penalty * (k - 1 - j)
                if v > dp[i, k]:
                    dp[i, k] = v
                    bk[i, k] = (j, 1)
    return _backtrack(dp, bk, n, m)


# ────────────────────────── DP：O(n·m) 前缀最大值形式 ──────────────────────────
# 转移 v = dp[i-1][j] + S[i-1][k-1] - skip*(k-1-j)
#          = [S[i-1][k-1] - skip*(k-1)] + [dp[i-1][j] + skip*j]
# 后一项对 j 取前缀最大即可 ⇒ 内层 k 循环摊还 O(1)

def align_fast(S, skip_penalty, unmatched_penalty):
    n, m = S.shape
    dp = np.full((n + 1, m + 1), NEG)
    bk = np.zeros((n + 1, m + 1, 2), dtype=np.int32)
    dp[0, 0] = 0.0
    for i in range(1, n + 1):
        prev = dp[i - 1]
        # 无对应分支
        v_un = prev - unmatched_penalty
        better = v_un > dp[i]
        dp[i][better] = v_un[better]
        for j in np.nonzero(better)[0]:
            bk[i, j] = (j, 0)
        # 匹配分支：前缀最大
        P = np.where(prev > NEG / 2, prev + skip_penalty * np.arange(m + 1), NEG)
        argj = -np.ones(m + 1, dtype=np.int32)
        run_max = NEG
        run_arg = -1
        for k in range(1, m + 1):
            cand = P[k - 1]
            if cand > run_max:
                run_max = cand
                run_arg = k - 1
            if run_max <= NEG / 2:
                continue
            v = S[i - 1, k - 1] - skip_penalty * (k - 1) + run_max
            if v > dp[i, k]:
                dp[i, k] = v
                bk[i, k] = (run_arg, 1)
    return _backtrack(dp, bk, n, m)


def _backtrack(dp, bk, n, m):
    j = int(np.argmax(dp[n]))
    match = [-1] * n
    for i in range(n, 0, -1):
        pj, kind = bk[i, j]
        match[i - 1] = (j - 1) if kind == 1 else -1
        j = int(pj)
    return match


# ────────────────────────── 语料近重复等价类（评分必须容忍）──────────────────────────
# 语料取自 OCR，同一条台词可能被收进两次（一次带 OCR 噪音、一次干净）。
# glupov 实测 12 对（1↔22 … 21↔34，相似度 0.974~1.000）、pierro 1 对（[98]↔[99] 完全相同）。
# 此时"命中哪一个下标"在语义上等价，按**下标精确相等**评分会把正确结果判成错——
# 实测 glupov 因此从 22/22 掉到 13/22（且使真值本身非单调，与单调对齐不相容）。
# 故评分改用等价类：预测落在真值所属类内即算对。

DUP_THR = 0.95


def equiv_classes(corpus):
    """→ 每个语料下标（0-based）所属的等价类 id"""
    import re
    bodies = [re.sub(r"[\s「」\[\]【】（）()〈〉《》『』、。，！？…~·．,\.!\?\"'’‘“”—\-]", "",
                     split_header(t)[1]) for t in corpus]
    n = len(corpus)
    parent = list(range(n))

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for i in range(n):
        for j in range(i + 1, n):
            if not bodies[i] or not bodies[j]:
                continue
            if difflib.SequenceMatcher(None, bodies[i], bodies[j]).ratio() >= DUP_THR:
                parent[find(j)] = find(i)
    return [find(i) for i in range(n)]


def score(match, truth, idxs, cls):
    """等价类感知评分：预测与真值同属一类即算对"""
    ok = 0
    for n, k in enumerate(idxs):
        pred = match[n] + 1
        t = truth[k]
        if pred <= 0 or t <= 0:
            continue
        if cls[pred - 1] == cls[t - 1]:
            ok += 1
    return ok


def ceiling_of(truth, idxs, cls):
    """严格递增 DP 的**等价类天花板**：可被同时满足的最大段数。

    每段可接受集合 = 真值所属等价类的全部语料下标；在"严格递增选择"下用贪心
    取每段可用的最小下标（留最大余量），贪心对本问题是最大基数最优。
    """
    members = {}
    for j, c in enumerate(cls):
        members.setdefault(c, []).append(j + 1)
    last = 0
    cnt = 0
    for k in idxs:
        t = truth[k]
        if t <= 0:
            continue
        cand = [x for x in members[cls[t - 1]] if x > last]
        if cand:
            last = min(cand)
            cnt += 1
    return cnt, len(idxs) - cnt


# ────────────────────────── 案例与输入口径 ──────────────────────────

def load_truth(key):
    p = os.path.join(BENCH_OUT, "truth_{}.json".format(key))
    if not os.path.exists(p):
        raise SystemExit("缺少真值 {}（先跑 scripts/fuse_truth.py）".format(p))
    with io.open(p, encoding="utf-8") as f:
        return json.load(f)


def dedup_prefix(segs):
    """去掉"与相邻段互为前缀"的较短段（打字机首帧）；返回保留下标"""
    def nz(s):
        return "".join(s.split())
    keep, i = [], 0
    while i < len(segs):
        j = i
        while j + 1 < len(segs):
            a, b = nz(segs[i]["text"]), nz(segs[j + 1]["text"])
            if a.startswith(b) or b.startswith(a):
                j += 1
            else:
                break
        keep.append(max(range(i, j + 1), key=lambda k: len(nz(segs[k]["text"]))))
        i = j + 1
    return keep


def ceiling_of_old(truth, idxs):
    """（保留供对照）下标精确口径的天花板：真值里同一语料行被多段复用时每组至少错 1 段"""
    groups = {}
    for k in idxs:
        t = truth[k]
        if t > 0:
            groups.setdefault(t, []).append(k)
    lost = sum(len(v) - 1 for v in groups.values() if len(v) > 1)
    return len(idxs) - lost, lost


def score_exact(match, truth, idxs):
    return sum(1 for n, k in enumerate(idxs) if match[n] + 1 == truth[k])


# ────────────────────────── 主流程 ──────────────────────────

def build_matrices(keys, tok, sess):
    """→ {key: {"raw": (S, idxs), "dedup": (S, idxs), truth, corpus}}"""
    out = {}
    for key in keys:
        T = load_truth(key)
        corpus = T["corpus"]
        rows = T["rows"]
        truth = [r["truth"] for r in rows]
        cb = [split_header(t)[1] for t in corpus]
        cls = equiv_classes(corpus)
        ndup = len(corpus) - len(set(cls))
        data = {"truth": truth, "corpus": corpus, "n_raw": len(rows),
                "cls": cls, "ndup": ndup, "protocols": {}}
        for proto in ("raw", "dedup"):
            idxs = list(range(len(rows))) if proto == "raw" else dedup_prefix(rows)
            sb = [split_header(rows[k]["text"])[1] for k in idxs]
            S = similarity_matrix(tok, sess, cb, sb)
            data["protocols"][proto] = (S, idxs)
        out[key] = data
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", nargs="*", default=None)
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--protocol", default="both", choices=["raw", "dedup", "both"])
    args = ap.parse_args()

    keys = args.cases or ["moon", "glupov", "pierro"]
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = build_matrices(keys, tok, sess)

    # ── DP 等价性自检（校准不能悄悄换算法）──
    print("=" * 100)
    print("DP 等价性自检（O(n·m²) 朴素式 vs O(n·m) 前缀最大值式）")
    ok = True
    for key in keys:
        for proto, (S, idxs) in data[key]["protocols"].items():
            for (sp, up) in [(0.02, 0.25), (0.005, 0.1), (0.08, 0.5), (0.0, 0.02)]:
                a = align_naive(S, sp, up)
                b = align_fast(S, sp, up)
                same = a == b
                ok = ok and same
                if not same:
                    d = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
                    print("  ✗ {} {} sp={} up={} 差异 {} 处 {}".format(key, proto, sp, up, len(d), d[:4]))
    print("  等价: {}".format("✓ 全部一致" if ok else "✗ 存在不一致"))
    if args.selfcheck or not ok:
        return 0 if ok else 1

    # ── 基线（现行初值）──
    print()
    print("── 语料近重复等价类（评分容忍口径）──")
    for key in keys:
        d = data[key]
        print("  {:<8} 语料 {:3} 条 → 等价类 {:3} 个（合并 {} 条重复）".format(
            key, len(d["corpus"]), len(set(d["cls"])), d["ndup"]))
    print()
    print("── 现行初值基线（skip={}, unmatched={}）──".format(*DEFAULT))
    protos = ["raw", "dedup"] if args.protocol == "both" else [args.protocol]
    for key in keys:
        d = data[key]
        line = ["  {:<8}".format(key)]
        for proto in protos:
            S, idxs = d["protocols"][proto]
            m = align_fast(S, *DEFAULT)
            c = score(m, d["truth"], idxs, d["cls"])
            cx = score_exact(m, d["truth"], idxs)
            ceil, lost = ceiling_of(d["truth"], idxs, d["cls"])
            line.append("{} 类口径{:3}/{:3} ({:5.1f}%) 下标口径{:3} 天花板{:3} ({:5.1f}%)".format(
                proto, c, len(idxs), c / len(idxs) * 100, cx,
                ceil, ceil / len(idxs) * 100))
        print("  ".join(line))

    # ── 网格搜索 ──
    print()
    print("── 网格搜索：skip_penalty × unmatched_penalty（等价类口径）──")
    results = {}
    for proto in protos:
        print()
        print("  口径 = {}".format(proto))
        best = None
        table = {}
        for sp in GRID_SKIP:
            for up in GRID_UNMATCHED:
                tot_c = tot_n = 0
                per = {}
                for key in keys:
                    d = data[key]
                    S, idxs = d["protocols"][proto]
                    m = align_fast(S, sp, up)
                    c = score(m, d["truth"], idxs, d["cls"])
                    per[key] = (c, len(idxs))
                    tot_c += c
                    tot_n += len(idxs)
                table[(sp, up)] = (tot_c, tot_n, per)
                if best is None or tot_c / tot_n > best[0]:
                    best = (tot_c / tot_n, sp, up, per, tot_c, tot_n)
        results[proto] = table
        acc, sp, up, per, tc, tn = best
        print("    最优：skip={:<6} unmatched={:<5} 合计 {}/{} = {:.1f}%".format(
            sp, up, tc, tn, acc * 100))
        for key in keys:
            c, n = per[key]
            print("        {:<8} {:3}/{:3} ({:5.1f}%)".format(key, c, n, c / n * 100))
        # 平台宽度：与最优同分的参数组合数
        plateau = [(k, v) for k, v in table.items() if v[0] == tc]
        print("    同分（合计 {} 段正确）的参数组合：{} 组 / 共 {} 组".format(
            tc, len(plateau), len(table)))
        sps = sorted({k[0] for k, _ in plateau})
        ups = sorted({k[1] for k, _ in plateau})
        print("      skip 取值范围 {} ; unmatched 取值范围 {}".format(sps, ups))
        # 现行初值排名
        cur = table[DEFAULT]
        rank = sum(1 for v in table.values() if v[0] > cur[0]) + 1
        print("    现行初值 {}：合计 {}/{} = {:.1f}%（并列第 {} 名）".format(
            DEFAULT, cur[0], cur[1], cur[0] / cur[1] * 100, rank))

    # ── 落盘 ──
    out = os.path.join(BENCH_OUT, "calib_grid.json")
    ser = {proto: {"{:.4f}|{:.4f}".format(k[0], k[1]): {"correct": v[0], "total": v[1],
                                                        "per": v[2]}
                   for k, v in table.items()}
           for proto, table in results.items()}
    with io.open(out, "w", encoding="utf-8", newline="") as f:
        json.dump({"grid_skip": GRID_SKIP, "grid_unmatched": GRID_UNMATCHED,
                   "default": list(DEFAULT), "results": ser}, f, ensure_ascii=False, indent=1)
    print()
    print("已落盘 {}".format(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
