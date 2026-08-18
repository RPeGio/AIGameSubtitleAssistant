# -*- coding: utf-8 -*-
"""FunASR Worker：一次性子进程模式（MOSS 式），argv[1] = WAV 路径，stdout 输出 JSON 段数组。

用法：python funasr_worker.py <audio.wav>
stdout: [{"start":0.5,"end":2.3,"speaker":"SPK0","text":"..."}, ...]

环境变量（Rust 侧注入）：
  - PYTHONPATH：runtime/deps_funasr（torch/funasr/modelscope 依赖，pip --target 安装）
  - MODELSCOPE_CACHE：runtime/models/funasr（模型缓存，bootstrap 预下载）
  - GSA_FUNASR_DEVICE："cuda"（默认，不可用时自动回退 cpu）| "cpu"
  - GSA_FUNASR_LANGUAGE：识别语言（空/auto = 自动检测；Fun-ASR-Nano-2512 支持 中文/英文/日文）
  - GSA_FUNASR_SPK_MODEL：说话人模型完整 id（默认 cam++，可换 iic/speech_eres2netv2_sv_zh-cn_16k-common）

模型组合（完整 id，不用 "fsmn-vad"/"cam++" 别名，防版本映射漂移）：
  Fun-ASR-Nano-2512（识别，zh/en/ja，自带标点与 LLM 语义断句）
   + speech_fsmn_vad_zh-cn-16k-common-pytorch（VAD 分段）
   + speech_campplus_sv_zh-cn_16k-common（说话人，默认）
重要：不要给 Fun-ASR-Nano 配 punc_model —— 它自带标点，二次标点会导致
句子边界错乱、时间戳与说话人分配错误（FunASR issue #2857，官方确认）。
断句粒度由 Nano 的 LLM 语义理解 + VAD 段边界共同决定。
sentence_info 的 start/end 单位毫秒。
"""

import io
import json
import os
import sys


def pick_device(want):
    """解析设备请求：cuda 优先，不可用时回退 cpu（torch 未装/无 GPU 均可）"""
    want = (want or "cuda").strip().lower()
    if want.startswith("cuda"):
        try:
            import torch

            if torch.cuda.is_available():
                return "cuda:0"
        except Exception:
            pass
        sys.stderr.write("[funasr] CUDA 不可用，回退 CPU\n")
    return "cpu"


def split_long_segment(s):
    """把过长的 VAD 段按句末标点切成子句（断句后处理）。

    Fun-ASR-Nano 的 text 自带 LLM 语义断句与标点，但 sentence_info 的
    粒度是 VAD 段（连续语音段可达 30s+，单条字幕会超过 100 词）。
    这里用段内字级 timestamp 按标点边界切分子句并映射时间。
    短段（<=12s）原样返回，避免过度切碎。
    返回段 dict 列表（start/end 为秒）。
    """
    text = s.get("sentence", "").strip()
    ts = s.get("timestamp") or []
    dur_s = (s.get("end", 0) - s.get("start", 0)) / 1000.0
    if dur_s <= 12.0 or not text or len(ts) < 2:
        return [{
            "start": s.get("start", 0) / 1000.0,
            "end": s.get("end", 0) / 1000.0,
            "speaker": "SPK%d" % s.get("spk", 0),
            "text": text,
        }]
    # 收集句子边界：句末标点后的位置（跳过收尾引号/括号）
    text_len = len(text)
    boundaries = []
    i = 0
    while i < text_len:
        if text[i] in ".!?。！？":
            j = i + 1
            while j < text_len and text[j] in "…\"')]”』」』>),，;；:：":
                j += 1
            boundaries.append(j)
            i = j
        else:
            i += 1
    if len(boundaries) < 2:
        return [{
            "start": s.get("start", 0) / 1000.0,
            "end": s.get("end", 0) / 1000.0,
            "speaker": "SPK%d" % s.get("spk", 0),
            "text": text,
        }]
    # 子句 = 相邻边界之间的文本
    pieces = []
    prev = 0
    for b in boundaries:
        piece = text[prev:b].strip()
        if piece:
            pieces.append((piece, prev, b))
        prev = b
    if prev < text_len:
        tail = text[prev:].strip()
        if tail:
            pieces.append((tail, prev, text_len))
    if len(pieces) < 2:
        return [{
            "start": s.get("start", 0) / 1000.0,
            "end": s.get("end", 0) / 1000.0,
            "speaker": "SPK%d" % s.get("spk", 0),
            "text": text,
        }]

    # 字符位置 → ts 索引：按比例近似映射（ts 是 token 级，与字符数
    # 不严格等长，但句边界偏差通常 <=1 token，对字幕可接受）
    n_ts = len(ts)

    def ts_at(char_idx):
        idx = int(round(char_idx * n_ts / text_len))
        return min(max(idx, 0), n_ts - 1)

    out = []
    for piece, c0, c1 in pieces:
        # 丢弃纯标点子句（LLM 输出省略号等残片，如 "HOW CAN I..." 的尾部 ".."）
        if all(ch in "….,。！？!?;；:：\"'()[]“”‘’" for ch in piece):
            continue
        t0 = ts[ts_at(c0)][0]
        t1 = ts[ts_at(max(c1 - 1, c0))][1]
        out.append({
            "start": t0 / 1000.0,
            "end": max(t1 / 1000.0, t0 / 1000.0 + 0.05),
            "speaker": "SPK%d" % s.get("spk", 0),
            "text": piece,
        })
    return out if out else [{
        "start": s.get("start", 0) / 1000.0,
        "end": s.get("end", 0) / 1000.0,
        "speaker": "SPK%d" % s.get("spk", 0),
        "text": text,
    }]


def main():
    # 强制 UTF-8：中文 locale Windows 下 stdout 默认 GBK，Rust 侧按 UTF-8 解析会乱码
    stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
    # Python 3.12 移除了 distutils；PYTHONPATH 目录的 .pth 不被自动执行，
    # 需手动激活 setuptools 的 distutils shim（funasr 部分模型模块仍 import distutils）
    try:
        import _distutils_hack

        _distutils_hack.add_shim()
    except Exception:
        pass
    if len(sys.argv) < 2:
        sys.stderr.write("用法: python funasr_worker.py <audio.wav>\n")
        sys.exit(2)
    wav = sys.argv[1]
    device = pick_device(os.environ.get("GSA_FUNASR_DEVICE", ""))
    language = os.environ.get("GSA_FUNASR_LANGUAGE", "").strip()
    spk_model = os.environ.get(
        "GSA_FUNASR_SPK_MODEL", "iic/speech_campplus_sv_zh-cn_16k-common"
    ).strip()

    from funasr import AutoModel

    # 模型用完整 id（不用 "fsmn-vad"/"cam++" 别名）：
    # 别名映射随 funasr 版本变动，bootstrap 预下载与这里必须一一对应
    # 注意：不配 punc_model（Nano 自带标点，二次标点会破坏句子边界）
    model = AutoModel(
        model="FunAudioLLM/Fun-ASR-Nano-2512",
        trust_remote_code=True,
        vad_model="iic/speech_fsmn_vad_zh-cn-16k-common-pytorch",
        vad_kwargs={"max_single_segment_time": 30000},
        spk_model=spk_model,
        device=device,
        disable_update=True,
    )
    gen_kwargs = dict(
        input=[wav],
        cache={},
        # GPU 上批量 VAD 段一次解码，官方实测约 1.6x 提速
        batch_size_s=120,
    )
    # language 进 prompt（"请转写为X"）；空 = 自动检测，多语言切片不硬编码语言
    if language and language.lower() not in ("auto", "none", ""):
        gen_kwargs["language"] = language
    res = model.generate(**gen_kwargs)

    segments = []
    for sent in res[0].get("sentence_info", []):
        for piece in split_long_segment(sent):
            if piece["text"]:
                segments.append(piece)
    stdout.write(json.dumps(segments, ensure_ascii=False))


if __name__ == "__main__":
    main()
