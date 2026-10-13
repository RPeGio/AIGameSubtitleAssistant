#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""产品侧融合对齐 worker（一次性子进程）。

    python embed_worker.py <input.json> <output.json>

为什么是「一次性子进程 + 文件交换」而不是常驻 JSON-Lines（`ocr_worker.py` 那种）：
整条流水线在四案例上**含模型加载仅 2.1s**（`scripts/fuse_align_srt.py` 实测），
融合也不是交互式高频操作 ⇒ 常驻进程带来的复杂度（生命周期、取消、超时、僵死清理）
没有任何收益。结果走**输出文件**而不是 stdout：stdout 在中文 Windows 上可能混入
GBK 噪音（`funasr.rs` 已踩过这个坑），文件交换没有这个失效面。

输入 JSON：
    {
      "corpus":   ["语料原文（可含名字行）", ...],
      "segments": [{"index": 1, "start": 0.0, "end": 1.0,
                    "text": "转写原文（未命中时即产物文本）",
                    "model_text": "可选：只用于编码的文本（如带说话人行的副本）"}, ...],
      "unmatch_filter": false,                 # 「该不配」掩码（按素材开关）
      "top_k": 5,                              # 候选名单长度
      "model": "multilingual-e5-small",        # runtime/models/embed/<model>/
      "onnx":  "model_qint8_avx512_vnni.onnx"  # 缺省用 fuse_core.DEFAULT_ONNX
    }

输出 JSON：
    {
      "ok": true, "corpus_size": 17, "segment_count": 15, "elapsed_ms": 2100,
      "model": "...", "onnx": "...", "unmatch_filter": false, "top_k": 5,
      "stats": {"matched": 15, "unmatched": 0, "masked_rows": 0, "masked_cells": 0,
                "reused_rows": 0,
                "transition": {"forward": 15, "repeat": 0, "reset": 0, "unmatched": 0}},
      "segments": [{
         "index": 1, "start": 0.0, "end": 1.0,
         "ocr_index": 3,                  # 1-based 语料下标；-1 = 不配
         "score": 0.9071,                 # 命中行的余弦；不配时为 0.0
         "source_text": "转写原文",         # 融合时刻的转写原文（审计表回退真值）
         "product_text": "语料原文逐字",     # 不配时为输入 text 原文
         "candidates": [[3, 0.9071], ...],# 相似度前 top_k（1-based 语料下标 + 余弦）
         "masked": false,                 # 该段被「该不配」判据整行封死
         "transition": "forward",         # forward / repeat / reset / unmatched
         "reused": false                  # 命中的语料行被 >= 2 段命中
      }, ...]
    }

失败时写出 `{"ok": false, "error": "..."}` 并以退出码 1 结束（traceback 另打 stderr）。

**审计信号全部由这条流水线产出**（不重算、不二次编码）。注意 `score` **不是正确性信号**
（`benchmark/FUSE_THRESHOLD_CALIBRATION.md` §7.9：任何阈值下精确率 ≈ 错误基准率 4.2%，
0.86 只是召回饱和点、标记率 89.4% ⇒ 清单等价「全量复核」），故审计表依赖的是
`ocr_index == -1` / `transition` / `reused` / `masked` 这些**结构化信号**。
"""
import io
import json
import os
import sys
import time
import traceback

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from fuse_core import (  # noqa: E402
    DEFAULT, DEFAULT_EMBED_MODEL, DEFAULT_ONNX, NEG, REPEAT_DEFAULT, RESET_DEFAULT,
    UNMATCH_ALPHA, UNMATCH_TIE_EPS, UNMATCH_TOPK, align_v2, load_embedder,
    mask_empty_body, mask_should_unmatched, similarity_matrix, split_header,
)

DEFAULT_TOP_K = 5


def run(inp):
    t0 = time.time()
    corpus = [str(t) for t in (inp.get("corpus") or [])]
    segs = list(inp.get("segments") or [])
    if not corpus:
        raise ValueError("语料为空：融合需要至少一条可靠文本")
    if not segs:
        raise ValueError("转写段为空：没有可对齐的内容")
    top_k = max(1, int(inp.get("top_k") or DEFAULT_TOP_K))
    use_unmatch = bool(inp.get("unmatch_filter", False))
    model = inp.get("model") or DEFAULT_EMBED_MODEL
    onnx = inp.get("onnx") or None

    # 编码只用正文（表头剥离）；产物文本必须用原文逐字（含名字行）
    corpus_body = [split_header(t)[1] for t in corpus]
    seg_text = [str(s.get("text", "")) for s in segs]
    seg_model = [str(s.get("model_text") or s.get("text", "")) for s in segs]
    seg_body = [split_header(t)[1] for t in seg_model]

    tok, sess = load_embedder(model, onnx)
    S = similarity_matrix(tok, sess, corpus_body, seg_body)

    masked_cells = int(mask_empty_body(S, corpus, seg_model))
    masked_rows = 0
    if use_unmatch:
        masked_rows = int(mask_should_unmatched(S, seg_model, corpus,
                                                alpha=UNMATCH_ALPHA, topk=UNMATCH_TOPK,
                                                tie_eps=UNMATCH_TIE_EPS))

    diag = {}
    match = align_v2(S, *DEFAULT, repeat_penalty=REPEAT_DEFAULT,
                     reset_penalty=RESET_DEFAULT, diag=diag)

    use_count = {}
    for j in match:
        if j >= 0:
            use_count[j] = use_count.get(j, 0) + 1

    rows = []
    prev_j = None
    for i, s in enumerate(segs):
        j = match[i]
        row = S[i]
        cols = [c for c in range(len(corpus)) if row[c] > NEG / 2]   # 排除被掩码封死的列
        cols.sort(key=lambda c: (-float(row[c]), c))
        candidates = [[c + 1, round(float(row[c]), 4)] for c in cols[:top_k]]
        if j >= 0:
            score = float(row[j])
            product_text = corpus[j]
            if prev_j is None or j > prev_j:
                transition = "forward"
            elif j == prev_j:
                transition = "repeat"
            else:
                transition = "reset"
            prev_j = j
            reused = use_count.get(j, 0) >= 2
        else:
            score = 0.0
            product_text = seg_text[i]
            transition = "unmatched"
            reused = False
        rows.append({
            "index": s.get("index", i + 1),
            "start": s.get("start"),
            "end": s.get("end"),
            "ocr_index": (j + 1) if j >= 0 else -1,
            "score": round(score, 6),
            # 融合时刻的转写原文（R3 审计表「不配/替换」的回退真值：
            # 产物时间轴在编辑器里可被拖动/分割，不能用 (start,end) 反查）
            "source_text": seg_text[i],
            "product_text": product_text,
            "candidates": candidates,
            "masked": bool(use_unmatch and not cols),
            "transition": transition,
            "reused": reused,
        })

    return {
        "corpus_size": len(corpus),
        "segment_count": len(segs),
        "model": model,
        "onnx": onnx or os.environ.get("GSA_EMBED_ONNX", DEFAULT_ONNX),
        "unmatch_filter": use_unmatch,
        "top_k": top_k,
        "stats": {
            "matched": sum(1 for j in match if j >= 0),
            "unmatched": sum(1 for j in match if j < 0),
            "masked_rows": masked_rows,
            "masked_cells": masked_cells,
            "reused_rows": sum(1 for c in use_count.values() if c >= 2),
            "transition": {k: int(diag.get(k, 0))
                           for k in ("forward", "repeat", "reset", "unmatched")},
        },
        "segments": rows,
        "elapsed_ms": int((time.time() - t0) * 1000),
    }


def main(argv):
    if len(argv) != 3:
        sys.stderr.write("usage: embed_worker.py <input.json> <output.json>\n")
        return 2
    in_path, out_path = argv[1], argv[2]
    try:
        with io.open(in_path, encoding="utf-8-sig") as f:
            inp = json.load(f)
        out = run(inp)
        out["ok"] = True
        rc = 0
    except Exception as e:                                   # noqa: BLE001
        traceback.print_exc()
        out = {"ok": False, "error": "{}: {}".format(type(e).__name__, e)}
        rc = 1
    d = os.path.dirname(os.path.abspath(out_path))
    if d and not os.path.isdir(d):
        os.makedirs(d)
    with io.open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(out, ensure_ascii=False, indent=2) + "\n")
    print("embed_worker ok={} segments={} elapsed_ms={}".format(
        out.get("ok"), out.get("segment_count", 0), out.get("elapsed_ms", 0)))
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv))
