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
剧情文字识别 + 打轴+转写（语音 ASR + 说话人分离 / 画面嵌字 OCR）+ 字幕整理
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
       PaddleOCR          MOSS / FunASR          Local LLM
       OCR Engine       ASR+Diarization        Qwen2.5-3B
                                      |
                              Project Storage
                              <name>.gsa (JSON)
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

目录（`src-tauri/src/`）：
```
├── video / ocr / asr / llm / fuse   # 视频处理、OCR、ASR、LLM 调用、融合
├── ai_runtime / export / project    # provider 抽象、字幕导出、项目持久化
└── timeline / subtitle              # 空壳占位（时间轴在前端 store，字幕生成在 export）
```

## Video Engine: FFmpeg

负责：视频读取、音频提取、截帧、编码

## OCR Engine: PaddleOCR

用途：识别游戏剧情文本、UI文字
运行方式：Rust → Python Worker → PaddleOCR
通信：JSON lines over stdio（常驻 worker 子进程，无端口/HTTP）

## ASR + Speaker Diarization: MOSS-Transcribe-Diarize 0.9B / FunASR

双引擎可选（用户按素材与硬件选）：MOSS-Transcribe-Diarize 0.9B（说话人分离自动、更准）或 FunASR + diarize（本地 GPU 更快；说话人分离用 WeSpeaker 嵌入，不经 pyannote）

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

默认：Qwen2.5-3B-Instruct (GGUF Q4_K_M)；可经 `runtime/config.json` 的 `llm_model` 换其它 GGUF

用途：语料与转写侧的对应（融合）——现为编号对应 + 语料文本逐字搬运；另供前端 LLM 测试台。**不产出角色名**（纯文本契约，见 §6）

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

（实现为 `TimelineEvent` 标签联合：`ocr_text` / `embed_ocr` / `ocr_region` / `asr` / `fused` / `manual`——见 [docs/development.md](docs/development.md) 的数据模型。）

## Project 结构

保存为 `<项目名>.gsa` 项目文件：首行为魔数头 `GSA-PROJECT v1`，其后为 JSON（实际字段如下，详见 [docs/development.md](docs/development.md)）：

```json
{
  "path": "<项目名>.gsa 的绝对路径（项目身份 = 文件）",
  "video": "",
  "source_video": "",
  "corpus": [ { "id": "", "text": "", "source": "paste|image_ocr|ocr_track", "created_at": "" } ],
  "corpus_ocr_diffs": [],
  "tracks": [
    {
      "id": "", "name": "",
      "type": "ocr_text|embed_ocr|ocr_region|asr|fused|manual",
      "track_role": "game|streamer", "scope": "output|control", "page": "", "video": "clip|source",
      "events": []
    }
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

变化检测使用 64-bit dHash（汉明距离阈值），避免每帧 OCR。

## ASR Pipeline

```
Audio → ASR 引擎（MOSS / FunASR + diarize）→ Speaker Segments → Text Events
```

输出：主播 S01、剧情角色 S02

## Character Resolver

目标：把 S01 转换为 "派蒙"

优先级（设计）：
1. OCR角色名匹配
2. 声音embedding
3. LLM辅助判断

**现状：未实现**——说话人契约已暂时整体放弃（缺陷台账 F1/F2）：融合只做文本搬运、**不产出角色名**，`character` 仅可人工填写（或由轨道命名同步到事件）。

## Subtitle Fusion

把可靠语料文本替换转写侧（ASR / 嵌字 OCR）文本，时间轴沿用转写侧段。
**现状**：LLM 编号对应 + 语料原文逐字搬运（纯文本契约）；实测该 LLM 未在做跨语言对齐（F13）。
**计划（T4）**：改为向量召回 + 统一转移 DP，见 §6.3。

---

# 7. 时间轴系统

支持多轨、拖动、分割、合并、修改时间

Track（实际类型 + 角色）：
```
Timeline
├── 控制轨 (scope=control)：ocr_region 选区，归属 page=corpus/asr/fuse/editor
└── 产物轨 (scope=output)：ocr_text / embed_ocr / asr (track_role=game|streamer) / fused / manual
```

---

# 8. 视频预览

Frontend: HTML5 Video + Canvas Overlay
显示：原视频、字幕实时预览、时间轴同步

---

# 9. 导出

已实现：单轨导出 SRT / ASS / LRC / TXT
后续：Premiere XML / DaVinci Resolve XML / Aegisub

---

# 10. AI Provider 接口设计

```rust
trait TextProcessor {
    fn process(input: String) -> Result<String>;
}
```

实现（现状）：`src-tauri/src/ai_runtime/` 的 provider trait + `llm.rs`（llama.cpp 调用）+ NoneProvider 占位；离线约束下不提供云端 provider（openai / ollama 未实现）。

---

# 11. 用户模式设计

规划：启动时 AI 模式选择：
- 完全离线模式（内置模型）
- 高级模式（自定义API）
- 混合模式（本地处理 + 云端翻译）

**现状：只有完全离线模式**——Rust 侧无 HTTP 客户端、应用不做任何网络请求（见 [docs/development.md](docs/development.md)）；自定义 API / 云端翻译与 §2.1 的离线约束冲突，需先复议，未实现。

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

**现状修正**：模型**不入包**，由 `scripts/bootstrap_*.ps1` 下载并校验 SHA256 后落到 `runtime/`；**应用启动只做就绪检测**（`check_*` 命令），不在应用内下载（安装包携带 runtime 的分发方案尚未配置）。

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
- OCR + 转写侧（ASR / 嵌字）合并、自动整理（已实现：LLM 编号对应 + 语料原文逐字搬运）
- 角色识别：**暂缓**（说话人契约整体放弃，见 F1/F2 与 §6 Character Resolver）

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
- 已实现：用户手动提供——上传剧情文本截图后 OCR（`run_ocr_images`），或直接粘贴 / 导入纯文本（`read_text_file`）
- 可靠文本与视频时间轴解耦，作为独立"文本语料"（由融合消费）

**② 上传要烤制的录播切片视频，获取精确的字幕轴与视频内嵌字轴**
- 打轴+转写有**两条并列路线**（可同时存在于同一工程，融合侧统一消费）：**ASR**（有配音：时间轴来自语音活动 + 说话人分离）与**嵌字 OCR**（无配音：时间轴来自画面文字变化）
- 用户手动标记：主播语音（仅供字幕轴参考 / 导出）与游戏内容（参与融合）
- 已实现：OCR 切片视频的内嵌字幕，适配没有配音的游戏任务
- 已封盘：起止两侧帧级真值归因完毕，0.5s 网格内无通用可修项（见 benchmark/OCR_TIMELINE_CLOSURE.md）——"变化检测处二分"不再列为待办

**③ 将可靠文本与转写侧（ASR / 嵌字）的外语文本对应**
- 用可靠的游戏文本替换转写文本，保留其时间轴
- 现状：LLM 融合（编号对应 + 语料原文逐字搬运）；实测该 3B LLM 的主导行为是**按下标对齐**，跨语言语义对齐未发生（缺陷台账 F13）
- 计划：T4 改用本地多语言向量召回 + 统一转移 DP（见 §6.3）

### 6.2 其他
- 字幕编辑器、双语字幕、翻译辅助

### 6.3 待办（跨轮次）：T4 落地 + 前端「融合后置信度审计表」（同轮做）

**T4 = 把已验证的向量召回 + 统一转移 DP 落地产品**，替换 `src-tauri/src/fuse/` 的 LLM 跨语言对齐。
依据：F13 证明 LLM 未在做跨语言对齐（[benchmark/FUSE_PIPELINE_DEFECTS.md](benchmark/FUSE_PIPELINE_DEFECTS.md)）；实验室路线四案例 **226/236 计分段（95.8%）**（[benchmark/FUSE_THRESHOLD_CALIBRATION.md](benchmark/FUSE_THRESHOLD_CALIBRATION.md) §7.9）。

**为什么与前端「融合后置信度审计表」同轮做**——两者**耦合**：审计表需要融合侧产出**置信度 / 告警信号**，而该信号的标定结论已经定了，必须一并设计：

- `score` **不是正确性信号**：精确率在任何阈值下都 ≈ 错误基准率（§7.9 实测；取 0.86 时标记率 89.4% 才换来召回 100% ⇒ 清单等价于"全量复核"）；
- 「该不配」行掩码判据**精确**（vesna 空集段 12/13、四案例假阳性 0），但**依赖语言对与嵌入模型**，须按素材开关（§7.6 / §7.9）；
- ⇒ 审计表**不应以 `score` 作为唯一信号**，需要结构化信号：**路径走了回退 / 复用**、**语料行被重复使用**、**掩码近似命中**、**段落未命中**。
  这条结论必须写进实现——否则后来者会照着已被证否的信号（`margin` / 单一 `score` 阈值）去实现。

**前端形态**（用户决策）：融合跑完后把低置信段罗列出来，逐段可选 **保留当前 / 从候选名单替换 / 在整个语料列表里选择替换 / 不配该条**。

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
