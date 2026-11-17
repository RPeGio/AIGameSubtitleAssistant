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

> ⚠️ Current version **v1.0.0**, Windows 10/11 x64 only. **Installers are on [Releases](https://github.com/RPeGio/AIGameSubtitleAssistant/releases)**: on first launch the app opens a "Preparing" page and downloads + verifies the AI runtime (embedded Python + OCR / MOSS / vector models, ~4 GB download, ~5 GB on disk, with mirrors and resume). Source development is described in [Quick Start](#quick-start).

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
| FFmpeg + ffprobe | **Bundled** (LGPL shared build, shipped in the installer — see [THIRD_PARTY.md](THIRD_PARTY.md)); run `scripts/fetch_ffmpeg.ps1` in a source checkout; falls back to `PATH` when absent |
| NVIDIA GPU (optional) | MOSS supports cpu / cuda / vulkan (the first-launch self-check picks the best one); without a GPU it falls back to cpu (transcription time increases severalfold) |

The AI modules together occupy about 13.9 GB of disk (models + Python dependencies); the E5-small qint8 model used by vector fusion adds about 113 MB.

## Quick Start

### Option 1: installer (recommended)

Download `GameSubtitleAssistant_1.0.0_x64-setup.exe` (NSIS) from [Releases](https://github.com/RPeGio/AIGameSubtitleAssistant/releases) and launch. On first start a "Preparing" page installs the full runtime automatically; if it fails you can skip into restricted mode and retry later from the welcome page or the settings panel (which also moves the runtime directory and switches download sources).

### Option 2: from source

```powershell
git clone https://github.com/RPeGio/AIGameSubtitleAssistant.git
cd AIGameSubtitleAssistant
pnpm install
```

The AI runtime is provisioned by `scripts/bootstrap_runtime.ps1` according to `src-tauri/resources/runtime/manifest.json` into a `runtime/` directory (gitignored) — embedded Python, OCR dependencies, MOSS binaries and models plus the e5-small vector model in one pass, with SHA256 verification, segmented resume and source probing:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_runtime.ps1
```

Useful parameters: `-Items moss,models-moss,deps-ocr,deps-embed` (install a subset), `-Flavor auto|cpu|cuda|vulkan` (default `auto`, detected locally), `-Source auto|mirror|direct`, `-Mirror ""` (direct; gh-proxy mirror by default), `-PipMirror <index-url>` (PyPI source for CUDA runtime libs), `-SegmentMB <n>` (segment size), `-Force` (reinstall / re-download), `-NoResume` (discard downloaded segments). See the script header and [docs/RUNTIME_BOOTSTRAP_CONTRACT.md](docs/RUNTIME_BOOTSTRAP_CONTRACT.md).

> Retired, not distributed: FunASR (`bootstrap_funasr.ps1`) and the local LLM (`bootstrap_llm.ps1`, llama.cpp + Qwen2.5-3B).
> Both scripts stay in the repo for reproducing historical issues; product builds and the runtime manifest never reference them.

Run in development mode or build:

```powershell
pnpm tauri dev     # development mode
pnpm tauri build   # produce NSIS / MSI installers under src-tauri/target/release/bundle/
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

**Done**: the full four-page workbench (corpus → timing + transcribe → fuse → edit/export), the OCR pipeline (region selection / change detection / majority vote / multi-frame merge), the MOSS ASR engine with speaker diarization (cpu / cuda / vulkan, best backend picked automatically), the hardsub OCR track, AI fusion (local vector recall + unified transition DP, including the **post-fusion confidence audit table**), multi-track timeline editing with undo/redo, single-track multi-format export, autosave, and the **runtime installer with a first-run setup page (installers available since v1.0.0)**.

**In progress / to improve**: further calibration of the fusion confidence signals — `score` is not a correctness signal, so the audit tiers are hints only (see [benchmark/FUSE_THRESHOLD_CALIBRATION.md](benchmark/FUSE_THRESHOLD_CALIBRATION.md) §7.9⑥).

**Planned**: translation assistance and bilingual subtitles, more export formats (Premiere XML / DaVinci Resolve XML / Aegisub), in-app model management, TTS and voice cloning (see Phase 7 of the plan).

## Known limitations

- Windows only; macOS / Linux would require porting the bootstrap scripts and runtime.
- The installer ships the bootstrap script and runtime manifest; **the AI runtime (Python + models + CUDA libs) is downloaded on first launch** (~4 GB download / ~5 GB on disk). Offline machines need a prepared `runtime/` directory; the settings panel can move it to another drive.
- FFmpeg/ffprobe are bundled (LGPL shared build, shipped in the installer); a source checkout that skipped `scripts/fetch_ffmpeg.ps1` falls back to an `ffmpeg` on `PATH`.
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
