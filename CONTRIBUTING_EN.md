# Contributing

**English** | [中文](CONTRIBUTING.md)

Thanks for your interest in GameSubtitleAssistant! The project is in an early stage — issues, discussions and PRs are all welcome.

## Setting up

Follow the [README Quick Start](README_EN.md#quick-start) to install dependencies and provision the AI runtime, then read the [development guide](docs/development_EN.md) for the architecture. Note: integration tests require a provisioned runtime (models and binaries in place).

## Workflow

1. Branch off `dev` or `master`. Naming follows the existing branches:
   - `feat/<topic>`: features (e.g. `feat/asr`, `feat/workflow`)
   - `fix/<topic>`: fixes (e.g. `fix/ocr`)
   - `docs/`, `style/`, `refactor/` prefixes are also used
2. Commit in small steps with **Conventional Commits** (`<type>(<scope>): <subject>`) — the repository history is the best example set:
   - `feat(ocr): merge 多数投票 + 空帧容错 + 孤立短段并入`
   - `fix(timeline): delete key on injected-store timelines + drag follow on auto-scroll`
   - `style(editor): compact row layout to widen text editing area`
   - `refactor(workflow): reuse review-pane timeline for corpus mini timeline`
3. Verify before committing:
   ```powershell
   pnpm build              # vue-tsc type check + vite build
   cd src-tauri; cargo test --release
   ```
4. Open a Pull Request (usually targeting `dev`) describing the motivation and how you verified it.

## Code conventions

- **Rust**: comment core logic and architecture decisions in Chinese — explain "why", not "what"; error messages should be user-readable.
- **TypeScript / Vue**: keep it concise; `src/types/index.ts` mirrors the Rust data structures — new fields go on both sides.
- **Surgical changes**: touch only what your task requires; file an issue for unrelated problems instead of fixing them in passing.
- When adding commands / events across the boundary, update the command list and event table in the [development guide](docs/development_EN.md).

## Reporting issues

When filing an issue, please include:

- App version and Windows version
- Which stage fails (corpus OCR / ASR / fusion / editing & export) and the AI modules in use
- Results of the `check_*` readiness probes and any relevant logs
- For recognition-quality issues: a short reproducible clip or screenshot (mind privacy)

## License & models

By contributing you agree to license your work under [MIT](LICENSE). Note that the models downloaded into `runtime/` (in particular Qwen2.5-3B under the Qwen Research license) carry their own terms, independent of this project's code license — see [THIRD_PARTY.md](THIRD_PARTY.md).
