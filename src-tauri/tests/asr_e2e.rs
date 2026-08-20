//! 端到端集成测试：真实视频 + 真实 ASR 环境。
//! 默认忽略（需先跑 scripts/bootstrap_moss.ps1 就绪 runtime/bin + 模型），
//! 运行：cargo test --release --test asr_e2e -- --ignored --nocapture
//!
//! 引擎与视频可通过环境变量覆盖（默认 funasr + examples/asr_test.mp4）：
//!   GSA_E2E_ENGINE=moss   选 MOSS 引擎（GPU/CPU 回归对比用）
//!   GSA_E2E_VIDEO=<path>  指定测试视频（相对仓库根或绝对路径）

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::{AsrManager, AsrSegment};
use ai_game_subtitle_assistant_lib::asr::{run_asr_pipeline, AsrRunParams};
use ai_game_subtitle_assistant_lib::ocr::fmt_time;
use ai_game_subtitle_assistant_lib::video::get_video_metadata;
use std::path::PathBuf;
use std::time::Instant;

/// 仓库根 = src-tauri 的上一级
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
#[ignore]
fn test_asr_end_to_end() {
    let root = repo_root();
    let video = std::env::var("GSA_E2E_VIDEO").map_or_else(
        |_| root.join(r"examples\asr_test.mp4"),
        |p| {
            let pb = PathBuf::from(&p);
            if pb.is_absolute() { pb } else { root.join(pb) }
        },
    );
    let engine = std::env::var("GSA_E2E_ENGINE").unwrap_or_else(|_| "funasr".into());
    let runtime_dir = root.join("runtime");
    assert!(video.is_file(), "缺少测试视频: {}", video.display());
    assert!(
        runtime_dir.join("config.json").is_file(),
        "缺少 runtime/config.json，请先跑 scripts/bootstrap_moss.ps1"
    );

    let config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    let manager = AsrManager::new(config, runtime_dir);

    let params = AsrRunParams {
        engine,
        max_speakers: None,
        language: None,
    };
    let ready = manager.with_engine(&params.engine, |p| p.is_ready());
    assert!(ready, "ASR 环境未就绪");

    let meta = get_video_metadata(video.to_string_lossy().into_owned()).expect("读取视频元数据失败");

    let mut last_progress = 0.0f64;
    let start = Instant::now();
    let segments = run_asr_pipeline(
        &manager,
        &video.to_string_lossy(),
        &params,
        |progress, message| {
            assert!(
                progress >= last_progress,
                "进度回退: {} < {}",
                progress,
                last_progress
            );
            last_progress = progress;
            println!("  [{:>5.1}%] {}", progress * 100.0, message);
        },
    )
    .expect("ASR 流水线失败");
    let elapsed = start.elapsed();

    println!("\n========== asr 实测结果 ==========");
    println!(
        "总耗时: {:.1}s（{:.2}min）  段数: {}  视频时长: {:.0}s",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() / 60.0,
        segments.len(),
        meta.duration
    );
    for s in segments.iter().take(10) {
        println!(
            "  [{} → {}] <{}> \"{}\"",
            fmt_time(s.start),
            fmt_time(s.end),
            s.speaker.as_deref().unwrap_or("-"),
            s.text
        );
    }
    if segments.len() > 10 {
        println!("  ... 共 {} 段", segments.len());
    }
    println!("=================================");

    // 基础健全性：非空、时间轴合法、时间不越界
    assert!(!segments.is_empty(), "转写结果为空");
    for s in &segments {
        assert!(s.start < s.end, "段时间轴非法: {} >= {}", s.start, s.end);
        assert!(s.end <= meta.duration + 0.5, "段越界: {} > 视频时长 {}", s.end, meta.duration);
        assert!(!s.text.trim().is_empty(), "存在空文本段");
    }

    // 单段过长检查：worker 设 vad_kwargs max_single_segment_time=30000，
    // 段长不应超过 30s（留 0.5s 余量防浮点边界抖动）。
    // 曾出现 >100 词长字幕的问题（Fun-ASR-Nano 误配 punc_model 导致句子边界错乱），
    // 这里硬性兜底 + 下方打印分布便于观察断句质量
    let mut longest: Option<&AsrSegment> = None;
    let mut n_short = 0usize; // <5s
    let mut n_mid = 0usize; // 5-15s
    let mut n_long = 0usize; // 15-30s
    for s in &segments {
        let dur = s.end - s.start;
        if dur > 30.5 {
            panic!("存在超长段 {:.1}s: <{}> \"{}\"", dur, s.speaker.as_deref().unwrap_or("-"), s.text);
        }
        if longest.is_none_or(|l| dur > l.end - l.start) {
            longest = Some(s);
        }
        if dur < 5.0 {
            n_short += 1;
        } else if dur < 15.0 {
            n_mid += 1;
        } else {
            n_long += 1;
        }
    }
    let (l_dur, l_text) = match longest {
        Some(s) => (s.end - s.start, s.text.clone()),
        None => (0.0, String::new()),
    };
    println!(
        "段长分布: <5s: {}  5-15s: {}  15-30s: {}  最长段 {:.1}s: \"{}\"",
        n_short, n_mid, n_long, l_dur, l_text
    );
}
