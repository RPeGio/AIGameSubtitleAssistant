#!/usr/bin/env python3
"""生成嵌字基准的「参考时基产物」examples/benchmark_examples/<case>_timebase.json。

背景（A4）：嵌字基准的真值是参考文件里的时间码，而参考是**素材英文字幕的中文翻译**、
时码继承自外部字幕源，其时间轴与交付 clip 差一个线性缩放（实测 glupov ≈+0.71%、
pierro ≈+0.065%、moon ≈0）。此前该换算以**手抄常量**形式写在 tests/common/mod.rs 的
CaseCfg 里，既不可审计，也无法察觉"素材被替换/重编码"（会静默错算分数）。

本脚本把换算搬进可审计产物：
  · 复现原标定脚本 temp/probe/ref_calib_fit.py 的算法（硬编码 EXCLUDE + 迭代稳健剔除），
    并**断言**复现出的 (k, a) 与生成时 CaseCfg 里的现用常量一致（防止产物与代码脱节）；
  · 补上原脚本没有的**不确定度**（OLS 斜率/截距 95% CI）与**抗差复核**（Theil–Sen 点对
    斜率中位数：不剔任何点也要给出接近的 k，用于回答"这个 k 是不是挑出来的"）；
  · 记录**逐条探针实测值**（含所测文本变化片段）与 **clip / 参考文件的 SHA256**；
  · 记录**实际施加**的换算（moon 刻意不校准，需与拟合值区分）。

原始探针数据 temp/probe/ref_calib.tsv 不在版本库内（temp/ 被 gitignore）；产物内嵌了
全部逐条实测值与所测文本，故可离线审计；需要重跑探针时用 temp/probe/ref_calib.py。

用法：python scripts/gen_timebase_from_probe.py [ref_calib.tsv 路径]
"""
import hashlib
import itertools
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "examples" / "benchmark_examples"
# 产物必须落在**入库**目录（examples/ 整体被 .gitignore 忽略，素材与参考都不入库，
# 故产物放那里等于不入库）；它记录的是"该素材的实测时基"，与本仓库的素材观测绑定。
OUT_DIR = ROOT / "benchmark" / "timebase"
DEFAULT_TSV = ROOT / "temp" / "probe" / "ref_calib.tsv"

# 生成时 CaseCfg 里的现用常量（(ref_scale, ref_offset)）——复现结果必须与之逐位一致，
# 否则说明产物与代码脱节（本脚本拒绝写盘）。
EXPECTED_FIT = {
    "glupov": (0.007092, -0.021),
    "moon_sisters": (-0.000057, -0.075),
    "pierro_questions": (0.000645, -0.121),
}
# 实际施加到基准的换算（moon 判为"参考本身准确、估计量的 −0.075s 是偏差地板"，刻意置 0）
APPLIED = {
    "glupov": (0.007092, -0.021, "沿用稳健拟合值"),
    "moon_sisters": (0.0, 0.0, "刻意不校准：参考经复核无漂移（k≈0），"
                              "拟合返回的 a=−0.075s 与估计量在真值 (0,0) 上的偏差地板同值"),
    "pierro_questions": (0.000645, -0.121, "沿用稳健拟合值（剔除 2 个离群探针点）"),
}
# 与 ref_calib_fit.py 一致：已知语义异常点（glupov #19 测的是姓名框清空，不是新句出现）
HARD_EXCLUDE = {("glupov", 19)}
# 质量门（Rust 侧加载时同样断言）：残差中位、k 的 95% CI 半宽
GATE = {"max_resid_median_sec": 0.15, "max_k_ci95": 0.001}

CASES = {
    "glupov": ("quality_bench_test(non-voiced)_11min.mp4",
               "quality_bench_test(non-voiced)_11min_reference.txt"),
    "moon_sisters": ("quality_bench_test(voiced)_5min.mp4",
                     "quality_bench_test(voiced)_5min_reference.txt"),
    "pierro_questions": ("quality_bench_test(voiced)_48min.mp4",
                         "quality_bench_test(voiced)_48min_reference.txt"),
}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fp:
        for chunk in iter(lambda: fp.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def probe_sha256(path):
    """调用 ffprobe 取 duration（只读，失败则留空）。"""
    try:
        out = subprocess.run(
            ["ffprobe", "-v", "error", "-show_entries", "format=duration",
             "-of", "default=nw=1:nk=1", str(path)],
            capture_output=True, text=True, check=True).stdout.strip()
        return float(out)
    except Exception:
        return None


def load_tsv(path):
    """按**记录**解析：记录以 'case<TAB>idx<TAB>float…' 起；文本字段内含换行，
    故先按记录收集原始文本，再整段按 TAB 切分成字段（边界才不会被换行打乱）。"""
    recs, raw = [], None
    for line in path.read_text(encoding="utf-8").splitlines():
        f = line.split("\t")
        head = len(f) >= 5 and f[0] in CASES
        if head:
            try:
                int(f[1]); float(f[2]); float(f[3]); float(f[4])
            except ValueError:
                head = False
        if head:
            if raw is not None:
                recs.append(raw)
            raw = line
        elif raw is not None:
            raw += "\n" + line
    if raw is not None:
        recs.append(raw)
    out = []
    for r in recs:
        f = r.split("\t")
        out.append({
            "case": f[0], "idx": int(f[1]), "ref": float(f[2]),
            "onset": float(f[3]), "delta": float(f[4]),
            "prod_start": f[5] if len(f) > 5 else "",
            "prod_delta": f[6] if len(f) > 6 else "",
            "ref_text": f[7] if len(f) > 7 else "",
            "before": f[8] if len(f) > 8 else "",
            "after": f[9] if len(f) > 9 else "",
        })
    return out


def squeeze(s, n):
    return " ".join(s.split())[:n]


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
    return a, k, 1 - ss_res / (ss_tot or 1e-12), (ss_res / n) ** 0.5, res


def se_ci(rows):
    """OLS 的 k / a 标准误与 95% CI 半宽。"""
    n = len(rows)
    a, k, _, rmse, _ = fit(rows)
    tbar = sum(r["ref"] for r in rows) / n
    sxx = sum((r["ref"] - tbar) ** 2 for r in rows)
    se_k = rmse / (sxx ** 0.5)
    se_a = rmse * (1.0 / n + tbar ** 2 / sxx) ** 0.5
    return se_k * 1.96, se_a * 1.96


def theil_sen(rows):
    sl = [(b["delta"] - a["delta"]) / (b["ref"] - a["ref"])
          for a, b in itertools.combinations(rows, 2) if abs(b["ref"] - a["ref"]) > 1e-9]
    sl.sort()
    m = len(sl)
    return (sl[m // 2] if m % 2 else (sl[m // 2 - 1] + sl[m // 2]) / 2)


def robust_keep(rows):
    """复现 ref_calib_fit.py 的迭代稳健剔除（阈值 max(1.0, 4·MAD)，至多 3 轮）。"""
    keep = [r for r in rows if (r["case"], r["idx"]) not in HARD_EXCLUDE]
    dropped = []
    for _ in range(3):
        a, k, _, _, res = fit(keep)
        med = sorted(res)[len(res) // 2]
        mad = sorted(abs(r - med) for r in res)[len(res) // 2] or 1e-9
        thr = max(1.0, 4 * mad)
        new = [r for r, x in zip(keep, res) if abs(x - med) <= thr]
        if len(new) == len(keep):
            break
        dropped += [r for r in keep if r not in new]
        keep = new
    return keep, dropped


def main():
    tsv = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_TSV
    if not tsv.is_file():
        print(f"[失败] 找不到探针数据 {tsv}", file=sys.stderr)
        return 1
    records = load_tsv(tsv)
    print(f"探针记录 {len(records)} 条（来自 {tsv}）\n")
    for case, (clip_name, ref_name) in CASES.items():
        rows = [r for r in records if r["case"] == case]
        keep, dropped = robust_keep(rows)
        a, k, r2, rmse, res = fit(keep)
        ci_k, ci_a = se_ci(keep)
        exp_k, exp_a = EXPECTED_FIT[case]
        print(f"{case:<17} n={len(keep):>3}/{len(rows):>3}  k={k:+.6f} a={a:+.3f}  "
              f"R²={r2:.4f} rmse={rmse:.3f}s  CI95(k)±{ci_k:.6f}")
        if round(k, 6) != exp_k or round(a, 3) != exp_a:
            print(f"  ✗ 与 CaseCfg 现用常量不符（期望 k={exp_k:+.6f} a={exp_a:+.3f}，"
                  f"复现得 k={k:+.6f} a={a:+.3f}）→ 不写盘", file=sys.stderr)
            return 1
        excl = []
        for r in dropped + [r for r in rows if (r["case"], r["idx"]) in HARD_EXCLUDE]:
            hard = (r["case"], r["idx"]) in HARD_EXCLUDE
            excl.append({
                "idx": r["idx"], "ref_start": r["ref"], "delta": r["delta"],
                "reason": "硬编码：探针测到的是相邻条目的变化（非本句出现）" if hard
                          else "稳健剔除：残差超 max(1.0, 4·MAD)",
                "reference_text": squeeze(r["ref_text"], 80),
                "probe_saw": f"{squeeze(r['before'], 50)} → {squeeze(r['after'], 50)}",
                "pipeline_delta": r["prod_delta"],
            })
        clip, ref = DATA / clip_name, DATA / ref_name
        ap_k, ap_a, note = APPLIED[case]
        art = {
            "schema": 1,
            "case": case,
            "probe": {
                "source": str(tsv.relative_to(ROOT)).replace("\\", "/"),
                "date": "2026-09-19",
                "method": "逐条实测视频中字幕真正出现的时刻（0.25s 网格 + 连续两帧变化判据；"
                          "跨语言、不依赖参考文本），与参考时间码作差得 Δ(t)",
                "n_total": len(rows),
                "excluded": excl,
            },
            "fit": {
                "estimator": "两参数最小二乘 + 迭代稳健剔除（阈值 max(1.0, 4·MAD)，至多 3 轮）；"
                             "本产物由 scripts/gen_timebase_from_probe.py 复现 ref_calib_fit.py 得到",
                "k": round(k, 8), "a": round(a, 4),
                "k_ci95": round(ci_k, 8), "a_ci95": round(ci_a, 4),
                "r2": round(r2, 4), "rmse_sec": round(rmse, 4),
                "resid_median_sec": round(sorted(res)[len(res) // 2], 4),
                "n_used": len(keep),
                "robust_check": {
                    "theil_sen_k_all_points": round(theil_sen(rows), 8),
                    "theil_sen_k_used_points": round(theil_sen(keep), 8),
                    "note": "Theil–Sen（点对斜率中位数）不做任何剔除；与拟合值接近即说明 k 不是挑点挑出来的",
                },
            },
            "applied": {"k": ap_k, "a": ap_a, "note": note},
            "quality_gate": GATE,
            "inputs": {
                "clip": {"file": clip_name, "bytes": clip.stat().st_size,
                         "sha256": sha256(clip), "duration_sec": probe_sha256(clip)},
                "reference": {"file": ref_name, "bytes": ref.stat().st_size,
                              "sha256": sha256(ref)},
            },
            "onsets": [{"idx": r["idx"], "ref_start": round(r["ref"], 3),
                        "t_onset": round(r["onset"], 3), "delta": round(r["delta"], 3)}
                       for r in sorted(rows, key=lambda z: z["idx"])],
        }
        out = OUT_DIR / f"{case}_timebase.json"
        OUT_DIR.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(art, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"  ✓ 写出 {out.relative_to(ROOT)}（剔除 {len(excl)} 点，"
              f"Theil–Sen 全量 {art['fit']['robust_check']['theil_sen_k_all_points']:+.6f}）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
