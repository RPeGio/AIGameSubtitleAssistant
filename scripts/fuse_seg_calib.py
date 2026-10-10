#!/usr/bin/env python3
"""T2 阶段 D：**显式重播分段**的参考无关边界检测器可行性验证（§7.5 方案的前置研究）。

背景（为什么不必再走"罚分"路线）：现行 `RESET_DEFAULT = 0.05` 用**局部罚分**表达
"序列整体后退"，而 DP 最大化的是「ΣS − 罚分」这个**代理目标**，与评分口径
（段落在可接受集合内）**不等价** ⇒ 大但有限的罚分反而危险（reset=1.0 → vesna 3/79，
比禁用回退的 39/79 还差）。更稳的做法是先把转写序列切成若干"播放遍"
（遍内语料下标严格递增、遍间允许回退），把全局结构**显式建模**。

**本脚本只做阶段 1 的测量，不改任何生产代码、不改任何已落地常量。**
核心未知：如何**不依赖参考文本**检测重播边界。四个候选信号全部只用
「产出段（文本 + 时间）」与相似度矩阵 S：

  T  相邻段时间间隙 gap(c) = start[c] − end[c−1]
  A  相邻段独立 argmax 语料下标回跳量 am[c−1] − am[c]
  B  后缀中 argmax 落在"前沿之前"的段占比（真边界处应接近 1）
  C  后缀对前沿**之前**语料行的平均相似度 / 对前沿**之后**语料行的平均相似度

硬门：moon/glupov/pierro 真值里没有真实顺序回退 ⇒ 任何检测器在它们上面都必须判定
"无边界"。

用法：
    python scripts/fuse_seg_calib.py            # 轨迹 + 信号表 + 阈值扫描
    python scripts/fuse_seg_calib.py --cases vesna
    python scripts/fuse_seg_calib.py --sweep    # 只打阈值扫描与平台宽度

运行环境（与其它 bench 脚本一致）：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    & runtime\\python\\python.exe scripts\\fuse_seg_calib.py
"""
import argparse
import io
import json
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "..", "src-tauri", "tests"))

import numpy as np  # noqa: E402
from fuse_lab import BENCH_OUT, load_embedder  # noqa: E402
import fuse_calib as FC  # noqa: E402

NEG = FC.NEG
KEYS = ["moon", "glupov", "pierro", "vesna"]
OLD = ["moon", "glupov", "pierro"]

# vesna 真实重播边界（由真值 truth 轨迹确定，见 --trace）：
#   c = 40  ← 主边界（第二遍播放开始，truth 30 → 1，时间 402.07s）
#   c = 39  ← 主边界前的 3 行小回退（truth 33 → 30）
#   c = 80、81 ← 尾部两处小回退（33 → 29 → 26）
# 设计目标只针对**主边界**（§7.5 的两遍窗口）；小回退属于"擦边"，
# 由 reset 罚分或分段内的局部容差处理。
VESNA_PRIMARY = 40
VESNA_SECONDARY = [39, 80, 81]


# ────────────────────────── 真值轨迹（阶段 1a，仅分析用）──────────────────────────

def truth_jumps(rows):
    """真值轨迹的"回跳"点：truth[i] < truth[i−1] 且两值都 > 0。"""
    out = []
    for i in range(1, len(rows)):
        a, b = rows[i - 1]["truth"], rows[i]["truth"]
        if a > 0 and b > 0 and b < a:
            out.append({"i": i, "from": a, "to": b, "drop": a - b,
                        "t": rows[i]["start"]})
    return out


def gaps_of(rows):
    g = [float("nan")]
    for i in range(1, len(rows)):
        g.append(rows[i]["start"] - rows[i - 1]["end"])
    return g


# ────────────────────────── 参考无关信号 ──────────────────────────

def argmax_of(S):
    """每段独立 argmax 语料下标（0-based；-1 = 整行被掩 ⇒ 无有效候选）。"""
    am = []
    for i in range(S.shape[0]):
        row = S[i]
        valid = row > NEG / 2
        am.append(int(np.argmax(np.where(valid, row, NEG))) if valid.any() else -1)
    return am


def smooth(am, k):
    """因果中位数平滑（K 窗口，只回溯、不看未来）：抗"独立 argmax 的孤立尖峰"。

    独立 argmax 是**极噪**序列（一条无语义内容的碎片段会把 argmax 甩到任意位置），
    任何建立在它上面的统计量（A/B/C）都会继承这份噪声。平滑是**对信号本身的
    预处理**，仍然只用产出段与 S 矩阵 ⇒ 不破坏"参考无关"这一前提。
    k <= 1 时不平滑。
    """
    if k <= 1:
        return list(am)
    out = []
    for i in range(len(am)):
        w = [x for x in am[max(0, i - k + 1):i + 1] if x >= 0]
        out.append(int(np.median(w)) if w else -1)
    return out


def dp_frontier(d, sp=FC.DEFAULT[0], up=FC.DEFAULT[1]):
    """→ F_dp[c]：**前缀自身单调对齐**（`align_fast`）走到的最远语料下标（0-based，-1 无）。

    这是"前沿"的**正解**：B/C 想表达的是"切点之后匹配的是**已经走过的**语料"
    （真重播处后缀会重新从小下标爬起），而"已经走过"这件事只有**对齐本身**才知道
    ——独立 argmax 的 `max(am[:c])` 会被孤立尖峰顶到 m−1（pierro 实测 F 恒为 145），
    于是 B 恒等于 1、彻底失去区分力（见 §7.7 的否决证据）。
    代价：每个切点跑一次前缀 DP（O(n²m) 总量），只在标定台里做。
    """
    S = d["S"]
    n = S.shape[0]
    out = [-1] * (n + 1)
    for c in range(1, n + 1):
        mm = FC.align_fast(S[:c], sp, up)
        used = [x for x in mm if x >= 0]
        out[c] = max(used) if used else -1
    return out


def signals(key, d, min_suffix=1, k_smooth=1, fdps=None):
    """→ 每个候选切点 c ∈ [1, n−1] 的信号值表（c 把序列切成 [0,c) 与 [c,n)）。

    F(c)（前沿）三种口径：
      `last` = am[c−1]（最后一段的 argmax，对应 DP 状态 j 的语义）；
      `max`  = max(am[0..c−1])（走过的最远语料位置，被孤立尖峰污染）；
      `dp`   = 前缀单调对齐走到的位置（正解，见 dp_frontier）。
    B(c)  = |{i ≥ c : am[i] ≥ 0 且 am[i] < F}| / |{i ≥ c : am[i] ≥ 0}|
    C(c)  = mean(S[i][j<F]) / mean(S[i][j≥F])   （i ≥ c，仅计有效格）
    """
    rows = d["rows"]
    S = d["S"]
    n, m = S.shape
    g = gaps_of(rows)
    am = smooth(argmax_of(S), k_smooth)
    out = []
    for c in range(1, n):
        suf = [i for i in range(c, n) if am[i] >= 0]
        cands = [("last", am[c - 1]), ("max", max(am[:c]) if c else -1)]
        if fdps is not None:
            cands.append(("dp", fdps[c]))
        for fname, F in cands:
            rec = {"c": c, "t": rows[c]["start"], "gap": g[c], "F_kind": fname,
                   "F": F, "am_prev": am[c - 1], "am_cur": am[c],
                   "A": (am[c - 1] - am[c]) if (am[c - 1] >= 0 and am[c] >= 0) else 0,
                   "n_suf": len(suf)}
            if F > 0 and len(suf) >= min_suffix:
                below = sum(1 for i in suf if am[i] < F)
                rec["B"] = below / len(suf)
                pre = S[np.ix_(suf, list(range(0, F)))]
                post = S[np.ix_(suf, list(range(F, m)))]
                mp = float(pre[pre > NEG / 2].mean()) if (pre > NEG / 2).any() else float("nan")
                mq = float(post[post > NEG / 2].mean()) if (post > NEG / 2).any() else float("nan")
                rec["C"] = (mp / mq) if mq and np.isfinite(mq) else float("nan")
                rec["C_pre"], rec["C_post"] = mp, mq
            else:
                rec["B"] = rec["C"] = float("nan")
            out.append(rec)
    return out


def sig_table(data, keys, min_suffix=1, k_smooth=1, use_dp=True):
    fdps = {k: dp_frontier(data[k]) for k in keys} if use_dp else {}
    return {k: signals(k, data[k], min_suffix, k_smooth, fdps.get(k)) for k in keys}


def get(tab, key, c, fname, field):
    for r in tab[key]:
        if r["c"] == c and r["F_kind"] == fname:
            return r.get(field, float("nan"))
    return float("nan")


# ────────────────────────── 报告 ──────────────────────────

def report_trace(data, keys):
    print("=" * 108)
    print("阶段 1a：真值轨迹与真实边界位置（参考文本推出，仅供分析；产品没有参考文本）")
    for k in keys:
        rows = data[k]["rows"]
        jm = truth_jumps(rows)
        g = gaps_of(rows)
        fin = [x for x in g[1:] if np.isfinite(x)]
        print("  {:<8} 段 {:>3}  truth 回跳点 {:<2} 处：{}".format(
            k, len(rows), len(jm),
            [(j["i"], "{}→{}".format(j["from"], j["to"]), "%.1fs" % j["t"]) for j in jm]))
        print("           gap 排序（前 6，c = 切点序号）：{}".format(
            ["c={} {:.1f}s".format(i, v) for i, v in
             sorted(enumerate(g), key=lambda x: -(x[1] if np.isfinite(x[1]) else -1))[:6]]))


def report_table(data, tab, key, fname, cuts=None):
    rows = data[key]["rows"]
    print("  ── {} 信号表（前沿口径 = {}）──".format(key, fname))
    print("     {:>4} {:>8} {:>7} {:>7} {:>7} {:>5} {:>6} {:>6} {:>4}".format(
        "c", "start", "gap", "F", "am[c-1]", "am[c]", "A", "B", "C"))
    for r in tab[key]:
        if r["F_kind"] != fname:
            continue
        if cuts is not None and r["c"] not in cuts:
            continue
        mark = ""
        if key == "vesna" and r["c"] == VESNA_PRIMARY:
            mark = "  ★主边界"
        elif key == "vesna" and r["c"] in VESNA_SECONDARY:
            mark = "  ·小回退"
        print("     {:>4} {:>8.2f} {:>7.2f} {:>7} {:>7} {:>7} {:>5} {:>6.2f} {:>6.2f}{}".format(
            r["c"], r["t"], r["gap"], r["F"], r["am_prev"], r["am_cur"], r["A"],
            r["B"], r["C"], mark))


# 信号名 → 表内字段名（T 的实现字段叫 gap）
SIGKEY = {"T": "gap", "A": "A", "B": "B", "C": "C"}


def _vec(tab, key, fk, ms, sigs):
    """→ [(c, {sig: value})]，只保留前沿口径匹配且后缀足够长的切点。"""
    out = []
    for r in tab[key]:
        if r["F_kind"] != fk or r["n_suf"] < ms:
            continue
        out.append((r["c"], {s: r.get(SIGKEY[s], float("nan")) for s in sigs}))
    return out


def feasibility(tab, sigs, fk, ms, keys, require_unique=False):
    """检测器（各信号取 `≥ 阈值` 的合取）是否存在可行阈值 —— **精确判定，非网格扫描**。

    关键化简：所有信号都是"越大越像边界"，检测器是 `∀s: v_s ≥ thr_s` 的合取 ⇒
      ① 提高任一 thr 只会**减少**判定（单调）；
      ② "老三案例零判定"等价于：**没有任何坏切点 r 在逐分量上 ≥ thr**
         （r ≥ thr ⇒ r 必然也被判为边界）。
    故 thr 取允许的上界 `v0`（vesna 主边界的信号值）时最有利：thr ≤ v0 才能命中主边界，
    而 thr 越大越难被坏切点支配。于是
      **存在可行阈值 ⟺ 不存在坏切点 r 使 r ≥ v0（逐分量）**。
    返回 (是否可行, 否决者, 各信号松弛区间)。
    """
    v0 = {}
    for c, v in _vec(tab, "vesna", fk, ms, sigs):
        if c == VESNA_PRIMARY:
            v0 = v
    if not v0 or any(not np.isfinite(v0[s]) for s in sigs):
        return False, ("vesna@{} 信号不可用（后缀过短/无有效格）".format(VESNA_PRIMARY), -1, {}), {}

    bad = []
    for k in OLD:
        if k in keys:
            bad += [(k,) + x for x in _vec(tab, k, fk, ms, sigs)]
    if require_unique:
        bad += [("vesna",) + x for x in _vec(tab, "vesna", fk, ms, sigs)
                if x[0] != VESNA_PRIMARY]
    for key, c, v in bad:
        if all(np.isfinite(v[s]) and v[s] >= v0[s] for s in sigs):
            return False, (key, c, v), v0
    # 鲁棒余量：坏切点要变成"支配者"还差多少（max 分量差 < 0 表示不支配；越负越安全）
    margin = min(max(v0[s] - v[s] for s in sigs) for _, _, v in bad) if bad else float("inf")
    # 各信号的一维平台：其它信号固定为 v0 时，本信号可行区间为 (lo, v0]
    slack = {}
    for s in sigs:
        lo = -np.inf
        for key, c, v in bad:
            if all(np.isfinite(v[t]) and v[t] >= (v0[t] if t != s else -np.inf) for t in sigs):
                lo = max(lo, v[s])
        slack[s] = (lo, v0[s])
    return True, margin, slack


DETS = [("T", ["T"]), ("A", ["A"]), ("B", ["B"]), ("C", ["C"]),
        ("T∧A", ["T", "A"]), ("T∧B", ["T", "B"]), ("T∧C", ["T", "C"]),
        ("A∧B", ["A", "B"]), ("A∧C", ["A", "C"]), ("B∧C", ["B", "C"]),
        ("T∧A∧B", ["T", "A", "B"]), ("T∧A∧C", ["T", "A", "C"]),
        ("T∧A∧B∧C", ["T", "A", "B", "C"])]


def report_sweep(tab, keys, tag):
    print("  ── 阈值扫描（精确判定：可行 ⟺ 无坏切点逐分量 ≥ vesna@{} 的信号值）──".format(
        VESNA_PRIMARY))
    for name, sigs in DETS:
        for fk in ("max", "last", "dp"):
            for ms in (1, 5, 10):
                ok, viol, slack = feasibility(tab, sigs, fk, ms, keys)
                if ok:
                    v0s = " ".join("{}={:.3f}".format(SIGKEY[s], slack[s][1]) for s in sigs)
                    pl = "  ".join("{}∈({:.3f},{:.3f}]".format(SIGKEY[s], *slack[s]) for s in sigs)
                    print("     {:<9} F={:<4} ms={:<3} ✓可行    v0 {}   鲁棒余量 {:.3f}   一维平台 {}".format(
                        name, fk, ms, v0s, viol, pl))
                else:
                    key, c, v = viol
                    v0s = " ".join("{}={:.3f}".format(SIGKEY[s], slack[s]) for s in sigs) if slack else ""
                    vs = " ".join("{:.3f}".format(v.get(s, float("nan"))) for s in sigs) if v else "-"
                    print("     {:<9} F={:<4} ms={:<3} ✗否决    坏切点 {}@c={} 信号 {}  ≥  v0 {}".format(
                        name, fk, ms, key, c, vs, v0s))


def report_sweep_unique(tab, keys, tag):
    print("  ── 同上，但额外要求 vesna 判定**唯一**（其它切点也不得触发）──")
    for name, sigs in DETS:
        for fk in ("max", "last", "dp"):
            for ms in (1, 5, 10):
                ok, viol, slack = feasibility(tab, sigs, fk, ms, keys, require_unique=True)
                if ok:
                    pl = "  ".join("{}∈({:.3f},{:.3f}]".format(SIGKEY[s], *slack[s]) for s in sigs)
                    print("     {:<9} F={:<4} ms={:<3} ✓唯一可行  余量 {:.3f}  一维平台 {}".format(
                        name, fk, ms, viol, pl))
                else:
                    key, c, v = viol
                    vs = " ".join("{}={:.3f}".format(SIGKEY[s], v.get(s, float("nan")))
                                  for s in sigs) if v else "-"
                    print("     {:<9} F={:<4} ms={:<3} ✗ 坏切点 {}@c={} {}".format(
                        name, fk, ms, key, c, vs))


def report_sep(tab, keys, ks, tag):
    print("  ── 分离度：{} ──".format(tag))
    print("     {:<7} {:>9} {:>26} {:>12} {:>24}".format(
        "信号", "vesna@40", "vesna 其它 min/med/max", "老三 max", "单信号可用阈值区间"))
    for fname in ("max", "last", "dp"):
        for f in ("gap", "A", "B", "C"):
            v0 = get(tab, "vesna", VESNA_PRIMARY, fname, f)
            others = [r[f] for r in tab["vesna"]
                      if r["F_kind"] == fname and r["c"] != VESNA_PRIMARY
                      and np.isfinite(r.get(f, float("nan")))]
            oldv = [r[f] for k in OLD if k in keys for r in tab[k]
                    if r["F_kind"] == fname and np.isfinite(r.get(f, float("nan")))]
            o = np.array(others) if others else np.array([np.nan])
            ov = np.array(oldv) if oldv else np.array([np.nan])
            lo = max(np.nanmax(o) if len(o) else -np.inf,
                     np.nanmax(ov) if len(ov) else -np.inf)
            print("     {:<7} {:>9.3f} {:>26} {:>12} {}".format(
                "{}.{}".format(f, fname[0]), v0,
                "{:.2f}/{:>5.2f}/{:>5.2f}".format(np.nanmin(o), np.nanmedian(o), np.nanmax(o)),
                "{:.2f}".format(np.nanmax(ov)) if len(ov) else "-",
                "({:.3f}, {:.3f}]".format(lo, v0) if (np.isfinite(lo) and v0 > lo)
                else "**空**（v0 {:.3f} ≤ 其它最大 {:.3f}）".format(v0, lo)))


def near_miss(tab, keys, sigs, fk, ms, thr, topn=6, include_vesna=True):
    """→ 最接近"被误判为边界"的切点列表（按 `min_s (v_s − thr_s)` 降序）。

    用途：量化"幸存检测器"的**脆弱性**——若某案例的某切点只差 0.x 秒/0.0x 就触发，
    那它不是结构判据，而是这批素材的数值巧合。
    """
    rows = []
    for k in list(keys):
        for c, v in _vec(tab, k, fk, ms, sigs):
            if not include_vesna and k == "vesna":
                continue
            if k == "vesna" and c == VESNA_PRIMARY:
                continue
            if any(not np.isfinite(v[s]) for s in sigs):
                continue
            slack = min(v[s] - thr[SIGKEY[s]] for s in sigs)
            rows.append((slack, k, c, {s: v[s] for s in sigs}))
    rows.sort(key=lambda x: -x[0])
    return rows[:topn]


def report_evidence(tab, keys, tag):
    """对"幸存检测器"给出**最近的误判候选**（脆弱性证据）。"""
    print("  ── 幸存检测器的脆弱性：最接近触发的非边界切点（差得越少越危险）──")
    cases = [
        ("T∧A", ["T", "A"], "last", 5, {"gap": 5.0, "A": 27.0}),
        ("T∧B", ["T", "B"], "last", 5, {"gap": 5.0, "B": 0.500}),
        ("T∧B", ["T", "B"], "dp", 5, {"gap": 5.0, "B": 0.500}),
    ]
    for name, sigs, fk, ms, thr in cases:
        nm = near_miss(tab, keys, sigs, fk, ms, thr)
        print("     {} F={} ms={} 阈值 {}（vesna@40 命中）".format(name, fk, ms, thr))
        for slack, k, c, v in nm:
            print("        {:<8} c={:<4} 差 {:>8.3f}  {}".format(
                k, c, slack, "  ".join("{}={:.3f}".format(SIGKEY[s], v[s]) for s in sigs)))
    # 老三案例各信号极值（说明"独立 argmax 有多噪"）
    print("     ── 独立 argmax 的噪声：老三案例 A 信号最大的 5 个切点 ──")
    rows = []
    for k in OLD:
        for c, v in _vec(tab, k, "last", 1, ["A"]):
            if np.isfinite(v["A"]):
                rows.append((v["A"], k, c))
    for a, k, c in sorted(rows, reverse=True)[:5]:
        g = get(tab, k, c, "last", "gap")
        print("        {:<8} c={:<4} A={:>7.0f}  gap={:.2f}s".format(k, c, a, g))


def report_rank(tab, keys, tag):
    """c=40 在各信号上的**排名**（1 = 全案例最大）——直接回答"有没有清晰分离"。"""
    print("  ── 主边界 c={} 的排名（越大越像边界 ⇒ 期望排名第 1 且断层明显）──".format(VESNA_PRIMARY))
    print("     {:<8} {:>10} {:>16} {:>16} {:>10}".format(
        "信号", "vesna@40", "vesna 其它最大", "老三最大", "排名"))
    for fk in ("max", "last", "dp"):
        for f in ("gap", "A", "B", "C"):
            v0 = get(tab, "vesna", VESNA_PRIMARY, fk, f)
            if not np.isfinite(v0):
                continue
            others = [r[f] for r in tab["vesna"]
                      if r["F_kind"] == fk and r["c"] != VESNA_PRIMARY
                      and np.isfinite(r.get(f, float("nan")))]
            oldv = [r[f] for k in OLD for r in tab[k]
                    if r["F_kind"] == fk and np.isfinite(r.get(f, float("nan")))]
            n_other = sum(1 for x in others if x > v0)
            n_old = sum(1 for x in oldv if x > v0)
            print("     {:<8} {:>10.3f} {:>16.3f} {:>16.3f} {:>10}".format(
                "{}.{}".format(f, fk), v0,
                max(others) if others else float("nan"),
                max(oldv) if oldv else float("nan"),
                "第 {}（vesna 内 {} 个更高，老三 {} 个更高）".format(
                    1 + n_other + n_old, n_other, n_old)))


def region2(tab, sig2, fk, ms, keys, require_unique=False):
    """两信号（T ∧ sig2）检测器的**精确二维可行域**描述。

    做法：把 T 的阈值 t1 扫过所有出现过的 gap 值（可行域只在断点处变化），对每个 t1
    求"必须超过的最大 sig2 值" `need(t1)`；可行 ⟺ `need(t1) < v0[sig2]`。
    返回 [(t1_lo, t1_hi, need_max, v0_sig2)] 合并后的区间。
    """
    v0g = get(tab, "vesna", VESNA_PRIMARY, fk, "gap")
    v0s = get(tab, "vesna", VESNA_PRIMARY, fk, sig2)
    bad = []
    for k in OLD:
        if k in keys:
            bad += [(k,) + x for x in _vec(tab, k, fk, ms, ["T", sig2])]
    if require_unique:
        bad += [("vesna",) + x for x in _vec(tab, "vesna", fk, ms, ["T", sig2])
                if x[0] != VESNA_PRIMARY]
    cands = sorted({0.0} | {v["T"] for _, _, v in bad if np.isfinite(v["T"])})
    pts = []
    for t1 in cands:
        if t1 > v0g:
            continue
        need = -np.inf
        for key, c, v in bad:
            if np.isfinite(v["T"]) and np.isfinite(v[sig2]) and v["T"] >= t1:
                need = max(need, v[sig2])
        pts.append((t1, need))
    good = [(t1, need) for t1, need in pts if need < v0s]
    if not good:
        return [], v0g, v0s, pts
    # need(t1) 随 t1 单调不增 ⇒ 可行集必是候选序列的**后缀**，即单个区间
    return [(good[0][0], good[-1][0], good[0][1])], v0g, v0s, pts


def report_region2(tab, keys, tag):
    print("  ── 两信号检测器的精确二维可行域（T 的阈值扫全部断点）──")
    for name, sig2 in (("T∧A", "A"), ("T∧B", "B"), ("T∧C", "C")):
        for fk in ("max", "last", "dp"):
            for uniq in (False, True):
                iv, v0g, v0s, pts = region2(tab, sig2, fk, 5, keys, uniq)
                tag2 = "唯一" if uniq else "仅硬门"
                if not iv:
                    print("     {:<5} F={:<4} {:<6} **无可行域**（gap 阈值在 (0, {:.2f}] 内"
                          "任一取值都要求 {} > {:.3f}）".format(
                              name, fk, tag2, v0g, sig2, v0s))
                else:
                    a, b, n = iv[0]
                    print("     {:<5} F={:<4} {:<6} gap ∈ ({:.3f}, {:.2f}] 且 {} ∈ ({:.3f}, {:.3f}]"
                          "   ← 平台宽 {:.3f}s / {:.3f}".format(
                              name, fk, tag2, a, b, sig2, n, v0s, b - a, v0s - n))


def region_2d(tab, sig1, sig2, fk, ms, keys, require_unique=False):
    """任意两信号检测器（sig1 ∧ sig2）的**精确二维可行域**。

    `need(t1)` = 在 `sig1 ≥ t1` 的坏切点里 `sig2` 的最大值 ⇒ 阈值必须 `> need(t1)`。
    `need` 随 `t1` 单调不增 ⇒ 可行集是候选序列的后缀。
    返回 (可行点列表 [(t1, need, v0_sig2)], v0_sig1, v0_sig2)。
    """
    v0a = get(tab, "vesna", VESNA_PRIMARY, fk, sig1)
    v0b = get(tab, "vesna", VESNA_PRIMARY, fk, sig2)
    bad = []
    for k in OLD:
        if k in keys:
            bad += [(k,) + x for x in _vec(tab, k, fk, ms, [sig1, sig2])]
    if require_unique:
        bad += [("vesna",) + x for x in _vec(tab, "vesna", fk, ms, [sig1, sig2])
                if x[0] != VESNA_PRIMARY]
    if not np.isfinite(v0a) or not np.isfinite(v0b):
        return [], v0a, v0b
    cands = sorted({v[sig1] for _, _, v in bad if np.isfinite(v[sig1])} | {v0a})
    pts = []
    for t1 in cands:
        if t1 > v0a:
            continue
        need = -np.inf
        for key, c, v in bad:
            if np.isfinite(v[sig1]) and np.isfinite(v[sig2]) and v[sig1] >= t1:
                need = max(need, v[sig2])
        if need < v0b:
            pts.append((t1, need, v0b))
    return pts, v0a, v0b


def report_region_2d(tab, keys, tag):
    print("  ── A∧B 的精确二维可行域（sig1 = A；B 三种前沿口径）──")
    for fk in ("max", "last", "dp"):
        for ms in (5, 10):
            for uniq in (False, True):
                pts, v0a, v0b = region_2d(tab, "A", "B", fk, ms, keys, uniq)
                tn = "唯一" if uniq else "仅硬门"
                if not pts:
                    print("     A∧B F={:<4} ms={:<3} {:<6} **无可行域**".format(fk, ms, tn))
                    continue
                bnd = "  ".join("A≥{:.0f}→B>{:.3f}".format(
                    t1, nd) for t1, nd, _ in pts[::max(1, len(pts) // 6)])
                print("     A∧B F={:<4} ms={:<3} {:<6} A ∈ [{:.0f}, {:.0f}]（B 上界 {:.3f}）；"
                      "边界 {}".format(fk, ms, tn, pts[0][0], v0a, v0b, bnd))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", nargs="*", default=None)
    ap.add_argument("--trace", action="store_true")
    ap.add_argument("--min-suffix", type=int, default=1)
    ap.add_argument("--ksmooth", type=int, default=5)
    ap.add_argument("--dump", default=None, help="把信号表落盘为 JSON（标定证据留档）")
    args = ap.parse_args()
    keys = args.cases or KEYS

    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = FC.build_matrices(keys, tok, sess)
    for k in keys:
        with io.open(os.path.join(BENCH_OUT, "truth_{}.json".format(k)), encoding="utf-8") as f:
            data[k]["rows"] = json.load(f)["rows"]

    report_trace(data, keys)
    if args.trace:
        return 0

    # ── 阶段 1b：信号表（原始口径）──
    print()
    print("=" * 108)
    print("阶段 1b：候选切点信号值（全部只用产出段与 S 矩阵，不含参考文本）")
    tab0 = sig_table(data, keys, args.min_suffix, 1)
    if "vesna" in keys:
        report_table(data, tab0, "vesna", "max")
    for k in OLD:
        if k in keys:
            report_table(data, tab0, k, "max")

    # ── 阶段 1c：分离度 + 阈值扫描 ──
    for tab, ks, tag in ((tab0, 1, "原始口径（独立 argmax 不平滑）"),
                         (sig_table(data, keys, args.min_suffix, args.ksmooth),
                          args.ksmooth, "中位数平滑口径 K={}".format(args.ksmooth))):
        print()
        print("=" * 108)
        print("阶段 1c/1d：分离度与阈值扫描 —— {}".format(tag))
        report_sep(tab, keys, ks, tag)
        print()
        report_rank(tab, keys, tag)
        print()
        report_sweep(tab, keys, tag)
        print()
        report_sweep_unique(tab, keys, tag)
        print()
        report_region2(tab, keys, tag)
        print()
        report_region_2d(tab, keys, tag)
        print()
        report_evidence(tab, keys, tag)
        if args.dump:
            ser = {}
            for k in keys:
                ser[k] = [r for r in tab[k]]
            with io.open("{}.k{}.json".format(args.dump, ks), "w", encoding="utf-8",
                         newline="") as f:
                json.dump({"tag": tag, "ksmooth": ks, "tables": ser}, f,
                          ensure_ascii=False, indent=1, default=float)
    return 0


if __name__ == "__main__":
    sys.exit(main())
