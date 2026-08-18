# -*- coding: utf-8 -*-
"""FunASR Worker：一次性子进程模式（MOSS 式），argv[1] = WAV 路径，stdout 输出 JSON 段数组。

用法：python funasr_worker.py <audio.wav>
stdout: [{"start":0.5,"end":2.3,"speaker":"SPK0","text":"..."}, ...]

环境变量（Rust 侧注入）：
  - PYTHONPATH：runtime/deps_funasr（torch/funasr/modelscope 依赖，pip --target 安装）
  - MODELSCOPE_CACHE：runtime/models/funasr（模型缓存，bootstrap 预下载）
  - GSA_FUNASR_DEVICE："cuda"（默认，不可用时自动回退 cpu）| "cpu"
  - GSA_FUNASR_LANGUAGE：识别语言（空/auto = 自动检测；Fun-ASR-Nano-2512 支持 中文/英文/日文）
  - GSA_FUNASR_MAX_SPEAKERS：说话人上限（空 = 自动估计；非法值忽略）
  - GSA_FUNASR_SPK_ENGINE：说话人分离引擎，"diarize"（默认）| "funasr"
    - diarize：独立说话人分离流水线（Silero VAD + WeSpeaker ResNet34-LM + 谱聚类，
      外网多语言切片效果远好于 funasr 内建聚类；CPU 运行，约 8 倍实时）。
      与 ASR 段按时间重叠分配说话人。不可用时自动回退 funasr 引擎。
      注意：Nano 必须配 spk_model 才会输出 sentence_info（断句依赖），
      所以 spk_model 照常加载，但说话人标签由 diarize 覆盖。
    - funasr：AutoModel 内建聚类（GSA_FUNASR_SPK_MODEL 选模型）
  - GSA_FUNASR_SPK_MODEL：说话人模型完整 id（默认 cam++，
    可换 iic/speech_eres2netv2_sv_zh-cn_16k-common）

模型组合（完整 id，不用 "fsmn-vad"/"cam++" 别名，防版本映射漂移）：
  Fun-ASR-Nano-2512（识别，zh/en/ja，自带标点与 LLM 语义断句）
   + speech_fsmn_vad_zh-cn-16k-common-pytorch（VAD 分段）
  diarize 引擎：speech_campplus_sv_zh-cn_16k-common（说话人，回退引擎默认）
重要：不要给 Fun-ASR-Nano 配 punc_model —— 它自带标点，二次标点会导致
句子边界错乱、时间戳与说话人分配错误（FunASR issue #2857，官方确认）。
断句粒度由 Nano 的 LLM 语义理解 + VAD 段边界共同决定。
sentence_info 的 start/end 单位毫秒。

VAD 分段参数（vad_kwargs）：
  - max_single_segment_time=30000：单段上限 30s，防超长字幕
  - max_end_silence_time=300：静音 300ms 即切段（默认 800ms）。
    游戏 BGM 下说话人切换停顿通常 <800ms，默认值会把多人对话合并成
    长段（实测 14.8s 段混 3 人 → 说话人分配必然错），调小后按 MOSS
    参考（whisper silero VAD 切分）粒度一致，说话人正确率大幅提升。
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


def assign_speakers(segments, diar_segments):
    """把 diarize 说话人时间线映射到 ASR 段。

    每个 ASR 段取与之重叠时长最大的说话人段；无重叠（VAD 边界差异）时
    取中心点最近的说话人段。SPEAKER_XX 标签按首次出现顺序重映射为
    SPK0/SPK1...，与 funasr 内建 spk 编号习惯一致（Rust 侧格式不变）。
    diar_segments: [{"start":s,"end":e,"speaker":"SPEAKER_XX"}, ...]
    """
    if not diar_segments:
        return segments
    label_map = {}
    label_order = []
    for d in diar_segments:
        lab = d["speaker"]
        if lab not in label_map:
            label_map[lab] = len(label_order)
            label_order.append(lab)
    for seg in segments:
        s0, e0 = seg["start"], seg["end"]
        best = max(
            diar_segments,
            key=lambda d: (min(e0, d["end"]) - max(s0, d["start"])),
        )
        if min(e0, best["end"]) - max(s0, best["start"]) <= 0:
            c = (s0 + e0) / 2.0
            best = min(
                diar_segments,
                key=lambda d: abs((d["start"] + d["end"]) / 2.0 - c),
            )
        seg["speaker"] = "SPK%d" % label_map[best["speaker"]]
    return segments


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
    spk_engine = os.environ.get("GSA_FUNASR_SPK_ENGINE", "diarize").strip().lower()
    spk_model = os.environ.get(
        "GSA_FUNASR_SPK_MODEL", "iic/speech_campplus_sv_zh-cn_16k-common"
    ).strip()

    # diarize 引擎不可用（未安装/导入失败）时回退 funasr 内建引擎
    diarize_enabled = spk_engine == "diarize"
    if diarize_enabled:
        try:
            from diarize import diarize as diarize_fn
        except Exception as e:
            sys.stderr.write("[funasr] diarize 不可用（%s），回退 funasr 内建说话人引擎\n" % e)
            diarize_enabled = False

    from funasr import AutoModel

    # 模型用完整 id（不用 "fsmn-vad"/"cam++" 别名）：
    # 别名映射随 funasr 版本变动，bootstrap 预下载与这里必须一一对应
    # 注意：不配 punc_model（Nano 自带标点，二次标点会破坏句子边界）
    # 说话人分离交给 diarize 时仍需 spk_model：Fun-ASR-Nano 只有配置 spk_model
    # 才输出 sentence_info（含段级 start/end 与字级 timestamp，断句后处理依赖），
    # 加载的 spk 结果会被 diarize 引擎覆盖，不影响最终 speaker 标签。
    model_kwargs = dict(
        model="FunAudioLLM/Fun-ASR-Nano-2512",
        trust_remote_code=True,
        vad_model="iic/speech_fsmn_vad_zh-cn-16k-common-pytorch",
        vad_kwargs={"max_single_segment_time": 30000, "max_end_silence_time": 300},
        spk_model=spk_model,
        device=device,
        disable_update=True,
    )
    model = AutoModel(**model_kwargs)
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

    # diarize 引擎：说话人时间线与 ASR 段按重叠分配（失败时降级，全部标 SPK0）。
    # 默认不传 max_speakers（自动估计，默认 1-20）：曾误加 max_speakers=4 压制，
    # 实测 6 人对话被压成 4 簇（应用里说话人一团糟）。
    # 短音频（<1min）上 GMM BIC 估计会失效（30s 截段 2 人被判 13 人），
    # 但完整转写场景音频数分钟，全片自动估计与 MOSS 参考（6 人）接近（5 人）。
    # GSA_FUNASR_MAX_SPEAKERS：前端面板"最大说话人数量"（用户确知人数时用，
    # 强制聚类上界；非法值忽略走自动估计）
    max_spk = None
    env_max_spk = os.environ.get("GSA_FUNASR_MAX_SPEAKERS", "").strip()
    if env_max_spk:
        try:
            max_spk = int(env_max_spk)
        except ValueError:
            sys.stderr.write(
                "[funasr] GSA_FUNASR_MAX_SPEAKERS=%r 不是整数，忽略（自动估计）\n"
                % env_max_spk
            )
    if diarize_enabled:
        try:
            if max_spk:
                diar_res = diarize_fn(wav, max_speakers=max_spk)
            else:
                diar_res = diarize_fn(wav)
            segments = assign_speakers(segments, diar_res.to_list())
        except Exception as e:
            sys.stderr.write("[funasr] diarize 运行失败（%s），说话人全部标 SPK0\n" % e)
    stdout.write(json.dumps(segments, ensure_ascii=False))


if __name__ == "__main__":
    main()
