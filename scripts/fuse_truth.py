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
    # T4c 新案例：主播 PV reaction。中英本地化**非 1:1**（英文合并/增补），参考文本按嵌字侧切，
    # 块内用 `---` 分隔多条语料行；英文多出（中文没有）的语气词保留英文原文——跨语言字符
    # 重合极低 ⇒ 匹配得分必然 < WEAK ⇒ 不入集合 ⇒ 该段期望"不配"。
    {"key": "vesna", "project": "vesna_trailer.gsa",
     "reference": "pv_reaction_vesna(voiced)_12min_reference.txt"},
]

WEAK = 0.72
# 段与参考块**零重叠**时的回退容差（秒）：取时间上最近的块，超过此距离则不指派。
# 取 1.5s：手打轴散布 ±0.5s + OCR 段界误差远小于它；而两遍播放之间的真实空档约 57s，
# 语气词段落也各有自己的参考块（走「弱匹配」路径而非本回退）⇒ 不会误指派。
NEAR_REF_SEC = 1.5
# 亚帧残留碎片阈值（秒）：与 OCR 侧 `SHORT_FRAGMENT_SUBFRAME_SEC = 0.5`（= frame_interval）
# 同一物理依据——不足一个采样网格间隔的产出段不可能是真实字幕，属过渡态残留。
# 这类段**参与对齐但不计分**（见 build_truth 末尾）。
SUBFRAME_SEC = 0.5
SENT_END = "。！？…!?."
# 纯标点行（省略号、间隔号等）——用于识别"名牌块的空正文"
PUNCT_ONLY = set("…~·．,，。！？!?、；;:：-—「」[]【】（）()\"'’‘“” \t")
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

# 参考块内分隔**多条语料行**的标记（单独一行）。用于"一个显示块覆盖多条语料"的情形：
# 英文 `Good morning, Snezhnograd!` 对应中文 `向你问候` + `至冬堡` ⇒ 块内写
# `---` 把两行语料分开。块内**没有文本**表示该显示在中英之间无对应（语气/感叹词）。
PART_SEP = "---"


def _split_head(body):
    """从若干文本行中剥出前导说话人/头衔行 → (head, 其余行)

    **只剩一行时的判定必须限定为"纯标点行"**（实测缺陷）：旧规则是"短且结尾非句末标点
    就当成说话人"，本意是处理**纯名牌块**（`「丑角」/ ···` ⇒ 正文为空）。但它会**误吃短台词**：
    PV 案例里 `派蒙 / 向你问候`（4 字、无句末标点）整条被吃成说话人 ⇒ `text` 变空、
    匹配得分 0。现有三案例的台词都长或带句末标点，所以一直没暴露。
    """
    body = list(body)
    head = []
    while len(body) > 1:
        h = body[0]
        if len(h) <= 16 and not h.endswith(tuple(SENT_END)):
            head.append(body.pop(0))
        else:
            break
    # 只剩一行：仅当它是**纯标点**（如 `···`）才视为名牌块的"空正文"
    if len(body) == 1 and body[0] and all(c in PUNCT_ONLY for c in body[0]):
        head.append(body.pop(0))
    return head, body


def parse_reference(path):
    """→ [{t0,t1,speaker,head,text,raw,parts}]

    块格式：首行时间码；其后为文本。文本里若出现**单独一行 `---`**，则把本块切成多个
    `part`——**每个 part 对应一条语料行**。块**可以没有文本**（中英无对应的显示）。

    每个 part 内：**前导的短行且结尾非句末标点**视为说话人/头衔（可多行，
    如 glupov 的「名字 + 头衔」两行），其余为台词。
    不能简单假定 body[0] 就是说话人：moon 的开场/收场旁白只有一行正文、没有说话人，
    按旧规则会把正文当说话人吃掉 ⇒ text 变空、匹配得分 0（实测两处）。
    """
    raw = io.open(path, encoding="utf-8-sig").read().replace("\r\n", "\n")
    out = []
    for b in [x.strip() for x in raw.split("\n\n") if x.strip()]:
        lines = [l.strip() for l in b.split("\n") if l.strip()]
        if not lines:
            continue
        m = re.match(r"(\d+):(\d+):(\d+):(\d+)\s*-\s*(\d+):(\d+):(\d+):(\d+)", lines[0])
        if not m:
            continue
        g = [int(x) for x in m.groups()]

        def sec(v):
            return v[0] * 3600 + v[1] * 60 + v[2] + v[3] / 100.0

        # 按 `---` 切成多个 part（无分隔符时只有一个）
        groups, cur = [], []
        for l in lines[1:]:
            if l == PART_SEP:
                groups.append(cur)
                cur = []
            else:
                cur.append(l)
        groups.append(cur)

        parts = []
        for gl in groups:
            head, body = _split_head(gl)
            parts.append({"speaker": head[0] if head else "", "head": head,
                          "text": " ".join(body), "raw": " ".join(gl)})
        out.append({"t0": sec(g[:4]), "t1": sec(g[4:]),
                    "speaker": parts[0]["speaker"] if parts else "",
                    "head": parts[0]["head"] if parts else [],
                    "text": "\n".join(p["text"] for p in parts if p["text"]),
                    "raw": " ".join(lines[1:]),
                    "parts": parts})
    return sorted(out, key=lambda r: r["t0"])


# ────────────────────────── 真值构建 ──────────────────────────

def build_truth(case, verbose=True):
    proj = load_project(case["project"])
    corpus = [c["text"] for c in proj["corpus"]]
    cbody = [norm(body_of(t)) for t in corpus]
    gc = collect_game_content(proj)
    refs = parse_reference(os.path.join(EXAMPLES, case["reference"]))

    # 参考块 → 语料行（文本匹配），**逐 part 匹配**（一个块可覆盖多条语料行）
    # 主判据：**正文 vs 正文**（两侧都按同一规则剥掉名字行/头衔行），避免同形表头
    # 干扰匹配。仅当主判据弱时才回退到「整条原文 vs 整条原文」——用于正文退化的
    # 情形（正文只剩省略号、或标题被当作正文，如 glupov 的 `安东 / 原「第九连队」
    # 临时连长 / ……`）。
    raw_corpus = [norm(t) for t in corpus]

    def match_one(rt, rr):
        """把一段文本匹配到语料行 → (j, score)"""
        rb = norm(rt)
        bj, bs = -1, 0.0
        for j, cb in enumerate(cbody):
            if not cb or not rb:
                continue
            score = difflib.SequenceMatcher(None, rb, cb).ratio()
            if len(rb) >= 6 and rb in cb:
                score = max(score, 0.97)
            elif len(cb) >= 6 and cb in rb:
                score = max(score, 0.95)
            if score > bs:
                bj, bs = j, score
        if bs < WEAK:
            rrn = norm(rr)
            for j, cr in enumerate(raw_corpus):
                if not cr or not rrn:
                    continue
                score = difflib.SequenceMatcher(None, rrn, cr).ratio()
                if len(rrn) >= 6 and rrn in cr:
                    score = max(score, 0.97)
                elif len(cr) >= 6 and cr in rrn:
                    score = max(score, 0.95)
                if score > bs:
                    bj, bs = j, score
        return bj, round(bs, 3)

    ref_to_corpus = []
    for r in refs:
        if not r["parts"]:                 # 块内无文本 ⇒ 该显示在中英间无对应
            ref_to_corpus.append({"ref": r, "j": -1, "score": 0.0, "parts": []})
            continue
        matched = [match_one(p["text"], p["raw"]) for p in r["parts"]]
        bi = max(range(len(matched)), key=lambda i: matched[i][1])
        ref_to_corpus.append({
            "ref": r, "j": matched[bi][0], "score": matched[bi][1],
            "parts": [{"j": j, "score": s} for j, s in matched]})

    # 转写段 → 参考块：**时长比 + λ·重叠比**（λ=0.3）
    #
    # 单一判据都会错，实测 157 段里有 5 处两判据分歧（逐条读文本裁定）：
    #   · 纯重叠：段跨边界时判给更长的那一块 —— glupov 段21、pierro 段106/116 判错；
    #   · 纯时长：相邻块时长接近时判给下一块 —— glupov 段2（9.03s vs 8.93/9.05）判错；
    #   · 名牌段（glupov 段18）：应配"……"块（时长近），而重叠会配到相邻台词。
    # 合并后两个信号互补：时长定位"哪一条字幕"，重叠在时长接近时打破平局。
    # λ 的可行区间由这 5 例夹出：(0.0084, 0.558)，取 0.3。
    LAMBDA_OVERLAP = 0.3
    # 段**实质重叠**参考块的重叠比下限：达到此比例的块都进入该段的**可接受集合**。
    # 0.25 的取法：一个显示块覆盖两条语料时，段对两块的 overlap/dur 各约 0.5 ⇒ 两块都进；
    # 而边界处的轻微搭接（通常 <10%）不会误进。空集合 ⇒ 该段**应当不配**。
    OV_MIN = 0.25
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
        if best is None:
            # 无任何重叠：退回**时间上最近**的参考块（限容差内）。
            #
            # 旧实现是"退回最大重叠（含 0）"，但比较写的是 `ov > best_ov`，而 `best_ov`
            # 初值就是 0.0 ⇒ 全为 0 时永不成立 ⇒ `best` 保持 None ⇒ 该段被记成
            # `ref_text=""`、可接受集合为空 ⇒ **真值判它"语料里没有对应行"**。
            #
            # 实测后果（vesna，2026-10-08）：段56 [570.95→571.76] 与段83 [673.26→673.91]
            # 落在参考块之间的**小空隙**里（参考块 `570.14→570.59` / `672.34→673.10`，
            # 手打轴 ±0.5s 散布 + OCR 段界误差所致），于是被误判"该不配"——
            # **这是推导缺陷，不是素材缺块**（参考文件里一个空块都没有）。
            near, nd = None, float("inf")
            for x in ref_to_corpus:
                r = x["ref"]
                # 时间距离：重叠时为 0，否则为间隙长度
                d = max(r["t0"] - g["end"], g["start"] - r["t1"], 0.0)
                if d < nd:
                    near, nd = x, d
            if near is not None and nd <= NEAR_REF_SEC:
                best, best_ov = near, 0.0
        # ── 可接受集合（T4c：对应关系是"关系"而非"函数"）──
        # 段实质重叠的**所有**参考块所映语料行取并集。集合为空 ⇒ 该显示在中英之间
        # 无对应（语气/感叹词等）⇒ **正确行为是"不配"**（输出转写原文），不是缺陷。
        #
        # **必须始终包含"最佳参考块"**（即旧单值口径的 truth）：否则会引入回归——
        # 实测 glupov 22/22 → 20/22（天花板 22 → 21）：名牌段的最佳块是按**时长接近**
        # 选出的"……"块，它与该段的**重叠比可能 < OV_MIN**，于是被集合漏掉。
        # 有了这条，集合恒为旧真值的**超集**，分数只可能升、不可能降。
        ok = set()
        if best:
            for p in best["parts"]:
                if p["j"] >= 0 and p["score"] >= WEAK:
                    ok.add(p["j"] + 1)
        for x in ref_to_corpus:
            r = x["ref"]
            ov = overlap(g["start"], g["end"], r["t0"], r["t1"])
            if ov / max(dur, 1e-9) < OV_MIN:
                continue
            for p in x["parts"]:
                if p["j"] >= 0 and p["score"] >= WEAK:
                    ok.add(p["j"] + 1)
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
            "truth_ok": sorted(ok),
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
            # 归位结果并入可接受集合（原集合保留：两种都可能对）
            r["truth_ok"] = sorted(set(r["truth_ok"]) | {j})
            r["namebox_fixed"] = True

    # ── 亚帧残留碎片：**不参与评分**（2026-10-06）──
    # 时长 < SUBFRAME_SEC 的产出段物理上不可能是一条真实字幕（人眼读不完），必是帧级
    # 精化/姓名框切换切出的**过渡态**。它**仍留在序列里参与对齐**（产品确实会输出它），
    # 但**不计入真值、不计入分母**——与嵌字基准的"噪音段（仅统计）"同口径。
    #
    # 为什么必须处理（实测）：pierro `段26 [549.52→549.65] (0.13s)` 的转写是
    # `Mitya / Ronova's Curse of Death persists, we won't be able to advance the experiment…`，
    # 即**前一条字幕的残留文本**；而按时间重叠它落在 `[549.36, 560.52]` 的参考块里
    # ⇒ 真值被算成 `语料[42]`（严冬计划…）。**但语料[41] 才是它文本对应的行**，
    # 而 `语料[41]` **不在参考覆盖范围内**（146 条里 28 条未被任何参考块覆盖）
    # ⇒ 参考中介这条路径**原理上到不了它**。于是真值错、DP 对，还凭空造出
    # 一组"一对多"（`语料[42] ← 段[26,27]`）污染评分口径。
    artifacts = [r for r in rows if r["dur"] < SUBFRAME_SEC]
    for r in artifacts:
        r["artifact"] = True
        r["truth"] = 0
        r["truth_ok"] = []
    for r in rows:
        r.setdefault("artifact", False)

    weak = [x for x in ref_to_corpus if x["score"] < WEAK]
    no_ref = [r for r in rows if not r["ref_text"]]
    # 可接受集合统计
    scored_rows = [r for r in rows if not r["artifact"]]
    empty_ok = [r for r in scored_rows if not r["truth_ok"]]
    multi_ok = [r for r in scored_rows if len(r["truth_ok"]) > 1]
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
        if artifacts:
            print("  亚帧残留碎片（**参与对齐但不计分**）{} 处：".format(len(artifacts)))
            for r in artifacts:
                print("    段{:3} [{:8.2f}→{:8.2f}] ({:.2f}s) 原真值[{}] | {}".format(
                    r["index"], r["start"], r["end"], r["dur"], r["truth"],
                    r["text"].replace("\n", " / ")[:52]))
        # 可接受集合（T4c：对应关系是"关系"而非"函数"）
        sizes = {}
        for r in scored_rows:
            sizes[len(r["truth_ok"])] = sizes.get(len(r["truth_ok"]), 0) + 1
        print("  可接受集合（计分段 {} 段）：{}".format(
            len(scored_rows),
            "  ".join("|集合|={} : {} 段".format(k, sizes[k]) for k in sorted(sizes))))
        if empty_ok:
            print("  **空集合（正确行为 = 不配）{} 段**：".format(len(empty_ok)))
            for r in empty_ok:
                print("    段{:3} [{:8.2f}→{:8.2f}] ({:.2f}s) | {}".format(
                    r["index"], r["start"], r["end"], r["dur"],
                    r["text"].replace("\n", " / ")[:56]))
        if multi_ok:
            print("  多元素集合 {} 段（中英切分不一致，两种都算对）：".format(len(multi_ok)))
            for r in multi_ok[:10]:
                print("    段{:3} [{:8.2f}] 可接受 {} | {}".format(
                    r["index"], r["start"], r["truth_ok"],
                    r["text"].replace("\n", " / ")[:44]))

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
            "artifacts": len(artifacts),
            "scored_segments": len(rows) - len(artifacts),
            "empty_sets": len(empty_ok),
            "multi_sets": len(multi_ok),
        },
        "rows": rows,
        "ref_to_corpus": [{"t0": x["ref"]["t0"], "t1": x["ref"]["t1"],
                           "speaker": x["ref"]["speaker"], "text": x["ref"]["text"],
                           "corpus_index": x["j"] + 1, "score": x["score"],
                           "parts": [{"corpus_index": p["j"] + 1, "score": p["score"]}
                                     for p in x["parts"]]}
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
