# Third-Party Notices

The source code of this project is licensed under the MIT License (see [LICENSE](LICENSE)).
The third-party components listed below are **not** covered by that license — each remains under its own terms.
Licenses were verified against the official upstream repositories / registries at the time of writing; please consult the linked sources for authoritative and current terms.

Components are grouped by how they are consumed:

- **Build-time dependencies** — compiled into or bundled with the app from the package registries.
- **Runtime components** — downloaded to the local `runtime/` directory by the `scripts/bootstrap_*.ps1` setup scripts when you first set up the AI features. They are **not** distributed with this repository.
- **Bundled binaries** — fetched by a setup script and shipped inside the installer. Each keeps its own license, and the notices it requires are reproduced below and travel with the binaries themselves.

---

## Build-time dependencies

### Rust crates (`src-tauri/`)

| Component | Purpose | License |
|---|---|---|
| [Tauri](https://github.com/tauri-apps/tauri) 2 (+ dialog / opener plugins) | Desktop app framework | MIT OR Apache-2.0 |
| [serde](https://github.com/serde-rs/serde) / serde_json | Serialization | MIT OR Apache-2.0 |
| [image](https://github.com/image-rs/image) | Frame decoding | MIT OR Apache-2.0 |
| [uuid](https://github.com/uuid-rs/uuid) | ID generation | MIT OR Apache-2.0 |
| [base64](https://github.com/marshallpierce/rust-base64) | Base64 encoding of JPEG frames on the OCR IPC path | MIT OR Apache-2.0 |

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

### Embedding / vector recall (scripts/bootstrap_embed.ps1)

| Component | Purpose | License | Source |
|---|---|---|---|
| [onnxruntime](https://github.com/microsoft/onnxruntime) 1.20.1 (CPU) | ONNX inference runtime for sentence embeddings | MIT | PyPI `onnxruntime` |
| [tokenizers](https://github.com/huggingface/tokenizers) | Tokenizer for the embedding model | Apache-2.0 | PyPI `tokenizers` |
| [numpy](https://github.com/numpy/numpy) | Array math | BSD-3-Clause | PyPI `numpy` |
| [huggingface_hub](https://github.com/huggingface/huggingface_hub) | Model / config download | Apache-2.0 | PyPI `huggingface_hub` |
| [multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small) (ONNX qint8 variant) | Multilingual sentence embeddings for the vector-recall alignment route | MIT | Hugging Face / [intfloat](https://huggingface.co/intfloat) |

> These components are consumed by the **product fusion pipeline** (local vector recall + unified transition DP, landed as T4 in 2026-10) as well as by the offline fusion-alignment lab; `scripts/bootstrap_embed.ps1` installs them into `runtime/deps_embed/` and `runtime/models/embed/`. See `docs/GameSubtitleAssistant_Plan.md` §6.3.

---

## Bundled binaries

### FFmpeg / ffprobe (LGPL shared build)

| Component | Purpose | License | Source |
|---|---|---|---|
| [FFmpeg](https://ffmpeg.org) 8.1 — `ffmpeg.exe`, `ffprobe.exe` and the `av*`/`sw*` DLLs in `resources/ffmpeg/` | Video metadata, frame extraction, audio extraction | LGPL-2.1-or-later | [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), tag `autobuild-2026-10-28-19-47`, asset `ffmpeg-n8.1.3-25-g611cec792a-win64-lgpl-shared-8.1.zip`, SHA-256 `0448c3f3b9bdc724cc0189d5ce6e6bfa5b8937e7ec408e1e97fb08719bdb3b92` |

Why this particular build:

- **`-lgpl-shared`, not the default GPL build.** The builds most packagers publish by default enable GPL-only components (libx264/x265 and friends); shipping one of those would put the entire application under the GPL. This build enables neither `--enable-gpl` nor `--enable-nonfree`.
- **Shared, not static.** DLLs are the LGPL §4 route with the smallest obligations: the user may replace the FFmpeg libraries with their own build. We also do not link against FFmpeg at all — `ffmpeg.exe` / `ffprobe.exe` are invoked as separate processes — so nothing of ours needs relinking.
- **DLLs ship in plain sight**, next to the executables, in `resources/ffmpeg/`. Replacing them with a self-built set of the same major version is sufficient to swap in a modified FFmpeg.
- **License text and provenance travel with the binaries**: `resources/ffmpeg/LICENSE.txt` is the LGPL text extracted from the same archive, and `resources/ffmpeg/SOURCE.txt` records the exact URL, asset name, SHA-256 and fetch time. Both are installed alongside the binaries.
- **Corresponding source**: the sources for this exact build are published by BtbN under the release tag above; `scripts/fetch_ffmpeg.ps1` pins tag + asset + SHA-256 so the same sources can always be re-fetched.
- **Nothing is downloaded at runtime.** The binaries are fetched at build time by `scripts/fetch_ffmpeg.ps1` and shipped inside the installer. If `resources/ffmpeg/` is absent (e.g. a developer checkout that skipped the script), the app falls back to an `ffmpeg` on `PATH` and otherwise reports `FFMPEG_NOT_FOUND`.

---

## Worker scripts

`scripts/ocr_worker.py` and `scripts/funasr_worker.py` (and their copies under `runtime/worker/`) are part of this project and carry the same MIT license as the rest of the codebase — as are the fusion calibration scripts (`scripts/fuse_*.py`, `src-tauri/tests/fuse_lab/`). Pip packages installed alongside them (e.g. `pydantic`, `scikit-learn`, `onnxruntime`, `soundfile`, `tqdm`) are transitive dependencies governed by their own licenses as published on PyPI.
