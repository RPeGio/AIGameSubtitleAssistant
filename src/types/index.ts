export interface TimelineEventBase {
  id: string;
  start: number;
  end: number;
}

export interface OcrTextEvent extends TimelineEventBase {
  type: "ocr_text";
  text: string;
  confidence: number;
}

/// 切片视频内嵌字幕 OCR 事件 —— 主播画面中游戏字幕的识别文本（嵌字轴，供融合）。
/// 结构同 ocr_text，但语义不同：这是"待替换的转写文本"，不是"可靠剧情文本"
export interface EmbedOcrEvent extends TimelineEventBase {
  type: "embed_ocr";
  text: string;
  confidence: number;
}

export interface OcrRegionEvent extends TimelineEventBase {
  type: "ocr_region";
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export interface AsrEvent extends TimelineEventBase {
  type: "asr";
  text: string;
  speaker: string;
  character?: string;
  confidence: number;
}

export interface FusedEvent extends TimelineEventBase {
  type: "fused";
  text: string;
  character?: string;
  /// 融合审计元数据（T4 起写入；旧工程无此字段）
  fuse_meta?: FuseMeta;
}

export interface ManualEvent extends TimelineEventBase {
  type: "manual";
  text: string;
  character?: string;
}

export type TimelineEvent =
  | OcrTextEvent
  | EmbedOcrEvent
  | OcrRegionEvent
  | AsrEvent
  | FusedEvent
  | ManualEvent;

export interface Track {
  id: string;
  name: string;
  type: string;
  /// 轨道内容属性（仅 asr 轨道使用）："streamer" 主播语音 | "game" 游戏内容
  /// 旧项目缺省视为 "game"
  track_role: string;
  /// 轨道角色："control" 控制轨（页面工作状态，如 OCR 选区）| "output" 产物轨（字幕数据）
  /// 旧项目缺省视为 "output"
  scope: string;
  /// 控制轨归属页面（仅 scope=control 使用）："corpus" | "asr" | "fuse" | "editor"
  page: string;
  /// 轨道绑定的视频源："source" 剧情录屏（文本源）| "clip" 切片（时间轴基准）
  /// 旧项目缺省视为 "clip"
  video: string;
  /// 是否在预览窗口中显示该轨道字幕（纯显示偏好，随项目保存）
  preview_visible: boolean;
  events: TimelineEvent[];
}

export interface CorpusItem {
  id: string;
  text: string;
  /// 来源："paste" 手动粘贴 | "image_ocr" 截图 OCR | "ocr_track" 从 OCR 轨提取
  source: string;
  created_at: string;
}

export interface Project {
  /// .gsa 项目文件的绝对路径（项目身份 = 文件，同一目录可有多个项目）
  path: string;
  /// 切片视频路径 —— 时间轴基准
  video: string;
  /// 剧情录屏视频路径（文本源，OCR 语料用）
  source_video: string;
  name: string;
  /// 可靠文本语料集合（独立于轨道，供 LLM 融合消费）
  corpus: CorpusItem[];
  /// 语料 OCR 的待审批文本纠正（术语表纠错）。Rust 侧只标记、**不改写**语料文本；
  /// 采纳 = 对全部 corpus 条目执行 old→new 替换并移除该条目，放弃 = 仅移除条目。
  /// 重跑语料 OCR 时整体覆盖；旧项目文件无此字段（后端 serde default 补空数组）
  corpus_ocr_diffs: Diff[];
  tracks: Track[];
  created_at: string;
  updated_at: string;
}

export interface RecentProject {
  path: string;
  name: string;
  updated_at: string;
}

export interface VideoMetadata {
  path: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  codec: string;
  audio_codec: string | null;
}

export interface OcrRegionInput {
  start: number;
  end: number;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

export interface OcrRunParams {
  frame_interval: number;
  dhash_threshold: number;
  batch_size: number;
  merge_similarity: number;
  /// 字幕预估最短长度（秒，默认 1.5，与后端 DEFAULT_MIN_SUBTITLE_SEC 同源）：短于此的
  /// 产出段若与后一条弱关联则并入后一条（保留碎片起点 + 后条终点/文本）。调大能减少碎片，
  /// 但会提高误吞真实短句的概率（实测 1.6s 时基准语料出现缺失）；≤0 关闭该合并。
  min_subtitle_sec: number;
  /// 标点归一化配置（精度策略统一前置层；目标字符可由用户个性化）
  punctuation: PunctuationNorm;
  /// 术语表（可选精度策略）：正确词条列表，产出文本在标点归一化后与之模糊匹配并纠正。
  /// 空数组 = 关闭。前端为可增删的条目列表。
  glossary: string[];
}

/// 标点归一化配置：把 OCR 产出的各类括号/省略号统一为用户偏好的形态。
/// 归一化是术语表纠错与一致性纠错的前置条件（两侧词条需同形才能匹配）。
export interface PunctuationNorm {
  /// 左括号目标（默认「，可改 [）
  open_bracket: string;
  /// 右括号目标（默认」，可改 ]）
  close_bracket: string;
  /// 省略号目标（默认 …，可改 ……）
  ellipsis: string;
  /// 是否修复标点被识别成拉丁字母（如行尾 j → 右括号；默认开）
  fix_misread_letters: boolean;
}

export interface OcrSegment {
  start: number;
  end: number;
  text: string;
  confidence: number;
}

/// 一条待审批的文本纠正（Rust 侧 `ocr::Diff`，术语表/一致性纠错共用）。
/// `old` 为集合语义：同一目标词条可对应多种误读形态，采纳时一并替换。
/// Rust 侧只标记、不改写文本，纠正由前端审批后执行。
export interface Diff {
  old: string[];
  new: string;
}

export interface OcrProgress {
  clip_index: number;
  clip_count: number;
  progress: number;
  message: string;
}

/// OCR 进度事件名（Rust 侧 `OCR_PROGRESS_EVENT` 需保持一致）
export const OCR_PROGRESS_EVENT = "ocr-progress";

export interface AsrSegment {
  start: number;
  end: number;
  text: string;
  /// Rust Option<String> 序列化为 "S01"/null
  speaker: string | null;
  /// MOSS 无置信度输出 → null
  confidence: number | null;
}

export interface AsrRunParams {
  /// 引擎："funasr" | "moss"
  engine: string;
  /// 说话人上限：null = 自动估计（仅 funasr+diarize 生效；moss 忽略）
  max_speakers: number | null;
  /// 识别语言：null = 自动检测（仅 funasr 生效；moss 自动识别）
  language: string | null;
}

export interface AsrEngineStatus {
  /// "funasr" | "moss"
  engine: string;
  ready: boolean;
  message: string;
}

export interface AsrProgress {
  progress: number;
  message: string;
}

/// ASR 进度事件名（Rust 侧 `ASR_PROGRESS_EVENT` 需保持一致）
export const ASR_PROGRESS_EVENT = "asr-progress";

export interface LlmRuntimeStatus {
  /// provider 名称（"none" / "llama" 等）
  provider: string;
  ready: boolean;
  message: string;
}

export interface LlmProgress {
  progress: number;
  message: string;
}

/// LLM 进度事件名（Rust 侧 `LLM_PROGRESS_EVENT` 需保持一致）
export const LLM_PROGRESS_EVENT = "llm-progress";

/// AI 融合输入：一个游戏内容转写段（index = 输入顺序，向量召回输出按此对应）。
/// text 原样进入对齐输入（含换行）；**不带说话人前缀**（F14，见 fuse/mod.rs）。
export interface FuseAsrInput {
  index: number;
  start: number;
  end: number;
  text: string;
}

/// 候选名单条目（1-based 语料下标 + 该行的相似度）
export interface FuseCandidate {
  ocr_index: number;
  score: number;
}

/// 融合逐段审计元数据（由向量 worker 产出；score 不是正确性信号 —— 见标定 §7.9）
export interface FuseMeta {
  /// 命中的语料下标（1-based）；0 = 不配
  ocr_index: number;
  /// 命中行的余弦相似度；不配时为 0
  score: number;
  /// 候选名单（相似度降序，含命中的那条）
  candidates: FuseCandidate[];
  /// 该段被「该不配」判据整行封死
  masked: boolean;
  /// forward / repeat / reset / unmatched
  transition: "forward" | "repeat" | "reset" | "unmatched";
  /// 该段命中的语料行被 >=2 段命中
  reused: boolean;
}

/// 融合产物段（纯文本契约：无说话人字段，文本逐字搬运）
export interface FusedSegment {
  start: number;
  end: number;
  /// 命中段 = 语料原文（含换行/角色名行）；未命中段 = 转写原文
  text: string;
  /// false = 未匹配 OCR 文本，保留转写原文本
  matched: boolean;
}

export interface FuseStats {
  total: number;
  matched: number;
  /// 「该不配」掩码命中数（该段被判必不配；是精确判据，假阳性 0）
  masked_segments: number;
  /// 「候选语料行被重复使用」的段数（同一段语料被 ≥2 段命中）
  reused_segments: number;
}

export interface FuseResult {
  segments: FusedSegment[];
  /// 与 segments 一一对应的审计元数据（旧后端无此字段）
  metas?: FuseMeta[];
  stats: FuseStats;
}
