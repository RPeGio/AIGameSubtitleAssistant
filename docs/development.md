# 开发文档

[English](development_EN.md) | 中文

本文面向想理解代码结构或参与开发的协作者。项目愿景与分阶段计划见 [GameSubtitleAssistant_Plan.md](GameSubtitleAssistant_Plan.md)，上手使用见 [README](../README.md)。

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面壳 | Tauri 2（`protocol-asset`、dialog / opener 插件） |
| 前端 | Vue 3 + TypeScript + Pinia + Vue Router + Naive UI + Tailwind CSS 4 + Vite 6 |
| 后端 | Rust 2021（tauri / serde / image / uuid / ffmpeg-sidecar） |
| AI | PaddleOCR（Python worker）、FunASR / MOSS（ASR）、multilingual-e5-small + 统一转移 DP（融合）、llama.cpp + Qwen2.5-3B（LLM，测试台） |

约束：**完全离线**。Rust 侧无 HTTP 客户端，应用不做任何网络请求；所有 AI 能力通过本地子进程实现。

## 总体架构

```
┌─────────────────────────────────────────────────────────────┐
│ 前端 (Vue 3)                                                │
│  ProjectLayout = AppSidebar + 工作区视图 + ReviewPane        │
│  stores: project(项目态+自动保存) / timeline(多实例,inject)  │
└──────────────┬──────────────────────────────────────────────┘
               │ invoke() 命令 / listen() 事件（ocr-progress 等）
┌──────────────▼──────────────────────────────────────────────┐
│ Rust 后端 (src-tauri)                                       │
│  video ── ffmpeg/ffprobe（外部依赖，PATH 查找）              │
│  ocr / asr / llm / fuse ── 各自的 pipeline + 进度事件        │
│  ai_runtime ── provider trait + manager + runtime 配置      │
│  project / export ── <项目名>.gsa 持久化 / 字幕导出          │
└──────────────┬──────────────────────────────────────────────┘
               │ 子进程 + stdio（无端口/HTTP）
┌──────────────▼──────────────────────────────────────────────┐
│ runtime/（bootstrap 脚本生成）                                │
│  python/ + deps/            ← PaddleOCR worker（常驻）       │
│  deps_funasr/ + models/     ← funasr_worker.py（一次性）     │
│  deps_embed/ + models/embed/ ← embed_worker.py（一次性，融合）│
│  bin/moss-transcribe.exe    ← MOSS CLI（一次性）             │
│  bin/llm/llama-cli.exe      ← llama.cpp（一次性，测试台）    │
└─────────────────────────────────────────────────────────────┘
```

## 后端模块地图

| 模块 | 职责 |
|---|---|
| `src-tauri/src/video/mod.rs` | ffprobe 元数据（`get_video_metadata`）；帧提取 / 音频提取供 OCR / ASR 复用；FFmpeg 定位：系统 PATH → ffmpeg-sidecar 目录，找不到报 `FFMPEG_NOT_FOUND` |
| `src-tauri/src/ocr/`（`mod.rs` 门面 + `params`/`pipeline`/`merge`/`refine`/`correct`/`commands` 子模块） | 完整 OCR 流水线 `run_ocr`（后台线程 + `ocr-progress` 事件）：抽帧 → 变化检测 → 窗口精化 → OCR → 合并；`run_ocr_images` 直接识别截图；`approve_corpus_diff` 采纳语料纠正（替换规则的单一实现，与基准共用） |
| `src-tauri/src/ai_runtime/dhash.rs` | 帧裁切区域的 64-bit dHash + 汉明距离变化判定 |
| `src-tauri/src/ai_runtime/mod.rs` | AI Runtime 骨架：OCR/ASR/LLM 的 provider trait + manager + NoneProvider 占位（环境未就绪时保证可编译、返回"未就绪"） |
| `src-tauri/src/ai_runtime/paddle.rs` | PaddleOCR worker provider：常驻子进程，JSON lines over stdio（`{"id":1,"images":[...]}` / `ping` / `shutdown`） |
| `src-tauri/src/ai_runtime/funasr.rs` | FunASR provider：一次性 `funasr_worker.py <wav>`，stdout 输出 JSON 段数组；启动前清理孤儿 worker 进程 |
| `src-tauri/src/ai_runtime/moss.rs` | MOSS CLI provider：`moss-transcribe transcribe <model.gguf> <audio.wav> --format json`；转写前清理残留进程 |
| `src-tauri/src/ai_runtime/llm.rs` | llama.cpp provider：一次性 `llama-cli -m <model> -p <prompt> -st -n <tokens> --no-display-prompt`（融合能力测试台用，产品融合不再依赖） |
| `src-tauri/src/ai_runtime/embed.rs` | 向量融合 provider：一次性 `embed_worker.py <in.json> <out.json>`（E5-small 编码 + 统一转移 DP）；出参逐段含 `ocr_index` / `score` / `product_text` / `candidates` / `masked` / `transition` / `reused` / `source_text` 与统计 |
| `src-tauri/src/ai_runtime/config.rs` | `RuntimeConfig`：runtime 目录与各 provider 路径/参数的解析与校验（见下文） |
| `src-tauri/src/asr/mod.rs` | `run_asr`：音频提取（RAII 临时目录）→ 按参数选引擎 → `AsrSegment` 列表；重入守卫 + 取消（`asr_cancel`） |
| `src-tauri/src/llm/mod.rs` | `run_llm`：单次 prompt 推理（前端测试台用）；`llm-progress` 事件 |
| `src-tauri/src/fuse/mod.rs` | `run_fuse`：向量召回 + 统一转移 DP 融合，产出最终字幕段与审计元数据（详见[融合流水线](#融合流水线fuse)） |
| `src-tauri/src/project/mod.rs` | Project / Track / TimelineEvent 数据模型、`<项目名>.gsa` 读写（魔数头 `GSA-PROJECT v1` + JSON 主体，原子写）、最近项目列表 |
| `src-tauri/src/export/mod.rs` | `export_track_subtitle`：单轨导出 SRT / ASS / LRC / TXT（SRT/LRC/TXT 带 UTF-8 BOM 防播放器按 GBK 误读；ASS 固定样式并转义 `\` / `{}`） |

`subtitle/`、`timeline/` 是空壳注释模块：字幕生成实际在 `export`，时间轴实际在前端 store。

## AI Runtime 层

### runtime/ 目录

```
runtime/
├── config.json          # 唯一配置文件（bootstrap 脚本合并写入）
├── python/              # 内嵌 CPython 3.12.10（NuGet 包解压）
├── deps/                # OCR pip 依赖（--target 隔离）
├── deps_gpu/            # OCR GPU 专有件（可选，bootstrap_ocr_gpu.ps1；运行时 PYTHONPATH=deps_gpu;deps）
├── deps_funasr/         # FunASR pip 依赖（torch 与 paddle 依赖隔离）
├── deps_embed/          # 向量召回 pip 依赖（onnxruntime CPU + tokenizers，融合用）
├── models/
│   ├── paddleocr/       # PP-OCRv5 缓存
│   ├── funasr/          # Fun-ASR-Nano + VAD + 说话人模型（modelscope 缓存）
│   ├── embed/           # multilingual-e5-small（ONNX qint8，融合用）
│   ├── moss/            # moss-transcribe-<quant>.gguf
│   └── qwen/            # qwen2.5-3b-instruct-q4_k_m.gguf（可选，测试台）
├── worker/              # bootstrap 拷贝的 worker 脚本
└── bin/                 # moss-transcribe.exe、bin/llm/（llama.cpp）
```

### config.json 关键字段

| 字段 | 说明 |
|---|---|
| `python_path` / `worker_script` / `deps_dir` / `model_dir` | OCR worker 相关（相对 runtime 目录） |
| `ocr_model` | `mobile`（默认）或 `server`（PP-OCRv5 档位） |
| `language` / `dev_debug` | OCR 语言、调试帧落盘（`temp/`） |
| `moss_binary` / `moss_model` / `moss_threads` / `moss_device` | MOSS 配置；`moss_device`：空=auto（ggml 自选）/ cpu / cuda / vulkan（显式请求不可用时硬报错，见下节） |
| `llm_binary` / `llm_model` / `llm_threads` | llama.cpp 配置（可选，仅融合能力测试台） |
| `asr_provider` | `moss` / `funasr` / 空=自动 |
| `funasr_worker` / `funasr_deps` / `funasr_model_dir` / `funasr_device` / `funasr_language` / `funasr_timeout_minutes` | FunASR 配置 |
| `embed_worker` / `embed_model` / `embed_tokenizer` / `embed_deps_dir` | 融合 worker 脚本 / E5-small 模型 / 分词器 / 依赖路径；由 `bootstrap_embed.ps1` 写入，产品融合与实验台共用；**四个字段整组必配或整组为空** |

### MOSS 后端与三设备实测（2026-10-19）

后端由 ggml 按 `MTD_DEVICE` 决定（空=自动；双 GPU 实测 auto 偏好独显）；**实际生效后端只认 stderr 的 `[mt I] backend:` 行**——请求不可用的后端会静默降级（exit 0、无警告，`backend.cpp` 对 `vulkan` 这类"合法名"不告警）。

同一基准三设备各跑一次：`examples/benchmark_examples/quality_bench_test(voiced)_5min.mp4` 提取 5:14.99 / 16kHz mono wav（与 `video::extract_audio` 同参），`moss-transcribe-q5_k.gguf`，`--format json`，未设 `MTD_THREADS`：

| 设备（取值） | 实际 backend | 耗时 | 相对 CPU |
|---|---|---|---|
| CPU（`MTD_DEVICE=cpu`） | `CPU` | 680.6 s | 1× |
| Vulkan（空 = auto） | `Vulkan1`（RTX 4060） | 134.7 s | 5.0× |
| CUDA（空 = auto，CUDA 构建） | `CUDA0` | 76.2 s | 8.9× |

- Vulkan 比 CUDA 慢 1.77×，但仍比 CPU 快 5 倍——无 NVIDIA 的机器上 Vulkan 构建收益明确。
- 转写文本三方一致（总字数 3765/3765/3775，首末段逐字相同；段数 84 vs 90 属分界粒度差异）。
- ⚠ 双 GPU 机器上裸 `MTD_DEVICE=vulkan` 落**核显**（枚举序第一块，`Vulkan0`）；产品侧 vulkan 构建的 `moss_device` 应留空（auto），见 `docs/engines_vulkan_design.md` §3.3/§5-11。

### MOSS 后端语义与回退（`backend_probe.rs`，feat/engines-gpu-vulkan）

`moss_device` 取值与语义：`""`（=auto，ggml 自选，双 GPU 实测偏好独显）/ `cpu` / `cuda` / `vulkan`。判定链三级（`ai_runtime/backend_probe.rs`，15 单测）：

- **L2（唯一可信判据）**：spawn 时以与转写**同参**拉起 `info <model>`（显式设备才设 `MTD_DEVICE`），从 stderr 取 `[mt I] backend:` 行——静默降级（exit 0）只有这行能识破；
- **L0/L1**：nvidia-smi + vulkaninfo（机器能力）、`.backend` 标记 + `ggml-{cuda,vulkan}.dll`（已装资产），用于报错文案与"该装哪个后端"建议；
- **回退语义 = 方案 A**：资产层同时只有一套 ggml 后端（`ggml.dll` 同名互斥），故"回退"= 探测已装后端 + 告诉用户装哪个，**不是运行期切换**。显式 cpu/cuda/vulkan 与实测跨族不符 = **硬报错**（提示 `build_moss.ps1 -Backend …` 重装或改回 auto），不自动降级；只有 auto 允许 ggml 下沉（落 `uma:1` 核显时在理由中提示）。

实测生效后端经 `AsrProvider::effective_backend()`（仅 MOSS 实现）→ `check_asr_engines` 的 `effective_backend` 字段 → 欢迎页 ASR 卡显示「设备 · 配置值（实测 …）」。Vulkan 构建：`scripts/build_moss.ps1 -Backend vulkan`（需 Vulkan SDK/glslc；工作根超 ~50 字符会触发 MAX_PATH FTK1011，脚本已默认短根并告警）。详见 `docs/engines_vulkan_design.md`。

### runtime 目录解析顺序

1. 环境变量 `GSA_RUNTIME_DIR`（必须含 `config.json`）
2. 从 exe 目录逐级向上找 `runtime/config.json`（开发期即仓库根）
3. 兜底：Tauri `app_data_dir/runtime`

`validate()` 保证所有相对路径不越出 runtime 目录（防路径穿越）、python / moss / llm 二进制存在；`embed_*` 四项**整组必配或整组为空**（半配视为配置错误，避免运行期才失败），已配置时同样逐项做越界校验；模型文件不做存在性检查（由 provider 报"未就绪"）。

## 数据模型（`project/mod.rs`）

```
Project
├── path                     # .gsa 项目文件绝对路径（项目身份 = 文件，同一目录可有多个项目）
├── video                    # 切片视频 —— 全局时间轴基准
├── source_video             # 剧情录屏 —— 语料 OCR 的文本源
├── corpus: Vec<CorpusItem>  # 可靠文本语料（独立于轨道，供融合消费）
├── corpus_ocr_diffs: Vec<Diff>  # 语料 OCR 的待审批文本纠正（术语表纠错；缺省空，旧项目兼容）
└── tracks: Vec<Track>
     ├── track_type: "ocr_region" | "ocr_text" | "asr" | "manual" | "translation"
     ├── track_role:  "streamer" | "game"        # 仅 asr 轨，缺省 game
     ├── scope:       "control" | "output"       # 控制轨(如 OCR 选区) / 产物轨，缺省 output
     ├── page:        "corpus" | "asr" | "fuse" | "editor"   # 仅控制轨
     ├── video:       "source" | "clip"          # 轨道绑定哪个视频源，缺省 clip
     ├── preview_visible / events
```

`TimelineEvent` 是 serde tagged union（`type` 字段区分）六个变体：

| 变体 | 语义 |
|---|---|
| `ocr_text` | 可靠剧情文本（语料来源之一，含置信度） |
| `embed_ocr` | 切片内嵌字幕识别文本（嵌字轴——「待替换的转写文本」，结构同 ocr_text 但语义相反） |
| `ocr_region` | 归一化字幕选区（控制轨） |
| `asr` | 转写段（+ speaker；**character 不再由融合写入**——纯文本契约，仅可人工填写 / 由轨道命名同步） |
| `fused` | 融合产物（最终字幕轨） |
| `manual` | 手动事件 |

前端 `src/types/index.ts` 是与 Rust 结构镜像的 TS 定义，新增字段需两侧同步。

项目以 `<项目名>.gsa` 单文件保存在项目文件夹根：首行为魔数头 `GSA-PROJECT v1`，其后为 JSON 主体（即上方结构，字段不变）。文件名由 `sanitize(项目名)` 生成（替换 Windows 非法字符等），写入采用临时文件 + rename 原子替换。**项目身份 = .gsa 文件路径**：同一目录允许共存多个项目；打开支持两种形态——`.gsa` 文件路径直接打开，目录路径则查找其中唯一 `*.gsa`（兼容旧最近项目列表）；保存永远写回 `path` 指向的文件，项目改名只改 JSON 内字段、文件名不变。

## 前端结构

- **路由**（`src/router/index.ts`）：`/` 欢迎页；`/project/:path` → `ProjectLayout`，子路由 `corpus` / `asr` / `fuse` / `editor`（默认重定向到 editor）。
- **ProjectLayout** 三栏：`AppSidebar`（四步导航 + 保存状态）+ 工作区 `<router-view>` + `ReviewPane`（全局校对区：VideoPlayer + Timeline + TrackOverview）。持有全局快捷键 Ctrl+S / Ctrl+Z / Ctrl+Shift+Z。
- **stores/project.ts**：项目态 + 全部 Tauri 调用编排。深监听整个项目对象，任何变更 1 秒防抖自动保存（`applyingSaved` 防回环、await 期间切项目防竞态）；关闭前 best-effort flush。
- **stores/timeline.ts**：时间轴 store。组件通过 `inject(TIMELINE_STORE_KEY)` 获取实例，找不到才 fallback 到全局 store——同一组件树可挂多个独立时间轴（语料页迷你轴、打轴+转写页嵌字轴、校对区主轴）。
- **stores/fuseAudit.ts**：融合审计表的纯逻辑（零 Vue 依赖）——风险分层（`auditRisk`：已复核 > 结构信号 A 层（掩码 / 回退 / 复用 / 未命中）> 低分 B 层（`score < 0.82`））、行构建与排序、计数、四动作裁决（`applyFusedDecision`：保留 / 候选替换 / 全语料选择 / 不配）与 `reused` 重算；由 `project.ts` 的 `fusedAuditRows` / `applyFusedAuditDecision` 接入项目态并纳入撤销重做（失败零副作用）。
- **视图**：`Welcome`（项目创建/最近项目）、`CorpusView`（三种语料来源）、`AsrView`（② 打轴+转写：ASR（有配音）/ 嵌字 OCR（无配音）两种方案 + 说话人标记）、`FuseView`（双就绪卡片 → 融合 → 审计表：A 层默认展开、低分 B 层折叠标记、全语料弹窗惰性加载）、`Editor`（fused 轨逐条校对 + 导出）。

## 关键流水线

### OCR 流水线（`ocr/mod.rs`）

```
抽帧(ffmpeg) ─► 裁切选区 ─► dHash 变化检测(汉明距离≤阈值跳过OCR)
            ─► PaddleOCR worker(仅变化帧) ─► 帧文本顺延 ─► 合并成段
```

- 参数（前端默认值）：帧间隔 0.5s、dHash 阈值 3、批大小 16、合并相似度 0.3、字幕最短时长 1.5s（`DEFAULT_MIN_SUBTITLE_SEC`）。
- **合并规则**（`merge_frames`）：相邻相似文本归入同一 run；run 内**多数投票**选最终文本（替代"更长者优先"——被噪声污染的更长文本总相似度低，不会被选中）；空帧有 `(interval*1.5).max(0.8)` 的抖动容错窗口；打字机式渐进文本（前缀超集）合并为一条保留最长。
- 产物去向由前端决定：source 模式 → corpus 语料（去时间轴）；clip + page=asr → embed_ocr 嵌字轨。
- **精度纠错 = 只标记不改写**（用户决策，2026-09-24/25）：末步做标点归一化（静默，目标字符可配）
  与术语表模糊匹配，但**只产出** `Diff { old: Vec<String>, new: String }`（`old` 为集合语义：
  同一词条可对应多种误读形态，按 `new` 聚合）。`run_ocr` 返回 `(Vec<OcrSegment>, Vec<Diff>)`，
  文本保持"归一化后、未精化"形态；纠正由前端**逐条审批**（语料页"待审批纠正"列表）——
  采纳 = 对全部 corpus 条目替换并清理撞同文的重复条目，放弃 = 仅移除条目，两者都可撤销。
  术语表为**语料 OCR 独有**（嵌字面板无入口）。采纳规则在基准与前端各一份实现，须同步：
  `tests/common::apply_diffs_to_segments` ↔ `stores/project.ts::approveCorpusOcrDiff`。

### 融合流水线（`fuse/mod.rs`）

- **路线**：本地多语言向量召回（multilingual-e5-small qint8，ONNX / CPU）+ **统一转移 DP**。语料与转写段各自编码后取相似度矩阵，再用全局单调 DP 选整体最优路径——转移含前进 / 复用（`repeat_penalty`）/ 回退（`reset_penalty`，允许语料行被跳过后回跳）/ 不配（`unmatched`）。四案例实测 **210/236 计分段**，开启「该不配」掩码后 **226/236（95.8%）**；阈值与判据标定见 [benchmark/FUSE_THRESHOLD_CALIBRATION.md](../benchmark/FUSE_THRESHOLD_CALIBRATION.md) §7.9。此前基于 llama.cpp 的 LLM 编号对应路线实测**未在做跨语言语义对齐**（缺陷台账 F13），已随本路线退役；`src/llm/` 与 `run_llm` 仅保留给融合能力测试台。
- **纯文本契约**：命中段逐字复制语料原文（含换行与角色名行），未命中段保留转写原文；不改写、不剥离前缀、不做换行归一。**不产出说话人/角色名**——`character` 不再由融合写入（缺陷台账 F1/F2）。
- 输入约定（前端保证）：语料**只来自 corpus**（不回退 ocr_text 轨，防止把"待替换文本"当可靠语料）；转写段 = game 轨 ASR 段 + embed_ocr 嵌字段（**两种转写侧方案可并存，此处统一消费**；与 ASR 重叠占比过高的嵌字段丢弃——有配音处不靠嵌字）。R2 起不再向转写文本注入说话人首行（该注入会干扰前缀注意力，已移除）。
- 「该不配」掩码（可选，按素材开关，前端复选框，默认关）：判据是"该转写段的内容在语料任何一行里都放不下"——`len_sub >= 1` 且 `len_sub <= α · max{len_corpus[j] : j ∈ top-K}`（α=0.95 / K=3 / 相似度并列容差 `tie_eps`=0.0015）。四案例假阳性为 0，但收益依赖语言对（显示语言比语料语言单位内容更省），故按素材开关（当前仅 vesna 开）。**换嵌入模型或素材必须重标**（`tie_eps` 有效窗口仅约 2 倍宽）。
- 输出：时间轴沿用转写段；每段带审计元数据 `ocr_index` / `score` / `product_text` / `candidates` / `masked` / `transition` / `reused` / `source_text`（融合时刻的转写原文）/ `reviewed`（人工已复核），供前端审计表分层与四动作裁决。降级计数：`masked_segments`（掩码判为"不配"的段数）、`reused_segments`（复用语料行的段数）；`ocr_index` 越界直接报错，不静默降级。
- 工具：`scripts/fuse_core.py`（Python 单一真源：编码 / 相似度 / DP / 掩码）、`scripts/embed_worker.py`（产品一次性 worker）、`scripts/fuse_worker_parity.py`（实验室 ↔ worker 对拍）；实验台与判据见 `src-tauri/tests/fuse_lab/` 与 `scripts/fuse_*.py`。

### 进度事件

| 事件名 | 载荷 |
|---|---|
| `ocr-progress` | `{clip_index, clip_count, progress, message}` |
| `asr-progress` | `{progress: 0.0~1.0, message}` |
| `llm-progress` | `{progress: 0.0~1.0, message}`（LLM 测试台与**融合进度共用此事件名**） |

前端事件名常量集中在 `src/types/index.ts`，与 Rust 侧常量需保持一致。

## Tauri 命令清单

| 模块 | 命令 |
|---|---|
| project | `create_project` `open_project` `save_project` `set_project_video` `read_text_file` `list_recent_projects` `rename_project` `remove_recent_project` |
| video | `get_video_metadata` |
| ai_runtime | `check_ocr_runtime` `check_asr_runtime` `check_asr_engines` `check_llm_runtime` `check_embed_runtime` `asr_cancel` |
| ocr | `run_ocr` `run_ocr_images` `approve_corpus_diff` |
| asr | `run_asr` |
| llm | `run_llm` |
| fuse | `run_fuse` |
| export | `export_track_subtitle` |

退出钩子会调用 `asr.cancel()` 清理子进程，避免孤儿进程占满 CPU。

## 测试

`src-tauri/tests/` 下八个集成测试：端到端 `ocr_e2e.rs`、`asr_e2e.rs`、`fuse_e2e.rs`、`llm_e2e.rs`、`ocr_bench_refinement.rs`；质量基准 `bench_corpus.rs`、`bench_hardsub.rs`、`bench_pierro_embed_ocr.rs`（全部 `#[ignore]`，素材与跑法见 [benchmark/README.md](../benchmark/README.md)）。多数为端到端测试，**需要 runtime 环境就绪**（模型、二进制在位）才能通过；各模块内另有不依赖环境的单元测试（如"未就绪时报错且不触发回调"）。

**融合能力基准**（原 `bench_fusion.rs`，`#[ignore]`）随 LLM 融合路线退役而删除：它用自撰的干净用例判别"跨语言语义对齐"是否真的发生——语料顺序打成无不动点排列，使"照抄编号"的恒等映射每条皆错，故内容正确率可直接对比随机基线。实测结论已落档 [benchmark/FUSE_PIPELINE_DEFECTS.md](../benchmark/FUSE_PIPELINE_DEFECTS.md) F13；同一判别器的 Python 版本仍在 `fuse_lab`（下节），可复现该结论。

```powershell
cd src-tauri; cargo test
```

**融合对齐实验台**（`src-tauri/tests/fuse_lab/`，Python，非 `cargo test` 流程）是上述结论的可复现工具，
T4 落地后仍作为融合阈值 / 判据的**标定与回归来源**：LLM 判别器、**向量召回判别器**、**离线验证流水线**（预校对工程 → 召回 + 单调 DP → SRT）。
产品侧另用 `scripts/fuse_worker_parity.py` 做「实验室 ↔ 产品 worker」逐段对拍（8 组零差异）。
依赖 `scripts/bootstrap_embed.ps1` 搭的 CPU 运行时（onnxruntime + E5-small qint8 113MB，不占 GPU）。
详见 [src-tauri/tests/fuse_lab/README.md](../src-tauri/tests/fuse_lab/README.md)；
路线结论见 [benchmark/FUSE_VECTOR_RECALL_VALIDATION.md](../benchmark/FUSE_VECTOR_RECALL_VALIDATION.md)。

## 打包与分发现状

- `tauri.conf.json` 的 `bundle.resources` **尚未配置**：安装包不会携带 `runtime/`。分发方案（把 `runtime/**` 打进 bundle 并让 `config.rs` 识别安装后的资源目录布局）见 `config.rs` 内 TODO。
- FFmpeg 不打包：应用按 PATH → ffmpeg-sidecar 目录的顺序查找。
- 模型下载只存在于 bootstrap 脚本；应用侧只有就绪检查（`check_*` 命令）。

## 协作约定

见 [AGENTS.md](../AGENTS.md)，要点：Rust 核心逻辑写中文注释、TypeScript 保持简洁；小步提交（conventional commits）；改动只触及任务本身；流程细节见 [CONTRIBUTING.md](../CONTRIBUTING.md)。
