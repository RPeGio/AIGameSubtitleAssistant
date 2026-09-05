//! 端到端集成测试：真实视频 + 真实 OCR 环境。
//! 默认忽略（需先跑 scripts/bootstrap_ocr.ps1 就绪内嵌环境），
//! 运行：cargo test --release --test ocr_e2e -- --ignored --nocapture

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::OcrManager;
use ai_game_subtitle_assistant_lib::ocr::{
    fmt_time, run_ocr_pipeline, OcrRegionInput, OcrRunParams,
};
use ai_game_subtitle_assistant_lib::video::get_video_metadata;
use std::path::PathBuf;
use std::time::Instant;

/// 仓库根 = src-tauri 的上一级
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
#[ignore]
fn test_t3_end_to_end() {
    let root = repo_root();
    let video = root.join(r"examples\test(hi-res).mp4");
    let runtime_dir = root.join("runtime");
    assert!(video.is_file(), "缺少测试视频: {}", video.display());
    assert!(
        runtime_dir.join("config.json").is_file(),
        "缺少 runtime/config.json，请先跑 scripts/bootstrap_ocr.ps1"
    );

    let config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    let manager = OcrManager::new(config, runtime_dir);

    let ready = manager.with_provider(|p| p.is_ready());
    assert!(ready, "OCR 环境未就绪");

    let meta =
        get_video_metadata(video.to_string_lossy().into_owned()).expect("读取视频元数据失败");
    assert_eq!(meta.width, 1920);

    // 来自 t3 项目 project.json 语料区 OCR 选区轨（用户调整后的选区，已去除其它画面噪声）
    let clips = vec![
        OcrRegionInput { start: 36.373, end: 40.798, x1: 0.200, y1: 0.700, x2: 0.800, y2: 0.900 },
        OcrRegionInput { start: 43.330, end: 150.298, x1: 0.152, y1: 0.745, x2: 0.853, y2: 0.994 },
        OcrRegionInput { start: 153.263, end: 157.763, x1: 0.200, y1: 0.383, x2: 0.800, y2: 0.583 },
        OcrRegionInput { start: 178.277, end: 201.978, x1: 0.270, y1: 0.000, x2: 0.742, y2: 0.971 },
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
        &video.to_string_lossy(),
        meta.width,
        meta.height,
        &clips,
        &params,
        meta.fps,
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
