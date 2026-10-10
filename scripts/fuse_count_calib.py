#!/usr/bin/env python3
"""§7.8：置信计分目标（0/1 尺度）的整套重标 + 三个专项验证。

背景：DP 目标从「ΣS − 罚分」换成「落在可接受集合内的计数」——换矩阵不换 DP
（`fuse_calib.count_reward`）。老罚分按 ΣS 尺度（≈0.7~0.9）标定，换 0/1 尺度
全部失效 ⇒ (τ, miss_pen, eps, skip, repeat, reset) **整套重扫**。
本脚本**只测量**：落地状态（标定后的 COUNT_* 常量）写回 scripts/fuse_calib.py；
不改任何默认行为。

**实测先行结论（阶段 A 的动机）**：`R = where(S>=tau, +1, −miss_pen)` 的**纯 0/1
矩阵在四个案例上结构性退化**（同分多路径 ⇒ "配到哪一行"无歧视，vesna best 41/79、
pierro 1/120 量级塌方；miss_pen 因"不配免费"恒压弱格而不起作用）。最小修复
（**仍是换矩阵、不换 DP**）是加一个 ΣS 破平项：`R = where(...)+eps·S`——eps 进入
扫描并给出平台。

硬门：moon ≥ 15、glupov ≥ 21、pierro ≥ 119（现行 ΣS 默认逐位基线，不得低于）。

用法：
    python scripts/fuse_count_calib.py           # 全部
    python scripts/fuse_count_calib.py --stage A/B/C/1/2/3
"""
import argparse
import io
import json
import os
import sys

sys.dont_write_bytecode = True  # 防止 __pycache__ 再生
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                "..", "src-tauri", "tests"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np  # noqa: E402
import fuse_calib as fc  # noqa: E402

KEYS = ["moon", "glupov", "pierro", "vesna"]
GATE = {"moon": 15, "glupov": 21, "pierro": 119}
OUT_JSON = os.path.join(fc.BENCH_OUT, "count_calib.json")

# 本轮标定产物（与 fuse_calib.py 的 COUNT_* 常量一致）
BEST = {"tau": 0.88, "pen": 0.3, "eps": 0.38, "skip": 0.02, "un": 0.0,
        "rp": 0.25, "rs": 0.05}


def reward(S, tau, pen, eps):
    """count 矩阵（单源 = fuse_calib.count_reward；eps=0 即纯 0/1 设计态）。"""
    return fc.count_reward(S, tau=tau, miss_pen=pen, eps=eps)


def eval_cfg(data, keys, cfg, apply_mask=False):
    """→ dict(tot, n, per, diag, gate)；掩码判据始终在原始 S 上做（S_raw/S_masked）"""
    tot = n = 0
    per = {}
    diags = {}
    for key in keys:
        d = data[key]
        S0 = d["S_masked"] if apply_mask else d["S_raw"]
        R = S0 if cfg["eps"] is None else reward(S0, cfg["tau"], cfg["pen"], cfg["eps"])
        dg = {}
        m = fc.align_v2(R, cfg["skip"], cfg["un"],
                        repeat_penalty=cfg["rp"], reset_penalty=cfg["rs"], diag=dg)
        c = fc.score(m, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
        per[key] = (c, len(d["scored"]))
        tot += c
        n += len(d["scored"])
        diags[key] = dict(dg)
    ok = all(per[k][0] >= v for k, v in GATE.items())
    return {"cfg": dict(cfg), "tot": tot, "n": n, "per": per,
            "diag": diags, "gate": ok}


def fmt_cfg(c):
    return "tau={} eps={} pen={} skip={} un={} rp={} rs={}".format(
        c["tau"], c["eps"], c["pen"], c["skip"], c["un"],
        "inf" if not np.isfinite(c["rp"]) else c["rp"],
        "inf" if not np.isfinite(c["rs"]) else c["rs"])


def print_rows(rows, top=15):
    for r in rows[:top]:
        print("    {:>4}/{} gate={} {}  per={}".format(
            r["tot"], r["n"], "✓" if r["gate"] else "✗", fmt_cfg(r["cfg"]),
            {k: "{}/{}".format(*v) for k, v in r["per"].items()}))


def one_d_plateau(data, best, axis, grid):
    """其余参数固定在 best、单轴扫描：⟶ 该轴的逐点 (合计, per)；[x]=最优点"""
    want = (best["tot"], best["per"])
    vals = []
    for v in grid:
        if v == best["cfg"][axis]:
            vals.append("[%s]" % v)
            continue
        cfg = dict(best["cfg"])
        cfg[axis] = v
        r = eval_cfg(data, KEYS, cfg)
        same = r["gate"] and (r["tot"], r["per"]) == want
        vals.append(str(v) if same else "<{}>".format(r["tot"]))
    return vals


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--stage", default=None, help="A/B/C/1/2/3 之一，跑到该阶段为止")
    args = ap.parse_args()
    stop = args.stage

    tok, sess = fc.load_embedder()
    data = fc.build_matrices(KEYS, tok, sess)
    for k in KEYS:
        data[k]["S_raw"] = data[k]["S"]
        with io.open(os.path.join(fc.BENCH_OUT, "truth_{}.json".format(k)),
                     encoding="utf-8") as f:
            data[k]["rows"] = json.load(f)["rows"]
        Sm = data[k]["S_raw"].copy()
        data[k]["masked_rows"] = fc.mask_should_unmatched(Sm, data[k]["seg_texts"], data[k]["corpus"])
        data[k]["S_masked"] = Sm

    # ── 基线（硬门参照）──
    print("=" * 100)
    print("基线（ΣS 目标，DEFAULT=(0.02,0.25) rp=.25 rs=.05）")
    base_cfg = {"tau": None, "eps": None, "pen": None, "skip": fc.DEFAULT[0],
                "un": fc.DEFAULT[1], "rp": fc.REPEAT_DEFAULT, "rs": fc.RESET_DEFAULT}
    base = base_mask = {}
    for masked in (False, True):
        r = eval_cfg(data, KEYS, base_cfg, apply_mask=masked)
        if masked:
            base_mask = r
        else:
            base = r
        print("    {} 合计 {}/236  per={}  D(不配)={}".format(
            "掩码开" if masked else "      ", r["tot"],
            {k: "{}/{}".format(*v) for k, v in r["per"].items()},
            {k: r["diag"][k]["unmatched"] for k in KEYS}))
    results = {"base_ss": base, "base_ss_mask": base_mask}
    print("    §7.6 掩行数：{}".format({k: data[k]["masked_rows"] for k in KEYS}))

    # ── 阶段 A：eps × tau（pen=0.3 skip=0.05；eps=0 列即"纯 0/1 设计态"）──
    print()
    print("=" * 100)
    print("阶段 A：eps × tau（pen=0.3 skip=0.05 un=0 rp=0.25 rs=0.05）")
    print("   eps=0 列 = 设计原型（纯 0/1）——它塌不塌是 §7.8 的第一产出")
    EPSA = [0.0, 0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6]
    TAUSA = [0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9]
    rowsA = []
    for eps in EPSA:
        for tau in TAUSA:
            cfg = dict(BEST, tau=tau, eps=eps)
            rowsA.append(eval_cfg(data, KEYS, cfg))
    # 表：行=eps 列=tau
    print("   {:>6} {}".format("eps\\tau", "  ".join("{:>18}".format(t) for t in TAUSA)))
    for eps in EPSA:
        cells = []
        for tau in TAUSA:
            r = next(x for x in rowsA if x["cfg"]["tau"] == tau and x["cfg"]["eps"] == eps)
            g = "✓" if r["gate"] else "✗"
            cells.append("{:>7} {}".format("{}/{}".format(r["tot"], g), " "))
        print("   {:>6} {}".format(eps, " ".join("{:>15}".format(c) for c in cells)))
    feaA = sorted((r for r in rowsA if r["gate"]),
                  key=lambda r: (-r["tot"], r["cfg"]["eps"], r["cfg"]["tau"]))
    print("  过硬门 {}/{}；前 8：".format(len(feaA), len(rowsA)))
    print_rows(feaA, 8)
    bestA = max(feaA, key=lambda r: (-r["tot"], r["gate"]))
    results["stageA"] = [{k: r[k] for k in ("cfg", "tot", "per", "gate")} for r in rowsA]
    print("  A 最优：tot={} cfg={}".format(bestA["tot"], fmt_cfg(bestA["cfg"])))
    if stop == "A":
        dump(results)
        return 0

    # ── 阶段 B：两个候选区的精扫（A 给出：低 eps 低 τ 区、高 τ 区）──
    print()
    print("=" * 100)
    print("阶段 B：τ × eps × skip（pen=0.3 un=0 rp=0.25 rs=0.05；两区合一）")
    # 区1：低 eps（≈0.38 单点）× 高 τ（0.85~0.95）——§7.8 的最优岛（细步：eps 刀锋 ±0.005）
    # 区2：较高 eps ≥0.5 × 中低 τ（0.4~0.7）——已知平台 210（稳健备选）
    rowsB = []
    for tau in (0.75, 0.85, 0.88, 0.9, 0.92, 0.95):
        for eps in (0.3, 0.35, 0.375, 0.38, 0.385, 0.4, 0.45, 0.55):
            for skip in (0.01, 0.02, 0.03, 0.05):
                cfg = dict(BEST, tau=tau, eps=eps, skip=skip)
                rowsB.append(eval_cfg(data, KEYS, cfg))
    for tau, eps_list in [(t, [0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2]) for t in
                          (0.4, 0.5, 0.6, 0.7)]:
        for eps in eps_list:
            for skip in (0.02, 0.03, 0.05, 0.08):
                cfg = dict(BEST, tau=tau, eps=eps, skip=skip)
                rowsB.append(eval_cfg(data, KEYS, cfg))
    # 去重（两区在 τ 无交叠；保险起见仍按 cfg 去重）
    seen = set()
    uniqB = []
    for r in rowsB:
        k = json.dumps(r["cfg"], sort_keys=True)
        if k not in seen:
            seen.add(k)
            uniqB.append(r)
    rowsB = uniqB
    feaB = sorted((r for r in rowsB if r["gate"]),
                  key=lambda r: (-r["tot"], abs(r["cfg"]["eps"] - BEST["eps"]),
                                 abs(r["cfg"]["skip"] - BEST["skip"]),
                                 r["cfg"]["tau"], r["cfg"]["pen"]))
    print("  过硬门 {}/{}；前 10：".format(len(feaB), len(rowsB)))
    print_rows(feaB, 10)
    bestB = feaB[0]
    results["stageB"] = [{k: r[k] for k in ("cfg", "tot", "per", "gate")} for r in rowsB]
    print("  B 最优：tot={} cfg={}".format(bestB["tot"], fmt_cfg(bestB["cfg"])))
    if stop == "B":
        dump(results)
        return 0

    # ── 阶段 C：最终最优 + 1D 平台宽度 ──
    print()
    print("=" * 100)
    print("阶段 C：最优 = B 最优；平台（其余参数固定在最优；[x]=最优点，同 (合计, per) 且过硬门）")
    best = bestB
    print("  最优配置：{} ⇒ 合计 {}/{}  per={}  diag(vesna)={}".format(
        fmt_cfg(best["cfg"]), best["tot"], best["n"],
        {k: "{}/{}".format(*v) for k, v in best["per"].items()},
        best["diag"]["vesna"]))
    plat = {}
    for axis, grid in [
            ("tau", [0.4, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.78, 0.8, 0.82,
                     0.84, 0.85, 0.86, 0.88, 0.9, 0.92, 0.95]),
            ("eps", [0.2, 0.3, 0.35, 0.375, 0.38, 0.385, 0.4, 0.45, 0.5, 0.55, 0.6,
                     0.7, 0.8, 0.9, 1.0, 1.2, 1.5, 2.0]),
            ("skip", [0.0, 0.005, 0.01, 0.02, 0.03, 0.05, 0.08, 0.1, 0.15, 0.2, 0.3, 0.5]),
            ("pen", [0.02, 0.05, 0.1, 0.2, 0.25, 0.28, 0.3, 0.32, 0.34, 0.38, 0.5,
                     1.0, 2.0]),
            ("rp", [0.05, 0.1, 0.25, 0.4, 0.5, 1.0, float("inf")]),
            ("rs", [0.005, 0.01, 0.02, 0.05, 0.08, 0.1, 0.2, 0.3, 0.5, 1.0, float("inf")])]:
        vals = one_d_plateau(data, best, axis, grid)
        plat[axis] = vals
        print("  {:<5} {}".format(axis, "  ".join(vals)))
    results["best"] = {"cfg": best["cfg"], "tot": best["tot"], "per": best["per"],
                       "diag": best["diag"]}
    results["plateau1d"] = plat
    if stop == "C":
        dump(results)
        return 0

    # ── 专项① reset 敏感性 ──
    print()
    print("=" * 100)
    print("专项①：reset ∈ {0.05, 0.2, 1.0, inf}（新目标 vs ΣS 目标；观察'set大罚反而更差'是否消失）")
    spec1 = {}
    for cnt in (False, True):
        for rs in (0.05, 0.2, 1.0, float("inf")):
            if cnt:
                r = eval_cfg(data, KEYS, dict(best["cfg"], rs=rs))
            else:
                r = eval_cfg(data, KEYS, dict(base_cfg, rs=rs))
            tag = "计分" if cnt else "ΣS  "
            s = "inf" if not np.isfinite(rs) else rs
            spec1["{}@{}".format("count" if cnt else "ss", s)] = {
                "per": r["per"], "tot": r["tot"], "resets": {k: r["diag"][k]["reset"] for k in KEYS}}
            print("   {} @{:<5} 合计 {:>4}/{}  per={}  回退次数={}".format(
                tag, s, r["tot"], r["n"],
                {k: "{}/{}".format(*v) for k, v in r["per"].items()},
                {k: r["diag"][k]["reset"] for k in KEYS}))
    results["spec1"] = spec1
    if stop == "1":
        dump(results)
        return 0

    # ── 专项② §7.6 掩码是否被取代 ──
    print()
    print("=" * 100)
    print("专项②：掩码（关/开）× 对齐目标（ΣS/计分）同台；D=diag unmatched")
    spec2 = {}
    for cnt in (False, True):
        for masked in (False, True):
            r = eval_cfg(data, KEYS, base_cfg if not cnt else best["cfg"],
                         apply_mask=masked)
            tag = "{}+{}".format("计分" if cnt else "ΣS", "掩码开" if masked else "无掩码")
            spec2["{}_{}".format("count" if cnt else "ss",
                                 "on" if masked else "off")] = {
                "tot": r["tot"], "per": r["per"],
                "diag_unmatched": {k: r["diag"][k]["unmatched"] for k in KEYS}}
            print("   {:<9} 合计 {:>4}/{}  per={}  D={}".format(
                tag, r["tot"], r["n"],
                {k: "{}/{}".format(*v) for k, v in r["per"].items()},
                {k: r["diag"][k]["unmatched"] for k in KEYS}))
    print("   §7.6 掩行数 = {}；空集计分段数 = {}".format(
        {k: data[k]["masked_rows"] for k in KEYS},
        {k: sum(1 for kk in data[k]["scored"] if not data[k]["truth_ok"][kk]) for k in KEYS}))
    results["spec2"] = spec2
    if stop == "2":
        dump(results)
        return 0

    # ── 专项③ 分段 × 新目标 ──
    print()
    print("=" * 100)
    print("专项③：显式重播分段 × 对齐目标（vesna 块内天花板参照 72/79）")
    cuts_all = {}
    for key in KEYS:
        cuts = fc.replay_boundaries(data[key]["rows"], tok, sess, data[key]["seg_texts"])
        cuts_all[key] = cuts
        print("  {:<8} 分段切点 {}".format(key, cuts))
    d = data["vesna"]
    bnds = [0] + cuts_all["vesna"] + [d["S"].shape[0]]
    # 块下标 ↔ 本案例"段序号"（scored 的 idxs 即序号本身，等价类全局）
    clk = []
    for a, b in zip(bnds[:-1], bnds[1:]):
        cc, _ = fc.ceiling_of(d["truth_ok"], [k for k in d["scored"] if a <= k < b], d["cls"])
        clk.append((a, b, cc))
    blk_sum = sum(c for _, _, c in clk)
    print("  vesna 块内天花板 = {}：{}".format(
        blk_sum, " + ".join("块[{}, {})={}".format(a, b, c) for a, b, c in clk)))
    spec3 = {"vesna_block_ceiling": {"blocks": [[a, b, c] for a, b, c in clk], "sum": blk_sum}}
    for cnt in (False, True):
        for rs in (0.05, 0.2, 1.0, float("inf")):
            rows_out = []
            tot = 0
            for key in KEYS:
                dd = data[key]
                dg = {}
                if cnt:
                    R = reward(dd["S_raw"], best["cfg"]["tau"], best["cfg"]["pen"], best["cfg"]["eps"])
                    m, dg = fc.align_segmented(R, cuts_all[key], best["cfg"]["skip"], 0.0,
                                               repeat_penalty=best["cfg"]["rp"], reset_penalty=rs)
                else:
                    m, dg = fc.align_segmented(dd["S_raw"], cuts_all[key],
                                               fc.DEFAULT[0], fc.DEFAULT[1],
                                               repeat_penalty=fc.REPEAT_DEFAULT, reset_penalty=rs)
                c = fc.score(m, dd["truth_ok"], dd["scored"], dd["cls"], dd["idxs"])
                rows_out.append("{}:{}({}r{})".format(key, c, len(dd["scored"]), dg["reset"]))
                tot += c
            s = "inf" if not np.isfinite(rs) else rs
            spec3["{}_{}".format("count" if cnt else "ss", s)] = {
                "per": rows_out, "tot": tot}
            print("   {} rs={:<5} 分段后合计 {:>4}/{}   {}".format(
                "计分" if cnt else "ΣS  ", s, tot, base["n"], "  ".join(rows_out)))
    results["spec3"] = spec3
    dump(results)
    return 0


def dump(results):
    with io.open(OUT_JSON, "w", encoding="utf-8", newline="") as f:
        json.dump(results, f, ensure_ascii=False, indent=1)
    print()
    print("已落盘 {}".format(OUT_JSON))


if __name__ == "__main__":
    sys.exit(main())
