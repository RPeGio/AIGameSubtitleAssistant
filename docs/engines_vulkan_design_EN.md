# Vulkan Optimization for MOSS ASR and the GPU Backend Fallback Design

> Status: **steps 1–5 all done** (script pre-checks / Vulkan build measured / three-device benchmark / probing & fallback semantics / docs & wording verified) (decisions confirmed by the user on 2026-10-18, see §0.2)
> Scope of this branch: **the Vulkan path for MOSS ASR plus backend probing / fallback semantics only**
> Branch: `feat/engines-gpu-vulkan` (this worktree `AIGameSubtitleAssistant-gpu-vulkan`), baseline `7b8e306`
> Explicitly **out of scope**: LLM Vulkan (an official path already exists; the user decided to leave it alone), OCR / fusion embeddings on GPU (no upstream Vulkan path, see §1), FunASR (out of scope for this branch, see §7)

---

## 0. Summary and confirmed decisions

### 0.1 Summary

1. **MOSS is the only engine worth doing, and it genuinely can be done.** `MTD_DEVICE` is a real, effective instruction — `MTD_DEVICE=cpu` was measured to switch the backend to CPU — and on the Vulkan side ggml supports `-DMT_GGML_VULKAN=ON`.
2. **Two real gaps.** ① This machine has no `glslc` (Vulkan SDK), so `scripts/build_moss.ps1 -Backend vulkan` is guaranteed to fail at the cmake configure stage today. ② The project has no logic at all for "probing the installed backend / explaining why the GPU was not used".
3. **A dangerous behaviour (measured).** When asked for a backend that is *valid but unavailable*, `moss-transcribe` **degrades silently, exits 0, and prints no warning**. The only usable criterion is therefore textual evidence of the *actually effective* backend — never the exit code.
4. **Fallback semantics = Option A.** Only one set of ggml backends can exist in the asset layer at a time (`ggml.dll` collides by name), so "fallback" means **probing the installed backend and telling the user which one to install**, not switching at runtime.

### 0.2 Confirmed decisions (2026-10-18)

| # | Decision | Notes |
|---|---|---|
| ① | **Adopt Option A.** The ideal shape is that the **first-launch self-check probes the machine and auto-selects a backend**; but that self-check is being implemented by the parallel session, so this branch delivers **the probing logic itself** (reusable by that self-check) | see §3.1 |
| ② | **An explicitly requested backend that fails is a hard error** — the user should be told to untick GPU acceleration. Only `auto` may fall back automatically | see §3.4. The GPU-acceleration parameterisation (config fields + front-end panel) is implemented by the parallel session |
| ③ | **No LLM probe.** The MOSS `info` probe is zero-cost and serves as the primary criterion | see §3.2 |
| ④ | **Dual GPU (iGPU + dGPU) is a risk that must be verified specifically after the build**; record a memo | see §3.3 |
| ⑤ | External wording: **Vulkan optimization is only stated to apply to ASR.** OCR is acceptable on CPU; do not disclose technical details that would harm the product | see §4.4 |

---

## 1. Why the scope can only be MOSS (background matrix)

| Component | Carrier | Existing GPU path | Vulkan | This branch |
|---|---|---|---|---|
| **ASR MOSS** | moss-transcribe.cpp (ggml), self-built | CUDA (`.backend=cuda`, actually runs `backend: CUDA0`) | ✅ ggml supports it, **the Vulkan build has been built successfully** (§2.5) | **✅ do it** |
| LLM | llama.cpp prebuilt zip | CUDA (`ggml-cuda.dll`, 512.9 MB) | ✅ official prebuilt (`llama-b10333-bin-win-vulkan-x64.zip`, verified to exist) | ❌ not done (③) |
| ASR FunASR (fallback) | PyTorch + Fun-ASR-Nano | CUDA cu124 | ❌ no Vulkan backend in torch | ❌ not done (§7) |
| OCR | PaddleOCR + PaddleX | CUDA (`deps_gpu`, 3.5 GB) | ❌ no Vulkan backend in Paddle Inference | ❌ not done (⑤) |
| Fusion embeddings | onnxruntime | CPU only | ❌ no Vulkan EP in the official ORT list | ❌ not done |

> The LLM Vulkan path **needs no changes to be usable**: `scripts/bootstrap_llm.ps1:23` defaults `-Version` to `b10333`, the asset name composed at `:53` (`llama-b10333-bin-win-vulkan-x64.zip`) was verified against the GitHub releases API (34,147,775 bytes, sha256 `117bca1f594cffd96a77703aa23d98450800bc22022a307d1e3567187fbb50fe`), and the self-check at `:204-207` matches `Vulkan0`.

---

## 2. Measured facts (this machine, 2026-10-18)

### 2.1 Environment

| Item | Value |
|---|---|
| GPUs | NVIDIA RTX 4060 Laptop (driver 572.83, 8188 MiB, CC 8.9) + Intel RaptorLake-S iGPU (driver 101.5542) |
| Virtual display adapters | OrayIddDriver, Todesk Virtual Display (2 of them) |
| Vulkan runtime | `C:\Windows\System32\vulkan-1.dll` present; `vulkaninfo --summary` exits 0; loader 1.4.303 |
| Vulkan enumeration | **4 device records**: `DISCRETE_GPU` (NVIDIA, apiVersion 1.4.303) ×1 + `INTEGRATED_GPU` (Intel, 1.3.280) **×3** |
| `$env:VULKAN_SDK` / `glslc` | **empty / not on PATH** (no Vulkan SDK) |
| Available tools | `vulkaninfo` (system32), `cmake`, `ninja`, `nvcc` (`D:\CUDA-Toolkit\bin`), `winget` |
| Python | system `D:\Python\python.exe` (3.13); `py -3.12` points at the main worktree's `runtime\python\python.exe` |
| PowerShell | **Windows PowerShell 5.1 only** (`5.1.26100.9711`, no pwsh 7) ⇒ this repo's `scripts/*.ps1` must stay **UTF-8 with BOM**: 5.1 decodes a BOM-less file as GBK, so Chinese comments break parsing outright (hit this round — an editing tool rewrote the file without the BOM and the script immediately exited 1) |

### 2.2 How MOSS selects its backend (the decisive measurement)

Read-only probes (`GSA_RUNTIME_DIR` → the main worktree's `runtime/`; no file was written):

| Probe | Actual output | Exit code |
|---|---|---|
| `MTD_DEVICE` unset | `[mt I] backend: CUDA0` | 0 |
| `MTD_DEVICE=cpu` | `[mt I] CPU threads: 24` + `[mt I] backend: CPU` | 0 |
| `MTD_DEVICE=vulkan` (on a CUDA-only build) | **no warning at all**, `[mt I] backend: CUDA0` | **0** |
| `MTD_DEVICE=bogus` | `[mt W] backend: MTD_DEVICE=bogus requested but no matching device registered — falling back` + `backend: CUDA0` | 0 |

The binary contains exactly two custom environment variables: **`MTD_DEVICE`** and **`MTD_THREADS`** — precisely the two set by `src-tauri/src/ai_runtime/moss.rs:372,376`.

**Three conclusions:**

1. **`MTD_DEVICE` is a real instruction** — `cpu` genuinely takes effect, and `cuda`/`vulkan` live in the same value space. The work is therefore "probe correctly, choose the right value, make it visible", not "invent a mechanism".
2. **Failure is silent and the exit code is useless** — requesting `vulkan` while actually running on CUDA produces no hint whatsoever. ⇒ the effective backend must be parsed.
3. **Logs go to stderr** (`moss.rs:3` already documents "stdout emits the JSON segment array, stderr is logs"): `info` actually writes **two** lines to stdout: `arch=…` and `audio_token_id=… mel_bins=80 n_fft=400 hop=160`; `backend: <X>` goes to **stderr**. ⚠ Lesson learned: I first captured only the line containing `arch=`, concluded "there is only one line", and even doubted the binary's provenance on that basis — **a filtered single line lies**; only the complete `info` stdout is evidence.

**Why it stays silent (confirmed in upstream source)**: `name_to_type()` in `src/backend.cpp` (:34-45) treats `vulkan` as a **valid name** and maps it to `GGML_BACKEND_DEVICE_TYPE_GPU`; `ggml_backend_dev_by_type(GPU)` (`ggml-backend-reg.cpp:348-356`, **first match wins**) then returns `CUDA0` on a CUDA-only build. Because `g_backend` is non-null, the single warning site (`backend.cpp:90-92`) is skipped. `bogus` warns only because `name_to_type` returns false. ⇒ **the real meaning of `vulkan` is "any device of GPU type", not "must be Vulkan"**: the silence is by design, not an environment problem.

### 2.3 Build prerequisite: `glslc` and how to obtain it

- ggml's Vulkan backend hard-requires `find_package(Vulkan COMPONENTS glslc REQUIRED)` and `find_package(SPIRV-Headers CONFIG REQUIRED)` at **configure time**, and the shader generator invokes `--glslc`. **The SDK is needed only to build; at runtime only the graphics driver is needed.**
- ~~No SDK on this machine~~ → **installed `KhronosGroup.VulkanSDK` 1.4.363.0 on 2026-10-19**: the installer already set `VULKAN_SDK=C:\VulkanSDK\1.4.363.0` and put `Bin` on PATH (so `Get-Command glslc` resolves). `winget` also offers `KhronosGroup.VulkanRT`, which is runtime-only and does **not** include glslc.
- **The `SPIRV-Headers` CMake CONFIG package *is* shipped with the SDK**, but it lives at `<SDK>\Lib\cmake\SPIRV-Headers\SPIRV-HeadersConfig.cmake` — **not** under `share\cmake` (the SDK root also has `cmake\`, `share\`, `Source\`, `Include\spirv-headers\`). CMake expands `CMAKE_PREFIX_PATH=<SDK>` into a search over `<prefix>/[share|lib|Lib]/cmake/<name>/`, so ggml's `find_package(SPIRV-Headers CONFIG REQUIRED)` does resolve. **This branch's prerequisite check originally probed only `share\cmake` and would have raised a false warning on a correctly installed SDK; it now searches `Lib\cmake` / `lib\cmake` / `share\cmake` recursively for `SPIRV-HeadersConfig.cmake`, and the measured output is `找到 SPIRV-Headers CONFIG 包: …\Lib\cmake\SPIRV-Headers\SPIRV-HeadersConfig.cmake`.**
- `scripts/build_moss.ps1:9` mentions in its header comment that "Vulkan needs glslc + the Vulkan SDK", but **there is no check in the code** (only `-Backend cuda` looks for `nvcc`) ⇒ without the SDK the user gets lost in a cryptic cmake error. **Added in step 1 (see §4-1).**

### 2.4 Zero-cost probe primitives (verified usable)

| Primitive | Purpose | Measured |
|---|---|---|
| `nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader` | CUDA availability | exits 0, prints `NVIDIA GeForce RTX 4060 Laptop GPU, 572.83, 8188 MiB` |
| `vulkaninfo --summary` | Vulkan device count and types | exits 0; `deviceType = PHYSICAL_DEVICE_TYPE_{INTEGRATED,DISCRETE}_GPU` is regex-parseable |
| stderr of `moss-transcribe info <model>` | **the actually effective backend** | `[mt I] backend: CUDA0` |
| `Get-CimInstance Win32_VideoController` | GPU list without depending on Vulkan (fallback / cross-check) | lists 4 adapters (including 2 virtual displays) |

### 2.5 Vulkan build measured (2026-10-19): successful, and the artifact layout matches the CUDA build's shape

- **Environment**: Vulkan SDK 1.4.363.0 + VS2022 BuildTools (MSVC) + cmake 4.3.3 (default VS generator).
- **Artifacts** (`runtime/bin/`): `moss-transcribe.exe` (274,432 B) + `ggml.dll` (67 KB) + `ggml-base.dll` (639 KB) + `ggml-cpu.dll` (887 KB) + **`ggml-vulkan.dll` (46 MB)**; `.backend=vulkan`. ⇒ moss's CMake **does enable `GGML_BACKEND_DL`** — the §5-4 worry that "there may be no `ggml-vulkan.dll`" does not hold. The DLLs land in `build-vulkan/bin/Release/` (CMake `RUNTIME_OUTPUT_DIRECTORY`), and the script's multi-level probe (`…\Release\ → …\bin\Release → …\bin → next to the exe`) correctly hits it.
- **Upstream's "Windows build failure" (issue #1: MinGW + GCC 16, non-Vulkan path) does not apply to the MSVC route** (§5-9).
- **The only obstacle is MAX_PATH, not an upstream defect**:
  - The deepest transient path in the build tree is `…\build-vulkan\vulkan-shaders-gen-prefix\src\vulkan-shaders-gen-build\CMakeFiles\CMakeScratch\TryCompile-…\cmTC_….dir\Debug\cmTC_….tlog\link-cvtres.read.1.tlog`; the tail measured **208 characters**, independent of the work root; with `MAX_PATH=260` the **work root must be ≤ ~50 characters**, otherwise `FileTracker error FTK1011` (the default `%TEMP%\gsa_moss_build_<PID>`, 53 characters, was measured to cross the line).
  - ⇒ **the build directory must not be moved into the repo**: the repo path itself is already 58 characters, 58+208=266 ✗.
  - The CUDA build is unaffected: it uses Ninja, there is no vulkan-shaders-gen ExternalProject layer, and its tail is ~70 characters shorter — which is why CUDA never tripped on this.
  - Fix already in `build_moss.ps1`: default work root is now `<system drive>\gsa-moss-build\<backend>-<PID>` (30 characters), plus a `-WorkDir` override (accepts an **empty** directory only, to prevent accidental deletion) and a length warning when root length + 208 > 250.

---

## 3. Design

### 3.1 Asset-layer semantics: Option A (probe + point the way)

**Hard constraint:** llama.cpp's and moss's `ggml.dll` share a name and are mutually incompatible (DLLs load from the exe's own directory first), so `runtime/bin/` can hold only one ggml backend at a time. In practice it currently holds `.backend=cuda`, and there is no `ggml-vulkan.dll` anywhere in the repo.

⇒ **"Fall back from CUDA to Vulkan automatically" is impossible in the asset layer** (unless `bin/` becomes `bin/<backend>/` with several copies; cost: disk ×N, slower startup probing, a doubled build matrix — **not done on this branch**). What this branch delivers is:

> **Probe what the machine can run → check whether the installed binaries match → when they do not, tell the user plainly which backend to install.**

For end users this surfaces through the parallel session's first-launch self-check (①): select automatically where possible, otherwise tell the user — in words a human can act on — which command to run.

### 3.2 Three probe levels and their criteria

| Level | Content | Cost | Purpose |
|---|---|---|---|
| **L0 Machine capability** | CUDA: `nvidia-smi` exits 0; Vulkan: `vulkan-1.dll` exists **and** `vulkaninfo --summary` yields ≥1 `*_GPU` | milliseconds | compared against installed binaries to produce "which backend to install" |
| **L1 Asset consistency** | does `moss-transcribe.exe` exist, and does the `.backend` marker agree with the expectation; a DLL name works as a secondary criterion (measured `GGML_BACKEND_DL=ON`: the Vulkan build has `ggml-vulkan.dll`, the CUDA build has `ggml-cuda.dll`, §2.5) | filesystem | catches mismatches fast |
| **L2 Effective-backend verification** | run `moss-transcribe info <model>`, parse `[mt I] backend: <X>` from **stderr**, compare exactly against the expectation (`CUDA0`/`Vulkan0`/`CPU`); **on a multi-device backend the placement must be verified too**: `Vulkan<N>` is only an enumeration index — cross-check the `ggml_vulkan: <N> = <device name> \| uma: <0/1>` table on stderr (`uma:0` is usually the dGPU) — the `bootstrap_moss.ps1` self-check now includes this placement check | zero-cost (③; `info` measured to be fast) | **the only trustworthy criterion**; catches the silent degradation of §2.2 |

- ⚠ **The exit code must never be used as a criterion** (measured: exit 0 while degrading).
- ⚠ **Match the full name** (`CUDA0`, not `CUDA`): Rust's `str::contains` is case-sensitive, whereas `scripts/bootstrap_moss.ps1:175` currently only works by accident because PowerShell's `-match` is case-insensitive — unify this while we are here.
- The L0 Vulkan criterion must account for the fact that **`vulkaninfo` reports the same Intel iGPU three times** (4 records for 2 physical GPUs): **judge by `deviceType == DISCRETE_GPU` for preference, and deduplicate by `deviceName+driverInfo` before counting** — never count records.

### 3.3 Dual GPU (iGPU + dGPU) — mechanism settled; verify this way after the build (④)

**Upstream mechanism (confirmed in source)**: ggml's Vulkan backend pushes devices in `vkEnumeratePhysicalDevices()` **return order** at `ggml-vulkan.cpp:6814-6859` and **never reorders to prefer a discrete GPU**; devices are named `Vulkan0/Vulkan1/…` (`:17779`) and their type comes from `ctx->is_integrated_gpu` (`:16996` — iGPU maps to `IGPU`, dGPU to `GPU`). MOSS pins ggml at `ggml-org/ggml` `eced84c8` (**v0.15.3, 2026-06-26**), which **already contains** llama.cpp PR #15947 (2025-09-12), i.e. "expose iGPU and dGPU by default".

**Enumeration order on this machine**: `vulkaninfo --summary` lists the Intel iGPU first and the RTX 4060 second. The four settings therefore land as follows:

| `MTD_DEVICE` | Path taken | Lands on (this machine) |
|---|---|---|
| **unset (auto)** | `ggml_backend_init_best()`, priority **GPU → IGPU → CPU** | **RTX 4060** (the iGPU is typed `IGPU`, so `GPU` never matches it) ✅ |
| `vulkan` | `init_named()` substring-matches the device name `Vulkan0` | **Intel iGPU** ❌ |
| `Vulkan0` | substring-matches `Vulkan0` | Intel iGPU ❌ |
| `Vulkan1` | substring-matches `Vulkan1` | RTX 4060 ✅ (but the **index varies per machine — never hardcode it**) |

**Measured with the Vulkan build (2026-10-19, artifacts of §2.5, `info` stderr)**: the table above is now verified — **unset = `Vulkan1` = RTX 4060** (enumeration shows `uma: 0`); **bare `vulkan` = `Vulkan0` = the Intel iGPU** (`uma: 1`); `Vulkan0`/`Vulkan1` pin their respective cards; `GGML_VK_VISIBLE_DEVICES=1` leaves only `0 = RTX 4060`, at which point bare `vulkan` also lands on the dGPU. ggml's own stderr enumeration (the `ggml_vulkan: Found 2 Vulkan devices:` table) agrees with `vulkaninfo`: iGPU index 0, dGPU index 1, iGPU `uma:1 / bf16:0`, dGPU `uma:0 / warp size:32`.

**Conclusions (this corrects an earlier guess in this design)**:

1. **Do not set a bare `MTD_DEVICE=vulkan`** — on a Vulkan build it hits whichever device is enumerated first (here, the iGPU). It only "looks correct" on a CUDA-only build because it lands on `dev_by_type(GPU)` = `CUDA0` (§2.2).
2. **`auto` (empty) is the best choice**: `ggml_backend_init_best()`'s type priority naturally avoids the iGPU.
3. To **pin a specific card**, use `GGML_VK_VISIBLE_DEVICES=<original enumeration index>` to narrow the device set to the target card (which then becomes `Vulkan0`) together with `MTD_DEVICE=vulkan` — more robust than `MTD_DEVICE=Vulkan<N>`, which depends on enumeration order. Note the index is the **raw index from `vkEnumeratePhysicalDevices`**, not the name shown by `vulkaninfo`.
4. **MOSS creates a single backend** (the `g_backend` singleton in `src/backend.cpp`) and does no multi-device layer splitting ⇒ "the model is split across two cards" does not apply to MOSS, and the upstream 3× collapse (llama.cpp #30695) cannot hit us either — plus the PR that introduced it (#29741, 2026-10-13) postdates our pin.
5. MOSS's CLI has **no `--device`**: `MTD_DEVICE` and `GGML_VK_VISIBLE_DEVICES` are the only two channels.

**Placement verification is done (④, 2026-10-19)**: all four rows above measured to match (see above). The `bootstrap_moss.ps1` readiness check now includes the placement check (enumeration echo + `uma:1` warning; the config value is written as auto, §5-11); the welcome page's ASR self-check card shows the configured device. The transcription timing comparison has been measured too (step 3: CPU 680.6 s / Vulkan 134.7 s / CUDA 76.2 s, see §4-3 and `docs/development_EN.md`).

### 3.4 An explicitly requested backend that fails is an error (②)

| User intent | Probe result | Behaviour |
|---|---|---|
| `auto` (or empty) | CUDA binaries present | use CUDA, record a `reason` |
| `auto` (or empty) | no CUDA binaries but Vulkan binaries present | use Vulkan, record a `reason` |
| `auto` (or empty) | neither | use CPU, record a `reason`, **visible in the self-check panel** |
| explicit `cuda` / `vulkan` | that backend is unavailable (missing DLL, or L2 verification failed) | **hard error**, telling the user to "untick GPU acceleration to use CPU" or "install the matching backend"; **never fall back automatically** |

Rationale: the MOSS gap between CPU and GPU is an order of magnitude (the canonical record is 306 s → 50.4 s, roughly 6×); silent degradation would leave users believing they are on the GPU. The config fields and the front-end toggle belong to the parallel session (②); this branch only provides the decision function and its return value.

### 3.5 Interface with the parallel session (boundary)

- **This branch delivers**: `scripts/` (build prerequisites, probing), the **pure probing/decision logic** on the Rust side, and `docs/`.
- **The parallel session delivers**: field extensions in `src-tauri/src/ai_runtime/config.rs` (the `auto` semantics, `llm_device`, …) plus the front-end configuration / self-check panels.
- **Contract to align on** (so the two sides do not invent separate schemes): the value set `"" | "auto" | "cuda" | "vulkan" | "cpu"`; and the shape of the decision result (`{backend, reason, source}`).

---

## 4. Implementation steps (this branch, confirmed round by round)

1. ✅ **Done** (2026-10-18): `scripts/build_moss.ps1` gained an `if ($Backend -eq "vulkan")` branch — `glslc` is looked up via PATH → `VULKAN_SDK` → `C:\VulkanSDK\<version>`, and on a hit `VULKAN_SDK`/PATH are set so cmake can locate it; on a miss it joins `$missing` and **throws before clone/build** (with the `winget install --id KhronosGroup.VulkanSDK -e` hint), plus a pre-emptive warning about a missing `SPIRV-Headers` CONFIG package. Meanwhile the backend assertion in `scripts/bootstrap_moss.ps1`'s readiness check was corrected to parse `[mt I] backend: <device>` (the old form only worked by accident because `-match` is case-insensitive, and it took the `[mt W] …requested but no matching device` degradation warning for the result).
   → Verified (all passed): without the SDK, `-Backend vulkan` errors out before cloning with zero side effects (no `runtime/` created); with a fake SDK it finds `glslc` and prints the SPIRV-Headers warning; the real binary's `info` output parses to `CUDA0` for cpu/cuda/vulkan requests (cuda matches, cpu/vulkan are correctly flagged as degraded); with `MTD_DEVICE=bogus` the old form took the warning text as the result while the new one still reads `CUDA0`. **"Configure succeeds once the SDK is installed" is now confirmed by step 2 (✅).**
2. ✅ **Done** (2026-10-19): `-Backend vulkan` **builds successfully**; artifacts and DLL layout as in §2.5 (`ggml-vulkan.dll` 46 MB is indeed produced = `GGML_BACKEND_DL=ON`; `build-vulkan/bin/Release/` is correctly hit by the script's multi-level probe). Three problems were exposed and fixed along the way:
   - **MAX_PATH**: the default `%TEMP%` work root (53 chars) plus the 208-char TryCompile tail exceeds MAX_PATH (FTK1011) ⇒ `build_moss.ps1`'s default work root is now `<system drive>\gsa-moss-build\<backend>-<PID>` (30 chars), plus a `-WorkDir` override and a length warning (§2.5);
   - **`bootstrap_moss.ps1`'s `moss_device` write defect**: the vulkan backend used to write `"vulkan"` ⇒ on a dual-GPU machine it always lands on the iGPU (§3.3 measured) ⇒ now writes `""` (auto, measured to prefer the dGPU); cpu/cuda writes unchanged;
   - **Placement check in the self-check + welcome-page display**: the bootstrap self-check now echoes the Vulkan enumeration table and warns on a `uma:1` (usually iGPU) placement; the welcome page's ASR self-check card gained a "Device · <device>" row (`AsrEngineStatus.device`, the configured value, `""` shown as "auto (ggml picks the best device)" — it displays the configured value, not the measured backend, since measuring requires spawning a subprocess).
   → Verified (all passed): with a guaranteed-failing clone, the default root (30 chars) prints no warning, an overly long root (89 chars) warns correctly (estimate 297 > 250), and the `-WorkDir` non-empty guard works; the placement-check block parses the real `info` output into `Vulkan1` = RTX 4060 (uma:0); `cargo check` / `vue-tsc --noEmit` pass; script BOM/CRLF re-checked unharmed (§2.1 lesson).
3. ✅ **Done** (2026-10-19): after confirming placement (§3.3 measured), ran the benchmark once per device on `quality_bench_test(voiced)_5min.mp4` (extracted to a 5:14.99 / 16 kHz mono wav with the same parameters as `video::extract_audio`; `moss-transcribe-q5_k.gguf`; `--format json`; `MTD_THREADS` unset): **CPU 680.6 s (`backend: CPU`), Vulkan 134.7 s (`Vulkan1` = RTX 4060), CUDA 76.2 s (`CUDA0`)** — Vulkan ≈ 1.77× CUDA, still 5.0× faster than CPU (CUDA is 8.9×). The transcribed text matches across all three (total chars 3765/3765/3775; first and last segments byte-identical; 84 vs 90 segments is a boundary-granularity difference). The timing comparison is recorded in `docs/development_EN.md` under "MOSS backend and three-device benchmark".
   → Verified: all three runs `exit=0` with stderr backend lines `CPU`/`Vulkan1`/`CUDA0`; the output JSON is a segment array (`id/start/end/speaker/text`) in all cases.
4. ✅ **Done** (2026-10-19): probing logic + fallback semantics landed — new module `src-tauri/src/ai_runtime/backend_probe.rs` (pure parsing/decision functions + L0/L1 probes, 15 unit tests; config.rs untouched):
   - **L2 evidence**: `parse_info_stderr()` trusts only `[mt I] backend:` lines (the `[mt W]` degradation warning is not mistaken for a result); `parse_vulkan_devices()` parses the `ggml_vulkan: N = … | uma: …` enumeration table;
   - **L0/L1**: `probe_machine()` (`nvidia-smi -L`; `System32\vulkan-1.dll` + `vulkaninfo --summary` deduplicated by deviceName+driverInfo — this machine enumerates the iGPU 3 times), `probe_assets()` (`.backend` marker + ggml-{cuda,vulkan}.dll);
   - **Decision** (§3.4 matrix): `decide()` — explicit cpu is always available; an explicit cuda/vulkan request that **cross-family mismatches** the L2 measurement is a hard error (with "run scripts/build_moss.ps1 -Backend …" or "switch back to auto" guidance; when L2 is missing, falls back to asset inference); auto follows L2, and a Vulkan placement on a `uma:1` device appends an iGPU hint to the reason; `recommend_from_assets()` powers the self-check's "which backend to install" hint;
   - **Wiring**: `moss.rs` probe() runs with the same parameters as run_transcribe (MTD_DEVICE set only for explicit devices) → the L2 measurement matches the backend actually used at transcribe time; the decision is stored in `backend_decision` and exported through the new trait default method `effective_backend()` (only moss overrides it); `AsrEngineStatus` gains an `effective_backend` field (the welcome ASR card shows "设备 · <config>（实测 <L2>）").
   → Verified: `cargo test --lib` 273 pass (including the 14 new unit tests: degradation warning not mistaken, cross-family hard error, same-family iGPU warns only, triple iGPU enumeration dedup, etc.); `vue-tsc --noEmit` passes. The full §3.4 behaviour matrix is pinned by unit tests whose fixtures are the verbatim stderr of §2.2/§2.4.
5. ✅ **Done** (2026-10-19): documentation and wording —
   - `docs/development.md` gains the section "MOSS backend semantics and fallback (`backend_probe.rs`)" (`moss_device` value semantics, the L0/L1/L2 decision chain, Option A fallback semantics = explicit cross-family hard error / auto may sink, `effective_backend` → welcome-page measured display, Vulkan build entry and the MAX_PATH hint); the `moss_device` row of the `config.json` key-field table updated in step; `docs/development_EN.md` mirrored;
   - **⑤ wording check (README.md:59 / README_EN.md:60 / welcome-card copy)**: current copy is already compliant — GPU wording stays within ASR/LLM facts ("transcription time increases severalfold" refers to ASR; the OCR card is a neutral "local Python worker"), no unfavourable OCR disclosure, **zero changes needed**.
   → Verified: repo-wide grep of `GPU|vulkan|CUDA` over both READMEs and Welcome.vue, line by line — no unfavourable OCR wording.

---

## 5. Risks and open items

| # | Item | Status |
|---|---|---|
| 1 | ~~No Vulkan SDK here~~ | ✅ **installed** (2026-10-19, `KhronosGroup.VulkanSDK` 1.4.363.0): `glslc` is on PATH and the prerequisite check passes; whether the build itself succeeds is row 9 |
| 2 | Requesting an unavailable backend **degrades silently with exit 0** (`vulkan` without even a warning, while `bogus` does warn) | **settled (upstream source)**: `name_to_type()` treats `vulkan` as a valid name and maps it to GPU type (`backend.cpp:40`), and `dev_by_type(GPU)` returns the first match (`ggml-backend-reg.cpp:348-356`) ⇒ the silence is by design. The only usable criterion is the backend that actually took effect |
| 3 | **Dual-GPU placement** | ✅ **measured and closed** (2026-10-19, §3.3): unset = `Vulkan1` = RTX 4060 (uma:0); bare `vulkan` = `Vulkan0` = the Intel iGPU (uma:1); `Vulkan0`/`Vulkan1` pinning and `GGML_VK_VISIBLE_DEVICES=1` (only the RTX 4060 left) all match |
| 4 | The Vulkan artifact layout and DLL-copy paths in `build_moss.ps1` | ✅ **measured** (2026-10-19, §2.5): moss's CMake enables `GGML_BACKEND_DL` and **does produce a standalone `ggml-vulkan.dll`** (46 MB); DLLs land in `build-vulkan/bin/Release/`, correctly hit by the script's multi-level probe |
| 5 | The real performance gap between Vulkan and CUDA (third-party data is mixed: a measured pp512 −25%, and a "6× slower" case attributed to dual GPU + an old card; upstream moss issue #4 further shows the Vulkan path **has not even enabled flash attention**, worth 13.7× on an AMD iGPU) | ✅ **measured** (2026-10-19, this machine's RTX 4060 / q5_k / 5:14.99 audio, §4-3): **Vulkan is 1.77× slower than CUDA** (134.7 s vs 76.2 s) but 5.0× faster than CPU (680.6 s) — the win case = machines without NVIDIA; CUDA remains the pick for peak performance |
| 6 | `bootstrap_moss.ps1:175-181`'s backend assertion only works by accident (`-match` is case-insensitive) | ✅ **fixed** (2026-10-18, step 1): now parses `[mt I] backend: <device>` with `-cmatch` and prefix-matches `CPU`/`CUDA`/`Vulkan`; the degradation warning (`[mt W] …requested but no matching device`) is no longer mistaken for the result |
| 7 | `VULKAN_SDK` / PATH and the `SPIRV-Headers` **CONFIG package** | ✅ **settled** (2026-10-19): the installer set `VULKAN_SDK=C:\VulkanSDK\1.4.363.0` and put `Bin` on PATH; the CONFIG package is at `<SDK>\Lib\cmake\SPIRV-Headers\` (**not** `share\cmake`) — the prerequisite check was corrected accordingly (§2.3) |
| 8 | The provenance of the local `moss-transcribe.exe` | ✅ **clarified (user)**: the upstream repo has no GitHub Release at all, so this binary is a **self-built** artifact of this repo's `scripts/build_moss.ps1` (currently the `-Backend cuda` build, `runtime/bin/.backend=cuda`) — which is exactly why `bootstrap_moss.ps1 -BinaryUrl` defaults to empty |
| 9 | **Whether MOSS Vulkan can be built on Windows at all** is unconfirmed: upstream has no Windows CI, and issue #1 shows Windows + MinGW + GCC 16 failing on the **non-Vulkan** path already (its fix, PR #3, is still unmerged) | ✅ **measured** (2026-10-19, §2.5): the **MSVC + VS2022 BuildTools + cmake 4.3.3 route builds successfully**; issue #1 is the MinGW route, not applicable. The only obstacle was MAX_PATH (row 10), now fixed |
| 10 | **MAX_PATH**: the Vulkan build's TryCompile transient tail is 208 characters; a work root over ~50 characters hits `FileTracker FTK1011`; the build directory must not live inside the repo | ✅ **fixed** (2026-10-19): `build_moss.ps1`'s default work root is now `<system drive>\gsa-moss-build\<backend>-<PID>` (30 chars), plus `-WorkDir` (empty directories only) and a length warning (root + 208 > 250) (§2.5) |
| 11 | `bootstrap_moss.ps1` used to write `moss_device = "vulkan"` for the vulkan backend ⇒ on a dual-GPU machine it always lands on the iGPU (§3.3 measured), silently | ✅ **fixed** (2026-10-19): the vulkan backend now writes `""` (auto, measured to prefer the dGPU); the readiness self-check gained the placement check (enumeration table + `uma:1` warning); the welcome page's ASR card shows the configured device |

---

## 6. Boundary with the parallel session (avoid change conflicts)

- Item 12 of `docs/MISC_TODO.md` in the main worktree branch `feat/misc-fixes` (`d08219c`) — "parameterise CUDA GPU acceleration" — touches `src-tauri/src/ai_runtime/config.rs` and adds front-end UI, overlapping §3.5 of this design.
- Agreed split: **this branch touches only `scripts/` + pure probing logic + `docs/`**; fields and UI stay with the main worktree.
- This worktree **has no `runtime/`**; the main worktree's runtime is used read-only via `GSA_RUNTIME_DIR`, and **nothing in the main worktree is ever written**.

---

## 7. Outside this branch: FunASR character-level timestamps (a separate verification item)

> **Update 2026-10-30: FunASR is retired and no longer distributed.** This section keeps the mechanism findings for future reference/reproduction; it is no longer a pending task.

The comment at `scripts/funasr_worker.py:219` claims Fun-ASR-Nano's `sentence_info` carries **character-level timestamps** and that sentence post-processing depends on them; `:243` consumes them directly, and `:70-73` uses `len(ts) < 2` to decide "do not split this segment".

**The mechanism as established from source (funasr 1.4.16)**: Nano has exactly one timestamp path — **CTC forced alignment** (`fun_asr_nano/model.py:965-966,1057-1071` → `torchaudio.functional.forced_align`) — and **no code anywhere in the repo interpolates timestamps evenly across characters**. The gate is `checkpoint_utils.py:22-47 disable_incomplete_ctc()`: when the checkpoint lacks the `ctc_decoder.*` / `ctc.*` keys it nulls the CTC head, so `result` does not even carry a `timestamps` key; `auto/auto_model.py:1163-1187` (**the branch this worker takes**, because it configures no `punc_model` and therefore degrades to `vad_segment` mode) then logs `No timestamp found in ASR result. …`, sets `ts = []`, and degrades `start`/`end` to **VAD segment boundaries**. In that case `:70-73`'s splitting branch is dead code and character-level timing is only segment-level (up to 30 s).

**⚠ But the premise "the checkpoint lacks CTC weights" has been disproved by measurement**: issuing an HTTP Range request against `model.pt` of `FunAudioLLM/Fun-ASR-Nano-2512` on ModelScope (revision `db823224e5c2510001b4a3d3ed97f430bd52ac88`, commit message **"upload ctc weights"**, 2,127,426,538 bytes) for only the first 12 MB (`206`, `Content-Range: bytes 0-12582911/2127426538`) shows its `merged/data.pkl` **literally contains `ctc_decoder.linear1.weight` (84 hits for `ctc_decoder`) and `ctc.ctc_lo.weight`**. The official blog and issue #106 statement that "the released checkpoint lacks CTC weights" is **stale with respect to the current checkpoint**.

⇒ So the "ts must be empty" conclusion **cannot** simply be carried over. Two outcomes are now possible: (a) the CTC head works ⇒ ts are **real** forced-alignment results (the code is fine and nothing needs logging); (b) ts are still missing, but for a different reason (`model.py:810-817` states plainly that "CTC timestamps are not produced in batched mode", and this worker passes `batch_size_s=120`) ⇒ the defect stands. **Only a measurement can tell them apart.**

**The cost of an empirical check has been established:** the main worktree's FunASR runtime was cleaned up on 2026-09-30 by user decision (measured: `deps_funasr`, `models/funasr` and `worker/funasr_worker.py` **are all gone**). Reproducing it requires a reinstall:

- Via `scripts/bootstrap_funasr.ps1 -Device cpu`: **only CPU torch is needed** (on the order of 200 MB) plus the funasr/modelscope dependencies plus 4 models (the main model Fun-ASR-Nano-2512 is ~1.6 GB) ≈ **2.5–3 GB** (versus ~7 GB for the CUDA route). Timestamp semantics are device-independent, so **the CPU route suffices**.
- Prerequisite: the script expects `runtime/python/python.exe` (it reuses OCR's embedded interpreter, provisioned by `scripts/bootstrap_ocr.ps1`). An alternative: use `py -3.12` (which points at the main worktree's 3.12 interpreter) to create a venv **inside this worktree**.
- Decision method (sharper than "is it equidistant"): dump `res[0].get("timestamps")` and `res[0]["sentence_info"][i]["timestamp"]`, and watch stderr for `Disabling CTC timestamps…` / `No timestamp found…`. **Empty = the defect holds; non-empty and aligned with the audio content = genuine forced alignment** — while also checking whether `batch_size_s=120` pushes it into batched mode.
---

## 8. Prebuilt distribution (P1, 2026-10-22)

> Motivation (user decision): **do not require users to build on their own machine**. Ship prebuilt zips on GitHub Releases plus sha256 verification; the installer downloads and unpacks them per manifest. Branch `feat/runtime-distribution` (base `f8dc4ac`).

### 8.1 Measured size of the three flavors (this machine, 2026-10-22)

| flavor | zip | unpacked | main contents |
|---|---|---|---|
| cpu | 1.1 MB | 2.7 MB | `moss-transcribe.exe` + `ggml*.dll` (no backend DLL) |
| vulkan | 15.4 MB | 48.9 MB | same + `ggml-vulkan.dll` (46 MB of SPIR-V kernels) |
| cuda | 621.6 MB | 1,159.7 MB | + `ggml-cuda.dll` 549 MB, `cublasLt64_12.dll` 507 MB, `cublas64_12.dll` 100 MB |

⇒ **Default to vulkan** (15 MB, covers NVIDIA/AMD/Intel alike) and keep cuda as a second, on-demand entry; both manifest entries coexist, and the 621 MB cuda package is not downloaded by default.

### 8.2 CUDA target architectures: `75;86;89`

- Covers Turing(75) / Ampere(86) / Ada(89) and **drops sm_80** (A100 datacenter, irrelevant to players); the build log printed `-- Using CMAKE_CUDA_ARCHITECTURES=75;86;89`.
- `build_moss.ps1` gained `-CudaArch` (default `75;86;89`), `-OutputDir`, `-UpstreamRef` and `-Jobs`; artifacts land in `<OutputDir>/<flavor>/`, ready for the packaging script.

### 8.3 MSVC runtime must be shipped app-local

`dumpbin /dependents` shows `moss-transcribe.exe` and every `ggml*.dll` **statically import** `msvcp140.dll` / `vcruntime140.dll` / `vcruntime140_1.dll` (`ggml-base`/`ggml-cpu` also need `vcomp140.dll`, i.e. OpenMP). The Rust host does not need them (static CRT), but on a clean Windows box a missing one means `0xC0000135`, which **looks exactly like "missing CUDA DLL"** — the user only sees "failed to start".

⇒ Ship them app-local (next to the exe: no admin rights, no install order), and have the packaging script verify that each of the four carries an Authenticode signature whose subject contains `Microsoft`.

### 8.4 Tooling

- `scripts/package_moss_flavors.ps1`: builds the zips and emits `manifest.json` (matching the schema in `docs/RUNTIME_BOOTSTRAP_CONTRACT.md` §4, `dest` always `bin`) plus `packages.json` (build record: per-file hash, upstream pin). Exit codes 0/1/2/3 = ok/args/verification/packaging.
- `scripts/verify_moss_flavors.ps1`: purity (backend DLLs that must be absent are absent), dependencies, signatures.
> ⚠ **The zips are not byte-reproducible**: the bundled `SOURCE.txt` records the build time and the zip entries carry file mtimes, so packing the same artifacts twice yields different hashes (measured `098E28…` vs `272A23…`). Therefore **the hash recorded in `packages.json` is authoritative**: publish the sha256 of the exact file you upload; never verify by rebuilding and comparing hashes.

### 8.5 ⚠ PowerShell parameter type constraint (already hit; do not repeat)

A variable declared `param([string]$Flavors)` stays **permanently constrained to string**: a later `$Flavors = @()` is coerced back to a string, and `$Flavors += $x` becomes **string concatenation** — so `"cpu,vulkan"` silently turns into `"cpuvulkan"` or `"cpu vulkan"` while the only symptom is "unknown flavor", which is very hard to trace.

⇒ Put the split result in a **different variable name** (this script uses `$FlavorList`); never reuse the parameter variable. Also note that with `-File`, `-Flavors 'cpu,vulkan'` arrives as a **single string**, not an array.

### 8.6 Done: release published and manifest URLs filled

- ✅ **Published** (2026-10-24): release tag **`moss-runtime-v1`** (target `dev`, pointing at `408e082`), three assets, and the manifest's absolute URLs are filled in.
  Download base `https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/moss-runtime-v1/`:

  | Asset | Size | sha256 |
  |---|---|---|
  | `moss-cpu.zip` | 1,121,692 B | `545e2b1a18f35c6cd462c26b768d177458bf4d6918a619f24d4767492c4d791d` |
  | `moss-vulkan.zip` | 16,139,894 B | `e848847289982b8322262341aa133e40b8f152ee563b8af63e3412e4eb6a89b5` |
  | `moss-cuda.zip` | 651,834,262 B | `5faa48b8b85e7f5763b835ad8b18ba467daef8243f0b1c7172fb3a1c87ed894d` |

- **Upload verified independently by GitHub**: the `digest` field (`sha256:…`) returned by `gh release view --json assets` matches the table above entry by entry, and so does `size` ⇒ the transfer was lossless, no need to re-download and re-hash.
- **Anonymous download test**: `moss-vulkan.zip` fetched from the public URL — 16,139,894 B, hash matches, 6.6 s; the extracted 11 files are identical to the local artifact (`.backend`=`vulkan`).
- The manifest's three `url` values are now the absolute addresses above instead of the `-UrlBase` placeholder; `sha256`/`size`/`dest` are unchanged and re-hashing the three local zips matched every entry.

### 8.7 Release procedure notes (`gh`)

```powershell
# Create with the small assets first; upload the 621 MB cuda zip separately (10.5 min measured)
gh release create moss-runtime-v1 --target dev --title "..." -F notes.md moss-cpu.zip moss-vulkan.zip
gh release upload  moss-runtime-v1 moss-cuda.zip
```

> ⚠ **`--target` accepts a branch or tag name only**: passing a commit SHA (`--target 408e082`) is rejected by GitHub as
> `HTTP 422 Validation Failed: Release.target_commitish is invalid`. Use `--target dev` — a release only needs to point at
> a commit that contains the artifacts; exact SHA provenance comes from the bundle's `SOURCE.txt` and `packages.json`.
> Also: **do not** repack and re-upload after publishing — the zips are not reproducible (§8.4), so a re-pack stops matching the manifest hashes.

### 8.8 The three former "not done" items, closed (2026-10-26)

- **Older-card routing** → patched as `_gsa_staging/patches/0001-old-gpu-cuda-fallback.patch` (`cuda_runnable()` gate at compute capability 7.5, `decide()` hard error now points at Vulkan instead of asking for a rebuild, `recommend()` recommends Vulkan), based on installer-side HEAD `dcdd1a5`, awaiting `git am` there.
- **`manifest.json` in the repo + `bundle.resources`** → done on the installer side (`2a766c8`, item 12 phase C): `src-tauri/resources/runtime/manifest.json` plus `"resources": ["resources/runtime/manifest.json"]` in `tauri.conf.json`.
- **Contract §4's `.7z` sample URL** → corrected on the contract side: `docs/RUNTIME_BOOTSTRAP_CONTRACT.md` now has §4.3 "deliverables are zip (not 7z), and the three unpacking pitfalls".

Also new and now closed: **`.installed.json` reconciliation** (previously "not this round") → `_gsa_staging/patches/0002-installed-state-check.patch` (adds `ai_runtime/installed_state.rs` and the `probe_moss_install` command, seven states); the writer, `scripts/bootstrap_runtime.ps1`, and the Rust-side install infrastructure ship with `7d3f8d5` (`ai_runtime/runtime_install.rs`: `precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install` + the `runtime-install-progress` event). Apply instructions and boundaries: `_gsa_staging/patches/HANDOFF.md`.

### 8.9 Runtime installer (`bootstrap_runtime.ps1` + `runtime_install.rs`)

> Landed in `7d3f8d5`. The point is that installing a runtime no longer means asking the user to copy a command line — the frontend can spawn it, watch progress, and cancel.
> The contract is `docs/RUNTIME_BOOTSTRAP_CONTRACT.md` (this implementation follows it; the contract itself is unchanged).

- **Script side** (`scripts/bootstrap_runtime.ps1`): stdout is pure JSONL (`start` / `progress` / `item_skip` / `item_done` / `item_fail` / `error` / `done`, `done` always last), human diagnostics go to stderr; exit codes `0` success / `1` pre-check failure / `2` verification failure / `3` some items failed / `4` unknown error; `-Flavor auto` runs the L0 probe (`nvidia-smi` compute capability ≥ 7.5 → `vulkan-1.dll` → cpu); disk pre-check (download size ×3 + 100 MB) before any download; mirror failure falls back to a direct connection; unpacking only after SHA256 matches; on success it writes `runtime/.installed.json` and supports `item_skip`.
- **Rust side** (`src-tauri/src/ai_runtime/runtime_install.rs`): the `precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install` commands, progress forwarded line by line over `runtime-install-progress`; cancel kills the process tree via `taskkill /T /F`; only one install task may run at a time.
- **Why they are decoupled**: child-process I/O lives in `stream_child` plus the exit-code mapping `summarize`, neither of which touches Tauri, so the unit tests can drive a **real script in a real child process** (`end_to_end_bogus_item_reports_failure`, `cancel_stops_waiting`) instead of mocking things.
- **Leftover**: in packaged form the script is not shipped with the app yet (`bundle.resources` maps only the manifest). `bootstrap_candidates` already reserves the `resources/scripts/bootstrap_runtime.ps1` slot; one more resource mapping is all it takes. (Checked 2026-10-30: that mapping and `scripts/sync_bundle_resources.mjs` landed on `feat/misc-fixes` as `bd8ee43`; this branch does not have it yet.)

### 8.10 Model joins the distribution (`models-moss`, 2026-10-30)

> Why: `probe_moss_install` only looked at the binary, so after "install runtime" the card showed **ok** and the first transcription then failed with `MOSS 模型不存在`. The 618 MB gguf was in no manifest and had no automatic path.

- **Release**: `models-moss.zip` = 626,397,892 B, sha256 `809f8999ac51ef0836b9324b2c1ad6a40e520b42f69dfc60c260dd01ab9a8cb1`, published as a fourth asset of Release `moss-runtime-v1` (assets are appended to the same tag; the version number does not change).
- **Manifest**: a fourth entry `{id:"models-moss", dest:"models/moss"}` with **no `flavor` and no `backend_marker`** — all three backends share the same q5_k model, and device probing is irrelevant to it.
- **Default items**: `-Items` in `bootstrap_runtime.ps1` changed from `moss` to **`moss,models-moss`** (you need both).
- **Script fix**: the flavor candidate set for `-Flavor auto` is now the **union over the selected items**. The old code looked at `items[0]` only, so installing just `models-moss` was rejected with "no usable flavor in the manifest". Both paths were exercised on a live run (`models-moss` alone → `auto: 所选条目与 flavor 无关，跳过设备探测`, exit 0; `models-moss,moss` → `auto 选择: cpu`, both `item_done`, and a second run reports `item_skip` for both).
- **State machine**: `evaluate` gained `model_missing` (binary present, gguf absent ⇒ cannot run, ranked above the version-only verdicts), and the "no `.installed.json` record" branch now also runs the model check.
- **Empty config = standard layout**: the one-click install only unpacks and never writes `config.json`, so empty `moss_binary` / `moss_model` must fall back to `bin/moss-transcribe.exe` and `models/moss/moss-transcribe-q5_k.gguf` (new `moss_paths()`, used by `MossProvider::spawn`, `probe_moss_backend` and `probe_moss_install`). Otherwise `resolve_path("")` resolves to the runtime directory itself and a freshly installed runtime is reported as `file_missing`.
- **BOM tolerance**: `parse_installed` / `parse_manifest_items` strip a UTF-8 BOM before parsing — PowerShell 5.1's `Set-Content -Encoding UTF8` adds one, and serde_json treats it as a syntax error, which would silently turn the whole manifest into "empty" and lose every status.
### 8.11 OCR runtime joins the distribution (`deps-ocr` / `deps-ocr-gpu` + CUDA libs fetched by pip, 2026-10-30)

**Goal**: OCR is the engine "almost everyone will use", so it must work before the user starts
work on a project ⇒ the first launch installs everything in one go (`moss` + `models-moss` + the
two OCR packs + the CUDA runtime libraries). Missing binary, missing model, or missing OCR runtime
all mean the product does not run.

- **Shape (A)**: the OCR runtime ships as a single archive that **preserves directory structure**
  (`python/` `deps/` `models/paddleocr/` `worker/`) with `dest: "."` in the manifest. Bundling it
  into the installer was rejected (every user would pay +350 MB); splitting it per component was
  rejected too (state machine ×N, a wall of progress rows). Relocatability was verified on a real
  machine: after copying the tree elsewhere `sys.prefix` follows, and `import paddleocr/numpy/cv2`
  all succeed (3.4.0 / 2.5.3 / 4.10.0) ⇒ no pip and no venv on the user's machine.
- **Pinned lists**: `scripts/ocr-requirements.lock.txt` (61 packages) and
  `scripts/ocr-gpu-requirements.lock.txt` (9 packages) generated from
  `pip freeze --path runtime/deps(_gpu)`; `package_ocr_runtime.ps1` freezes again after install and
  compares line by line, refusing to package on any drift (the artifact must be byte-reproducible).
- **Artifacts** (Release `ocr-runtime-v1`):
  | asset | size | unpacked | flavor |
  |---|---|---|---|
  | `ocr-runtime-cpu.zip` | 309,975,635 | 911,575,386 | cpu |
  | `ocr-runtime-cuda-paddle.zip` | 502,533,301 | 996,251,695 | cuda (optional) |
  An earlier third asset, `ocr-runtime-cuda-libs.zip` (1,856,518,097 / 2,693,178,035 unpacked), held
  the `nvidia/*` runtime libraries. At 1.86 GB it sat right on GitHub's hard **2 GiB** per-asset limit
  (the 2,378,095,332 B archive was rejected with 422 `size must be less than 2147483648`), so it has
  been **withdrawn** — see the next bullet.
- **CUDA/cuDNN runtime libraries are fetched by pip at install time (revised 2026-10-30)**: the eight
  `nvidia/*` wheels are no longer packaged; the installer pulls them into the same `deps_gpu/` with
  the `pip` that ships inside the OCR baseline. Two reasons:
  1. **Licensing** -- CUDA Toolkit EULA 1.2.2 forbids distributing the SDK "as a stand-alone product",
     and 1.1.2 requires distributable parts be "accessed only by your application"; hosting a zip of
     bare DLLs behind a public direct link violates both.
  2. **Size** -- 1.86 GB sat right on the per-asset cap mentioned above.
  Implementation: `$GpuLibs` in `bootstrap_runtime.ps1` (8 x `name==version + sha256`) and
  `pip install --no-deps --require-hashes --target deps_gpu`; the default index is the **Huawei Cloud
  PyPI mirror** (measured 10.3 MB/s; Aliyun 0.79, upstream PyPI 0.09). The pip phase **clears
  `HTTP(S)_PROXY`** unless `-PipUseProxy` is passed -- a domestic mirror is slower through a proxy
  (0.13-0.7 MB/s measured). Real run: 2,568.4 MB / 258 files landed; a second run yields `item_skip`.
  Events look like any other item (`item = "deps-ocr-cuda-libs"`), so the frontend is unchanged.
- **flavor rules (now extended to OCR)**: the global `-Flavor` is a **preference**, not a command;
  each entry resolves it on its own: ① exact match wins; ② otherwise fall back to the entry's
  `cpu` variant (an AMD/Intel box picks `vulkan` under `auto`, and OCR has no vulkan build);
  ③ still no match and the entry is `optional` ⇒ skip silently, otherwise record a failure.
- **Three script fixes**: structure-preserving extraction (the old code flattened by file name and
  would have turned `python/` and `deps/` into a pile of same-named files), `unpacked_size` used by
  the disk precheck, and `optional` semantics.
- **Disk precheck formula (fixed)**: `Σdownload + Σunpacked + max single unpacked + 100 MB` (plus
  1.73 GB + 2.51 GB when the pip phase runs). The
  last term is the copy that exists in the temp staging dir and in `dest` at the same time; it used
  to be missing, so the check passed "needs 6.87 GB, 7.53 GB free" and then the 2.69 GB CUDA pack
  failed mid-extraction with `There is not enough space on the disk.` (observed: exit code 3,
  `item_fail`).
- **Truncation guard in the packager**: when the disk fills up `tar.exe` only prints `Write error`
  and still leaves a truncated zip that can be hashed, uploaded, and only blows up on extraction
  (observed once: 119/319 entries) ⇒ `New-Zip` verifies the entry count after packing **and**
  checks `tar -tf`'s exit code (a truncated zip does not necessarily have fewer entries; tar reads
  names from local headers first and errors afterwards).
- **Downloader rework (added the same day, 2026-10-30)**: a dropped connection no longer restarts
  from zero. `bootstrap_runtime.ps1` now does "multi-source candidates + byte-range segments +
  in-segment resume", **without repacking any asset** (the largest pack is 502 MB; repacking and
  re-uploading costs more than it saves). Candidate order: `urls[]` → `-Mirror` prefix → `url`
  direct. Each candidate is probed with `curl -r 0-0` for `206`; sources that ignore Range
  (gh-proxy does this for large files) are used single-stream and **never resumed** (`curl -C -`
  appends duplicates when Range is ignored — measured: a 1770 MB download grew to 2036 MB). Segments
  live in `<runtime>\.downloads\<id>-<sha8>\`, and a leftover `.curlpart` is merged into the segment
  file on the next run ⇒ resume across processes. Measured: a 20 MB asset in 5 segments, and after a
  kill it logged `续传：缓存里已有 3.0 MB，只需补 17.0 MB` and still matched the manifest SHA256; the
  real 295.6 MB `deps-ocr` came down through gh-proxy in 5 segments.
- **Still not done (on record)**: the OCR install gate UI (block project entry until OCR is installed)
  belongs to the frontend wiring and ships together with 11c; the external acceleration options
  (ModelScope as primary source / self-hosted Cloudflare Worker reverse proxy) are researched but not
  deployed — see the survey summary at the end of §8.11.
