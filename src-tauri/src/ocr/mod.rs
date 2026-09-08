// ─── OCR 字幕生成流水线 ────────────────────────────────────
// 纯函数部分：
//   ocr_pass    —— 只 OCR 变化帧，未变化帧顺延文本，产出每帧的 FrameText
//   merge_frames —— 连续相同文本合并成 OcrSegment（事件）
// 编排命令：
//   run_ocr     —— 串联 扫描→抽帧→窗口精化→OCR→合并，后台线程跑并上报进度
//
// 依赖：
//   video::scan_frame_hashes     → Vec<(time, dhash)>（零落盘变化检测，网格+精化共用）
//   video::extract_frames_bytes  → mjpeg 管道内存帧（仅变化帧保留字节，零落盘）
//   2.4 change_flags             → Vec<(is_changed, base_hash)>
//   2.2 OcrProvider              → 批量识别（images 元素 = base64 JPEG，IPC 不经磁盘）

use crate::ai_runtime::dhash::FrameChange;
use crate::ai_runtime::{OcrError, OcrManager, OcrResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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

/// JPEG 字节 → base64（OCR IPC 的图像载荷格式）
fn b64(data: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD as B64_ENGINE;
    use base64::Engine as _;
    B64_ENGINE.encode(data)
}

/// 只 OCR 变化帧，未变化帧顺延上一次 OCR 的文本。
///
/// 契约：`changes` 必须按帧时间升序；`jpegs` 与 `changes` 中 is_changed 帧
/// 按出现顺序一一对齐。
///
/// - 变化帧的 JPEG 字节 base64 编码后按顺序批量交给 `recognize`（每次调用只处理一批），
///   worker 在内存中解码为 ndarray，全程不经磁盘
/// - 传 `recognize` 闭包而非 provider 引用，是为了让调用方把锁的持有范围缩小到单次批量调用
/// - 文本在入口处 `trim` 归一化（空检测与后续合并语义保持一致）
/// - 若返回结果数量与请求不符，视为 worker 错误并终止（不让部分失败悄悄污染字幕）
/// - `on_progress` 每处理一批回调一次：(已处理帧数, 总帧数)
pub fn ocr_pass(
    changes: &[FrameChange],
    jpegs: &[Vec<u8>],
    mut recognize: impl FnMut(&[String]) -> Result<Vec<OcrResult>, OcrError>,
    batch_size: usize,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Vec<FrameText>, OcrError> {
    let bs = batch_size.max(1);
    let total = changes.len();
    let changed_total = changes.iter().filter(|c| c.is_changed).count();
    if jpegs.len() != changed_total {
        return Err(OcrError::Worker(format!(
            "图像数据与变化帧数量不符：{} 变化帧，{} 张 JPEG",
            changed_total,
            jpegs.len()
        )));
    }
    let mut jpeg_iter = jpegs.iter();
    let mut texts = Vec::with_capacity(total);
    // 当前顺延的 (text, confidence)；Arc 共享避免未变化帧反复分配
    let mut current: Option<(Arc<str>, f64)> = None;

    for batch in changes.chunks(bs) {
        // 变化帧在此批内的索引
        let changed: Vec<usize> = batch
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_changed)
            .map(|(i, _)| i)
            .collect();
        let images: Vec<String> = changed
            .iter()
            .map(|_| {
                let jpeg = jpeg_iter
                    .next()
                    .ok_or_else(|| OcrError::Worker("图像数据少于变化帧".into()))?;
                Ok(b64(jpeg))
            })
            .collect::<Result<Vec<String>, OcrError>>()?;

        let results = if images.is_empty() {
            Vec::new()
        } else {
            recognize(&images)?
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
                time: change.time,
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
    // 前缀互相覆盖（打字机/渐进文本）：仅当较短文本足够长（≥ 较长文本 1/3）时，
    // 避免把"派蒙"与"派蒙：旅行者你来了"这类独立两行误合并（约占 1/4 的前缀）
    let (short, long) = if a.chars().count() <= b.chars().count() {
        (a, b)
    } else {
        (b, a)
    };
    let short_len = short.chars().count();
    if short_len > 0 && long.starts_with(short) && short_len * 3 >= long.chars().count() {
        return true;
    }
    edit_distance_ratio(a, b) <= threshold
}

/// 从 run 内所有相似帧文本做多数投票：选与其它帧总相似度最高者。
///
/// 替代"更长者胜出"：被噪声污染的更长文本与多数帧差异大，不会被选中；
/// 完全相同的候选平局时取更长（与旧行为兼容）。
fn vote_text(texts: &[(Arc<str>, f64)]) -> (Arc<str>, f64) {
    let mut best: &(Arc<str>, f64) = &texts[0];
    let mut best_score = f64::MIN;
    for cand in texts {
        let score: f64 = texts
            .iter()
            .map(|(t, _)| 1.0 - edit_distance_ratio(&cand.0, t))
            .sum();
        let is_longer = cand.0.chars().count() > best.0.chars().count();
        if score > best_score + 1e-9 || ((score - best_score).abs() <= 1e-9 && is_longer) {
            best_score = score;
            best = cand;
        }
    }
    (best.0.clone(), best.1)
}

/// 把连续相同（或高度相似）文本的帧合并成字幕事件。
///
/// 契约：`frames` 必须按 time 升序，且文本已 trim 归一化（由 `ocr_pass` 保证）。
///
/// - 空文本是边界：不产生事件，且打断 run（连续空帧短于容错窗口时视为抖动不打断）
/// - 相似文本（`merge_similarity`）视为同一句的过渡帧：合并，flush 时多数投票取最终文本
/// - 事件 end = 该段最后一帧时间 + interval，并对 clip 结尾截断
/// - 整段落在 clip 之外（start >= clip_end）时丢弃，不产生越界时间
/// - confidence 取投票胜出文本对应帧的置信度
pub fn merge_frames(
    frames: Vec<FrameText>,
    interval: f64,
    clip_end: f64,
    merge_similarity: f64,
) -> Vec<OcrSegment> {
    // 空帧容错窗口：连续空帧短于此值视为 OCR 抖动，不打断 run
    let empty_gap = (interval * 1.5).max(0.8);

    struct Run {
        start: f64,
        /// run 内所有相似帧的 (文本, 置信度)；flush 时多数投票取最终文本
        texts: Vec<(Arc<str>, f64)>,
        last: f64,
        /// 当前连续空帧的起点时间（容错窗口内不打断 run）
        empty_since: Option<f64>,
    }

    fn flush(segments: &mut Vec<OcrSegment>, run: &Run, interval: f64, clip_end: f64) {
        // 整段在 clip 之外：丢弃，避免零长度/越界时间
        if run.start >= clip_end {
            return;
        }
        let end = (run.last + interval).min(clip_end).max(run.start);
        let (text, confidence) = vote_text(&run.texts);
        segments.push(OcrSegment {
            start: run.start,
            end,
            text: text.to_string(),
            confidence,
        });
    }

    let mut segments = Vec::new();
    let mut run: Option<Run> = None;

    for f in frames {
        if f.text.is_empty() {
            // 空帧容错：累计空帧时长 < empty_gap 时视为 OCR 抖动，run 继续
            if let Some(r) = run.as_mut() {
                let es = *r.empty_since.get_or_insert(f.time);
                if f.time - es < empty_gap {
                    continue;
                }
            }
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            continue;
        }

        // 相似基准 = run 内最近一帧文本（渐进时是超集，比较稳定）
        let is_same = matches!(&run, Some(r) if {
            let base: &str = r.texts.last().map(|(t, _)| t.as_ref()).unwrap_or("");
            similar_text(base, f.text.as_ref(), merge_similarity)
        });
        if is_same {
            if let Some(r) = run.as_mut() {
                r.last = f.time;
                r.empty_since = None;
                r.texts.push((f.text, f.confidence));
            }
        } else {
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            run = Some(Run {
                start: f.time,
                texts: vec![(f.text, f.confidence)],
                last: f.time,
                empty_since: None,
            });
        }
    }
    if let Some(r) = run.take() {
        flush(&mut segments, &r, interval, clip_end);
    }
    segments
}

/// 归一化：仅留 Unicode 字母数字（含 CJK），剔除空白与标点 —— 用于子序列近似匹配
fn norm_chars(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// 判断 a 的字符是否按序出现在 b 中（近似前缀/渐进片段匹配，双指针）
fn is_subsequence(a: &[char], b: &[char]) -> bool {
    let mut j = 0;
    for &c in a {
        while j < b.len() && b[j] != c {
            j += 1;
        }
        if j >= b.len() {
            return false;
        }
        j += 1;
    }
    true
}

/// 孤立短段并入相邻段：短段（≤ 2×interval）文本归一化后是相邻长段的子序列，
/// 说明它是该句的渐进中间态/残缺帧 → 并入（取完整文本，时间合并）。
/// 一遍正向扫描即可同时处理"短段在前"与"短段在后"两种方向。
pub fn merge_fragments(segments: Vec<OcrSegment>, interval: f64) -> Vec<OcrSegment> {
    let mut out: Vec<OcrSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = out.last_mut() {
            let last_short = last.end - last.start <= interval * 2.0;
            let seg_short = seg.end - seg.start <= interval * 2.0;
            let ln = norm_chars(&last.text);
            let sn = norm_chars(&seg.text);
            if !ln.is_empty() && !sn.is_empty() {
                if last_short && ln.len() < sn.len() && is_subsequence(&ln, &sn) {
                    // 前段是后段的渐进片段 → 前段并入后段（取完整文本）
                    last.text = seg.text;
                    last.confidence = seg.confidence;
                    last.end = seg.end;
                    continue;
                }
                if seg_short && sn.len() < ln.len() && is_subsequence(&sn, &ln) {
                    // 后段是前段的渐进片段 → 后段并入前段（文本不变，时间合并）
                    last.end = seg.end;
                    continue;
                }
            }
        }
        out.push(seg);
    }
    out
}

/// 修正相邻段时间重叠：段 `end` 不得超过下一段 `start`。
///
/// 阶段 1 的窗口精化会把 changed 帧 `start` 提前到帧级边界，而 `end` 仍按
/// 网格 `last + interval` 计算，可能产生交叉/重叠 → 统一 clamp 保证时间单调。
fn clamp_segment_times(mut segments: Vec<OcrSegment>) -> Vec<OcrSegment> {
    for i in 1..segments.len() {
        if segments[i - 1].end > segments[i].start {
            segments[i - 1].end = segments[i].start;
        }
    }
    segments
}

/// 相邻段文本相似（相等/前缀/编辑距离 ≤ 阈值）→ 合并为一段（取更长文本、时间取并集）。
///
/// 用于阶段 2 短字幕召回后：dHash 判为变化但 OCR 文本与相邻段相同的"伪短字幕"
/// （字幕视觉抖动/OCR 抖动）会与相邻段相似，在此合并，避免同一句被拆成碎片。
fn merge_similar_adjacent(segments: Vec<OcrSegment>, threshold: f64) -> Vec<OcrSegment> {
    let mut out: Vec<OcrSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = out.last_mut() {
            if similar_text(&last.text, &seg.text, threshold) {
                last.end = last.end.max(seg.end);
                // 更长且置信度不低于原段才替换文本，避免用低置信度长文本覆盖清晰短文本
                if seg.text.chars().count() > last.text.chars().count()
                    && seg.confidence >= last.confidence
                {
                    last.text = seg.text;
                    last.confidence = seg.confidence;
                }
                continue;
            }
        }
        out.push(seg);
    }
    out
}

/// 对每个 changed 帧做窗口精化：从零落盘扫描序列中切出窗口密帧哈希，
/// 逐帧相对基准 A 判定，计算帧级主边界（阶段 1）+ 召回窗口内短字幕（阶段 2）。
///
/// 返回 `(精化后的 changes, 召回的短字幕段)`。
/// 单独成函数便于集成测试/基准直接调用，不依赖 Tauri 命令栈。
///
/// 性能策略：哈希全部来自扫描阶段的内存序列（`scan_frame_hashes`），
/// 本函数零抽帧零落盘；仅阶段 2 召回时按需抽取一张中间帧 JPEG
/// （OCR worker 消费文件路径，每窗口至多一次）。
#[allow(clippy::too_many_arguments)]
pub fn refine_window_changes(
    changes: &[crate::ai_runtime::dhash::FrameChange],
    scan_stream: &[(f64, u64)],
    video_path: &str,
    clip: &OcrRegionInput,
    video_w: u32,
    video_h: u32,
    src_fps: f64,
    dhash_threshold: u32,
    recall_dir: &Path,
    manager: &crate::ai_runtime::OcrManager,
    dev_debug: bool,
) -> (Vec<crate::ai_runtime::dhash::FrameChange>, Vec<OcrSegment>) {
    let dense_interval = 1.0 / src_fps;
    let mut short_segments: Vec<OcrSegment> = Vec::new();
    let mut refined: Vec<crate::ai_runtime::dhash::FrameChange> =
        Vec::with_capacity(changes.len());

    for idx in 0..changes.len() {
        let mut fc = changes[idx].clone();
        if fc.is_changed && idx > 0 {
            if let Some(base) = fc.base_hash {
                let lo = changes[idx - 1].time;
                let hi = fc.time;
                if hi > lo {
                    // 窗口 = 扫描序列的时间切片（与旧"整段密帧落盘后切片"同一帧集合，
                    // 哈希同源：网格哈希也取自该序列，交叉比较基准一致）
                    let window: Vec<(f64, u64)> = scan_stream
                        .iter()
                        .filter(|(t, _)| *t >= lo && *t < hi)
                        .copied()
                        .collect();
                    let hashes: Vec<u64> = window.iter().map(|(_, h)| *h).collect();
                    let boundaries =
                        crate::ai_runtime::dhash::boundary_indices(&hashes, base, dhash_threshold);
                    // 下一段（f_k）的真实边界 = 最后一个变化帧
                    if let Some(&last) = boundaries.last() {
                        // 用窗口内实际帧时间（而非 lo + last*interval），窗口首帧
                        // 未必恰在 lo，更精确也避免假设
                        let new_time = window[last].0;
                        if new_time < hi {
                            if dev_debug {
                                eprintln!(
                                    "[ocr]   精化边界 [{} → {}]",
                                    fmt_time(fc.time),
                                    fmt_time(new_time)
                                );
                            }
                            fc.time = new_time;
                        }
                        // 阶段 2：恰好两次变化 → [main, sub[0]) 为短字幕候选。
                        // 注意：只召回恰好 2 个边界的窗口（A→短字幕→C）。遍历全部边界对
                        // 会过度召回（每次召回一次慢速 OCR IPC，实测事件数翻倍、耗时×3），
                        // 故不做泛化，多突变/多短字幕留给后续更精确的判定。
                        if boundaries.len() == 2 {
                            let (m, s) = (boundaries[0], boundaries[1]);
                            if s > m + 1 {
                                // 区间中点帧做 OCR（避开切换过渡帧）；
                                // 单帧 mjpeg 管道取内存字节 → base64，不落盘
                                let mid = m + (s - m) / 2;
                                if let Some(&(mid_time, _)) = window.get(mid) {
                                    if let Ok(jpeg) = crate::video::extract_single_frame_bytes(
                                        video_path,
                                        mid_time,
                                        clip.x1,
                                        clip.y1,
                                        clip.x2,
                                        clip.y2,
                                        video_w,
                                        video_h,
                                    ) {
                                        if dev_debug {
                                            // dev 模式留一份召回帧供人工检查
                                            let dump = recall_dir
                                                .join(format!("recall_{idx}.jpg"));
                                            let _ = std::fs::write(dump, &jpeg);
                                        }
                                        let image = b64(&jpeg);
                                        if let Ok(res) = manager.with_provider(|p| {
                                            p.recognize_batch(std::slice::from_ref(&image))
                                        }) {
                                            if let Some(r) = res.into_iter().next() {
                                                let text = r.text.trim().to_string();
                                                if !text.is_empty() {
                                                    short_segments.push(OcrSegment {
                                                        start: lo
                                                            + m as f64 * dense_interval,
                                                        end: lo
                                                            + s as f64 * dense_interval,
                                                        confidence: r.confidence,
                                                        text,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        refined.push(fc);
    }
    (refined, short_segments)
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
/// 秒 → "MM:SS.mmm"（集成测试打印结果用）
pub fn fmt_time(s: f64) -> String {
    let total_ms = (s * 1000.0).round() as i64;
    let ms = total_ms % 1000;
    let sec = (total_ms / 1000) % 60;
    let min = total_ms / 60000;
    format!("{:02}:{:02}.{:03}", min, sec, ms)
}

/// 串起完整 OCR 流水线（抽帧 → 变化检测 → 窗口精化 → OCR → 合并）。
///
/// 抽取为独立函数便于集成测试直接调用（不依赖 Tauri 命令栈）。
/// `src_fps`：源视频帧率，>0 时启用切换窗口的帧级主边界精化（阶段 1）。
/// `on_progress(clip_index, clip_count, progress, message)` 每次阶段推进调用一次。
#[allow(clippy::too_many_arguments)] // 参数为流水线依赖的显式事实，不聚合为结构体以保持可测性
pub fn run_ocr_pipeline<F>(
    manager: &OcrManager,
    video_path: &str,
    video_w: u32,
    video_h: u32,
    region_clips: &[OcrRegionInput],
    params: &OcrRunParams,
    src_fps: f64,
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
        let clip_started = std::time::Instant::now();
        let clip_dir = base_dir.join(format!("clip_{}", i));
        std::fs::create_dir_all(&clip_dir).map_err(|e| format!("无法创建 clip 目录: {}", e))?;

        // ── 零落盘扫描：一次得到全 clip 的 dHash 序列 ──
        // 网格变化检测与窗口精化共用同一序列（哈希同源，交叉比较基准才一致）。
        // 扫描密度取源帧率（精化需要帧级时间）；源帧率未知时退化为网格密度。
        let scan_interval = if src_fps > 0.0 {
            (1.0 / src_fps).min(frame_interval)
        } else {
            frame_interval
        };
        emit(
            i,
            i as f64 / clip_count as f64,
            format!("扫描变化 {}/{}", i + 1, clip_count),
        );
        let scan_started = std::time::Instant::now();
        let scan_stream = crate::video::scan_frame_hashes(
            video_path,
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            video_w,
            video_h,
            scan_interval,
        )
        .map_err(|e| format!("扫描失败（clip {}）: {}", i, e))?;
        let scan_dt = scan_started.elapsed();

        // ── 网格变化标志：网格帧哈希 = 扫描序列中时间最近帧 ──
        // （网格与精化窗口的哈希同源，boundary_indices 交叉比较基准才一致）
        let last_scan = scan_stream.len().saturating_sub(1);
        let grid_frame_count = ((clip.end - clip.start) / frame_interval).ceil() as usize;
        let grid_hashes: Vec<u64> = (0..grid_frame_count)
            .map(|k| {
                let idx =
                    ((k as f64 * frame_interval / scan_interval).round() as usize).min(last_scan);
                scan_stream[idx].1
            })
            .collect();
        let flags = crate::ai_runtime::dhash::change_flags(&grid_hashes, dhash_threshold);
        // mjpeg 抽帧只为变化帧保留字节；未变化帧在流中读到即丢（零落盘）
        let keep: Vec<usize> = flags
            .iter()
            .enumerate()
            .filter(|(_, (c, _))| *c)
            .map(|(k, _)| k)
            .collect();

        // ── 网格抽帧（mjpeg 管道 → 内存字节，OCR IPC 载荷）──
        emit(
            i,
            (i as f64 + 0.4) / clip_count as f64,
            format!("提取帧 {}/{}", i + 1, clip_count),
        );
        let extract_started = std::time::Instant::now();
        let grid = crate::video::extract_frames_bytes(
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
            &keep,
        )
        .map_err(|e| format!("抽帧失败（clip {}）: {}", i, e))?;
        let extract_dt = extract_started.elapsed();
        if dev_debug {
            // dev 模式把保留的帧写盘供人工检查（生产模式全程零落盘）
            for fb in &grid.kept {
                let dump = clip_dir.join(format!("frame_{:05}.jpg", fb.index + 1));
                let _ = std::fs::write(dump, &fb.jpeg);
            }
            eprintln!(
                "[ocr] clip {}/{}：扫描 {} 帧，网格 {} 帧，变化 {} 帧 → OCR",
                i + 1,
                clip_count,
                scan_stream.len(),
                grid.total,
                grid.kept.len()
            );
        }

        let changes: Vec<FrameChange> = (0..grid.total)
            .map(|k| {
                let (is_changed, base_hash) = flags.get(k).copied().unwrap_or((false, None));
                FrameChange {
                    time: clip.start + (k as f64) * frame_interval,
                    is_changed,
                    base_hash,
                }
            })
            .collect();
        let jpegs: Vec<Vec<u8>> = grid.kept.iter().map(|f| f.jpeg.clone()).collect();

        // ── 窗口精化：帧级主边界（阶段 1）+ 短字幕召回（阶段 2）────
        // 窗口 = 扫描序列的时间切片（零抽帧零落盘）：
        // - 真正的下一段边界 = 最后一个变化帧（而非首个 main，规避 A→短字幕→C 误判）
        // - 窗口内恰好两次变化时，中间区间为短字幕：≥2 帧则按需抽中间帧召回为独立段
        let refine_started = std::time::Instant::now();
        let (changes, mut short_segments) = if src_fps > 0.0 {
            emit(
                i,
                (i as f64 + 0.55) / clip_count as f64,
                format!("精化边界 {}/{}", i + 1, clip_count),
            );
            refine_window_changes(
                &changes,
                &scan_stream,
                video_path,
                clip,
                video_w,
                video_h,
                src_fps,
                dhash_threshold,
                &clip_dir,
                manager,
                dev_debug,
            )
        } else {
            (changes, Vec::new())
        };
        let refine_dt = refine_started.elapsed();


        emit(
            i,
            (i as f64 + 0.6) / clip_count as f64,
            format!("OCR 识别 {}/{}", i + 1, clip_count),
        );

        let total_frames = changes.len();
        let ocr_started = std::time::Instant::now();
        let mut ipc_time = std::time::Duration::ZERO;
        let mut ipc_batches = 0usize;
        let texts = ocr_pass(
            &changes,
            &jpegs,
            |images| {
                let t = std::time::Instant::now();
                let r = manager.with_provider(|p| p.recognize_batch(images));
                ipc_time += t.elapsed();
                ipc_batches += 1;
                r
            },
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
        let ocr_dt = ocr_started.elapsed();
        if dev_debug {
            eprintln!(
                "[ocr] clip {}/{}：耗时 共{:.2}s = 扫描{:.2} + 抽帧{:.2} + 精化{:.2} + OCR{:.2}（IPC {:.2}s，{} 批）",
                i + 1,
                clip_count,
                clip_started.elapsed().as_secs_f64(),
                scan_dt.as_secs_f64(),
                extract_dt.as_secs_f64(),
                refine_dt.as_secs_f64(),
                ocr_dt.as_secs_f64(),
                ipc_time.as_secs_f64(),
                ipc_batches
            );
        }

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

        let mut segments = merge_frames(texts, frame_interval, clip.end, merge_similarity);
        // 阶段 2：并入窗口内召回的短字幕段，按时间排序
        segments.append(&mut short_segments);
        segments.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // 第二遍：孤立短段（渐进中间态/残缺帧，含误召回的抖动短字幕）并入相邻完整段
        let segments = merge_fragments(segments, frame_interval);
        // 第三遍：相邻文本相似合并（消除阶段 2 召回的同句碎片/伪短字幕）
        let segments = merge_similar_adjacent(segments, merge_similarity);
        // 第四遍：精化可能让 start 提前 → clamp 相邻段时间，保证单调不重叠
        let segments = clamp_segment_times(segments);
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

        // ── 每 clip 增量清理：本 clip 的帧已全部消费完，及时释放磁盘 ──
        // dev_debug 保留整个目录供人工检查；运行级 TempDirGuard 兜底失败路径
        if !dev_debug {
            let _ = std::fs::remove_dir_all(&clip_dir);
        }
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
    // 源视频帧率：窗口精化用。前端已获取元数据，直接传入避免重复探测
    src_fps: f64,
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
            src_fps,
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

/// 把 OCR 结果拆成语料文本行：每张图的整图文本按换行拆分，
/// trim 后丢弃空行并批内去重（精确匹配）。
/// 行级清洗（白名单/低置信度/重复行）已由 Python worker 完成，这里只做拆分归一。
pub fn lines_from_results(results: &[OcrResult]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for result in results {
        for line in result.text.split('\n') {
            let line = line.trim();
            if line.is_empty() || !seen.insert(line.to_string()) {
                continue;
            }
            lines.push(line.to_string());
        }
    }
    lines
}

/// Tauri 命令：直接识别用户选择的剧情文本截图（语料来源，不经过视频抽帧流水线）。
///
/// 直接读取文件字节并 base64 编码交给 worker（worker 内存解码 ndarray）——
/// 不经临时目录，也根除了 cv2 读图在 Windows 上对非 ASCII 路径不可靠的问题。
#[tauri::command]
pub async fn run_ocr_images(
    app: AppHandle,
    image_paths: Vec<String>,
    batch_size: usize,
) -> Result<Vec<String>, String> {
    if image_paths.is_empty() {
        return Err("未选择图片".into());
    }
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<OcrManager>();
        let ready = manager.with_provider(|p| p.is_ready());
        if !ready {
            return Err("OCR 运行环境未就绪".into());
        }

        let mut images = Vec::with_capacity(image_paths.len());
        for src in &image_paths {
            let bytes =
                std::fs::read(src).map_err(|e| format!("读取图片失败（{}）: {}", src, e))?;
            images.push(b64(&bytes));
        }

        let bs = batch_size.max(1);
        let total_batches = images.len().div_ceil(bs);
        let mut results = Vec::new();
        for (i, chunk) in images.chunks(bs).enumerate() {
            let batch = manager
                .with_provider(|p| p.recognize_batch(chunk))
                .map_err(|e| format!("OCR 失败: {}", e))?;
            results.extend(batch);
            let _ = app_handle.emit(
                OCR_PROGRESS_EVENT,
                OcrProgress {
                    // 与 run_ocr 的 clip_index（1 基）及消息文本 {i+1} 保持一致
                    clip_index: i + 1,
                    clip_count: total_batches,
                    progress: (i + 1) as f64 / total_batches as f64,
                    message: format!("识别截图 {}/{}", i + 1, total_batches),
                },
            );
        }
        Ok(lines_from_results(&results))
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
            time,
            is_changed: changed,
            base_hash: None,
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
        let jpegs = vec![b"J1".to_vec(), b"J2".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {}).unwrap();
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
        let jpegs = vec![b"J".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A"); // 已 trim
    }

    #[test]
    fn test_ocr_pass_empty_change_resets() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: String::new(), confidence: 0.0 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, false)];
        let jpegs = vec![b"J1".to_vec(), b"J2".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 1, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[1].text.as_ref(), ""); // 变化帧识别为空 → 清空
        assert_eq!(texts[2].text.as_ref(), ""); // 顺延空
    }

    #[test]
    fn test_ocr_pass_batch_mixed_changed() {
        let provider = mock(vec![OcrResult { text: "X".into(), confidence: 0.7 }]);
        let changes = vec![frame(0.0, false), frame(1.0, true)];
        let jpegs = vec![b"J".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 2, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), ""); // 首帧未变化且无 previous → 空
        assert_eq!(texts[1].text.as_ref(), "X");
    }

    #[test]
    fn test_ocr_pass_result_mismatch_errors() {
        // 预置结果比变化帧少 → 应返回错误而非静默空文本；
        // 图像数据与变化帧数量不符 → 在调用 recognize 前即报错
        let provider = mock(vec![OcrResult { text: "A".into(), confidence: 0.9 }]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, true)];
        let jpegs = vec![b"J".to_vec()];
        assert!(ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 1, |_, _| {}).is_err());
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

    // ── 多数投票 ──

    #[test]
    fn test_vote_ignores_noise_long_text() {
        // 噪声帧更长但与多数帧差异大 → 投票选多数一致文本，不被更长噪声污染
        let frames = vec![
            ft(1.0, "前方似乎有什么东西在等待。", 0.9),
            ft(2.0, "前方似乎有什么东西在等待。", 0.9),
            ft(3.0, "前方似乎有什么东西在等待。前方似乎有什么东X乱码Z", 0.6),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "前方似乎有什么东西在等待。");
    }

    // ── 空帧容错 ──

    #[test]
    fn test_merge_empty_tolerance_keeps_run() {
        // 单帧空（interval 0.5 → 容错窗口 0.8s）视为 OCR 抖动，不打断 run
        let frames = vec![
            ft(1.0, "A", 0.9),
            ft(2.0, "A", 0.9),
            ft(2.5, "", 0.0),
            ft(3.0, "A", 0.9),
        ];
        let segs = merge_frames(frames, 0.5, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "A");
        assert!((segs[0].end - 3.5).abs() < 1e-9); // last=3.0 + interval 0.5
    }

    #[test]
    fn test_merge_empty_beyond_tolerance_breaks() {
        // 连续空帧超过容错窗口（1.0s 间隔 → 窗口 1.5s）→ 打断
        let frames = vec![
            ft(1.0, "A", 0.9),
            ft(2.0, "A", 0.9),
            ft(3.0, "", 0.0),
            ft(4.0, "", 0.0),
            ft(5.0, "", 0.0),
            ft(6.0, "B", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 2);
    }

    // ── 孤立短段并入 ──

    #[test]
    fn test_merge_fragments_absorb_prefix() {
        // 渐进中间态短段在前、完整句在后 → 并入（诊断案例：卡侬·那是我本职工）
        let segs = vec![
            OcrSegment { start: 1.0, end: 2.0, text: "卡侬\n·那是我本职工".into(), confidence: 0.8 },
            OcrSegment { start: 2.0, end: 9.0, text: "卡侬\n…那是我本职工作的一部分。他们的主祭呼唤我的名字。".into(), confidence: 0.9 },
        ];
        let out = merge_fragments(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.starts_with("卡侬\n…那是我本职工作的一部分"));
        assert!((out[0].start - 1.0).abs() < 1e-9);
        assert!((out[0].end - 9.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_fragments_absorb_suffix() {
        // 完整句在前、尾部残缺短段在后 → 并入（时间合并，文本不变）
        let segs = vec![
            OcrSegment { start: 1.0, end: 8.0, text: "卡侬\n…那是我本职工作的一部分。他们的主祭呼唤我的名字。".into(), confidence: 0.9 },
            OcrSegment { start: 8.0, end: 9.0, text: "呼唤我的名字".into(), confidence: 0.8 },
        ];
        let out = merge_fragments(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert!((out[0].end - 9.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_fragments_keeps_real_short() {
        // 真短句（非相邻段子序列）不并入
        let segs = vec![
            OcrSegment { start: 1.0, end: 2.0, text: "嗯？".into(), confidence: 0.9 },
            OcrSegment { start: 2.0, end: 9.0, text: "我们出发吧。".into(), confidence: 0.9 },
        ];
        let out = merge_fragments(segs, 0.5);
        assert_eq!(out.len(), 2);
    }

    // ── 相邻段时间 clamp（精化后防重叠）──

    #[test]
    fn test_clamp_segment_times_no_overlap() {
        // 前段 end 与后段 start 交叉 → clamp 到后段 start
        let segs = vec![
            OcrSegment { start: 1.0, end: 5.0, text: "A".into(), confidence: 0.9 },
            OcrSegment { start: 3.0, end: 8.0, text: "B".into(), confidence: 0.9 },
        ];
        let out = clamp_segment_times(segs);
        assert_eq!(out.len(), 2);
        assert!((out[0].end - 3.0).abs() < 1e-9);
        assert!((out[1].start - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_clamp_segment_times_keeps_gap() {
        // 本就无重叠 → 时间不变
        let segs = vec![
            OcrSegment { start: 1.0, end: 3.0, text: "A".into(), confidence: 0.9 },
            OcrSegment { start: 5.0, end: 9.0, text: "B".into(), confidence: 0.9 },
        ];
        let out = clamp_segment_times(segs);
        assert!((out[0].end - 3.0).abs() < 1e-9);
        assert!((out[1].end - 9.0).abs() < 1e-9);
    }

    // ── 相邻相似合并（阶段 2 去伪短字幕）──

    #[test]
    fn test_merge_similar_adjacent_merges_same_text() {
        // 完全相同文本的相邻段 → 合并为一段（时间取并集）
        let segs = vec![
            OcrSegment { start: 1.0, end: 3.0, text: "卡侬\n…那是我本职工作的一部分。".into(), confidence: 0.9 },
            OcrSegment { start: 3.0, end: 3.2, text: "卡侬\n…那是我本职工作的一部分。".into(), confidence: 0.9 },
        ];
        let out = merge_similar_adjacent(segs, 0.3);
        assert_eq!(out.len(), 1);
        assert!((out[0].end - 3.2).abs() < 1e-9);
    }

    #[test]
    fn test_merge_similar_adjacent_keeps_distinct() {
        // 文本差异大的相邻段 → 不合并
        let segs = vec![
            OcrSegment { start: 1.0, end: 3.0, text: "卡侬".into(), confidence: 0.9 },
            OcrSegment { start: 3.0, end: 5.0, text: "获得".into(), confidence: 0.9 },
        ];
        let out = merge_similar_adjacent(segs, 0.3);
        assert_eq!(out.len(), 2);
    }

    // ── lines_from_results ──

    #[test]
    fn test_lines_from_results_split_and_dedup() {
        // 拆行、trim、丢空行；批内及跨图去重（精确匹配）
        let results = vec![
            OcrResult {
                text: "旅行者，你来了\n\n  派蒙：  \n旅行者，你来了".into(),
                confidence: 0.9,
            },
            OcrResult {
                text: "  第二张的行  \n旅行者，你来了".into(),
                confidence: 0.8,
            },
        ];
        let lines = lines_from_results(&results);
        assert_eq!(lines, vec!["旅行者，你来了", "派蒙：", "第二张的行"]);
    }

    #[test]
    fn test_lines_from_results_empty() {
        assert!(lines_from_results(&[]).is_empty());
        let blank = vec![OcrResult { text: "  \n\n".into(), confidence: 0.5 }];
        assert!(lines_from_results(&blank).is_empty());
    }
}
