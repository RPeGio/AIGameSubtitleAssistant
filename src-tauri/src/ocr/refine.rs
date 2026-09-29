use std::path::Path;
use super::params::{OcrRegionInput, OcrSegment};
use super::pipeline::{b64, fmt_time};

/// 阶段 2 短字幕召回的时长下限（秒）：短于此视为逐帧闪烁/噪声，不值得召回。
///
/// 背景：每次召回 = 一次 ffmpeg 单帧抽取（~0.19s）+ 一次单图 OCR IPC（~0.42s）。
/// pierro 嵌字跑实测在「恰 2 边界」窗口共触发 878 次候选、无门限时精化 546s；
/// 绝大多数候选只持续 1~2 帧（≈33ms），是打字机/画面动效的亚阈值抖动而非真短字幕。
///
/// 取值依据（2026-09-14 门限网格 + 同会话受控四跑，pierro 候选窗 878、贴边被拒 2）：
/// | 门限 | 召回 | 精化 | clip 总计 | 复合 | Δend p95 |
/// | 无   | 878 | 519s | 2567s | 15.5 | 9.80s |
/// | .10/.05 | 586 | 339s | 2371s | 16.1 | 12.32s |
/// | .15/.075| 299 | 179s | 2213s | 17.4 | 10.77s |
/// | .20/.10 | 113 |  71s | 2157s | 18.8 | 10.48s |
/// 边际质量代价随剪枝加深而递增（0.0021 / 0.0045 / 0.0075 分·召回⁻¹）。取 0.15/0.075：
/// 精化 −65%、总耗时 −13.8%，且新三轴报告（结构/时间/覆盖）全面不劣于 0.10/0.05。
pub(crate) const RECALL_MIN_SPAN_SEC: f64 = 0.15;
/// 召回候选两侧（A/C 状态）各自的持续时长下限（秒）：排除"短字幕恰好贴住
/// 窗口边缘"的伪影（真正的短字幕出现在窗口内部时，两侧都有垫底状态）。
pub(crate) const RECALL_MIN_SIDE_SEC: f64 = 0.075;

/// 阶段 2 召回候选判定：窗口内 `[m, s)` 为疑似短字幕（boundaries.len()==2 时）。
///
/// - 区间至少两帧（对应旧条件 `s > m + 1`）
/// - 候选可见时长 ≥ `min_span`
/// - 候选前后（A 侧 `[0, m)`、C 侧 `[s, wlen)`）各持续 ≥ `min_side`
pub(crate) fn recall_eligible(
    m: usize,
    s: usize,
    wlen: usize,
    dense_interval: f64,
    min_span: f64,
    min_side: f64,
) -> bool {
    if s <= m + 1 || s >= wlen {
        return false;
    }
    let span = (s - m) as f64 * dense_interval;
    let a_side = m as f64 * dense_interval;
    let c_side = (wlen - s) as f64 * dense_interval;
    span >= min_span && a_side >= min_side && c_side >= min_side
}

/// 条带（检测层思路②）：把 9×8 哈希的 8 行切成 3 条横带（3/3/2 行）。
pub(crate) const BAND_ROWS: [(usize, usize); 3] = [(0, 3), (3, 6), (6, 8)];

/// 主导性倍数：最大变化带须 ≥ 其它带最大值的此倍数，才认"这是文字带在变"。
///
/// 依据：字幕变化只发生在文字所在带；**运镜/整体运动会让所有带一起变** → 无主导 →
/// 回退整区判据（这正是检测层尝试①过冲的根因：无案例区分能力时单侧灵敏度必然此消彼长）。
pub(crate) const BAND_DOMINANCE: f64 = 2.0;

/// 用**变化最大的那条带**定位本段起点（检测层思路②）。
///
/// 返回窗口内越阈帧下标；`None` 表示无主导带（运镜/整体变化）→ 调用方回退整区判据。
/// 条带阈值按位数等比缩放（`threshold × 带位数 / 64`，下限 2），故条带判据在文字带上
/// 比整区判据更灵敏——字幕带的局部字形变化不再被其它带平均稀释。
pub(crate) fn band_onset(window: &[u64], base: u64, threshold: u32) -> Option<usize> {
    let last = *window.last()?;
    let totals: Vec<u32> = BAND_ROWS
        .iter()
        .map(|(a, b)| {
            crate::ai_runtime::dhash::hamming_distance_rows(base, last, *a, *b)
        })
        .collect();
    let (bi, &tmax) = totals.iter().enumerate().max_by_key(|(_, &t)| t)?;
    let other_max = totals
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != bi)
        .map(|(_, &t)| t)
        .max()
        .unwrap_or(0);
    // 无主导带（最大值不足其它带的 2 倍）→ 视为整体变化，交回整区判据
    if tmax == 0 || (tmax as f64) < BAND_DOMINANCE * (other_max as f64) {
        return None;
    }
    let (a, b) = BAND_ROWS[bi];
    let band_bits = ((b - a) as u32) * 8;
    let band_th = ((threshold * band_bits) / 64).max(2);
    window
        .iter()
        .position(|&h| crate::ai_runtime::dhash::hamming_distance_rows(base, h, a, b) > band_th)
}

/// 段尾精化（B-ii）：把 `end = 末采样 + interval×0.5` 的**量化估计**替换为密帧实测的切换时刻。
///
/// 依据（2026-09-18 边界形态诊断）：Δend p95 达 0.87s（glupov）/0.62s（moon），而容差 0.5s。
/// `末采样 + interval×0.5` 只是"覆盖率中点"估计（D2′），切换发生在采样点之后即欠伸、
/// 采样点紧贴切换即过伸。而**切换是突变**（整行新字幕），密帧能逐帧定位。
///
/// 与阶段 1 起点精化同源同数据（`scan_stream` 内存密帧，零抽帧零落盘）：以末采样处的哈希为
/// 基准，在**其后 1.5×interval 窗口**内定位下一条字幕的切换帧，取该时刻为段尾。
///
/// **条带优先（检测层思路②延伸，2026-09-18）**：与起点同法——窗口内若存在**主导变化带**
/// （某条带变化 ≥2× 其它带），用该带的首个越阈帧作段尾（局部字形切换不被其它带稀释）；
/// 无主导带（运镜/整体变化）回退整区判据（同阈值同语义）。
/// 窗口内无越阈（本段延续到 clip 末尾）→ 保留原估计。
pub(crate) fn refine_segment_ends(
    segments: Vec<OcrSegment>,
    dense: &[(f64, u64)],
    threshold: u32,
    interval: f64,
    clip_end: f64,
) -> Vec<OcrSegment> {
    if dense.is_empty() {
        return segments;
    }
    // 取时间上最接近 `t` 的密帧哈希（密帧间隔 ~1/src_fps，同一显示状态）
    let hash_near = |t: f64| -> Option<u64> {
        let i = dense.partition_point(|(dt, _)| *dt < t);
        let cand = [i.checked_sub(1), (i < dense.len()).then_some(i)];
        cand.into_iter()
            .flatten()
            .min_by(|&a, &b| {
                (dense[a].0 - t)
                    .abs()
                    .partial_cmp(&(dense[b].0 - t).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|k| dense[k].1)
    };
    segments
        .into_iter()
        .map(|mut seg| {
            let last_sample = seg.end - interval * 0.5;
            if let Some(base) = hash_near(last_sample) {
                let hi = last_sample + interval * 1.5;
                let win: Vec<(f64, u64)> = dense
                    .iter()
                    .filter(|(dt, _)| *dt > last_sample && *dt <= hi)
                    .copied()
                    .collect();
                let win_hashes: Vec<u64> = win.iter().map(|(_, h)| *h).collect();
                // 条带优先；无主导带则回退整区首个越阈帧
                let pick = band_onset(&win_hashes, base, threshold)
                    .map(|j| win[j].0)
                    .or_else(|| {
                        win.iter()
                            .find(|(_, h)| {
                                crate::ai_runtime::dhash::hamming_distance(base, *h) > threshold
                            })
                            .map(|(t, _)| *t)
                    });
                if let Some(t) = pick {
                    seg.end = t.clamp(seg.start, clip_end);
                }
            }
            seg
        })
        .collect()
}

/// 修正相邻段时间重叠：段 `end` 不得超过下一段 `start`。
///
/// 阶段 1 的窗口精化会把 changed 帧 `start` 提前到帧级边界，而 `end` 仍按
/// 网格 `last + interval` 计算，可能产生交叉/重叠 → 统一 clamp 保证时间单调。
pub(crate) fn clamp_segment_times(mut segments: Vec<OcrSegment>) -> Vec<OcrSegment> {
    for i in 1..segments.len() {
        if segments[i - 1].end > segments[i].start {
            segments[i - 1].end = segments[i].start;
        }
    }
    segments
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
///
/// `on_refine(done, total, frac, message)`：每批次帧进度回调一次（聚合上限 ~50 次），
/// 用于消除精化阶段的长静默（pierro 曾 546s 无输出）。
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
    mut on_refine: impl FnMut(usize, usize, f64, String),
) -> (Vec<crate::ai_runtime::dhash::FrameChange>, Vec<OcrSegment>) {
    let dense_interval = 1.0 / src_fps;
    let mut short_segments: Vec<OcrSegment> = Vec::new();
    let mut refined: Vec<crate::ai_runtime::dhash::FrameChange> =
        Vec::with_capacity(changes.len());
    // 进度聚合步长：全片至多 ~50 次回调，避免逐帧刷屏
    let total = changes.len();
    let step = (total / 50).max(1);
    // P1 预筛门限：env 可覆盖（调参用），默认取常量
    let min_span = std::env::var("GSA_RECALL_MIN_SPAN_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RECALL_MIN_SPAN_SEC);
    let min_side = std::env::var("GSA_RECALL_MIN_SIDE_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RECALL_MIN_SIDE_SEC);

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
                    // 起点优先用**变化最大带**的越阈帧（检测层思路②）：文字带局部字形变化
                    // 不被其它带稀释；无主导带（运镜/整体变化）则回退整区判据（取最后越阈）
                    let pick = band_onset(&hashes, base, dhash_threshold)
                        .or_else(|| boundaries.last().copied());
                    if let Some(main) = pick {
                        // 用窗口内实际帧时间（而非 lo + main*interval），窗口首帧
                        // 未必恰在 lo，更精确也避免假设
                        let new_time = window[main].0;
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
                            // P1 预筛：候选须足够时长且两侧状态持续，
                            // 过滤逐帧闪烁，避免无收益的 ffmpeg 抽帧 + OCR IPC
                            if recall_eligible(
                                m,
                                s,
                                window.len(),
                                dense_interval,
                                min_span,
                                min_side,
                            ) {
                                // 区间中点帧做 OCR（避开切换过渡帧）；
                                // 单帧 mjpeg 管道取内存字节 → base64，不落盘
                                let mid = m + (s - m) / 2;
                                let start = lo + m as f64 * dense_interval;
                                let end = lo + s as f64 * dense_interval;
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
                                                        start,
                                                        end,
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
        if idx % step == 0 || idx + 1 == total {
            on_refine(
                idx + 1,
                total,
                (idx + 1) as f64 / total.max(1) as f64,
                format!("精化边界 {}/{} 帧", idx + 1, total),
            );
        }
    }
    (refined, short_segments)
}

// ─── 编排命令 ─────────────────────────────────────────────

