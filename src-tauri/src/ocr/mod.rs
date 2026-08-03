// ─── OCR 字幕生成流水线 ────────────────────────────────────
// 纯函数部分：
//   ocr_pass    —— 只 OCR 变化帧，未变化帧顺延文本，产出每帧的 FrameText
//   merge_frames —— 连续相同文本合并成 OcrSegment（事件）
//
// 依赖：
//   2.3 extract_frames  →  Vec<ExtractedFrame>
//   2.4 detect_changes  →  Vec<FrameChange>
//   2.2 OcrProvider     →  批量识别
//
// 最终写入 ocr_text 轨道由 2.6 的编排命令负责。

use crate::ai_runtime::dhash::FrameChange;
use crate::ai_runtime::{OcrError, OcrProvider};

/// 合并后的一段字幕事件（不含 id，写轨道时由 store 分配）
#[derive(Debug, Clone)]
pub struct OcrSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f64,
}

/// 一帧的文本（顺延后的完整序列）
#[derive(Debug, Clone)]
pub struct FrameText {
    pub time: f64,
    pub text: String,
    pub confidence: f64,
}

/// 只 OCR 变化帧，未变化帧顺延上一次 OCR 的文本。
///
/// - 变化帧的路径按顺序批量送去 `recognize_batch`
/// - 结果按变化帧顺序回填
/// - worker 结果数不足时按空文本处理（防御，不 panic）
pub fn ocr_pass(
    changes: &[FrameChange],
    provider: &dyn OcrProvider,
    batch_size: usize,
) -> Result<Vec<FrameText>, OcrError> {
    let bs = batch_size.max(1);
    let mut texts = Vec::with_capacity(changes.len());
    // 当前顺延的 (text, confidence)
    let mut current: Option<(String, f64)> = None;

    for batch in changes.chunks(bs) {
        // 变化帧在此批内的索引 + 路径
        let changed: Vec<usize> = batch
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_changed)
            .map(|(i, _)| i)
            .collect();
        let paths: Vec<String> = changed
            .iter()
            .map(|&i| batch[i].frame.path.to_string_lossy().to_string())
            .collect();

        let results = if paths.is_empty() {
            Vec::new()
        } else {
            provider.recognize_batch(&paths)?
        };

        let mut res_iter = results.into_iter();
        for change in batch.iter() {
            if change.is_changed {
                let r = res_iter
                    .next()
                    .unwrap_or_else(|| crate::ai_runtime::OcrResult {
                        text: String::new(),
                        confidence: 0.0,
                    });
                current = Some((r.text, r.confidence));
            }
            let (text, confidence) = current.clone().unwrap_or_default();
            texts.push(FrameText {
                time: change.frame.time,
                text,
                confidence,
            });
        }
    }
    Ok(texts)
}

/// 把连续相同文本的帧合并成字幕事件。
///
/// - 空文本（含纯空白）是边界：不产生事件，且打断 run
/// - 事件 end = 该段最后一帧时间 + interval，并对 clip 结尾截断
/// - confidence 取引入该段文本的变化帧
pub fn merge_frames(frames: Vec<FrameText>, interval: f64, clip_end: f64) -> Vec<OcrSegment> {
    struct Run {
        start: f64,
        text: String,
        confidence: f64,
        last: f64,
    }

    fn flush(segments: &mut Vec<OcrSegment>, run: &Run, interval: f64, clip_end: f64) {
        let end = (run.last + interval).min(clip_end).max(run.start);
        segments.push(OcrSegment {
            start: run.start,
            end,
            text: run.text.clone(),
            confidence: run.confidence,
        });
    }

    let mut segments = Vec::new();
    let mut run: Option<Run> = None;

    for f in frames {
        // 空文本：flush 当前段并清空
        if f.text.trim().is_empty() {
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            continue;
        }

        let is_same = matches!(&run, Some(r) if r.text == f.text);
        if is_same {
            if let Some(r) = run.as_mut() {
                r.last = f.time;
            }
        } else {
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            run = Some(Run {
                start: f.time,
                text: f.text,
                confidence: f.confidence,
                last: f.time,
            });
        }
    }
    if let Some(r) = run.take() {
        flush(&mut segments, &r, interval, clip_end);
    }
    segments
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::dhash::FrameChange;
    use crate::ai_runtime::{OcrResult, OcrError};
    use crate::video::ExtractedFrame;
    use std::path::PathBuf;

    /// 按调用顺序消费预置结果的 mock provider（跨批次连续取，越界补空）
    struct MockProvider {
        results: Vec<OcrResult>,
        cursor: std::sync::Mutex<usize>,
    }

    impl OcrProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        fn is_ready(&self) -> bool {
            true
        }
        fn recognize_batch(&self, paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
            let mut cursor = self.cursor.lock().unwrap();
            Ok(paths
                .iter()
                .map(|_| {
                    let r = self
                        .results
                        .get(*cursor)
                        .cloned()
                        .unwrap_or_else(|| OcrResult {
                            text: String::new(),
                            confidence: 0.0,
                        });
                    *cursor += 1;
                    r
                })
                .collect())
        }
    }

    fn mock(results: Vec<OcrResult>) -> MockProvider {
        MockProvider {
            results,
            cursor: std::sync::Mutex::new(0),
        }
    }

    fn frame(time: f64, changed: bool) -> FrameChange {
        FrameChange {
            frame: ExtractedFrame {
                path: PathBuf::from("dummy.jpg"),
                time,
            },
            is_changed: changed,
        }
    }

    // ── ocr_pass ──

    #[test]
    fn test_ocr_pass_carries_forward() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: "B".into(), confidence: 0.8 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, false), frame(2.0, true), frame(3.0, false)];
        let texts = ocr_pass(&changes, &provider, 16).unwrap();
        assert_eq!(texts.len(), 4);
        assert_eq!(texts[0].text, "A");
        assert_eq!(texts[1].text, "A"); // 顺延
        assert_eq!(texts[2].text, "B");
        assert_eq!(texts[3].text, "B"); // 顺延
        assert!((texts[0].confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_ocr_pass_empty_change_resets() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: String::new(), confidence: 0.0 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, false)];
        let texts = ocr_pass(&changes, &provider, 1).unwrap();
        assert_eq!(texts[0].text, "A");
        assert_eq!(texts[1].text, ""); // 变化帧识别为空 → 清空
        assert_eq!(texts[2].text, ""); // 顺延空
    }

    #[test]
    fn test_ocr_pass_batch_mixed_changed() {
        let provider = mock(vec![OcrResult { text: "X".into(), confidence: 0.7 }]);
        // 一批 2 帧：一个变化一个未变化，batch 1
        let changes = vec![frame(0.0, false), frame(1.0, true)];
        let texts = ocr_pass(&changes, &provider, 2).unwrap();
        assert_eq!(texts[0].text, ""); // 首帧未变化且无 previous → 空
        assert_eq!(texts[1].text, "X");
    }

    // ── merge_frames ──

    #[test]
    fn test_merge_consecutive_same() {
        let frames = vec![
            FrameText { time: 1.0, text: "A".into(), confidence: 0.9 },
            FrameText { time: 2.0, text: "A".into(), confidence: 0.9 },
            FrameText { time: 3.0, text: "A".into(), confidence: 0.9 },
        ];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 4.0).abs() < 1e-9); // 3 + 1
    }

    #[test]
    fn test_merge_empty_breaks_and_skips() {
        let frames = vec![
            FrameText { time: 0.0, text: String::new(), confidence: 0.0 },
            FrameText { time: 1.0, text: "A".into(), confidence: 0.9 },
            FrameText { time: 2.0, text: "A".into(), confidence: 0.9 },
            FrameText { time: 3.0, text: String::new(), confidence: 0.0 },
        ];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_text_change_split() {
        let frames = vec![
            FrameText { time: 1.0, text: "A".into(), confidence: 0.9 },
            FrameText { time: 2.0, text: "B".into(), confidence: 0.8 },
            FrameText { time: 3.0, text: "B".into(), confidence: 0.8 },
        ];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 2.0).abs() < 1e-9); // A 到 t2 变
        assert!((segs[1].start - 2.0).abs() < 1e-9);
        assert!((segs[1].end - 4.0).abs() < 1e-9); // 3 + 1
    }

    #[test]
    fn test_merge_clamp_to_clip_end() {
        let frames = vec![
            FrameText { time: 4.0, text: "B".into(), confidence: 0.8 },
            FrameText { time: 5.0, text: "B".into(), confidence: 0.8 },
        ];
        let segs = merge_frames(frames, 1.0, 5.5);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 4.0).abs() < 1e-9);
        assert!((segs[0].end - 5.5).abs() < 1e-9); // min(5+1, 5.5)
    }

    #[test]
    fn test_merge_whitespace_treated_empty() {
        let frames = vec![
            FrameText { time: 0.0, text: "  ".into(), confidence: 0.0 },
            FrameText { time: 1.0, text: "A".into(), confidence: 0.9 },
        ];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
    }
}
