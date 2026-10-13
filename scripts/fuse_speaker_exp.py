# -*- coding: utf-8 -*-
"""【实验，只测量、不动生产代码】F14/R4 前置：flat 全局 DP vs 按说话人分块 DP。

来源缺陷 F14（benchmark/FUSE_PIPELINE_DEFECTS.md）：同台词跨说话人——正文相同 ⇒ 嵌入
几乎相同；且语料行序 ≠ 全局说话序时，全局单调 DP 会把 A 的段错配给 B 的语料行。
R4 草案：把「一个全局 DP」改成「每个说话人一个 DP、全语料共享」；单一（或无）说话人
的案例应**完全退化等价**。本脚本验证两件事：
  1) 真 4 案例（truth 评分）：分块不回归——默认 210/236、掩码开 226/236 不掉；
  2) 合成「同台词跨说话人」案例：分块修复 flat 的错配。

评分口径与 fuse_calib 一致：`score`（等价类 + 集合感知）为主门、`score_exact` 作参考。
运行：
  $env:PYTHONPATH="<repo>\\runtime\\deps_embed"; $env:PYTHONIOENCODING="utf-8"
  runtime\\python\\python.exe scripts\\fuse_speaker_exp.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from fuse_calib import (DEFAULT, REPEAT_DEFAULT, RESET_DEFAULT, UNMATCH_ALPHA,  # noqa: E402
                        UNMATCH_TOPK, UNMATCH_TIE_EPS, align_v2, build_matrices,
                        equiv_classes, mask_empty_body, mask_should_unmatched,
                        score, score_exact)
from fuse_lab import load_embedder  # noqa: E402
import numpy as np  # noqa: E402

CASES = ["moon", "glupov", "pierro", "vesna"]


def align_flat(S):
    """现行行为：全部段一起跑一个全局 DP。"""
    return align_v2(S, *DEFAULT, repeat_penalty=REPEAT_DEFAULT,
                    reset_penalty=RESET_DEFAULT)


def align_by_speaker(S, speakers):
    """分块行为：每个说话人一个 align_v2（全语料共享），结果合并回全局行序。

    speakers 与 S 的行同序（按时间升序）；同一说话人天然保持时间升序。
    """
    groups = {}
    for i, sp in enumerate(speakers):
        groups.setdefault(sp, []).append(i)
    match = [0] * S.shape[0]
    for sp, rows in groups.items():
        rows = sorted(rows)
        sub = S[np.asarray(rows, dtype=int), :]
        m = align_v2(sub, *DEFAULT, repeat_penalty=REPEAT_DEFAULT,
                     reset_penalty=RESET_DEFAULT)
        for pos, r in enumerate(rows):
            match[r] = m[pos]
    return match, groups


def score_pair(m, d):
    ok = score(m, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
    ex = score_exact(m, d["truth"], d["idxs"])
    return ok, ex


def real_cases(tok, sess):
    data = build_matrices(CASES, tok, sess)
    total = {"flat_def": [0, 0], "grp_def": [0, 0],
             "flat_um": [0, 0], "grp_um": [0, 0]}
    tbl = []
    for key in CASES:
        d = data[key]
        n = d["n_raw"]
        speakers = [(r.get("ref_speaker") or "") for r in _rows(key)]
        S = d["S"]  # 已含 mask_empty_body（原地掩码），与现行默认完全一致
        S_um = S.copy()
        nmask = mask_should_unmatched(S_um, d["seg_texts"], d["corpus"],
                                      alpha=UNMATCH_ALPHA, topk=UNMATCH_TOPK,
                                      tie_eps=UNMATCH_TIE_EPS)
        runs = {
            "flat_def": align_flat(S),
            "grp_def": align_by_speaker(S, speakers)[0],
            "flat_um": align_flat(S_um),
            "grp_um": align_by_speaker(S_um, speakers)[0],
        }
        n_speaker_groups = len(set(speakers))
        row = {"key": key, "n": n, "groups": n_speaker_groups, "mask": nmask}
        for mode, m in runs.items():
            ok, ex = score_pair(m, d)
            row[mode] = (ok, ex)
            total[mode][0] += ok
            total[mode][1] += ex
        row["same_def"] = runs["flat_def"] == runs["grp_def"]
        row["same_um"] = runs["flat_um"] == runs["grp_um"]
        # 语料侧「说话人块」可判定性：原始语料行表头（split_header 剥掉的部分）
        # 里能认出多少段侧说话人名；相邻行说话人切换次数 = 块结构强度（分块是否有望）。
        senamed = sorted({s for s in speakers if s})
        from fuse_core import split_header

        def infer_spk(t):
            h = split_header(t)[0] or ""
            for s in senamed:
                if s in h:
                    return s
            return ""
        inferred = [infer_spk(t) for t in d["corpus"]]
        named = sum(1 for x in inferred if x)
        switches = sum(1 for a, b in zip(inferred, inferred[1:])
                       if a and b and a != b)
        row["cohesion"] = "{}  辨名行 {}/{},切换 {} 次".format(
            senamed, named, len(inferred), switches)
        tbl.append(row)
        # 差异明细：分块与 flat 结论不同的段
        diffs = []
        for k in d["scored"]:
            for mask_tag, fm, gm in (("def", runs["flat_def"], runs["grp_def"]),
                                     ("um", runs["flat_um"], runs["grp_um"])):
                if fm[k] != gm[k]:
                    diffs.append((k, mask_tag, d["truth"][k], fm[k] + 1, gm[k] + 1))
        row["diffs"] = diffs
    return tbl, total


def _rows(key):
    import json
    path = os.path.join(_bench_out(), "truth_{}.json".format(key))
    with open(path, encoding="utf-8") as f:
        return json.load(f)["rows"]


def _bench_out():
    # 与 fuse_calib 的 BENCH_OUT 同源（ Scripts 目录上一级 temp/bench_output；
    # 直接复用 fuse_calib 的常量最稳）
    import fuse_calib
    return fuse_calib.BENCH_OUT


def synth_cases(tok, sess):
    """合成「同台词跨说话人」案例（构造式真值，不走 .gsa）。

    s1：语料行带说话人名且**按说话人分组排序**，段按全局时间交错 —— flat 必错、
        分块应全对（F14 的核心受害场景，模拟「整屏 OCR 先读完甲的再读乙的」）。
    s2：语料精确去重后每个台词只剩**一行**（无说话人名）—— 平行行缺失，任何行等价，
        语义上无差别（只验证不劣化、行复用如实）。
    s3：对照组——语料行序与全局说话序一致 —— flat 本就对，分块不得劣化。
    """
    cases = {}

    names_lines = ["工作真是太忙了。", "今晚的月亮真圆。", "一起去人类的村子吧。"]
    # s1：语料分组（卡侬 3 行 → 桑娜妲 3 行），段全局时间交错
    corpus1 = ["卡侬\n" + t for t in names_lines] + ["桑娜妲\n" + t for t in names_lines]
    rows1 = []
    truth1 = [1, 4, 2, 5, 3, 6]
    speakers1 = ["卡侬", "桑娜妲"] * 3
    for i, sp in enumerate(speakers1):
        line = names_lines[i // 2]
        rows1.append((line, truth1[i], [truth1[i]], sp))
    cases["s1_同台词语料分组"] = (corpus1, rows1)

    # s2：去重后仅 3 行；6 段仍然交错
    corpus2 = list(names_lines)
    rows2 = []
    for i, sp in enumerate(speakers1):
        line = names_lines[i // 2]
        rows2.append((line, i // 2 + 1, [i // 2 + 1], sp))
    cases["s2_同台词去重后仅一行"] = (corpus2, rows2)

    # s3：语料行序 = 全局说话序（对照组）
    corpus3 = []
    rows3 = []
    order = [("卡侬", 0), ("桑娜妲", 0), ("卡侬", 1), ("桑娜妲", 1),
             ("卡侬", 2), ("桑娜妲", 2)]
    for gi, (sp, li) in enumerate(order):
        corpus3.append(sp + "\n" + names_lines[li])
        rows3.append((names_lines[li], gi + 1, [gi + 1], sp))
    cases["s3_语料行序与说话序一致"] = (corpus3, rows3)

    # s1b：语料**按说话人分组排序**、各行文本互不相同，段全局时间交错 ——
    # 同台词「替身行」消失后，这是 flat 真正会错配的场景（F14 受害面）。
    can_lines = ["剑术的奥义在于凝神静气。", "月亮的轨迹早已注定。",
                 "回去告诉你们的祭司，卡侬不会退缩。"]
    son_lines = ["工作就丢给月使们呗。", "姐姐你最近总是偷偷跑下界。",
                 "好啦好啦，我这就回去干活。"]
    corpus1b = ["卡侬\n" + t for t in can_lines] + ["桑娜妲\n" + t for t in son_lines]
    rows1b = []
    speakers1b = ["卡侬", "桑娜妲"] * 3
    truth1b = [1, 4, 2, 5, 3, 6]
    for i, sp in enumerate(speakers1b):
        line = (can_lines + son_lines)[i // 2] if False else (
            can_lines[i // 2] if sp == "卡侬" else son_lines[i // 2])
        rows1b.append((line, truth1b[i], [truth1b[i]], sp))
    cases["s1b_异台词语料分组"] = (corpus1b, rows1b)

    # s3b：对照组——异台词但语料行序与全局说话序一致
    corpus3b = []
    rows3b = []
    seq = [("卡侬", can_lines[0]), ("桑娜妲", son_lines[0]),
           ("卡侬", can_lines[1]), ("桑娜妲", son_lines[1]),
           ("卡侬", can_lines[2]), ("桑娜妲", son_lines[2])]
    for gi, (sp, line) in enumerate(seq):
        corpus3b.append(sp + "\n" + line)
        rows3b.append((line, gi + 1, [gi + 1], sp))
    cases["s3b_异台词时间序对照"] = (corpus3b, rows3b)

    from fuse_core import split_header
    out = {}
    for name, (corpus, rows) in cases.items():
        texts = [t for t, _, _, _ in rows]
        cls = equiv_classes(corpus)
        S = _synth_S(tok, sess, corpus, texts)
        match_flat = align_flat(S)
        match_grp, groups = align_by_speaker(S, [r[3] for r in rows])
        out[name] = {"corpus": corpus, "rows": rows, "cls": cls, "S": S,
                     "flat": match_flat, "grp": match_grp, "groups": groups}
    return out


def _synth_S(tok, sess, corpus, texts):
    from fuse_core import similarity_matrix, split_header
    cb = [split_header(t)[1] for t in corpus]
    sb = [split_header(t)[1] for t in texts]
    S = similarity_matrix(tok, sess, cb, sb)
    mask_empty_body(S, corpus, texts)
    return S


def main():
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))

    print("== 真 4 案例：flat vs 按说话人分块 ==")
    tbl, total = real_cases(tok, sess)
    hdr = "{:<8}{:>4}{:>7}{:>12}{:>12}{:>12}{:>12}{:>9}{:>9}".format(
        "case", "n", "spk组", "flat默认", "分块默认", "flat掩码", "分块掩码", "退等?", "退UM?")
    print(hdr)
    for r in tbl:
        print("{:<8}{:>4}{:>7}{:>12}{:>12}{:>12}{:>12}{:>9}{:>9}".format(
            r["key"], r["n"], r["groups"],
            "{}/{}".format(*r["flat_def"]), "{}/{}".format(*r["grp_def"]),
            "{}/{}".format(*r["flat_um"]), "{}/{}".format(*r["grp_um"]),
            str(r["same_def"]), str(r["same_um"])))
    print("{:<8}{:>4}{:>7}{:>12}{:>12}{:>12}{:>12}".format(
        "合计", "", "",
        "{}/236".format(total["flat_def"][0]), "{}/236".format(total["grp_def"][0]),
        "{}/236".format(total["flat_um"][0]), "{}/236".format(total["grp_um"][0])))
    print("（数字为 score 等价类口径；括号内为 score_exact：")
    for r in tbl:
        print("   {} exact: flatdef {}/{}, grpdef {}/{}, flatum {}/{}, grpum {}/{}"
              .format(r["key"], r["flat_def"][1], r["n"], r["grp_def"][1], r["n"],
                      r["flat_um"][1], r["n"], r["grp_um"][1], r["n"]))
    print()
    print("== 语料侧说话人块可判定性（决定分块是否有前景）==")
    for r in tbl:
        print(" [{}] {}".format(r["key"], r["cohesion"]))
    print("== 分块与 flat 不一致的段（真案例）==")
    any_diff = False
    for r in tbl:
        if r["diffs"]:
            any_diff = True
            print(" [{}] 共 {} 段不同：".format(r["key"], len(r["diffs"])))
            diffs_speakers = _rows(r["key"])
            for k, mt, truth, f_pred, g_pred in r["diffs"]:
                sp = diffs_speakers[k].get("ref_speaker") or "?"
                print("   seg{:>3} 掩={:<2} spk={} truth={:<3} flat={:<3} grp={}".format(
                    k + 1, mt, sp, truth, f_pred, g_pred))
    if not any_diff:
        print(" （无差异）")

    print()
    print("== 合成 F14 案例（同台词跨说话人）==")
    sc = synth_cases(tok, sess)
    for name, d in sc.items():
        n = len(d["rows"])
        truth = [r[1] for r in d["rows"]]
        truth_ok = [r[2] for r in d["rows"]]
        scored = list(range(n))
        print(" [{}]".format(name))
        print("  flat：{}/{}  分块：{}/{}".format(
            score(d["flat"], truth_ok, scored, d["cls"], scored), n,
            score(d["grp"], truth_ok, scored, d["cls"], scored), n))
        print("  逐段（truth / flat / 分块，均为 1-based 行号）：")
        for i, r in enumerate(d["rows"]):
            mark = "" if d["flat"][i] + 1 == truth[i] else "  <-- flat 错"
            mark2 = "" if d["grp"][i] + 1 == truth[i] else "  <-- 分块错"
            print("   seg{} {}: truth={} flat={} 分块={}{}".format(
                i + 1, r[3], truth[i], d["flat"][i] + 1, d["grp"][i] + 1, mark + mark2))


if __name__ == "__main__":
    main()
