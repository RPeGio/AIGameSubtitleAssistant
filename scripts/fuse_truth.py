#!/usr/bin/env python3
"""为三个案例推导**独立真值**（T2 阈值校准的立足点）。

方法（与 benchmark/FUSE_ALIGNMENT_SCALE_VALIDATION.md §三 一致，此处泛化到 3 案例）：
    转写侧段（英文，带时间轴）
        --IoU-->  参考文本块（中文，带时间窗；= 该时刻实际播出的台词）
        --文本匹配-->  语料行（中文；= 作品全集中的同一条）

为什么必须用参考文本而不是 moon/glupov 现成的「15/15、22/22」：
那两组数字是**对齐输出本身**经用户抽查认可的结果，拿它当期望值是循环论证。
参考文本是独立于对齐算法的第三方记录（时间窗 + 台词），故可作真值来源。

三个案例与参考文本的对应（1:1，见各自 .gsa 的 video 字段）：

| 工程 | 视频 | 参考文本 | 转写侧 |
|---|---|---|---|
| moon_sisters.gsa | (voiced)_5min.mp4 | (voiced)_5min_reference.txt | ASR 15 段（game 轨） |
| glupov.gsa | (non-voiced)_11min.mp4 | (non-voiced)_11min_reference.txt | embed_ocr 22 段 |
| pierro_questions.gsa | (voiced)_48min.mp4 | (voiced)_48min_reference.txt | embed_ocr 120 段 |

用法：python scripts/fuse_truth.py [案例名 ...]   （默认全部）
产物：temp/bench_output/truth_<case>.json
"""
import difflib
import io
import json
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "tests"))
from fuse_lab import BENCH_OUT, EXAMPLES, collect_game_content, load_project  # noqa: E402

CASES = [
    {"key": "moon", "project": "moon_sisters.gsa",
     "reference": "quality_bench_test(voiced)_5min_reference.txt"},
    {"key": "glupov", "project": "glupov.gsa",
     "reference": "quality_bench_test(non-voiced)_11min_reference.txt"},
    {"key": "pierro", "project": "pierro_questions.gsa",
     "reference": "quality_bench_test(voiced)_48min_reference.txt"},
]

WEAK = 0.72
SENT_END = "。！？…!?."
_ONLY_PUNCT = set("…~·．,，。！？!?、；;:：-—「」[]【】（）()\"'’‘“” \n\t")


# ────────────────────────── 文本工具 ──────────────────────────

def norm(s):
    return re.sub(r"[\s「」\[\]【】（）()〈〉《》『』、。，！？…~·．,\.!\?\"'’‘“”—\-]", "", s)


def body_of(corpus_line):
    """语料条目的正文（按 split_header 同规则去名字行/头衔行）"""
    lines = [l.strip() for l in corpus_line.split("\n")]
    lines = [l for l in lines if l]
    while len(lines) > 1:
        h = lines[0]
        if len(h) <= 16 and not any(c in h for c in SENT_END):
            lines.pop(0)
        else:
            break
    return "\n".join(lines).strip() or corpus_line.strip()


def is_namebox_only(text):
    """整段是否只是名字（名牌）+ 省略号/标点，没有正文台词。

    判据用「**末行只有标点/省略号**」而不是"剥掉前导短行后无正文"：
    后者依赖 16 字符的短行门，而**英文**名牌/头衔远长于 16 字符
    （glupov 的 `Former Acting Captain,"Ninth Company` 长 35），会漏判。
    单行情形（如 `[丑角】…`、裸名字 `米晨亚`）另按"整行只有标点或很短"处理。
    """
    lines = [l.strip() for l in text.split("\n") if l.strip()]
    if not lines:
        return False
    if len(lines) == 1:
        return all(c in _ONLY_PUNCT for c in lines[0]) or len(lines[0]) <= 12
    return all(c in _ONLY_PUNCT for c in lines[-1])


def name_key(s):
    return "".join(c for c in s if c not in _ONLY_PUNCT)


def first_name(text):
    """条目名 = 首行（名牌态下首行就是名字）"""
    for l in text.split("\n"):
        if l.strip():
            return l.strip()
    return ""


def overlap(a0, a1, b0, b1):
    return max(0.0, min(a1, b1) - max(a0, b0))


# ────────────────────────── 参考文本 ──────────────────────────

def parse_reference(path):
    """→ [{t0,t1,speaker,head,text,raw}]

    首行为时间码；其后**前导的短行且结尾非句末标点**视为说话人/头衔（可多行，
    如 glupov 的「名字 + 头衔」两行），其余为台词。
    不能简单假定 body[0] 就是说话人：moon 的开场/收场旁白只有一行正文、没有说话人，
    按旧规则会把正文当说话人吃掉 ⇒ text 变空、匹配得分 0（实测两处）。
    """
    raw = io.open(path, encoding="utf-8-sig").read().replace("\r\n", "\n")
    out = []
    for b in [x.strip() for x in raw.split("\n\n") if x.strip()]:
        lines = [l for l in b.split("\n") if l.strip()]
        if not lines:
            continue
        m = re.match(r"(\d+):(\d+):(\d+):(\d+)\s*-\s*(\d+):(\d+):(\d+):(\d+)", lines[0])
        if not m:
            continue
        g = [int(x) for x in m.groups()]

        def sec(v):
            return v[0] * 3600 + v[1] * 60 + v[2] + v[3] / 100.0

        body = lines[1:]
        head = []
        while len(body) > 1:
            h = body[0]
            if len(h) <= 16 and not h.endswith(tuple(SENT_END)):
                head.append(body.pop(0))
            else:
                break
        # 只剩一行时：短且结尾非句末标点 ⇒ 是说话人（台词为空，如纯名牌块）
        if len(body) == 1 and len(body[0]) <= 16 and not body[0].endswith(tuple(SENT_END)):
            head.append(body.pop(0))
        out.append({"t0": sec(g[:4]), "t1": sec(g[4:]),
                    "speaker": head[0] if head else "",
                    "head": head,
                    "text": " ".join(body),
                    "raw": " ".join(lines[1:])})
    return sorted(out, key=lambda r: r["t0"])


# ────────────────────────── 真值构建 ──────────────────────────

def build_truth(case, verbose=True):
    proj = load_project(case["project"])
    corpus = [c["text"] for c in proj["corpus"]]
    cbody = [norm(body_of(t)) for t in corpus]
    gc = collect_game_content(proj)
    refs = parse_reference(os.path.join(EXAMPLES, case["reference"]))

    # 参考块 → 语料行（文本匹配）
    # 主判据：**正文 vs 正文**（两侧都按同一规则剥掉名字行/头衔行），避免同形表头
    # 干扰匹配。仅当主判据弱时才回退到「整条原文 vs 整条原文」——用于正文退化的
    # 情形（正文只剩省略号、或标题被当作正文，如 glupov 的 `安东 / 原「第九连队」
    # 临时连长 / ……`）。
    raw_corpus = [norm(t) for t in corpus]
    ref_to_corpus = []
    for r in refs:
        rb = norm(r["text"])
        best_j, best_score = -1, 0.0
        for j, cb in enumerate(cbody):
            if not cb or not rb:
                continue
            score = difflib.SequenceMatcher(None, rb, cb).ratio()
            if len(rb) >= 6 and rb in cb:
                score = max(score, 0.97)
            elif len(cb) >= 6 and cb in rb:
                score = max(score, 0.95)
            if score > best_score:
                best_j, best_score = j, score
        if best_score < WEAK:
            rr = norm(r["raw"])
            for j, cr in enumerate(raw_corpus):
                if not cr or not rr:
                    continue
                score = difflib.SequenceMatcher(None, rr, cr).ratio()
                if len(rr) >= 6 and rr in cr:
                    score = max(score, 0.97)
                elif len(cr) >= 6 and cr in rr:
                    score = max(score, 0.95)
                if score > best_score:
                    best_j, best_score = j, score
        ref_to_corpus.append({"ref": r, "j": best_j, "score": round(best_score, 3)})

    # 转写段 → 参考块：**时长比 + λ·重叠比**（λ=0.3）
    #
    # 单一判据都会错，实测 157 段里有 5 处两判据分歧（逐条读文本裁定）：
    #   · 纯重叠：段跨边界时判给更长的那一块 —— glupov 段21、pierro 段106/116 判错；
    #   · 纯时长：相邻块时长接近时判给下一块 —— glupov 段2（9.03s vs 8.93/9.05）判错；
    #   · 名牌段（glupov 段18）：应配"……"块（时长近），而重叠会配到相邻台词。
    # 合并后两个信号互补：时长定位"哪一条字幕"，重叠在时长接近时打破平局。
    # λ 的可行区间由这 5 例夹出：(0.0084, 0.558)，取 0.3。
    LAMBDA_OVERLAP = 0.3
    rows = []
    for i, g in enumerate(gc):
        dur = g["end"] - g["start"]
        best, best_ov, best_score = None, 0.0, -1.0
        for x in ref_to_corpus:
            r = x["ref"]
            ov = overlap(g["start"], g["end"], r["t0"], r["t1"])
            if ov <= 0:
                continue
            rdur = r["t1"] - r["t0"]
            dr = min(dur, rdur) / max(dur, rdur) if max(dur, rdur) > 0 else 0.0
            score = dr + LAMBDA_OVERLAP * (ov / max(dur, 1e-9))
            if score > best_score:
                best, best_ov, best_score = x, ov, score
        if best is None:   # 无任何重叠：退回最大重叠（含 0）
            for x in ref_to_corpus:
                r = x["ref"]
                ov = overlap(g["start"], g["end"], r["t0"], r["t1"])
                if ov > best_ov:
                    best, best_ov = x, ov
        rows.append({
            "index": i + 1, "kind": g["kind"],
            "start": g["start"], "end": g["end"], "dur": round(dur, 2),
            "text": g["text"],
            "ref_t0": best["ref"]["t0"] if best else None,
            "ref_t1": best["ref"]["t1"] if best else None,
            "ref_speaker": best["ref"]["speaker"] if best else "",
            "ref_text": best["ref"]["text"] if best else "",
            "ref_score": best["score"] if best else 0.0,
            "ref_dur_ratio": round(best_score, 3),
            "time_cov": round(best_ov / max(dur, 1e-9), 3),
            "truth": (best["j"] + 1) if best else 0,
            "weak_ref": bool(best and best["score"] < WEAK),
            "namebox_fixed": False,
        })

    # 纯名字框段归位：名牌段应配"同样是名牌"的语料条目，而不是相邻台词。
    # 语料侧的名字取**首行**（名牌态下首行就是名字），参考侧用参考块的说话人。
    name_only = {}
    for j, t in enumerate(corpus, 1):
        if is_namebox_only(t):
            k = name_key(first_name(t))
            if k and k not in name_only:
                name_only[k] = j
    fixed = []
    for r in rows:
        if not is_namebox_only(r["text"]) or not r["ref_speaker"]:
            continue
        j = name_only.get(name_key(r["ref_speaker"]))
        if j and j != r["truth"]:
            fixed.append((r["index"], r["truth"], j, r["text"], r["ref_speaker"]))
            r["truth"] = j
            r["namebox_fixed"] = True

    weak = [x for x in ref_to_corpus if x["score"] < WEAK]
    no_ref = [r for r in rows if not r["ref_text"]]
    groups = {}
    for r in rows:
        if r["truth"] > 0:
            groups.setdefault(r["truth"], []).append(r["index"])
    multi = {k: v for k, v in groups.items() if len(v) > 1}

    if verbose:
        print("=" * 96)
        print("{}（{}）".format(case["key"], case["project"]))
        print("  语料 {} 条 / 转写 {} 段（{}）/ 参考块 {} 块".format(
            len(corpus), len(gc),
            "+".join(sorted({g["kind"] for g in gc})) or "-", len(refs)))
        print("  参考块→语料：强 {} / 弱 {}（阈值 {:.2f}）".format(
            len(ref_to_corpus) - len(weak), len(weak), WEAK))
        for x in weak:
            r = x["ref"]
            print("    弱 [{:7.1f}-{:7.1f}] score={:.3f} j={} | 参考 {!r}".format(
                r["t0"], r["t1"], x["score"], x["j"] + 1, r["raw"][:44]))
        print("  段→参考块：无参考 {} / 时间覆盖<50% {}".format(
            len(no_ref), sum(1 for r in rows if r["ref_text"] and r["time_cov"] < 0.5)))
        if fixed:
            print("  纯名字框归位 {} 处：".format(len(fixed)))
            for idx, old, new, txt, sp in fixed:
                print("    段{:3} {} → {}（说话人 {!r}）| {}".format(
                    idx, old, new, sp, txt.replace("\n", " / ")[:34]))
        print("  真值>0 {}/{}；覆盖语料 {}/{}；一对多 {} 组涉及 {} 段".format(
            sum(1 for r in rows if r["truth"] > 0), len(rows),
            len(groups), len(corpus), len(multi), sum(len(v) for v in multi.values())))
        for k in sorted(multi):
            print("    语料[{:3}] ← 段 {}".format(k, multi[k]))

    return {
        "key": case["key"], "project": case["project"], "reference": case["reference"],
        "corpus": corpus, "segments": len(gc), "corpus_lines": len(corpus),
        "ref_blocks": len(refs),
        "summary": {
            "ref_strong": len(ref_to_corpus) - len(weak), "ref_weak": len(weak),
            "no_ref": len(no_ref),
            "low_cov": sum(1 for r in rows if r["ref_text"] and r["time_cov"] < 0.5),
            "truth_pos": sum(1 for r in rows if r["truth"] > 0),
            "corpus_covered": len(groups),
            "multi_groups": len(multi), "multi_segments": sum(len(v) for v in multi.values()),
            "namebox_fixed": len(fixed),
        },
        "rows": rows,
        "ref_to_corpus": [{"t0": x["ref"]["t0"], "t1": x["ref"]["t1"],
                           "speaker": x["ref"]["speaker"], "text": x["ref"]["text"],
                           "corpus_index": x["j"] + 1, "score": x["score"]}
                          for x in ref_to_corpus],
    }


def main():
    want = [a for a in sys.argv[1:] if not a.startswith("-")]
    cases = [c for c in CASES if not want or c["key"] in want]
    os.makedirs(BENCH_OUT, exist_ok=True)
    out = {}
    for c in cases:
        T = build_truth(c)
        p = os.path.join(BENCH_OUT, "truth_{}.json".format(c["key"]))
        with io.open(p, "w", encoding="utf-8", newline="") as f:
            json.dump(T, f, ensure_ascii=False, indent=1)
        out[c["key"]] = T["summary"]
        print("  已落盘 {}".format(p))
    print()
    print("── 汇总 ──")
    print("  {:<8} {:>6} {:>6} {:>7} {:>6} {:>8} {:>9}".format(
        "案例", "语料", "转写", "参考块", "弱匹配", "一对多组", "覆盖语料"))
    for k, s in out.items():
        T = json.load(io.open(os.path.join(BENCH_OUT, "truth_{}.json".format(k)), encoding="utf-8"))
        print("  {:<8} {:>6} {:>6} {:>7} {:>6} {:>8} {:>9}".format(
            k, T["corpus_lines"], T["segments"], T["ref_blocks"],
            s["ref_weak"], s["multi_groups"], s["corpus_covered"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
