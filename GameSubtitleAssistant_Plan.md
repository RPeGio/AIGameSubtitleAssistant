# GameSubtitleAssistant 项目开发计划

## 1. 项目概述

### 项目名称（暂定）

**GameSubtitleAssistant**

### 项目定位

一个面向游戏剧情实况翻译/烤肉创作者的：

> 完全离线运行的 AI 游戏剧情字幕生产工作站。

目标是替代目前人工流程：

```
观看录播
 ↓
截图OCR
 ↓
复制文本
 ↓
寻找角色名
 ↓
人工打轴
 ↓
翻译整理
 ↓
导出字幕
```

通过 AI 自动完成：

```
视频输入
 ↓
剧情文字识别 + 语音识别 + 说话人分离 + 自动打轴 + 字幕整理
 ↓
人工校正
 ↓
SRT/ASS字幕输出
```

---

# 2. 核心设计原则

## 2.1 Offline First

软件必须支持：

- 无账号
- 无 API Key
- 不上传视频
- 不依赖云端服务

所有核心功能：

- OCR
- ASR
- Speaker diarization
- 文本整理

均支持本地运行。

---

## 2.2 Modular AI Runtime

AI模块必须可替换。禁止硬编码某个模型。

设计：

```
AI Runtime
├── OCR Provider
├── ASR Provider
├── Text Processing Provider
└── Translation Provider
```

支持：

- 本地模型
- 云端API
- 用户自定义模型

---

# 3. 总体架构

```
                  Desktop Application
                       Tauri 2
                         |
        --------------------------------
        |                              |
    Frontend                       Backend
    Vue3 TS                        Rust
        |                              |
 Timeline UI                 Core Processing Engine
 Video Preview                       |
 Subtitle Editor                     |
                               AI Runtime Layer
                                      |
          ------------------------------------------------
          |                    |                       |
       PaddleOCR            MOSS                 Local LLM
       OCR Engine       ASR+Diarization        Qwen/Gemma
                                      |
                              Project Storage
                              SQLite + JSON
```

---

# 4. 技术栈

## Desktop: Tauri 2

用途：桌面窗口、文件管理、系统调用、Rust桥接

## Frontend: Vue 3 + TypeScript

技术：Vue3 / Vite / Pinia / TailwindCSS / Naive UI

职责：视频播放器、时间轴、字幕编辑、参数设置

## Backend: Rust

负责：视频处理调度、AI任务管理、文件管理、时间轴数据、导出

目录：
```
src-tauri
├── video
├── timeline
├── subtitle
├── ai_runtime
├── export
└── project
```

## Video Engine: FFmpeg

负责：视频读取、音频提取、截帧、编码

## OCR Engine: PaddleOCR

用途：识别游戏剧情文本、UI文字
运行方式：Rust → Python Worker → PaddleOCR
通信：gRPC / ZeroMQ

## ASR + Speaker Diarization: MOSS-Transcribe-Diarize 0.9B

替代 FunASR + pyannote

输入：audio
输出：
```json
{
  speaker: "S01",
  start: 10.5,
  end: 14.2,
  text: "旅行者，你来了"
}
```

## Local LLM: llama.cpp Runtime

默认推荐：Qwen3-4B-Instruct / Qwen2.5-3B-Instruct (GGUF Q4_K_M)

用途：OCR文本整理、去重、格式化、OCR纠错、翻译辅助

---

# 5. 核心数据模型

## Subtitle Event

```typescript
interface SubtitleEvent {
  id: string;
  start: number;
  end: number;
  text: string;
  speaker?: string;
  character?: string;
  source: "ocr" | "asr" | "manual";
  confidence: number;
}
```

## Project 结构

保存为 `project.json`，格式：

```json
{
  "video": "",
  "tracks": [
    { "type": "subtitle", "events": [] },
    { "type": "voice", "events": [] }
  ]
}
```

---

# 6. 核心 Pipeline

## 视频输入

```
Video → FFmpeg → Frame Stream + Audio Stream
```

## OCR Pipeline

```
Frame → 字幕区域裁剪 → 变化检测 → OCR → Raw Text → Deduplicate → Subtitle Event
```

变化检测使用 SSIM / perceptual hash，避免每帧OCR。

## ASR Pipeline

```
Audio → MOSS → Speaker Segments → Text Events
```

输出：主播 S01、剧情角色 S02

## Character Resolver

目标：把 S01 转换为 "派蒙"

优先级：
1. OCR角色名匹配
2. 声音embedding
3. LLM辅助判断

## Subtitle Fusion

融合 OCR字幕 + ASR字幕 + Speaker信息，生成最终字幕事件。

---

# 7. 时间轴系统

支持多轨、拖动、分割、合并、修改时间

Track:
```
Timeline
├── Video Track
├── Character Subtitle Track
├── Streamer Subtitle Track
└── Translation Track
```

---

# 8. 视频预览

Frontend: HTML5 Video + Canvas Overlay
显示：原视频、字幕实时预览、时间轴同步

---

# 9. 导出

第一阶段：SRT / ASS
第二阶段：Premiere XML / DaVinci Resolve XML / Aegisub

---

# 10. AI Provider 接口设计

```rust
trait TextProcessor {
    fn process(input: String) -> Result<String>;
}
```

实现：local_llama.rs / openai.rs / ollama.rs / none.rs

---

# 11. 用户模式设计

启动时 AI 模式选择：
- 完全离线模式（内置模型）
- 高级模式（自定义API）
- 混合模式（本地处理 + 云端翻译）

---

# 12. 模型管理

不直接打包模型。结构：
```
GameSubtitleAssistant
├── app
├── runtime
└── models
      ├── moss
      └── qwen
```

首次启动检测模型是否存在 → 下载 → 校验hash → 启用

---

# 13. 开发阶段规划

## Phase 0：工程初始化
- Tauri + Vue + Rust 通信
- 创建项目、文件管理、基础UI

## Phase 1：视频工作台
- 视频导入、播放、时间轴基础、区域选择

## Phase 2：OCR字幕生成
- 视频 → OCR → Subtitle Event
- 区域裁剪、OCR参数调整、去重

## Phase 3：MOSS接入
- Audio → MOSS → Speaker Timeline
- 生成主播轴和角色语音轴

## Phase 4：AI融合
- OCR + ASR 合并、角色识别、自动整理

## Phase 5：Local LLM
- llama.cpp + Qwen 模型接入
- OCR纠错、格式化、智能整理

## Phase 6：字幕生产流程

### 6.1 完整生产链路（核心愿景）

把三个环节拼接成一条完整生产流水线，产出最终字幕：

```
① 可靠游戏内文本 ──► ② 录播切片字幕轴/嵌字轴 ──► ③ 文本对应替换 ──► 最终字幕
```

**① 获取可靠的游戏内文本**
- 现状：OCR 剧情录屏（区域框选 + 变化检测 + PaddleOCR + 去重合并）
- 未来：支持用户手动提供——上传剧情文本截图后 OCR，或直接上传纯文本
- 可靠文本与视频时间轴解耦，作为独立"文本语料"（后续由 LLM 融合消费）

**② 上传要烤制的录播切片视频，获取精确的字幕轴与视频内嵌字轴**
- ASR 说话人分离：提供说话人时间轴 + 转录文本
- 用户手动标记：主播语音（用于字幕轴）与游戏内语音（用于视频内嵌字轴）
- 未来：OCR 切片视频的字幕，适配没有配音的游戏任务
- 未来：变化检测处使用二分（或其他方法）提高时间轴准确度

**③ 将可靠文本与嵌字轴中的转写外语文本对应**
- 用可靠的游戏文本替换转写文本，保留其时间轴
- 实现：LLM 融合（链路已基本搭建，但未实测实际长文本场景下的效果，需后续优化）

### 6.2 其他
- 字幕编辑器、双语字幕、翻译辅助

## Phase 7：高级功能（未来）
- TTS、声音克隆、自动翻译、游戏角色声音库

---

# 14. MVP 目标

第一版必须完成：
1. 导入游戏录屏
2. OCR 剧情字幕
3. MOSS 识别声音
4. 自动生成时间轴
5. 人工调整
6. 导出 ASS/SRT

暂不实现：TTS、自动翻译、云端AI、声音克隆

---

# 15. 项目最终愿景

> 一个免费、开源、完全离线的 AI 游戏剧情本地化工作站。

目标用户：游戏剧情烤肉作者、Vtuber切片作者、JRPG翻译组、游戏社区字幕组

核心卖点：无需 API Key、无需订阅、无需上传视频、一次安装、AI自动完成剧情字幕生产
