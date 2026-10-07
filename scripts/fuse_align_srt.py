#!/usr/bin/env python3
"""产品形态的融合对齐 → SRT（供人工检验效果）。

这是 [benchmark/FUSE_ALIGNMENT_SCALE_VALIDATION.md](../benchmark/FUSE_ALIGNMENT_SCALE_VALIDATION.md)
与 [benchmark/FUSE_THRESHOLD_CALIBRATION.md](../benchmark/FUSE_THRESHOLD_CALIBRATION.md) 所验证路线的
**可执行产品形态**：多语言向量召回 + 单调 DP 做跨语言对齐，把语料文本替换到转写侧的时间轴上。

流程（对应 FUSE_VECTOR_RECALL_VALIDATION.md §三 的产品形态）：

    ① 语料 corpus → **剥表头（仅编码用）** → E5 编码
    ② 转写侧段 → 剥表头 → 编码 → 召回
    ③ 统一转移 DP（前进 / 复用 / 回退 / 不配；skip=0.02 / unmatched=0.25 /
       repeat=0.25 / reset=**禁用**）
    ④ 一对多：同一语料行被多段命中 = 同句的多次显示，由**复用转移**表达（不再靠输入侧合并）
    ⑤ 低置信（score<0.78）或未命中 → 待人工确认清单
    ⑥ 产物：命中段 = **语料原文逐字（含名字行）**；未命中段 = 转写原文；时间轴沿用转写侧

**转移模型（T4c 选项 i）**：把"序"与"重数"两个假设分开——
`前进`（严格递增，现行）、`复用`（同一下标，有界一对多，`repeat_penalty`）、
`回退`（下标回退，拖进度条重看，**罚分设为 ∞ 即禁用**）、`不配`。
`repeat=0.25` 实测把 pierro 从 **95/121 → 119/121**，而 moon/glupov **零回归**；
`reset` 禁用是因为现有三案例真值里**没有任何真实顺序回退**，该路径无素材可验。

**本脚本不做输入卫生**（"同一条字幕被 OCR 拆成两段"的合并）——那属 OCR 合并层的
raw 缺陷，在管线层修（见下方常量区的长注释与 benchmark/OCR_PIPELINE_DEFECTS.md D12）。
本脚本只消费管线产出；产物轨干净与否由管线负责。

两条不可动摇的约定（见 src-tauri/tests/fuse_lab/README.md）：

  · **表头剥离只用于模型输入**：产物文本必须是语料原文逐字（含名字行）。
    表头在段间完全同形会淹没正文语义——glupov 带表头时召回 3/22，剥离后 22/22。
  · **单调性必须真正强制**：曾因"无对应"分支沿用 argmax 前驱导致下标回退/回绕。

阈值说明（T2 实测结论）：`skip_penalty=0.02` / `unmatched_penalty=0.25` 已在最优平台上，**不需调整**；
告警只用 `score<0.78`（`margin<0.02` 已实测无区分度，标记率 63.5% 而精确率 ≤12%，故**不采用**）。

用法：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    python scripts/fuse_align_srt.py                 # 三个案例
    python scripts/fuse_align_srt.py pierro          # 指定案例

产物（默认 temp/bench_output/）：
    fuse_srt_<case>.srt        —— 最终字幕（BOM + CRLF，与产品 export 一致）
    fuse_srt_<case>.todo.txt   —— 待人工确认清单
    fuse_srt_<case>.json       —— 逐段判定表（含分数、候选、命中语料下标）
"""
import io
import json
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(_HERE, "..", "src-tauri", "tests"))
sys.path.insert(0, _HERE)

import numpy as np  # noqa: E402
from fuse_lab import (BENCH_OUT, collect_game_content, load_embedder,  # noqa: E402
                      load_project, similarity_matrix, split_header, write_srt)
from fuse_calib import (DEFAULT, REPEAT_DEFAULT, RESET_DEFAULT,  # noqa: E402
                        align_v2, mask_empty_body)

# 案例 → 工程（转写侧由 collect_game_content 按产品口径收集：game 轨 ASR + 嵌字）
CASES = ["moon", "glupov", "pierro"]
PROJECTS = {
    "moon": "moon_sisters.gsa",
    "glupov": "glupov.gsa",
    "pierro": "pierro_questions.gsa",
}
# 告警阈值：只保留 score（T2 实测 margin 无区分度，故不使用）
THR_SCORE = 0.78
TOP_K = 5

# 注：本脚本**不做"互为前缀的相邻段"合并**。
#
# 那类"同一条字幕被 OCR 拆成两段"属 **OCR 合并层的 raw 缺陷**，应在管线层修
# （`src-tauri/src/ocr/merge.rs` 的 `is_stable_line_boundary_cut`），本脚本只消费
# 管线产出。曾在此处加过一层文本判据的合并，**已被证否并删除**：
#
#   benchmark/OCR_PIPELINE_DEFECTS.md **D12** 证明，正当用例（glupov #17：姓名框的
#   省略号行 conf=0.000 被 CONF_THRESHOLD 丢弃）与误伤用例（pierro：对话行正在打字）
#   在 (短态, 长态) 文本上**完全同形**，时长区间也重叠 ⇒ **任何只依赖该文本对的判据
#   不可能两全**。实测该实现会**误并** pierro 的 `The Jester / …`（1.50s，语料里
#   `[17]「丑角」···` 与 `[18]「丑角」我尊重你的意见…` 是两条独立条目）。
#
# 故输入卫生的**唯一归属是 OCR 合并层**；修好后重新生成产物轨，本脚本即得到干净输入。


def run(case, tok, sess, out_dir):
    proj = load_project(PROJECTS[case])
    corpus = [c["text"] for c in proj["corpus"]]
    gc = collect_game_content(proj)
    if not gc:
        print("  {}：无转写侧段，跳过".format(case))
        return None

    # ① / ② 编码：表头剥离**只作用于编码**（产物用原文）
    cb = [split_header(t)[1] for t in corpus]
    sb = [split_header(g["text"])[1] for g in gc]
    S = similarity_matrix(tok, sess, cb, sb)
    # 吸引子防护：空正文语料条目不得被有正文的段命中（与 fuse_calib 同源实现）
    mask_empty_body(S, corpus, [g["text"] for g in gc])

    # ③ 单调 DP
    match = align_v2(S, *DEFAULT, repeat_penalty=REPEAT_DEFAULT,
                     reset_penalty=RESET_DEFAULT)

    # ④⑤⑥ 组装产物
    rows, todo = [], []
    matched = 0
    for i, g in enumerate(gc):
        j = match[i]
        order = sorted(range(len(corpus)), key=lambda x: -S[i, x])[:TOP_K]
        score = float(S[i, j]) if j >= 0 else 0.0
        hit = j >= 0
        if hit:
            matched += 1
        text = corpus[j] if hit else g["text"]
        rows.append({
            "start": g["start"], "end": g["end"],
            "product_text": text,
            "ocr_index": j + 1 if hit else 0,
            "score": round(score, 4),
            "gc": g["text"],
            "cands": [(c + 1, round(float(S[i, c]), 4)) for c in order],
        })
        if not hit or score < THR_SCORE:
            todo.append({
                "start": g["start"], "end": g["end"],
                "reason": "未命中" if not hit else "低置信(score={:.3f})".format(score),
                "gc": g["text"], "product_text": text,
                "cands": [(c + 1, round(float(S[i, c]), 4)) for c in order],
            })

    # 产物落盘
    os.makedirs(out_dir, exist_ok=True)
    srt = os.path.join(out_dir, "fuse_srt_{}.srt".format(case))
    write_srt(rows, srt, text_key="product_text")
    with io.open(os.path.join(out_dir, "fuse_srt_{}.json".format(case)), "w",
                 encoding="utf-8", newline="") as f:
        json.dump({"case": case, "project": PROJECTS[case], "corpus": corpus,
                   "segments": len(gc),
                   "matched": matched, "todo": len(todo),
                   "rows": rows, "todo_rows": todo}, f, ensure_ascii=False, indent=1)
    tp = os.path.join(out_dir, "fuse_srt_{}.todo.txt".format(case))
    with io.open(tp, "w", encoding="utf-8", newline="") as f:
        f.write("待人工确认清单 —— {}\n".format(case))
        f.write("（低置信 score<{:.2f} 或未命中；阈值见 benchmark/FUSE_THRESHOLD_CALIBRATION.md）\n\n".format(THR_SCORE))
        for t in todo:
            f.write("[{:8.2f} -> {:8.2f}] {}\n".format(t["start"], t["end"], t["reason"]))
            f.write("   转写: {}\n".format(t["gc"].replace("\n", " / ")))
            f.write("   产物: {}\n".format(t["product_text"].replace("\n", " / ")))
            f.write("   候选: {}\n\n".format(
                ", ".join("OCR[{}]={:.3f}".format(c, s) for c, s in t["cands"])))

    print("  {:<8} 语料 {:3} 条 | 转写 {:3} 段 | 已配 {:3}/{:3} | 待确认 {:2}".format(
        case, len(corpus), len(gc), matched, len(gc), len(todo)))
    print("           （「已配」= DP 给出了语料下标，**不等于正确**；正确率见 fuse_calib.py）")
    print("           → {}".format(srt))
    return {"case": case, "corpus": len(corpus), "segments": len(gc),
            "matched": matched, "todo": len(todo)}


def main():
    want = [a for a in sys.argv[1:] if not a.startswith("-")] or CASES
    out_dir = os.environ.get("GSA_BENCH_OUT_DIR", BENCH_OUT)
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    print("=" * 100)
    print("融合对齐 → SRT（向量召回 + 单调 DP；表头剥离仅用于编码，产物用语料原文）")
    res = []
    for case in want:
        r = run(case, tok, sess, out_dir)
        if r:
            res.append(r)
    if res:
        print()
        print("── 合计 ──")
        print("  段 {} | 已配 {} | 待确认 {}".format(
            sum(r["segments"] for r in res), sum(r["matched"] for r in res),
            sum(r["todo"] for r in res)))
    print()
    print("产物目录：{}".format(out_dir))
    return 0


if __name__ == "__main__":
    sys.exit(main())
