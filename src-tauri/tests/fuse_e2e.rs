//! 端到端集成测试：真实 LLM 融合链路（OCR 文本 + ASR 段 → LLM → fused 段）。
//! 默认忽略（需先跑 scripts/bootstrap_llm.ps1 就绪环境），
//! 运行：cargo test --release --test fuse_e2e -- --ignored --nocapture

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::LlmManager;
use ai_game_subtitle_assistant_lib::fuse::{fuse_pipeline, FuseAsrInput};
use std::path::PathBuf;
use std::time::Instant;

/// 仓库根 = src-tauri 的上一级
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
#[ignore]
fn test_fuse_end_to_end() {
    let root = repo_root();
    let runtime_dir = root.join("runtime");
    assert!(
        runtime_dir.join("config.json").is_file(),
        "缺少 runtime/config.json，请先跑 scripts/bootstrap_llm.ps1"
    );

    let config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    let manager = LlmManager::new(config, runtime_dir);
    let ready = manager.with_provider(|p| p.is_ready());
    assert!(ready, "LLM 环境未就绪");

    // 小批量双语样例：OCR 中文字幕 vs ASR 日语语音（跨语言语义对应）
    let ocr_texts = vec![
        "派蒙：旅行者，你来了".to_string(),
        "安柏：前方似乎有什么东西在等待".to_string(),
    ];
    let inputs = vec![
        FuseAsrInput {
            index: 1,
            start: 0.0,
            end: 1.5,
            text: "トラベラー、来たな".into(),
        },
        FuseAsrInput {
            index: 2,
            start: 1.5,
            end: 3.0,
            text: "何かが待っているようだ".into(),
        },
    ];

    let start = Instant::now();
    let result = fuse_pipeline(&manager, &ocr_texts, &inputs, |p, m| {
        println!("  [{:>5.1}%] {}", p * 100.0, m);
    })
    .expect("融合流水线失败");
    let elapsed = start.elapsed();

    println!("\n========== fuse 实测结果 ==========");
    println!(
        "耗时: {:.1}s   匹配: {}/{}   失败批: {}",
        elapsed.as_secs_f64(),
        result.stats.matched,
        result.stats.total,
        result.stats.failed_batches
    );
    for s in &result.segments {
        println!(
            "  [{} → {}] <{}> \"{}\"",
            s.start,
            s.end,
            s.character.as_deref().unwrap_or("-"),
            s.text
        );
    }
    println!("=================================");

    // 链路健全性：段数完整、文本非空；匹配与否由模型能力决定，不强断言
    assert_eq!(result.segments.len(), 2);
    for s in &result.segments {
        assert!(!s.text.trim().is_empty(), "存在空文本段");
    }
}
