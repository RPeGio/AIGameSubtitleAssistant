# Third-Party Notices

The source code of this project is licensed under the MIT License (see [LICENSE](LICENSE)).
The third-party components listed below are **not** covered by that license — each remains under its own terms.
Licenses were verified against the official upstream repositories / registries at the time of writing; please consult the linked sources for authoritative and current terms.

Components are grouped by how they are consumed:

- **Build-time dependencies** — compiled into or bundled with the app from the package registries.
- **Runtime components** — downloaded to the local `runtime/` directory by the `scripts/bootstrap_*.ps1` setup scripts when you first set up the AI features. They are **not** distributed with this repository.
- **External requirement** — must be provided by the user; not downloaded by this project.

---

## Build-time dependencies

### Rust crates (`src-tauri/`)

| Component | Purpose | License |
|---|---|---|
| [Tauri](https://github.com/tauri-apps/tauri) 2 (+ dialog / opener plugins) | Desktop app framework | MIT OR Apache-2.0 |
| [serde](https://github.com/serde-rs/serde) / serde_json | Serialization | MIT OR Apache-2.0 |
| [image](https://github.com/image-rs/image) | Frame decoding | MIT OR Apache-2.0 |
| [uuid](https://github.com/uuid-rs/uuid) | ID generation | MIT OR Apache-2.0 |
| [ffmpeg-sidecar](https://github.com/nathanbabcock/ffmpeg-sidecar) | Locating FFmpeg binaries | MIT |

### Frontend packages (`package.json`)

| Component | License |
|---|---|
| [Vue](https://github.com/vuejs/core) / [vue-router](https://github.com/vuejs/router) | MIT |
| [Pinia](https://github.com/vuejs/pinia) | MIT |
| [Naive UI](https://github.com/tusen-ai/naive-ui) | MIT |
| [Vite](https://github.com/vitejs/vite) | MIT |
| [Tailwind CSS](https://github.com/tailwindlabs/tailwindcss) | MIT |
| [TypeScript](https://github.com/microsoft/TypeScript) | Apache-2.0 |

---

## Runtime components (downloaded by bootstrap scripts)

### OCR (scripts/bootstrap_ocr.ps1)

| Component | Purpose | License | Source |
|---|---|---|---|
| CPython 3.12.10 (NuGet package) | Embedded Python interpreter | PSF-2.0 | [python.org](https://www.python.org/) |
| PaddlePaddle 3.2.2 | OCR inference framework | Apache-2.0 | [PaddlePaddle/Paddle](https://github.com/PaddlePaddle/Paddle) |
| PaddleOCR 3.4.0 | OCR engine | Apache-2.0 | [PaddlePaddle/PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) |
| PP-OCRv5 models (mobile/server) | OCR models, auto-cached on first run | Apache-2.0 | PaddleOCR official release |

### ASR — FunASR provider (scripts/bootstrap_funasr.ps1)

| Component | Purpose | License | Source |
|---|---|---|---|
| [FunASR](https://github.com/modelscope/FunASR) | Transcription toolkit | MIT | PyPI `funasr` |
| [Fun-ASR-Nano-2512](https://modelscope.cn/models/FunAudioLLM/Fun-ASR-Nano-2512) | ASR main model (zh/en/ja) | Apache-2.0 | ModelScope / [HF mirror](https://huggingface.co/FunAudioLLM/Fun-ASR-Nano-2512) |
| iic/speech_fsmn_vad (zh-cn-16k) | Voice activity detection | Apache-2.0 | [ModelScope](https://modelscope.cn/models/iic/speech_fsmn_vad_zh-cn-16k-common-pytorch) |
| iic/speech_campplus_sv (zh-cn-16k) | Speaker embedding (fallback engine) | Apache-2.0 | [ModelScope](https://modelscope.cn/models/iic/speech_campplus_sv_zh-cn_16k-common) |
| iic/speech_eres2netv2_sv (zh-cn-16k) | Speaker embedding (alt. engine) | Apache-2.0 | [ModelScope](https://modelscope.cn/models/iic/speech_eres2netv2_sv_zh-cn_16k-common) |
| [PyTorch](https://github.com/pytorch/pytorch) / torchaudio | Tensor runtime | BSD-3-Clause | PyPI / PyTorch index |
| [ModelScope](https://github.com/modelscope/modelscope) | Model download client | Apache-2.0 | PyPI `modelscope` |
| [diarize](https://github.com/FoxNoseTech/diarize) | Speaker diarization pipeline (Silero VAD + WeSpeaker + spectral clustering) | Apache-2.0 | PyPI `diarize` |
| [WeSpeaker](https://github.com/wenet-e2e/wespeaker) runtime + voxceleb_resnet34_LM ONNX | Speaker embedding model | Apache-2.0 | PyPI `wespeakerruntime` |
| [silero-vad](https://github.com/snakers4/silero-vad) | VAD model | MIT | PyPI `silero-vad` |

### ASR — MOSS provider (scripts/bootstrap_moss.ps1)

| Component | Purpose | License | Source |
|---|---|---|---|
| [moss-transcribe.cpp](https://github.com/mudler/moss-transcribe.cpp) | C++ ggml inference binary for MOSS | MIT | Built from source or prebuilt zip |
| MOSS-Transcribe-Diarize 0.9B (GGUF q5_k etc.) | ASR + diarization model | Apache-2.0 | [OpenMOSS/MOSS-Transcribe-Diarize](https://modelscope.cn/models/OpenMOSS/MOSS-Transcribe-Diarize), GGUF: [mudler/moss-transcribe.cpp-gguf](https://huggingface.co/mudler/moss-transcribe.cpp-gguf) |

### LLM (scripts/bootstrap_llm.ps1)

| Component | Purpose | License | Source |
|---|---|---|---|
| [llama.cpp](https://github.com/ggml-org/llama.cpp) (release b10333) | LLM inference binary (`llama-cli`) | MIT | Official GitHub release |
| **Qwen2.5-3B-Instruct (GGUF q4_k_m)** | Text-processing LLM | **Qwen Research License** — ⚠️ see note below | [Qwen/Qwen2.5-3B-Instruct-GGUF](https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF) |

> **Qwen2.5-3B-Instruct license note.** Unlike the other models above, Qwen2.5-3B-Instruct is **not** distributed under a standard open-source license. Its "Qwen Research" license permits research use; **commercial use requires separate permission from Alibaba Cloud**. The terms are in the LICENSE file of the model repository linked above. If your use case is commercial, consider substituting a different model via `runtime/config.json` (`llm_binary` / `llm_model` fields) after checking its license.

---

## External requirement

| Component | Purpose | License |
|---|---|---|
| [FFmpeg](https://ffmpeg.org) / ffprobe | Video metadata, frame extraction, audio extraction | LGPL-2.1+ (may be GPL depending on build flags). Installed by the user; the app only locates it on `PATH` and never downloads it. |

---

## Worker scripts

`scripts/ocr_worker.py` and `scripts/funasr_worker.py` (and their copies under `runtime/worker/`) are part of this project and carry the same MIT license as the rest of the codebase. Pip packages installed alongside them (e.g. `pydantic`, `scikit-learn`, `onnxruntime`, `soundfile`, `tqdm`) are transitive dependencies governed by their own licenses as published on PyPI.
