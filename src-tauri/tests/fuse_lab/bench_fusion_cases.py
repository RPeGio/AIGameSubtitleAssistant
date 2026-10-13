# -*- coding: utf-8 -*-
"""判别器 A：LLM 是否具备跨语言语义对齐能力（干净用例，恒等基线=0）。

跑真实 `llama-cli`（argv 与 `ai_runtime/llm.rs::run_complete` 一致），
prompt 与 `fuse/mod.rs::build_prompt` 的契约保持同步（文本原样入 prompt + 条数上限）。
判读：错序用例正确率 ≈ 随机 ⇒ 只是按下标对齐；>> 随机 ⇒ 具备内容匹配。

用法：
    $env:PYTHONPATH = "<repo>\\runtime\\deps_embed"
    python src-tauri/tests/fuse_lab/bench_fusion_cases.py [reps]
环境变量：GSA_BENCH_LLM_MODEL（换模型，相对 runtime/）、GSA_BENCH_OUT_DIR
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from fuse_lab import (BATCH_SIZE, BENCH_OUT, build_cases, identity_baseline,  # noqa: E402
                      is_deranged_case, parse_fusion_output, run_llama, to_index, write_json)

# 与 Rust build_prompt 指令段保持一致（改 Rust 侧须同步此处）
INSTR = (
    "你是游戏字幕融合助手。下面是可靠的剧情字幕文本（OCR）和游戏内容时间轴文本（GC，"
    "来源可为游戏语音转写或画面内嵌字幕 OCR，语言可能与字幕不同）。\n"
    "请把每条 GC 与语义相同的 OCR 字幕对应（跨语言对应）：找到对应就填该 OCR 字幕的编号，找不到就填 0。\n"
    "条目可能不止一行（首行常是角色名或说话人标签），条目一律按行首的 OCR[编号] / GC[编号] 标记划分。\n"
    "严格只输出 JSON，严禁输出任何其他内容，格式："
    "{\"segments\":[{\"index\":GC编号,\"ocr_index\":OCR编号或0}]}\n"
    "index 和 ocr_index 都是纯数字（如 17），不要写成 \"GC[17]\"。\n"
    "示例（GC[3] 与 OCR[1] 语义相同）：{\"segments\":[{\"index\":3,\"ocr_index\":1}]}\n\n"
)


def build_prompt(corpus, gc_texts):
    """复刻 build_prompt：文本原样（保留换行）+ 显式条数上限"""
    p = [INSTR]
    p.append("本次 GC 共 {} 条：最多只输出 {} 条，不要输出 GC 编号以外的内容。\n\n".format(
        len(gc_texts), len(gc_texts)))
    p.append("== 字幕文本（OCR）==\n")
    for i, t in enumerate(corpus):
        p.append("OCR[{}] {}\n".format(i + 1, t.strip()))
    p.append("\n== 游戏内容时间轴文本（GC）==\n")
    for i, t in enumerate(gc_texts):
        p.append("GC[{}] {}\n".format(i + 1, t.strip()))
    return "".join(p)


def score(raw, truth, n_corpus):
    segs, mode = parse_fusion_output(raw)
    if segs is None:
        return None, mode, 0, 0
    got = {}
    for s in segs:
        try:
            got[to_index(s.get("index"))] = to_index(s.get("ocr_index"))
        except ValueError:
            continue
    ok = sum(1 for i, w in enumerate(truth) if w and got.get(i + 1) == w)
    ident = sum(1 for i in range(len(truth)) if got.get(i + 1) == i + 1)
    total = sum(1 for w in truth if w)
    return ok, mode, ident, total


def main():
    reps = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    out_dir = os.environ.get("GSA_BENCH_OUT_DIR", BENCH_OUT)
    rows = []
    print("{:28} {:>4} {:>14} {:>9} {:>9} {:>7}".format(
        "用例", "rep", "内容正确", "恒等基线", "随机%", "输出恒等"))
    for name, corpus, gc_texts, truth in build_cases():
        idn_base = identity_baseline(truth)
        total = sum(1 for w in truth if w)
        if is_deranged_case(name):
            assert idn_base == 0, "{}：恒等基线≠0，判别器失效".format(name)
        for rep in range(1, reps + 1):
            prompt = build_prompt(corpus, gc_texts)
            raw = run_llama(prompt)
            ok, mode, ident, tot = score(raw, truth, len(corpus))
            if ok is None:
                print("{:28} {:>4} {:>14} {:>9} {:>9} {:>7}".format(
                    name, rep, "解析失败({})".format(mode), idn_base, "-", "-"))
                rows.append({"case": name, "rep": rep, "parse": mode, "correct": 0,
                             "total": total, "identity_baseline": idn_base})
                continue
            print("{:28} {:>4} {:>14} {:>9} {:>9.1f} {:>7}".format(
                name, rep, "{}/{}".format(ok, tot), idn_base,
                100.0 / max(len(corpus), 1), "{}/{}".format(ident, len(truth))))
            rows.append({"case": name, "rep": rep, "parse": mode, "correct": ok,
                         "total": total, "identity_baseline": idn_base,
                         "identity_output": ident, "chance_pct": 100.0 / max(len(corpus), 1)})
    der = [r for r in rows if is_deranged_case(r["case"])]
    if der:
        ok = sum(r["correct"] for r in der)
        tot = sum(r["total"] for r in der)
        exp = sum(r["total"] * r.get("chance_pct", 0) / 100.0 for r in der)
        print("\n错序用例合计 {}/{}（随机期望 {:.0f}）".format(ok, tot, exp))
    write_json(os.path.join(out_dir, "fuse_lab_cases.json"),
               {"argv_args": "run_complete 同 argv", "rows": rows})
    print("结果落盘：{}/fuse_lab_cases.json".format(out_dir))


if __name__ == "__main__":
    main()
