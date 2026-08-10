//! 端到端集成测试：真实 llama-cli + Qwen 模型环境。
//! 默认忽略（需先跑 scripts/bootstrap_llm.ps1 就绪 runtime/bin/llm + 模型），
//! 运行：cargo test --release --test llm_e2e -- --ignored --nocapture

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::LlmManager;
use std::path::PathBuf;
use std::time::Instant;

/// 仓库根 = src-tauri 的上一级
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
#[ignore]
fn test_llm_end_to_end() {
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

    let start = Instant::now();
    let answer = manager
        .with_provider(|p| p.complete("2+2=?"))
        .expect("LLM 推理失败");
    let elapsed = start.elapsed();

    println!("\n========== llm 实测结果 ==========");
    println!("耗时: {:.1}s   回答: \"{}\"", elapsed.as_secs_f64(), answer);
    println!("=================================");

    // 基础健全性：链路通、有回答（模型质量不在此测试范围）
    assert!(!answer.trim().is_empty(), "推理结果为空");
}
