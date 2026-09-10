# 开发文档

[English](development_EN.md) | 中文

本文面向想理解代码结构或参与开发的协作者。项目愿景与分阶段计划见 [GameSubtitleAssistant_Plan.md](../GameSubtitleAssistant_Plan.md)，上手使用见 [README](../README.md)。

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面壳 | Tauri 2（`protocol-asset`、dialog / opener 插件） |
| 前端 | Vue 3 + TypeScript + Pinia + Vue Router + Naive UI + Tailwind CSS 4 + Vite 6 |
| 后端 | Rust 2021（tauri / serde / image / uuid / ffmpeg-sidecar） |
| AI | PaddleOCR（Python worker）、FunASR / MOSS（ASR）、llama.cpp + Qwen2.5-3B（LLM） |

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
│  bin/moss-transcribe.exe    ← MOSS CLI（一次性）             │
│  bin/llm/llama-cli.exe      ← llama.cpp（一次性）            │
└─────────────────────────────────────────────────────────────┘
```

## 后端模块地图

| 模块 | 职责 |
|---|---|
| `src-tauri/src/video/mod.rs` | ffprobe 元数据（`get_video_metadata`）；帧提取 / 音频提取供 OCR / ASR 复用；FFmpeg 定位：系统 PATH → ffmpeg-sidecar 目录，找不到报 `FFMPEG_NOT_FOUND` |
| `src-tauri/src/ocr/mod.rs` | 完整 OCR 流水线 `run_ocr`（后台线程 + `ocr-progress` 事件）：抽帧 → 变化检测 → 窗口精化 → OCR → 合并；`run_ocr_images` 直接识别截图 |
| `src-tauri/src/ai_runtime/dhash.rs` | 帧裁切区域的 64-bit dHash + 汉明距离变化判定 |
| `src-tauri/src/ai_runtime/mod.rs` | AI Runtime 骨架：OCR/ASR/LLM 的 provider trait + manager + NoneProvider 占位（环境未就绪时保证可编译、返回"未就绪"） |
| `src-tauri/src/ai_runtime/paddle.rs` | PaddleOCR worker provider：常驻子进程，JSON lines over stdio（`{"id":1,"images":[...]}` / `ping` / `shutdown`） |
| `src-tauri/src/ai_runtime/funasr.rs` | FunASR provider：一次性 `funasr_worker.py <wav>`，stdout 输出 JSON 段数组；启动前清理孤儿 worker 进程 |
| `src-tauri/src/ai_runtime/moss.rs` | MOSS CLI provider：`moss-transcribe transcribe <model.gguf> <audio.wav> --format json`；转写前清理残留进程 |
| `src-tauri/src/ai_runtime/llm.rs` | llama.cpp provider：一次性 `llama-cli -m <model> -p <prompt> -st -n <tokens> --no-display-prompt` |
| `src-tauri/src/ai_runtime/config.rs` | `RuntimeConfig`：runtime 目录与各 provider 路径/参数的解析与校验（见下文） |
| `src-tauri/src/asr/mod.rs` | `run_asr`：音频提取（RAII 临时目录）→ 按参数选引擎 → `AsrSegment` 列表；重入守卫 + 取消（`asr_cancel`） |
| `src-tauri/src/llm/mod.rs` | `run_llm`：单次 prompt 推理（前端测试台用）；`llm-progress` 事件 |
| `src-tauri/src/fuse/mod.rs` | `run_fuse`：LLM 跨语言融合（详见[融合流水线](#融合流水线fuse)） |
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
├── deps_funasr/         # FunASR pip 依赖（torch 与 paddle 依赖隔离）
├── models/
│   ├── paddleocr/       # PP-OCRv5 缓存
│   ├── funasr/          # Fun-ASR-Nano + VAD + 说话人模型（modelscope 缓存）
│   ├── moss/            # moss-transcribe-<quant>.gguf
│   └── qwen/            # qwen2.5-3b-instruct-q4_k_m.gguf
├── worker/              # bootstrap 拷贝的 worker 脚本
└── bin/                 # moss-transcribe.exe、bin/llm/（llama.cpp）
```

### config.json 关键字段

| 字段 | 说明 |
|---|---|
| `python_path` / `worker_script` / `deps_dir` / `model_dir` | OCR worker 相关（相对 runtime 目录） |
| `ocr_model` | `mobile`（默认）或 `server`（PP-OCRv5 档位） |
| `language` / `dev_debug` | OCR 语言、调试帧落盘（`temp/`） |
| `moss_binary` / `moss_model` / `moss_threads` / `moss_device` | MOSS 配置；`moss_device`：空=自动 / cpu / cuda / vulkan |
| `llm_binary` / `llm_model` / `llm_threads` | llama.cpp 配置 |
| `asr_provider` | `moss` / `funasr` / 空=自动 |
| `funasr_worker` / `funasr_deps` / `funasr_model_dir` / `funasr_device` / `funasr_language` / `funasr_timeout_minutes` | FunASR 配置 |

### runtime 目录解析顺序

1. 环境变量 `GSA_RUNTIME_DIR`（必须含 `config.json`）
2. 从 exe 目录逐级向上找 `runtime/config.json`（开发期即仓库根）
3. 兜底：Tauri `app_data_dir/runtime`

`validate()` 保证所有相对路径不越出 runtime 目录（防路径穿越）、python / moss / llm 二进制存在；模型文件不做存在性检查（由 provider 报"未就绪"）。

## 数据模型（`project/mod.rs`）

```
Project
├── path                     # .gsa 项目文件绝对路径（项目身份 = 文件，同一目录可有多个项目）
├── video                    # 切片视频 —— 全局时间轴基准
├── source_video             # 剧情录屏 —— 语料 OCR 的文本源
├── corpus: Vec<CorpusItem>  # 可靠文本语料（独立于轨道，供融合消费）
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
| `asr` | 转写段（+ speaker，character 由融合阶段填） |
| `fused` | 融合产物（最终字幕轨） |
| `manual` | 手动事件 |

前端 `src/types/index.ts` 是与 Rust 结构镜像的 TS 定义，新增字段需两侧同步。

项目以 `<项目名>.gsa` 单文件保存在项目文件夹根：首行为魔数头 `GSA-PROJECT v1`，其后为 JSON 主体（即上方结构，字段不变）。文件名由 `sanitize(项目名)` 生成（替换 Windows 非法字符等），写入采用临时文件 + rename 原子替换。**项目身份 = .gsa 文件路径**：同一目录允许共存多个项目；打开支持两种形态——`.gsa` 文件路径直接打开，目录路径则查找其中唯一 `*.gsa`（兼容旧最近项目列表）；保存永远写回 `path` 指向的文件，项目改名只改 JSON 内字段、文件名不变。

## 前端结构

- **路由**（`src/router/index.ts`）：`/` 欢迎页；`/project/:path` → `ProjectLayout`，子路由 `corpus` / `asr` / `fuse` / `editor`（默认重定向到 editor）。
- **ProjectLayout** 三栏：`AppSidebar`（四步导航 + 保存状态）+ 工作区 `<router-view>` + `ReviewPane`（全局校对区：VideoPlayer + Timeline + TrackOverview）。持有全局快捷键 Ctrl+S / Ctrl+Z / Ctrl+Shift+Z。
- **stores/project.ts**：项目态 + 全部 Tauri 调用编排。深监听整个项目对象，任何变更 1 秒防抖自动保存（`applyingSaved` 防回环、await 期间切项目防竞态）；关闭前 best-effort flush。
- **stores/timeline.ts**：时间轴 store。组件通过 `inject(TIMELINE_STORE_KEY)` 获取实例，找不到才 fallback 到全局 store——同一组件树可挂多个独立时间轴（语料页迷你轴、转写页嵌字轴、校对区主轴）。
- **视图**：`Welcome`（项目创建/最近项目）、`CorpusView`（三种语料来源）、`AsrView`（ASR + 说话人标记 + 嵌字 OCR）、`FuseView`（双就绪卡片 → 融合）、`Editor`（fused 轨逐条校对 + 导出）。

## 关键流水线

### OCR 流水线（`ocr/mod.rs`）

```
抽帧(ffmpeg) ─► 裁切选区 ─► dHash 变化检测(汉明距离≤阈值跳过OCR)
            ─► PaddleOCR worker(仅变化帧) ─► 帧文本顺延 ─► 合并成段
```

- 参数（前端默认值）：帧间隔 0.5s、dHash 阈值 3、批大小 16、合并相似度 0.3。
- **合并规则**（`merge_frames`）：相邻相似文本归入同一 run；run 内**多数投票**选最终文本（替代"更长者优先"——被噪声污染的更长文本总相似度低，不会被选中）；空帧有 `(interval*1.5).max(0.8)` 的抖动容错窗口；打字机式渐进文本（前缀超集）合并为一条保留最长。
- 产物去向由前端决定：source 模式 → corpus 语料（去时间轴）；clip + page=asr → embed_ocr 嵌字轨。

### 融合流水线（`fuse/mod.rs`）

- 输入约定（前端保证）：OCR 文本**只来自 corpus**（不回退 ocr_text 轨，防止把"待替换文本"当可靠语料）；ASR 段 = game 轨 + embed_ocr 嵌字段（与 ASR 重叠占比过高的嵌字段丢弃——有配音处不靠嵌字）。
- 分批：OCR 文本**全量**入每批 prompt（语义匹配需要全局视野），ASR 段每批 30 条（`BATCH_SIZE`），`MAX_TOKENS=4096`。
- Prompt 设计：OCR/GC 编号带前缀（`OCR[1]` / `GC[3]`，GC = 游戏内容时间轴段）——批内两套编号无前缀时小模型会混淆；LLM 只输出 `{"index", "ocr_index", "character"}` 对应关系，**最终文本由代码从 OCR 列表逐字复制**——实测小模型无法可靠"复制文本"，让它复述会改字。
- 输出：时间轴沿用 ASR 段；`matched=false` 或整批 JSON 解析失败的段保留 ASR 原文本（`failed_batches` 统计）。

### 进度事件

| 事件名 | 载荷 |
|---|---|
| `ocr-progress` | `{clip_index, clip_count, progress, message}` |
| `asr-progress` | `{progress: 0.0~1.0, message}` |
| `llm-progress` | `{progress: 0.0~1.0, message}` |

前端事件名常量集中在 `src/types/index.ts`，与 Rust 侧常量需保持一致。

## Tauri 命令清单

| 模块 | 命令 |
|---|---|
| project | `create_project` `open_project` `save_project` `set_project_video` `read_text_file` `list_recent_projects` `rename_project` `remove_recent_project` |
| video | `get_video_metadata` |
| ai_runtime | `check_ocr_runtime` `check_asr_runtime` `check_asr_engines` `check_llm_runtime` `asr_cancel` |
| ocr | `run_ocr` `run_ocr_images` |
| asr | `run_asr` |
| llm | `run_llm` |
| fuse | `run_fuse` |
| export | `export_track_subtitle` |

退出钩子会调用 `asr.cancel()` 清理子进程，避免孤儿进程占满 CPU。

## 测试

`src-tauri/tests/` 下五个集成测试：`ocr_e2e.rs`、`asr_e2e.rs`、`fuse_e2e.rs`、`llm_e2e.rs`、`ocr_bench_refinement.rs`。多数为端到端测试，**需要 runtime 环境就绪**（模型、二进制在位）才能通过；各模块内另有不依赖环境的单元测试（如"未就绪时报错且不触发回调"）。

```powershell
cd src-tauri; cargo test
```

## 打包与分发现状

- `tauri.conf.json` 的 `bundle.resources` **尚未配置**：安装包不会携带 `runtime/`。分发方案（把 `runtime/**` 打进 bundle 并让 `config.rs` 识别安装后的资源目录布局）见 `config.rs` 内 TODO。
- FFmpeg 不打包：应用按 PATH → ffmpeg-sidecar 目录的顺序查找。
- 模型下载只存在于 bootstrap 脚本；应用侧只有就绪检查（`check_*` 命令）。

## 协作约定

见 [AGENTS.md](../AGENTS.md)，要点：Rust 核心逻辑写中文注释、TypeScript 保持简洁；小步提交（conventional commits）；改动只触及任务本身；流程细节见 [CONTRIBUTING.md](../CONTRIBUTING.md)。
