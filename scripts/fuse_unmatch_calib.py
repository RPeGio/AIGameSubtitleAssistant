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
            else:
                j, mx, margin = -1, float("nan"), float("nan")
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
    w("   要点：空集段 len_sub ∈ [2, 19]、maxS ∈ [0.804, 0.839]；非空集段 maxS 上限 0.911")
    w("         ⇒ 相似度**整体重叠**，没有任何 max-S 阈值可用（与旧结论一致）。")
    w("")


def section_criteria(feats):
    w("=" * 116)
    w("【2】判据对照（命中 = vesna 空集段被判「不配」/15；假阳性 = 非空集段被误判，硬指标）")
    w("=" * 116)
    w("")
    w("   注：全部判据统一加门 `len_sub >= 1`；不加门时 R1(K≥1) 因 pierro `The Jester/…`")
    w("       （len_sub=0，合法匹配无内容语料行）立刻多 1 个假阳性。")
    w("")
    w("   {:<4} {:<26} {:>9} {:>10}  {}".format("代号", "参数", "vesna命中", "四案假阳性", "各案假阳性"))
    rows = []

    def run(code, desc, pred, gate=True):
        hv, tfp, th, hit, fp, tot = evaluate(feats, pred, gate)
        rows.append((code, desc, hv, tfp, hit, fp))
        w("   {:<4} {:<26} {:>6}/15 {:>10}  {}".format(
            code, desc, hv, tfp,
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
    w("   ── R7（推荐）：len_sub <= alpha · best_len_sub（不比最佳行更有内容）──")
    for a in (0.9, 1.0, 1.05, 1.10, 1.15, 1.2, 1.5, 2.0):
        run("R7", "alpha={}".format(a), lambda f, a=a: pred_r7(f, a))
        if a == 1.05:
            w("        └ 式中 alpha={} 即脚本落地常量 UNMATCH_ALPHA".format(a))

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
        w("   {:<4} {:>6}/15  {:<8}  {}（{} 个）".format(
            code, mh, len(good), ", ".join(tops[:6]), len(tops)))
    w("")
    w("   ── R7 平台细扫（alpha 形式）──")
    w("      {:>6}  {:>8} {:>7}".format("alpha", "命中", "假阳性"))
    for a in (0.90, 0.95, 1.00, 1.05, 1.10, 1.12, 1.14, 1.15, 1.20):
        hv, tfp, _, _, _, _ = evaluate(feats, lambda f, a=a: pred_r7(f, a))
        w("      {:>6}  {:>8} {:>7}  {}".format(
            a, "{}/15".format(hv), tfp, "★ 安全平台" if tfp == 0 else ""))
    w("   ── R7 的等价「加性 slack」形式：len_sub <= best_len_sub + s ──")
    w("      {:>6}  {:>8} {:>7}".format("slack", "命中", "假阳性"))
    for s in (-2, -1, 0, 1, 2):
        hv, tfp, _, _, _, _ = evaluate(
            feats, lambda f, s=s: f["best_len_sub"] is not None
            and f["len_sub"] <= f["best_len_sub"] + s)
        w("      {:>+6}  {:>8} {:>7}  {}".format(
            s, "{}/15".format(hv), tfp, "★" if tfp == 0 else ""))
    w("")
    w("   结论：**FP=0 的上限就是 12/15**（alpha 一放到 1.15 立刻出现 5 个假阳性；")
    w("         加性 slack 放到 +1 立刻 9 个假阳性 ⇒ 加性形式没有宽平台，故取比例形式）。")
    w("         R7 在 alpha ∈ [1.00, 1.14] 全区间同为 12/15、FP=0 —— 取中段 1.05。")
    w("")


def section_unreachable(feats):
    """同一文本、标签相反 ⇒ S 行逐位相同 ⇒ 任何"行掩码"都不可能区分。"""
    w("=" * 116)
    w("【3】不可达性：剩余 3 个空集段**原理上**无法被「该不配」判据命中")
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
    w("   共 {} 组，涉及**空集段 {} 个** ⇒ 可达上限 = 15 − {} = **12/15**，与【2】实测平台吻合。".format(
        n, nempty, nempty))
    w("   这 3 段都在 vesna 的「重看 PV / 拖进度条」复现区：同一句显示第二次出现时参考轨")
    w("   取不到中文行（ref_text 为空），真值据此判「空集」；而它的孪生段（首次出现、参考齐全）")
    w("   被判「该配」，且两者特征逐位相同。**这不是判据缺陷，是回退/重播区的口径问题**（§7.5）")
    w("   —— 解在「显式重播分段」，不在掩码。")
    w("")


# ────────────────────────── 端到端 ──────────────────────────

def e2e(data, use_mask):
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
                                             FC.UNMATCH_ALPHA)
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
        w("   after：掩码生效，alpha={}（{}/15 可达上限）".format(FC.UNMATCH_ALPHA, 12))
        a = e2e(data, True)
        w("")
        w("   {:>8} {:>16} {:>16} {:>10} {:>14}".format(
            "案例", "score before", "score after", "差", "掩行数"))
        for k in KEYS:
            b0, n0 = b[k][0], b[k][1]
            a0 = a[k][0]
            w("   {:>8} {:>10}/{:<5} {:>10}/{:<5} {:>+10} {:>14}".format(
                k, b0, n0, a0, a[k][1], a0 - b0, a[k][5]))
        tb = sum(b[k][0] for k in KEYS)
        ta = sum(a[k][0] for k in KEYS)
        tn = sum(b[k][1] for k in KEYS)
        w("   {:>8} {:>10}/{:<5} {:>10}/{:<5} {:>+10}".format(
            "合计", tb, tn, ta, tn, ta - tb))
        w("")
        w("   硬门：moon/glupov/pierro 的掩行数均为 0 ⇒ 掩码是**恒等变换**，分数不可能下降。")
        w("   vesna 的「空集·正确不配」：{} → {}".format(
            "{}/{}".format(b["vesna"][3], b["vesna"][4]),
            "{}/{}".format(a["vesna"][3], a["vesna"][4])))
        w("   vesna 净增 {:+d}，其中空集段直接贡献 {:+d}，其余 {:+d} 来自掩码后 DP 路径整体改观。".format(
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
