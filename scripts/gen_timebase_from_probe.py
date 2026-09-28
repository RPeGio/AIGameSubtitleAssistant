#!/usr/bin/env python3
"""生成嵌字基准的「参考时基产物」benchmark/timebase/<case>_timebase.json。

背景（A4）：嵌字基准的真值是参考文件里的时间码，而参考是**素材英文字幕的中文翻译**、
时码继承自外部字幕源，其时间轴与交付 clip 差一个线性缩放（`t_video ≈ t_ref×(1+k)+a`）。
此前该换算以手抄常量存在，既不可审计，也无法察觉素材被替换。

本脚本消费**细网格夹逼探针**（`scripts/bench_timebase_probe.py`，0.05s 网格、区间中点估计、
不确定度 ±0.025s）的输出，产出产物并保证两件事：

1. **fit 与 applied 分离**：`fit` = 当前最佳测量（含 95% CI、Theil–Sen 抗差复核、逐条实测值）；
   `applied` = 实际施加到基准的换算，**只有显式传 `--apply` 才会跟着 fit 改**——
   避免"重跑一次探针就悄悄改了分数"。
2. **离群点按先验规则排除，不按残差剔除**（旧脚本用 `max(1.0, 4·MAD)` 迭代剔除，
   在残差本来就小的时候退化成"随手丢点"）。规则：探针必须真的测到**实质文本变化**——
   `归一化(before)` 与 `归一化(after)` 的编辑距离比 > 0.35（与探针的检测阈值同源）。
   例如旧数据里 glupov #19 测到的是"同一姓名框的两种渲染"，归一化后几乎相同 → 按规则排除，
   而不是因为它偏离直线才被丢掉。

用法：
  python scripts/gen_timebase_from_probe.py            # 只更新 fit，applied 保持不变
  python scripts/gen_timebase_from_probe.py --apply    # 同时把 applied 切到新 fit（会改分数）
"""
import hashlib
import itertools
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "examples" / "benchmark_examples"
OUT_DIR = ROOT / "benchmark" / "timebase"   # 必须入库（examples/ 被 gitignore）
PROBE_TSV = ROOT / "temp" / "probe" / "ref_calib_fine.tsv"
CASES = {
    "glupov": ("quality_bench_test(non-voiced)_11min.mp4",
               "quality_bench_test(non-voiced)_11min_reference.txt"),
    "moon_sisters": ("quality_bench_test(voiced)_5min.mp4",
                     "quality_bench_test(voiced)_5min_reference.txt"),
    "pierro_questions": ("quality_bench_test(voiced)_48min.mp4",
                         "quality_bench_test(voiced)_48min_reference.txt"),
}
# 首次生成（产物不存在）时的 applied 兜底：现行生效值
APPLIED_FALLBACK = {
    "glupov": (0.007092, -0.021),
    "moon_sisters": (0.0, 0.0),
    "pierro_questions": (0.000645, -0.121),
}
NOTE = {
    "moon_sisters": "刻意不校准：参考经复核无漂移（k≈0）；拟合返回的 a 与估计量在真值 (0,0) 上的偏差地板同量级",
}
GATE = {"max_resid_median_sec": 0.15, "max_k_ci95": 0.001}
PROBE_DOC = ("细网格夹逼探针 scripts/bench_timebase_probe.py（0.05s 网格；"
             "t_a=末个未变帧、t_b=首个已变帧，取中点 τ，不确定度 ±(t_b−t_a)/2；"
             "窗口按当前换算预居中到 ±0.4s）。跨语言、不依赖参考文本内容。")
CHANGE_MIN_RATIO = 0.35   # 与探针检测阈值同源：测到的变化必须真是"实质文本变化"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fp:
        for chunk in iter(lambda: fp.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def duration(path):
    try:
        out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                              "-of", "default=nw=1:nk=1", str(path)],
                             capture_output=True, text=True, check=True).stdout.strip()
        return float(out)
    except Exception:
        return None


def norm(s):
    return "".join(c.lower() for c in s if c.isalnum())


def lev_ratio(a, b):
    if not a and not b:
        return 0.0
    n, m = len(a), len(b)
    prev = list(range(m + 1))
    for i in range(1, n + 1):
        cur = [i] + [0] * m
        for j in range(1, m + 1):
            cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] != b[j - 1]))
        prev = cur
    return prev[m] / max(n, m, 1)


def load_probe():
    rows = []
    for line in PROBE_TSV.read_text(encoding="utf-8").splitlines()[1:]:
        f = line.split("\t")
        if len(f) < 10 or f[0] not in CASES:
            continue
        rows.append({
            "case": f[0], "idx": int(f[1]), "ref": float(f[2]),
            "t_a": float(f[3]) if f[3] else None,
            "t_b": float(f[4]) if f[4] else None,
            "tau": float(f[5]) if f[5] else None,
            "half": float(f[6]) if f[6] else None,
            "before": f[7], "after": f[8], "note": f[9],
        })
    return rows


def validity(r):
    """先验有效性：必须测到实质文本变化（返回 (bool, 原因)）。"""
    if r["tau"] is None:
        return False, "窗口内未测到变化"
    if not norm(r["after"]):
        return False, "变化后文本为空（窗内被清空而非换句）"
    ratio = lev_ratio(norm(r["before"]), norm(r["after"]))
    if ratio <= CHANGE_MIN_RATIO:
        return False, (f"测到的不是实质文本变化（归一化差异 {ratio:.2f} ≤ {CHANGE_MIN_RATIO}）"
                       f"：{r['before'][:32]} → {r['after'][:32]}")
    return True, ""


def fit(rows):
    n = len(rows)
    sx = sum(r["ref"] for r in rows)
    sy = sum(r["delta"] for r in rows)
    sxx = sum(r["ref"] ** 2 for r in rows)
    sxy = sum(r["ref"] * r["delta"] for r in rows)
    den = n * sxx - sx * sx
    k = (n * sxy - sx * sy) / den
    a = (sy - k * sx) / n
    res = [r["delta"] - (a + k * r["ref"]) for r in rows]
    ss_res = sum(x * x for x in res)
    ss_tot = sum((r["delta"] - sy / n) ** 2 for r in rows)
    sxx_c = sum((r["ref"] - sx / n) ** 2 for r in rows)
    rmse = (ss_res / n) ** 0.5
    se_k = rmse / (sxx_c ** 0.5)
    se_a = rmse * (1.0 / n + (sx / n) ** 2 / sxx_c) ** 0.5
    return {"a": a, "k": k, "r2": 1 - ss_res / (ss_tot or 1e-12), "rmse": rmse,
            "res": res, "k_ci95": se_k * 1.96, "a_ci95": se_a * 1.96}


def theil_sen(rows):
    sl = [(b["delta"] - a["delta"]) / (b["ref"] - a["ref"])
          for a, b in itertools.combinations(rows, 2) if abs(b["ref"] - a["ref"]) > 1e-9]
    sl.sort()
    m = len(sl)
    return (sl[m // 2] if m % 2 else (sl[m // 2 - 1] + sl[m // 2]) / 2) if sl else 0.0


def main():
    apply_now = "--apply" in sys.argv
    set_a = None
    for i, tok in enumerate(sys.argv):          # 兼容 --set-a -0.139 与 --set-a=-0.139
        if tok == "--set-a" and i + 1 < len(sys.argv):
            set_a = float(sys.argv[i + 1])
        elif tok.startswith("--set-a="):
            set_a = float(tok.split("=", 1)[1])
    if not PROBE_TSV.is_file():
        print(f"[失败] 找不到探针数据 {PROBE_TSV}（先跑 scripts/bench_timebase_probe.py）",
              file=sys.stderr)
        return 1
    rows = load_probe()
    print(f"探针记录 {len(rows)} 条（{PROBE_TSV.name}）｜"
          f"{'应用新 fit 到 applied' if apply_now else '只更新 fit，applied 保持不变'}\n")
    for case, (clip_name, ref_name) in CASES.items():
        case_rows = [r for r in rows if r["case"] == case]
        valid, invalid = [], []
        for r in case_rows:
            ok, why = validity(r)
            r["delta"] = (r["tau"] - r["ref"]) if r["tau"] is not None else None
            (valid if ok else invalid).append(r)
        if len(valid) < 5:
            print(f"{case}: 有效点不足（{len(valid)}）→ 跳过", file=sys.stderr)
            continue
        f = fit(valid)
        out = OUT_DIR / f"{case}_timebase.json"
        prev = json.loads(out.read_text(encoding="utf-8")) if out.is_file() else None
        if set_a is not None:
            # 统一口径实验：三案例共用同一个 a，k 仍取各自 fit（配合 git checkout 还原）
            ap_k, ap_a = round(f["k"], 6), set_a
        elif prev and not apply_now:
            ap_k, ap_a = prev["applied"]["k"], prev["applied"]["a"]
        elif apply_now:
            ap_k, ap_a = round(f["k"], 6), round(f["a"], 4)
        else:
            ap_k, ap_a = APPLIED_FALLBACK[case]
        note = NOTE.get(case, "")
        if set_a is not None:
            note = f"实验：统一 a={set_a}（k 取本案例 fit）——实验后须还原"
        if abs(round(f["k"], 6) - ap_k) > 1e-9 or abs(round(f["a"], 3) - ap_a) > 1e-6:
            note = (note + "；" if note else "") + (
                f"⚠ 当前 fit（k={f['k']:+.6f} a={f['a']:+.3f}）与 applied 不同——"
                f"切换需显式运行 --apply 并重记基线")
        clip, ref = DATA / clip_name, DATA / ref_name
        art = {
            "schema": 1,
            "case": case,
            "probe": {
                "source": str(PROBE_TSV.relative_to(ROOT)).replace("\\", "/"),
                "date": "2026-09-28",
                "method": PROBE_DOC,
                "n_total": len([r for r in case_rows if r["tau"] is not None]),
                "failed": [{"idx": r["idx"], "ref_start": r["ref"], "reason": r["note"] or "未测到"}
                           for r in case_rows if r["tau"] is None],
                "excluded": [{"idx": r["idx"], "ref_start": r["ref"],
                              "bracket": [r["t_a"], r["t_b"]], "reason": why,
                              "before": r["before"], "after": r["after"]}
                             for r, why in ((r, validity(r)[1]) for r in invalid)],
            },
            "fit": {
                "estimator": ("最小二乘（有效点全量，**不做残差剔除**）＋ 95% CI；"
                              "另附 Theil–Sen 抗差复核。有效性由先验规则判定：探针必须测到"
                              f"实质文本变化（归一化差异 > {CHANGE_MIN_RATIO}）"),
                "k": round(f["k"], 8), "a": round(f["a"], 4),
                "k_ci95": round(f["k_ci95"], 8), "a_ci95": round(f["a_ci95"], 4),
                "r2": round(f["r2"], 4), "rmse_sec": round(f["rmse"], 4),
                "resid_median_sec": round(sorted(f["res"])[len(f["res"]) // 2], 4),
                "n_used": len(valid),
                "point_halfwidth_sec": max((r["half"] or 0) for r in valid),
                "robust_check": {
                    "theil_sen_k_valid_points": round(theil_sen(valid), 8),
                    "theil_sen_k_all_rows": round(theil_sen([r for r in case_rows
                                                             if r["tau"] is not None]), 8),
                    "note": "Theil–Sen（点对斜率中位数）不做任何剔除；与拟合值接近即说明 k 稳健",
                },
            },
            "applied": {"k": ap_k, "a": ap_a, "note": note},
            "quality_gate": GATE,
            "inputs": {
                "clip": {"file": clip_name, "bytes": clip.stat().st_size,
                         "sha256": sha256(clip), "duration_sec": duration(clip)},
                "reference": {"file": ref_name, "bytes": ref.stat().st_size,
                              "sha256": sha256(ref)},
            },
            "onsets": [{"idx": r["idx"], "ref_start": round(r["ref"], 3),
                        "t_onset": round(r["tau"], 4), "halfwidth": r["half"]}
                       for r in sorted([r for r in case_rows if r["tau"] is not None],
                                       key=lambda z: z["idx"])],
        }
        OUT_DIR.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(art, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"{case:<17} n={len(valid):>3}/{len(case_rows):>3}（规则排除 {len(invalid)}）"
              f"  k={f['k'] * 100:+.4f}% ±{f['k_ci95'] * 100:.4f}%"
              f"  a={f['a']:+.3f}s ±{f['a_ci95']:.3f}"
              f"  R²={f['r2']:.4f} rmse={f['rmse']:.3f}s"
              f"  TheilSen={theil_sen(valid) * 100:+.4f}%")
        print(f"{'':<17} applied: k={ap_k:+.6f} a={ap_a:+.3f}"
              f"{'  ⚠ 与 fit 不同（待决策切换）' if abs(round(f['k'], 6) - ap_k) > 1e-9 or abs(round(f['a'], 3) - ap_a) > 1e-6 else '  （与 fit 一致）'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
