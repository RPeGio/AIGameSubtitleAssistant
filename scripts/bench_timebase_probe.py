#!/usr/bin/env python3
"""参考时基**细网格夹逼探针**（A4 方案 B）。

与旧探针（`temp/probe/ref_calib.py`）的区别：
1. **网格 0.25s → 可配（默认 0.05s）**：分辨率 ×5。
2. **区间夹逼取代"首个变化帧"**：对每个参考起点，取
   `t_a = 最后一个仍与窗口起始文本一致的帧`、`t_b = 首个已变化的帧`，
   真实切换必在 (t_a, t_b] 内 → 估 `τ = (t_a+t_b)/2`、不确定度 `±(t_b−t_a)/2`。
   旧法报"首个变化帧"，在 0.25s 网格下天然偏晚 0~0.25s（均值 +0.125s），
   该偏差会**整体进入截距 a**；夹逼的中点估计在构造上抵消它。
3. **窗口按当前换算预居中**（`t_video = ref×(1+k)+a`，取自已入库产物）：窗口可窄到 ±0.4s，
   帧数与 OCR 次数大减，同时天然只接受"当前估计 ±0.4s 内"的修正。
4. **选区与视频尺寸不再硬编码**：选区从 `src-tauri/tests/common/mod.rs::hardsub_regions`
   解析（基准的受控副本），尺寸用 ffprobe 取——旧探针硬编码的那份曾导致 13px 错位。
5. 批处理 OCR（每批 16 帧）。

输出：`temp/probe/ref_calib_fine.tsv`（逐条：idx/ref_start/t_a/t_b/tau/halfwidth/before/after），
不修改任何基准数据。

用法：python scripts/bench_timebase_probe.py [--step 0.05] [--half-window 0.4] [--cases a,b]
环境：PYTHONPATH="<runtime>\\deps_gpu;<runtime>\\deps"、PADDLE_PDX_CACHE_HOME=<runtime>\\models\\paddleocr、
      PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK=True、GSA_OCR_DEVICE=gpu:0
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUNTIME = ROOT / "runtime"
DATA = ROOT / "examples" / "benchmark_examples"
TB = ROOT / "benchmark" / "timebase"
OUT_DIR = ROOT / "temp" / "probe" / "tb_frames"
TSV = ROOT / "temp" / "probe" / "ref_calib_fine.tsv"
BATCH = 16
os.environ.setdefault("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")
sys.path.insert(0, str(RUNTIME / "worker"))

import cv2  # noqa: E402
import numpy as np  # noqa: E402

import ocr_worker as w  # noqa: E402

CASES = {
    "glupov": ("quality_bench_test(non-voiced)_11min.mp4",
               "quality_bench_test(non-voiced)_11min_reference.txt"),
    "moon_sisters": ("quality_bench_test(voiced)_5min.mp4",
                     "quality_bench_test(voiced)_5min_reference.txt"),
    "pierro_questions": ("quality_bench_test(voiced)_48min.mp4",
                         "quality_bench_test(voiced)_48min_reference.txt"),
}
REF_FPS = {"glupov": 60000.0 / 1001.0, "moon_sisters": 60.0,
           "pierro_questions": 60000.0 / 1001.0}
TC = re.compile(r"^(\d{2}):(\d{2}):(\d{2}):(\d{2}) - (\d{2}):(\d{2}):(\d{2}):(\d{2})$")


def regions_from_bench(key):
    """从基准的受控副本解析嵌字选区（避免与 common/mod.rs 各存一份而漂移）。"""
    src = (ROOT / "src-tauri" / "tests" / "common" / "mod.rs").read_text(encoding="utf-8")
    body = src.split("pub fn hardsub_regions")[1]
    # 取该函数内本案例的 match 分支
    m = re.search(rf'"{key}" => vec!\[(.*?)\n        \]', body, re.S)
    if not m:
        raise SystemExit(f"未能在 common/mod.rs 找到 {key} 的嵌字选区")
    out = []
    for r in re.finditer(
            r"start:\s*([\d.]+),\s*end:\s*([\d.]+),\s*x1:\s*([\d.]+),\s*y1:\s*([\d.]+),"
            r"\s*x2:\s*([\d.]+),\s*y2:\s*([\d.]+)", m.group(1)):
        out.append(tuple(float(x) for x in r.groups()))
    return out


def parse_ref(path, fps):
    entries, cur = [], None
    for line in path.read_text(encoding="utf-8").splitlines():
        t = line.strip()
        if not t:
            continue
        mm = TC.match(t)
        if mm:
            g = [int(x) for x in mm.groups()]
            cur = {"start": g[0] * 3600 + g[1] * 60 + g[2] + g[3] / fps,
                   "end": g[4] * 3600 + g[5] * 60 + g[6] + g[7] / fps, "lines": []}
            entries.append(cur)
        elif cur is not None:
            cur["lines"].append(t)
    return entries


def video_size(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0",
                          "-show_entries", "stream=width,height", "-of", "csv=p=0", str(path)],
                         capture_output=True, text=True, check=True).stdout.strip()
    wpx, hpx = (int(x) for x in out.split(",")[:2])
    return wpx, hpx


def crop_px(reg, wpx, hpx):
    _, _, x1, y1, x2, y2 = reg
    x, y = round(x1 * wpx), round(y1 * hpx)
    return x, y, max(1, round(x2 * wpx) - x), max(1, round(y2 * hpx) - y)


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


def extract(video, t0, t1, box, tag, step):
    """抽帧 → 内存 BGR ndarray 列表（按时间升序）。"""
    x, y, cw, ch = box
    d = OUT_DIR / tag
    d.mkdir(parents=True, exist_ok=True)
    for f in d.glob("*.png"):
        f.unlink()
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", f"{t0:.3f}", "-i", str(video),
                    "-t", f"{t1 - t0:.3f}", "-vf", f"fps={1 / step:.6f},crop={cw}:{ch}:{x}:{y}",
                    "-fps_mode", "passthrough", "-f", "image2", str(d / "f_%04d.png")], check=True)
    imgs = []
    for f in sorted(d.glob("f_*.png")):
        img = cv2.imread(str(f), cv2.IMREAD_COLOR)
        if img is not None:
            imgs.append(img)
    return imgs


def ocr_batchs(ocr, imgs):
    """批处理 OCR，返回每个输入的管线文本（与 worker 同款后处理）。"""
    texts = []
    for i in range(0, len(imgs), BATCH):
        chunk = imgs[i:i + BATCH]
        for r in ocr.predict(chunk):
            txt, _ = w._extract_result(r)
            texts.append(txt)
    return texts


def bracket(texts, step):
    """区间夹逼：返回 (t_a, t_b, before, after) 或 None。

    before = 窗口前 2 帧中较长者（避免单帧抖动）；t_b = 首个与 before 差异 >0.35
    且其后一帧也差异 >0.35 的帧；t_a = 该帧的前一帧。
    """
    if len(texts) < 4:
        return None
    before = max(texts[:2], key=len)
    bn = norm(before)
    for j in range(1, len(texts) - 1):
        def diff(x):
            xn = norm(x)
            return 1.0 if (bool(xn) != bool(bn)) else lev_ratio(xn, bn)
        if diff(texts[j]) > 0.35 and diff(texts[j + 1]) > 0.35:
            after = max(texts[-2:], key=len)
            return (j - 1) * step, j * step, before, after
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--step", type=float, default=0.05)
    ap.add_argument("--half-window", type=float, default=0.4)
    ap.add_argument("--cases", default=",".join(CASES))
    args = ap.parse_args()

    ocr = w.make_ocr()
    rows = []
    for key in args.cases.split(","):
        clip_name, ref_name = CASES[key]
        clip, ref = DATA / clip_name, DATA / ref_name
        tb = json.loads((TB / f"{key}_timebase.json").read_text(encoding="utf-8"))
        k, a = tb["applied"]["k"], tb["applied"]["a"]
        wpx, hpx = video_size(clip)
        regions = regions_from_bench(key)
        entries = parse_ref(ref, REF_FPS[key])
        print(f"\n########## {key}（{len(entries)} 条参考，视频 {wpx}x{hpx}，"
              f"当前换算 k={k:+.6f} a={a:+.3f}）##########")
        done = skipped = 0
        for ei, e in enumerate(entries, 1):
            reg = next((r for r in regions if r[0] + 1.0 <= e["start"] <= r[1] - 1.0), None)
            if reg is None:
                skipped += 1
                continue
            center = e["start"] * (1 + k) + a          # 预居中到视频时基
            box = crop_px(reg, wpx, hpx)
            half = args.half_window
            imgs = extract(clip, center - half, center + half, box, f"{key}_{ei}", args.step)
            texts = ocr_batchs(ocr, imgs)
            br = bracket(texts, args.step)
            if br is None and half < 1.5:              # 预居中失败 → 放宽一次
                half = 1.5
                imgs = extract(clip, center - half, center + half, box, f"{key}_{ei}", args.step)
                texts = ocr_batchs(ocr, imgs)
                br = bracket(texts, args.step)
            if br is None:
                rows.append({"case": key, "idx": ei, "ref": e["start"], "t_a": None, "t_b": None,
                             "tau": None, "half": None, "before": "", "after": "",
                             "note": "窗口内未测到变化"})
                continue
            t_a, t_b, before, after = br
            base = center - half
            tau = base + (t_a + t_b) / 2
            # 存**全文**（换行折成空格，保持 TSV 单行）：生成器要用完整文本判定"是否实质变化"，
            # 截断会把姓名框+头衔之后真正变化的那一行切掉（glupov 曾因此误排除 10/20 条）。
            rows.append({"case": key, "idx": ei, "ref": e["start"],
                         "t_a": round(base + t_a, 4), "t_b": round(base + t_b, 4),
                         "tau": round(tau, 4), "half": round((t_b - t_a) / 2, 4),
                         "before": " ".join(before.split()),
                         "after": " ".join(after.split()), "note": ""})
            done += 1
            if done % 20 == 0:
                print(f"   … {done} 条（跳过 {skipped}）")
        print(f"  完成 {done} 条，跳过 {skipped} 条（选区外）")

    TSV.parent.mkdir(parents=True, exist_ok=True)
    cols = ["case", "idx", "ref", "t_a", "t_b", "tau", "half", "before", "after", "note"]
    # 增量合并：只替换本次跑过的案例，其余案例的既有行保留（便于单案例重跑）
    ran = set(args.cases.split(","))
    old = []
    if TSV.is_file():
        for line in TSV.read_text(encoding="utf-8").splitlines()[1:]:
            f = line.split("\t")
            if len(f) == len(cols) and f[0] not in ran and f[0] in CASES:
                old.append(f)
    merged = old + [[("" if r[c] is None else str(r[c])) for c in cols] for r in rows]
    merged.sort(key=lambda f: (f[0], int(f[1])))
    with TSV.open("w", encoding="utf-8") as f:
        f.write("\t".join(cols) + "\n")
        for row in merged:
            f.write("\t".join(row) + "\n")
    print(f"\n逐条结果：{TSV}（本次 {len(rows)} 条，合并后共 {len(merged)} 条）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
