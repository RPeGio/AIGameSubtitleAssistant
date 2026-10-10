import type { OcrRunParams, PunctuationNorm } from "../types";

/// 语料/嵌字 OCR 参数的共享默认值（CorpusView 与 AsrView 各自创建独立 ref，
/// 互不共享状态；新增参数只改此处）。
export function defaultPunctuation(): PunctuationNorm {
  return {
    open_bracket: "「",
    close_bracket: "」",
    ellipsis: "…",
    fix_misread_letters: true,
  };
}

/// 产品默认 `min_subtitle_sec` 与后端 `DEFAULT_MIN_SUBTITLE_SEC` 同源（1.5s）
export function createDefaultOcrParams(): OcrRunParams {
  return {
    frame_interval: 0.5,
    dhash_threshold: 3,
    batch_size: 16,
    merge_similarity: 0.3,
    min_subtitle_sec: 1.5,
    punctuation: defaultPunctuation(),
    glossary: [],
  };
}
