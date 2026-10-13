#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""融合对齐核心：表头剥离、向量编码、统一转移 DP、两类掩码。

**单一真源**：产品侧一次性子进程 `scripts/embed_worker.py` 与实验台
（`scripts/fuse_*.py`、`src-tauri/tests/fuse_lab/`）都从这里导入——避免"已验证的算法"
出现第二份实现而漂移（本文件由 `fuse_calib.py` / `fuse_lab/__init__.py` 原文搬迁而来）。

两条不可动摇的实现约定（见 `src-tauri/tests/fuse_lab/README.md`）：
  1. 表头（名字行）剥离**只用于模型输入**；产物文本必须是语料原文逐字（`rebuild`）。
  2. 单调性必须真正强制——`align_v2` 在 `repeat_penalty = reset_penalty = inf` 时必须与
     严格递增 DP **逐段等价**（`scripts/fuse_calib.py --selfcheck` 断言，是"新结构不破坏
     旧行为"的长期护栏）。

阈值与判据的来龙去脉见 `benchmark/FUSE_THRESHOLD_CALIBRATION.md` §7.6/§7.9 与
`benchmark/FUSE_VECTOR_RECALL_VALIDATION.md`。
"""
import os
import re
import sys

# ── 路径解析：上溯找仓库根（同时含 scripts/ 与 runtime/ 的那一层）──
# 本文件有两个合法位置：仓内 `scripts/fuse_core.py` 与部署副本 `runtime/worker/fuse_core.py`
# （由 `scripts/bootstrap_embed.ps1` 拷贝），两者到仓库根的层数不同 ⇒ 用特征目录定位。
_HERE = os.path.dirname(os.path.abspath(__file__))


def _find_repo_root(start):
    d = start
    for _ in range(6):
        if os.path.isdir(os.path.join(d, "scripts")) and os.path.isdir(os.path.join(d, "runtime")):
            return d
        parent = os.path.dirname(d)
        if parent == d:
            break
        d = parent
    return os.path.abspath(os.path.join(start, ".."))


REPO_ROOT = _find_repo_root(_HERE)
RUNTIME = os.path.join(REPO_ROOT, "runtime")
DEPS_EMBED = os.path.join(RUNTIME, "deps_embed")
MODELS_EMBED = os.path.join(RUNTIME, "models", "embed")

# 嵌入依赖（numpy / onnxruntime / tokenizers）装在 runtime/deps_embed，而非系统环境。
# 产品侧由 Rust 传 PYTHONPATH（与 ocr/funasr worker 同法），此处再兜一次，便于直接跑脚本。
if os.path.isdir(DEPS_EMBED) and DEPS_EMBED not in sys.path:
    sys.path.insert(0, DEPS_EMBED)

import numpy as np  # noqa: E402

# ── 模型与 E5 前缀（E5 要求：语料走 passage、查询走 query）──
DEFAULT_EMBED_MODEL = "multilingual-e5-small"
DEFAULT_ONNX = "model_qint8_avx512_vnni.onnx"
E5_PREFIX = ("query: ", "passage: ")

SENT_END = "。！？…!?."

NEG = -1e9

DEFAULT = (0.02, 0.25)   # 现行未校准初值
# 统一转移模型 B（有界一对多）的复用罚分。实测安全窗口 **[0.2, 0.3]**：
#   ≥0.4 组解不开（pierro 107/121）；≤0.18 glupov 出现**误复用**（22/22 → 21/22）；
#   [0.2, 0.3] 内三案例：moon 15/15、glupov 22/22、pierro **119/121**（复用 2 次）。
# 取窗口中值 0.25（与 unmatched_penalty 同量级，纯属巧合，两者语义无关）。
#
# **2026-10-08 起 B 实际上休眠**：C（回退）放开后，DP 在本批素材上**一次都不用 B**
# （复用计数 0），且 repeat ∈ {inf, 0.25~0.5} 分数完全相同（206/236）——回退把"一对多"
# 的活也干了（碎片所在的第二遍播放被回退整段吃下）。保留 0.25 而非设为 inf：B 是
# 为"OCR 把一条字幕切成 N 段"设计的能力，本批素材上它被 C 遮蔽，但换素材可能重新需要，
# 而保留它是**零代价**的（分数逐位相同）。
REPEAT_DEFAULT = 0.25
# 统一转移模型 C（回退 / 拖进度条）的罚分。**2026-10-08 已落地启用**（原为 inf / 选项 i）。
#
# 启用依据（vesna「主播反复拖进度条重看 PV」案例到位后实测，四案例 236 计分段）：
#   · 平台极宽：reset ∈ **[0.01, 0.08]** 全为 **206/236 (87.3%)**；0.1→201、0.12→196、
#     0.15→194、0.2→185、inf→183。取平台中段 **0.05**（对素材波动留余量）。
#   · 相对"回退禁用"净 **+23**（183→206）：pierro **107→119**、vesna **39→51**。
#   · **代价 glupov 22→21（−1）**：细扫 0.01~0.2 全区间都是 21 ⇒ 该损失**不是阈值问题、
#     是结构性的**（启用回退后 DP 全局改走另一条路径，其中一处判错）。净收益 +23 远大于它。
#   · repeat 与本项**互相遮蔽**：只开 B 时 pierro 119 / vesna 23；只开 C 时 119 / 51
#     ⇒ C 严格优于 B（详见 REPEAT_DEFAULT 注释）。
#
# **已知的语义边界**：C 用"罚分"表达回退，而 DP 最大化的是「ΣS − 罚分」这个**代理目标**，
# 与评分口径（段落在可接受集合内）**不等价**。故大但有限的罚分反而危险：reset=1.0 时
# DP 会做少数几次"赚得回罚分"的回退，把后续一大段对齐带偏 ⇒ 3/79（比 inf 的 39/79 还差）。
# 更稳的做法是**显式重播分段**（先切出两遍播放、各自内部单调对齐），已记入
# `benchmark/FUSE_THRESHOLD_CALIBRATION.md` §7.5 待后续实施。
#
# 注：以上数值是各自标定当时的口径；现行产品基线见
# `benchmark/FUSE_THRESHOLD_CALIBRATION.md` §7.9 —— 默认 **210/236**、开「该不配」掩码 **226/236**。
RESET_DEFAULT = 0.05

# ── 「该不配」判据（§7.6 标定、**§7.9 修订**；开关**默认关闭**）──
# 问题：`truth_ok == []` 的段（转写多出来的英文语气词等）在语料里**根本没有对应行**，
# 正确行为是输出转写原文（DP 走 D 不配）；但 DP 只会选相似度最大的那条 ⇒ 必然硬塞一行。
#
# **实测结论：嵌入相似度矩阵不携带"有没有对应"这个信息**——原始 max-S 阈值、行/列/双向
# 中心化、裕度(max−2nd)、限定短段后的 max-S 全部被否（详见 §7.6 与 fuse_unmatch_calib.py）。
# 唯一可用的信号是**内容量**：`len_sub`（段剥标点/空白后的实质字符数）与**候选语料行**的
# 实质字符数的相对关系。
#
# 规则（语言无关：只比较两个整数字符数，不检查字符集）：一段**有实质内容**（len_sub ≥ 1）
# 却**不比它「并列候选」里最长的那一行更有内容**时，判"该不配"：
#     len_sub ≤ alpha · max_len_sub( top-K(S[i]) ∩ {j : S[i,j] ≥ maxS_i − tie_eps} )
#
# **§7.9 修订（判据改看 top-K，不再只看 argmax）**：argmax 对相似度微差**极不稳定**——
# vesna `Huh?`（len_sub=3）的 argmax 落在 `上吧`（行 len=2，S=0.8050）上，它对
# `那真不好意思呀`（行 len=7）是 0.8047，**只差 0.0003** ⇒ 旧判据 `3 > 1.05·2` 失效；
# 但"候选里有长行"这一事实本应判"该不配"。用户补进语料行 `薇斯纳` 后短段 argmax 进一步
# 被吸到短行，旧判据命中 12/13 → **9/13**，正是这个脆弱性的表现。
#
# **`tie_eps` 不是可选装饰，而是实测必需**：**字面**的 top-K max（即 `tie_eps = 0`）
# 在四案例上**假阳性爆炸**（K=3, alpha=1.05 → 命中 12/13 但 FP **20**，moon 15→13、
# pierro 119→110）。机制：语料长行很多、相似度差距常在 0.01~0.05，按**名次**取 K 个
# 必然把远处的长行纳进来，`max` 退化成"语料的典型长行长度"（实例：moon `Aria.` len=4、
# 名次 2 的行 len=4、S 差 0.023 ⇒ `4 ≤ 1.05·4` 误伤）。**近似并列**才是"argmax 不稳"的
# 真正来源 ⇒ 只在并列簇里取最长行：判据对"谁当第一名"免疫，又不让远处的长行抬高门槛。
# 标定结果（§7.9）：K=3、tie_eps=0.0015、alpha=0.95 命中 **12/13**（= 可达上限）、
# 四案例假阳性 **0**；端到端 vesna **55 → 71/79**（旧判据 66），老三案例逐位不变。
# `topk=1` 逐位复现 §7.6 旧判据（9/13），`tie_eps=0` 逐位复现字面 top-K（供对照）。
# **注意**：平台的**形式**语言无关，但它依赖"显示语言比语料语言单位内容更省"这一素材
# 事实（本批为 英文显示 ← 中文语料），换语言对必须重标。
UNMATCH_FILTER = False
# α=0.95 取实测平台 [0.75, 1.1429] 的中点。两端由谁决定（§7.9 实测）：
#   下沿 0.75 = vesna `…Huh?`（len 3 / 并列最长 4，α 再低就漏它）；
#   上沿 1.1429 = pierro `MurderofBirds / So, Mitya helped too?`（len 16 / 并列最长 14，α 一到就误伤）。
UNMATCH_ALPHA = 0.95
UNMATCH_TOPK = 3
# 并列带（相似度尺度）：`S[i,j] ≥ maxS_i − tie_eps` 的候选才算"并列"。0.0015 取实测窗口
# **[0.00101, 0.00203)** 的中点：下沿由 `*hum*.`@418.59 的第二候选（差 0.00101、行 len 9，
# 需要它进来才判得对）决定；上沿由 `Ohh!`@175.52 的第三候选（差 0.00203、行 len 6，
# 进来就误伤）决定。**窗口只有 ~2 倍宽**，且与 argmax 的微差同源 ⇒ 换嵌入模型必须重标。
UNMATCH_TIE_EPS = 0.0015


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


# ────────────────────────── 向量编码（onnxruntime + tokenizers）──────────────────────────

_SESSION_CACHE = {}


def load_embedder(model=DEFAULT_EMBED_MODEL, onnx=None):
    """加载 tokenizer 与 onnx 会话（CPU）。返回 (tokenizer, session)。"""
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


# ────────────────────────── DP：统一转移模型 ──────────────────────────

def _backtrack(dp, bk, n, m):
    j = int(np.argmax(dp[n]))
    match = [-1] * n
    for i in range(n, 0, -1):
        pj, kind = bk[i, j]
        match[i - 1] = (j - 1) if kind != 0 else -1
        j = int(pj)
    return match


# ────────────────── DP：统一转移模型（前进 / 复用 / 回退 / 不配）──────────────────
# T4c：把"序"与"重数"两个假设分开，四种转移并列——
#
#   A 前进   k > j'   dp[i-1][j'] + S[i-1][k-1] - skip_penalty*(k-1-j')
#   B 复用   k == j'  dp[i-1][k]   + S[i-1][k-1] - repeat_penalty      ← 有界一对多
#   C 回退   k < j'   dp[i-1][j'] + S[i-1][k-1] - reset_penalty       ← 拖进度条重看
#   D 不配   —        dp[i-1][j]  - unmatched_penalty
#
# 为什么需要 B：一段字幕被 OCR 切成 N 段时，严格递增 DP 无法让多段复用同一语料行 ⇒
# 后段被挤到下一行 ⇒ **此后整条链顺移**。pierro 实测：2 组一对多造成 21 个百分点的损失
# （78.5% vs 天花板 97.5%），远大于 OCR 残留碎片本身的代价。
#
# 为什么需要 C：实况里主播会**反复拖进度条重看** PV/剧情，此时语料下标顺序会**回退**
# （`1-2-3-4-1-2`）甚至跳进（`1-3-4-2`）——严格递增 DP 原理上无法表达。
#
# **安全性质（本函数的存在意义）**：`repeat_penalty = reset_penalty = inf` 时，
# 本函数与 `align_fast`（严格递增 DP）**逐段等价**，由 `--selfcheck` 断言。
# 新结构当时先在"回退禁用"下上线（选项 i）；**2026-10-08 C 已放开**（`RESET_DEFAULT = 0.05`，
# 依据与代价见该常量注释）。自检断言保留——它是"新结构不破坏旧行为"的长期护栏。
#
# 复杂度仍是 O(n·m)：A 用**前缀**最大（k 递增一趟）、C 用**后缀**最大（k 递减一趟）、
# B/D 各 O(1)。
#
# B 的"有界"由**线性累积罚分**实现（连续复用 r 次即付 r×repeat_penalty），无需额外状态；
# 若将来实测出现长链复用，再考虑加硬上限。
#
# C 的代价同样按次线性累加。但要注意 C 与 B 的**语义层级不同**：B 描述"同一行的重数"，
# C 描述"序列整体后退"——后者用局部罚分表达，只在本批素材的实测平台内可靠
# （见 RESET_DEFAULT 注释的"已知语义边界"与 §7.5 的显式重播分段方案）。

KIND_UNMATCHED, KIND_FORWARD, KIND_REPEAT, KIND_RESET = 0, 1, 2, 3


def align_v2(S, skip_penalty, unmatched_penalty,
             repeat_penalty=float("inf"), reset_penalty=float("inf"),
             diag=None):
    n, m = S.shape
    dp = np.full((n + 1, m + 1), NEG)
    bk = np.zeros((n + 1, m + 1, 2), dtype=np.int32)
    dp[0, 0] = 0.0
    rep_on = np.isfinite(repeat_penalty)
    res_on = np.isfinite(reset_penalty)
    for i in range(1, n + 1):
        prev = dp[i - 1]
        # ── D 不配 ──
        v_un = prev - unmatched_penalty
        better = v_un > dp[i]
        dp[i][better] = v_un[better]
        for j in np.nonzero(better)[0]:
            bk[i, j] = (j, KIND_UNMATCHED)
        # ── C 回退：max_{j' > k} dp_prev[j']，k 递减一趟 ──
        if res_on:
            suf_max, suf_arg = NEG, -1
            for k in range(m, 0, -1):
                if suf_max > NEG / 2:
                    v = S[i - 1, k - 1] - reset_penalty + suf_max
                    if v > dp[i, k]:
                        dp[i, k] = v
                        bk[i, k] = (suf_arg, KIND_RESET)
                if prev[k] > suf_max:      # 纳入 j' = k，供下一轮（k-1）使用
                    suf_max, suf_arg = prev[k], k
        # ── A 前进（前缀最大）+ B 复用，k 递增一趟 ──
        P = np.where(prev > NEG / 2, prev + skip_penalty * np.arange(m + 1), NEG)
        run_max, run_arg = NEG, -1
        for k in range(1, m + 1):
            cand = P[k - 1]
            if cand > run_max:
                run_max, run_arg = cand, k - 1
            if run_max > NEG / 2:
                v = S[i - 1, k - 1] - skip_penalty * (k - 1) + run_max
                if v > dp[i, k]:
                    dp[i, k] = v
                    bk[i, k] = (run_arg, KIND_FORWARD)
            if rep_on and prev[k] > NEG / 2:
                vb = prev[k] + S[i - 1, k - 1] - repeat_penalty
                if vb > dp[i, k]:
                    dp[i, k] = vb
                    bk[i, k] = (k, KIND_REPEAT)
    match = _backtrack(dp, bk, n, m)
    if diag is not None:
        j = int(np.argmax(dp[n]))
        kinds = {}
        for i in range(n, 0, -1):
            pj, kind = bk[i, j]
            kinds[kind] = kinds.get(kind, 0) + 1
            j = int(pj)
        diag.update({"repeat": kinds.get(KIND_REPEAT, 0),
                     "reset": kinds.get(KIND_RESET, 0),
                     "forward": kinds.get(KIND_FORWARD, 0),
                     "unmatched": kinds.get(KIND_UNMATCHED, 0)})
    return match


# ────────────────────────── 正文归一化与两类掩码 ──────────────────────────

# 正文归一化要去掉的字符（空白 + 各类括号 + 中英标点）
_BODY_STRIP = r"[\s「」\[\]【】（）()〈〉《》『』、。，！？…~·．,\.!\?\"'’‘“”—\-]"


def body_norm(text):
    """**正文**归一化：剥表头 + 去空白与标点。空串 ⇒ 该条没有可匹配的正文。"""
    return re.sub(_BODY_STRIP, "", split_header(text)[1])


# 判"有无**判别性内容**"用的句末标点、纯标点集与长行门。
# 长行门 40：英文"姓名框+长头衔"实测 35 字符（glupov `Former Acting Captain,"Ninth Company`）
# 且无句末标点 ⇒ 必须落在"无判别性内容"一侧，否则会把名牌段误判成台词段。
CONTENT_LINE_MAX = 40
_SENT_END = "。！？…!?."
_PUNCT_ONLY = set("…~·．,，。！？!?、；;:：-—「」[]【】（）()\"'’‘“” \t")


def content_free(text):
    """该文本（除首行外）是否**没有判别性内容**——各行为"纯标点"或"无句末标点的短行"。

    **术语澄清（重要）**：省略号**是正文台词**（用户 2026-10 明确），但它在嵌入空间里
    **没有判别力**（`body_norm` 后为空）。本函数判的是"**有没有可用于区分的内容**"，
    **不是**"是不是正文"——故命名为 content_free，不用"名牌态/无正文"那类词。

    首行通常是姓名框，不参与判定（它天然无判别力）；单行文本则连它一起判，
    否则单行台词段会被误判为无内容。
    """
    lines = [l.strip() for l in text.split("\n") if l.strip()]
    if not lines:
        return True
    rest = lines[1:] if len(lines) > 1 else lines
    for l in rest:
        if all(c in _PUNCT_ONLY for c in l):
            continue                     # 纯标点行（省略号等）无判别力
        if any(c in l for c in _SENT_END):
            return False
        if len(l) > CONTENT_LINE_MAX:
            return False
    return True


def mask_empty_body(S, corpus, seg_texts):
    """吸引子防护（2026-10-06）：**没有判别性内容的语料条目，不得被有内容的段命中**。

    成因：无内容条目在句向量空间里对**几乎所有**查询都给高余弦（短文本模长小、方向趋同），
    与"语义相关"无关。实测 top-1 占比：pierro `语料[17]「丑角」···`（正文归一化 **0 字符**）
    **54/121 (44.6%)**、glupov `语料[30]安东/头衔/…` **10/22 (45.5%)**。

    **为什么不能用相似度/裕度判据**（实测否决）：吸引子的相似度分布是**平的**——
    glupov `[30]` 全列 max 0.841 / 中位 0.817 / min 0.798（极差 0.043），而**合法匹配**
    （段18，真值就是它）只有 0.826，**根本不是该列最大值**（段1 是 0.841）。
    ⇒ 没有任何阈值能把"合法匹配"与"误命中"分开。唯一可用信号是**段自身有没有内容**。

    两侧判据**故意不同源**（实测教训）：
    - **语料侧**用 `body_norm(t) == ""`——即"正文剥掉标点后为空"。这条**精确命中**两个已知
      吸引子（[17]、[30]），且不会误伤"正文短但真实"的条目。
      曾改用 `content_free` 判语料侧 ⇒ 把 22 条**正文短但合法**的条目（如 `「丑角」可以。`
      剥标点后仍非空，但短且无句末标点）判成无内容 ⇒ pierro 20/120、glupov 11/22（实测）。
    - **段侧**用 `content_free`——因为语料是中文、转写是英文，`split_header` 的 16 字符表头门
      对英文长头衔失效（`Former Acting Captain,"Ninth Company` 35 字符被当正文），
      于是**同一显示**在语料侧"无正文"、在段侧"有正文"。`content_free` 用句末标点 + 40 字符
      长行门把这个不对称抹平。

    规则（单方向）：`语料 body_norm 为空 且 段 !content_free` → 置 NEG。
    反向（段 content_free、语料有内容）**不 mask**——那是 OCR 把正文行丢了
    （如省略号行 `conf=0.000` 被 `CONF_THRESHOLD` 丢弃），此时把该段配到有内容的条目反而对。

    合法匹配因此保留：glupov 段18（`Anton / 头衔 / …`）与 pierro 用户轨段13
    （`The Jester / …`）本身都 `content_free` ⇒ 不受影响。
    """
    empty_corpus = np.array([body_norm(t) == "" for t in corpus])
    if not empty_corpus.any():
        return 0
    cols = np.nonzero(empty_corpus)[0]
    masked = 0
    for i, t in enumerate(seg_texts):
        if content_free(t):
            continue
        for j in cols:
            if S[i, j] > NEG / 2:
                S[i, j] = NEG
                masked += 1
    return masked


def mask_should_unmatched(S, seg_texts, corpus, alpha=UNMATCH_ALPHA, topk=UNMATCH_TOPK,
                          tie_eps=UNMATCH_TIE_EPS):
    """「该不配」判据（§7.6 标定、**§7.9 修订为 top-K 并列簇**）：**行掩码**——命中的段整行置
    `NEG`，DP 随即只能走 D 不配。

    与既有 `mask_empty_body` 的**列掩码**对称：那条防的是"语料里没有内容的行被硬配"，
    这条防的是"语料里根本没有对应行的段被硬塞一行"。两者都是**单方向**、都不改 S 的其它部分。

    判据（语言无关，只比整数）：
        `len_sub >= 1` 且 `len_sub <= alpha · max_len_sub(T)`，
        `T = top-K(S[i]) ∩ {j : S[i,j] >= maxS_i − tie_eps}`
    其中 `len_sub` = 段剥表头/标点/空白后的实质字符数，`max_len_sub(T)` = **相似度前 K 名
    候选里、与 argmax 近似并列（差 ≤ tie_eps）的那些**中最长的语料行的实质字符数（故本函数
    必须在 `mask_empty_body` **之后**调用）。

    **§7.9 为什么从 argmax 改成 top-K**（缺陷实测）：旧判据只看 argmax 那一行，而 argmax
    对相似度微差**极不稳定**。vesna `Huh?`（`len_sub=3`）的 argmax 落在 `上吧`（`clen=2`，
    `S=0.8050`）上，它对 `那真不好意思呀`（`clen=7`）是 `0.8047`——**差 0.0003** ⇒ 旧判据
    `3 > 1.05·2` 失效、漏判；用户补进语料行 `薇斯纳`（`clen=3`）后短段 argmax 进一步被吸到
    短行，旧判据命中从 12/13 掉到 **9/13**。取并列簇里的**最长**候选对"谁是第一名"免疫，
    且语义更贴判据本意（"这些并列候选行没有一个能为该段提供内容"）。

    **`tie_eps` 是实测必需的**：`tie_eps = 0`（字面 top-K max）在四案例上假阳性爆炸
    （K=3, alpha=1.05 → FP 20，moon 15→13、pierro 119→110），因为按名次取 K 个必然纳入
    远处的长行（语料长行多、相似度差距常 0.01~0.05），`max` 退化为"语料典型长行长度"。
    `topk=1` 逐位复现 §7.6 旧判据。

    `len_sub >= 1` 这道门是必需的、不是补丁：`len_sub == 0` 的段（如 pierro
    `The Jester / …`）**没有可用于判别的内容**，它配到同样无内容的语料行是**正确**的
    ——那正是 `mask_empty_body` 要保护的合法用例，不能在这里误伤。

    实测（§7.9 重标：K=3、tie_eps=0.0015、alpha=0.95）：vesna 13 个空集段命中 **12**（可达上限，
    剩 1 段是文本逐字相同的复现对）、四案例非空集段**零误伤**；旧判据（`topk=1`）只有 9。
    "同一文本在别处被正确匹配"的复现对**原理上不可达**（`S` 行逐位相同），见 §7.6/§7.9。
    """
    clen = np.array([len(body_norm(t)) for t in corpus])
    masked = 0
    for i, t in enumerate(seg_texts):
        ls = len(body_norm(t))
        if ls < 1:
            continue                      # 无实质内容 ⇒ 不判（见上）
        row = S[i]
        valid = row > NEG / 2
        if not valid.any():
            continue
        cols = np.nonzero(valid)[0]
        if topk < len(cols):
            cols = cols[np.argsort(-row[cols])[:topk]]   # 相似度前 K 名（合法列内）
        if tie_eps > 0:                                  # 只留与 argmax 近似并列的候选
            top_s = float(row[cols].max())
            cols = cols[row[cols] >= top_s - tie_eps]
        if ls <= alpha * max(clen[j] for j in cols):
            S[i][valid] = NEG             # 整行封死 ⇒ DP 只能选"不配"
            masked += 1
    return masked
