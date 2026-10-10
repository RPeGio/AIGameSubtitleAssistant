#!/usr/bin/env python3
"""T2 阶段 C：「该不配」判据标定。

背景：`truth_ok == []` 的段（转写里多出来的英文语气词等）**在语料里根本没有对应行**，
正确行为是输出转写原文（DP 走 D 不配分支），而不是硬塞一条语料行。
`score()` 早已按此口径评分，但 DP 没有任何机制知道"没得配"——它只会选相似度最大的那条。

**已实测否定的路线**（不要再走）：原始 max-S 阈值；行中心化；列中心化；双向中心化；
裕度(max−2nd)；限定"只看短段"后的 max-S（空集 ≈ 非空集，零区分力）。
⇒ **嵌入相似度矩阵 S 不携带"有没有对应"这个信息。**

本脚本只在**当前真值**上重建特征表并对照候选判据（旧数字已随真值修订过期：
语料剔除 `M`/`X` 单字符垃圾行、参考文本补 `---` 分隔行）。
仅做嵌入 + DP，不需要 GPU。

用法：
    python scripts/fuse_unmatch_calib.py            # 特征表 + 判据对照 + 不可达性
    python scripts/fuse_unmatch_calib.py --e2e      # 追加端到端 before/after
    python scripts/fuse_unmatch_calib.py --save F   # 顺带把特征表落盘

运行环境（与其它 bench 脚本一致）：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    & runtime\\python\\python.exe scripts\\fuse_unmatch_calib.py
"""
import argparse
import io
import json
import os
import sys
from collections import defaultdict

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "..", "src-tauri", "tests"))

import numpy as np  # noqa: E402
from fuse_lab import load_embedder  # noqa: E402
import fuse_calib as FC  # noqa: E402

KEYS = ["moon", "glupov", "pierro", "vesna"]
NEG = FC.NEG
TOPK_MAX = 10  # 特征表里保留的候选数（top-K 判据扫描用；见 §7.9）


# ────────────────────────── 特征提取 ──────────────────────────

def len_sub(text):
    """**实质字符数**：剥表头 + 剥标点与空白后的字符数（= 嵌入真正"看到"的内容量）。"""
    return len(FC.body_norm(text))


def build_features(data):
    """对每个段算特征；返回 {key: [feat, ...]}（按 idxs 顺序，含不计分的 artifact 段）。

      len_sub       段实质字符数                       ← 唯一可用的"有没有内容"信号
      maxS          允许列上的最大相似度（已含吸引子防护的 NEG 掩码）
      argmax_j      最大相似度对应的语料下标（0-based；-1 = 全列被掩）
      margin        maxS − 次大
      best_len_sub  最佳匹配语料行的实质字符数
      topk_j        **按相似度降序**的合法候选语料下标（长度 TOPK_MAX）  ← §7.9
      topk_len_sub  与 topk_j 对齐的候选行实质字符数                      ← §7.9
      topk_S        与 topk_j 对齐的候选相似度                            ← §7.9
      maxS_short[L] 仅短语料行（len_sub(corpus_j) <= L）上的最大相似度（None = 无短语料行）
      empty         truth_ok == []（该不配）
    """
    out = {}
    for key in KEYS:
        d = data[key]
        S, corpus = d["S"], d["corpus"]
        clen = np.array([len_sub(t) for t in corpus])
        feats = []
        for i in d["idxs"]:
            row = S[i]
            valid = row > NEG / 2
            if valid.any():
                vr = row[valid]
                order = np.argsort(vr)[::-1]
                j = int(np.nonzero(valid)[0][int(order[0])])
                mx = float(vr[order[0]])
                second = float(vr[order[1]]) if len(vr) > 1 else mx
                margin = mx - second
                vcols = np.nonzero(valid)[0][order][:TOPK_MAX]
            else:
                j, mx, margin = -1, float("nan"), float("nan")
                vcols = np.array([], dtype=int)
            short = {}
            for L in (3, 4, 5):
                cols = [c for c in np.nonzero(clen <= L)[0] if row[c] > NEG / 2]
                short[L] = max(float(row[c]) for c in cols) if cols else None
            feats.append({
                "i": i,
                "text": d["seg_texts"][i],
                "start": data[key]["starts"][i],
                "len_sub": len_sub(d["seg_texts"][i]),
                "maxS": mx,
                "argmax_j": j,
                "margin": margin,
                "best_len_sub": int(clen[j]) if j >= 0 else None,
                "topk_j": [int(x) for x in vcols],
                "topk_len_sub": [int(clen[x]) for x in vcols],
                "topk_S": [float(S[i, x]) for x in vcols],
                "maxS_short": short,
                "empty": not d["truth_ok"][i],
                "truth_ok": d["truth_ok"][i],
                "scored": i in set(d["scored"]),
            })
        out[key] = feats
    return out


def load_all(keys):
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = FC.build_matrices(keys, tok, sess)
    for k in keys:
        rows = FC.load_truth(k)["rows"]
        data[k]["starts"] = {i: rows[i]["start"] for i in data[k]["idxs"]}
    return data


# ────────────────────────── 判据 ──────────────────────────
# 全部**语言无关**：只比较实质字符数与相似度，绝不检查字符集（用户否决"CJK"那类规则）。
# 统一施加一道**门**：`len_sub >= 1`（段有实质内容）。
#   理由：`len_sub == 0` 的段没有可判别内容，它配到同样无内容的语料行是**正确**的
#   （pierro `The Jester / …` ↔ 语料 `「丑角」/···`，正是 mask_empty_body 保护的合法用例）。
#   不加这道门则 R1(K≥1) 立刻多 1 个假阳性。

def pred_r1(f, K):
    return f["len_sub"] <= K


def pred_r2(f, K, T):
    return f["len_sub"] <= K and (not np.isfinite(f["maxS"]) or f["maxS"] < T)


def pred_r3(f, K, L):
    return (f["len_sub"] <= K and f["best_len_sub"] is not None
            and f["best_len_sub"] > L)


def pred_r4(f, K, T, L):
    s = f["maxS_short"][L]
    return f["len_sub"] <= K and (s is None or s < T)


def pred_r5(f, K, M):
    return f["len_sub"] <= K and (not np.isfinite(f["margin"]) or f["margin"] < M)


def pred_r6(f, alpha):
    b = f["best_len_sub"]
    return b is not None and b > 0 and f["len_sub"] < alpha * b


def pred_r7(f, alpha):
    """**推荐判据**：段不比它最佳匹配的那一行更有内容。"""
    b = f["best_len_sub"]
    return b is not None and f["len_sub"] <= alpha * b


def pred_r8(f, K, alpha):
    """**§7.9 修订判据**：段不比它 **top-K 候选里最长**的那一行更有内容。

    与 R7 的唯一区别是把 `best_len_sub`（argmax 那一行）换成 `max(topk_len_sub[:K])`。
    动机：argmax 对相似度微差极不稳定（vesna `Huh?` 的 top1/top2 只差 0.0003，而两行
    长度 2 vs 7），只看第一名会把"候选里明明有长行"的段漏掉。

    内置 `len_sub >= 1` 那道门（与落地实现 `fuse_calib.mask_should_unmatched` 同构）：
    `len_sub == 0` 的段没有可判别内容，配到同样无内容的语料行是**正确**的。
    """
    ls = f["len_sub"]
    if ls < 1:
        return False
    tl = f["topk_len_sub"][:K]
    return bool(tl) and ls <= alpha * max(tl)


def pred_r9(f, K, eps, alpha):
    """**§7.9 落地判据**：top-K 候选里**与 argmax 近似并列**（`S >= maxS - eps`）的那几条，
    取其中**最长**的行与段比较。

    为什么必须加"并列"这道限制（实测否决了字面 top-K max，见 `pred_r8`）：语料里长行很多，
    而任意段与长行的相似度差距常在 0.01~0.05 ⇒ 按名次取 K 个必然把长行纳进来，
    `max` 立刻退化成"语料的典型长行长度"，判据失真为纯长度规则（实测 moon `Aria.`
    len=4、名次 2 的行 len=4（S 差 0.023）⇒ `4 <= 1.05·4` 误伤）。而**近似并列**才是
    "argmax 不稳"的真正来源：`Huh?` 的 top1/top2 只差 0.0003。故只在这一小簇里取最长行，
    判据对"谁当第一名"免疫，又不让远处的长行把门槛抬起来。
    """
    ls = f["len_sub"]
    if ls < 1:
        return False
    sl = f["topk_len_sub"][:K]
    sv = f["topk_S"][:K]
    if not sl:
        return False
    tied = [l for l, s in zip(sl, sv) if s >= sv[0] - eps]
    return ls <= alpha * max(tied)


GATED = lambda f: f["len_sub"] >= 1  # noqa: E731


def evaluate(feats, pred, gate=True, keys=KEYS):
    """→ (on_gate, 命中数, 假阳性数, 各案命中, 各案假阳性)"""
    hit = {k: 0 for k in keys}
    fp = {k: 0 for k in keys}
    tot = {k: 0 for k in keys}
    for k in keys:
        for f in feats[k]:
            if not f["scored"]:
                continue
            if f["empty"]:
                tot[k] += 1
            if gate and not GATED(f):
                continue
            if not pred(f):
                continue
            if f["empty"]:
                hit[k] += 1
            else:
                fp[k] += 1
    return hit["vesna"], sum(fp.values()), sum(hit.values()), hit, fp, tot


# ────────────────────────── 报告 ──────────────────────────

def w(s=""):
    sys.stdout.write(s + "\n")


def fmt(x, nd=3):
    if x is None or (isinstance(x, float) and not np.isfinite(x)):
        return " n/a"
    return ("{:." + str(nd) + "f}").format(x)


def section_features(feats, data):
    w("=" * 116)
    w("【1】特征表（当前真值；真值已修订：语料剔除 M/X 单字符垃圾行、参考文本补 --- 分隔行）")
    w("=" * 116)
    w("")
    w("── 1a. vesna 全部空集计分段（truth_ok == []）逐条 ──")
    w("   {:>7} {:>4} {:>6} {:>7}  {:>6} {:<24} {:>5}  {:>17}  {}".format(
        "t(s)", "len", "maxS", "margin", "argmax", "最佳语料行", "行len",
        "maxS_short 3/4/5", "文本"))
    vs = sorted([f for f in feats["vesna"] if f["empty"] and f["scored"]],
                key=lambda f: f["start"])
    for f in vs:
        j = f["argmax_j"]
        ctext = data["vesna"]["corpus"][j].replace("\n", "\\n")[:22] if j >= 0 else "-"
        ss = "/".join(fmt(f["maxS_short"][L], 3) for L in (3, 4, 5))
        w("   {:>7.2f} {:>4} {} {:>7}  {:>6} {:<24} {:>5}  {:>17}  {}".format(
            f["start"], f["len_sub"], fmt(f["maxS"]), fmt(f["margin"]),
            str(f["argmax_j"] + 1 if j >= 0 else 0), ctext, str(f["best_len_sub"]), ss,
            f["text"].replace("\n", " / ")))
    w("   空集计分段合计 {} 条（另有不计分 artifact 空集段 {} 条，未列）".format(
        len(vs), len([f for f in feats["vesna"] if f["empty"] and not f["scored"]])))

    w("")
    w("── 1b. 各案例 len_sub 最小的 10 个**非空集**段（必须不被误判的对照）──")
    for k in KEYS:
        fs = sorted([f for f in feats[k] if f["scored"] and not f["empty"]],
                    key=lambda f: (f["len_sub"], f["start"]))[:10]
        w("   [{}]".format(k))
        for f in fs:
            j = f["argmax_j"]
            ctext = data[k]["corpus"][j].replace("\n", "\\n")[:22] if j >= 0 else "-"
            ss = "/".join(fmt(f["maxS_short"][L], 3) for L in (3, 4, 5))
            w("      {:>7.2f} len={:>3} maxS={} mgn={} argmax={:<4} 行len={:>3} "
              "short={}  {}".format(
                  f["start"], f["len_sub"], fmt(f["maxS"]), fmt(f["margin"]),
                  str(f["argmax_j"] + 1 if j >= 0 else 0), str(f["best_len_sub"]), ss,
                  f["text"].replace("\n", " / ")[:44]))
    w("")
    emp = [f for k in KEYS for f in feats[k] if f["scored"] and f["empty"]]
    nok = [f for k in KEYS for f in feats[k] if f["scored"] and not f["empty"]]
    w("   要点：空集段 len_sub ∈ [{}, {}]、maxS ∈ [{:.3f}, {:.3f}]；非空集段 maxS 上限 {:.3f}".format(
        min(f["len_sub"] for f in emp), max(f["len_sub"] for f in emp),
        min(f["maxS"] for f in emp), max(f["maxS"] for f in emp),
        max(f["maxS"] for f in nok)))
    w("         ⇒ 相似度**整体重叠**，没有任何 max-S 阈值可用（与旧结论一致）。")
    w("")


def section_criteria(feats):
    nempty = sum(1 for f in feats["vesna"] if f["empty"] and f["scored"])
    w("=" * 116)
    w("【2】判据对照（命中 = vesna 空集段被判「不配」/{}；假阳性 = 非空集段被误判，硬指标）".format(nempty))
    w("=" * 116)
    w("")
    w("   注：全部判据统一加门 `len_sub >= 1`；不加门时 R1(K≥1) 因 pierro `The Jester/…`")
    w("       （len_sub=0，合法匹配无内容语料行）立刻多 1 个假阳性。")
    w("   注：本节为 §7.6 的**旧判据**（argmax 那一行）在当前真值下的复测；§7.9 的修订判据见【2b】。")
    w("")
    w("   {:<4} {:<26} {:>9} {:>10}  {}".format("代号", "参数", "vesna命中", "四案假阳性", "各案假阳性"))
    rows = []

    def run(code, desc, pred, gate=True):
        hv, tfp, th, hit, fp, tot = evaluate(feats, pred, gate)
        rows.append((code, desc, hv, tfp, hit, fp))
        w("   {:<4} {:<26} {:>6}/{} {:>10}  {}".format(
            code, desc, hv, nempty, tfp,
            {k: v for k, v in fp.items() if v} or "—"))
        return hv, tfp

    w("   ── R1：len_sub <= K ──")
    for K in range(0, 9):
        run("R1", "K={}".format(K), lambda f, K=K: pred_r1(f, K))
    w("")
    w("   ── R2：len_sub <= K 且 maxS < T ──")
    for K in (2, 3, 5):
        for T in (0.82, 0.84, 0.86, 0.90):
            run("R2", "K={},T={}".format(K, T), lambda f, K=K, T=T: pred_r2(f, K, T))
    w("")
    w("   ── R3：len_sub <= K 且 best_len_sub > L（短显示却匹配到长行）──")
    for K in (3, 5, 6):
        for L in (2, 4, 6):
            run("R3", "K={},L={}".format(K, L), lambda f, K=K, L=L: pred_r3(f, K, L))
    w("")
    w("   ── R4：len_sub <= K 且 maxS_short < T（没有可用的短对应行）──")
    for K in (3, 5):
        for L in (3, 5):
            for T in (0.80, 0.82, 0.84):
                run("R4", "K={},L={},T={}".format(K, L, T),
                    lambda f, K=K, L=L, T=T: pred_r4(f, K, T, L))
    w("")
    w("   ── R5：len_sub <= K 且 margin < M ──")
    for K in (2, 3, 5):
        for M in (0.005, 0.01, 0.02, 0.05):
            run("R5", "K={},M={}".format(K, M), lambda f, K=K, M=M: pred_r5(f, K, M))
    w("")
    w("   ── R6：len_sub < alpha · best_len_sub（严格小于）──")
    for a in (0.5, 0.8, 0.9, 1.0, 1.1):
        run("R6", "alpha={}".format(a), lambda f, a=a: pred_r6(f, a))
    w("")
    w("   ── R7（§7.6 旧判据）：len_sub <= alpha · best_len_sub（不比**最佳那一行**更有内容）──")
    for a in (0.9, 1.0, 1.05, 1.10, 1.15, 1.2, 1.5, 2.0):
        run("R7", "alpha={}".format(a), lambda f, a=a: pred_r7(f, a))

    w("")
    w("   ── 汇总：各判据在「假阳性 = 0」下的最高命中与平台 ──")
    w("   {:<4} {:>9}  {:<8}  {}".format("代号", "最高命中", "FP=0 组合", "并列最优的参数"))
    bycode = defaultdict(list)
    for code, desc, hv, tfp, hit, fp in rows:
        bycode[code].append((desc, hv, tfp))
    for code in ("R1", "R2", "R3", "R4", "R5", "R6", "R7"):
        good = [(d, h) for d, h, t in bycode[code] if t == 0]
        if not good:
            w("   {:<4} {:>9}  {:<8}  {}".format(code, "—", "0", "无 FP=0 组合"))
            continue
        mh = max(h for _, h in good)
        tops = [d for d, h in good if h == mh]
        w("   {:<4} {:>6}/{}  {:<8}  {}（{} 个）".format(
            code, mh, nempty, len(good), ", ".join(tops[:6]), len(tops)))
    w("")
    w("   ── R7 平台细扫（alpha 形式）──")
    w("      {:>6}  {:>8} {:>7}".format("alpha", "命中", "假阳性"))
    r7_ok = []
    for a in (0.90, 0.95, 1.00, 1.05, 1.10, 1.12, 1.14, 1.15, 1.20):
        hv, tfp, _, _, _, _ = evaluate(feats, lambda f, a=a: pred_r7(f, a))
        if tfp == 0:
            r7_ok.append((a, hv))
        w("      {:>6}  {:>8} {:>7}  {}".format(
            a, "{}/{}".format(hv, nempty), tfp, "★ 安全平台" if tfp == 0 else ""))
    w("   ── R7 的等价「加性 slack」形式：len_sub <= best_len_sub + s ──")
    w("      {:>6}  {:>8} {:>7}".format("slack", "命中", "假阳性"))
    for s in (-2, -1, 0, 1, 2):
        hv, tfp, _, _, _, _ = evaluate(
            feats, lambda f, s=s: f["best_len_sub"] is not None
            and f["len_sub"] <= f["best_len_sub"] + s)
        w("      {:>+6}  {:>8} {:>7}  {}".format(
            s, "{}/{}".format(hv, nempty), tfp, "★" if tfp == 0 else ""))
    w("")
    r7_best = max((h for _, h in r7_ok), default=0)
    r7_plat = [a for a, h in r7_ok if h == r7_best]
    w("   结论：R7（argmax 旧判据）在 FP=0 下最高命中 {}/{}，平台 α∈[{:.2f}, {:.2f}]；".format(
        r7_best, nempty, min(r7_plat), max(r7_plat)))
    w("         α=1.15 起出现假阳性（加性 slack 放到 +1 立刻出假阳性 ⇒ 加性形式没有宽平台，故取比例形式）。")
    w("         **但它的可达上限只有 {}/{}**：argmax 对相似度微差极不稳定 ⇒ §7.9 改判据。".format(
        r7_best, nempty))
    w("")


# top-K 判据扫描的网格（§7.9）：α 在旧平台 [1.00, 1.14] 附近加密，并向低端延伸
# （并列带判据的命中平台下沿实测在 0.75 附近，故 α 网格必须覆盖 <0.8）
ALPHAS = [0.40, 0.50, 0.60, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95,
          1.00, 1.05, 1.10, 1.14, 1.15, 1.20, 1.30]
KS = [1, 2, 3, 5, 10]
EPS_GRID = [0.0002, 0.0004, 0.0006, 0.0008, 0.0010, 0.0012, 0.0015, 0.0018,
            0.0020, 0.0021, 0.0025, 0.0030, 0.0040, 0.0050, 0.0100, 0.0200]


def _plateau(alphas, cells):
    """→ (平台内最高命中, FP=0 且达最高命中的 α 列表)"""
    good = [a for a in alphas if cells[a][1] == 0]
    best = max((cells[a][0] for a in good), default=0)
    return best, [a for a in good if cells[a][0] == best]


def _first_bad(alphas, cells, nempty):
    bad = [a for a in alphas if cells[a][1] > 0]
    if not bad:
        return "（网格内无假阳性）"
    a0 = min(bad)
    return "α={:.3f} → {}/{} FP {}".format(a0, cells[a0][0], nempty, cells[a0][1])


def _print_grid(label, alphas, cells_by_row, rowfmt="{:>6}"):
    w("   {:<8} {}".format(label, "  ".join(rowfmt.format(a) for a in alphas)))
    for key in cells_by_row:
        cells = cells_by_row[key]
        w("   {:<8} {}".format(
            key, "  ".join("{:>6}".format("{}!{}".format(cells[a][0], cells[a][1])
                                           if cells[a][1] else str(cells[a][0]))
                           for a in alphas)))


def section_criteria_topk(feats, data):
    """§7.9：判据从 argmax 改为 top-K 候选里的**最长行** —— (K, α) 与 (ε, α) 可行域。"""
    nempty = sum(1 for f in feats["vesna"] if f["empty"] and f["scored"])
    rk, ra, re = FC.UNMATCH_TOPK, FC.UNMATCH_ALPHA, FC.UNMATCH_TIE_EPS
    w("=" * 116)
    w("【2b】§7.9 修订判据：(K, α) / (ε, α) 可行域（命中 = vesna 空集段被判「不配」/{}；"
      "`!` 后 = 四案例假阳性数）".format(nempty))
    w("=" * 116)
    w("")
    w("   R8（**字面** top-K）：`len_sub >= 1 且 len_sub <= alpha * max(clen[j] for j in top-K(S[i]))`")
    w("   R9（**并列带**，落地）：把 R8 的 top-K 换成 `top-K ∩ {{j : S[i,j] >= maxS_i - eps}}`")
    w("   `K=1` 即 §7.6 旧判据（只看 argmax 那一行）。硬门：四案例非空集段假阳性必须为 0。")
    w("")
    w("   ── (a) R8 字面 top-K max：K × α 网格 ──")
    g8 = {}
    for K in KS:
        g8[K] = {a: evaluate(feats, lambda f, K=K, a=a: pred_r8(f, K, a), gate=False)
                 for a in ALPHAS}
    _print_grid("K", ALPHAS, {"K={}".format(K): g8[K] for K in KS})
    w("")
    w("   ⇒ **R8 不可用**：K≥2 起，任何 α 都有假阳性（K=3, α=1.05 → 命中 12 但 FP **20**）。")
    w("      机制：语料里长行很多，而任意段与长行的相似度差距常在 0.01~0.05 ⇒ 按**名次**取 K 个")
    w("      必然把远处的长行纳进来，`max` 退化成「语料的典型长行长度」，判据失真为纯长度规则。")
    w("      两个实例：moon `Aria.`(len 4) 名次 2 的行 len 4（S 差 0.023）⇒ `4 <= 1.05·4` 误伤；")
    w("      vesna `Ohh!`(len 3) 名次 3 的行 len 6（S 差 0.0021）⇒ `3 <= 1.05·6` 误伤。")
    w("")
    w("   ── (b) R9 并列带（eps={}）：K × α 网格 ──".format(re))
    g9 = {}
    for K in KS:
        g9[K] = {a: evaluate(feats, lambda f, K=K, e=re, a=a: pred_r9(f, K, e, a), gate=False)
                 for a in ALPHAS}
    _print_grid("K", ALPHAS, {"K={}".format(K): g9[K] for K in KS})
    w("      ⇒ 加并列带后 K 不再是瓶颈：K∈[2,10] 结果逐格相同（并列簇本身很小），")
    w("         K=1 退化为旧判据（看不到并列行，故漏掉 3 段）。")
    w("")
    w("   ── (c) R9 的 (eps, α) 网格（K={}）——真正的可行域 ──".format(rk))
    gc = {}
    for e in EPS_GRID:
        gc[e] = {a: evaluate(feats, lambda f, e=e, a=a: pred_r9(f, rk, e, a), gate=False)
                 for a in ALPHAS}
    _print_grid("eps", ALPHAS, {"{:.4f}".format(e): gc[e] for e in EPS_GRID})
    w("")
    w("   ── (d) 每个 eps 的 FP=0 平台（α 区间）与命中（K={}）──".format(rk))
    w("   {:>8} {:>9} {:<18} {:>9}  {}".format(
        "eps", "最高命中", "FP=0 平台 α", "平台命中", "首次出 FP 的 α"))
    for e in EPS_GRID:
        best, plat = _plateau(ALPHAS, gc[e])
        band = "[{:.3f}, {:.3f}]".format(min(plat), max(plat)) if plat else "—"
        w("   {:>8.4f} {:>6}/{} {:<18} {:>6}/{}  {}".format(
            e, best, nempty, band, best, nempty, _first_bad(ALPHAS, gc[e], nempty)))
    w("")
    w("   ── (e) 落地常量校验（K={}, eps={}, alpha={}）──".format(rk, re, ra))
    hv, tfp, _, hit, fp, tot = evaluate(
        feats, lambda f: pred_r9(f, rk, re, ra), gate=False)
    w("      命中 {}/{}（空集段）  四案例假阳性 {}  各案假阳性 {}  {}".format(
        hv, nempty, tfp, {k: v for k, v in fp.items() if v} or "—",
        "★ 硬门通过" if tfp == 0 else "✗ 硬门失败"))
    w("      K=1（旧判据）同 α 对照：命中 {}/{}、假阳性 {}".format(
        evaluate(feats, lambda f: pred_r8(f, 1, ra), gate=False)[0], nempty,
        evaluate(feats, lambda f: pred_r8(f, 1, ra), gate=False)[1]))
    w("")
    w("   ── (f) 空集段逐条：argmax 行 vs 并列簇最长行（K={}, eps={}, α={}）──".format(rk, re, ra))
    w("   {:>8} {:>4} {:>6} {:>9} {:>6} {:>7}  {}".format(
        "t(s)", "len", "top1行", "并列最长", "K=1判", "落地判", "文本"))
    for f in sorted([x for x in feats["vesna"] if x["empty"] and x["scored"]],
                    key=lambda x: x["start"]):
        sl, sv = f["topk_len_sub"][:rk], f["topk_S"][:rk]
        tied = [l for l, s in zip(sl, sv) if s >= sv[0] - re] if sl else []
        w("   {:>8.2f} {:>4} {:>6} {:>9} {:>6} {:>7}  {}".format(
            f["start"], f["len_sub"], sl[0] if sl else 0, max(tied) if tied else 0,
            "命中" if pred_r8(f, 1, ra) else "漏",
            "命中" if pred_r9(f, rk, re, ra) else "漏",
            f["text"].replace("\n", " / ")))
    w("")
    w("   ── 各案例最短非空集段（硬门对照：判据必须不碰它们）──")
    for k in KEYS:
        fs = sorted([x for x in feats[k] if x["scored"] and not x["empty"]],
                    key=lambda x: (x["len_sub"], x["start"]))[:3]
        for f in fs:
            sl, sv = f["topk_len_sub"][:rk], f["topk_S"][:rk]
            tied = [l for l, s in zip(sl, sv) if s >= sv[0] - re] if sl else []
            w("      {:<8} t={:>8.2f} len={:>3} 并列最长={:>3} 比值 len/最长={:>5}  {} {}".format(
                k, f["start"], f["len_sub"], max(tied) if tied else 0,
                "{:.2f}".format(f["len_sub"] / max(tied)) if tied and max(tied) else "inf",
                f["text"].replace("\n", " / ")[:34],
                "← 掩行" if pred_r9(f, rk, re, ra) else ""))
    w("")

    w("   ── (g) 边界约束：α 平台的两端由谁决定（K={}, eps={}）──".format(rk, re))
    ok_rows, bad_rows = [], []
    for k in KEYS:
        for f in feats[k]:
            if not f["scored"]:
                continue
            sl, sv = f["topk_len_sub"][:rk], f["topk_S"][:rk]
            tied = [l for l, s in zip(sl, sv) if s >= sv[0] - re] if sl else []
            mx = max(tied) if tied else 0
            ratio = (f["len_sub"] / mx) if mx else float("inf")
            (bad_rows if f["empty"] else ok_rows).append((ratio, k, f, mx))
    # 命中侧：空集段里比值最大者决定 α 下沿；假阳性侧：非空集段里比值最小者决定 α 上沿
    fp_hi = min(ok_rows, key=lambda x: x[0], default=None)
    a_max = fp_hi[0] if fp_hi else float("inf")
    reach = [r for r in bad_rows if r[0] < a_max]      # 比值 ≥ α 上沿 ⇒ 原理上不可达
    hit_lo = max(reach, key=lambda x: x[0], default=None)
    if hit_lo:
        w("      α 下沿（**可达**的空集段里 len/并列最长 最大者，α 低于它该段就漏）：")
        w("         {:<8} t={:>8.2f} len={:>3} 并列最长={:>3} 比值={:.4f}  {}".format(
            hit_lo[1], hit_lo[2]["start"], hit_lo[2]["len_sub"], hit_lo[3], hit_lo[0],
            hit_lo[2]["text"].replace("\n", " / ")[:34]))
    if fp_hi:
        w("      α 上沿（非空集段里 len/并列最长 最小者，α 一到它就被误伤）：")
        w("         {:<8} t={:>8.2f} len={:>3} 并列最长={:>3} 比值={:.4f}  {}".format(
            fp_hi[1], fp_hi[2]["start"], fp_hi[2]["len_sub"], fp_hi[3], fp_hi[0],
            fp_hi[2]["text"].replace("\n", " / ")[:34]))
    for r in bad_rows:
        if r[0] >= a_max:
            w("      **不可达**（比值 {:.4f} ≥ α 上沿 {:.4f}）：{:<8} t={:>8.2f} len={:>3} "
              "并列最长={:>3}  {}".format(
                  r[0], a_max, r[1], r[2]["start"], r[2]["len_sub"], r[3],
                  r[2]["text"].replace("\n", " / ")[:30]))
    w("      eps 窗口（在 alpha={} 下逐段算「要判对它 / 会误伤它」所需的最小间隔）：".format(ra))
    req_hit, req_fp = [], []
    for k in KEYS:
        for f in feats[k]:
            if not f["scored"] or not f["topk_S"]:
                continue
            ls = f["len_sub"]
            if ls < 1:
                continue
            sl, sv = f["topk_len_sub"][:TOPK_MAX], f["topk_S"][:TOPK_MAX]
            if ls <= ra * sl[0]:
                continue                     # 不看并列行就已经判出结果 ⇒ 与 eps 无关
            gap = None
            for r in range(1, len(sv)):
                if ls <= ra * sl[r]:         # 纳入第 r 个候选就会判「不配」
                    gap = sv[0] - sv[r]
                    break
            if gap is None:
                continue
            (req_hit if f["empty"] else req_fp).append((gap, k, f))
    if req_hit:
        g, k, f = max(req_hit, key=lambda x: x[0])
        w("         eps **下沿** = {:.5f}（空集段里「所需最小间隔」最大者，低于它该段就漏）：".format(g))
        w("            {:<8} t={:>8.2f} len={:>3}  {}".format(
            k, f["start"], f["len_sub"], f["text"].replace("\n", " / ")[:30]))
    if req_fp:
        g, k, f = min(req_fp, key=lambda x: x[0])
        w("         eps **上沿** = {:.5f}（该配段里「触发所需最小间隔」最小者，一到它就误伤）：".format(g))
        w("            {:<8} t={:>8.2f} len={:>3}  {}".format(
            k, f["start"], f["len_sub"], f["text"].replace("\n", " / ")[:30]))
    w("")
    w("   ── 与落地实现的一致性自检（特征代理 vs `fuse_calib.mask_should_unmatched`）──")
    tot_bad = 0
    for k in KEYS:
        d = data[k]
        base = d["S"]
        S = base.copy()
        real = FC.mask_should_unmatched(S, d["seg_texts"], d["corpus"])
        real_rows = set()
        for i in d["idxs"]:
            valid = base[i] > NEG / 2
            if valid.any() and not (S[i][valid] > NEG / 2).any():
                real_rows.add(i)
        proxy = {f["i"] for f in feats[k] if pred_r9(f, rk, re, ra)}
        bad = real_rows ^ proxy
        tot_bad += len(bad)
        w("      {:<8} 实现掩行 {:3} / 特征代理 {:3}  一致 {}".format(
            k, real, len(proxy), "✓" if not bad else "✗ 差异 {}".format(sorted(bad))))
    w("      ⇒ {}".format("两路逐段一致" if tot_bad == 0 else "存在差异，必须排查"))
    w("")


def section_unreachable(feats):
    """同一文本、标签相反 ⇒ S 行逐位相同 ⇒ 任何"行掩码"都不可能区分。"""
    nempty_all = sum(1 for f in feats["vesna"] if f["empty"] and f["scored"])
    w("=" * 116)
    w("【3】不可达性：文本逐字相同的「复现对」在原理上无法被「该不配」判据区分")
    w("=" * 116)
    w("")
    groups = defaultdict(list)
    for k in KEYS:
        for f in feats[k]:
            if f["scored"] and f["len_sub"] >= 1:
                groups[f["text"]].append((k, f))
    w("   「该不配」行掩码的输入只有 (S 的第 i 行, 段文本, 语料)。段文本相同 ⇒ 编码输入相同")
    w("   ⇒ S 行**逐位相同** ⇒ 判据必然给出相同结论。故「同一文本既被判该配、又被判该不配」时，")
    w("   **只要要求假阳性为 0，就只能两处都不判**。")
    w("")
    n = 0
    nempty = 0
    for t, g in groups.items():
        labs = {x[1]["empty"] for x in g}
        if len(labs) < 2:
            continue
        n += 1
        nempty += sum(1 for x in g if x[1]["empty"])
        w("   · {!r} 出现 {} 次：".format(t, len(g)))
        for k, f in g:
            w("       {} t={:>7.2f}  len_sub={:>2} best_len_sub={:>2} maxS={} margin={}  {}".format(
                k, f["start"], f["len_sub"], f["best_len_sub"], fmt(f["maxS"]),
                fmt(f["margin"]), "**该不配(空集)**" if f["empty"] else "该配(真值 ok)"))
    w("")
    w("   共 {} 组，涉及**空集段 {} 个** ⇒ 可达上限 = {} − {} = **{}/{}**。".format(
        n, nempty, nempty_all, nempty, nempty_all - nempty, nempty_all))
    w("   这 {} 段都在 vesna 的「重看 PV / 拖进度条」复现区：同一句显示第二次出现时参考轨".format(nempty))
    w("   取不到中文行（ref_text 为空），真值据此判「空集」；而它的孪生段（首次出现、参考齐全）")
    w("   被判「该配」，且两者特征逐位相同。**这不是判据缺陷，是回退/重播区的口径问题**（§7.5）")
    w("   —— 解在「显式重播分段」，不在掩码。")
    w("")


# ────────────────────────── 端到端 ──────────────────────────

def e2e(data, use_mask, alpha=None, topk=None, tie_eps=None):
    alpha = FC.UNMATCH_ALPHA if alpha is None else alpha
    topk = FC.UNMATCH_TOPK if topk is None else topk
    tie_eps = FC.UNMATCH_TIE_EPS if tie_eps is None else tie_eps
    w("   {:<8} {:>10} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}".format(
        "案例", "score", "合计", "不配D", "回退C", "复用B", "前进A", "空集对"))
    tot = totn = 0
    detail = {}
    for k in KEYS:
        d = data[k]
        S = d["S"].copy()
        nmask = 0
        if use_mask:
            nmask = FC.mask_should_unmatched(S, d["seg_texts"], d["corpus"],
                                             alpha, topk, tie_eps)
        diag = {}
        m = FC.align_v2(S, *FC.DEFAULT, repeat_penalty=FC.REPEAT_DEFAULT,
                        reset_penalty=FC.RESET_DEFAULT, diag=diag)
        c = FC.score(m, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
        pos = {x: n for n, x in enumerate(d["idxs"])}
        empt = [x for x in d["scored"] if not d["truth_ok"][x]]
        ne = sum(1 for x in empt if m[pos[x]] < 0)
        tot += c
        totn += len(d["scored"])
        w("   {:<8} {:>4}/{:<4} {:>7} {:>7} {:>7} {:>7} {:>7} {:>5}/{}".format(
            k, c, len(d["scored"]), "", diag["unmatched"], diag["reset"],
            diag["repeat"], diag["forward"], ne, len(empt)))
        detail[k] = (c, len(d["scored"]), diag, ne, len(empt), nmask, m)
    w("   {:<8} {:>4}/{:<4}            四案例合计 {}/{} = {:.1f}%".format(
        "合计", tot, totn, tot, totn, tot / totn * 100))
    return detail


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--e2e", action="store_true", help="追加端到端 before/after")
    ap.add_argument("--save", default=None, help="把特征表落盘为 JSON")
    args = ap.parse_args()

    data = load_all(KEYS)
    feats = build_features(data)
    if args.save:
        with io.open(args.save, "w", encoding="utf-8", newline="") as f:
            json.dump(feats, f, ensure_ascii=False, indent=1)
    section_features(feats, data)
    section_criteria(feats)
    section_criteria_topk(feats, data)
    section_unreachable(feats)
    if args.e2e:
        w("=" * 116)
        w("【4】端到端 before/after（align_v2 + score；repeat={} reset={} skip={} unmatched={}）".format(
            FC.REPEAT_DEFAULT, FC.RESET_DEFAULT, FC.DEFAULT[0], FC.DEFAULT[1]))
        w("=" * 116)
        w("")
        w("   before：UNMATCH_FILTER=False（现行默认，掩码不生效）")
        b = e2e(data, False)
        w("")
        w("   after-旧判据：topk=1（§7.6 argmax 那一行，alpha={}）".format(FC.UNMATCH_ALPHA))
        o = e2e(data, True, topk=1, tie_eps=0)
        w("")
        w("   after-新判据：K={} eps={} alpha={}（§7.9；FP=0 实测上限见【2b】）".format(
            FC.UNMATCH_TOPK, FC.UNMATCH_TIE_EPS, FC.UNMATCH_ALPHA))
        a = e2e(data, True)
        w("")
        w("   {:>8} {:>12} {:>12} {:>12} {:>8} {:>12}".format(
            "案例", "before", "after(旧)", "after(新)", "新−旧", "掩行 旧/新"))
        for k in KEYS:
            b0, n0 = b[k][0], b[k][1]
            w("   {:>8} {:>7}/{:<4} {:>7}/{:<4} {:>7}/{:<4} {:>+8} {:>7}/{:<4}".format(
                k, b0, n0, o[k][0], o[k][1], a[k][0], a[k][1], a[k][0] - o[k][0],
                o[k][5], a[k][5]))
        tb = sum(b[k][0] for k in KEYS)
        to = sum(o[k][0] for k in KEYS)
        ta = sum(a[k][0] for k in KEYS)
        tn = sum(b[k][1] for k in KEYS)
        w("   {:>8} {:>7}/{:<4} {:>7}/{:<4} {:>7}/{:<4} {:>+8}".format(
            "合计", tb, tn, to, tn, ta, tn, ta - to))
        w("")
        w("   硬门：moon/glupov/pierro 的掩行数均为 0 ⇒ 掩码是**恒等变换**，分数不可能下降。")
        w("   vesna 的「空集·正确不配」：{} → 旧 {} → 新 {}".format(
            "{}/{}".format(b["vesna"][3], b["vesna"][4]),
            "{}/{}".format(o["vesna"][3], o["vesna"][4]),
            "{}/{}".format(a["vesna"][3], a["vesna"][4])))
        w("   vesna 净增 {:+d}（相对掩码关），其中空集段直接贡献 {:+d}，其余 {:+d} 来自掩码后 DP 路径整体改观。".format(
            a["vesna"][0] - b["vesna"][0], a["vesna"][3] - b["vesna"][3],
            (a["vesna"][0] - b["vesna"][0]) - (a["vesna"][3] - b["vesna"][3])))
        w("   after 判对的空集段：")
        d = data["vesna"]
        pos = {x: n for n, x in enumerate(d["idxs"])}
        for x in [y for y in d["scored"] if not d["truth_ok"][y]]:
            if a["vesna"][6][pos[x]] < 0:
                w("      · t={:>7.2f} {}".format(
                    d["starts"][x], d["seg_texts"][x].replace("\n", " / ")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
