# -*- coding: utf-8 -*-
"""融合对齐实验共用底座：干净用例、向量编码、LLM 调用、对齐算法。

被 `bench_fusion_cases.py`（LLM 判别器）、`bench_fusion_recall.py`（向量召回判别器）、
`fuse_offline_validate.py`（离线验证流水线）共同引用。

设计原则（与项目既有基准一致）：
  · 用例**自撰**、不入工程产物；真值可机械推导，不靠人工标注
  · 判别器用**无不动点排列**（derangement）——使"照抄编号"的恒等映射每条皆错，
    故内容正确率可直接与随机基线比较
  · 依赖与模型路径可被环境变量覆盖，便于换模型 / 换目录

运行时依赖：`runtime/deps_embed`（onnxruntime CPU + tokenizers）
模型：`runtime/models/embed/multilingual-e5-small`
"""
import io
import json
import os
import re
import subprocess

# ── 路径解析：仓库根 = 本文件上溯四级（src-tauri/tests/fuse_lab/__init__.py → 仓库根）──
_HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(_HERE, "..", "..", ".."))
RUNTIME = os.path.join(REPO_ROOT, "runtime")
DEPS_EMBED = os.path.join(RUNTIME, "deps_embed")
MODELS_EMBED = os.path.join(RUNTIME, "models", "embed")
EXAMPLES = os.path.join(REPO_ROOT, "examples", "benchmark_examples")
BENCH_OUT = os.path.join(REPO_ROOT, "temp", "bench_output")

# ── 仅作实验输入：模型/提示由运行时配置与 Rust build_prompt 决定，此处保持一致 ──
BATCH_SIZE = 30
MAX_TOKENS = 4096
DEFAULT_EMBED_MODEL = "multilingual-e5-small"
DEFAULT_ONNX = "model_qint8_avx512_vnni.onnx"
E5_PREFIX = ("query: ", "passage: ")

SENT_END = "。！？…!?."

# ── 30 组自撰干净对照：(英文 = 转写侧, 中文 = 可靠语料) ──
# 每句完整、语义互不重叠（不共享实体词），故不存在"靠关键词撞上"的捷径。
PAIRS = [
    ("The seal on the northern gate has weakened.", "北门的封印已经减弱了。"),
    ("I left the supplies by the fountain.", "我把补给放在喷泉旁边了。"),
    ("Do not speak of this to the captain.", "别把这件事告诉队长。"),
    ("The old clock tower stopped at midnight.", "那座旧钟楼在午夜停了。"),
    ("Her fever finally broke this morning.", "她的烧今早终于退了。"),
    ("We need three more lanterns for the festival.", "庆典还需要三盏灯笼。"),
    ("The merchant never returned from the eastern road.", "那个商人再也没从东边那条路回来。"),
    ("Someone has been reading my letters.", "有人在偷看我的信。"),
    ("The river froze earlier than last year.", "河水比去年冻得更早。"),
    ("I can hear footsteps in the cellar.", "我能听见地窖里有脚步声。"),
    ("Please deliver this package before sunset.", "请在日落前把这个包裹送到。"),
    ("The bridge collapsed under the weight of the cart.", "桥被货车的重量压塌了。"),
    ("A stranger asked me about the abandoned mine.", "有个陌生人向我打听那座废弃的矿。"),
    ("The kitchen ran out of salt again.", "厨房的盐又用完了。"),
    ("My sword was not sharpened properly.", "我的剑没有磨好。"),
    ("The choir will practice in the chapel tonight.", "唱诗班今晚在礼拜堂排练。"),
    ("He traded his horse for a broken compass.", "他用马换了一个坏掉的罗盘。"),
    ("The harvest this year was smaller than expected.", "今年的收成比预想的少。"),
    ("Someone carved a warning into the wooden door.", "有人在木门上刻了一句警告。"),
    ("The physician refuses to leave the patient alone.", "医生不肯让病人单独待着。"),
    ("Our water reserves will last two more days.", "我们的水还够撑两天。"),
    ("The beacon on the hill has not been lit.", "山上的烽火没有点起来。"),
    ("I found a torn map inside the old chest.", "我在旧箱子里找到一张撕破的地图。"),
    ("The wolves have moved closer to the village.", "狼群离村子更近了。"),
    ("She has not spoken since the funeral.", "葬礼之后她一直没说话。"),
    ("The blacksmith is still waiting for the iron shipment.", "铁匠还在等那批铁。"),
    ("Nobody remembers who built this tower.", "没人记得这座塔是谁建的。"),
    ("The rain washed away the markings on the road.", "雨水冲掉了路上的标记。"),
    ("He hid the key beneath the third step.", "他把钥匙藏在第三级台阶下面。"),
    ("The council will decide the matter tomorrow.", "议会明天决定这件事。"),
]
EN = [p[0] for p in PAIRS]
CN = [p[1] for p in PAIRS]

# 12 条固定任意排列（无不动点）
PERM12 = [3, 7, 1, 9, 12, 2, 8, 4, 11, 5, 10, 6]
# 30 条确定性无不动点排列：q(k) = (7k+1) mod 30（7 与 30 互素 ⇒ 双射；无不动点）
PERM30 = [((7 * k + 1) % 30) + 1 for k in range(30)]
assert all(PERM30[k] != k + 1 for k in range(30)), "PERM30 有不动点"
assert sorted(PERM30) == list(range(1, 31)), "PERM30 非双射"
assert all(PERM12[k] != k + 1 for k in range(12)), "PERM12 有不动点"

UNMATCHED_GC = "The innkeeper said the room upstairs is free tonight."


# ────────────────────────── 用例构造（真值机械推导）──────────────────────────

def case_shift(n):
    """循环移位 1：GC[i] 应对 OCR[i-1]（GC[1] → OCR[n]）"""
    corpus = CN[1:n] + CN[0:1]
    truth = [n if i == 0 else i for i in range(n)]
    return corpus, truth


def case_aligned(n):
    return CN[:n], [i + 1 for i in range(n)]


def case_with_unmatched(pos=5, n=12):
    """对齐语料 + 在 pos 处插入一条无语料对应的 GC（期望 ocr_index=0）"""
    gc, truth, = [], []
    for i in range(n):
        gc.append(EN[i])
        truth.append(i + 1)
        if i + 1 == pos:
            gc.append(UNMATCHED_GC)
            truth.append(0)
    return CN[:n], gc, truth


def case_same_lang_deranged(perm, lang="en"):
    """同语言**逐字相同**文本 + 打乱语料顺序：隔离"翻译难"这一解释。
    连字符串相等都不去利用 ⇒ 根本没在做内容匹配。"""
    src = EN if lang == "en" else CN
    corpus = [src[perm[i] - 1] for i in range(len(perm))]
    pos = {perm[i]: i + 1 for i in range(len(perm))}
    truth = [pos[i + 1] for i in range(len(perm))]
    return corpus, src[:len(perm)], truth


def build_cases():
    """返回 [(name, corpus_texts, gc_texts, truth)]；truth[i] = OCR 位置（1-based，0=无对应）"""
    cases = []
    c, t = case_shift(12)
    cases.append(("A1-错序12(移位)", c, EN[:12], t))
    c = [CN[p - 1] for p in PERM12]
    t = [{PERM12[k]: k + 1 for k in range(12)}[i + 1] for i in range(12)]
    cases.append(("A2-错序12(任意排列)", c, EN[:12], t))
    c, t = case_shift(30)
    cases.append(("A3-错序30(移位)", c, EN[:30], t))
    c = [CN[p - 1] for p in PERM30]
    t = [{PERM30[k]: k + 1 for k in range(30)}[i + 1] for i in range(30)]
    cases.append(("A4-错序30(任意排列)", c, EN[:30], t))
    c, t = case_aligned(12)
    cases.append(("C1-对齐12(对照)", c, EN[:12], t))
    corpus, gc, truth = case_with_unmatched()
    cases.append(("C2-对齐12+1条无对应", corpus, gc, truth))
    c, g, t = case_same_lang_deranged(PERM12, "en")
    cases.append(("E2-同语言逐字相同(错序)", c, g, t))
    c, g, t = case_same_lang_deranged(list(range(1, 13)), "en")
    cases.append(("E3-同语言逐字相同(对齐)", c, g, t))
    return cases


def is_deranged_case(name):
    """判别器用例：恒等映射正确率必须为 0（否则判别器失效）"""
    return name.startswith("A") or name.startswith("E2")


def identity_baseline(truth):
    return sum(1 for i, w in enumerate(truth) if w == i + 1 and w != 0)


# ────────────────────────── 表头（名字行）处理 ──────────────────────────

def split_header(text):
    """拆分前导表头（名字行/头衔行/名牌）与正文 → (header|None, body)。

    判据：行短（≤16 字符）且不含句末标点 ⇒ 视为表头。
    **只用于模型输入**；产物文本必须用原文（header + body），
    因为"产物与对应语料逐字一致"是既定产品要求。
    整段皆为表头（无正文）时**不剥离**，避免空串参与匹配。
    """
    lines = [l.strip() for l in text.split("\n")]
    lines = [l for l in lines if l]
    head_lines = []
    while len(lines) > 1:
        head = lines[0]
        if len(head) <= 16 and not any(c in head for c in SENT_END):
            head_lines.append(lines.pop(0))
        else:
            break
    body = "\n".join(lines).strip()
    if not body:
        return None, text.strip()
    return ("\n".join(head_lines).strip() or None), body


def strip_for_embed(text):
    """取用于编码的正文（表头不参与语义匹配——表头同形会淹没正文）"""
    return split_header(text)[1]


def rebuild(header, body):
    """产物文本还原：表头 + 正文"""
    return "{}\n{}".format(header, body) if header else body


# ────────────────────────── 单调 DP 对齐 ──────────────────────────

def monotonic_align(S, skip_penalty=0.02, unmatched_penalty=0.25):
    """单调最优对齐。语料与转写同序 ⇒ 匹配到的语料下标必须**严格递增**。

    dp[i][j]（j ∈ 0..m）：前 i 段处理完，且已用到的最大语料下标为 j（j=0 表示尚未匹配）。
      · 第 i 段无对应：dp[i-1][j] - unmatched_penalty            → dp[i][j]
      · 第 i 段匹配语料行 k（k > j）：dp[i-1][j] + S[i-1][k-1]
        - skip_penalty × (k-1-j)（跨过的语料行代价）             → dp[i][k]
    故 j 单调不减、k 严格大于 j，**不会出现回退或回绕**。
    返回 (每个 GC 的语料下标（-1=未匹配）, dp)。
    """
    import numpy as np
    n, m = S.shape
    NEG = -1e9
    dp = np.full((n + 1, m + 1), NEG, dtype=np.float64)
    bk = np.zeros((n + 1, m + 1, 2), dtype=np.int32)
    dp[0, 0] = 0.0
    for i in range(1, n + 1):
        for j in range(m + 1):
            if dp[i - 1, j] <= NEG / 2:
                continue
            v = dp[i - 1, j] - unmatched_penalty
            if v > dp[i, j]:
                dp[i, j] = v
                bk[i, j] = (j, 0)
        for j in range(m + 1):
            if dp[i - 1, j] <= NEG / 2:
                continue
            base = dp[i - 1, j]
            for k in range(j + 1, m + 1):
                v = base + S[i - 1, k - 1] - skip_penalty * (k - 1 - j)
                if v > dp[i, k]:
                    dp[i, k] = v
                    bk[i, k] = (j, 1)
    j = int(np.argmax(dp[n]))
    match = [-1] * n
    for i in range(n, 0, -1):
        prev_j, kind = bk[i, j]
        match[i - 1] = (j - 1) if kind == 1 else -1
        j = int(prev_j)
    return match, dp


# ────────────────────────── 向量编码（onnxruntime + tokenizers）──────────────────────────

_SESSION_CACHE = {}


def load_embedder(model=DEFAULT_EMBED_MODEL, onnx=None):
    """加载 tokenizer 与 onnx 会话（CPU）。返回 (tokenizer, session)。"""
    import sys
    if DEPS_EMBED not in sys.path:
        sys.path.insert(0, DEPS_EMBED)
    key = (model, onnx)
    if key in _SESSION_CACHE:
        return _SESSION_CACHE[key]
    from tokenizers import Tokenizer
    import onnxruntime as ort
    d = os.path.join(MODELS_EMBED, model)
    tok = Tokenizer.from_file(os.path.join(d, "tokenizer.json"))
    tok.enable_truncation(max_length=256)
    sess = ort.InferenceSession(
        os.path.join(d, onnx or os.environ.get("GSA_EMBED_ONNX", DEFAULT_ONNX)),
        providers=["CPUExecutionProvider"])
    _SESSION_CACHE[key] = (tok, sess)
    return tok, sess


def encode_texts(tok, sess, texts):
    """batch 编码 → L2 归一化句向量（mask 平均池化，E5 官方做法）"""
    import numpy as np
    encs = tok.encode_batch(list(texts))
    maxlen = max(len(e.ids) for e in encs)
    pad_id = 1
    ids = np.full((len(encs), maxlen), pad_id, dtype=np.int64)
    mask = np.zeros((len(encs), maxlen), dtype=np.int64)
    for i, e in enumerate(encs):
        n = len(e.ids)
        ids[i, :n] = e.ids
        mask[i, :n] = e.attention_mask
    feed = {}
    for inp in sess.get_inputs():
        if inp.name == "input_ids":
            feed[inp.name] = ids
        elif inp.name == "attention_mask":
            feed[inp.name] = mask
        elif inp.name == "token_type_ids":
            feed[inp.name] = np.zeros_like(ids)
    out = sess.run(None, feed)
    h = out[0]
    if h.ndim == 3:
        m = mask[..., None].astype(np.float32)
        v = (h * m).sum(1) / np.clip(m.sum(1), 1e-9, None)
    else:
        v = h
    return v / np.clip(np.linalg.norm(v, axis=1, keepdims=True), 1e-9, None)


def similarity_matrix(tok, sess, corpus, queries, prefix=E5_PREFIX):
    """cos 相似度矩阵 S[i][j]（已归一化 ⇒ 点积）"""
    C = encode_texts(tok, sess, [prefix[1] + t for t in corpus])
    Q = encode_texts(tok, sess, [prefix[0] + t for t in queries])
    return Q @ C.T


# ────────────────────────── 真实工程（.gsa）读取 ──────────────────────────

def load_project(filename):
    """读取 examples/benchmark_examples/<filename>（首行为魔数头）"""
    with io.open(os.path.join(EXAMPLES, filename), encoding="utf-8") as f:
        raw = f.read()
    return json.loads(raw.split("\n", 1)[1])


def collect_game_content(proj):
    """复刻前端 collectGameContentSegments：game 轨 ASR 段 + 无配音处嵌字段（按时间升序）。
    主播轨（track_role=streamer）按设计排除。"""
    asr = [e for t in proj["tracks"]
           if t["type"] == "asr" and t.get("track_role") == "game"
           for e in t.get("events", [])]
    embeds = [e for t in proj["tracks"]
              if t["type"] == "embed_ocr" and t.get("track_role") == "game"
              for e in t.get("events", [])]

    def overlap_ratio(embed):
        if embed["end"] <= embed["start"]:
            return 0.0
        cov = 0.0
        for a in asr:
            s, e = max(embed["start"], a["start"]), min(embed["end"], a["end"])
            if e > s:
                cov += e - s
        return cov / (embed["end"] - embed["start"])

    kept = [e for e in embeds if overlap_ratio(e) < 0.5]
    merged = sorted(asr + kept, key=lambda e: e["start"])
    return [{"start": e["start"], "end": e["end"], "text": e["text"],
             "kind": "asr" if any(e is a for a in asr) else "embed"} for e in merged]


# ────────────────────────── LLM 直调（与 Rust run_complete 同 argv）──────────────────────────

_RUN_COMPLETE_ARGS = ("-st", "--no-display-prompt", "--temp", "0.2", "--color", "on")


def llama_paths():
    cfg_path = os.path.join(RUNTIME, "config.json")
    with io.open(cfg_path, encoding="utf-8") as f:
        cfg = json.load(f)
    binary = os.path.join(RUNTIME, cfg.get("llm_binary", "bin/llm/llama-cli.exe"))
    model = os.path.join(RUNTIME, os.environ.get("GSA_BENCH_LLM_MODEL", cfg.get("llm_model", "")))
    return binary, model


def run_llama(prompt, max_tokens=MAX_TOKENS):
    """调用 llama-cli（argv 与 src-tauri/src/ai_runtime/llm.rs run_complete 一致），
    返回按 parse_answer 同规则提取的回答文本。"""
    binary, model = llama_paths()
    cmd = [binary, "-m", model, "-p", prompt, "-n", str(max_tokens), *_RUN_COMPLETE_ARGS]
    p = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if p.returncode != 0:
        return "LLAMA_ERROR rc={} stderr_tail={}".format(p.returncode, p.stderr[-400:])
    return parse_answer(p.stdout)


def parse_answer(stdout):
    """复刻 ai_runtime/llm.rs parse_answer：取首个 \\x1b[0m 之后、首个 \\x1b[35m 之前"""
    reset, magenta = "\x1b[0m", "\x1b[35m"
    i = stdout.find(reset)
    if i == -1:
        return stdout.strip()
    tail = stdout[i + len(reset):]
    j = tail.find(magenta)
    return tail[:j if j != -1 else len(tail)].strip()


def to_index(value):
    """复刻 fuse/mod.rs deser_index：数字直用；字符串取其中数字；null→0"""
    if value is None:
        return 0
    if isinstance(value, bool):
        raise ValueError("bool 不是合法 index")
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        raise ValueError("float 不受支持（visit_f64 未实现 → 整批失败）")
    if isinstance(value, str):
        digits = "".join(c for c in value if c.isdigit())
        return int(digits) if digits else None
    raise ValueError("type " + type(value).__name__)


def parse_fusion_output(raw):
    """复刻 fuse/mod.rs parse_fusion_output（三级：严格 → 花括号截取 → 补缺失外层）"""
    t = raw.strip()
    try:
        return json.loads(t)["segments"], "strict"
    except Exception:
        pass
    cands = []
    o, c = t.find("{"), t.rfind("}")
    if o != -1 and c > o:
        cands.append(t[o:c + 1])
    opens, closes = t.count("{"), t.count("}")
    if opens > closes and opens - closes <= 4:
        o2, lb = t.find("{"), t.rfind("]")
        if o2 != -1 and lb > o2:
            cands.append(t[o2:lb + 1] + "}" * (opens - closes))
    for cd in cands:
        try:
            return json.loads(cd)["segments"], "brace-repair"
        except Exception:
            pass
    return None, "fail"


def write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)

    def _plain(o):
        for attr, fn in (("item", lambda x: x.item()),):
            if hasattr(o, attr):
                try:
                    return fn(o)
                except Exception:
                    pass
        return str(o)

    with io.open(path, "w", encoding="utf-8", newline="") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1, default=_plain)


def srt_timestamp(seconds):
    ms = int(round(seconds * 1000))
    return "{:02}:{:02}:{:02},{:03}".format(
        ms // 3600000, ms // 60000 % 60, ms // 1000 % 60, ms % 1000)


def write_srt(rows, path, text_key="product_text"):
    """按事件时间轴写 SRT（BOM + CRLF，与产品 export 一致）"""
    rows = sorted(rows, key=lambda r: r["start"])
    blocks = []
    for i, r in enumerate(rows, 1):
        blocks.append("{}\n{} --> {}\n{}\n".format(
            i, srt_timestamp(r["start"]), srt_timestamp(r["end"]),
            r.get(text_key, "").strip()))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with io.open(path, "w", encoding="utf-8-sig", newline="\r\n") as f:
        f.write("\n".join(blocks))
