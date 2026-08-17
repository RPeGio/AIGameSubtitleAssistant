# -*- coding: utf-8 -*-
"""FunASR Worker：一次性子进程模式（MOSS 式），argv[1] = WAV 路径，stdout 输出 JSON 段数组。

用法：python funasr_worker.py <audio.wav>
stdout: [{"start":0.5,"end":2.3,"speaker":"SPK0","text":"..."}, ...]

环境变量（Rust 侧注入）：
  - PYTHONPATH：runtime/deps_funasr（torch/funasr/modelscope 依赖，pip --target 安装）
  - MODELSCOPE_CACHE：runtime/models/funasr（模型缓存，bootstrap 预下载）
  - GSA_FUNASR_DEVICE："cuda"（默认，不可用时自动回退 cpu）| "cpu"
  - GSA_FUNASR_LANGUAGE：识别语言（默认"中文"，Fun-ASR-Nano 支持 中文/英文/日文）

模型组合：Fun-ASR-Nano-2512（识别）+ fsmn-vad（VAD 分段）+ cam++（说话人）+ ct-punc（标点）。
sentence_info 的 start/end 是 VAD 段边界（官方确认可靠，非字符级时间戳），单位毫秒。
"""

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


def main():
    if len(sys.argv) < 2:
        sys.stderr.write("用法: python funasr_worker.py <audio.wav>\n")
        sys.exit(2)
    wav = sys.argv[1]
    device = pick_device(os.environ.get("GSA_FUNASR_DEVICE", ""))
    language = os.environ.get("GSA_FUNASR_LANGUAGE", "中文").strip() or "中文"

    from funasr import AutoModel

    model = AutoModel(
        model="FunAudioLLM/Fun-ASR-Nano-2512",
        trust_remote_code=True,
        vad_model="fsmn-vad",
        spk_model="cam++",
        punc_model="ct-punc",
        device=device,
        disable_update=True,
    )
    res = model.generate(
        input=[wav],
        cache={},
        # GPU 上批量 VAD 段一次解码，官方实测约 1.6x 提速
        batch_size_s=120,
        language=language,
    )

    segments = []
    for sent in res[0].get("sentence_info", []):
        segments.append({
            "start": sent.get("start", 0) / 1000.0,
            "end": sent.get("end", 0) / 1000.0,
            "speaker": "SPK%d" % sent.get("spk", 0),
            "text": sent.get("sentence", "").strip(),
        })
    sys.stdout.write(json.dumps(segments, ensure_ascii=False))


if __name__ == "__main__":
    main()
