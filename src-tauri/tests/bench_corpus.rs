//! 基准①：语料收集（OCR 剧情录屏/游戏录屏 → 语料条目）质量评测。
//!
//! 判定标准（与用户确认）：期望 = 参考文本全部条目**按 trim 后全等文本去重**
//! （语料片内容保证全覆盖，时长差异无关；参考按实况片校对，主播切页回放会让
//! 同一对话出现两条时间轴，与产物侧 pushCorpusTexts 的去重语义镜像——见
//! review-reports/BENCH_SCORING_DUPLICATE_EXPECTED.md）；产物多余行 = 噪音
//! （仅统计不判败，案例3 噪音留作融合容忍度验证素材）；缺失/识别错误 =
//! 流水线缺陷，按严重程度扣分（正确 0 分；轻度扣 1−sim；严重/缺失扣 1，
//! 见 common::score_corpus）。
//!
//! 运行：cargo test --release --test bench_corpus -- --ignored --nocapture --test-threads=1
//! 跑完把打印的 markdown 行粘到 benchmark/<案例>.md 的「语料收集」表。

mod common;

use common::{
    align_sequences, bench_data_dir, build_ocr_manager, corpus_regions, print_md_row, run_ocr,
    score_corpus, require_file, CaseCfg, GLUPOV, MOON_SISTERS, PIERRO_QUESTIONS,
};

fn run_case(cfg: &CaseCfg) {
    println!("\n========== 语料收集基准：{} ==========", cfg.key);
    let data = bench_data_dir();
    let video = data.join(cfg.corpus_video);
    let ref_path = data.join(cfg.ref_file);
    if !require_file(&video, "语料片视频") || !require_file(&ref_path, "参考文本") {
        return;
    }
    let Some(manager) = build_ocr_manager() else { return };

    let refs = match common::parse_reference(&ref_path, cfg.ref_fps) {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => {
            eprintln!("[跳过] 参考文本解析为空：{}", ref_path.display());
            return;
        }
        Err(e) => {
            eprintln!("[跳过] {e}");
            return;
        }
    };
    // 期望侧去重（方案 A，review-reports/BENCH_SCORING_DUPLICATE_EXPECTED.md）：
    // 参考按实况片校对，主播切页回放会产生同文双时间轴；产物侧按设计去重，
    // 期望侧也按 trim 后全等文本去重（保留首现），折叠数单列统计。
    let mut expected: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut folded = 0usize;
    for r in &refs {
        let t = r.text.trim();
        if t.is_empty() || !seen.insert(t.to_string()) {
            folded += 1;
            continue;
        }
        expected.push(t.to_string());
    }

    let (segments, elapsed) = run_ocr(&manager, &video, &corpus_regions(cfg.key));

    // 语料提取：移植 src/stores/project.ts pushCorpusTexts（trim→丢空→按序精确去重）
    let produced = common::corpus_from_segments(&segments);
    let (pairs, missed, extra) = align_sequences(&expected, &produced);
    let sc = score_corpus(&expected, &produced, &pairs, missed.len(), extra.len());

    // ── 终端摘要 ──
    println!(
        "耗时 {:.1}s｜OCR 段 {} → 语料 {} 条（参考 {} 条，同文折叠 {}）",
        elapsed,
        segments.len(),
        produced.len(),
        refs.len(),
        folded
    );
    println!(
        "评分 {:.1}/100｜正确 {} 轻度 {} 严重 {} 缺失 {}｜噪音行 {}｜CER 均值 {:.3} p95 {:.3} max {:.3}",
        sc.score, sc.correct, sc.minor, sc.severe, sc.missing, sc.noise,
        sc.cer_mean, sc.cer_p95, sc.cer_max
    );
    if !missed.is_empty() {
        println!("── 缺失条目 ──");
        for &i in &missed {
            println!("  [{}] {}", i + 1, expected[i].replace('\n', " / "));
        }
    }
    let mut worst: Vec<_> = pairs.iter().collect();
    worst.sort_by(|a, b| a.sim.partial_cmp(&b.sim).unwrap());
    if !worst.is_empty() {
        println!("── 最差匹配 Top{}（参考 → 产出）──", worst.len().min(10));
        for p in worst.iter().take(10) {
            println!(
                "  sim {:.3}: {}\n        → {}",
                p.sim,
                expected[p.exp].replace('\n', " / "),
                produced[p.prod].replace('\n', " / ")
            );
        }
    }

    // ── 可粘贴 markdown 行（列：日期|commit|评分|扣分明细|噪音行|备注）──
    println!("\n<!-- 粘贴到 benchmark/{}.md 的「语料收集」表 -->", cfg.key);
    print_md_row(&[
        String::new(),
        String::new(),
        format!("{:.1}", sc.score),
        format!("缺失{} 严重{} 轻度{}", sc.missing, sc.severe, sc.minor),
        format!("{}", sc.noise),
        format!("期望侧同文折叠 {}", folded),
    ]);
}

#[test]
#[ignore]
fn bench_corpus_moon_sisters() {
    run_case(&MOON_SISTERS);
}

#[test]
#[ignore]
fn bench_corpus_glupov() {
    run_case(&GLUPOV);
}

#[test]
#[ignore]
fn bench_corpus_pierro_questions() {
    run_case(&PIERRO_QUESTIONS);
}
