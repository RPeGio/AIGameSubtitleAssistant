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

export interface ManualEvent extends TimelineEventBase {
  type: "manual";
  text: string;
  character?: string;
}

export type TimelineEvent = OcrTextEvent | OcrRegionEvent | AsrEvent | ManualEvent;

export interface Track {
  id: string;
  name: string;
  type: string;
  events: TimelineEvent[];
}

export interface Project {
  path: string;
  video: string;
  name: string;
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
}

export interface OcrSegment {
  start: number;
  end: number;
  text: string;
  confidence: number;
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

export interface AsrProgress {
  progress: number;
  message: string;
}

/// ASR 进度事件名（Rust 侧 `ASR_PROGRESS_EVENT` 需保持一致）
export const ASR_PROGRESS_EVENT = "asr-progress";
