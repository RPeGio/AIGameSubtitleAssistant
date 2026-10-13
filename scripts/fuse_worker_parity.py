#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""门③：产品 worker（`scripts/embed_worker.py`）与实验台产物**逐段对拍**（R1 验收）。

对拍双方：
  · 产品：`embed_worker.py`，以**一次性子进程**方式调用（与 Rust 侧将来的调用方式一致）
  · 实验台：`scripts/fuse_align_srt.py` 的 `run()`（已验证路线：表头剥离 + `mask_empty_body`
    + 可选 `mask_should_unmatched` + `align_v2`，四案例 226/236 的同一实现）

两种口径各跑一遍（共 8 组）：
  · `product`：worker 的 `unmatch_filter = fuse_align_srt.UNMATCH_PER_CASE[case]`（产品配置）
  · `off`    ：worker 的 `unmatch_filter = False`（实验室默认；对 vesna 是强制关）

比对字段：`ocr_index`（实验台用 0 表示"不配"，worker 用 -1）、`score`（实验台 4 位小数）、
`product_text`、`candidates`（先把实验台里被掩码封死的列 score ≈ -1e9 剔除再比——worker
只输出合法候选）。

用法：python scripts/fuse_worker_parity.py   （退出码 0 = 零差异）
环境：numpy/onnxruntime 装在 runtime/deps_embed，直接跑需先
    $env:PYTHONPATH="<仓库根>\\runtime\\deps_embed"（产品侧由 Rust 传入，见 embed.rs）
"""
import io
import json
import os
import subprocess
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)
sys.path.insert(0, os.path.join(_HERE, "..", "src-tauri", "tests"))
sys.dont_write_bytecode = True

import fuse_align_srt as fas  # noqa: E402
from fuse_lab import BENCH_OUT, collect_game_content, load_embedder, load_project  # noqa: E402

WORKER = os.path.join(_HERE, "embed_worker.py")
OUT_DIR = os.path.join(BENCH_OUT, "parity")
NEG_ISH = -1e8


def worker_input(case, unmatch):
    proj = load_project(fas.PROJECTS[case])
    corpus = [c["text"] for c in proj["corpus"]]
    gc = collect_game_content(proj)
    segs = [{"index": i + 1, "start": g["start"], "end": g["end"], "text": g["text"]}
            for i, g in enumerate(gc)]
    return {"corpus": corpus, "segments": segs, "unmatch_filter": unmatch, "top_k": fas.TOP_K}


def run_worker(case, tag, inp):
    os.makedirs(OUT_DIR, exist_ok=True)
    ip = os.path.join(OUT_DIR, "in_{}_{}.json".format(tag, case))
    op = os.path.join(OUT_DIR, "worker_{}_{}.json".format(tag, case))
    with io.open(ip, "w", encoding="utf-8", newline="\n") as f:
        json.dump(inp, f, ensure_ascii=False, indent=1)
    r = subprocess.run([sys.executable, WORKER, ip, op],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode != 0:
        raise SystemExit("worker 失败（{} {}）：rc={}\n{}\n{}".format(
            tag, case, r.returncode, r.stdout, r.stderr))
    with io.open(op, encoding="utf-8") as f:
        return json.load(f)


def run_lab(case, tag, tok, sess):
    os.environ.pop("GSA_UNMATCH_FILTER", None)
    if tag == "off":
        os.environ["GSA_UNMATCH_FILTER"] = "0"
    d = os.path.join(OUT_DIR, "lab_{}_{}".format(tag, case))
    fas.run(case, tok, sess, d)
    with io.open(os.path.join(d, "fuse_srt_{}.json".format(case)), encoding="utf-8") as f:
        return json.load(f)


def compare(lab, work):
    diffs = []
    L, W = lab["rows"], work["segments"]
    if len(L) != len(W):
        diffs.append("段数不同: lab {} vs worker {}".format(len(L), len(W)))
    if lab["matched"] != work["stats"]["matched"]:
        diffs.append("matched 不同: lab {} vs worker {}".format(
            lab["matched"], work["stats"]["matched"]))
    for k, (a, b) in enumerate(zip(L, W)):
        where = "段{}".format(k + 1)
        exp_idx = a["ocr_index"] if a["ocr_index"] else -1
        if exp_idx != b["ocr_index"]:
            diffs.append("{} ocr_index: lab {} vs worker {}".format(where, exp_idx, b["ocr_index"]))
        if abs(float(a["score"]) - float(b["score"])) > 5.1e-5:
            diffs.append("{} score: lab {} vs worker {}".format(where, a["score"], b["score"]))
        if a["product_text"] != b["product_text"]:
            diffs.append("{} product_text: lab {!r} vs worker {!r}".format(
                where, a["product_text"], b["product_text"]))
        lab_c = [list(c) for c in a["cands"] if float(c[1]) > NEG_ISH]
        work_c = [list(c) for c in b["candidates"]]
        if lab_c != work_c:
            diffs.append("{} candidates: lab {} vs worker {}".format(where, lab_c, work_c))
    return diffs


def main():
    tok, sess = load_embedder()
    total = bad = 0
    for case in fas.CASES:
        for tag in ("product", "off"):
            unmatch = fas.UNMATCH_PER_CASE.get(case, False) if tag == "product" else False
            work = run_worker(case, tag, worker_input(case, unmatch))
            lab = run_lab(case, tag, tok, sess)
            diffs = compare(lab, work)
            total += 1
            if diffs:
                bad += 1
                print("[DIFF] {} {}：{} 处差异".format(case, tag, len(diffs)))
                for d in diffs[:10]:
                    print("   " + d)
            else:
                print("[OK]   {} {}：{} 段（matched {}、不配 {}），逐段一致".format(
                    case, tag, len(lab["rows"]), lab["matched"], work["stats"]["unmatched"]))
    print("=== {} 组口径，{} 组有差异 ===".format(total, bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
