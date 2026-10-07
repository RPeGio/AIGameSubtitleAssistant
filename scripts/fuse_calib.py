#!/usr/bin/env python3
"""T2 阶段 B：统一评估台 + DP 参数网格搜索。

真值来自 scripts/fuse_truth.py（参考文本为中介，独立于对齐算法）。

设计要点：
  · 相似度矩阵 S 只依赖编码，**与 DP 参数无关** ⇒ 每案例只算一次，网格内只跑 DP；
  · DP 用 O(n·m) 的前缀最大值形式（等价于 O(n·m²) 朴素式，脚本内自带等价性自检）；
  · 准确率同时给出**相对天花板**：严格递增 DP 无法表达真值里的一对多
    （同一语料行被多段复用）⇒ 天花板 = n − Σ(组内段数−1)。

**本脚本不做输入卫生**（"互为前缀的相邻段"的合并）。那类"同一条字幕被 OCR 拆成两段"
属 **OCR 合并层的 raw 缺陷**，应在管线层修；曾在此处按文本判据合并，**已被证否并删除**
——见 benchmark/OCR_PIPELINE_DEFECTS.md **D12**：正当用例与误伤用例在 (短态, 长态)
文本上完全同形，**任何只依赖该文本对的判据不可能两全**。故本脚本只有一种口径：
**消费管线产出**。管线修好后重新生成产物轨，即得到干净输入。

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
# 统一转移模型 B（有界一对多）的复用罚分。实测安全窗口 **[0.2, 0.3]**：
#   ≥0.4 组解不开（pierro 107/121）；≤0.18 glupov 出现**误复用**（22/22 → 21/22）；
#   [0.2, 0.3] 内三案例：moon 15/15、glupov 22/22、pierro **119/121**（复用 2 次）。
# 取窗口中值 0.25（与 unmatched_penalty 同量级，纯属巧合，两者语义无关）。
REPEAT_DEFAULT = 0.25
# 统一转移模型 C（回退 / 拖进度条）的罚分：**保持禁用**（选项 i）。
# 现有三案例真值里**没有任何真实顺序回退**（glupov 的 5 处是语料近重复行的下标假象），
# 故该路径无素材可验；待新增"PV reaction（主播反复拖进度条）"案例后再放开。
RESET_DEFAULT = float("inf")


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
        match[i - 1] = (j - 1) if kind != 0 else -1
        j = int(pj)
    return match


# ────────────────── DP：统一转移模型（前进 / 复用 / 回退 / 不配）──────────────────
# T4c：把"序"与"重数"两个假设分开，四种转移并列——
#
#   A 前进   k > j'   dp[i-1][j'] + S[i-1][k-1] - skip_penalty*(k-1-j')
#   B 复用   k == j'  dp[i-1][k]   + S[i-1][k-1] - repeat_penalty      ← 有界一对多
#   C 回退   k < j'   dp[i-1][j'] + S[i-1][k-1] - reset_penalty       ← 拖进度条重看
#   D 不配   —        dp[i-1][j]  - unmatched_penalty
#
# 为什么需要 B：一段字幕被 OCR 切成 N 段时，严格递增 DP 无法让多段复用同一语料行 ⇒
# 后段被挤到下一行 ⇒ **此后整条链顺移**。pierro 实测：2 组一对多造成 21 个百分点的损失
# （78.5% vs 天花板 97.5%），远大于 OCR 残留碎片本身的代价。
#
# 为什么需要 C：实况里主播会**反复拖进度条重看** PV/剧情，此时语料下标顺序会**回退**
# （`1-2-3-4-1-2`）甚至跳进（`1-3-4-2`）——严格递增 DP 原理上无法表达。
#
# **安全性质（本函数的存在意义）**：`repeat_penalty = reset_penalty = inf` 时，
# 本函数与 `align_fast`（现行严格递增 DP）**逐段等价**，由 `--selfcheck` 断言。
# 故新结构可先在"回退禁用"下上线（选项 i），待有素材再放开 C。
#
# 复杂度仍是 O(n·m)：A 用**前缀**最大（k 递增一趟）、C 用**后缀**最大（k 递减一趟）、
# B/D 各 O(1)。
#
# B 的"有界"由**线性累积罚分**实现（连续复用 r 次即付 r×repeat_penalty），无需额外状态；
# 若将来实测出现长链复用，再考虑加硬上限。

KIND_UNMATCHED, KIND_FORWARD, KIND_REPEAT, KIND_RESET = 0, 1, 2, 3


def align_v2(S, skip_penalty, unmatched_penalty,
             repeat_penalty=float("inf"), reset_penalty=float("inf"),
             diag=None):
    n, m = S.shape
    dp = np.full((n + 1, m + 1), NEG)
    bk = np.zeros((n + 1, m + 1, 2), dtype=np.int32)
    dp[0, 0] = 0.0
    rep_on = np.isfinite(repeat_penalty)
    res_on = np.isfinite(reset_penalty)
    for i in range(1, n + 1):
        prev = dp[i - 1]
        # ── D 不配 ──
        v_un = prev - unmatched_penalty
        better = v_un > dp[i]
        dp[i][better] = v_un[better]
        for j in np.nonzero(better)[0]:
            bk[i, j] = (j, KIND_UNMATCHED)
        # ── C 回退：max_{j' > k} dp_prev[j']，k 递减一趟 ──
        if res_on:
            suf_max, suf_arg = NEG, -1
            for k in range(m, 0, -1):
                if suf_max > NEG / 2:
                    v = S[i - 1, k - 1] - reset_penalty + suf_max
                    if v > dp[i, k]:
                        dp[i, k] = v
                        bk[i, k] = (suf_arg, KIND_RESET)
                if prev[k] > suf_max:      # 纳入 j' = k，供下一轮（k-1）使用
                    suf_max, suf_arg = prev[k], k
        # ── A 前进（前缀最大）+ B 复用，k 递增一趟 ──
        P = np.where(prev > NEG / 2, prev + skip_penalty * np.arange(m + 1), NEG)
        run_max, run_arg = NEG, -1
        for k in range(1, m + 1):
            cand = P[k - 1]
            if cand > run_max:
                run_max, run_arg = cand, k - 1
            if run_max > NEG / 2:
                v = S[i - 1, k - 1] - skip_penalty * (k - 1) + run_max
                if v > dp[i, k]:
                    dp[i, k] = v
                    bk[i, k] = (run_arg, KIND_FORWARD)
            if rep_on and prev[k] > NEG / 2:
                vb = prev[k] + S[i - 1, k - 1] - repeat_penalty
                if vb > dp[i, k]:
                    dp[i, k] = vb
                    bk[i, k] = (k, KIND_REPEAT)
    match = _backtrack(dp, bk, n, m)
    if diag is not None:
        j = int(np.argmax(dp[n]))
        kinds = {}
        for i in range(n, 0, -1):
            pj, kind = bk[i, j]
            kinds[kind] = kinds.get(kind, 0) + 1
            j = int(pj)
        diag.update({"repeat": kinds.get(KIND_REPEAT, 0),
                     "reset": kinds.get(KIND_RESET, 0),
                     "forward": kinds.get(KIND_FORWARD, 0),
                     "unmatched": kinds.get(KIND_UNMATCHED, 0)})
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
    """→ {key: {"S": 相似度矩阵, "idxs": 段下标, truth, corpus}}"""
    out = {}
    for key in keys:
        T = load_truth(key)
        corpus = T["corpus"]
        rows = T["rows"]
        truth = [r["truth"] for r in rows]
        cb = [split_header(t)[1] for t in corpus]
        cls = equiv_classes(corpus)
        ndup = len(corpus) - len(set(cls))
        idxs = list(range(len(rows)))
        sb = [split_header(rows[k]["text"])[1] for k in idxs]
        S = similarity_matrix(tok, sess, cb, sb)
        out[key] = {"truth": truth, "corpus": corpus, "n_raw": len(rows),
                    "cls": cls, "ndup": ndup, "S": S, "idxs": idxs}
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", nargs="*", default=None)
    ap.add_argument("--selfcheck", action="store_true")
    args = ap.parse_args()

    keys = args.cases or ["moon", "glupov", "pierro"]
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = build_matrices(keys, tok, sess)

    # ── DP 等价性自检（校准不能悄悄换算法）──
    print("=" * 100)
    print("DP 等价性自检（O(n·m²) 朴素式 vs O(n·m) 前缀最大值式）")
    ok = True
    for key in keys:
        S, idxs = data[key]["S"], data[key]["idxs"]
        for (sp, up) in [(0.02, 0.25), (0.005, 0.1), (0.08, 0.5), (0.0, 0.02)]:
            a = align_naive(S, sp, up)
            b = align_fast(S, sp, up)
            same = a == b
            ok = ok and same
            if not same:
                d = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
                print("  ✗ {} sp={} up={} 差异 {} 处 {}".format(key, sp, up, len(d), d[:4]))
    print("  等价: {}".format("✓ 全部一致" if ok else "✗ 存在不一致"))

    # ── 统一转移模型：B/C 关闭时必须与现行严格递增 DP 逐段等价 ──
    # 这是"新结构零风险上线"的证明：结构换了、行为没换。
    print()
    print("统一转移模型自检（repeat=inf 且 reset=inf ⇒ 应与 align_fast 逐段相同）")
    ok2 = True
    for key in keys:
        S, idxs = data[key]["S"], data[key]["idxs"]
        for (sp, up) in [(0.02, 0.25), (0.005, 0.1), (0.08, 0.5), (0.0, 0.02)]:
            a = align_fast(S, sp, up)
            b = align_v2(S, sp, up)
            same = a == b
            ok2 = ok2 and same
            if not same:
                d = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
                print("  ✗ {} sp={} up={} 差异 {} 处 {}".format(key, sp, up, len(d), d[:4]))
    print("  等价: {}".format("✓ 全部一致" if ok2 else "✗ 存在不一致"))
    ok = ok and ok2
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
    for key in keys:
        d = data[key]
        S, idxs = d["S"], d["idxs"]
        m = align_fast(S, *DEFAULT)
        c = score(m, d["truth"], idxs, d["cls"])
        cx = score_exact(m, d["truth"], idxs)
        ceil, lost = ceiling_of(d["truth"], idxs, d["cls"])
        print("  {:<8} 类口径{:3}/{:3} ({:5.1f}%)  下标口径{:3}  天花板{:3} ({:5.1f}%)".format(
            key, c, len(idxs), c / len(idxs) * 100, cx, ceil, ceil / len(idxs) * 100))

    # ── 统一转移模型：B（有界一对多）实测 ──
    print()
    print("── 统一转移模型 B：有界一对多（repeat_penalty 扫描；reset 保持禁用 = 选项 i）──")
    print("   {:>8}  {:<8} {:>9} {:>7} {:>7} {:>6}".format(
        "repeat", "案例", "类口径", "天花板", "复用次数", "前进"))
    for rp in [float("inf"), 0.5, 0.4, 0.35, 0.3, 0.28, 0.25, 0.22, 0.2, 0.18, 0.15]:
        for key in keys:
            d = data[key]
            S, idxs = d["S"], d["idxs"]
            diag = {}
            m = align_v2(S, *DEFAULT, repeat_penalty=rp, diag=diag)
            c = score(m, d["truth"], idxs, d["cls"])
            ceil, lost = ceiling_of(d["truth"], idxs, d["cls"])
            print("   {:>8}  {:<8} {:>4}/{:<4} {:>7} {:>8} {:>6}".format(
                "inf" if not np.isfinite(rp) else rp, key, c, len(idxs), ceil,
                diag["repeat"], diag["forward"]))
        # 三案例合计
        tot_c = tot_n = tot_r = 0
        for key in keys:
            d = data[key]
            S, idxs = d["S"], d["idxs"]
            diag = {}
            m = align_v2(S, *DEFAULT, repeat_penalty=rp, diag=diag)
            tot_c += score(m, d["truth"], idxs, d["cls"])
            tot_n += len(idxs)
            tot_r += diag["repeat"]
        print("   {:>8}  {:<8} {:>4}/{:<4} {:>7} {:>8}".format(
            "inf" if not np.isfinite(rp) else rp, "合计", tot_c, tot_n, "-", tot_r))

    # ── 网格搜索 ──
    print()
    print("── 网格搜索：skip × unmatched（等价类口径；模型 = 统一转移，repeat={}, reset=禁用）──".format(REPEAT_DEFAULT))
    best = None
    table = {}
    for sp in GRID_SKIP:
        for up in GRID_UNMATCHED:
            tot_c = tot_n = 0
            per = {}
            for key in keys:
                d = data[key]
                S, idxs = d["S"], d["idxs"]
                m = align_v2(S, sp, up, repeat_penalty=REPEAT_DEFAULT, reset_penalty=RESET_DEFAULT)
                c = score(m, d["truth"], idxs, d["cls"])
                per[key] = (c, len(idxs))
                tot_c += c
                tot_n += len(idxs)
            table[(sp, up)] = (tot_c, tot_n, per)
            if best is None or tot_c / tot_n > best[0]:
                best = (tot_c / tot_n, sp, up, per, tot_c, tot_n)
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
    ser = {"{:.4f}|{:.4f}".format(k[0], k[1]): {"correct": v[0], "total": v[1],
                                                "per": v[2]}
           for k, v in table.items()}
    with io.open(out, "w", encoding="utf-8", newline="") as f:
        json.dump({"grid_skip": GRID_SKIP, "grid_unmatched": GRID_UNMATCHED,
                   "default": list(DEFAULT), "results": ser}, f, ensure_ascii=False, indent=1)
    print()
    print("已落盘 {}".format(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
