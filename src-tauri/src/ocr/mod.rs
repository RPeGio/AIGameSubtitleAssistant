// ─── OCR 字幕生成流水线 ────────────────────────────────────
// 纯函数部分：
//   ocr_pass    —— 只 OCR 变化帧，未变化帧顺延文本，产出每帧的 FrameText
//   merge_frames —— 连续相同文本合并成 OcrSegment（事件）
// 编排命令：
//   run_ocr     —— 串联 抽帧→变化检测→OCR→合并，后台线程跑并上报进度
//
// 依赖：
//   2.3 extract_frames  →  Vec<ExtractedFrame>
//   2.4 detect_changes  →  Vec<FrameChange>
//   2.2 OcrProvider     →  批量识别

use crate::ai_runtime::dhash::FrameChange;
use crate::ai_runtime::{OcrError, OcrManager, OcrResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// OCR 进度事件名（前端 `OCR_PROGRESS_EVENT` 需保持一致）
pub const OCR_PROGRESS_EVENT: &str = "ocr-progress";

/// 合并后的一段字幕事件（不含 id，写轨道时由 store 分配）
#[derive(Debug, Clone, Serialize)]
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
/// - 变化帧的路径按顺序批量交给 `recognize`（每次调用只处理一批），结果按变化帧顺序回填
/// - 传 `recognize` 闭包而非 provider 引用，是为了让调用方把锁的持有范围缩小到单次批量调用
/// - 文本在入口处 `trim` 归一化（空检测与后续合并语义保持一致）
/// - 若返回结果数量与请求不符，视为 worker 错误并终止（不让部分失败悄悄污染字幕）
/// - `on_progress` 每处理一批回调一次：(已处理帧数, 总帧数)
pub fn ocr_pass(
    changes: &[FrameChange],
    mut recognize: impl FnMut(&[String]) -> Result<Vec<OcrResult>, OcrError>,
    batch_size: usize,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Vec<FrameText>, OcrError> {
    let bs = batch_size.max(1);
    let total = changes.len();
    let mut texts = Vec::with_capacity(total);
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
            recognize(&paths)?
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
        on_progress(texts.len(), total);
    }
    Ok(texts)
}

/// 计算两个字符串的归一化编辑距离比例（0=相同，1=完全不同）
fn edit_distance_ratio(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let (m, n) = (ac.len(), bc.len());
    if m == 0 && n == 0 {
        return 0.0;
    }
    let max_len = m.max(n) as f64;
    if max_len == 0.0 {
        return 1.0;
    }
    // 一维 DP 求莱文斯坦距离
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = if ac[i - 1] == bc[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n] as f64 / max_len
}

/// 判断两段文本是否属于同一条字幕的延续
///
/// - 完全相等：是
/// - 前缀互相覆盖（打字机/渐进文本）：是
/// - 编辑距离比例 ≤ 阈值（OCR 抖动/半句）：是
fn similar_text(a: &str, b: &str, threshold: f64) -> bool {
    if a == b {
        return true;
    }
    if a.starts_with(b) || b.starts_with(a) {
        return true;
    }
    edit_distance_ratio(a, b) <= threshold
}

/// 把连续相同（或高度相似）文本的帧合并成字幕事件。
///
/// 契约：`frames` 必须按 time 升序，且文本已 trim 归一化（由 `ocr_pass` 保证）。
///
/// - 空文本是边界：不产生事件，且打断 run
/// - 相似文本（`merge_similarity`）视为同一句的过渡帧：合并、保留最长文本
/// - 事件 end = 该段最后一帧时间 + interval，并对 clip 结尾截断
/// - 整段落在 clip 之外（start >= clip_end）时丢弃，不产生越界时间
/// - confidence 取引入该段文本（或扩展为更长文本）的那次 OCR
pub fn merge_frames(
    frames: Vec<FrameText>,
    interval: f64,
    clip_end: f64,
    merge_similarity: f64,
) -> Vec<OcrSegment> {
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

        let is_same = matches!(&run, Some(r) if similar_text(&r.text, &f.text, merge_similarity));
        if is_same {
            if let Some(r) = run.as_mut() {
                r.last = f.time;
                // 取更完整的文本（更长者胜出）
                if f.text.chars().count() > r.text.chars().count() {
                    r.text = f.text.clone();
                    r.confidence = f.confidence;
                }
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

// ─── 编排命令 ─────────────────────────────────────────────

/// 前端传入的 ocr_region 选区（归一化坐标 + 时间段）
#[derive(Deserialize)]
pub struct OcrRegionInput {
    pub start: f64,
    pub end: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// OCR 运行参数
#[derive(Deserialize)]
pub struct OcrRunParams {
    /// 帧间隔（秒），默认 1.0
    pub frame_interval: f64,
    /// dHash 变化检测阈值，默认 3
    pub dhash_threshold: u32,
    /// OCR 批大小，默认 16
    pub batch_size: usize,
    /// 合并相似度阈值（编辑距离比例，默认 0.3）
    pub merge_similarity: f64,
}

/// 进度事件载荷
#[derive(Clone, Serialize)]
pub struct OcrProgress {
    pub clip_index: usize,
    pub clip_count: usize,
    /// 整体进度 0.0 ~ 1.0
    pub progress: f64,
    pub message: String,
}

/// RAII 临时目录守卫：任何退出路径（含 panic 展开、JoinHandle 错误）都会清理。
/// `keep` 为 true 时保留目录（dev_debug 用于人工检查帧）。
struct TempDirGuard {
    path: PathBuf,
    keep: bool,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// 格式化时间 mm:ss.mmm
fn fmt_time(s: f64) -> String {
    let total_ms = (s * 1000.0).round() as i64;
    let ms = total_ms % 1000;
    let sec = (total_ms / 1000) % 60;
    let min = total_ms / 60000;
    format!("{:02}:{:02}.{:03}", min, sec, ms)
}

/// 串起完整 OCR 流水线（抽帧 → 变化检测 → OCR → 合并）。
///
/// 抽取为独立函数便于集成测试直接调用（不依赖 Tauri 命令栈）。
/// `on_progress(clip_index, clip_count, progress, message)` 每次阶段推进调用一次。
pub fn run_ocr_pipeline<F>(
    manager: &OcrManager,
    video_path: &str,
    video_w: u32,
    video_h: u32,
    region_clips: &[OcrRegionInput],
    params: &OcrRunParams,
    mut on_progress: F,
) -> Result<Vec<OcrSegment>, String>
where
    F: FnMut(usize, usize, f64, String),
{
    let clip_count = region_clips.len();
    let ready = manager.with_provider(|p| p.is_ready());
    if !ready {
        return Err("OCR 运行环境未就绪".into());
    }
    if clip_count == 0 {
        return Err("没有可处理的 OCR 选区".into());
    }

    let dev_debug = manager.config().dev_debug;
    let frame_interval = if params.frame_interval > 0.0 {
        params.frame_interval
    } else {
        1.0
    };
    let dhash_threshold = params.dhash_threshold;
    let batch_size = params.batch_size.max(1);
    let merge_similarity = params.merge_similarity;

    // 临时目录：dev 时写仓库根 temp/ocr/<uuid> 并保留；否则系统临时目录 + 自动清理
    let base_dir = if dev_debug {
        let repo = manager
            .runtime_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir());
        repo.join("temp").join("ocr").join(Uuid::new_v4().to_string())
    } else {
        std::env::temp_dir().join(format!("gsa_ocr_{}", Uuid::new_v4()))
    };
    let _guard = TempDirGuard {
        path: base_dir.clone(),
        keep: dev_debug,
    };
    std::fs::create_dir_all(&base_dir).map_err(|e| format!("无法创建临时目录: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&base_dir, std::fs::Permissions::from_mode(0o700));
    }
    if dev_debug {
        eprintln!("[ocr] 帧输出目录: {}", base_dir.display());
    }

    let mut emit = |clip_index: usize, progress: f64, message: String| {
        on_progress(clip_index, clip_count, progress, message);
    };

    let mut all_segments = Vec::new();
    for (i, clip) in region_clips.iter().enumerate() {
        let clip_dir = base_dir.join(format!("clip_{}", i));
        std::fs::create_dir_all(&clip_dir).map_err(|e| format!("无法创建 clip 目录: {}", e))?;

        emit(
            i,
            i as f64 / clip_count as f64,
            format!("提取帧 {}/{}", i + 1, clip_count),
        );

        let frames = crate::video::extract_frames(
            video_path,
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            video_w,
            video_h,
            frame_interval,
            &clip_dir,
        )
        .map_err(|e| format!("抽帧失败（clip {}）: {}", i, e))?;
        if dev_debug {
            eprintln!(
                "[ocr] clip {}/{}：抽取 {} 帧（间隔 {}s）",
                i + 1,
                clip_count,
                frames.len(),
                frame_interval
            );
        }

        emit(
            i,
            (i as f64 + 0.5) / clip_count as f64,
            format!("变化检测 {}/{}", i + 1, clip_count),
        );

        let changes = crate::ai_runtime::dhash::detect_changes(&frames, dhash_threshold)
            .map_err(|e| format!("变化检测失败（clip {}）: {}", i, e))?;
        let changed_count = changes.iter().filter(|c| c.is_changed).count();
        if dev_debug {
            eprintln!(
                "[ocr] clip {}/{}：变化 {} / {} 帧 → OCR",
                i + 1,
                clip_count,
                changed_count,
                changes.len()
            );
        }

        emit(
            i,
            (i as f64 + 0.6) / clip_count as f64,
            format!("OCR 识别 {}/{}", i + 1, clip_count),
        );

        let total_frames = changes.len();
        let texts = ocr_pass(
            &changes,
            |paths| manager.with_provider(|p| p.recognize_batch(paths)),
            batch_size,
            |done, total| {
                let overall = (i as f64 + done as f64 / total.max(1) as f64) / clip_count as f64;
                emit(
                    i,
                    overall,
                    format!("OCR 识别 {}/{}（{}/{} 帧）", i + 1, clip_count, done, total_frames),
                );
            },
        )
        .map_err(|e| format!("OCR 失败（clip {}）: {}", i, e))?;

        if dev_debug {
            // 打印每个变化帧的 OCR 结果
            for (change, ft) in changes.iter().zip(texts.iter()) {
                if change.is_changed && !ft.text.is_empty() {
                    eprintln!(
                        "[ocr]   {} conf={:.2} \"{}\"",
                        fmt_time(ft.time),
                        ft.confidence,
                        ft.text
                    );
                }
            }
        }

        let segments = merge_frames(texts, frame_interval, clip.end, merge_similarity);
        if dev_debug {
            eprintln!("[ocr] clip {}/{}：合并 {} 条事件", i + 1, clip_count, segments.len());
            for seg in &segments {
                eprintln!(
                    "[ocr]   事件 [{} → {}] \"{}\"",
                    fmt_time(seg.start),
                    fmt_time(seg.end),
                    seg.text
                );
            }
        }
        all_segments.extend(segments);
    }

    Ok(all_segments)
}

/// Tauri 命令：串联完整 OCR 流水线。
///
/// 在后台线程跑（不阻塞 UI），过程中通过 `ocr-progress` 事件上报进度，
/// 返回按时间排序的 OcrSegment 列表，由前端写入 ocr_text 轨道。
#[tauri::command]
pub async fn run_ocr(
    app: AppHandle,
    video_path: String,
    video_w: u32,
    video_h: u32,
    region_clips: Vec<OcrRegionInput>,
    params: OcrRunParams,
) -> Result<Vec<OcrSegment>, String> {
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<OcrManager>();
        run_ocr_pipeline(
            &manager,
            &video_path,
            video_w,
            video_h,
            &region_clips,
            &params,
            |clip_index, clip_count, progress, message| {
                let _ = app_handle.emit(
                    OCR_PROGRESS_EVENT,
                    OcrProgress {
                        clip_index,
                        clip_count,
                        progress,
                        message,
                    },
                );
            },
        )
    })
    .await
    .map_err(|e| format!("OCR 任务内部错误: {}", e))?
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::dhash::FrameChange;
    use crate::ai_runtime::{OcrProvider, OcrResult, OcrError};
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
        let texts = ocr_pass(&changes, |paths| provider.recognize_batch(paths), 16, |_, _| {}).unwrap();
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
        let texts = ocr_pass(&changes, |paths| provider.recognize_batch(paths), 16, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A"); // 已 trim
    }

    #[test]
    fn test_ocr_pass_empty_change_resets() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: String::new(), confidence: 0.0 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, false)];
        let texts = ocr_pass(&changes, |paths| provider.recognize_batch(paths), 1, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[1].text.as_ref(), ""); // 变化帧识别为空 → 清空
        assert_eq!(texts[2].text.as_ref(), ""); // 顺延空
    }

    #[test]
    fn test_ocr_pass_batch_mixed_changed() {
        let provider = mock(vec![OcrResult { text: "X".into(), confidence: 0.7 }]);
        let changes = vec![frame(0.0, false), frame(1.0, true)];
        let texts = ocr_pass(&changes, |paths| provider.recognize_batch(paths), 2, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), ""); // 首帧未变化且无 previous → 空
        assert_eq!(texts[1].text.as_ref(), "X");
    }

    #[test]
    fn test_ocr_pass_result_mismatch_errors() {
        // 预置结果比变化帧少 → 应返回错误而非静默空文本
        let provider = mock(vec![OcrResult { text: "A".into(), confidence: 0.9 }]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, true)];
        assert!(ocr_pass(&changes, |paths| provider.recognize_batch(paths), 1, |_, _| {}).is_err());
    }

    // ── merge_frames ──

    #[test]
    fn test_merge_consecutive_same() {
        let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "A", 0.9), ft(3.0, "A", 0.9)];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
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
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_text_change_split() {
        let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "B", 0.8), ft(3.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 2.0).abs() < 1e-9); // A 到 t2 变
        assert!((segs[1].start - 2.0).abs() < 1e-9);
        assert!((segs[1].end - 4.0).abs() < 1e-9); // 3 + 1
    }

    #[test]
    fn test_merge_clamp_to_clip_end() {
        let frames = vec![ft(4.0, "B", 0.8), ft(5.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 5.5, 0.3);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 4.0).abs() < 1e-9);
        assert!((segs[0].end - 5.5).abs() < 1e-9); // min(5+1, 5.5)
    }

    #[test]
    fn test_merge_skips_run_beyond_clip() {
        // 段起始已越过 clip_end → 丢弃
        let frames = vec![ft(6.0, "B", 0.8), ft(7.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 5.5, 0.3);
        assert!(segs.is_empty());
    }

    // ── 相似度合并 ──

    #[test]
    fn test_edit_distance_ratio() {
        assert!((edit_distance_ratio("", "") - 0.0).abs() < 1e-9);
        assert!((edit_distance_ratio("abc", "abc") - 0.0).abs() < 1e-9);
        assert!((edit_distance_ratio("abc", "abd") - 1.0 / 3.0).abs() < 1e-9);
        assert!((edit_distance_ratio("abc", "xyz") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_similar_prefix_extends() {
        // 打字机/渐进文本：后续帧是前缀的超集 → 合并为一条，保留最长文本
        let frames = vec![
            ft(1.0, "旅行者，你来了", 0.9),
            ft(2.0, "旅行者，你来了。前方似乎有东西在等待。", 0.9),
            ft(3.0, "旅行者，你来了。前方似乎有东西在等待。", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "旅行者，你来了。前方似乎有东西在等待。");
        assert!((segs[0].end - 4.0).abs() < 1e-9); // 3 + 1
    }

    #[test]
    fn test_merge_similar_noise_keeps_longest() {
        // OCR 抖动：同一句少一字 → 相似合并，保留更长
        let frames = vec![
            ft(1.0, "前方似乎有什么东西在等待", 0.9),
            ft(2.0, "前方似乎有什么东西在等待。", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "前方似乎有什么东西在等待。");
    }

    #[test]
    fn test_merge_dissimilar_splits() {
        // 完全不同 → 分两段
        let frames = vec![ft(1.0, "甲", 0.9), ft(2.0, "乙", 0.8)];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 2);
    }

    // ── 集成实测（t3 项目，需本地环境；默认跳过）──

    #[test]
    #[ignore]
    fn test_t3_end_to_end() {
        use crate::ai_runtime::OcrManager;
        use crate::ai_runtime::config::RuntimeConfig;
        use std::path::Path;
        use std::time::Instant;

        let video = r"F:\RPeGio\Rust_Project\AIGameSubtitleAssistant\tests\test(hi-res).mp4";
        let runtime_dir = Path::new(r"F:\RPeGio\Rust_Project\AIGameSubtitleAssistant\runtime");
        let config = RuntimeConfig::load(runtime_dir).expect("读取 runtime 配置失败");
        let manager = OcrManager::new(config, runtime_dir.to_path_buf());

        let ready = manager.with_provider(|p| p.is_ready());
        assert!(ready, "OCR 环境未就绪");

        let meta = crate::video::get_video_metadata(video.to_string()).expect("读取视频元数据失败");
        assert_eq!(meta.width, 1920);

        // 来自 t3 项目 project.json 的 4 段选区
        let clips = vec![
            OcrRegionInput { start: 0.0, end: 36.373, x1: 0.2, y1: 0.7, x2: 0.8, y2: 0.9 },
            OcrRegionInput { start: 36.373, end: 40.798, x1: 0.285, y1: 0.397, x2: 0.716, y2: 0.567 },
            OcrRegionInput { start: 40.798, end: 153.263, x1: 0.131, y1: 0.782, x2: 0.877, y2: 0.942 },
            OcrRegionInput { start: 153.263, end: 369.983, x1: 0.353, y1: 0.404, x2: 0.662, y2: 0.548 },
        ];
        let params = OcrRunParams {
            frame_interval: 1.0,
            dhash_threshold: 3,
            batch_size: 16,
            merge_similarity: 0.3,
        };

        let start = Instant::now();
        let segments = run_ocr_pipeline(
            &manager,
            video,
            meta.width,
            meta.height,
            &clips,
            &params,
            |_, _, _, _| {},
        )
        .expect("OCR 流水线失败");
        let elapsed = start.elapsed();

        println!("\n========== t3 实测结果 ==========");
        println!(
            "总耗时: {:.1}s（{:.2}min）  事件数: {}",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() / 60.0,
            segments.len()
        );
        for s in segments.iter().take(10) {
            println!("  [{} → {}] \"{}\"", fmt_time(s.start), fmt_time(s.end), s.text);
        }
        if segments.len() > 10 {
            println!("  ... 共 {} 条", segments.len());
        }
        println!("=================================");
    }
}
