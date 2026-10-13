//! 端到端集成测试：向量融合链路（真实 worker 子进程 + 真实模型）。
//! 默认忽略（需先跑 scripts/bootstrap_embed.ps1 就绪环境），
//! 运行：cargo test --release --test fuse_e2e -- --ignored --nocapture
//!
//! 对拍对象：`scripts/fuse_worker_parity.py` 已验证的 moon 案例结果
//! （R1 实测：15/15 全对，与实验台逐段一致）。Rust 侧只断言：
//! ① 段数与 matched 数 ② 每段命中的语料下标序列（1-based，-1=不配）
//! ③ 元数据完整性。正确性基线以 Ruby 层? ——— 见 §7.9：不硬断言文本内容。

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::fuse::{fuse_pipeline, FuseAsrInput};
use std::path::PathBuf;
use std::time::Instant;

/// 仓库根 = src-tauri 的上一级
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// moon 案例的预期命中序列（1-based 语料下标）：来源
/// `temp/bench_output/parity/worker_product_moon.json` 的 `segments[].ocr_index`
/// （R1 实测 = 实验台 fuse_align_srt.py 逐段一致，见 fuse_worker_parity.py 8/8 零差异）
const MOON_EXPECTED: &[i64] = &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// 从 .gsa JSON 正文读 corpus 与 game-asr 段表（moon 案例采用与 fuse_align_srt.py 相同口径）：
/// - corpus = project.corpus[].text
/// - 段表 = type=="asr" 且 track_role=="game" 全部事件按 start 排序
/// 注意：不读 embed_ocr（moon 案例无画面嵌字段）；不加 speaker 行（走 R2 的 F14 约定）
fn load_moon_input(root: &PathBuf) -> Result<(Vec<String>, Vec<FuseAsrInput>), String> {
    let gsa_path = root.join("examples/benchmark_examples/moon_sisters.gsa");
    let raw = std::fs::read_to_string(&gsa_path)
        .map_err(|e| format!("读取 {} 失败: {}", gsa_path.display(), e))?;
    let json = raw
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(&raw)
        .to_string();
    let proj: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!(".gsa JSON 解析失败: {}", e))?;

    let corpus: Vec<String> = proj["corpus"]
        .as_array()
        .ok_or("corpus 不是数组")?
        .iter()
        .map(|c| c["text"].as_str().unwrap_or("").to_string())
        .collect();
    if corpus.is_empty() {
        return Err("corpus 为空".into());
    }

    let mut inputs: Vec<FuseAsrInput> = Vec::new();
    for track in proj["tracks"].as_array().ok_or("tracks 不是数组")? {
        let is_asr = track["type"].as_str() == Some("asr");
        let is_game = track["track_role"].as_str() == Some("game");
        if !is_asr || !is_game {
            continue;
        }
        for ev in track["events"].as_array().ok_or("events 不是数组")? {
            inputs.push(FuseAsrInput {
                index: inputs.len() + 1,
                start: ev["start"].as_f64().unwrap_or(0.0),
                end: ev["end"].as_f64().unwrap_or(0.0),
                text: ev["text"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    inputs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    // 排序后 index 必须重排（前端是排好序填充 index，这里 sync 同口径）
    for (i, s) in inputs.iter_mut().enumerate() {
        s.index = i + 1;
    }
    if inputs.is_empty() {
        return Err("game=aSR 段表为空".into());
    }
    Ok((corpus, inputs))
}

#[test]
#[ignore]
fn test_fuse_end_to_end_vector_moon() {
    let root = repo_root();
    let runtime_dir = root.join("runtime");
    assert!(
        runtime_dir.join("config.json").is_file(),
        "缺少 runtime/config.json，请先跑 scripts/bootstrap_embed.ps1"
    );

    let config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    let missing: Vec<&str> = [
        ("embed_worker", &config.embed_worker),
        ("embed_model", &config.embed_model),
        ("embed_tokenizer", &config.embed_tokenizer),
        ("embed_deps_dir", &config.embed_deps_dir),
    ]
    .iter()
    .filter(|(_, v)| v.is_empty())
    .map(|(n, _)| *n)
    .collect();
    assert!(
        missing.is_empty(),
        "embed_* 未配置（缺 {:?}），请先跑 scripts/bootstrap_embed.ps1（详见 fuse_worker_parity.py 或《development.md》）",
        missing
    );

    let (corpus, inputs) = load_moon_input(&root).expect("装配 moon 输入失败");

    let start = Instant::now();
    let result = fuse_pipeline(
        &config, &runtime_dir, &corpus, &inputs, false,
        |p, m| println!("  [{:>5.1}%] {}", p * 100.0, m),
    )
    .expect("融合流水线失败");
    let elapsed = start.elapsed();

    println!("\n========== fuse(vector) 实测结果 ==========");
    println!(
        "耗时: {:.2}s   匹配: {}/{}",
        elapsed.as_secs_f64(),
        result.stats.matched,
        result.stats.total
    );
    for (i, s) in result.segments.iter().enumerate() {
        let meta = result.metas.as_ref().and_then(|m| m.get(i));
        let meta_s = meta
            .map(|m| format!(" ocr_index={} score={:.4}", m.ocr_index, m.score))
            .unwrap_or_default();
        println!("  [{} → {}]{} \"{}\"", s.start, s.end, meta_s, s.text);
    }
    println!("=========================================");

    // 链路健全性：段数完整 + 全命中 + 命中序列与 parity 基线一致
    assert_eq!(result.segments.len(), MOON_EXPECTED.len(), "段数不符");
    assert_eq!(result.stats.matched, MOON_EXPECTED.len(), "全段落都应命中（moon 已验证 15/15）");
    for s in &result.segments {
        assert!(!s.text.trim().is_empty(), "存在空文本段");
    }
    let metas = result.metas.as_ref().expect("必须有审计元数据");
    for (i, (meta, exp)) in metas.iter().zip(MOON_EXPECTED).enumerate() {
        assert_eq!(
            meta.ocr_index as i64, *exp,
            "第 {} 段命中语料下标不符（期望 {}）",
            i + 1, exp
        );
    }
}
