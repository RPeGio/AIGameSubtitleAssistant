//! 基准：测量窗口精化（refine_window_changes）耗时，隔离于整体 OCR 流水线。
//! 运行：cargo test --release --test ocr_bench_refinement -- --ignored --nocapture

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::dhash::detect_changes;
use ai_game_subtitle_assistant_lib::ai_runtime::OcrManager;
use ai_game_subtitle_assistant_lib::ocr::{refine_window_changes, OcrRegionInput};
use ai_game_subtitle_assistant_lib::video::{extract_frames, get_video_metadata};
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

    let dir = std::env::temp_dir().join(format!("gsa_bench_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut total_refine = std::time::Duration::ZERO;
    let mut total_frames = 0usize;
    let mut total_changed = 0usize;

    for (i, clip) in clips.iter().enumerate() {
        let clip_dir = dir.join(format!("clip_{}", i));
        std::fs::create_dir_all(&clip_dir).unwrap();
        let grid = extract_frames(
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
            &clip_dir,
        )
        .unwrap();
        let changes = detect_changes(&grid, dhash_threshold).unwrap();
        let changed = changes.iter().filter(|c| c.is_changed).count();

        let t0 = Instant::now();
        let (refined, short) = refine_window_changes(
            &changes,
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
        let dt = t0.elapsed();
        total_refine += dt;
        total_frames += grid.len();
        total_changed += changed;

        println!(
            "clip {i}: grid={} changed={} refined={} short={} refine={:.3}s",
            grid.len(),
            changed,
            refined.len(),
            short.len(),
            dt.as_secs_f64()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);

    // ── 成本分解：对 clip 2 测量 extract_frames 与 dHash 各自的耗时 ──
    {
        let clip = &clips[2];
        let clip_dir = dir.join("clip_decomp");
        std::fs::create_dir_all(&clip_dir).unwrap();
        let grid = extract_frames(
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
            &clip_dir,
        )
        .unwrap();
        let changes = detect_changes(&grid, dhash_threshold).unwrap();
        let dense_interval = 1.0 / src_fps;
        let mut ext_time = std::time::Duration::ZERO;
        let mut dhash_time = std::time::Duration::ZERO;
        let mut n_extract = 0usize;
        let mut n_dhash = 0usize;
        for idx in 0..changes.len() {
            let fc = &changes[idx];
            if fc.is_changed && idx > 0 {
                if let Some(base) = fc.base_hash {
                    let lo = changes[idx - 1].frame.time;
                    let hi = fc.frame.time;
                    if hi > lo {
                        let dense_dir = clip_dir.join(format!("dense_{}", idx));
                        let t0 = Instant::now();
                        if let Ok(dense_frames) = extract_frames(
                            &video.to_string_lossy(),
                            lo,
                            hi,
                            clip.x1,
                            clip.y1,
                            clip.x2,
                            clip.y2,
                            meta.width,
                            meta.height,
                            dense_interval,
                            &dense_dir,
                        ) {
                            ext_time += t0.elapsed();
                            n_extract += 1;
                            let t1 = Instant::now();
                            for f in &dense_frames {
                                let _ = ai_game_subtitle_assistant_lib::ai_runtime::dhash::dhash_file(&f.path);
                                n_dhash += 1;
                            }
                            dhash_time += t1.elapsed();
                            let _ = base;
                        }
                    }
                }
            }
        }
        println!("\n===== 成本分解（clip 2）=====");
        println!("extract_frames: {} 次调用, 共 {:.3}s", n_extract, ext_time.as_secs_f64());
        println!("dhash_file: {} 次, 共 {:.3}s", n_dhash, dhash_time.as_secs_f64());

        // 整段一次性抽密帧（30fps）耗时对比
        let whole_dir = dir.join("clip_whole");
        std::fs::create_dir_all(&whole_dir).unwrap();
        let t0 = Instant::now();
        let whole = extract_frames(
            &video.to_string_lossy(),
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            meta.width,
            meta.height,
            dense_interval,
            &whole_dir,
        )
        .unwrap();
        let whole_extract = t0.elapsed();
        let t1 = Instant::now();
        for f in &whole {
            let _ = ai_game_subtitle_assistant_lib::ai_runtime::dhash::dhash_file(&f.path);
        }
        let whole_dhash = t1.elapsed();
        println!(
            "整段抽帧: {} 帧, extract={:.3}s, dhash={:.3}s",
            whole.len(),
            whole_extract.as_secs_f64(),
            whole_dhash.as_secs_f64()
        );
    }

    println!("\n===== 精化基准 =====");
    println!("网格帧总数: {}  变化帧: {}", total_frames, total_changed);
    println!("精化总耗时: {:.3}s", total_refine.as_secs_f64());
}
