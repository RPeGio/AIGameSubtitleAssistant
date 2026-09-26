//! 基准②：嵌字时间轴（OCR 切片视频内嵌字幕）评测。
//!
//! 判定标准（与用户确认）：期望 = 参考文本内的时间轴；同理扣分：
//! d = max(|Δstart|,|Δend|)，d ≤ 0.5×容差 0 分；(0.5×容差, 容差] 线性扣 0..1；
//! > 容差 扣 1；碎片化（1:N）扣 (N−1)×0.5；被吞并/缺失扣 1。
//! 产出多余段 = 噪音（仅统计）；期望条目落在选区时间窗外（选区未覆盖）
//! 单独报告、不计缺陷。文本相似度仅报告，不参与评分。
//!
//! 运行：cargo test --release --test bench_hardsub -- --ignored --nocapture --test-threads=1
//! 容差可用环境变量 GSA_BENCH_TOLERANCE_SEC 覆盖（默认 0.3s——1s 级偏差对字幕
//! 生产已是严重偏离，只应作为严重缺陷出现）。
//! 跑完把打印的 markdown 行粘到 benchmark/<案例>.md 的「嵌字时间轴」表。

mod common;

use common::{
    align_temporal, apply_ref_calibration, bench_data_dir, build_ocr_manager, hardsub_regions,
    print_md_row, run_ocr, score_hardsub, require_file, CaseCfg, GLUPOV, MOON_SISTERS,
    PIERRO_QUESTIONS,
};

fn tolerance_sec() -> f64 {
    // 容差与采样量子自洽：帧网格 0.5s 的段边界量化误差天然为 ±0.25s，旧 0.3s 容差比量子
    // 还小——把量化噪声当缺陷扣分。2026-09-18 先取 0.5s（=2×量化），后按用户决策提到
    // **0.6s**（=2.4×量化，口径上更自洽；实测三案例 65.6 / 60.5 / 71.4 全过 60）。
    // env GSA_BENCH_TOLERANCE_SEC 可覆盖。
    std::env::var("GSA_BENCH_TOLERANCE_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.6)
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

    let mut refs = match common::parse_reference(&ref_path, cfg.ref_fps) {
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
    // D5 校准：参考时间轴是视频时间轴的线性缩放（逐条实测后稳健拟合，见 CaseCfg 注释）
    apply_ref_calibration(&mut refs, cfg);
    // 排除计分条目（素材侧缺陷，如 pierro 的人工 transition——用户主观评审确认）
    let dropped = common::drop_excluded_refs(&mut refs, cfg);
    if !dropped.is_empty() {
        println!("排除计分条目 {} 条（素材侧缺陷，不计缺陷）：{}", dropped.len(), dropped.join("、"));
    }
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

    let (segments, elapsed) = run_ocr(&manager, &video, &regions);

    // 主观评估产物：把嵌字段写成 SRT（复用产品侧 format_srt + BOM），
    // 供导入剪辑软件、对照实况切片视频逐条目视（分数之外的定性判断）
    match common::write_bench_srt(cfg.key, "hardsub", &segments) {
        Ok((path, n)) => println!("主观评估产物（SRT，可直接导入剪辑软件）：{path}（{n} 条）"),
        Err(e) => eprintln!("[警告] SRT 导出失败：{e}"),
    }

    let align = align_temporal(&refs, &segments);
    let tol = tolerance_sec();
    let sc = score_hardsub(&refs, &segments, &align, out_of_region, tol);

    // ── 终端摘要：结构轴 / 时间轴 / 复合评分 三轴并列 ──
    // 读法：先看 ① 结构轴与 ② 时间轴，再看 ③ 复合评分。2026-09-15 口径变更后
    // 碎片条目按并集首尾补算时间分（与 1:1 同公式），同一期望条目合并后扣分
    // 必不高于碎片时（合并永不亏）；复合分历史数字不可跨口径对比（破坏性变更，
    // 见 review-reports/BENCH_HARDSUB_SCORE_UNION_TIMING.md）。
    println!(
        "耗时 {:.1}s｜OCR 段 {}（参考 {} 条，选区外 {} 条，容差 {:.2}s）",
        elapsed,
        segments.len(),
        refs.len(),
        sc.n_out_of_region,
        tol
    );
    println!("── ① 结构轴 ──");
    println!(
        "1:1 {}｜碎片 {} 条（多出 {} 段：2段 {} / 3段 {} / ≥4段 {}）｜被吞并 {}｜缺失 {}｜噪音段 {}",
        sc.one_to_one,
        sc.fragmented,
        sc.extra_segments,
        sc.frag_hist[0],
        sc.frag_hist[1],
        sc.frag_hist[2],
        sc.merged,
        sc.missed,
        sc.spurious
    );
    println!(
        "── ② 时间轴（仅 1:1 配对，n={}；Δ = 产出 − 参考，+ 表示偏晚/过伸）──",
        sc.one_to_one
    );
    println!(
        "Δstart：中位 {:+.3}s｜p95(|·|) {:.3}s｜最大(|·|) {:.3}s｜偏晚 {} / 偏早 {}｜容差内 {:.0}%",
        sc.dstart_median_signed,
        sc.dstart_p95,
        sc.dstart_max,
        sc.dstart_late,
        sc.dstart_early,
        sc.within_tol_start * 100.0
    );
    println!(
        "Δend  ：中位 {:+.3}s｜p95(|·|) {:.3}s｜最大(|·|) {:.3}s｜过伸 {} / 欠伸 {}｜容差内 {:.0}%",
        sc.dend_median_signed,
        sc.dend_p95,
        sc.dend_max,
        sc.dend_over,
        sc.dend_under,
        sc.within_tol_end * 100.0
    );
    println!(
        "复合 d=max(|Δstart|,|Δend|)：容差内 {:.0}%｜覆盖率 {:.0}%｜文本相似度均值 {:.3}",
        sc.within_tolerance * 100.0,
        sc.coverage * 100.0,
        sc.text_sim_mean
    );
    println!("── ③ 复合评分（碎片并集计时口径，2026-09-15 破坏性变更）──");
    println!("{:.1}/100", sc.score);
    if !align.missed.is_empty() {
        println!("── 缺失条目（时间轴）──");
        for &i in &align.missed {
            println!("  [{}] {:7.2} → {:7.2}  {}", i + 1, refs[i].start, refs[i].end, refs[i].text.replace('\n', " / "));
        }
    }
    // 碎片明细：计数之外的定性信息（哪条参考被拆、拆成什么样），用于定位合并层缺口
    // （D1/D12 类问题必须看"参考一行 vs 产出多行"的形态才能判定成因）
    if !align.fragmented.is_empty() {
        println!("── 碎片条目明细（1:N）──");
        for (ei, parts) in &align.fragmented {
            let e = &refs[*ei];
            println!(
                "  [{}] {:7.2} → {:7.2}  1→{} 段｜参考：{}",
                ei + 1,
                e.start,
                e.end,
                parts.len(),
                e.text.replace('\n', " / ")
            );
            for &pi in parts {
                let p = &segments[pi];
                println!(
                    "        产出 {:7.2} → {:7.2}  conf={:.2}  {}",
                    p.start,
                    p.end,
                    p.confidence,
                    p.text.replace('\n', " / ")
                );
            }
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
