<div align="center">

# GameSubtitleAssistant

[![Rust](https://img.shields.io/badge/build_with-Rust-orange)](https://rust-lang.org/)
[![Vue](https://img.shields.io/badge/Vue-3.5+-4FC08D.svg)](https://vuejs.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2.0-yellow)](https://tauri.app/)
[![Platform](https://img.shields.io/badge/Platform-Windows-blue.svg)](https://www.microsoft.com/zh-cn/windows/)
[![License](https://img.shields.io/badge/License-MIT-green.svg)](https://mit-license.org/)

**A fully offline AI workstation for producing game quest subtitles.**

**English** | [中文](README.md)

</div>

GameSubtitleAssistant is built for game quest fan-translators, clip makers and subtitle groups. It turns the manual workflow — watch the recording → (screenshot OCR → copy text → find character names)(a.k.a. corpus collection) → hand-timed subtitles → translate → export — into a single AI-assisted pipeline:

```
Video input ──► OCR + timing & transcription (speech ASR / on-screen hardsub OCR)
            ──► AI fusion (text replacement) ──► manual correction ──► SRT / ASS / LRC / TXT
```

### **No account, no API keys, no video uploads, no cloud services** — all AI inference (OCR / ASR / speaker diarization / multilingual vector-recall alignment) runs **locally**.
### [(Jump to Usage)](#usage)

> ⚠️ Early-stage project (v0.3.0). Windows only, no installer yet: the AI runtime is set up locally via scripts (see [Quick Start](#quick-start)).

## Features

- **Text corpus collecting (OCR)**: frame a subtitle region on the quest recording; use change detection to skip unchanged frames; PaddleOCR recognition with majority-vote merging produces game text decoupled from the timeline. Subtitle screenshots can also be OCR'd directly, or text can be pasted / imported if text corpuses has already existed in any platform.
- **Timing & transcription**: the transcript side of the clip (the timing source) has **two parallel options** — they may coexist in one project, and fusion consumes both. **Speech ASR** (voiced quests: timing comes from speech activity): the engine is MOSS-Transcribe-Diarize 0.9B (automatic diarization, cpu / cuda / vulkan with automatic best-backend selection; FunASR is retired and no longer distributed). Tag each speaker track as "game speech" (provides a timeline for fusion) or "streamer speech" (transcription / export only) in one click.
- **Hardsub OCR (embed_ocr)** (unvoiced quests: timing comes from on-screen text changes): OCR a framed hardsub region on the clip video to build an embedded-subtitle track, coexisting with ASR tracks and fed to fusion as game-content segments.
- **AI fusion**: reliable corpus text replaces the transcript-side text while the transcript-side timing is kept. It is implemented as **local multilingual vector recall (multilingual-e5-small qint8, ONNX / CPU) + a unified transition DP**: each transcript segment recalls candidates from the corpus, a global monotonic DP (advance / reuse / reset / unmatch) then picks the overall best path, and matched segments copy the corpus text **verbatim** in code (no character names are produced). The four benchmark cases score **210/236** scored segments, rising to **226/236 (95.8%)** with the "should-not-match" mask enabled. After fusion the same page shows a **confidence audit table** listing suspicious segments by risk tier (masked / reset / reused / unmatched / low score) with four manual verdicts (**keep / replace from candidates / pick from the full corpus / mark as unmatched**), persisted with the project. The former LLM route (llama.cpp + Qwen2.5-3B index mapping) measured **no cross-language semantic alignment** (see [benchmark/FUSE_PIPELINE_DEFECTS.md](benchmark/FUSE_PIPELINE_DEFECTS.md) F13) and has been retired.
- **Timeline editing**: multi-track timeline (split / merge / drag / snap / segment swap), video preview sync, undo & redo, per-event proofreading list.
- **Export**: single-track export to SRT / ASS / LRC / TXT.
- **Engineering**: project data (`<project-name>.gsa` — a magic header + JSON) autosaves with a 1-second debounce plus Ctrl+S manual save; recent-projects list.

## Workflow

The app is organized as a four-page workbench, navigated in order from the sidebar:

```
① Corpus        ──► ② Timing + transcribe ──► ③ Fuse            ──► ④ Edit
  quest-recording    ASR (voiced) /           reliable text          proofread +
  OCR / screenshots / hardsub OCR             replaces transcript    export the final
  manual text         (unvoiced)              (timing kept)          subtitles
```

A project mounts **two video sources**: `source` (the quest recording — the OCR corpus target, decoupled from the timeline) and `clip` (the video being subtitled — the global timeline reference). Fusion output goes into a single "final subtitles" track, proofread and exported in the editor.

## Requirements

| Dependency | Notes |
|---|---|
| Windows 10/11 x64 | The currently only supported platform (embedded Python is win_amd64, scripts are PowerShell) |
| [Node.js](https://nodejs.org/) 18+ (20 LTS recommended) + [pnpm](https://pnpm.io/) | Frontend build |
| Rust stable (MSVC toolchain) | Desktop build; see [Tauri prerequisites](https://tauri.app/start/prerequisites/) |
| FFmpeg + ffprobe | Install yourself and add to `PATH`; the app never downloads it |
| NVIDIA GPU (optional) | MOSS supports cpu / cuda / vulkan (the first-launch self-check picks the best one); without a GPU it falls back to cpu (transcription time increases severalfold) |

The AI modules together occupy about 13.9 GB of disk (models + Python dependencies); the E5-small qint8 model used by vector fusion adds about 113 MB.

## Quick Start

```powershell
git clone https://github.com/RPeGio/AIGameSubtitleAssistant.git
cd AIGameSubtitleAssistant
pnpm install
```

The AI runtime is provisioned by the PowerShell bootstrap scripts under `scripts/`. They install the embedded Python, dependencies and models into a `runtime/` directory (gitignored), downloading official models with SHA256 verification:

```powershell
# ① OCR (required: used by the corpus page and the hardsub track; also the Python base for the others)
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr.ps1

# ② ASR: MOSS (the only engine; cpu backend by default; a GPU binary is either self-built via build_moss.ps1
#     or installed by the app's "install runtime" action, which picks the backend automatically)
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_moss.ps1 [-Backend cpu]

# ③ Vector fusion (required by AI fusion; CPU inference, model ≈ 113 MB)
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_embed.ps1
```

> Retired, not distributed: FunASR (`bootstrap_funasr.ps1`) and the local LLM (`bootstrap_llm.ps1`, llama.cpp + Qwen2.5-3B).
> Both scripts stay in the repo for reproducing historical issues; product builds and the runtime manifest never reference them.

Useful parameters: `-HfMirror https://hf-mirror.com` (model mirror), `-GitMirror https://gh-proxy.com/` (GitHub mirror), `-PipMirror https://mirrors.aliyun.com/pypi/simple` (pip mirror), `-Force` (reinstall / re-download). See each script's header comment for details.

Run in development mode or build:

```powershell
pnpm tauri dev     # development mode
pnpm tauri build   # produce installers (note: runtime-resource bundling is not configured yet, see Known Limitations)
```

## Usage

1. **Create a project**: "Create New Project" on the welcome page; pick a save folder (project data lives in `<project-name>.gsa` inside it; multiple projects may share one folder).
2. **Import videos**: import the quest recording (source) on the corpus page; import the clip video (clip) on the timing & transcription page.
3. **① Corpus**: frame the subtitle region and time range, run OCR to collect reliable text — or OCR subtitle screenshots / paste plain text.
4. **② Timing + transcribe**: pick the option that fits the material (both may be used) — for voiced quests run ASR (engine / speaker cap / language) and tag tracks containing game speech as "game speech"; for unvoiced quests (or clips that already carry hardsubs) frame the hardsub region and run hardsub OCR.
5. **③ Fuse**: once corpus and timeline are both ready, start fusion to get final subtitle events whose timing comes from the transcript / embed-subtitle and text from the reliable corpus; the **audit table** on the same page then reviews them by risk tier (keep / replace from candidates / pick from the full corpus / mark as unmatched).
6. **④ Edit**: proofread event by event (edit text / character, split, merge, delete) and export SRT / ASS / LRC / TXT.

## Project layout

```
├── src/                    # Vue 3 frontend (views / components / stores / router)
├── src-tauri/              # Rust backend
│   └── src/
│       ├── video/          # ffprobe metadata, frame/audio extraction
│       ├── ocr/            # OCR pipeline (frame → change detection → OCR → merge)
│       ├── asr/            # ASR dispatch (dual engine)
│       ├── llm/            # llama.cpp invocation (fusion-capability test bench)
│       ├── fuse/           # AI fusion (vector recall + unified transition DP)
│       ├── export/         # SRT/ASS/LRC/TXT export
│       ├── project/        # project data model & persistence
│       └── ai_runtime/     # AI runtime (provider abstraction, config, worker subprocesses)
├── scripts/                # bootstrap scripts (bootstrap_*.ps1) and Python workers
├── runtime/                # local AI runtime (script-generated, gitignored)
└── docs/
│   ├── GameSubtitleAssistant_Plan.md  # project plan
```

## Documentation

- [GameSubtitleAssistant_Plan.md](docs/GameSubtitleAssistant_Plan.md) — vision, architecture design and phased plan (Chinese)
- [docs/development.md](docs/development.md) — development guide: architecture / data model / pipeline details / testing (Chinese)
- [benchmark/README.md](benchmark/README.md) — quality benchmarks and the validation record for the fusion-alignment route
- [CONTRIBUTING.md](CONTRIBUTING.md) — contributing guide
- [THIRD_PARTY.md](THIRD_PARTY.md) — third-party components & model licenses

## Roadmap

**Done**: the full four-page workbench (corpus → timing + transcribe → fuse → edit/export), the OCR pipeline (region selection / change detection / majority vote / multi-frame merge), the MOSS ASR engine with speaker diarization (cpu / cuda / vulkan, best backend picked automatically), the hardsub OCR track, AI fusion (local vector recall + unified transition DP, including the **post-fusion confidence audit table**), multi-track timeline editing with undo/redo, single-track multi-format export, autosave.

**In progress / to improve**: runtime-resource bundling (installer distribution); further calibration of the fusion confidence signals — `score` is not a correctness signal, so the audit tiers are hints only (see [benchmark/FUSE_THRESHOLD_CALIBRATION.md](benchmark/FUSE_THRESHOLD_CALIBRATION.md) §7.9⑥).

**Planned**: translation assistance and bilingual subtitles, more export formats (Premiere XML / DaVinci Resolve XML / Aegisub), in-app model management, TTS and voice cloning (see Phase 7 of the plan).

## Known limitations

- Windows only; macOS / Linux would require porting the bootstrap scripts and runtime.
- No installer — the AI environment is set up by scripts; models are downloaded at setup time, not by the app (which only performs readiness checks).
- FFmpeg must be provided by the user (added to `PATH`).
- Fusion output still needs per-event manual proofreading: at any threshold the precision of `score` stays close to the error base rate (4.2%), so it is **not a correctness signal** and the audit table only provides structural hints (see [benchmark/FUSE_THRESHOLD_CALIBRATION.md](benchmark/FUSE_THRESHOLD_CALIBRATION.md) §7.9⑥). In the "same line, different speakers" case ([F14](benchmark/FUSE_PIPELINE_DEFECTS.md)) the mis-bound row's text is verbatim identical — subtitle content is unaffected — but the audit table's speaker attribution may be wrong; experiments ruled out an algorithm-side fix, so it is left to human judgement.
- The local LLM (llama.cpp + Qwen2.5-3B-Instruct) is **retired and not distributed**: the historical implementation remains only as the fusion-capability test bench, and Qwen uses the **Qwen Research license** (commercial use requires separate permission) — see [THIRD_PARTY.md](THIRD_PARTY.md).

## License

The source code of this project is released under the [MIT License](LICENSE). Third-party dependencies and models (PaddleOCR, FunASR, MOSS, llama.cpp, Qwen, etc.) keep their original licenses — if you download and use them in `runtime/`, you must comply with their respective terms; see [THIRD_PARTY.md](THIRD_PARTY.md).

## Acknowledgements

The AI capabilities of this project stand on the shoulders of:
- [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR)
- [FunASR](https://github.com/modelscope/FunASR)
- [MOSS-Transcribe-Diarize](https://github.com/OpenMOSS) / [moss-transcribe.cpp](https://github.com/mudler/moss-transcribe.cpp)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
- [WeSpeaker](https://github.com/wenet-e2e/wespeaker)
- [Silero VAD](https://github.com/snakers4/silero-vad)
