# MOSS ASR 的 Vulkan 优化与 GPU 后端回退设计

> 状态：**步骤 1–5 全部完成**（脚本前置检查 / Vulkan 构建实测 / 三设备基准 / 探测与 fallback 语义 / 文档与文案核验）（2026-10-18 用户确认要点见 §0.2）
> 本分支范围：**仅 MOSS ASR 的 Vulkan 通路 + 后端探测/回退语义**
> 分支：`feat/engines-gpu-vulkan`（本 worktree `AIGameSubtitleAssistant-gpu-vulkan`），基线 `7b8e306`
> 明确**不做**：LLM 的 Vulkan（已有官方通路，用户决定不动）、OCR / 融合嵌入的 GPU（上游无 Vulkan 通路，见 §1）、FunASR（不在本分支范围，见 §7）

---

## 0. 摘要与已确认决策

### 0.1 摘要

1. **只有 MOSS 值得做，而且它确实能做**：`MTD_DEVICE` 是真实生效的指令，`MTD_DEVICE=cpu` 实测能切到 CPU；Vulkan 侧 ggml 支持 `-DMT_GGML_VULKAN=ON`。
2. **两个真实缺口**：① 本机缺 `glslc`（Vulkan SDK），`scripts/build_moss.ps1 -Backend vulkan` 现在必在 cmake configure 阶段失败；② 项目没有任何"探测已装后端 / 解释为什么没走 GPU"的逻辑。
3. **危险行为（实测）**：请求一个"合法但不可用"的后端时，`moss-transcribe` **静默降级、退出码 0、无告警**。所以判据只能是"实际生效后端"的文本证据，不能是退出码。
4. **回退语义 = 方案 A**：资产层同时只能存在一套 ggml 后端（`ggml.dll` 同名互斥），所以"回退"是**探测已装后端 + 告诉用户该装哪个**，不是运行期自动切换。

### 0.2 已确认的决策（2026-10-18）

| # | 决策 | 说明 |
|---|---|---|
| ① | **采用方案 A**，理想形态是**首次启动自检时探测用户机器并自动选后端**；但"首次启动自检"由并行会话实现，本分支先交付**探测逻辑本身**（可被自检复用） | 见 §3.1 |
| ② | **显式指定后端失败 → 直接报错**，让用户取消勾选 GPU 加速；只有 `auto` 才允许自动下沉 | 见 §3.4。GPU 加速参数化（config 字段 + 前端面板）由并行会话实现 |
| ③ | **不做 LLM 探针**；MOSS 的 `info` 探针零成本，作为主判据 | 见 §3.2 |
| ④ | **双 GPU（核显 + 独显）是构建后必须专门验证的风险项**，记录备忘 | 见 §3.3 |
| ⑤ | 对外文案：**Vulkan 优化仅说明作用于 ASR**；OCR 在 CPU 下速度可接受，不做对产品不利的技术披露 | 见 §4.4 |

---

## 1. 为什么范围只能是 MOSS（背景矩阵）

| 组件 | 载体 | 现有 GPU 通路 | Vulkan | 本分支 |
|---|---|---|---|---|
| **ASR MOSS** | moss-transcribe.cpp（ggml）自构建 | CUDA（`.backend=cuda`，实跑 `backend: CUDA0`） | ✅ ggml 支持，**Vulkan 版已构建成功**（§2.5） | **✅ 做** |
| LLM | llama.cpp 预编译 zip | CUDA（`ggml-cuda.dll` 512.9 MB） | ✅ 官方预编译包（`llama-b10333-bin-win-vulkan-x64.zip` 已核实存在） | ❌ 不做（③） |
| ASR FunASR（备用） | PyTorch + Fun-ASR-Nano | CUDA cu124 | ❌ torch 无 Vulkan 后端 | ❌ 不做（§7） |
| OCR | PaddleOCR + PaddleX | CUDA（`deps_gpu` 3.5 GB） | ❌ Paddle Inference 无 Vulkan 后端 | ❌ 不做（⑤） |
| 融合嵌入 | onnxruntime | 仅 CPU | ❌ ORT 官方 EP 列表无 Vulkan | ❌ 不做 |

> LLM 的 Vulkan 分支**无需改动即可用**：`scripts/bootstrap_llm.ps1:23` 的 `-Version` 默认 `b10333`，`:53` 拼出的 `llama-b10333-bin-win-vulkan-x64.zip` 经 GitHub releases API 核实存在（34,147,775 字节，sha256 `117bca1f594cffd96a77703aa23d98450800bc22022a307d1e3567187fbb50fe`），`:204-207` 的自检匹配 `Vulkan0`。

---

## 2. 已实测的事实（本机，2026-10-18）

### 2.1 环境

| 项 | 值 |
|---|---|
| GPU | NVIDIA RTX 4060 Laptop（驱动 572.83，8188 MiB，CC 8.9）+ Intel RaptorLake-S 核显（驱动 101.5542） |
| 虚拟显示适配器 | OrayIddDriver、Todesk Virtual Display（2 个） |
| Vulkan 运行时 | `C:\Windows\System32\vulkan-1.dll` 存在；`vulkaninfo --summary` 退出码 0；loader 1.4.303 |
| Vulkan 枚举结果 | **4 条设备记录**：`DISCRETE_GPU`（NVIDIA，apiVersion 1.4.303）×1 + `INTEGRATED_GPU`（Intel，1.3.280）**×3** |
| `$env:VULKAN_SDK` / `glslc` | **均为空/不在 PATH**（无 Vulkan SDK） |
| 可用工具 | `vulkaninfo`（system32）、`cmake`、`ninja`、`nvcc`（`D:\CUDA-Toolkit\bin`）、`winget` |
| Python | 系统 `D:\Python\python.exe`（3.13）；`py -3.12` 指向主 worktree 的 `runtime\python\python.exe` |
| PowerShell | **只有 Windows PowerShell 5.1**（`5.1.26100.9711`，无 pwsh 7）⇒ 本仓 `scripts/*.ps1` 必须是 **UTF-8 带 BOM**：5.1 对无 BOM 文件按 GBK 解码，中文注释会崩成语法错误（本轮实测踩过：编辑工具改写后剥掉 BOM，脚本立刻 exit 1） |

### 2.2 MOSS 的后端选择机制（决定性实测）

只读探针（`GSA_RUNTIME_DIR` → 主 worktree `runtime/`，未写任何文件）：

| 探针 | 实际输出 | 退出码 |
|---|---|---|
| 不设 `MTD_DEVICE` | `[mt I] backend: CUDA0` | 0 |
| `MTD_DEVICE=cpu` | `[mt I] CPU threads: 24` + `[mt I] backend: CPU` | 0 |
| `MTD_DEVICE=vulkan`（CUDA-only 构建上） | **无任何告警**，`[mt I] backend: CUDA0` | **0** |
| `MTD_DEVICE=bogus` | `[mt W] backend: MTD_DEVICE=bogus requested but no matching device registered — falling back` + `backend: CUDA0` | 0 |

二进制字符串确认存在的环境变量只有 **`MTD_DEVICE`** 与 **`MTD_THREADS`**（正是 `src-tauri/src/ai_runtime/moss.rs:372,376` 传的两个）。

**三条结论**：

1. **`MTD_DEVICE` 是真指令**——`cpu` 确实生效，`cuda`/`vulkan` 是同一个取值空间。所以设计的着力点是"探测 + 选对值 + 让它可见"，而不是新增机制。
2. **失败静默、退出码不可用**——请求 `vulkan` 却跑在 CUDA 上时没有任何提示。⇒ 必须解析实际生效的后端。
3. **日志走 stderr**（`moss.rs:3` 注释即"stdout 输出 JSON 段数组，stderr 是日志"）：`info` 的 stdout 实测是**两行**：`arch=…` 与 `audio_token_id=… mel_bins=80 n_fft=400 hop=160`；`backend: <X>` 在 **stderr**。⚠ 反面教训：我早期只截了含 `arch=` 的那一行，就当成"只有一行"，并据此怀疑手上的二进制不是 master——**过滤后的单行会骗人**，`info` 的完整 stdout 才是判据。

**静默的机制（上游源码确证）**：`src/backend.cpp` 的 `name_to_type()`（:34-45）把 `vulkan` 判为**合法名**并映射到 `GGML_BACKEND_DEVICE_TYPE_GPU`；随后 `ggml_backend_dev_by_type(GPU)`（`ggml-backend-reg.cpp:348-356`，**返回第一个匹配**）在 CUDA-only 构建里直接返回 `CUDA0`。`g_backend` 非空 ⇒ 跳过唯一告警点（`backend.cpp:90-92`）。`bogus` 之所以有告警，只因 `name_to_type` 返回 false。⇒ **「vulkan」的真实语义是"任选一块 GPU 类型设备"，不是"必须是 Vulkan"**：静默是设计的必然行为，不是环境问题。

### 2.3 构建前置：`glslc` 与获取途径

- ggml 的 Vulkan 后端在 **configure 阶段**硬性要求 `find_package(Vulkan COMPONENTS glslc REQUIRED)` 与 `find_package(SPIRV-Headers CONFIG REQUIRED)`，着色器生成器要调 `--glslc`。**只在构建期需要 SDK，运行时只需要显卡驱动。**
- ~~本机无 SDK~~ → **2026-10-19 已装 `KhronosGroup.VulkanSDK` 1.4.363.0**：安装器已设好 `VULKAN_SDK=C:\VulkanSDK\1.4.363.0` 并把 `Bin` 加入 PATH（`glslc` 可直接 `Get-Command` 命中）。`winget` 里另有 `KhronosGroup.VulkanRT`（仅运行时组件，不含 glslc）。
- **`SPIRV-Headers` 的 CMake CONFIG 包确实随 SDK 提供**，但位置是 `<SDK>\Lib\cmake\SPIRV-Headers\SPIRV-HeadersConfig.cmake`，**不是** `share\cmake`（SDK 根下另有 `cmake\`、`share\`、`Source\`、`Include\spirv-headers\`）。CMake 把 `CMAKE_PREFIX_PATH=<SDK>` 展开为 `<prefix>/[share|lib|Lib]/cmake/<name>/` 搜索，故 ggml 的 `find_package(SPIRV-Headers CONFIG REQUIRED)` 能命中。**本分支的前置检查最初只探 `share\cmake`，对装好的 SDK 会误报；已改为在 `Lib\cmake` / `lib\cmake` / `share\cmake` 三处递归找 `SPIRV-HeadersConfig.cmake`，实测输出 `找到 SPIRV-Headers CONFIG 包: …\Lib\cmake\SPIRV-Headers\SPIRV-HeadersConfig.cmake`。**
- `scripts/build_moss.ps1:9` 的头注释写了"Vulkan 需要 glslc + Vulkan SDK"，但**代码里没有任何检查**（只有 `-Backend cuda` 会找 `nvcc`）⇒ 缺 SDK 时用户在 cmake 的晦涩报错里迷路。**已在步骤 1 补齐（见 §4-1）。**

### 2.4 零成本探测原语（已核实可用）

| 原语 | 用途 | 实测 |
|---|---|---|
| `nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader` | CUDA 可用性 | 退出码 0，输出 `NVIDIA GeForce RTX 4060 Laptop GPU, 572.83, 8188 MiB` |
| `vulkaninfo --summary` | Vulkan 设备数量与类型 | 退出码 0，`deviceType = PHYSICAL_DEVICE_TYPE_{INTEGRATED,DISCRETE}_GPU` 可正则解析 |
| `moss-transcribe info <model>` 的 stderr | **实际生效后端** | `[mt I] backend: CUDA0` |
| `Get-CimInstance Win32_VideoController` | 不依赖 Vulkan 的 GPU 清单（兜底/交叉验证） | 列出 4 个适配器（含 2 个虚拟显示） |

### 2.5 Vulkan 构建实测（2026-10-19）：成功，产物布局与 CUDA 版同形

- **环境**：Vulkan SDK 1.4.363.0 + VS2022 BuildTools（MSVC）+ cmake 4.3.3（默认 VS 生成器）。
- **产物**（`runtime/bin/`）：`moss-transcribe.exe`（274,432 B）+ `ggml.dll`（67 KB）+ `ggml-base.dll`（639 KB）+ `ggml-cpu.dll`（887 KB）+ **`ggml-vulkan.dll`（46 MB）**；`.backend=vulkan`。⇒ moss 的 CMake **打开了 `GGML_BACKEND_DL`**——§5-4 原先"可能不产出 ggml-vulkan.dll"的担忧不成立。DLL 落在 `build-vulkan/bin/Release/`（CMake `RUNTIME_OUTPUT_DIRECTORY`），脚本的 `…\Release\ → …\bin\Release → …\bin → exe 同目录`多级探测正确命中。
- **上游"Windows 编译失败"（issue #1：MinGW + GCC 16，非 Vulkan 路径）不适用于 MSVC 路线**（§5-9）。
- **唯一障碍是 MAX_PATH，不是上游缺陷**：
  - 构建树最深瞬态路径是 `…\build-vulkan\vulkan-shaders-gen-prefix\src\vulkan-shaders-gen-build\CMakeFiles\CMakeScratch\TryCompile-…\cmTC_….dir\Debug\cmTC_….tlog\link-cvtres.read.1.tlog`，与工作根无关的尾巴实测 **208 字符**；`MAX_PATH=260` ⇒ **工作根 ≤ ~50 字符**，否则 `FileTracker error FTK1011`（默认 `%TEMP%\gsa_moss_build_<PID>` 实测 53 字符恰好超线）。
  - ⇒ **构建目录不能挪进仓库**：仓库路径本身已 58 字符，58+208=266 ✗。
  - CUDA 版不受影响：走 Ninja，没有 vulkan-shaders-gen 这层 ExternalProject，尾巴短约 70 字符——这解释了 CUDA 版一直没踩到。
  - 修法已进 `build_moss.ps1`：默认工作根改 `<系统盘>\gsa-moss-build\<backend>-<PID>`（30 字符），新增 `-WorkDir` 覆盖参数（只收空目录，防误删），根长+208 > 250 时打印超长告警。

---

## 3. 设计

### 3.1 资产层语义：方案 A（探测 + 指路）

**事实约束**：llama.cpp 与 moss 的 `ggml.dll` 同名且互不兼容（DLL 从 exe 同目录优先加载），因此 `runtime/bin/` 同时只能存在一套 ggml 后端。实测现状就是 `.backend=cuda`，全仓无 `ggml-vulkan.dll`。

⇒ **"CUDA 失败自动切 Vulkan"在资产层不成立**（除非把 `bin/` 改成 `bin/<backend>/` 并存，代价：磁盘 ×N、启动探测变慢、构建矩阵翻倍——**本分支不做**）。本分支交付的语义是：

> **探测用户机器能跑什么 → 校验已装二进制是否与之一致 → 不一致时给出"该装哪个后端"的明确指路**。

对最终用户表现为（由并行会话的首次启动自检承载，①）：能自动选则自动选；不能自动选则用人能看懂的话告诉他跑哪条命令。

### 3.2 三级探测与判据

| 级别 | 内容 | 成本 | 用途 |
|---|---|---|---|
| **L0 机器能力** | CUDA：`nvidia-smi` 退出码 0；Vulkan：`vulkan-1.dll` 存在 **且** `vulkaninfo --summary` 解析到 ≥1 个 `*_GPU` | 毫秒级 | 与已装二进制比对，产出"该装哪个后端" |
| **L1 资产一致性** | `moss-transcribe.exe` 是否存在 + `.backend` 标记是否与期望一致；DLL 名可作辅助判据（实测 `GGML_BACKEND_DL=ON`：vulkan 版有 `ggml-vulkan.dll`、cuda 版有 `ggml-cuda.dll`，§2.5） | 文件系统 | 快速发现错配 |
| **L2 实际生效校验** | 跑 `moss-transcribe info <model>`，解析 **stderr** 的 `[mt I] backend: <X>`，与期望值精确比对（`CUDA0`/`Vulkan0`/`CPU`）；**多设备后端还要连落点一起核**：`Vulkan<N>` 只是枚举序号，须对照 stderr 的 `ggml_vulkan: <N> = <设备名> \| uma: <0/1>` 表（`uma:0` 通常 = 独显）——`bootstrap_moss.ps1` 自检已内置落点复核 | 零成本（③，实测 `info` 很快） | **唯一可信判据**，兜住 §2.2 的静默降级 |

- ⚠ **退出码一律不可作判据**（§2.2 实测 exit 0 却降级）。
- ⚠ **必须精确匹配全名**（`CUDA0` 而非 `CUDA`）：Rust 侧 `str::contains` 大小写敏感，而 `scripts/bootstrap_moss.ps1:175` 现在靠 PowerShell `-match` 的大小写不敏感侥幸成立——顺手统一。
- L0 的 Vulkan 判据要考虑 **`vulkaninfo` 会把同一块 Intel 核显报 3 次**（实测 4 条记录对应 2 块物理 GPU）：**按 `deviceType==DISCRETE_GPU` 判断优先，按 `deviceName+driverInfo` 去重后再计数**，别用"记录条数"。

### 3.3 双 GPU（核显 + 独显）——机制已定，构建后按此验证（④）

**上游机制（源码确证）**：ggml 的 Vulkan 后端在 `ggml-vulkan.cpp:6814-6859` 里按 `vkEnumeratePhysicalDevices()` 的**返回顺序递增 push**，**不做"独显优先"重排**；设备名是 `Vulkan0/Vulkan1/…`（`:17779`），设备类型由 `ctx->is_integrated_gpu` 决定（`:16996`，核显 = `IGPU`、独显 = `GPU`）。MOSS 固定的 ggml 是 `ggml-org/ggml` 的 `eced84c8`（**v0.15.3，2026-06-26**），**已包含** llama.cpp PR #15947（2025-09-12）"默认同时暴露 iGPU + dGPU"的行为。

**本机枚举序**：`vulkaninfo --summary` 中 Intel 核显在前、RTX 4060 在后。据此推出四种设置的真实落点：

| `MTD_DEVICE` | 命中路径 | 本机落点 |
|---|---|---|
| **不设（auto）** | `ggml_backend_init_best()`，优先级 **GPU → IGPU → CPU** | **RTX 4060**（核显是 `IGPU` 类型，不会被 `GPU` 命中）✅ |
| `vulkan` | `init_named()` 子串命中设备名 `Vulkan0` | **Intel 核显** ❌ |
| `Vulkan0` | 子串命中 `Vulkan0` | Intel 核显 ❌ |
| `Vulkan1` | 子串命中 `Vulkan1` | RTX 4060 ✅（但**索引随机器变化，不可硬编码**） |

**2026-10-19 Vulkan 构建实测（§2.5 产物，`info` stderr）**：上表已验证——**不设 = `Vulkan1` = RTX 4060**（枚举表 `uma: 0`）；**裸 `vulkan` = `Vulkan0` = Intel 核显**（`uma: 1`）；`Vulkan0`/`Vulkan1` 定点命中各自卡；`GGML_VK_VISIBLE_DEVICES=1` 实测枚举只剩 `0 = RTX 4060`，此时裸 `vulkan` 也落独显。ggml 自己的 stderr 枚举（即 `ggml_vulkan: Found 2 Vulkan devices:` 表）与 `vulkaninfo` 一致：核显枚举 0、独显枚举 1，核显 `uma:1 / bf16:0`、独显 `uma:0 / warp size:32`。

**结论（纠正了本设计早先的猜想）**：

1. **不要设裸 `MTD_DEVICE=vulkan`**——在 Vulkan 构建上它会命中枚举序第一块（本机即核显）。它在 CUDA-only 构建上"看起来正常"，只是因为落到了 `dev_by_type(GPU)` = `CUDA0`（§2.2）。
2. **`auto`（空）才是最优解**：`ggml_backend_init_best()` 的类型优先级天然避开核显。
3. 需要**硬指定某块卡**时，用 `GGML_VK_VISIBLE_DEVICES=<原始枚举下标>` 把设备裁到只剩目标卡（此后它就是 `Vulkan0`），再配合 `MTD_DEVICE=vulkan`——比 `MTD_DEVICE=Vulkan<N>` 稳，因为后者依赖枚举序。注意它是 `vkEnumeratePhysicalDevices` 的**原始下标**，不是 `vulkaninfo` 的显示名。
4. **MOSS 只创建一个 backend**（`src/backend.cpp` 的 `g_backend` 单例），不做多设备层切分 ⇒ "模型被劈到两块卡"在 MOSS 上不成立；上游那个 3× 塌陷（llama.cpp #30695）也不会命中，且引入它的 #29741（2026-10-13）晚于本 pin。
5. MOSS 的 CLI **没有 `--device`**：`MTD_DEVICE` 与 `GGML_VK_VISIBLE_DEVICES` 是仅有的两个通道。

**落点验证已完成（④，2026-10-19）**：上表四行全部实测命中（见上）。`bootstrap_moss.ps1` 就绪自检已内置落点复核（枚举表回显 + `uma:1` 警告，config 写值改为 auto，§5-11）；欢迎页 ASR 自检卡显示当前配置的 device。转写耗时对比也已实测（步骤 3：CPU 680.6 s / Vulkan 134.7 s / CUDA 76.2 s，见 §4-3 与 `docs/development.md`）。

### 3.4 显式指定失败 = 报错（②）

| 用户意图 | 探测结果 | 行为 |
|---|---|---|
| `auto`（或空） | 有 CUDA 二进制 | 用 CUDA，记 `reason` |
| `auto`（或空） | 无 CUDA 二进制但有 Vulkan 二进制 | 用 Vulkan，记 `reason` |
| `auto`（或空） | 都没有 | 用 CPU，记 `reason`，**在自检面板可见** |
| 显式 `cuda` / `vulkan` | 该后端不可用（DLL 缺失或 L2 校验失败） | **直接报错**，提示"取消勾选 GPU 加速改用 CPU"或"装对应后端"，**不自动降级** |

理由：MOSS 在 CPU 与 GPU 上的差距是量级级别（canonical 记录 306 s → 50.4 s，约 6×），静默降级会让用户以为在用 GPU。字段与前端开关由并行会话实现（②），本分支只提供判定函数与其返回值。

### 3.5 与并行会话的接口（边界）

- **本分支交付**：`scripts/`（构建前置检查、探测）、Rust 侧的**纯探测/判定逻辑**、`docs/`。
- **并行会话交付**：`src-tauri/src/ai_runtime/config.rs` 的字段扩展（`auto` 语义、`llm_device` 等）与前端配置面板/自检面板。
- **需要对齐的契约**（避免两边各写一套）：取值集合 `"" | "auto" | "cuda" | "vulkan" | "cpu"`；判定结果的结构（`{backend, reason, source}`）。

---

## 4. 实施步骤（本分支，逐轮确认）

1. ✅ **已完成**（2026-10-18）：`scripts/build_moss.ps1` 新增 `if ($Backend -eq "vulkan")` 分支 —— `glslc` 按 PATH → `VULKAN_SDK` → `C:\VulkanSDK\<版本>` 三级查找，命中即自动设 `VULKAN_SDK`/PATH 供 cmake 定位；缺失则并入 `$missing`，**在 clone/构建之前** `throw`（附 `winget install --id KhronosGroup.VulkanSDK -e`），并预先警告缺失的 `SPIRV-Headers` CONFIG 包。顺带修正 `scripts/bootstrap_moss.ps1` 就绪自检段的后端断言：改为解析 `[mt I] backend: <设备名>`（原写法靠 `-match` 大小写不敏感侥幸成立，且会把 `[mt W] …requested but no matching device` 降级警告当成结果）。
   → 验证（已跑通）：`-Backend vulkan` 无 SDK 时于 clone 前报错、零副作用（不产生 `runtime/`）；伪造 SDK 时正确发现 `glslc` 并打出 SPIRV-Headers 警告；真实二进制的 `info` 输出在 cpu/cuda/vulkan 三种请求下分别解析为 `CUDA0`（cuda 命中，cpu/vulkan 正确判为降级）；`MTD_DEVICE=bogus` 时旧写法把警告文本当结果、新写法仍取到 `CUDA0`。**"装了 SDK 后 configure 通过"已由步骤 2 确认（✅）。**
2. ✅ **已完成**（2026-10-19）：`-Backend vulkan` **构建成功**，产物与 DLL 布局见 §2.5（`ggml-vulkan.dll` 46 MB 确实产出 = `GGML_BACKEND_DL=ON`；`build-vulkan/bin/Release/` 被脚本多级探测正确命中）。过程中暴露并修复三个问题：
   - **MAX_PATH**：默认 `%TEMP%` 工作根（53 字符）+ 208 字符 TryCompile 尾巴超 MAX_PATH（FTK1011）⇒ `build_moss.ps1` 默认工作根改 `<系统盘>\gsa-moss-build\<backend>-<PID>`（30 字符）+ `-WorkDir` 覆盖 + 超长告警（§2.5）；
   - **`bootstrap_moss.ps1` 的 `moss_device` 写值缺陷**：vulkan 后端原来写 `"vulkan"` ⇒ 双 GPU 机必落核显（§3.3 实测）⇒ 改为写 `""`（auto，实测偏好独显）；cpu/cuda 写值不变；
   - **落点复核进自检 + 欢迎页展示**：bootstrap 自检新增 Vulkan 枚举表回显与 `uma:1`（多为核显）落点警告；欢迎页 ASR 自检卡新增"设备 · <device>"行（`AsrEngineStatus.device` 取 config 值，`""` 显示为 "auto（ggml 自动选优）"——展示配置值而非实测后端，实测需拉起子进程）。
   → 验证（已跑通）：clone 必败法实测默认根 30 字符无告警、超长根（89 字符）正确告警（估算 297 > 250）、`-WorkDir` 非空守卫生效；落点复核块对真实 `info` 输出解析出 `Vulkan1` = RTX 4060（uma:0）；`cargo check` / `vue-tsc --noEmit` 通过；脚本 BOM/CRLF 复核无恙（§2.1 教训）。
3. ✅ **已完成**（2026-10-19）：落点确认（§3.3 实测）后用 `quality_bench_test(voiced)_5min.mp4`（提取 5:14.99 / 16kHz mono wav，与 `video::extract_audio` 同参；`moss-transcribe-q5_k.gguf`；`--format json`；未设 `MTD_THREADS`）跑了三设备各一次：**CPU 680.6 s（`backend: CPU`）、Vulkan 134.7 s（`Vulkan1` = RTX 4060）、CUDA 76.2 s（`CUDA0`）**——Vulkan ≈ CUDA 的 1.77×，仍比 CPU 快 5.0×（CUDA 快 8.9×）。转写文本三方一致（总字数 3765/3765/3775，首末段逐字相同；段数 84 vs 90 属分界粒度差异）。耗时对比已记入 `docs/development.md` 的「MOSS 后端与三设备实测」。
   → 验证：三跑 `exit=0` 且 stderr 的 backend 行分别为 `CPU`/`Vulkan1`/`CUDA0`；输出 JSON 同为段数组（`id/start/end/speaker/text`）。
4. ✅ **已完成**（2026-10-19）：探测逻辑 + 回退语义落地——新模块 `src-tauri/src/ai_runtime/backend_probe.rs`（纯函数解析/判定 + L0/L1 探测，14 个单测；不碰 config.rs）：
   - **L2 实证**：`parse_info_stderr()` 只认 `[mt I] backend:` 行（`[mt W]` 降级警告不误判）；`parse_vulkan_devices()` 解析 `ggml_vulkan: N = … | uma: …` 枚举表；
   - **L0/L1**：`probe_machine()`（nvidia-smi -L；System32\vulkan-1.dll + vulkaninfo --summary 按 deviceName+driverInfo 去重——本机核显被枚举 3 次）、`probe_assets()`（`.backend` 标记 + ggml-{cuda,vulkan}.dll）；
   - **判定**（§3.4 矩阵）：`decide()` —— 显式 cpu 恒可用；显式 cuda/vulkan 与 L2 实测**跨族不符 = 硬报错**（含"build_moss.ps1 -Backend … 重装"或"改回 auto"指引；L2 缺失时退回资产推断）；auto 以 L2 为准，Vulkan 落 `uma:1` 设备时在 reason 附核显提示；`recommend_from_assets()` 给自检"该装哪个后端"提示；
   - **接线**：`moss.rs` probe() 与 run_transcribe 同参（显式设备才设 MTD_DEVICE）→ L2 实测与转写真实后端一致；判定存 `backend_decision`，经 trait 新默认方法 `effective_backend()`（仅 moss 覆写）导出；`AsrEngineStatus` 新增 `effective_backend` 字段（欢迎页 ASR 卡显示"设备 · <config>（实测 <L2>）"）。
   → 验证：`cargo test --lib` 273 通过（含 15 个新单测：降级警告不误判、跨族硬报错、同族核显只警告、核显三连枚举去重等）；`vue-tsc --noEmit` 通过。§3.4 全行为矩阵以单测固化（夹具 = §2.2/§2.4 实测 stderr 原文）。
5. ✅ **已完成**（2026-10-19）：文档与文案——
   - `docs/development.md` 新增「MOSS 后端语义与回退（`backend_probe.rs`）」小节（`moss_device` 取值语义、L0/L1/L2 判定链、方案 A 回退语义=显式跨族硬报错/auto 允许下沉、`effective_backend` → 欢迎页实测展示、Vulkan 构建入口与 MAX_PATH 提示）；`config.json 关键字段` 表的 `moss_device` 行同步；`docs/development_EN.md` 镜像；
   - **⑤ 口径核验（README.md:59 / README_EN.md:60 / 欢迎页卡片文案）**：现行文案已合规——GPU 表述只落在 ASR/LLM 事实范围（"转写时间延长数倍"指 ASR；OCR 卡片仅中性描述"本地 Python worker"），无对 OCR 的不利披露，**零改动**。
   → 验证：全仓 grep `GPU|vulkan|CUDA` 于两版 README 与 Welcome.vue 逐行核对，无 OCR 不利表述。

---

## 5. 风险与待验证

| # | 项 | 状态 |
|---|---|---|
| 1 | ~~本机无 Vulkan SDK~~ | ✅ **已装**（2026-10-19，`KhronosGroup.VulkanSDK` 1.4.363.0）：`glslc` 在 PATH，前置检查通过；构建本身是否成功见行 9 |
| 2 | 请求不可用后端 **静默降级 + exit 0**（`vulkan` 连告警都没有，`bogus` 反而有） | **已定论（上游源码）**：`name_to_type()` 把 `vulkan` 判为合法名并映射到 GPU 类型（`backend.cpp:40`），`dev_by_type(GPU)` 返回第一个匹配（`ggml-backend-reg.cpp:348-356`）⇒ 静默是设计使然。判据只能取"实际生效的后端" |
| 3 | **双 GPU 落点** | ✅ **已实测并结案**（2026-10-19，§3.3）：不设 = `Vulkan1` = RTX 4060（uma:0）；裸 `vulkan` = `Vulkan0` = Intel 核显（uma:1）；`Vulkan0`/`Vulkan1` 定点与 `GGML_VK_VISIBLE_DEVICES=1`（只留 RTX 4060）均命中 |
| 4 | `build_moss.ps1` 的 vulkan 产物布局与 DLL 拷贝路径 | ✅ **已实测**（2026-10-19，§2.5）：moss 的 CMake 打开了 `GGML_BACKEND_DL`，**产出独立 `ggml-vulkan.dll`**（46 MB）；DLL 在 `build-vulkan/bin/Release/`，脚本多级探测命中 |
| 5 | Vulkan 相对 CUDA 的真实性能差（第三方数据混杂：有 pp512 −25% 的实测，也有被归因于双 GPU + 老卡的"慢 6 倍"个案；上游 moss issue #4 还显示 Vulkan 路径**连 flash attention 都没启用**，AMD iGPU 上启用后快 13.7×） | ✅ **已实测**（2026-10-19，本机 RTX 4060 / q5_k / 5:14.99 音频，§4-3）：**Vulkan 比 CUDA 慢 1.77×**（134.7 s vs 76.2 s），但比 CPU 快 5.0×（680.6 s）——收益成立的场景 = 无 NVIDIA 的机器；追求极致仍应 CUDA |
| 6 | `bootstrap_moss.ps1:175-181` 的后端断言用 `-match` 大小写不敏感侥幸成立 | ✅ **已修**（2026-10-18，步骤 1）：改为 `-cmatch` 解析 `[mt I] backend: <设备名>` 并与 `CPU`/`CUDA`/`Vulkan` 前缀比对；降级警告（`[mt W] …requested but no matching device`）不再被误当成结果 |
| 7 | `VULKAN_SDK` / PATH 与 `SPIRV-Headers` 的 **CONFIG 包** | ✅ **已核实**（2026-10-19）：安装器设好 `VULKAN_SDK=C:\VulkanSDK\1.4.363.0` 并把 `Bin` 入 PATH；CONFIG 包在 `<SDK>\Lib\cmake\SPIRV-Headers\`（**不是** `share\cmake`）——前置检查已按此修正（§2.3） |
| 8 | 手上 `moss-transcribe.exe` 的来源 | ✅ **已澄清（用户）**：上游仓库没有任何 GitHub Release，所以该二进制就是本仓 `scripts/build_moss.ps1` **自构建**的产物（现装的是 `-Backend cuda`，`runtime/bin/.backend=cuda`）——`bootstrap_moss.ps1 -BinaryUrl` 默认空正是为此 |
| 9 | **MOSS Vulkan 能否在 Windows 上构建成功**未确证：上游无 Windows CI，且 issue #1 显示 Windows + MinGW + GCC 16 在**非 Vulkan** 路径上就已编译失败（修复 PR #3 至今未合并） | ✅ **已实测**（2026-10-19，§2.5）：**MSVC + VS2022 BuildTools + cmake 4.3.3 路线构建成功**；issue #1 是 MinGW 路线，不适用。唯一障碍是 MAX_PATH（行 10），已修 |
| 10 | **MAX_PATH**：vulkan 构建的 TryCompile 瞬态尾巴 208 字符，工作根 > ~50 字符即 `FileTracker FTK1011`；构建目录不能挪进仓库 | ✅ **已修**（2026-10-19）：`build_moss.ps1` 默认工作根改 `<系统盘>\gsa-moss-build\<backend>-<PID>`（30 字符）+ `-WorkDir`（仅收空目录）+ 超长告警（根长+208 > 250）（§2.5） |
| 11 | `bootstrap_moss.ps1` 曾把 vulkan 后端的 `moss_device` 写成 `"vulkan"` ⇒ 双 GPU 机必落核显（§3.3 实测），且静默无告警 | ✅ **已修**（2026-10-19）：vulkan 后端改写 `""`（auto，实测偏好独显）；就绪自检新增落点复核（枚举表 + `uma:1` 警告）；欢迎页 ASR 卡显示当前 device |

---

## 6. 与并行会话的边界（避免改动冲突）

- 主 worktree 分支 `feat/misc-fixes`（`d08219c`）的 `docs/MISC_TODO.md` 第 12 项「CUDA GPU 加速参数化」要改 `src-tauri/src/ai_runtime/config.rs` 并做前端面板，与本设计 §3.5 重叠。
- 已确认划分：**本分支只碰 `scripts/` + 纯探测逻辑 + `docs/`**；字段与 UI 留给主 worktree。
- 本 worktree **没有 `runtime/`**；只读使用主 worktree 的 runtime（`GSA_RUNTIME_DIR`），**绝不写主 worktree 的文件**。

---

## 7. 本分支之外：FunASR 字级时间戳（独立验证项）

> **2026-10-30 更新：FunASR 已退役、不参与分发**，本节保留的是机制结论（供日后查证/复现），不再作为待办。

`scripts/funasr_worker.py:219` 注释称 Fun-ASR-Nano 的 `sentence_info` 含**字级 timestamp** 且断句后处理依赖它，`:243` 直接消费，`:70-73` 用 `len(ts) < 2` 决定"整段不切分"。

**已查明的机制（funasr 1.4.16 源码）**：Nano 只有一条时间戳通路——**CTC 强制对齐**（`fun_asr_nano/model.py:965-966,1057-1071` → `torchaudio.functional.forced_align`），**全仓不存在"按字数均分"的插值代码**。闸门在 `checkpoint_utils.py:22-47 disable_incomplete_ctc()`：checkpoint 缺 `ctc_decoder.*`/`ctc.*` 键时把 CTC 头置空，于是 `result` 连 `timestamps` 键都没有；随后 `auto/auto_model.py:1163-1187`（**本 worker 走这条**：不配 `punc_model` ⇒ 退化到 `vad_segment` 模式）拿不到 ts 时 `logging.error("No timestamp found in ASR result. …")`，令 `ts = []`、`start/end` 退化为 **VAD 段边界**。⇒ 那样的话 `:70-73` 的切分分支就是死代码，字级时间戳只剩段级精度（最长 30 s）。

**⚠ 但"checkpoint 缺 CTC 权重"这个前提已被实测推翻**：对 ModelScope 上 `FunAudioLLM/Fun-ASR-Nano-2512` 的 `model.pt`（revision `db823224e5c2510001b4a3d3ed97f430bd52ac88`，提交信息 **"upload ctc weights"**，2,127,426,538 字节）做 HTTP Range 只取头部 12 MB（`206`，`Content-Range: bytes 0-12582911/2127426538`），其 `merged/data.pkl` 中**逐字含 `ctc_decoder.linear1.weight`（`ctc_decoder` 命中 84 次）与 `ctc.ctc_lo.weight`**。官方 blog 与 issue #106 关于"发布版缺 CTC 权重"的说法**相对当前 checkpoint 已过期**。

⇒ 所以**不能**照搬"ts 必为空"的结论，现在有两种可能：(a) CTC 头可用 ⇒ ts 是**真实**强制对齐（这段代码没问题，台账无需登记）；(b) 仍拿不到 ts，但原因换成了别的（`model.py:810-817` 明确"batched 模式不产 CTC 时间戳"，而本 worker 传 `batch_size_s=120`）⇒ 缺陷依旧。**实测是唯一能分辨的手段。**

**实测的成本已查清**：主 worktree 的 FunASR 运行时已于 2026-09-30 按用户决策清理（实测 `deps_funasr`、`models/funasr`、`worker/funasr_worker.py` **均已不存在**）。要复现需重新安装：

- 走 `scripts/bootstrap_funasr.ps1 -Device cpu`：**只需 CPU 版 torch**（约 200 MB 量级）＋ funasr/modelscope 依赖 ＋ 4 个模型（主模型 Fun-ASR-Nano-2512 约 1.6 GB）≈ **2.5–3 GB**（CUDA 路线约 7 GB）。时间戳语义与设备无关，**cpu 路线即可**。
- 前置：脚本要求 `runtime/python/python.exe`（复用 OCR 的内嵌解释器，由 `scripts/bootstrap_ocr.ps1` 提供）。替代路线：用 `py -3.12`（指向主 worktree 的 3.12 解释器）在**本 worktree 内**建 venv。
- 判定方法（比"是否等距"更准）：dump `res[0].get("timestamps")`、`res[0]["sentence_info"][i]["timestamp"]`，并留意 stderr 的 `Disabling CTC timestamps…` / `No timestamp found…`。**空 = 缺陷成立；非空且与音频内容对齐 = 真实强制对齐**——同时验证 `batch_size_s=120` 是否把它推入 batched 模式。
---

## 8. 预构建分发（P1，2026-10-22）

> 起因（用户决策）：**不要求用户在自己机器上现构建**。改为 GitHub Release 发布预构建 zip + sha256 校验，安装器按 manifest 下载解包。分支 `feat/runtime-distribution`（基线 `f8dc4ac`）。

### 8.1 三个 flavor 的实测体积（本机，2026-10-22）

| flavor | zip | 展开 | 主要构成 |
|---|---|---|---|
| cpu | 1.1 MB | 2.7 MB | `moss-transcribe.exe` + `ggml*.dll`（无后端 DLL） |
| vulkan | 15.4 MB | 48.9 MB | 同上 + `ggml-vulkan.dll`（46 MB，SPIR-V 内核） |
| cuda | 621.6 MB | 1,159.7 MB | + `ggml-cuda.dll` 549 MB、`cublasLt64_12.dll` 507 MB、`cublas64_12.dll` 100 MB |

⇒ **默认走 vulkan**（15 MB，跨 NVIDIA/AMD/Intel 三厂），cuda 作为「按需补装」的第二条目；manifest 两条目共存，不默认下发 621 MB 的 cuda 包。

### 8.2 CUDA 目标架构：`75;86;89`

- 覆盖 Turing(75) / Ampere(86) / Ada(89)，**去掉 sm_80**（A100 数据中心卡，与玩家无关）；实测构建日志 `-- Using CMAKE_CUDA_ARCHITECTURES=75;86;89`。
- `build_moss.ps1` 新增 `-CudaArch`（默认 `75;86;89`）、`-OutputDir`、`-UpstreamRef`、`-Jobs`；产物落 `<OutputDir>/<flavor>/`，供打包脚本直接吃。

### 8.3 MSVC 运行库必须 app-local 自带

`dumpbin /dependents` 实测：`moss-transcribe.exe` 与 `ggml*.dll` **全部静态导入** `msvcp140.dll` / `vcruntime140.dll` / `vcruntime140_1.dll`（`ggml-base`/`ggml-cpu` 另需 `vcomp140.dll` = OpenMP）。Rust 主程序不需要（静态链 CRT），但干净 Windows 上缺任一即 `0xC0000135`，**表现与「缺 CUDA DLL」完全一样**，用户只能看到"启动失败"。

⇒ 采用微软的 app-local 部署（DLL 放 exe 同目录，免管理员权限、免安装次序），打包脚本校验这 4 个文件的 Authenticode 签名主体含 `Microsoft`。

### 8.4 工具链

- `scripts/package_moss_flavors.ps1`：打 zip + 产 `manifest.json`（与 `docs/RUNTIME_BOOTSTRAP_CONTRACT.md` §4 schema 对齐，`dest` 一律 `bin`）+ `packages.json`（构建记录：每文件哈希、上游 pin）。退出码 0/1/2/3 = 成功/参数/校验/打包。
- `scripts/verify_moss_flavors.ps1`：纯净性（不该有的后端 DLL 必须没有）+ 依赖 + 签名校验。
> ⚠ **zip 不是字节可复现的**：包内 `SOURCE.txt` 记录构建时刻、zip 条目带文件 mtime，同一份产物打两次哈希不同（实测 `098E28…` vs `272A23…`）。所以**必须以 `packages.json` 记录的哈希为准**，上传哪个文件就发那个文件的 sha256，不能靠"重建一遍对一下哈希"来校验。

### 8.5 ⚠ PowerShell 参数类型约束的坑（已踩，勿重犯）

`param([string]$Flavors)` 声明的变量**被永久约束为字符串**：后续 `$Flavors = @()` 会被强制转回字符串，`$Flavors += $x` 变成**字符串拼接**——表现是 `"cpu,vulkan"` 变成 `"cpuvulkan"` 或 `"cpu vulkan"`（分隔符丢失/变空格），且报错信息只显示"未知 flavor"，极难定位。

⇒ 拆分结果必须放进**另一个变量名**（本脚本用 `$FlavorList`），不要复用参数变量。另：用 `-File` 调用时 `-Flavors 'cpu,vulkan'` 传入的是**单个字符串**，不会绑成数组。

### 8.6 已完成：Release 发布与 manifest 填址

- ✅ **已发布**（2026-10-24）：release tag **`moss-runtime-v1`**（target `dev`，指向 `408e082`），三个资产 + manifest 绝对 URL 已填好。
  下载基址 `https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/moss-runtime-v1/`：

  | 资产 | 大小 | sha256 |
  |---|---|---|
  | `moss-cpu.zip` | 1,121,692 B | `545e2b1a18f35c6cd462c26b768d177458bf4d6918a619f24d4767492c4d791d` |
  | `moss-vulkan.zip` | 16,139,894 B | `e848847289982b8322262341aa133e40b8f152ee563b8af63e3412e4eb6a89b5` |
  | `moss-cuda.zip` | 651,834,262 B | `5faa48b8b85e7f5763b835ad8b18ba467daef8243f0b1c7172fb3a1c87ed894d` |

- **上传结果已由 GitHub 侧独立核验**：`gh release view --json assets` 返回的 `digest` 字段（`sha256:…`）与上表逐条一致，`size` 也一致 ⇒ 传输无损，不需要「重下一次再比哈希」。
- **匿名下载实测**：`moss-vulkan.zip` 从公网 URL 拉下来 16,139,894 B、哈希一致、耗时 6.6 s；解包后 11 个文件与本地产物完全相同（`.backend`=`vulkan`）。
- `manifest.json` 的三条 `url` 已从 `-UrlBase` 占位改为上面的绝对地址，`sha256`/`size`/`dest` 未变，本地三份 zip 重新哈希逐条对上。

### 8.7 发布操作备忘（`gh`）

```powershell
# 首次创建（同时带上小资产；cuda 621 MB 单独补传，实测耗时 10.5 分钟）
gh release create moss-runtime-v1 --target dev --title "..." -F notes.md moss-cpu.zip moss-vulkan.zip
gh release upload  moss-runtime-v1 moss-cuda.zip
```

> ⚠ **`--target` 只认分支名/标签**：传 commit SHA（`--target 408e082`）会被 GitHub 拒为
> `HTTP 422 Validation Failed: Release.target_commitish is invalid`。用 `--target dev` 即可——release 只需
> 指向一个包含该产物的提交，不要求 sha 精确对应（产物自身的 `SOURCE.txt` + `packages.json` 才是溯源依据）。
> 另：发布后**不要**再重打包上传，zip 不可复现（§8.4），重打就会与 manifest 的哈希对不上。

### 8.8 原「未完成」三条的结案（2026-10-26）

- **老卡分流** → 已出补丁 `_gsa_staging/patches/0001-old-gpu-cuda-fallback.patch`（算力 < 7.5 不再推荐 CUDA：`cuda_runnable()` 判据、`decide()` 老卡硬报错改指 Vulkan、`recommend()` 反向推荐），基线 = installer 侧 HEAD `dcdd1a5`，待那边 `git am`。
- **`manifest.json` 进仓库 + `bundle.resources`** → installer 侧已完成（`2a766c8`，第 12 项阶段 C）：`src-tauri/resources/runtime/manifest.json` + `tauri.conf.json` 的 `"resources": ["resources/runtime/manifest.json"]`。
- **契约 §4 示例 URL 的 `.7z`** → 契约侧已订正：`docs/RUNTIME_BOOTSTRAP_CONTRACT.md` 新增 §4.3「交付物是 zip（不是 7z）、以及解压的三个坑」。

另新增一件并已结案：**`.installed.json` 比对**（原写"本期不做"）→ 补丁 `_gsa_staging/patches/0002-installed-state-check.patch`（新增 `ai_runtime/installed_state.rs` + `probe_moss_install` 命令，七态）；写入方 `scripts/bootstrap_runtime.ps1` 与 Rust 侧安装基础设施随 `7d3f8d5` 交付（`ai_runtime/runtime_install.rs`：`precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install` + `runtime-install-progress` 事件）。两个补丁的套用说明与边界见 `_gsa_staging/patches/HANDOFF.md`。

### 8.9 运行时安装器（`bootstrap_runtime.ps1` + `runtime_install.rs`）

> 落地：`7d3f8d5`。目的是让「装运行时」不必再让用户手抄命令行——前端能起进程、看进度、能取消。
> 契约见 `docs/RUNTIME_BOOTSTRAP_CONTRACT.md`（本实现按它写，未改动契约）。

- **脚本侧**（`scripts/bootstrap_runtime.ps1`）：stdout 纯 JSONL（`start` / `progress` / `item_skip` / `item_done` / `item_fail` / `error` / `done`，末行必 `done`），人类诊断走 stderr；退出码 `0` 成功 / `1` 预检失败 / `2` 校验失败 / `3` 部分条目失败 / `4` 未知错误；`-Flavor auto` 走 L0 探测（`nvidia-smi` 算力 ≥ 7.5 → `vulkan-1.dll` → cpu），下载前做磁盘预检（下载量 ×3 + 100 MB），镜像失败自动回退直连，SHA256 通过后才解压，成功后写 `runtime/.installed.json` 并支持 `item_skip`。
- **Rust 侧**（`src-tauri/src/ai_runtime/runtime_install.rs`）：`precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install` 三个命令，进度经 `runtime-install-progress` 逐条转发；取消用 `taskkill /T /F` 杀进程树；同一时刻只允许一个安装任务。
- **为什么两者解耦**：子进程读写收敛在 `stream_child` + 退出码映射 `summarize` 两个不碰 Tauri 的函数里，单测可以用**真脚本、真子进程**跑通（`end_to_end_bogus_item_reports_failure`、`cancel_stops_waiting`），不必为测试造 mock。
- **遗留**：打包形态下脚本未随 app 发布（`bundle.resources` 目前只映射 manifest）。`bootstrap_candidates` 已预留 `resources/scripts/bootstrap_runtime.ps1` 位置，补一条资源映射即可。（2026-10-30 核对：该映射已由 `feat/misc-fixes` 的 `bd8ee43` 与 `scripts/sync_bundle_resources.mjs` 补齐，本分支尚未合入。）

### 8.10 模型并入分发（`models-moss`，2026-10-30）

> 起因：`probe_moss_install` 只看二进制 ⇒ 用户点完「安装运行时」显示 **ok**，第一次转写才报
> `MOSS 模型不存在`。gguf 有 618 MB，此前既不在清单里、也没有任何自动通路。

- **发布**：`models-moss.zip` = 626,397,892 B，sha256 `809f8999ac51ef0836b9324b2c1ad6a40e520b42f69dfc60c260dd01ab9a8cb1`，
  与三个 `moss-*.zip` 一起挂在 Release `moss-runtime-v1` 上（同一 tag 追加资产，不改版本号）。
- **清单**：第 4 条 `{id:"models-moss", dest:"models/moss"}`，**不带 `flavor` 与 `backend_marker`**
  ——三种后端共用同一份 q5_k，设备探测与模型无关。
- **缺省条目**：`bootstrap_runtime.ps1` 的 `-Items` 从 `moss` 改为 **`moss,models-moss`**（缺一不可）。
- **脚本修复**：`-Flavor auto` 的可用集合改为**本次选中条目的并集**。原实现只看
  `items[0]`，于是 `-Items models-moss` 单装模型时被误判成「清单里没有可用的 flavor」而直接失败；
  实测复现与修复后各跑一遍（`models-moss` 单装 → `auto: 所选条目与 flavor 无关，跳过设备探测`，
  退出码 0；`models-moss,moss` 组合 → `auto 选择: cpu`，两项 `item_done`，再跑一次两项 `item_skip`）。
- **状态机**：`evaluate` 增加 `model_missing`（二进制在、gguf 不在 ⇒ 跑不了，优先于版本类结论），
  并在「无 `.installed.json` 记录」的分支上也过一遍模型检查。
- **空字段 = 标准布局**：一键安装只解包、不写 `config.json`，因此 `moss_binary` / `moss_model`
  为空时必须回退到 `bin/moss-transcribe.exe` 与 `models/moss/moss-transcribe-q5_k.gguf`
  （新增 `moss_paths()`，`MossProvider::spawn`、`probe_moss_backend`、`probe_moss_install` 三处统一）。
  否则 `resolve_path("")` 会解析成 runtime 目录本身，把刚装好的运行时判成 `file_missing`。
- **BOM 容错**：`parse_installed` / `parse_manifest_items` 读文件前先剥 UTF-8 BOM
  ——PowerShell 5.1 的 `Set-Content -Encoding UTF8` 会加 BOM，而 serde_json 见 BOM 即语法错误，
  整份清单会被当成空 ⇒ 状态全丢。