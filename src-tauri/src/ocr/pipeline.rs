use uuid::Uuid;
use std::path::PathBuf;
use std::sync::Arc;
use crate::ai_runtime::dhash::FrameChange;
use crate::ai_runtime::{OcrError, OcrManager, OcrResult};
use super::params::FrameText;
use super::correct::{collect_glossary_hits, consistency_diffs, merge_diff, normalize_punctuation, Diff};
use super::merge::{
    merge_contained_adjacent, merge_frames, merge_short_fragments_into_next,
    merge_similar_adjacent,
};
use super::params::{OcrRegionInput, OcrRunParams, OcrSegment};
use super::refine::{clamp_segment_times, refine_segment_ends, refine_window_changes};

/// JPEG 字节 → base64（OCR IPC 的图像载荷格式）
pub(crate) fn b64(data: &[u8]) -> String {
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
                sample_time: change.sample_time,
                text,
                confidence,
            });
        }
        on_progress(texts.len(), total);
    }
    Ok(texts)
}

/// 把变化标志与 mjpeg 流实际帧装配成 ocr_pass 的输入（changes 与 jpegs 严格对齐）。
///
/// 背景：网格帧数按 ceil(时长/间隔) 估算，而 ffmpeg fps 滤镜实际输出按 round 舍入，
/// 实测 dur=5s、interval=0.7s 时估算 8 帧、实际输出 7 帧。多出的"幻影"网格帧
/// （号 ≥ total）在 extract_frames_bytes 中无实际帧匹配、被静默丢弃。
///
/// 装配规则：
/// - 实际帧（0..total）直接装配；`kept_jpegs` 必须与其中变化帧一一对应（显式校验，
///   违反即报错而非静默错位）
/// - 幻影网格帧若判为变化，说明片段尾部确有字幕切换：经 `recover_frame` 按网格时间
///   补抽单帧；补抽失败则放弃该帧（changes/jpegs 同步跳过，对齐不变）
pub(crate) fn assemble_grid_frames(
    flags: &[(bool, Option<u64>)],
    kept_jpegs: Vec<Vec<u8>>,
    total: usize,
    start: f64,
    interval: f64,
    mut recover_frame: impl FnMut(f64) -> Option<Vec<u8>>,
) -> Result<(Vec<FrameChange>, Vec<Vec<u8>>), OcrError> {
    // 对齐校验：kept 即全部 <total 的变化帧字节（extract_frames_bytes 按 keep 升序保留）
    let changed_below = (0..total.min(flags.len())).filter(|&k| flags[k].0).count();
    if kept_jpegs.len() != changed_below {
        return Err(OcrError::Worker(format!(
            "网格帧装配错位：{} 个变化帧，{} 张 JPEG",
            changed_below,
            kept_jpegs.len()
        )));
    }
    let mut changes: Vec<FrameChange> = (0..total)
        .map(|k| {
            let (is_changed, base_hash) = flags.get(k).copied().unwrap_or((false, None));
            let time = start + (k as f64) * interval;
            FrameChange {
                time,
                sample_time: time,
                is_changed,
                base_hash,
            }
        })
        .collect();
    let mut jpegs = kept_jpegs;
    // 幻影网格帧回收：补抽的帧号大于所有实际帧，追加在末尾保持升序对齐
    for (k, &(is_changed, base_hash)) in flags.iter().enumerate().skip(total) {
        if !is_changed {
            continue;
        }
        let time = start + (k as f64) * interval;
        if let Some(jpeg) = recover_frame(time) {
            changes.push(FrameChange {
                time,
                sample_time: time,
                is_changed: true,
                base_hash,
            });
            jpegs.push(jpeg);
        }
    }
    Ok((changes, jpegs))
}

/// 静态超时强制采样（D8 1b + C）：网格距最近一次 OCR 超过该秒数 → 强制 OCR 一帧。
///
/// 背景：9×8 dHash 对"画面静止 + 同位置整行文字替换"的分辨力不足（实测 pierro
/// 语料 [74]↔[75] 距离仅 2~4 位 ≤ 阈值 3），且渐进显示的逐步变化同样低于阈值——
/// 变化检测整句漏检时，静态超时帧可读到完整文本，由合并层吸收文本（时间轴不动）。
///
/// 取值（2026-09-16 由 4.0 → 1.5，C 方案）：保险丝锚点是 last_ocr_t = 字幕的
/// 帧级切换时刻（1a 注入覆写），故 T 即"该字幕至少每隔 T 秒被重读一次"——
/// 打字机通常 ~1s 内补全，T=1.5 保证寿命 >1.5s 的字幕至少采到一次完整态
/// （glupov [卡佳和科利亚] 句寿命 3.1s < 旧 T=4.0，完整态窗口 2.2s 从未被采）。
/// 代价：静态段 OCR 频率 ×2.7（同源模拟 glupov 语料两窗 20→24 / 32→35；
/// pierro 语料静默约 961s，约 +400 帧 ≈ +3 分钟）。env：GSA_OCR_STALE_TIMEOUT_SEC（≤0 关闭）。
pub(crate) const STALE_OCR_TIMEOUT_SEC: f64 = 1.5;

/// RAII 临时目录守卫：任何退出路径（含 panic 展开、JoinHandle 错误）都会清理。
/// `keep` 为 true 时保留目录（dev_debug 用于人工检查帧）。
pub(crate) struct TempDirGuard {
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
) -> Result<(Vec<OcrSegment>, Vec<Diff>), String>
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
    // 字幕预估最短长度：≤0 视为关闭短碎片激进合并
    let min_subtitle_sec = params.min_subtitle_sec;
    // 标点归一化配置（精度策略的前置层，用户可个性目标字符）
    let punctuation = params.punctuation.clone();
    // 术语表（S2，可选）：在标点归一化之后应用（词条与产出须同形才能匹配）
    let glossary = params.glossary.clone();
    // 一致性纠错（S3，可选）：只在末尾产出建议，不改写文本
    let consistency_hints = params.consistency_hints;
    // D8(1b) 静态超时保险丝：env 可覆盖，≤0 关闭
    let stale_timeout = std::env::var("GSA_OCR_STALE_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(STALE_OCR_TIMEOUT_SEC);

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
    // 待审批的文本纠正（术语表 + 一致性纠错）；Rust 侧只标记，不改文本
    let mut diffs: Vec<Diff> = Vec::new();
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
        let grid_times: Vec<f64> = (0..grid_frame_count)
            .map(|k| clip.start + (k as f64) * frame_interval)
            .collect();
        // D8：变化检测 + 补漏（帧级差分注入 + 静态超时），语义见 dhash::change_flags_rescued
        let (flags, time_overrides) = crate::ai_runtime::dhash::change_flags_rescued(
            &grid_hashes,
            &grid_times,
            &scan_stream,
            dhash_threshold,
            stale_timeout,
        );
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

        // 装配 changes/jpegs：以 mjpeg 流实际帧数为准，幻影变化帧按网格时间补抽单帧
        let (mut changes, jpegs) = assemble_grid_frames(
            &flags,
            grid.kept.iter().map(|f| f.jpeg.clone()).collect(),
            grid.total,
            clip.start,
            frame_interval,
            |time| {
                crate::video::extract_single_frame_bytes(
                    video_path,
                    time,
                    clip.x1,
                    clip.y1,
                    clip.x2,
                    clip.y2,
                    video_w,
                    video_h,
                )
                .ok()
            },
        )
        .map_err(|e| format!("网格帧装配失败（clip {}）: {}", i, e))?;
        // D8(1a)：帧级差分注入的变化帧，有效时刻覆写为切换帧时刻（帧级精确段首）；
        // 采样覆盖（sample_time）保留原网格时刻，段尾推进不受段首前移影响。
        // 精化的窗口边界读取 changes[].time，覆写须在精化前完成。
        for (k, t) in time_overrides.iter().enumerate() {
            if let (Some(t), Some(ch)) = (t, changes.get_mut(k)) {
                ch.sample_time = ch.time;
                ch.time = *t;
            }
        }

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
                // 精化阶段整体占用该 clip 进度 [0.55, 0.60]，随帧聚合上报（消除长静默）
                |done, total, _frac, msg| {
                    let frac = if total == 0 { 0.0 } else { done as f64 / total as f64 };
                    let progress = (i as f64 + 0.55 + frac * 0.05) / clip_count as f64;
                    emit(i, progress, msg);
                },
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
        // 第二遍：包含关系拼接（同一句的字幕分片合成一段，含误召回的抖动短字幕）
        let segments = merge_contained_adjacent(segments, frame_interval);
        // 第三遍：相邻文本相似合并（消除阶段 2 召回的同句碎片/伪短字幕）
        let segments = merge_similar_adjacent(segments, merge_similarity);
        // 第四遍（B-ii）：段尾精化——用密帧实测切换时刻替换"末采样+半间隔"的量化估计
        let segments =
            refine_segment_ends(segments, &scan_stream, dhash_threshold, frame_interval, clip.end);
        // 第五遍：精化可能让 start 提前 → clamp 相邻段时间，保证单调不重叠
        let segments = clamp_segment_times(segments);
        // 第六遍：短碎片激进合并（弱关联 + 时长/间隔门；保留碎片起点，见函数注释）。
        // 2026-10-06 起含**两个方向**：弱关联碎片并后条；亚帧过渡帧（如 pierro `[4]`
        // 0.084s `MurderofE/Mitya`——姓名框切换的那一帧）并**前**条，理由见函数注释。
        //
        // ⚠ **顺序要求（P2，2026-09-28）**：必须在**段尾精化与 clamp 之后**跑。本 pass 的时长门
        // `min_subtitle_sec` 语义是"字幕最短寿命"，而它的判据取 `last.end - last.start`：
        //   · 精化把终点前移到**实测切换时刻**；
        //   · clamp 再把与后段重叠的终点压掉。
        // 两者都会让时长变短——若在它们之前跑，比较的是"末采样 + 半间隔"的**粗估**时长
        // （长 0~0.5s），擦边的碎片会被误判为超阈而漏并。实测 pierro SRT `#111`（D12 碎片）：
        // clamp 前 ≈3.63s（超 3.5s 门被拒）、clamp 后 **3.38s**（应在门内）。
        // 合并后再 `clamp` 本会产生重叠，故必须放在 clamp 之后（合并结果沿用后条已 clamp 的终点，
        // 而碎片的起点 ≤ 前段终点，故不引入重叠）。
        let mut segments = merge_short_fragments_into_next(segments, min_subtitle_sec);
        // 末步（输出层）：标点/省略号归一化（**静默执行**，用户 2026-09-24 决策）+
        // 术语表**标记**（只收集 Diff，不改文本——纠正由前端审批后执行）。
        // 顺序关键：归一化必须先做（词条与产出须同形才能匹配）
        for seg in &mut segments {
            let normalized = normalize_punctuation(&seg.text, &punctuation);
            if let Some((olds, new)) = collect_glossary_hits(&normalized, &glossary) {
                // 同一词条的多种误读形态合并进一个 Diff（`old` 为集合，跨条目全局生效）
                merge_diff(&mut diffs, olds, new);
            }
            seg.text = normalized;
        }
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

    // ── 一致性纠错（S3，可选）：全部 clip 完成后扫描形近变体，产出待审批 Diff ──
    // 与术语表一致：**只标记不改文本**（用户指出"多数派不一定正确"，须保留否决权）。
    // 走一遍进度回调告知数量，便于前端/日志观察（审批列表由返回的 diffs 提供）
    if consistency_hints {
        let cd = consistency_diffs(&all_segments);
        if cd.is_empty() {
            on_progress(0, 0, 1.0, "[精度建议] 未发现形近变体（无需处理）".to_string());
        } else {
            on_progress(
                0,
                0,
                1.0,
                format!("[精度建议] 发现 {} 组形近变体（多数派未必正确，请自行确认）", cd.len()),
            );
            for d in &cd {
                on_progress(
                    0,
                    0,
                    1.0,
                    format!("[精度建议] {} → {}", d.old.join("/"), d.new),
                );
                merge_diff(&mut diffs, d.old.iter().cloned().collect(), d.new.clone());
            }
        }
    }

    Ok((all_segments, diffs))
}

