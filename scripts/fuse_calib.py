#!/usr/bin/env python3
"""T2 阶段 B：统一评估台 + DP 参数网格搜索。

真值来自 scripts/fuse_truth.py（参考文本为中介，独立于对齐算法）。

设计要点：
  · 相似度矩阵 S 只依赖编码，**与 DP 参数无关** ⇒ 每案例只算一次，网格内只跑 DP；
  · DP 用 O(n·m) 的前缀最大值形式（等价于 O(n·m²) 朴素式，脚本内自带等价性自检）；
  · 准确率同时给出**相对天花板**：严格递增 DP 无法表达真值里的一对多
    （同一语料行被多段复用）⇒ 天花板 = n − Σ(组内段数−1)。

**本脚本不做输入卫生**（"互为前缀的相邻段"的合并）。那类"同一条字幕被 OCR 拆成两段"
属 **OCR 合并层的 raw 缺陷**，应在管线层修；曾在此处按文本判据合并，**已被证否并删除**
——见 benchmark/OCR_PIPELINE_DEFECTS.md **D12**：正当用例与误伤用例在 (短态, 长态)
文本上完全同形，**任何只依赖该文本对的判据不可能两全**。故本脚本只有一种口径：
**消费管线产出**。管线修好后重新生成产物轨，即得到干净输入。

用法：
    python scripts/fuse_calib.py                # 全案例网格搜索
    python scripts/fuse_calib.py --selfcheck    # 只做 DP 等价性自检
    python scripts/fuse_calib.py --cases pierro # 限定案例
环境：numpy/onnxruntime 装在 runtime/deps_embed，直接跑需先
    $env:PYTHONPATH="<仓库根>\\runtime\\deps_embed"（产品侧由 Rust 传入，见 embed.rs）
"""
import argparse
import difflib
import io
import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "tests"))
sys.dont_write_bytecode = True  # 不写 __pycache__（与 fuse_lab 同源，避免工作区再生 .pyc）
import numpy as np  # noqa: E402
from fuse_lab import BENCH_OUT, load_embedder, similarity_matrix, split_header  # noqa: E402
from fuse_core import (  # noqa: E402
    DEFAULT, NEG, REPEAT_DEFAULT, RESET_DEFAULT,
    UNMATCH_ALPHA, UNMATCH_FILTER, UNMATCH_TOPK, UNMATCH_TIE_EPS,
    _backtrack, align_v2, body_norm, mask_empty_body, mask_should_unmatched,
)

GRID_SKIP = [0.0, 0.002, 0.005, 0.01, 0.02, 0.04, 0.08, 0.16]
GRID_UNMATCHED = [0.02, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5]


# ── 显式重播分段（§7.7 **重评**，2026-10-08；**默认关闭**，与 UNMATCH_FILTER 同风格）──
# 动机：C（回退）用**局部罚分**表达"序列整体后退"，而 DP 最大化的是「ΣS − 罚分」这个
# **代理目标**，与评分口径不等价 ⇒ 大但有限的罚分反而危险（reset=1.0 → vesna 3/79）。
# 分段方案把"重播"这一**全局结构**从逐步罚分里拿出来：先检测重播边界，再按边界切块、
# **块内独立对齐**（块间不传递语料下标约束）。
#
# 检测器（**参考无关**：只用产出段自身文本 + 段间相似度 + 时间间隙）沿用 §7.7 末尾记的
# "唯一有希望方向"——重播的本质是**整段内容被重新覆盖**，落到可算量上就是**孪生段 onset**：
#   ① 对每段 i，在 `i − SEG_MIN_LAG` 之前找最相似的另一段（段×段余弦，只用产出段文本）；
#   ② 相似度 ≥ SEG_TAU ⇒ 记"该段有孪生"（= 这个显示之前出现过）；
#   ③ 切点 c 的分数 `P5 = [c, c+W)` 内有孪生的占比 − `[c−W, c)` 内同占比 ——
#      **onset** 是关键：重播区**内部**两侧都接近 1 ⇒ 差值 0（上一轮 B 只统计后缀侧，
#      故 `c=47` 的 `B.dp = 1.000` 压过主边界；加了 onset 后它自然归零）；
#   ④ 再过一道时间间隙门 `gap(c) ≥ SEG_MIN_GAP`，滤掉零间隙的"同一行显示两次"。
#
# 实测（`scripts/fuse_seg_calib2.py`，四案例 236 切点）：vesna 检测点 = **c=39、40**
# （主边界 + 它的擦边小回退），moon/glupov/pierro **零判定**；(W, τ) 参数平台上
# **9/16 格可行**（τ ∈ [0.90, 0.95] × W ∈ {3,5,8,10}）⇒ 不是单点巧合。
# **已知分辨率边界**：重播长度 < 5 段的"小程序回退"（vesna 尾部 c=80/81，落差 4/3 行）
# 检测不到 —— 它们由 `reset` 在**块内**承担（分段不接管小回退）。
SEGMENTED_FILTER = False
SEG_WIN = 5               # P5 窗口（段）
SEG_TAU = 0.90            # 孪生相似度门
SEG_MIN_LAG = 3           # 孪生最小间隔（排除相邻重复显示）
SEG_MIN_GAP = 5.0         # 时间间隙门（秒）
SEG_P5_MIN = 0.4          # P5 阈值
# 最小块长：**短块没有上下文**——块内独立对齐从语料头起步，1~2 段的块必然配错。
# 实测（vesna，2026-10-08）：检测器原始输出 {39, 40} 会切出一个 **1 段的块**（段40
# `And using/forbidden alchemy…`），它独立对齐后配到 `语料[5]` 而不是 `语料[30]`
# ⇒ 端到端 **53 → 52（−1）**；丢弃造成短块的切点后（{40}）回到 **53（持平）**。
# 相邻检测点（间隔 < SEG_MIN_BLOCK）视为**同一次 onset**：取靠后者——靠前者的窗口只是
# 跨进了重播区（P5 的窗口模糊），故它往往提前 1 段；独立信号 S6 在 vesna 也判 39→1 / 40→5。
SEG_MIN_BLOCK = 3

# ── 置信计分目标函数（§7.8，**默认关闭**；与 UNMATCH_FILTER / SEGMENTED_FILTER 同风格）──
# 动机（三条后果，见 §7.8①）：DP 最大化的是「ΣS − 罚分」这个**代理目标**，而评分口径是
# 「**段落在可接受集合内的段数**（集合为空 ⇒ 正确行为是不配）」——两者不等价：
#   ① `reset=1.0` 时 vesna 只有 3/79（比 inf 的 39/79 还差）：DP 会做少数几次"赚得回罚分"
#     的回退，把后续一大段带偏（§7.5/§7.7）；
#   ② vesna 分段后块内 DP 只拿到 27/38 与 25/40，比块天花板低 9~10 分（结构天花板已 72/79）
#     ⇒ 剩余差距是目标错配，不是结构（§7.7 重评）；
#   ③ §7.6 的「该不配」行掩码本质是本思路的**特例**（把不该配的行整行置 NEG），
#     单独就贡献 +17（53→70）。
#
# 设计（**不写新 DP**）：`align_v2(S, skip, unmatched, repeat, reset)` 只吃矩阵与罚分
# ⇒ 换目标 = **换矩阵**。但**设计原型的纯 0/1 矩阵实测结构性退化**（§7.8③）：
# `R = where(S>=tau, +1, −miss_pen)` 里所有 ≥τ 的格同值 ⇒ "配到哪一行"无歧视
# （同分多路径，回溯任意），四案例最高只有 161/236 且**硬门全败**；且 miss_pen
# 因"不配免费 + 跳行廉价"而基本不 bind。**最小修复（仍是换矩阵）**：加一个
# ΣS 破平项 `+eps·S`——它只负责"+1 层内选哪一行"的排序，不改变主目标的 0/1 层级。
#
# **2026-10-08 标定落点（`scripts/fuse_count_calib.py` 全量扫描）**：
#   最优 = (τ=0.88, eps=0.38, miss_pen=0.3, skip=0.02, unmatched=0) ⇒ **219/236**——
#   老三案例 15/21/119 **逐位等于现行基线**（硬门逐位持平），vesna 53→**64 (+11)**。
#   平台：tau **[0.86, 1.0+] 极宽**、rs **[0.02, 1.0]（比 ΣS 尺度 [0.01,0.08] 宽 10×，**
#   `reset=1.0` 异常消失）**、rp [0.05, inf] 无约束、un 必须为 0（"+0.05 即损 −9"）；
#   **eps 与 miss_pen 均为单点**（eps ±0.005 即 −3~−8、miss_pen +0.02 即 −127）——窄如
#   刀锋，是本轮**最大的稳健性疑问**（见 §7.8⑥ 结论：不默认启用）。
#   对照：ΣS+掩码 225 仍是最优口径；计分+掩码也是 225（两路在掩码口径上汇合）。
COUNT_OBJECTIVE = False
COUNT_TAU = 0.88            # 置信门：S ≥ τ 才算"+1 层"（平台 [0.86, 1.0+]）
COUNT_EPS = 0.38            # ΣS 破平项权重（**单点**；纯 0/1 设计态 = 0）
COUNT_MISS_PEN = 0.3        # 硬塞一个弱匹配的代价（**单点**；+0.02 即崩）
COUNT_SKIP = 0.02           # 跳行成本（平台 [0.01, 0.02]，0.05 已损 −10）
COUNT_REPEAT = 0.25         # 复用（有界一对多）每事件罚分（[0.05, inf] 无约束）
COUNT_RESET = 0.05          # 回退（重播）每事件罚分（平台 [0.02, 1.0]，inf ⇒ −23）


# ────────────────────────── DP：朴素式（参考实现）──────────────────────────

def align_naive(S, skip_penalty, unmatched_penalty):
    n, m = S.shape
    dp = np.full((n + 1, m + 1), NEG)
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
    return _backtrack(dp, bk, n, m)


# ────────────────────────── DP：O(n·m) 前缀最大值形式 ──────────────────────────
# 转移 v = dp[i-1][j] + S[i-1][k-1] - skip*(k-1-j)
#          = [S[i-1][k-1] - skip*(k-1)] + [dp[i-1][j] + skip*j]
# 后一项对 j 取前缀最大即可 ⇒ 内层 k 循环摊还 O(1)

def align_fast(S, skip_penalty, unmatched_penalty):
    n, m = S.shape
    dp = np.full((n + 1, m + 1), NEG)
    bk = np.zeros((n + 1, m + 1, 2), dtype=np.int32)
    dp[0, 0] = 0.0
    for i in range(1, n + 1):
        prev = dp[i - 1]
        # 无对应分支
        v_un = prev - unmatched_penalty
        better = v_un > dp[i]
        dp[i][better] = v_un[better]
        for j in np.nonzero(better)[0]:
            bk[i, j] = (j, 0)
        # 匹配分支：前缀最大
        P = np.where(prev > NEG / 2, prev + skip_penalty * np.arange(m + 1), NEG)
        argj = -np.ones(m + 1, dtype=np.int32)
        run_max = NEG
        run_arg = -1
        for k in range(1, m + 1):
            cand = P[k - 1]
            if cand > run_max:
                run_max = cand
                run_arg = k - 1
            if run_max <= NEG / 2:
                continue
            v = S[i - 1, k - 1] - skip_penalty * (k - 1) + run_max
            if v > dp[i, k]:
                dp[i, k] = v
                bk[i, k] = (run_arg, 1)
    return _backtrack(dp, bk, n, m)




# ────────────────────────── 显式重播分段（§7.7 重评，默认关闭）──────────────────────────

def seg_pair_sim(tok, sess, seg_texts):
    """→ (SS, sb)：产出段 × 产出段 余弦相似度 + 段正文列表（**只用产出段自身文本**）。

    产品侧不必重算：`S = Q @ C.T` 里的 `Q` 就是段向量 ⇒ `Q @ Q.T` 即得 SS；
    本台为口径一致走 `similarity_matrix`（同一编码路径）。
    """
    sb = [split_header(t)[1] for t in seg_texts]
    return similarity_matrix(tok, sess, sb, sb), sb


def seg_twins(SS, tau=SEG_TAU, min_lag=SEG_MIN_LAG):
    """→ twin[i] = "i 之前（lag ≥ min_lag）最相似的那一段"的下标（-1 = 无孪生）。

    "孪生"= 该显示之前出现过 ⇒ 重播区的每一段都该有孪生段，重播区之外不该有。
    """
    n = SS.shape[0]
    twin = [-1] * n
    for i in range(n):
        bj, bs = -1, -1.0
        for j in range(0, i - min_lag + 1):
            v = SS[i, j]
            if v > bs:
                bj, bs = j, v
        if bj >= 0 and bs >= tau:
            twin[i] = bj
    return twin


def seg_p5(twin, c, win=SEG_WIN):
    """P5（孪生 onset）= 窗口后侧"有孪生"占比 − 前侧同占比。"""
    n = len(twin)
    aft = list(range(c, min(n, c + win)))
    bef = list(range(max(0, c - win), c))
    if not aft:
        return float("nan")
    fa = sum(1 for i in aft if twin[i] >= 0) / float(len(aft))
    fb = (sum(1 for i in bef if twin[i] >= 0) / float(len(bef))) if bef else 0.0
    return fa - fb


def replay_boundaries(rows, tok, sess, seg_texts, tau=SEG_TAU, win=SEG_WIN,
                      min_lag=SEG_MIN_LAG, min_gap=SEG_MIN_GAP, p5_min=SEG_P5_MIN,
                      min_block=SEG_MIN_BLOCK):
    """→ 重播边界切点列表（**参考无关**；切点 c = 后一块的第一段下标）。

    只消费「产出段文本 + 时间」。SS 的编码是本函数唯一的额外开销（产品侧已有 Q ⇒ 免费）。
    末尾再过一道**最小块长**守卫：丢弃会造成 < `min_block` 段的块的切点（相邻检测点视为
    同一次 onset，**取靠后者**，理由见 SEG_MIN_BLOCK 注释）。
    """
    SS, _ = seg_pair_sim(tok, sess, seg_texts)
    twin = seg_twins(SS, tau, min_lag)
    n = len(rows)
    raw = []
    for c in range(1, n):
        gap = rows[c]["start"] - rows[c - 1]["end"]
        if gap < min_gap:
            continue
        if seg_p5(twin, c, win) >= p5_min:
            raw.append(c)
    out = []
    for c in reversed(raw):                       # 从右往左 ⇒ 相邻时保留靠后者
        if c < min_block or n - c < min_block:
            continue
        if out and out[-1] - c < min_block:
            continue
        out.append(c)
    return sorted(out)


def align_segmented(S, cuts, skip_penalty, unmatched_penalty,
                    repeat_penalty=REPEAT_DEFAULT, reset_penalty=RESET_DEFAULT):
    """按 `cuts` 把 S 切成块，**块内独立对齐**（块间不传递语料下标约束）→ (match, diag)。

    块内仍走现行统一转移 DP（前进/复用/回退/不配）：分段要拿掉的只是**跨块**的下标约束，
    不是块内的局部能力（小回退 c=80/81 就该由块内 `reset` 承担，见 §7.7 重评）。
    """
    n = S.shape[0]
    bnds = [0] + [c for c in sorted(set(cuts)) if 0 < c < n] + [n]
    match = []
    diag = {"repeat": 0, "reset": 0, "forward": 0, "unmatched": 0, "blocks": len(bnds) - 1}
    for a, b in zip(bnds[:-1], bnds[1:]):
        d = {}
        match += align_v2(S[a:b], skip_penalty, unmatched_penalty,
                          repeat_penalty=repeat_penalty, reset_penalty=reset_penalty, diag=d)
        for k in ("repeat", "reset", "forward", "unmatched"):
            diag[k] += d.get(k, 0)
    return match, diag


def count_reward(S, tau=COUNT_TAU, miss_pen=COUNT_MISS_PEN, eps=COUNT_EPS):
    """置信计分矩阵（§7.8②）：把「ΣS − 罚分」的目标函数换成「落在可接受集合内的计数」。

    **不写新 DP**：`align_v2` 只吃矩阵与罚分 ⇒ 换目标 = 换矩阵。
        `R = np.where(S >= tau, 1.0, -miss_pen) + eps * S`
    语义：**+1 层**（S ≥ τ）= "有把握的匹配"；**−miss_pen 层** = "硬塞一个弱匹配"；
    **不配（走 D）= 0，免费** ⇒ "该不配"（truth_ok 为空）从目标函数里自然涌现
    （实测 D=22 ≥ §7.6 掩行 13；但掩码口径下两路持平于 225 ⇒ 掩码并未被取代，
    见 §7.8④）。

    `eps·S`（**ε 破平项**，非设计原型而是实测必需）：纯 0/1 矩阵里所有 ≥τ 的格同值
    ⇒ "配到哪一行"无歧视（同分多路径），vesna 最高只 161/236 且硬门全败；eps 只
    在层内做排序、不改主目标的 0/1 层级。eps=0 可复现设计原型（供对照）。

    输入 S 是**管线矩阵**（含 `mask_empty_body` 的吸引子防护：被封死的格已是 NEG < τ
    ⇒ 自动落入 −miss_pen 层）。其余罚分（skip/repeat/reset）按 0/1 尺度另用
    `COUNT_*` 常量，与 ΣS 尺度的 `DEFAULT`/`REPEAT_DEFAULT`/`RESET_DEFAULT` **互不干扰**。
    """
    return np.where(S >= tau, 1.0, -miss_pen) + eps * S
# 语料取自 OCR，同一条台词可能被收进两次（一次带 OCR 噪音、一次干净）。
# glupov 实测 12 对（1↔22 … 21↔34，相似度 0.974~1.000）、pierro 1 对（[98]↔[99] 完全相同）。
# 此时"命中哪一个下标"在语义上等价，按**下标精确相等**评分会把正确结果判成错——
# 实测 glupov 因此从 22/22 掉到 13/22（且使真值本身非单调，与单调对齐不相容）。
# 故评分改用等价类：预测落在真值所属类内即算对。

DUP_THR = 0.95


def equiv_classes(corpus):
    """→ 每个语料下标（0-based）所属的等价类 id"""
    bodies = [body_norm(t) for t in corpus]
    n = len(corpus)
    parent = list(range(n))

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for i in range(n):
        for j in range(i + 1, n):
            if not bodies[i] or not bodies[j]:
                continue
            if difflib.SequenceMatcher(None, bodies[i], bodies[j]).ratio() >= DUP_THR:
                parent[find(j)] = find(i)
    return [find(i) for i in range(n)]


def score(match, truth_ok, scored, cls, idxs=None):
    """等价类感知 + **集合感知**评分。

    - 段的**可接受集合**非空：预测落在集合内任一元素所属的等价类即算对
      （中英切分不一致时一个显示块可能覆盖多条语料行，两种都算对）；
    - 集合为**空**：正确行为是**不配**（输出转写原文，如英文多出的语气词）
      ⇒ 只有预测为 unmatched 才算对。**这不是缺陷**。
    """
    pos = {k: n for n, k in enumerate(idxs)} if idxs is not None else None
    ok = 0
    for k in scored:
        n = pos[k] if pos is not None else k
        pred = match[n] + 1
        s = truth_ok[k]
        if not s:
            if pred <= 0:
                ok += 1
        elif pred > 0 and any(cls[pred - 1] == cls[t - 1] for t in s):
            ok += 1
    return ok


def ceiling_of(truth_ok, scored, cls):
    """严格递增 DP 的**等价类天花板（集合感知）**：可被同时满足的最大段数。

    每段的可接受集合 = 其 `truth_ok` 各元素所属等价类的全部下标；在"严格递增选择"下
    贪心取每段可用的**最小**下标（留最大余量），贪心对本问题是最大基数最优。
    **空集合的段总是可满足**（选"不配"即可，不改变已用下标）⇒ 不构成约束。
    """
    members = {}
    for j, c in enumerate(cls):
        members.setdefault(c, []).append(j + 1)
    last = 0
    cnt = 0
    for k in scored:
        s = truth_ok[k]
        if not s:
            cnt += 1
            continue
        cand = [x for t in s for x in members[cls[t - 1]] if x > last]
        if cand:
            last = min(cand)
            cnt += 1
    return cnt, len(scored) - cnt


# ────────────────────────── 案例与输入口径 ──────────────────────────

def load_truth(key):
    p = os.path.join(BENCH_OUT, "truth_{}.json".format(key))
    if not os.path.exists(p):
        raise SystemExit("缺少真值 {}（先跑 scripts/fuse_truth.py）".format(p))
    with io.open(p, encoding="utf-8") as f:
        return json.load(f)


def ceiling_of_old(truth, idxs):
    """（保留供对照）下标精确口径的天花板：真值里同一语料行被多段复用时每组至少错 1 段"""
    groups = {}
    for k in idxs:
        t = truth[k]
        if t > 0:
            groups.setdefault(t, []).append(k)
    lost = sum(len(v) - 1 for v in groups.values() if len(v) > 1)
    return len(idxs) - lost, lost


def score_exact(match, truth, idxs):
    return sum(1 for n, k in enumerate(idxs) if match[n] + 1 == truth[k])


# ────────────────────────── 显式重播分段的端到端测量（默认关闭）──────────────────────────

def measure_segmented(keys, tok, sess, data, reset_list=(RESET_DEFAULT, 0.2, 1.0),
                      use_unmatch=False, use_count=False, sp=None, un=None, rp=None):
    """四案例 before/after + reset 罚分敏感性（`--segmented` 才跑；不改默认行为）。

    before = 现行统一转移 DP（整条序列一次对齐）；
    after  = 同一 DP，但先按参考无关检测器切块、块内独立对齐。

    `use_count=True`（§7.8）即对齐矩阵改用 `count_reward`（掩码判据仍在原始 S 上做）；
    `sp/un/rp` 三个参数**显式传入**时使用（0/1 尺度），None ⇒ ΣS 尺度现行值。
    """
    sp = DEFAULT[0] if sp is None else sp
    un = DEFAULT[1] if un is None else un
    rp = REPEAT_DEFAULT if rp is None else rp
    print()
    print("=" * 100)
    print("── 显式重播分段（§7.7 重评 / §7.8 计分目标={}；{}）──".format(
        "开" if use_count else "关",
        "**已启用**" if SEGMENTED_FILTER else "默认关闭（仅对照）"))
    cuts_all = {}
    for key in keys:
        d = data[key]
        cuts = replay_boundaries(d["rows"], tok, sess, d["seg_texts"])
        cuts_all[key] = cuts
        print("   {:<8} 检测边界 {:<16} 时间 {}".format(
            key, str(cuts), ["%.1f" % d["rows"][c]["start"] for c in cuts]))
    print()
    print("   {:<8} {:>8} {:>12} {:>12} {:>10} {:>10}".format(
        "案例", "reset", "before", "after", "回退次数", "分块"))
    for rs in reset_list:
        tot_b = tot_a = tot_n = 0
        for key in keys:
            d = data[key]
            S0 = d.get("S_raw", d["S"]).copy()
            if use_unmatch:
                mask_should_unmatched(S0, d["seg_texts"], d["corpus"])
            S = count_reward(S0) if use_count else S0
            db, da = {}, {}
            mb = align_v2(S, sp, un, repeat_penalty=rp, reset_penalty=rs, diag=db)
            ma, da = align_segmented(S, cuts_all[key], sp, un,
                                     repeat_penalty=rp, reset_penalty=rs)
            cb = score(mb, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
            ca = score(ma, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
            nb = len(d["scored"])
            tot_b += cb
            tot_a += ca
            tot_n += nb
            print("   {:<8} {:>8} {:>7}/{:<4} {:>7}/{:<4} {:>10} {:>10}  {}".format(
                key, "inf" if not np.isfinite(rs) else rs, cb, nb, ca, nb,
                "{}→{}".format(db["reset"], da["reset"]), da["blocks"],
                "" if ca >= cb else " ↓"))
        print("   {:<8} {:>8} {:>7}/{:<4} {:>7}/{:<4}".format(
            "合计", "inf" if not np.isfinite(rs) else rs, tot_b, tot_n, tot_a, tot_n))
    return cuts_all


# ────────────────────────── 主流程 ──────────────────────────

def build_matrices(keys, tok, sess):
    """→ {key: {"S", "idxs", "scored", "truth", "truth_ok", "corpus", ...}}"""
    out = {}
    for key in keys:
        T = load_truth(key)
        corpus = T["corpus"]
        rows = T["rows"]
        truth = [r["truth"] for r in rows]
        # 可接受集合：旧真值文件没有 truth_ok ⇒ 退化为单元素集合（向后兼容）
        truth_ok = [r.get("truth_ok") or ([r["truth"]] if r["truth"] > 0 else [])
                    for r in rows]
        cb = [split_header(t)[1] for t in corpus]
        cls = equiv_classes(corpus)
        ndup = len(corpus) - len(set(cls))
        idxs = list(range(len(rows)))
        seg_texts = [rows[k]["text"] for k in idxs]
        sb = [split_header(rows[k]["text"])[1] for k in idxs]
        S = similarity_matrix(tok, sess, cb, sb)
        # 吸引子防护：空正文语料条目不得被有正文的段命中（见 mask_empty_body）
        masked = mask_empty_body(S, corpus, seg_texts)
        # 计分集合：排除"亚帧残留碎片"（参与对齐但不计分，见 fuse_truth 的 artifact）
        scored = [k for k in idxs if not rows[k].get("artifact")]
        out[key] = {"truth": truth, "truth_ok": truth_ok, "corpus": corpus, "n_raw": len(rows),
                    "cls": cls, "ndup": ndup, "S": S, "idxs": idxs, "seg_texts": seg_texts,
                    "scored": scored, "masked": masked}
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", nargs="*", default=None)
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--unmatch-filter", action="store_true",
                    help="启用「该不配」行掩码（§7.6；默认关闭，标定阶段不改默认行为）")
    ap.add_argument("--segmented", action="store_true",
                    help="启用显式重播分段的 before/after 测量（§7.7 重评；默认关闭）")
    ap.add_argument("--count-objective", action="store_true",
                    help="启用「置信计分」目标函数（§7.8；默认关闭，标定阶段不改默认行为）")
    args = ap.parse_args()

    # 默认案例集与 fuse_truth.CASES / fuse_align_srt.CASES 保持一致（vesna 已是一等案例；
    # 此前本行硬编码三个案例，导致 vesna 从未进入本校准台的任何输出段）
    keys = args.cases or ["moon", "glupov", "pierro", "vesna"]
    tok, sess = load_embedder(os.environ.get("GSA_EMBED_MODEL", "multilingual-e5-small"))
    data = build_matrices(keys, tok, sess)
    # 原始 ΣS 矩阵总是留底：§7.6 掩码判据与 §7.8 的对照都必须在原始尺度上算
    for k in keys:
        data[k]["S_raw"] = data[k]["S"]

    # ── 置信计分目标（§7.8）：**默认关闭**；--count-objective 才换矩阵与参数 ──
    # 「该不配」（unmatched）在 0/1 尺度上免费（=0）；其余罚分亦整套换到 0/1 尺度。
    count_on = args.count_objective or COUNT_OBJECTIVE
    if count_on:
        SP_P, UP_P, RP_P, RSP_P = COUNT_SKIP, 0.0, COUNT_REPEAT, COUNT_RESET
        SCALE_NOTE = "0/1 尺度"
    else:
        SP_P, UP_P, RP_P, RSP_P = DEFAULT[0], DEFAULT[1], REPEAT_DEFAULT, RESET_DEFAULT
        SCALE_NOTE = "ΣS 尺度"

    # ── 显式重播分段（§7.7 重评 / §7.8 专项③）：默认关闭，--segmented 才测 ──
    if args.segmented:
        for k in keys:
            with io.open(os.path.join(BENCH_OUT, "truth_{}.json".format(k)),
                         encoding="utf-8") as f:
                data[k]["rows"] = json.load(f)["rows"]
        rst_list = (RSP_P, 0.2, 1.0, float("inf")) if count_on else \
                   (RESET_DEFAULT, 0.2, 1.0)
        for cnt in ((False,) if not count_on else (False, True)):
            for um in (False, True):
                print()
                print("### 对齐目标 = {}；「该不配」掩码：{}".format(
                    "置信计分" if cnt else "ΣS", "开" if um else "关"))
                measure_segmented(keys, tok, sess, data, reset_list=rst_list,
                                  use_unmatch=um, use_count=cnt,
                                  sp=SP_P, un=UP_P, rp=RP_P)
        if not SEGMENTED_FILTER:
            print()
            print("   （SEGMENTED_FILTER = False ⇒ 上述 after 列只是对照，默认行为未改）")
        return 0

    # ── DP 等价性自检（校准不能悄悄换算法）──
    print("=" * 100)
    print("DP 等价性自检（O(n·m²) 朴素式 vs O(n·m) 前缀最大值式）")
    ok = True
    for key in keys:
        S, idxs = data[key]["S"], data[key]["idxs"]
        for (sp, up) in [(0.02, 0.25), (0.005, 0.1), (0.08, 0.5), (0.0, 0.02)]:
            a = align_naive(S, sp, up)
            b = align_fast(S, sp, up)
            same = a == b
            ok = ok and same
            if not same:
                d = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
                print("  ✗ {} sp={} up={} 差异 {} 处 {}".format(key, sp, up, len(d), d[:4]))
    print("  等价: {}".format("✓ 全部一致" if ok else "✗ 存在不一致"))

    # ── 统一转移模型：B/C 关闭时必须与现行严格递增 DP 逐段等价 ──
    # 这是"新结构零风险上线"的证明：结构换了、行为没换。
    print()
    print("统一转移模型自检（repeat=inf 且 reset=inf ⇒ 应与 align_fast 逐段相同）")
    ok2 = True
    for key in keys:
        S, idxs = data[key]["S"], data[key]["idxs"]
        for (sp, up) in [(0.02, 0.25), (0.005, 0.1), (0.08, 0.5), (0.0, 0.02)]:
            a = align_fast(S, sp, up)
            b = align_v2(S, sp, up)
            same = a == b
            ok2 = ok2 and same
            if not same:
                d = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
                print("  ✗ {} sp={} up={} 差异 {} 处 {}".format(key, sp, up, len(d), d[:4]))
    print("  等价: {}".format("✓ 全部一致" if ok2 else "✗ 存在不一致"))
    ok = ok and ok2
    if args.selfcheck or not ok:
        return 0 if ok else 1

    # ── 置信计分目标 before/after（§7.8；掩码关/开各一列 = 专项②的同台对照）──
    print()
    print("── 置信计分目标函数（§7.8；{}）──".format(
        "**已启用**（矩阵与罚分换到 0/1 尺度）" if count_on else
        "默认关闭（仅对照；--count-objective 才启用）"))
    if count_on:
        print("   τ={} eps={} miss_pen={} skip={} unmatched={} repeat={} reset={}".format(
            COUNT_TAU, COUNT_EPS, COUNT_MISS_PEN, COUNT_SKIP, 0.0,
            COUNT_REPEAT, COUNT_RESET))
        print("   {:<8} {:>6}  {:<18} {:<18} {:<18}".format(
            "案例", "计分段", "ΣS矩阵@计分罚分/掩关", "同+掩码开", "0/1计分矩阵/掩关"))
        for key in keys:
            d = data[key]

            def run_t(M):
                dg = {}
                mm = align_v2(M, SP_P, UP_P, repeat_penalty=RP_P,
                              reset_penalty=RSP_P, diag=dg)
                return score(mm, d["truth_ok"], d["scored"], d["cls"], d["idxs"]), dg

            c0, dg0 = run_t(d["S_raw"])
            S1m = d["S_raw"].copy()
            nm = mask_should_unmatched(S1m, d["seg_texts"], d["corpus"])
            c1, dg1 = run_t(count_reward(S1m))
            c2, dg2 = run_t(count_reward(d["S_raw"]))
            n = len(d["scored"])
            print("   {:<8} {:>6}  {:>8}/{:<4}  {:>8}/{:<4}  {:>8}/{:<4}".format(
                key, n, c0, n, c1, n, c2, n))
            print("           掩行={}  D(不配)：矩阵①={} / 矩阵②={} / 矩阵③={}  repeat/reset/forward：③ {}/{}/{}".format(
                nm, dg0["unmatched"], dg1["unmatched"], dg2["unmatched"],
                dg2["repeat"], dg2["reset"], dg2["forward"]))
        for key in keys:
            data[key]["S"] = count_reward(data[key]["S_raw"])
        print("   （已启用：后续各段（掩码/基线/扫描/网格）均在 0/1 矩阵上测量）")

    # ── 「该不配」判据（§7.6 标定 / **§7.9 修订为 top-K 并列簇**）：**默认关闭**；
    # --unmatch-filter 才真正改变后续结果 ──
    # 两种状态都打印，便于对照；关闭时 data[key]["S"] 保持原样（不改默认行为）。
    # 掩码判据（top-K 并列簇的行）始终在**原始 S** 上算：判据的语义与目标函数解耦，
    # 启用计数目标时矩阵再经 count_reward（§7.8）。
    print()
    print("── 「该不配」行掩码（§7.6 标定 / §7.9 修订；alpha={} K={} tie_eps={}，{}）──".format(
        UNMATCH_ALPHA, UNMATCH_TOPK, UNMATCH_TIE_EPS,
        "**已启用**" if args.unmatch_filter else "默认关闭（仅对照）"))
    print("   {:<8} {:>6}  {:<22} {:<22} {:>6}".format(
        "案例", "掩行", "before 类口径/D", "after 类口径/D", "空集对错"))
    for key in keys:
        d = data[key]
        S = d["S_raw"].copy()
        pos = {k: n for n, k in enumerate(d["idxs"])}
        empt = [k for k in d["scored"] if not d["truth_ok"][k]]

        def run(mat):
            dg = {}
            mm = align_v2(mat, SP_P, UP_P, repeat_penalty=RP_P,
                          reset_penalty=RSP_P, diag=dg)
            cc = score(mm, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
            ne = sum(1 for k in empt if mm[pos[k]] < 0)
            return cc, dg, ne

        c0, dg0, e0 = run(count_reward(S) if count_on else S)
        nmask = mask_should_unmatched(S, d["seg_texts"], d["corpus"])
        c1, dg1, e1 = run(count_reward(S) if count_on else S)
        print("   {:<8} {:>6}  {:>4}/{:<4} (D {:>3})       {:>4}/{:<4} (D {:>3})       {}/{}{} -> {}/{}".format(
            key, nmask, c0, len(d["scored"]), dg0["unmatched"],
            c1, len(d["scored"]), dg1["unmatched"], e0, len(empt),
            "  " if c1 >= c0 else " ↓", e1, len(empt)))
        if args.unmatch_filter:
            data[key]["S"] = count_reward(S) if count_on else S

    # ── 基线（现行初值）──
    print()
    print("── 语料近重复等价类（评分容忍口径）──")
    for key in keys:
        d = data[key]
        print("  {:<8} 语料 {:3} 条 → 等价类 {:3} 个（合并 {} 条重复）".format(
            key, len(d["corpus"]), len(set(d["cls"])), d["ndup"]))
    print()
    print("── 严格递增 DP 基线（skip={}, unmatched={}；回退/复用均禁用，供对照；{}）──".format(
        SP_P, UP_P, SCALE_NOTE))
    for key in keys:
        d = data[key]
        S, idxs = d["S"], d["idxs"]
        m = align_fast(S, SP_P, UP_P)
        sc = d["scored"]
        c = score(m, d["truth_ok"], sc, d["cls"], idxs)
        cx = score_exact(m, d["truth"], idxs)
        ceil, lost = ceiling_of(d["truth_ok"], sc, d["cls"])
        print("  {:<8} 类口径{:3}/{:3} ({:5.1f}%)  下标口径{:3}  天花板{:3} ({:5.1f}%)  mask{:3} 不计分{:2}".format(
            key, c, len(sc), c / len(sc) * 100, cx, ceil, ceil / len(sc) * 100,
            d["masked"], len(idxs) - len(sc)))

    # ── 统一转移模型：B（有界一对多）实测 ──
    # 注意：自 2026-10-08 C（回退）已落地（RESET_DEFAULT = 0.05），故本表**在 C 开启下**扫 B。
    # 实测结论：C 开启后 B 完全休眠（复用次数恒为 0），repeat 取 inf 与 0.25~0.5 分数相同。
    # 扫描表按目标尺度取（0/1 尺度上老罚分全部失效，重扫见 scripts/fuse_count_calib.py）。
    print()
    print("── 统一转移模型 B：有界一对多（repeat_penalty 扫描；{}；reset = {}）──".format(
        SCALE_NOTE, RSP_P))
    REP_SCAN = ([float("inf"), 0.5, 0.4, 0.3, 0.25, 0.2, 0.15, 0.1]
                if count_on else
                [float("inf"), 0.5, 0.4, 0.35, 0.3, 0.28, 0.25, 0.22, 0.2, 0.18, 0.15])
    print("   {:>8}  {:<8} {:>9} {:>7} {:>7} {:>6} {:>6}".format(
        "repeat", "案例", "类口径", "天花板", "复用次数", "前进", "回退"))
    for rp in REP_SCAN:
        for key in keys:
            d = data[key]
            S, idxs = d["S"], d["idxs"]
            diag = {}
            m = align_v2(S, SP_P, UP_P, repeat_penalty=rp, reset_penalty=RSP_P, diag=diag)
            sc = d["scored"]
            c = score(m, d["truth_ok"], sc, d["cls"], idxs)
            ceil, lost = ceiling_of(d["truth_ok"], sc, d["cls"])
            print("   {:>8}  {:<8} {:>4}/{:<4} {:>7} {:>8} {:>6} {:>6}".format(
                "inf" if not np.isfinite(rp) else rp, key, c, len(sc), ceil,
                diag["repeat"], diag["forward"], diag["reset"]))
        # 全案例合计
        tot_c = tot_n = tot_r = 0
        for key in keys:
            d = data[key]
            S, idxs = d["S"], d["idxs"]
            diag = {}
            m = align_v2(S, SP_P, UP_P, repeat_penalty=rp, reset_penalty=RSP_P, diag=diag)
            tot_c += score(m, d["truth_ok"], d["scored"], d["cls"], idxs)
            tot_n += len(d["scored"])
            tot_r += diag["repeat"]
        print("   {:>8}  {:<8} {:>4}/{:<4} {:>7} {:>8}".format(
            "inf" if not np.isfinite(rp) else rp, "合计", tot_c, tot_n, "-", tot_r))

    # ── 统一转移模型：C（回退）落地标定 ──
    # 落地依据的可复现扫描：平台 [0.01, 0.08] 全为最优，0.1 起单调下降，inf = 回退禁用（ΣS 尺度）。
    print()
    print("── 统一转移模型 C：回退标定（{}；repeat = inf，保持落地口径 B 关闭）──".format(SCALE_NOTE))
    print("   {:>8}  {:<8} {:>9} {:>7} {:>6}".format("reset", "案例", "类口径", "回退次数", "前进"))
    RES_SCAN = ([float("inf"), 0.01, 0.02, 0.03, 0.05, 0.08, 0.1, 0.2, 0.5, 1.0]
                if count_on else
                [float("inf"), 0.01, 0.02, 0.03, 0.05, 0.08, 0.1, 0.12, 0.15, 0.2])
    for rs in RES_SCAN:
        for key in keys:
            d = data[key]
            S, idxs = d["S"], d["idxs"]
            diag = {}
            m = align_v2(S, SP_P, UP_P, reset_penalty=rs, diag=diag)
            c = score(m, d["truth_ok"], d["scored"], d["cls"], idxs)
            print("   {:>8}  {:<8} {:>4}/{:<4} {:>8} {:>6}".format(
                "inf" if not np.isfinite(rs) else rs, key, c, len(d["scored"]),
                diag["reset"], diag["forward"]))
        tot_c = tot_n = tot_r = 0
        for key in keys:
            d = data[key]
            diag = {}
            m = align_v2(d["S"], SP_P, UP_P, reset_penalty=rs, diag=diag)
            tot_c += score(m, d["truth_ok"], d["scored"], d["cls"], d["idxs"])
            tot_n += len(d["scored"])
            tot_r += diag["reset"]
        print("   {:>8}  {:<8} {:>4}/{:<4} {:>8}".format(
            "inf" if not np.isfinite(rs) else rs, "合计", tot_c, tot_n, tot_r, "-"))

    # ── 网格搜索 ──
    # 0/1 尺度上 unmatched 的设计值为 0（"不配免费"），单一取值即设计档；
    # 其与 skip 的交互已在 scripts/fuse_count_calib.py 的整套重标里覆盖。
    UP_SCAN = [UP_P] if count_on else GRID_UNMATCHED
    print()
    print("── 网格搜索：skip × unmatched（等价类口径；{}；模型 = 统一转移，repeat={}, reset={}）──".format(
        SCALE_NOTE, RP_P, RSP_P))
    best = None
    table = {}
    for sp in GRID_SKIP:
        for up in UP_SCAN:
            tot_c = tot_n = 0
            per = {}
            for key in keys:
                d = data[key]
                S, idxs = d["S"], d["idxs"]
                m = align_v2(S, sp, up, repeat_penalty=REPEAT_DEFAULT, reset_penalty=RESET_DEFAULT)
                c = score(m, d["truth_ok"], d["scored"], d["cls"], idxs)
                per[key] = (c, len(d["scored"]))
                tot_c += c
                tot_n += len(d["scored"])
            table[(sp, up)] = (tot_c, tot_n, per)
            if best is None or tot_c / tot_n > best[0]:
                best = (tot_c / tot_n, sp, up, per, tot_c, tot_n)
    acc, sp, up, per, tc, tn = best
    print("    最优：skip={:<6} unmatched={:<5} 合计 {}/{} = {:.1f}%".format(
        sp, up, tc, tn, acc * 100))
    for key in keys:
        c, n = per[key]
        print("        {:<8} {:3}/{:3} ({:5.1f}%)".format(key, c, n, c / n * 100))
    # 平台宽度：与最优同分的参数组合数
    plateau = [(k, v) for k, v in table.items() if v[0] == tc]
    print("    同分（合计 {} 段正确）的参数组合：{} 组 / 共 {} 组".format(
        tc, len(plateau), len(table)))
    sps = sorted({k[0] for k, _ in plateau})
    ups = sorted({k[1] for k, _ in plateau})
    print("      skip 取值范围 {} ; unmatched 取值范围 {}".format(sps, ups))
    # 现行初值排名
    cur_key = (SP_P, UP_P)
    cur = table[cur_key] if cur_key in table else table[DEFAULT]
    rank = sum(1 for v in table.values() if v[0] > cur[0]) + 1
    print("    {}：合计 {}/{} = {:.1f}%（并列第 {} 名）".format(
        DEFAULT, cur[0], cur[1], cur[0] / cur[1] * 100, rank))

    # ── 落盘 ──
    out = os.path.join(BENCH_OUT, "calib_grid.json")
    ser = {"{:.4f}|{:.4f}".format(k[0], k[1]): {"correct": v[0], "total": v[1],
                                                "per": v[2]}
           for k, v in table.items()}
    with io.open(out, "w", encoding="utf-8", newline="") as f:
        json.dump({"grid_skip": GRID_SKIP, "grid_unmatched": GRID_UNMATCHED,
                   "default": list(DEFAULT), "results": ser}, f, ensure_ascii=False, indent=1)
    print()
    print("已落盘 {}".format(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
