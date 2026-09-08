//! 基准：测量 OCR 变化检测（零落盘扫描）+ 内存抽帧 + 窗口精化耗时，隔离于整体流水线。
//! 运行：cargo test --release --test ocr_bench_refinement -- --ignored --nocapture

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::dhash::{change_flags, FrameChange};
use ai_game_subtitle_assistant_lib::ai_runtime::OcrManager;
use ai_game_subtitle_assistant_lib::ocr::{refine_window_changes, OcrRegionInput};
use ai_game_subtitle_assistant_lib::video::{
    extract_frames_bytes, get_video_metadata, scan_frame_hashes,
};
use std::path::PathBuf;
use std::time::Instant;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
#[ignore]
fn bench_refinement() {
    let root = repo_root();
    let video = root.join(r"examples\test(hi-res).mp4");
    let runtime_dir = root.join("runtime");
    assert!(video.is_file(), "缺少测试视频: {}", video.display());
    assert!(runtime_dir.join("config.json").is_file(), "缺少 runtime/config.json");

    let config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    let manager = OcrManager::new(config, runtime_dir);
    let meta = get_video_metadata(video.to_string_lossy().into_owned())
        .expect("读取视频元数据失败");

    // 与 e2e 相同的 4 段选区
    let clips = vec![
        OcrRegionInput { start: 0.0, end: 36.373, x1: 0.2, y1: 0.7, x2: 0.8, y2: 0.9 },
        OcrRegionInput { start: 36.373, end: 40.798, x1: 0.285, y1: 0.397, x2: 0.716, y2: 0.567 },
        OcrRegionInput { start: 40.798, end: 153.263, x1: 0.131, y1: 0.782, x2: 0.877, y2: 0.942 },
        OcrRegionInput { start: 153.263, end: 369.983, x1: 0.353, y1: 0.404, x2: 0.662, y2: 0.548 },
    ];
    let frame_interval = 1.0;
    let dhash_threshold = 3u32;
    let src_fps = meta.fps;
    // 与 run_ocr_pipeline 一致：扫描密度取源帧率，但不超过网格密度
    let scan_interval = (1.0 / src_fps).min(frame_interval);

    let dir = std::env::temp_dir().join(format!("gsa_bench_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut total_scan = std::time::Duration::ZERO;
    let mut total_extract = std::time::Duration::ZERO;
    let mut total_refine = std::time::Duration::ZERO;
    let mut total_frames = 0usize;
    let mut total_changed = 0usize;

    for (i, clip) in clips.iter().enumerate() {
        let clip_dir = dir.join(format!("clip_{}", i));
        std::fs::create_dir_all(&clip_dir).unwrap();

        // ── 零落盘扫描（变化检测 + 精化共用的哈希序列）──
        let t0 = Instant::now();
        let stream = scan_frame_hashes(
            &video.to_string_lossy(),
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            meta.width,
            meta.height,
            scan_interval,
        )
        .unwrap();
        let scan_dt = t0.elapsed();

        // 网格变化标志（与 run_ocr_pipeline 相同的映射）
        let last_scan = stream.len().saturating_sub(1);
        let grid_frame_count = ((clip.end - clip.start) / frame_interval).ceil() as usize;
        let grid_hashes: Vec<u64> = (0..grid_frame_count)
            .map(|k| {
                let idx = ((k as f64 * frame_interval / scan_interval).round() as usize)
                    .min(last_scan);
                stream[idx].1
            })
            .collect();
        let flags = change_flags(&grid_hashes, dhash_threshold);
        let keep: Vec<usize> = flags
            .iter()
            .enumerate()
            .filter(|(_, (c, _))| *c)
            .map(|(k, _)| k)
            .collect();

        // ── mjpeg 管道内存抽帧（只为变化帧保留字节，零落盘）──
        let t1 = Instant::now();
        let grid = extract_frames_bytes(
            &video.to_string_lossy(),
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            meta.width,
            meta.height,
            frame_interval,
            &keep,
        )
        .unwrap();
        let extract_dt = t1.elapsed();

        let changed = grid.kept.len();
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

        let t2 = Instant::now();
        let (refined, short) = refine_window_changes(
            &changes,
            &stream,
            &video.to_string_lossy(),
            clip,
            meta.width,
            meta.height,
            src_fps,
            dhash_threshold,
            &clip_dir,
            &manager,
            false,
        );
        let refine_dt = t2.elapsed();

        total_scan += scan_dt;
        total_extract += extract_dt;
        total_refine += refine_dt;
        total_frames += grid.total;
        total_changed += changed;

        println!(
            "clip {i}: grid={} changed={} refined={} short={} scan={:.3}s extract={:.3}s refine={:.3}s",
            grid.total,
            changed,
            refined.len(),
            short.len(),
            scan_dt.as_secs_f64(),
            extract_dt.as_secs_f64(),
            refine_dt.as_secs_f64()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);

    println!("\n===== 扫描+抽帧+精化基准 =====");
    println!("网格帧总数: {}  变化帧: {}", total_frames, total_changed);
    println!("扫描总耗时: {:.3}s", total_scan.as_secs_f64());
    println!("内存抽帧总耗时: {:.3}s（替代旧 JPEG 落盘）", total_extract.as_secs_f64());
    println!("精化总耗时: {:.3}s（含召回帧抽取+OCR）", total_refine.as_secs_f64());
}
