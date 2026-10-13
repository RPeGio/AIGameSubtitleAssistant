#!/usr/bin/env python3
"""§7.7 重评（2026-10-08，**评估框架修正后**）：显式重播分段的边界检测器可行性。

上一轮（`scripts/fuse_seg_calib.py`，见 `benchmark/FUSE_THRESHOLD_CALIBRATION.md` §7.7）
用**「唯一性」判据**否决了本方案：要求检测器**只**标出 vesna `c=40`。用户核对素材后指出
真实边界**约 6 处**（主播完整重看一遍 = 1 次大回退，另有约 5 次局部拖进度条导致的跳进）
⇒ 「唯一性」是错误框架；正确问法是**「能否找出全部边界」**（查全为主、查准为辅）。

仍然成立的硬事实（本轮不推翻）：单信号 T/A/B/C **都不存在宽平台**，且 `c=40` 不是任何
单信号的极值。故本轮**换信号族**，不再在单点阈值上打转。

本轮两步：
  阶段 1a  建立**可靠真边界标注**。`truth_<case>.json` 的 `ref_to_corpus` 是**参考块级**的
           （由用户手打的参考文本推出，与 OCR 分段无关）⇒ 其 `corpus_index` 序列**一回退
           就是一次拖进度条**，是最干净的真值来源。据此取全部回退点 → 映射到段下标空间
           （时间上最近的切点）→ 与段级真值轨迹 `rows[].truth` 的回跳点交叉核对。
  阶段 1b  换信号族：测**窗口内的整体行为**而非切点瞬间值。
           P1 持续后退占比、P2 最长后退游程、P3 两侧置信度、P4 = P1∧T、
           **P5 孪生（块重复）onset** —— "切点之后每一段都能在**切点之前**找到高度相似的
           孪生段"，这是"整段内容被重新覆盖"的直接量化，也是上一轮 §7.7 末尾记的
           **唯一有希望方向**（内容覆盖检测）。孪生关系只消费产出段自身（文本 + 段间相似度），
           完全不依赖参考文本，也不依赖语料。

评估口径（本轮重点）：查全率 / 查准率 / 误标位置分布 / 阈值平台宽度 / 硬门
（moon/glupov/pierro 必须零边界判定）。

用法：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    & runtime\\python\\python.exe scripts\\fuse_seg_calib2.py
    & runtime\\python\\python.exe scripts\\fuse_seg_calib2.py --cases vesna --dump temp\\seg2
"""
import argparse
import io
import json
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "..", "src-tauri", "tests"))
# 本脚本 import 同目录的 fuse_calib，会在 scripts/ 下留 __pycache__（仓库不跟踪它）
sys.dont_write_bytecode = True

import numpy as np  # noqa: E402
from fuse_lab import BENCH_OUT, load_embedder  # noqa: E402
import fuse_calib as FC  # noqa: E402
import fuse_seg_calib as FS  # noqa: E402

KEYS = ["moon", "glupov", "pierro", "vesna"]
OLD = ["moon", "glupov", "pierro"]
NEG = FC.NEG

# 边界命中容差（秒）。参考块与产出段的时间轴相差 ±0.5s 量级（手打轴抖动 + OCR 段界），
# 但"边界是不是同一处"是**语义**判断，取 10s：既容得下 ±1 段的错位，又不会把
# vesna 两个相距 45s 以上的候选并成一类。
TOL_SEC = 10.0


# ══════════════════════════ 阶段 1a：真边界标注 ══════════════════════════

def ref_retreats(r2c):
    """参考块级**回退点**：corpus_index 严格下降且两值都 > 0（0 = 该参考块无语料对应）。

    返回 [{b, t, drop, gap, a, ci}]，b = 参考块下标（新的一遍从 b 开始）。
    """
    out = []
    for i in range(1, len(r2c)):
        a, b = r2c[i - 1]["corpus_index"], r2c[i]["corpus_index"]
        if a > 0 and b > 0 and b < a:
            out.append({"b": i, "t": r2c[i]["t0"], "drop": a - b, "a": a, "ci": b,
                        "gap": r2c[i]["t0"] - r2c[i - 1]["t1"]})
    return out


def ref_repeats(r2c):
    """参考块级**紧邻同值**（同一语料行被连打两次 ⇒ 该显示被重放，但顺序没后退）。

    与回退点性质不同（不破坏单调性），只用于"若把 6 处理解为'不连续事件'"的敏感性分析。
    """
    out = []
    for i in range(1, len(r2c)):
        a, b = r2c[i - 1]["corpus_index"], r2c[i]["corpus_index"]
        if a > 0 and a == b:
            out.append({"b": i, "t": r2c[i]["t0"], "ci": a,
                        "gap": r2c[i]["t0"] - r2c[i - 1]["t1"]})
    return out


def ref_retreats_content(r2c, corpus, rep):
    """**内容顺序**上的回退点：先剔除"无语料对应"与"**无判别性正文**"的参考块（名牌行/省略号行
    在内容上不占位），再把语料下标折算成**等价类代表下标**，最后找严格下降。

    为什么必须做这两步（实测）：
      · 等价类：glupov 的 `22→2`（[1]↔[22]）、`23→4` 等 4 处是**同一条台词被 OCR 收进两次**，
        裸下标在退、内容在前进；
      · 无正文行：glupov 第 5 处 `30→18` 里的 `语料[30]` 是 `安东 / 头衔 / …`（正文归一化为空），
        它是**名牌态**显示，不占内容位 ⇒ 内容顺序是 `17→18`（前进）。
    剔除后 glupov **0 处**、moon/pierro 0 处、vesna 仍是那 4 处（与段级真值轨迹逐处一致）。
    """
    seq = []
    for i, x in enumerate(r2c):
        j = x["corpus_index"]
        if j <= 0 or not FC.body_norm(corpus[j - 1]):
            continue
        seq.append((i, x["t0"], j))
    out = []
    for pa, pb in zip(seq[:-1], seq[1:]):
        va, vb = rep[pa[2] - 1], rep[pb[2] - 1]
        if vb < va:
            i = pb[0]
            out.append({"b": i, "t": pb[1], "drop": va - vb, "a": pa[2], "ci": pb[2],
                        "gap": r2c[i]["t0"] - r2c[i - 1]["t1"]})
    return out


def nearest_cut(rows, t):
    """把参考块级边界（时间 t）映射到**段下标空间**：取时间上最近的切点 c。

    切点 c 的含义：序列切成 `[0, c)` 与 `[c, n)`，即第 c 段（0-based）是后一块的第一段。
    """
    k = min(range(len(rows)), key=lambda i: abs(rows[i]["start"] - t))
    return max(1, min(len(rows) - 1, k))


def class_rep(cls):
    """→ 每个语料下标的**等价类代表下标**（取类内最小下标 ⇒ 类之间可比大小）。

    必要性：glupov 的"回退"是 `22 → 2` 这种**同一条台词被 OCR 收进两次**造成的（[1]↔[22]），
    裸下标在退、**内容顺序其实在前进**。只比较"类 id 是否相等"不够（类 id 无序），
    要先把每条映射到类内最小下标再比大小。
    """
    first = {}
    for j, c in enumerate(cls):
        first.setdefault(c, j)
    return [first[c] for c in cls]


def truth_of(key, d, cls=None):
    """→ 该案例的全部真值口径（段下标空间，已排序去重）。

    `cuts_real`：只保留**跨等价类**的回退——语料近重复等价类内的"回退"只是换了个下标，
    内容顺序并没有后退（glupov 的 5 处回退全部属此类）。硬门与"主边界"都按这一口径判。
    """
    rows = d["rows"]
    ret = ref_retreats(d["ref_to_corpus"])
    rpt = ref_repeats(d["ref_to_corpus"])
    rep = class_rep(cls) if cls is not None else list(range(len(d["corpus"])))
    real = ref_retreats_content(d["ref_to_corpus"], d["corpus"], rep)
    arts = [x for x in ret if x not in real]
    cuts_ret = sorted({nearest_cut(rows, x["t"]) for x in ret})
    cuts_real = sorted({nearest_cut(rows, x["t"]) for x in real})
    cuts_rpt = sorted({nearest_cut(rows, x["t"]) for x in rpt})
    big = [max(real, key=lambda x: x["drop"])] if real else []
    cuts_big = sorted({nearest_cut(rows, x["t"]) for x in big})
    jumps = sorted({j["i"] for j in FS.truth_jumps(rows)})
    return {"retreats": ret, "repeats": rpt, "real": real, "artifacts": arts,
            "cuts_retreat": cuts_ret, "cuts_real": cuts_real, "cuts_repeat": cuts_rpt,
            "cuts_big": cuts_big,
            "cuts_all": sorted(set(cuts_ret) | set(cuts_rpt)), "cuts_jump": jumps}


def report_trace(data, keys, draw=True):
    print("=" * 112)
    print("阶段 1a：真边界标注（参考块级 ref_to_corpus → 段下标空间；只读真值产物）")
    for k in keys:
        d = data[k]
        T = truth_of(k, d, d.get("cls"))
        rows = d["rows"]
        print()
        print("── {}：参考块 {} 个 / 产出段 {} 个 / 语料 {} 行（等价类 {} 个）".format(
            k, len(d["ref_to_corpus"]), len(rows), len(d["corpus"]),
            len(set(d["cls"])) if d.get("cls") else "-"))
        print("   裸 corpus_index 回退点：**{} 处**；**内容顺序**回退点（剔等价类/无正文行）："
              "**{} 处**{}".format(
                  len(T["retreats"]), len(T["real"]),
                  "" if not T["real"] else "（1 大 + {} 小）".format(
                      sum(1 for x in T["real"] if x["drop"] < 10))))
        if T["real"] and draw:
            print("     {:>3} {:>9} {:>10} {:>7} {:>8} {:>10} {:>9}  段文本".format(
                "块", "t(s)", "ci a→b", "落差", "gap(s)", "段切点 c", "段 c 时间"))
            for x in T["real"]:
                c = nearest_cut(rows, x["t"])
                print("     {:>3} {:>9.2f} {:>10} {:>7} {:>8.2f} {:>10} {:>9.2f}  {}".format(
                    x["b"], x["t"], "{}→{}".format(x["a"], x["ci"]), x["drop"], x["gap"],
                    c, rows[c]["start"], rows[c]["text"][:38].replace("\n", " / ")))
        if T["artifacts"]:
            print("   裸下标假象（内容顺序未退）：{}".format(
                ["b={} {}→{}".format(x["b"], x["a"], x["ci"]) for x in T["artifacts"]]))
        if T["repeats"]:
            print("   参考块级**紧邻同值**（顺序未后退，属重放同一行）：{}".format(
                ["b={} t={:.2f} ci={} gap={:.2f}".format(x["b"], x["t"], x["ci"], x["gap"])
                 for x in T["repeats"]]))
        print("   段空间真边界集合：裸下标 {} | **内容顺序** {} | 回退+同值 {} | "
              "主边界（最大落差） {} | 段级真值轨迹回跳 {}".format(
                  T["cuts_retreat"], T["cuts_real"], T["cuts_all"], T["cuts_big"],
                  T["cuts_jump"]))
        # 交叉核对：参考块级映射 vs 段级真值轨迹（都按**内容顺序**口径）
        agree = sorted(set(T["cuts_real"]) & set(T["cuts_jump"]))
        only_ref = sorted(set(T["cuts_real"]) - set(T["cuts_jump"]))
        only_jump = sorted(set(T["cuts_jump"]) - set(T["cuts_real"]))
        print("   交叉核对：一致 {} 处 {}｜仅参考块级 {}｜仅真值轨迹 {}".format(
            len(agree), agree, only_ref, only_jump))
        if only_ref or only_jump:
            for c in sorted(set(only_ref) | set(only_jump)):
                r = rows[c]
                print("      c={:<4} t={:>8.2f} text={!r} ref_text={!r} truth={} truth_ok={}".format(
                    c, r["start"], r["text"][:40], (r["ref_text"] or "")[:20],
                    r["truth"], r["truth_ok"]))
        if T["cuts_big"]:
            report_two_pass(rows, T)


def report_two_pass(rows, T):
    """两遍对照：同一显示在两遍里的时间间隙**是否相同**——用来判定"大间隙"是什么。

    判据：把主边界之后的段按正文逐字匹配回主边界之前的段（两遍放的是同一批显示），
    逐对比较 `gap = start[i] − end[i−1]`。
      · 若间隙**是素材/播放的性质**（某段没有字幕），两遍应当**一样大**；
      · 若间隙是主播**暂停**造成的，两遍就会不同（暂停只发生在其中一遍）。
    实测（vesna）：38 对里 **37 对相差 ≤ 1.5s**（多数 < 1s）⇒ 间隙**主要是素材自身的性质**，
    不是拖进度条；只有 1 对差 **+33.9s**（`c=7` 的 24.97s vs `c=47` 的 58.87s）⇒ 那一处
    主播多停/卡了 34s。**两种成因都与"内容顺序是否回退"无关**——这正是最大间隙出现在
    `c=47`（第二遍**内部**、真值在前进）而不是主边界 `c=40` 的原因：`T` 连"素材结构"都不算，
    它测的是**播放行为**，只能当精化门用（滤掉零间隙的重复显示）。
    """
    cb = T["cuts_big"][0]
    def body(t):
        return FC.body_norm(t)
    pairs = []
    used = set()
    for i in range(cb, len(rows)):
        bi = body(rows[i]["text"])
        if not bi:
            continue
        for j in range(1, cb):                    # j=0 无前置段 ⇒ 间隙无定义
            if j in used or body(rows[j]["text"]) != bi:
                continue
            used.add(j)
            gi = rows[i]["start"] - rows[i - 1]["end"] if i > 0 else float("nan")
            gj = rows[j]["start"] - rows[j - 1]["end"]
            pairs.append((j, i, gj, gi, gi - gj))
            break
    pairs.sort(key=lambda x: -abs(x[4]))
    print("   两遍对照（同一显示：第二遍 c={} 起 vs 第一遍）：匹配 {} 对，"
          "|间隙差| > 5s 的 {} 对".format(cb, len(pairs), sum(1 for p in pairs if abs(p[4]) > 5)))
    print("     {:>4} {:>4} {:>9} {:>9} {:>9}  {}".format(
        "第一遍", "第二遍", "gap1(s)", "gap2(s)", "差", "显示正文"))
    for j, i, gj, gi, dd in pairs[:6]:
        print("     {:>4} {:>4} {:>9.2f} {:>9.2f} {:>9.2f}  {}".format(
            j, i, gj, gi, dd, rows[j]["text"][:34].replace("\n", " / ")))


# ══════════════════════════ 阶段 1b：新信号族 ══════════════════════════

def seg_sim(tok, sess, data):
    """段 × 段 相似度（产出段自身文本，**不含参考、不含语料下标**）。"""
    sb = [FC.split_header(t)[1] for t in data["seg_texts"]]
    return FC.similarity_matrix(tok, sess, sb, sb), sb


def twin_table(SS, sb, tau=0.90, lmin=3, min_chars=0):
    """→ (twin, elig, sim)：`twin[i]` = i 之前（lag ≥ lmin）最相似的孪生段下标（-1 = 无）。

    τ 门 + lag 门 + 可选"实质字符数"门（`min_chars`，仍只用产出段文本）。
    孪生 = 同一显示被**重新播放**过 ⇒ 重播区每一段都该有孪生段，重播区之外不该有。
    """
    n = SS.shape[0]
    elig = [len(sb[i]) >= min_chars for i in range(n)] if min_chars > 0 else [True] * n
    twin, sim = [-1] * n, [0.0] * n
    for i in range(n):
        if not elig[i]:
            continue
        bj, bs = -1, -1.0
        for j in range(0, i - lmin + 1):
            if not elig[j]:
                continue
            v = SS[i, j]
            if v > bs:
                bj, bs = j, v
        if bj >= 0 and bs >= tau:
            twin[i], sim[i] = bj, float(bs)
    return twin, elig, sim


def P1(am, c, W, kind="max"):
    """P1 持续后退占比：窗口 `[c, c+W)` 内独立 argmax 落在"切点前窗口参考水平之下"的占比。

    与上一轮 B（对**全局前缀前沿**比较）的关键差别：这里比的是**紧邻的局部水平**。
    重播边界把水平整体抬高（前缀刚走完整个语料）又重置（后缀从头爬），局部比较能看见，
    全局比较看不见（上一轮 `c=47` 的 `B.dp = 1.000` 即此）。
    """
    n = len(am)
    bef = [x for x in am[max(0, c - W):c] if x >= 0]
    aft = [x for x in am[c:min(n, c + W)] if x >= 0]
    if not bef or not aft:
        return float("nan")
    lvl = max(bef) if kind == "max" else float(np.median(bef))
    return sum(1 for x in aft if x < lvl) / float(len(aft))


def P2(am, c, W):
    """P2 最长后退游程：`[c, c+W)` 内 argmax 连续严格下降的最长游程。"""
    n = len(am)
    best = cur = 0
    for i in range(c + 1, min(n, c + W)):
        if am[i] >= 0 and am[i - 1] >= 0 and am[i] < am[i - 1]:
            cur += 1
            best = max(best, cur)
        else:
            cur = 0
    return float(best)


def P3(rm, c, W):
    """P3 两侧置信度：切点前后两侧"最佳匹配相似度"的平均值取小（真边界两侧都应有真实匹配）。"""
    n = len(rm)
    bef, aft = rm[max(0, c - W):c], rm[c:min(n, c + W)]
    if not len(bef) or not len(aft):
        return float("nan")
    return float(min(bef.mean(), aft.mean()))


def P5(twin, elig, c, W, min_lag=None):
    """P5 孪生 onset：`[c, c+W)` 内"有孪生段"的占比 − `[c−W, c)` 内同占比。

    重播区起点是这条曲线**从 0 跳到 1 的位置**；重播区**内部**的两侧都是 1 ⇒ 差值 0
    （这正是上一轮 B 失效的地方：它只有"后缀侧"，没有"onset"）。
    """
    n = len(twin)
    aft = [i for i in range(c, min(n, c + W)) if elig[i]]
    bef = [i for i in range(max(0, c - W), c) if elig[i]]
    if not aft:
        return float("nan")
    fa = sum(1 for i in aft if twin[i] >= 0) / float(len(aft))
    fb = (sum(1 for i in bef if twin[i] >= 0) / float(len(bef))) if bef else 0.0
    return fa - fb


def S6(SS, c, tau, lmin, W):
    """S6 块重复**onset** 游程：切点 c 之后最多连续多少段能按**同一 lag** 与切点之前的段逐段配对，
    且该配对**不得向前延伸**（否则它是重播区内部，不是起点）。

    与 P5 互补：P5 只看"有没有孪生段"，S6 要求"孪生段构成一个连续的块"（更像"整段被重放"）。
    onset 条件（`SS[c-1, c-L-1] < tau`）是必需的、不是补丁：不加它，整段重播区的**每一个**
    切点都拿到满窗（实测 `c=42..47` 全是 5），信号退化成"在重播区里"而非"重播区从这开始"。
    """
    n = SS.shape[0]
    best = 0
    for L in range(lmin, c + 1):
        if c - L - 1 >= 0 and SS[c - 1, c - L - 1] >= tau:
            continue                      # 配对可向前延伸 ⇒ 非 onset
        k = 0
        while c + k < n and c - L + k >= 0 and SS[c + k, c - L + k] >= tau and k < W:
            k += 1
        best = max(best, k)
    return float(best)


def build_signals(key, data, tok, sess, W, tau, lmin, min_chars):
    """→ 该案例的全部信号向量（下标 = 切点 c ∈ [1, n−1]）。"""
    S = data["S"]
    rows = data["rows"]
    n = S.shape[0]
    SS, sb = seg_sim(tok, sess, data)
    am = FS.argmax_of(S)
    rm = S.max(axis=1)
    twin, elig, tsim = twin_table(SS, sb, tau, lmin, min_chars)
    g = FS.gaps_of(rows)
    out = []
    for c in range(1, n):
        sw = [tsim[i] for i in range(c, min(n, c + W)) if elig[i] and twin[i] >= 0]
        out.append({
            "c": c, "t": rows[c]["start"], "gap": g[c],
            "P1max": P1(am, c, W, "max"), "P1med": P1(am, c, W, "med"),
            "P2": P2(am, c, W), "P3": P3(rm, c, W),
            "P5": P5(twin, elig, c, W),
            "S6": S6(SS, c, tau, lmin, W),
            "tsim": float(np.median(sw)) if sw else float("nan"),
            "am": am[c], "am_prev": am[c - 1], "rowmax": float(rm[c]),
            "noise": sum(1 for i in range(c, min(n, c + W))
                         if FC.content_free(data["seg_texts"][i])) / float(
                             max(1, min(W, n - c))),
        })
    return out, twin, elig, am, rm


# ══════════════════════════ 评估 ══════════════════════════

def evaluate(det, truth, rows, tol=TOL_SEC):
    """→ 查全/查准。检测点按时间聚簇（簇内间隔 ≤ tol）后，含真边界的簇算命中，其余算误标。

    误标另按"到最近真边界的时间差"分桶（`fp_dist`）：与真边界同属一次事件（≤2·tol）的误标
    与"落在单调播放遍内部"的误标，危害完全不同——后者才会把一遍拆成若干块、
    每块各自从语料头部重新起步（分段方案要避免的失效模式）。
    """
    det = sorted(det)
    truth = sorted(truth)
    hit = set()
    for tc in truth:
        for c in det:
            if abs(rows[c]["start"] - rows[tc]["start"]) <= tol:
                hit.add(tc)
                break
    cl = []
    for c in det:
        if cl and rows[c]["start"] - rows[cl[-1][-1]]["start"] <= tol:
            cl[-1].append(c)
        else:
            cl.append([c])
    fp, dist = [], []
    for g in cl:
        if any(abs(rows[c]["start"] - rows[tc]["start"]) <= tol
               for c in g for tc in truth):
            continue
        fp.append(g)
        dist.append(min(abs(rows[c]["start"] - rows[tc]["start"])
                        for c in g for tc in truth) if truth else float("inf"))
    return {"hit": sorted(hit), "recall": (len(hit) / float(len(truth))) if truth else float("nan"),
            "n_det": len(det), "n_fp_cluster": len(fp), "fp": fp, "fp_dist": dist,
            "clusters": cl}


def report_signals(data, tab, key, cuts, title):
    print("  ── {} 信号值（{}）──".format(key, title))
    print("     {:>4} {:>8} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6}  {}".format(
        "c", "start", "gap", "P1max", "P1med", "P2", "P3", "P5", "S6", "标记"))
    marked = set(cuts.get("cuts_retreat", [])) | set(cuts.get("cuts_jump", []))
    for r in tab[key]:
        if r["c"] not in cuts.get("show", []):
            continue
        mk = ""
        if r["c"] in cuts.get("cuts_retreat", []):
            mk = "◆真边界"
        if r["c"] in cuts.get("cuts_jump", []) and r["c"] not in cuts.get("cuts_retreat", []):
            mk += "（真值轨迹）"
        print("     {:>4} {:>8.2f} {:>7.2f} {:>7.2f} {:>7.2f} {:>7.0f} {:>7.3f} {:>7.2f} {:>6.0f}  {}".format(
            r["c"], r["t"], r["gap"], r["P1max"], r["P1med"], r["P2"], r["P3"], r["P5"], r["S6"], mk))


def top_of(tab, key, field, k=8, exclude=()):
    rows = [r for r in tab[key] if r["c"] not in exclude
            and np.isfinite(r.get(field, float("nan")))]
    rows.sort(key=lambda r: -r[field])
    return rows[:k]


def scan_1d(tab, keys, field, thrs, truth, tol=TOL_SEC):
    """单信号阈值扫描：→ 每个阈值下的 (硬门, vesna 查全, vesna 误标簇, 检测点)。"""
    out = []
    for thr in thrs:
        det = {k: [r["c"] for r in tab[k]
                   if np.isfinite(r.get(field, float("nan"))) and r[field] >= thr]
               for k in keys}
        gate = all(not det[k] for k in OLD if k in keys)
        ev = evaluate(det["vesna"], truth["cuts_retreat"], tab["_rows"]["vesna"], tol) \
            if "vesna" in keys else None
        evb = evaluate(det["vesna"], truth["cuts_big"], tab["_rows"]["vesna"], tol) \
            if "vesna" in keys else None
        evj = evaluate(det["vesna"], truth["cuts_jump"], tab["_rows"]["vesna"], tol) \
            if "vesna" in keys else None
        out.append({"thr": thr, "gate": gate, "det": det, "ev": ev, "evb": evb, "evj": evj})
    return out


def plat(tab, keys, field, thrs, truth, tol=TOL_SEC):
    """→ 满足「硬门 ∧ 主边界查全」的阈值区间（平台宽度）。"""
    good = []
    for s in scan_1d(tab, keys, field, thrs, truth, tol):
        if s["gate"] and s["evb"] and s["evb"]["recall"] == 1.0:
            good.append(s["thr"])
    if not good:
        return None
    return (min(good), max(good))


# ══════════════════════════ 主流程 ══════════════════════════

# ══════════════════════════ 合成重播：召回包线的机制测试 ══════════════════════════
#
# 上一轮否决的根因之一是"本批只有 1 个正样本"。本轮真值给出 vesna **4 处**回退，但
# 有回退的案例仍只有 1 个。为了不让"能不能找到边界"只靠一个案例，这里用**合成重播**
# 扩正样本：把任一无重播案例的段序列 `[0,k)` 与 `[k−L,k)`（重播 L 段）拼接，拼接处
# 加一个停顿（保证 T 门可过）。它只用已算好的 S / 段文本 / 段间相似度 ⇒ 不需要新素材。
#
# **口径声明**：重播块是逐字相同的拷贝 ⇒ 孪生相似度退化为 1.0，这是"重 OCR 完全干净"
# 的**上界**；真素材（vesna）实测孪生相似度见 1b-4 的 `tsim` 列。故合成测试只回答
# **"信号机制是否在正确的结构上触发"**与**"重播多长才够被看见"**，不替代真实素材验证。

def synth_case(data, k, L, pause=10.0):
    """→ 合成序列（rows[0:k] + rows[k−L:k]），并把拼接处的时间停顿设为 `pause`。

    **底座只取第一个单调块**（有真边界时取 `[0, 第一个切点)`）：否则合成的"重播"会和
    素材自身的重播区叠在一起（vesna 的 pass 2 本身就是 pass 1 的拷贝），测的就不是
    检测器而是双重拷贝下的孪生歧义。
    """
    rows = data["rows"]
    idx = list(range(0, k)) + list(range(k - L, k))
    st, en = [], []
    for i in idx[:k]:
        st.append(rows[i]["start"])
        en.append(rows[i]["end"])
    for j, i in enumerate(idx[k:]):
        s0 = (en[-1] + pause) if j == 0 else (en[-1] + max(
            0.0, rows[i]["start"] - rows[i - 1]["end"]))
        st.append(s0)
        en.append(s0 + (rows[i]["end"] - rows[i]["start"]))
    gr = [float("nan")] + [st[i] - en[i - 1] for i in range(1, len(st))]
    texts = [data["seg_texts"][i] for i in idx]
    return {"S": data["S"][idx], "seg_texts": texts, "idx": idx, "cut": k, "gap": gr,
            "rows": [{"start": st[i], "end": en[i], "text": texts[i]}
                     for i in range(len(idx))]}


def synth_sweep(data, keys, tok, sess, W, tau, lmin, mc, p5_min, min_gap, truth=None):
    """→ [(case, k, L, 检测点, 是否命中拼接切点, 误标数)]

    重播长度扫 {2,3,5,8,15,30}：`L=2` 是"低于分辨率"的对照（与 vesna 尾部 c=80/81 同量级）。
    """
    out = []
    for k in keys:
        if k not in data:
            continue
        SS, sb = seg_sim(tok, sess, data[k])
        n0 = len(data[k]["rows"])
        base = truth[k]["cuts_big"][0] if (truth and truth.get(k, {}).get("cuts_big")) else n0
        n = base
        for pos in sorted({max(6, n // 2), max(6, n - 2)}):
            for L in (2, 3, 5, 8, 15, 30):
                if pos - L < 1 or pos >= n:
                    continue
                c = synth_case(data[k], pos, L)
                SS2 = SS[np.ix_(c["idx"], c["idx"])]
                sb2 = [sb[i] for i in c["idx"]]
                twin, elig, _ = twin_table(SS2, sb2, tau, lmin, mc)
                det = [t for t in range(1, len(c["idx"]))
                       if c["gap"][t] >= min_gap and P5(twin, elig, t, W) >= p5_min]
                hit = 1 if any(abs(t - c["cut"]) <= 1 for t in det) else 0
                # 误标只数**簇**：把间隔 ≤ W 的检测点并成一个
                cl = []
                for t in det:
                    if cl and t - cl[-1][-1] <= W:
                        cl[-1].append(t)
                    else:
                        cl.append([t])
                fp = [g for g in cl if not any(abs(t - c["cut"]) <= 1 for t in g)]
                out.append((k, pos, L, det, hit, fp))
    return out


def report_synth(data, keys, tok, sess, W, tau, lmin, mc, p5_min=0.4, min_gap=5.0,
                 truth=None):
    print()
    print("=" * 112)
    print("阶段 1b-5：合成重播的召回包线（拼接处切点 = 真边界；τ={} W={} P5≥{} ∧ gap≥{}s）".format(
        tau, W, p5_min, min_gap))
    print("     {:<8} {:>4} {:>4}  {:<22} {:>5} {:>6}  {}".format(
        "案例", "位置", "重播长", "检测点", "命中", "误标簇", "备注"))
    res = synth_sweep(data, keys, tok, sess, W, tau, lmin, mc, p5_min, min_gap, truth)
    for k, pos, L, det, hit, fp in res:
        print("     {:<8} {:>4} {:>4}  {:<22} {:>5} {:>6}  {}".format(
            k, pos, L, str(det), "✓" if hit else "✗", len(fp),
            "重播被看见" if hit else "**低于检测分辨率**（小程序回退，由 reset 承担）"))
    tot = {}
    for k, pos, L, det, hit, fp in res:
        tot.setdefault(L, [0, 0])
        tot[L][0] += hit
        tot[L][1] += 1
    print("     ── 按重播长度汇总（命中/总数）──")
    for L in sorted(tot):
        print("        L={:<3} {}/{}".format(L, tot[L][0], tot[L][1]))
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", nargs="*", default=None)
    ap.add_argument("--dump", default=None)
    ap.add_argument("--no-embed", action="store_true", help="跳过孪生类信号（只测 P1~P4）")
    ap.add_argument("--synthetic", action="store_true", help="追加合成重播的召回包线")
    args = ap.parse_args()
    keys = args.cases or KEYS

    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = FC.build_matrices(keys, tok, sess)
    for k in keys:
        with io.open(os.path.join(BENCH_OUT, "truth_{}.json".format(k)), encoding="utf-8") as f:
            data[k]["rows"] = json.load(f)["rows"]
        with io.open(os.path.join(BENCH_OUT, "truth_{}.json".format(k)), encoding="utf-8") as f:
            data[k]["ref_to_corpus"] = json.load(f)["ref_to_corpus"]

    report_trace(data, keys)

    if args.no_embed:
        return 0

    # ── 参数主档：W=5、τ=0.90、lag ≥ 3、不设字符门 ──
    W, tau, lmin, mc = 5, 0.90, 3, 0
    print()
    print("=" * 112)
    print("阶段 1b：新信号族（W={} τ={} lag≥{} 字符门={}）".format(W, tau, lmin, mc))
    tab, twins = {}, {}
    for k in keys:
        tab[k], tw, el, am, rm = build_signals(k, data[k], tok, sess, W, tau, lmin, mc)
        twins[k] = {"twin": tw, "elig": el, "am": am, "rm": rm}
    tab["_rows"] = {k: data[k]["rows"] for k in keys}
    truth = {k: truth_of(k, data[k], data[k].get("cls")) for k in keys}

    for k in keys:
        T = truth[k]
        show = sorted(set(T["cuts_retreat"]) | set(T["cuts_jump"]))
        for c in T["cuts_big"]:
            show += [c - 1, c + 1]
        show = sorted({c for c in show if 0 < c < len(data[k]["rows"]) - 1})
        report_signals(data, tab, k, {"cuts_retreat": T["cuts_retreat"],
                                      "cuts_jump": T["cuts_jump"], "show": show}, "真值邻域")
        print("     ── {} 各信号 Top-6（不含真值邻域）──".format(k))
        for f in ("P1max", "P1med", "P2", "P3", "P5", "S6", "gap"):
            ex = set(show)
            tp = top_of(tab, k, f, 6, exclude=ex)
            print("        {:<6} {}".format(f, "  ".join(
                "c={}({:.2f})".format(r["c"], r[f]) for r in tp)))

    # ── 阈值扫描：平台宽度 + 硬门 ──
    # 查全/查准只对**有真边界的案例**（本批只有 vesna）有意义；老三案例只进硬门。
    tv = truth.get("vesna")
    if tv is None:
        print()
        print("（--cases 未含 vesna ⇒ 无真边界案例，跳过阈值扫描/平台/合成包线）")
        return 0
    print()
    print("=" * 112)
    print("阶段 1b-2：阈值扫描（查全为主）。列 = 阈值 / 硬门 / 主边界查全 / 回退集查全 / "
          "误标簇 / vesna 检测点")
    for f, thrs in (("gap", [0, 5, 10, 20, 30, 40, 50, 55]),
                    ("P1max", [0.2, 0.4, 0.6, 0.8, 1.0]),
                    ("P1med", [0.2, 0.4, 0.6, 0.8, 1.0]),
                    ("P2", [1, 2, 3, 4, 5]),
                    ("P3", [0.70, 0.75, 0.80, 0.85, 0.88, 0.90]),
                    ("P5", [0.0, 0.2, 0.4, 0.6, 0.8]),
                    ("S6", [1, 2, 3, 4, 5])):
        print("  ── {} ──".format(f))
        for s in scan_1d(tab, keys, f, thrs, tv):
            det = s["det"]["vesna"]
            print("     thr={:<6} 硬门 {:<3} 主 {}/{} 回退 {}/{} 误标簇 {}  检测点 {}".format(
                s["thr"], "✓" if s["gate"] else "✗",
                s["evb"]["hit"], 1, s["ev"]["hit"], len(tv["cuts_retreat"]),
                s["ev"]["n_fp_cluster"], det))

    # ── 合取：P5/P1 ∧ T / P3 ──
    print()
    print("  ── 合取（两个信号都过阈值）──")
    combos = [("P5", 0.4, "gap", 5.0), ("P5", 0.4, "P3", 0.85), ("P5", 0.6, "gap", 5.0),
              ("P5", 0.4, "gap", 10.0), ("P1max", 0.6, "gap", 5.0),
              ("P5", 0.8, "P2", 2.0), ("S6", 3.0, "gap", 5.0), ("S6", 4.0, "gap", 5.0)]
    for f1, t1, f2, t2 in combos:
        det = {k: [r["c"] for r in tab[k]
                   if np.isfinite(r[f1]) and np.isfinite(r[f2])
                   and r[f1] >= t1 and r[f2] >= t2] for k in keys}
        gate = all(not det[k] for k in OLD if k in keys)
        ev = evaluate(det["vesna"], tv["cuts_retreat"], tab["_rows"]["vesna"])
        evb = evaluate(det["vesna"], tv["cuts_big"], tab["_rows"]["vesna"])
        evj = evaluate(det["vesna"], tv["cuts_jump"], tab["_rows"]["vesna"])
        print("     {}≥{} ∧ {}≥{}  硬门 {:<3} 主 {}/1 回退 {}/{} 真值轨迹 {}/{} "
              "误标簇 {}  检测 {}".format(
                  f1, t1, f2, t2, "✓" if gate else "✗", evb["hit"], ev["hit"],
                  len(tv["cuts_retreat"]), evj["hit"],
                  len(tv["cuts_jump"]), ev["n_fp_cluster"], det["vesna"]))
        for k in OLD:
            if det[k]:
                print("        ✗ 硬门失败 {}：{}".format(k, [
                    (r["c"], round(r[f1], 3), round(r[f2], 3))
                    for r in tab[k] if r["c"] in det[k]]))

    # ── 参数平台：(W, τ) 网格上的「硬门 ∧ 主边界命中」 ──
    print()
    print("=" * 112)
    print("阶段 1b-3：参数平台 —— 孪生族在 (W, τ) 网格上的「硬门 ∧ 主边界命中 ∧ 回退集命中」")
    print("     判据：P5 ≥ 0.4 ∧ gap ≥ 5s（gap 门只用来滤掉零间隙的重复行，不是边界主判据）")
    print("     {:>3} {:>6} {:>8} {:>10} {:>12} {:>12}  {}".format(
        "W", "τ", "硬门", "主边界", "回退集", "误标簇", "vesna 检测点"))
    grid_ok = []
    for Wg in (3, 5, 8, 10):
        for tg in (0.85, 0.90, 0.95, 0.98):
            t2 = {}
            for k in keys:
                t2[k], _, _, _, _ = build_signals(k, data[k], tok, sess, Wg, tg, lmin, mc)
            t2["_rows"] = tab["_rows"]
            det = {k: [r["c"] for r in t2[k]
                       if np.isfinite(r["P5"]) and np.isfinite(r["gap"])
                       and r["P5"] >= 0.4 and r["gap"] >= 5.0] for k in keys}
            gate = all(not det[k] for k in OLD if k in keys)
            ev = evaluate(det["vesna"], tv["cuts_retreat"], tab["_rows"]["vesna"])
            evb = evaluate(det["vesna"], tv["cuts_big"], tab["_rows"]["vesna"])
            ok = gate and evb["recall"] == 1.0
            if ok:
                grid_ok.append((Wg, tg, ev["hit"]))
            print("     {:>3} {:>6.2f} {:>8} {:>10} {:>12} {:>12}  {}".format(
                Wg, tg, "✓" if gate else "✗", "{}/1".format(evb["hit"]),
                "{}/{}".format(ev["hit"], len(tv["cuts_retreat"])),
                ev["n_fp_cluster"], det["vesna"]))
    if grid_ok:
        print("     → 可行网格 {} 个：{}".format(
            len(grid_ok), ["W={} τ={:.2f} 回退{}/4".format(*x) for x in grid_ok]))
    else:
        print("     → 可行网格 **空**")

    # ── 主档检测器的检测点与误标分布 ──
    print()
    print("=" * 112)
    print("阶段 1b-4：主档（P5≥0.4 ∧ gap≥5s，W=5 τ=0.90）的检测点与误标分布")
    det = {k: [r["c"] for r in tab[k]
               if np.isfinite(r["P5"]) and np.isfinite(r["gap"])
               and r["P5"] >= 0.4 and r["gap"] >= 5.0] for k in keys}
    for k in keys:
        rows = tab["_rows"][k]
        T = truth[k]
        ev = evaluate(det[k], T["cuts_retreat"], rows)
        print("  ── {}：检测 {} 个；对回退集 {}/{}，误标簇 {}，误标到最近真边界的时间差 {} ──".format(
            k, len(det[k]), ev["hit"], len(T["cuts_retreat"]), ev["n_fp_cluster"],
            ["%.1fs" % x for x in ev["fp_dist"]]))
        for c in det[k]:
            r = next(x for x in tab[k] if x["c"] == c)
            tag = "真边界" if c in T["cuts_retreat"] else (
                "真值轨迹" if c in T["cuts_jump"] else "**误标**")
            print("     c={:<4} t={:>8.2f} gap={:>6.2f} P5={:>5.2f} P1max={:>5.2f} S6={:>3.0f} "
                  "P3={:.3f} 噪声段占比={:.2f}  {}  {!r}".format(
                      c, r["t"], r["gap"], r["P5"], r["P1max"], r["S6"], r["P3"], r["noise"], tag,
                      rows[c]["text"][:46].replace("\n", " / ")))
        for g, dd in zip(ev["fp"], ev["fp_dist"]):
            print("     误标簇 {} → t={}（距最近真边界 {:.1f}s）".format(
                g, ["%.1f" % rows[c]["start"] for c in g], dd))

    # ── 合成重播：召回包线 ──
    if args.synthetic:
        report_synth(data, keys, tok, sess, W, tau, lmin, mc, truth=truth)
    if args.dump:
        ser = {k: tab[k] for k in keys}
        with io.open(args.dump + ".json", "w", encoding="utf-8", newline="") as f:
            json.dump({"W": W, "tau": tau, "lmin": lmin, "tables": ser},
                      f, ensure_ascii=False, indent=1, default=float)
    return 0


if __name__ == "__main__":
    sys.exit(main())
