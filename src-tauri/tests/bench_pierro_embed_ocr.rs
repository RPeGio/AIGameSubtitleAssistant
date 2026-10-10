//! 为 pierro_questions.gsa 补跑"嵌字 OCR"转写侧（T1 的前置工序）。
//!
//! 背景：pierro 工程已就绪的是**语料侧**（148 条 corpus）与**选区**，但切片视频的
//! `embed_ocr` 产物轨从未跑过（该工程只有 ocr_region×2 + ocr_text(mock)），故
//! `fuse_offline_validate.py` 拿到 0 段转写侧、无法验证规模化。
//!
//! 本测试**复用产品同一条链路**：
//!   · OCR 编排：`ocr::run_ocr_pipeline`（与命令 `run_ocr` 同一入口）
//!   · 视频元数据：`video::get_video_metadata`（与前端 invoke 同一命令函数）
//!   · 参数：`src/composables/ocrDefaults.ts::createDefaultOcrParams` 的产品默认值
//!   · 选区：`.gsa` 中 page=asr（嵌字）的 ocr_region 事件
//! 不重写任何 OCR 逻辑；产物落盘 JSON 供 Python 侧离线验证消费。
//!
//! 运行：
//!   cargo test --release --test bench_pierro_embed_ocr -- --ignored --nocapture
//! 环境变量：
//!   GSA_PIERRO_MAX_SEC  截断处理的时间跨度（秒；未设 = 全量 ~49min），用于先小段试跑
//!   GSA_PIERRO_REGION   "asr"（默认，嵌字）| "corpus"（剧情录屏选区）
//!   GSA_PIERRO_VIDEO    切片视频本地路径（入库 `.gsa` 的素材路径已清空；
//!                       未设时回退基准素材约定名，本机 `.gsa` 有记录值时也可直接用）
//!   GSA_BENCH_OUT_DIR   产物目录（默认 <repo>/temp/bench_output）

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::OcrManager;
use ai_game_subtitle_assistant_lib::ocr::{run_ocr_pipeline, OcrRegionInput, OcrRunParams, PunctuationNorm};
use ai_game_subtitle_assistant_lib::video::get_video_metadata;
use std::path::PathBuf;
use std::time::Instant;

/// 仓库根 = src-tauri 的上一级
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// 读 .gsa（首行魔数 + JSON）
fn load_gsa(path: &std::path::Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path).expect("读取 .gsa 失败");
    let json = raw.split_once('\n').map(|(_, b)| b).unwrap_or(&raw);
    serde_json::from_str(json).expect(".gsa JSON 解析失败")
}

/// 产品默认 OCR 参数（与 src/composables/ocrDefaults.ts 同源）
fn default_params() -> OcrRunParams {
    OcrRunParams {
        frame_interval: 0.5,
        dhash_threshold: 3,
        batch_size: 16,
        merge_similarity: 0.3,
        min_subtitle_sec: 1.5,
        punctuation: PunctuationNorm::default(),
        glossary: Vec::new(),
        consistency_hints: false,
    }
}

#[test]
#[ignore]
fn run_pierro_embed_ocr() {
    let root = repo_root();
    let runtime_dir = root.join("runtime");
    assert!(runtime_dir.join("config.json").is_file(), "缺少 runtime/config.json");

    let gsa = root.join("examples/benchmark_examples/pierro_questions.gsa");
    assert!(gsa.is_file(), "缺少 pierro_questions.gsa");
    let proj = load_gsa(&gsa);

    let want_page = std::env::var("GSA_PIERRO_REGION").unwrap_or_else(|_| "asr".to_string());
    // 切片视频定位（P1-1 修复）：入库 `.gsa` 已把素材路径字段清空（避免提交本机绝对路径），
    // 故优先级 = 环境变量 `GSA_PIERRO_VIDEO` > `.gsa` 记录值（本机未清空时仍可用）>
    // 基准素材约定名（bench_data_dir 下的 CaseCfg 命名）。
    let video = {
        let from_gsa = proj["video"].as_str().unwrap_or("").trim().to_string();
        std::env::var("GSA_PIERRO_VIDEO")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| (!from_gsa.is_empty()).then_some(from_gsa))
            .unwrap_or_else(|| {
                root.join("examples/benchmark_examples/quality_bench_test(voiced)_48min.mp4")
                    .to_string_lossy()
                    .into_owned()
            })
    };
    assert!(
        PathBuf::from(&video).is_file(),
        "切片视频不存在: {video}（可设环境变量 GSA_PIERRO_VIDEO 指定本地素材）"
    );

    // 选区：仅取目标 page 的 ocr_region（片段内嵌字用 page=asr）
    let mut clips: Vec<OcrRegionInput> = Vec::new();
    for t in proj["tracks"].as_array().expect("tracks 非数组") {
        if t["type"] != "ocr_region" {
            continue;
        }
        let page = t["page"].as_str().unwrap_or("");
        let name = t["name"].as_str().unwrap_or("");
        let tvid = t["video"].as_str().unwrap_or("");
        if page != want_page {
            println!("  跳过选区轨（page={page:?}, video={tvid:?}）: {name}");
            continue;
        }
        println!("  采用选区轨: {name} (page={page}, video={tvid})");
        for e in t["events"].as_array().expect("events 非数组") {
            let (s, en) = (e["start"].as_f64().unwrap_or(0.0), e["end"].as_f64().unwrap_or(0.0));
            println!(
                "    选区 [{s:.2}s, {en:.2}s] x=[{:.3},{:.3}] y=[{:.3},{:.3}]",
                e["x1"].as_f64().unwrap_or(0.0),
                e["x2"].as_f64().unwrap_or(0.0),
                e["y1"].as_f64().unwrap_or(0.0),
                e["y2"].as_f64().unwrap_or(0.0)
            );
            clips.push(OcrRegionInput {
                start: s,
                end: en,
                x1: e["x1"].as_f64().unwrap_or(0.0),
                y1: e["y1"].as_f64().unwrap_or(0.0),
                x2: e["x2"].as_f64().unwrap_or(0.0),
                y2: e["y2"].as_f64().unwrap_or(0.0),
            });
        }
    }
    clips.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    assert!(!clips.is_empty(), "未找到 page={want_page} 的 OCR 选区");
    println!("切片视频: {video}");

    // 可选：截断时间跨度用于先小段试跑
    let max_sec: f64 = std::env::var("GSA_PIERRO_MAX_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    if max_sec > 0.0 {
        for c in clips.iter_mut() {
            if c.start > max_sec {
                c.end = c.start; // 空区间 → 无帧
            } else if c.end > max_sec {
                c.end = max_sec;
            }
        }
        println!("【试跑模式】时间跨度截到 {max_sec:.1}s");
    }

    // 视频元数据：直接调产品同一命令函数（非 async，测试可直呼）
    let meta = get_video_metadata(video.clone()).expect("获取视频元数据失败");
    println!("视频元数据: {}x{} @ {:.3}fps, {:.1}s", meta.width, meta.height, meta.fps, meta.duration);

    let config = RuntimeConfig::load(&runtime_dir).expect("读取 runtime 配置失败");
    let manager = OcrManager::new(config, runtime_dir);
    let st = manager.status();
    assert!(st.ready, "OCR 运行环境未就绪: {}", st.message);

    let params = default_params();
    println!(
        "OCR 参数（产品默认）: frame_interval={} dhash={} batch={} merge_sim={} min_subtitle={}",
        params.frame_interval, params.dhash_threshold, params.batch_size,
        params.merge_similarity, params.min_subtitle_sec
    );

    let t0 = Instant::now();
    let (segments, diffs) = run_ocr_pipeline(
        &manager,
        &video,
        meta.width,
        meta.height,
        &clips,
        &params,
        meta.fps,
        |clip_index, clip_count, progress, message| {
            println!("  [{}/{}] {:>5.1}% {}", clip_index + 1, clip_count, progress * 100.0, message);
        },
    )
    .expect("OCR 流水线失败");
    let elapsed = t0.elapsed().as_secs_f64();

    println!("\n========== 嵌字 OCR 结果 ==========");
    println!("耗时 {:.1}s；产出 {} 段；待审批 Diff {} 条", elapsed, segments.len(), diffs.len());
    for (i, s) in segments.iter().enumerate() {
        println!(
            "{:3}. [{:8.2} → {:8.2}] ({:5.2}s) conf={:.2} {}",
            i + 1, s.start, s.end, s.end - s.start, s.confidence,
            s.text.replace('\n', "⏎")
        );
    }
    if !diffs.is_empty() {
        println!("\n待审批纠错（只标记不改写）:");
        for d in &diffs {
            println!("  {} → {}  ({} 处)", d.old.join("/"), d.new, d.old.len());
        }
    }
    println!("==================================");

    // 落盘：与 .gsa 的 embed_ocr 事件同形态，供 Python 侧离线验证消费
    let out_dir = std::env::var("GSA_BENCH_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("temp/bench_output"));
    std::fs::create_dir_all(&out_dir).expect("创建产物目录失败");
    let events: Vec<serde_json::Value> = segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            serde_json::json!({
                "type": "embed_ocr",
                "id": format!("pierro-embed-{i:04}"),
                "start": s.start,
                "end": s.end,
                "text": s.text,
                "confidence": s.confidence,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "source": "pierro_questions.gsa",
        "video": video,
        "region_page": want_page,
        "max_sec": max_sec,
        "elapsed_sec": elapsed,
        "params": {
            "frame_interval": params.frame_interval,
            "dhash_threshold": params.dhash_threshold,
            "batch_size": params.batch_size,
            "merge_similarity": params.merge_similarity,
            "min_subtitle_sec": params.min_subtitle_sec,
        },
        "segments": segments.iter().map(|s| serde_json::json!({
            "start": s.start, "end": s.end, "text": s.text, "confidence": s.confidence
        })).collect::<Vec<_>>(),
        "embed_ocr_events": events,
        "diffs": diffs.iter().map(|d| serde_json::json!({
            "old": d.old, "new": d.new,
        })).collect::<Vec<_>>(),
    });
    let out = out_dir.join(if max_sec > 0.0 {
        "pierro_embed_ocr_trial.json"
    } else {
        "pierro_embed_ocr.json"
    });
    std::fs::write(&out, serde_json::to_string_pretty(&payload).unwrap()).expect("写产物失败");
    println!("已落盘 {}", out.display());
}
