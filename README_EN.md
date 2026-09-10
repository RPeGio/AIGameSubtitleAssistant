# GameSubtitleAssistant

**English** | [中文](README.md)

> A fully offline AI workstation for producing game quest subtitles.

GameSubtitleAssistant is built for game quest fan-translators, clip makers and subtitle groups. It turns the manual workflow — watch the recording → (screenshot OCR → copy text → find character names)(a.k.a. corpus collection) → hand-timed subtitles → translate → export — into a single AI-assisted pipeline:

```
Video input ──► OCR + ASR + speaker diarization + auto timing
            ──► AI fusion (text replacement) ──► manual correction ──► SRT / ASS / LRC / TXT
```

### **No account, no API keys, no video uploads, no cloud services** — all AI inference (OCR / ASR / speaker diarization / LLM text alignment) runs **locally**.
### [(Jump to Usage)](#usage)

> ⚠️ Early-stage project (v0.1.0). Windows only, no installer yet: the AI runtime is set up locally via scripts (see [Quick Start](#quick-start)).

## Features

- **Text corpus collecting (OCR)**: frame a subtitle region on the quest recording; use change detection to skip unchanged frames; PaddleOCR recognition with majority-vote merging produces game text decoupled from the timeline. Subtitle screenshots can also be OCR'd directly, or text can be pasted / imported if text corpuses has already existed in any platform.
- **Speech transcription (ASR)**: two engines —MOSS-Transcribe-Diarize 0.9B (Auto diarization, and the most accurate) or FunASR (Fun-ASR-Nano, fast on a local GPU). Tag each speaker track as "game speech" (provide timeline for fusion part) or "streamer speech" (transcribing & exporting subtitle only) in one click.
- **Hardsub track**: OCR a framed hardsub region on the clip video to build an embedded-subtitle timeline, in order to adapt non-voiced quests.
- **AI fusion**: a local LLM (llama.cpp + Qwen2.5-3B) performs **cross-language semantic alignment** between the reliable corpus and the game-speech transcript — the model only outputs the correspondence; text is copied verbatim by code, so a small model cannot "paraphrase" it. Character names are extracted from corpus prefixes.
- **Timeline editing**: multi-track timeline (split / merge / drag / snap / segment swap), video preview sync, undo & redo, per-event proofreading list.
- **Export**: single-track export to SRT / ASS / LRC / TXT.
- **Engineering**: project data (`<project-name>.gsa` — a magic header + JSON) autosaves with a 1-second debounce plus Ctrl+S manual save; recent-projects list.

## Workflow

The app is organized as a four-page workbench, navigated in order from the sidebar:

```
① Corpus        ──► ② Transcribe     ──► ③ Fuse              ──► ④ Edit
  quest-recording    clip-video ASR +      reliable text          proofread +
  OCR / screenshots / speaker tagging +    replaces transcript    export the final
  manual text         hardsub OCR          (timing kept)          subtitles
```

A project mounts **two video sources**: `source` (the quest recording — the OCR corpus target, decoupled from the timeline) and `clip` (the video being subtitled — the global timeline reference). Fusion output goes into a single "final subtitles" track, proofread and exported in the editor.

## Requirements

| Dependency | Notes |
|---|---|
| Windows 10/11 x64 | The currently only supported platform (embedded Python is win_amd64, scripts are PowerShell) |
| [Node.js](https://nodejs.org/) 18+ (20 LTS recommended) + [pnpm](https://pnpm.io/) | Frontend build |
| Rust stable (MSVC toolchain) | Desktop build; see [Tauri prerequisites](https://tauri.app/start/prerequisites/) |
| FFmpeg + ffprobe | Install yourself and add to `PATH`; the app never downloads it |
| NVIDIA GPU (optional) | FunASR defaults to CUDA 12.4; llama.cpp / MOSS support cpu / cuda / vulkan backends — use cpu without a GPU(transcription time increases severalfold) |

The AI modules together occupy about 13.9 GB of disk (models + Python dependencies).

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

# ② ASR — pick one or install both: MOSS (recommended; cpu backend by default; a GPU binary must first be self-built via build_moss.ps1)
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_moss.ps1 [-Backend cpu]
#     or FunASR (defaults to -Device cuda)
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_funasr.ps1 [-Device cpu]

# ③ LLM (required by AI fusion)
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_llm.ps1 [-Backend cpu]
```

Useful parameters: `-HfMirror https://hf-mirror.com` (model mirror), `-GitMirror https://gh-proxy.com/` (GitHub mirror), `-PipMirror https://mirrors.aliyun.com/pypi/simple` (pip mirror), `-Force` (reinstall / re-download). See each script's header comment for details.

Run in development mode or build:

```powershell
pnpm tauri dev     # development mode
pnpm tauri build   # produce installers (note: runtime-resource bundling is not configured yet, see Known Limitations)
```

## Usage

1. **Create a project**: "Create New Project" on the welcome page; pick a save folder (project data lives in `<project-name>.gsa` inside it; multiple projects may share one folder).
2. **Import videos**: import the quest recording (source) on the corpus page; import the clip video (clip) on the transcribe page.
3. **① Corpus**: frame the subtitle region and time range, run OCR to collect reliable text — or OCR subtitle screenshots / paste plain text.
4. **② Transcribe**: run ASR on the clip video (engine / speaker cap / language), then tag tracks containing game speech as "game speech". For games without voice acting, additionally frame the hardsub region to build an embedded-subtitle track.
5. **③ Fuse**: once corpus and timeline are both ready, start fusion to get final subtitle events whose timing comes from the transcript / embed-subtitle and text from the reliable corpus.
6. **④ Edit**: proofread event by event (edit text / character, split, merge, delete) and export SRT / ASS / LRC / TXT.

## Project layout

```
├── src/                    # Vue 3 frontend (views / components / stores / router)
├── src-tauri/              # Rust backend
│   └── src/
│       ├── video/          # ffprobe metadata, frame/audio extraction
│       ├── ocr/            # OCR pipeline (frame → change detection → OCR → merge)
│       ├── asr/            # ASR dispatch (dual engine)
│       ├── llm/            # llama.cpp invocation
│       ├── fuse/           # cross-language LLM fusion
│       ├── export/         # SRT/ASS/LRC/TXT export
│       ├── project/        # project data model & persistence
│       └── ai_runtime/     # AI runtime (provider abstraction, config, worker subprocesses)
├── scripts/                # bootstrap scripts (bootstrap_*.ps1) and Python workers
├── runtime/                # local AI runtime (script-generated, gitignored)
└── GameSubtitleAssistant_Plan.md  # project plan
```

## Documentation

- [GameSubtitleAssistant_Plan.md](GameSubtitleAssistant_Plan.md) — vision, architecture design and phased plan (Chinese)
- [docs/development.md](docs/development.md) — development guide: architecture / data model / pipeline details / testing (Chinese)
- [CONTRIBUTING.md](CONTRIBUTING.md) — contributing guide
- [THIRD_PARTY.md](THIRD_PARTY.md) — third-party components & model licenses

## Roadmap

**Done**: the full four-page workbench (corpus → transcribe → fuse → edit/export), the OCR pipeline (region selection / change detection / majority vote / multi-frame merge), FunASR + MOSS dual ASR engines with speaker diarization, the hardsub track, cross-language LLM fusion, multi-track timeline editing with undo/redo, single-track multi-format export, autosave.

**In progress / to improve**: real-world long-text evaluation and tuning of the fusion pipeline; runtime-resource bundling (installer distribution).

**Planned**: translation assistance and bilingual subtitles, more export formats (Premiere XML / DaVinci Resolve XML / Aegisub), in-app model management, TTS and voice cloning (see Phase 7 of the plan).

## Known limitations

- Windows only; macOS / Linux would require porting the bootstrap scripts and runtime.
- No installer — the AI environment is set up by scripts; models are downloaded at setup time, not by the app (which only performs readiness checks).
- FFmpeg must be provided by the user (added to `PATH`).
- Fusion quality is bounded by small local models; long-text behavior is still being tuned.
- The default LLM, Qwen2.5-3B-Instruct, uses the **Qwen Research license** (commercial use requires separate permission) — see [THIRD_PARTY.md](THIRD_PARTY.md).

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
