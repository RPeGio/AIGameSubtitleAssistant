//! 基准②：嵌字时间轴（OCR 切片视频内嵌字幕）评测。
//!
//! 判定标准（与用户确认）：期望 = 参考文本内的时间轴；同理扣分：
//! d = max(|Δstart|,|Δend|)，d ≤ 0.5×容差 0 分；(0.5×容差, 容差] 线性扣 0..1；
//! > 容差 扣 1；碎片化（1:N）扣 (N−1)×0.5；被吞并/缺失扣 1。
//! 产出多余段 = 噪音（仅统计）；期望条目落在选区时间窗外（选区未覆盖）
//! 单独报告、不计缺陷。文本相似度仅报告，不参与评分。
//!
//! 运行：cargo test --release --test bench_hardsub -- --ignored --nocapture --test-threads=1
//! 容差可用环境变量 GSA_BENCH_TOLERANCE_SEC 覆盖（默认 1.0s = 2×默认帧间隔）。
//! 跑完把打印的 markdown 行粘到 benchmark/<案例>.md 的「嵌字时间轴」表。

mod common;

use common::{
    align_temporal, bench_data_dir, build_ocr_manager, hardsub_regions, print_md_row, run_ocr,
    score_hardsub, require_file, CaseCfg, GLUPOV, MOON_SISTERS, PIERRO_QUESTIONS,
};

fn tolerance_sec() -> f64 {
    std::env::var("GSA_BENCH_TOLERANCE_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

fn run_case(cfg: &CaseCfg) {
    println!("\n========== 嵌字时间轴基准：{} ==========", cfg.key);
    let data = bench_data_dir();
    let video = data.join(cfg.clip_video);
    let ref_path = data.join(cfg.ref_file);
    if !require_file(&video, "测试片视频") || !require_file(&ref_path, "参考文本") {
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
    let regions = hardsub_regions(cfg.key);

    // 选区未覆盖的期望条目：与任何选区时间窗都不相交 → 单独报告，不计缺陷
    let out_of_region = refs
        .iter()
        .filter(|e| {
            regions
                .iter()
                .all(|r| e.end <= r.start || e.start >= r.end)
        })
        .count();

    let (segments, elapsed) = match run_ocr(&manager, &video, &regions) {
        Some(x) => x,
        None => return,
    };

    let align = align_temporal(&refs, &segments);
    let tol = tolerance_sec();
    let sc = score_hardsub(&refs, &segments, &align, out_of_region, tol);

    // ── 终端摘要 ──
    println!(
        "耗时 {:.1}s｜OCR 段 {}（参考 {} 条，选区外 {} 条，容差 {:.2}s）",
        elapsed,
        segments.len(),
        refs.len(),
        sc.n_out_of_region,
        tol
    );
    println!(
        "评分 {:.1}/100｜1:1 {} 碎片 {} 合并 {} 缺失 {}｜噪音段 {}｜Δstart p95 {:.3}s Δend p95 {:.3}s｜≤容差 {:.0}%｜覆盖率 {:.0}%｜文本相似度均值 {:.3}",
        sc.score, sc.one_to_one, sc.fragmented, sc.merged, sc.missed, sc.spurious,
        sc.dstart_p95, sc.dend_p95,
        sc.within_tolerance * 100.0,
        sc.coverage * 100.0,
        sc.text_sim_mean
    );
    if !align.missed.is_empty() {
        println!("── 缺失条目（时间轴）──");
        for &i in &align.missed {
            println!("  [{}] {:7.2} → {:7.2}  {}", i + 1, refs[i].start, refs[i].end, refs[i].text.replace('\n', " / "));
        }
    }

    // ── 可粘贴 markdown 行（列：日期|commit|评分|1:1/碎片/合并/缺失|Δstart p95|≤容差|覆盖率|噪音段|备注）──
    println!("\n<!-- 粘贴到 benchmark/{}.md 的「嵌字时间轴」表 -->", cfg.key);
    print_md_row(&[
        String::new(),
        String::new(),
        format!("{:.1}", sc.score),
        format!("{}/{}/{}/{}", sc.one_to_one, sc.fragmented, sc.merged, sc.missed),
        format!("{:.2}s", sc.dstart_p95),
        format!("{:.0}%", sc.within_tolerance * 100.0),
        format!("{:.0}%", sc.coverage * 100.0),
        format!("{}", sc.spurious),
        String::new(),
    ]);
}

#[test]
#[ignore]
fn bench_hardsub_moon_sisters() {
    run_case(&MOON_SISTERS);
}

#[test]
#[ignore]
fn bench_hardsub_glupov() {
    run_case(&GLUPOV);
}

#[test]
#[ignore]
fn bench_hardsub_pierro_questions() {
    run_case(&PIERRO_QUESTIONS);
}
