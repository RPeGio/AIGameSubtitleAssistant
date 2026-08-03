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
use std::sync::Arc;

/// 合并后的一段字幕事件（不含 id，写轨道时由 store 分配）
#[derive(Debug, Clone)]
pub struct OcrSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f64,
}

/// 一帧的文本（顺延后的完整序列）。
/// text 用 `Arc<str>` 共享，未变化帧只做引用计数递增，不逐帧拷贝字符串。
#[derive(Debug, Clone)]
pub struct FrameText {
    pub time: f64,
    pub text: Arc<str>,
    pub confidence: f64,
}

/// 只 OCR 变化帧，未变化帧顺延上一次 OCR 的文本。
///
/// 契约：`changes` 必须按帧时间升序。
///
/// - 变化帧的路径按顺序批量送去 `recognize_batch`，结果按变化帧顺序回填
/// - 文本在入口处 `trim` 归一化（空检测与后续合并语义保持一致）
/// - 若返回结果数量与请求不符，视为 worker 错误并终止（不让部分失败悄悄污染字幕）
pub fn ocr_pass(
    changes: &[FrameChange],
    provider: &dyn OcrProvider,
    batch_size: usize,
) -> Result<Vec<FrameText>, OcrError> {
    let bs = batch_size.max(1);
    let mut texts = Vec::with_capacity(changes.len());
    // 当前顺延的 (text, confidence)；Arc 共享避免未变化帧反复分配
    let mut current: Option<(Arc<str>, f64)> = None;

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

        // 数量校验：短结果不能静默吞掉，避免顺延状态被污染
        if results.len() != changed.len() {
            return Err(OcrError::Worker(format!(
                "OCR 结果数量不匹配：请求 {} 帧，返回 {} 帧",
                changed.len(),
                results.len()
            )));
        }

        let mut res_iter = results.into_iter();
        for change in batch.iter() {
            if change.is_changed {
                let r = res_iter
                    .next()
                    .ok_or_else(|| OcrError::Worker("OCR 结果缺失".into()))?;
                // trim 归一化：保证空检测与合并时的一致性
                current = Some((Arc::from(r.text.trim()), r.confidence));
            }
            let (text, confidence) = match &current {
                Some((t, c)) => (Arc::clone(t), *c),
                None => (Arc::from(""), 0.0),
            };
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
/// 契约：`frames` 必须按 time 升序，且文本已 trim 归一化（由 `ocr_pass` 保证）。
///
/// - 空文本是边界：不产生事件，且打断 run
/// - 事件 end = 该段最后一帧时间 + interval，并对 clip 结尾截断
/// - 整段落在 clip 之外（start >= clip_end）时丢弃，不产生越界时间
/// - confidence 取引入该段文本的变化帧
pub fn merge_frames(frames: Vec<FrameText>, interval: f64, clip_end: f64) -> Vec<OcrSegment> {
    struct Run {
        start: f64,
        text: Arc<str>,
        confidence: f64,
        last: f64,
    }

    fn flush(segments: &mut Vec<OcrSegment>, run: &Run, interval: f64, clip_end: f64) {
        // 整段在 clip 之外：丢弃，避免零长度/越界时间
        if run.start >= clip_end {
            return;
        }
        let end = (run.last + interval).min(clip_end).max(run.start);
        segments.push(OcrSegment {
            start: run.start,
            end,
            text: run.text.to_string(),
            confidence: run.confidence,
        });
    }

    let mut segments = Vec::new();
    let mut run: Option<Run> = None;

    for f in frames {
        if f.text.is_empty() {
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            continue;
        }

        let is_same = matches!(&run, Some(r) if r.text.as_ref() == f.text.as_ref());
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

    /// 按调用顺序消费预置结果的 mock provider。
    /// 结果耗尽后返回"短向量"（比请求少），用于触发数量不匹配的错误路径。
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
            let remaining = self.results.get(*cursor..).unwrap_or(&[]);
            let count = remaining.len().min(paths.len());
            let out = remaining[..count].to_vec();
            *cursor += count;
            Ok(out)
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

    fn ft(time: f64, text: &str, confidence: f64) -> FrameText {
        FrameText {
            time,
            text: Arc::from(text),
            confidence,
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
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[1].text.as_ref(), "A"); // 顺延
        assert_eq!(texts[2].text.as_ref(), "B");
        assert_eq!(texts[3].text.as_ref(), "B"); // 顺延
        assert!((texts[0].confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_ocr_pass_trims_text() {
        let provider = mock(vec![OcrResult { text: "  A  ".into(), confidence: 0.9 }]);
        let changes = vec![frame(0.0, true)];
        let texts = ocr_pass(&changes, &provider, 16).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A"); // 已 trim
    }

    #[test]
    fn test_ocr_pass_empty_change_resets() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: String::new(), confidence: 0.0 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, false)];
        let texts = ocr_pass(&changes, &provider, 1).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[1].text.as_ref(), ""); // 变化帧识别为空 → 清空
        assert_eq!(texts[2].text.as_ref(), ""); // 顺延空
    }

    #[test]
    fn test_ocr_pass_batch_mixed_changed() {
        let provider = mock(vec![OcrResult { text: "X".into(), confidence: 0.7 }]);
        let changes = vec![frame(0.0, false), frame(1.0, true)];
        let texts = ocr_pass(&changes, &provider, 2).unwrap();
        assert_eq!(texts[0].text.as_ref(), ""); // 首帧未变化且无 previous → 空
        assert_eq!(texts[1].text.as_ref(), "X");
    }

    #[test]
    fn test_ocr_pass_result_mismatch_errors() {
        // 预置结果比变化帧少 → 应返回错误而非静默空文本
        let provider = mock(vec![OcrResult { text: "A".into(), confidence: 0.9 }]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, true)];
        assert!(ocr_pass(&changes, &provider, 1).is_err());
    }

    // ── merge_frames ──

    #[test]
    fn test_merge_consecutive_same() {
        let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "A", 0.9), ft(3.0, "A", 0.9)];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 4.0).abs() < 1e-9); // 3 + 1
        assert_eq!(segs[0].text, "A");
    }

    #[test]
    fn test_merge_empty_breaks_and_skips() {
        let frames = vec![
            ft(0.0, "", 0.0),
            ft(1.0, "A", 0.9),
            ft(2.0, "A", 0.9),
            ft(3.0, "", 0.0),
        ];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_text_change_split() {
        let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "B", 0.8), ft(3.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 10.0);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 2.0).abs() < 1e-9); // A 到 t2 变
        assert!((segs[1].start - 2.0).abs() < 1e-9);
        assert!((segs[1].end - 4.0).abs() < 1e-9); // 3 + 1
    }

    #[test]
    fn test_merge_clamp_to_clip_end() {
        let frames = vec![ft(4.0, "B", 0.8), ft(5.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 5.5);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 4.0).abs() < 1e-9);
        assert!((segs[0].end - 5.5).abs() < 1e-9); // min(5+1, 5.5)
    }

    #[test]
    fn test_merge_skips_run_beyond_clip() {
        // 段起始已越过 clip_end → 丢弃
        let frames = vec![ft(6.0, "B", 0.8), ft(7.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 5.5);
        assert!(segs.is_empty());
    }
}
