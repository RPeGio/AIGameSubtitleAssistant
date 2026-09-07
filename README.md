# GameSubtitleAssistant
**中文** | [English](README_EN.md)

> 完全离线运行的 AI 游戏剧情字幕生产工作站。

GameSubtitleAssistant 面向游戏剧情实况翻译（烤肉）、切片和字幕组创作者，把「观看录播 → （截图 OCR → 复制文本 → 找角色名）（此三步可概括为游戏语料收集） → 人工打轴 → 翻译整理 → 导出字幕」的人工流程，压缩为一条 AI 辅助流水线：

```
视频输入 ──► 剧情文字识别(OCR) + 语音识别(ASR) + 说话人分离 + 自动打轴
        ──► AI 融合(文本替换) ──► 人工校正 ──► SRT / ASS / LRC / TXT
```

### **无需账号、无需 API Key、不上传视频、不依赖云端服务**——所有 AI 推理（OCR / ASR / 说话人分离 / LLM 文本对齐）**均在本地完成**。
### [（点击跳转使用指南）](#使用指南)

> ⚠️ 本项目处于早期阶段（v0.1.0），目前仅支持 Windows，尚未提供安装包：AI 运行环境需要通过脚本在本地搭建（见[快速开始](#快速开始)）。

## 功能特性

- **文本语料收集（OCR）**：对剧情录屏框选游戏字幕区域，通过变化检测跳过未变化帧，PaddleOCR 识别 + 多数投票合并，产出与时间轴解耦的「可靠游戏文本」；在其它平台已有游戏语料的情况下也支持直接识别字幕截图或粘贴/导入纯文本。
- **语音转写（ASR）**：双引擎可选——MOSS-Transcribe-Diarize 0.9B（说话人分离最强且自动，更准）或FunASR（Fun-ASR-Nano，本地 GPU 更快）；一键把每条说话人轨标记为「游戏内容」（提供时间轴参与融合）或「主播语音」（仅供字幕转写导出）。
- **嵌字轴**：对切片视频框选内嵌字幕区域跑 OCR，生成视频内嵌字时间轴，适配无配音的任务。
- **AI 融合**：本地 LLM（llama.cpp + Qwen2.5-3B）把可靠语料与游戏语音转写做**跨语言语义对齐**——只让模型输出对应关系，文本由代码逐字复制，避免小模型「复述」改字；角色名从语料前缀自动提取。
- **时间轴编辑**：多轨时间轴（分割 / 合并 / 拖拽 / 吸附 / 分段互换）、视频预览联动、撤销重做、逐条校对列表。
- **导出**：单轨导出 SRT / ASS / LRC / TXT。
- **工程化**：项目数据（`project.json`）1 秒防抖自动保存 + Ctrl+S 手动保存，最近项目列表。

## 工作流

应用按「四页工作台」组织，侧边栏按序导航：

```
① 文本语料 Corpus ──► ② 语音转写 ASR ──► ③ AI 融合 Fuse ──► ④ 时间轴编辑 Editor
  剧情录屏 OCR /        切片视频 ASR +        可靠文本替换      校对 + 导出
  截图 OCR / 手动文本   说话人标记 + 嵌字OCR   转写文本(留时间轴) 最终字幕
```

项目挂载**两个视频源**：`source`（剧情录屏，语料 OCR 的对象，与时间轴解耦）与 `clip`（要烤制的切片视频，全局时间轴基准）。融合产物写入唯一的「最终字幕」轨，在编辑页校对后导出。

## 环境要求

| 依赖 | 说明 |
|---|---|
| Windows 10/11 x64 | 目前唯一支持平台（内嵌 Python 为 win_amd64，脚本为 PowerShell） |
| [Node.js](https://nodejs.org/) 18+（建议 20 LTS）+ [pnpm](https://pnpm.io/) | 前端构建 |
| Rust stable（MSVC 工具链） | 桌面端编译，见 [Tauri 前置要求](https://tauri.app/zh-cn/start/prerequisites/) |
| FFmpeg + ffprobe | 需自行安装并加入 `PATH`，应用不会自动下载 |
| NVIDIA GPU（可选） | FunASR 默认 CUDA 12.4；llama.cpp / MOSS 支持 cpu / cuda / vulkan 后端，无 GPU 用 cpu（转写时间延长数倍） |

各 AI 模块合计占用 13.9 GB 左右磁盘空间（模型 + Python 依赖）。

## 快速开始

```powershell
git clone https://github.com/RPeGio/AIGameSubtitleAssistant.git
cd AIGameSubtitleAssistant
pnpm install
```

AI 运行环境通过 `scripts/` 下的 PowerShell 引导脚本搭建，脚本会把内嵌 Python、依赖和模型统一装到 `runtime/` 目录（已 gitignore），并自动下载、校验（SHA256）官方模型：

```powershell
# ① OCR（必装：语料页与嵌字轴依赖它；也是其它脚本的 Python 基础）
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr.ps1

# ② ASR 二选一或都装：MOSS（推荐，默认 cpu 后端；GPU 版二进制需先用 build_moss.ps1 自构建）
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_moss.ps1 [-Backend cpu]
#   或 FunASR（需 GPU 时默认 -Device cuda）
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_funasr.ps1 [-Device cpu]

# ③ LLM（AI 融合依赖）
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_llm.ps1 [-Backend cpu]
```

常用参数：`-HfMirror https://hf-mirror.com`（模型镜像）、`-GitMirror https://gh-proxy.com/`（GitHub 镜像）、`-PipMirror https://mirrors.aliyun.com/pypi/simple`（pip 镜像）、`-Force`（重装/重下）。详见各脚本头部注释。

启动开发模式或构建：

```powershell
pnpm tauri dev     # 开发模式
pnpm tauri build   # 产出安装包（注：运行时资源打包尚未配置，见下文已知限制）
```

## 使用指南

1. **创建项目**：欢迎页「创建新项目」，选择一个空文件夹（项目数据保存在其中的 `project.json`）。
2. **导入视频**：语料页导入剧情录屏（source）；转写页导入切片视频（clip）。
3. **① 文本语料**：框选字幕区域 + 时间段，跑 OCR 收集可靠文本；或识别字幕截图 / 粘贴纯文本。
4. **② 语音转写**：对切片视频跑 ASR（选引擎 / 说话人上限 / 语言），把含游戏语音的轨标记为「游戏内容」；无配音任务可额外框选内嵌字幕区域生成嵌字轴。
5. **③ AI 融合**：语料与时间轴双就绪后开始融合，得到以转写 / 内嵌时间轴为准、文本来自可靠语料的最终字幕段。
6. **④ 时间轴编辑**：逐条校对（改文本 / 角色、分割、合并、删除），导出 SRT / ASS / LRC / TXT。

## 项目结构

```
├── src/                    # Vue 3 前端（视图 / 组件 / stores / 路由）
├── src-tauri/              # Rust 后端
│   └── src/
│       ├── video/          # ffprobe 元数据、帧/音频提取
│       ├── ocr/            # OCR 流水线（抽帧→变化检测→识别→合并）
│       ├── asr/            # ASR 调度（双引擎）
│       ├── llm/            # llama.cpp 调用
│       ├── fuse/           # LLM 跨语言融合
│       ├── export/         # SRT/ASS/LRC/TXT 导出
│       ├── project/        # 项目数据模型与持久化
│       └── ai_runtime/     # AI 运行时（provider 抽象、配置、worker 子进程）
├── scripts/                # 环境引导脚本（bootstrap_*.ps1）与 Python worker
├── runtime/                # 本地 AI 运行环境（脚本生成，gitignored）
└── GameSubtitleAssistant_Plan.md  # 项目计划书
```

## 文档

- [GameSubtitleAssistant_Plan.md](GameSubtitleAssistant_Plan.md) — 项目定位、架构设计与分阶段计划
- [docs/development.md](docs/development.md) — 开发文档（架构 / 数据模型 / 流水线细节 / 测试）
- [CONTRIBUTING.md](CONTRIBUTING.md) — 参与贡献
- [THIRD_PARTY.md](THIRD_PARTY.md) — 第三方组件与模型许可证

## 路线图

**已完成**：四页工作台全链路（语料 → 转写 → 融合 → 编辑导出）、OCR 流水线（区域框选 / 变化检测 / 多数投票 / 多帧合并）、FunASR + MOSS 双 ASR 引擎与说话人分离、嵌字轴、LLM 跨语言融合、多轨时间轴编辑与撤销重做、单轨多格式导出、自动保存。

**进行中 / 待改进**：融合链路在真实长文本场景的实测与优化；运行时资源打包（安装包分发）；前端样式优化，工程存储配置重构等。

**未来计划**：翻译辅助与双语字幕、更多导出格式（Premiere XML / DaVinci Resolve XML / Aegisub）、应用内模型管理、TTS 与声音克隆（见计划书 Phase 7）。

## 已知限制

- 仅支持 Windows；macOS / Linux 需要移植引导脚本与运行时。
- 无安装包，AI 环境依赖脚本搭建；模型在安装阶段下载，应用内不自动下载（仅有就绪检测）。
- FFmpeg 需自备（加入 `PATH`）。
- 融合质量受本地小模型能力限制，长文本场景仍在优化中。
- 默认 LLM 为 Qwen2.5-3B-Instruct，采用 **Qwen Research 许可**（商用需另行授权），详见 [THIRD_PARTY.md](THIRD_PARTY.md)。

## 许可证

本项目源代码以 [MIT License](LICENSE) 发布。第三方依赖与模型（PaddleOCR、FunASR、MOSS、llama.cpp、Qwen 等）各自保留其原始许可证——若你在 `runtime/` 中下载使用它们，请遵守对应条款，详见 [THIRD_PARTY.md](THIRD_PARTY.md)。

## 致谢

本项目的 AI 能力建立在以下优秀开源项目之上：
- [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR)
- [FunASR](https://github.com/modelscope/FunASR)
- [MOSS-Transcribe-Diarize](https://github.com/OpenMOSS) / [moss-transcribe.cpp](https://github.com/mudler/moss-transcribe.cpp)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
- [WeSpeaker](https://github.com/wenet-e2e/wespeaker)
- [Silero VAD](https://github.com/snakers4/silero-vad)