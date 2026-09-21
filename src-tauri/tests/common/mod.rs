//! 基准测试共享工具：案例配置、素材/环境探测（缺失优雅跳过）、参考文本解析、
//! 文本归一化与编辑距离、语料序列对齐、嵌字时间轴对齐与扣分制评分。
//!
//! 本模块被 bench_corpus / bench_hardsub 两个测试二进制共同编译，各自只用到
//! 其中一部分，故整体放行 dead_code 警告（已逐项确认无真正未被使用的代码）。
#![allow(dead_code)]
//!
//! 评测口径（与用户确认）：
//! - 语料基准：期望 = 参考文本全部条目；产物多余 = 噪音（仅统计不判败）；
//!   缺失/识别错误 = 流水线缺陷，按严重程度扣分。
//! - 嵌字基准：期望 = 参考文本内的时间轴；同理扣分；产出多余段仅统计；
//!   期望条目落在选区时间窗外（选区未覆盖）单独报告、不计缺陷。

use ai_game_subtitle_assistant_lib::ai_runtime::config::RuntimeConfig;
use ai_game_subtitle_assistant_lib::ai_runtime::OcrManager;
use ai_game_subtitle_assistant_lib::ocr::{run_ocr_pipeline, OcrRegionInput, OcrRunParams, OcrSegment};
use ai_game_subtitle_assistant_lib::video::get_video_metadata;
use std::path::{Path, PathBuf};
use std::time::Instant;

// ── 配对/评分阈值 ──

/// 序列对齐的最低配对相似度（宽松归一化）
pub const PAIR_MIN_SIM: f64 = 0.5;
/// 评分档位：≥0.98 正确；0.7~0.98 轻度错误；<0.7 严重错误
pub const SIM_CORRECT: f64 = 0.98;
pub const SIM_SEVERE: f64 = 0.7;

// ── 路径 ──

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn bench_data_dir() -> PathBuf {
    repo_root().join("examples").join("benchmark_examples")
}

// ── 案例配置 ──
// 选区坐标逐字取自 examples/benchmark_examples/ 下同名 .gsa 工程的
// ocr_region 控制轨（归一化 0..1）。.gsa 工程不入库（本地素材，仅作留档），
// 故本文件的硬编码即基准选区的受控副本——调整坐标须同步更新此处。

pub struct CaseCfg {
    pub key: &'static str,
    /// 参考文本文件名（自定义格式：HH:MM:SS:FF 时间码行 + 多行文本，空行分隔）
    pub ref_file: &'static str,
    /// 参考时间码的帧率基准（帧号 → 秒）
    pub ref_fps: f64,
    /// 语料片（剧情/游戏录屏，语料基准输入）
    pub corpus_video: &'static str,
    /// 测试片（主播实况切片，嵌字基准输入）
    pub clip_video: &'static str,
    /// 参考时间码线性校准系数：`t_video ≈ t_ref × (1 + ref_scale) + ref_offset`。
    ///
    /// 依据（2026-09-18，D5 校准节）：探针 `temp/probe/ref_calib.py` 逐条实测"视频里
    /// 字幕真正出现的时刻"（跨语言，靠 OCR 文本变化判定），稳健拟合得每案例的
    /// 缩放/截距。三案例参考时间轴都是视频时间轴的**线性缩放**（Δ≈a+k·t、残差 ±0.02s）：
    /// glupov k=0.709%/R²=0.99、pierro k=0.065%/R²=0.93、moon k≈0（无漂移）。
    /// 仅嵌字基准调用（语料基准按文本顺序比对，不用时间码）。
    pub ref_scale: f64,
    pub ref_offset: f64,
}

pub const MOON_SISTERS: CaseCfg = CaseCfg {
    key: "moon_sisters",
    ref_file: "quality_bench_test(voiced)_5min_reference.txt",
    // 素材实测 r_frame_rate=60/1（ffprobe）——参考时间码按 60fps 帧号书写（帧号最大 57）
    ref_fps: 60.0,
    corpus_video: "quality_bench_test_corpus(voiced)_5min.mp4",
    clip_video: "quality_bench_test(voiced)_5min.mp4",
    // 实测**无漂移**（k≈0、R²=0.0015，15 条）→ 不施加校准：给一份本来就准的参考做
    // 偏移校正没有依据（拟合出的 a=-0.075s 落在探针 ±0.25s 分辨率内，且部分来自
    // "对话行清空早于新句首字"的判定拍差，非参考自身偏置）
    ref_scale: 0.0,
    ref_offset: 0.0,
};

pub const GLUPOV: CaseCfg = CaseCfg {
    key: "glupov",
    ref_file: "quality_bench_test(non-voiced)_11min_reference.txt",
    ref_fps: 60000.0 / 1001.0,
    corpus_video: "quality_bench_test_corpus(non-voiced)_11min.mp4",
    clip_video: "quality_bench_test(non-voiced)_11min.mp4",
    // 18 条实测：k=+0.709%、a=-0.021s、R²=0.990（参考时间轴比视频快 0.7%）
    ref_scale: 0.007092,
    ref_offset: -0.021,
};

pub const PIERRO_QUESTIONS: CaseCfg = CaseCfg {
    key: "pierro_questions",
    ref_file: "quality_bench_test(voiced)_48min_reference.txt",
    ref_fps: 60000.0 / 1001.0,
    corpus_video: "quality_bench_test_corpus(voiced)_48min.mp4",
    clip_video: "quality_bench_test(voiced)_48min.mp4",
    // 117 条实测：k=+0.065%、a=-0.121s、R²=0.928
    ref_scale: 0.000645,
    ref_offset: -0.121,
};

/// 对参考时间码施加线性校准（见 `CaseCfg::ref_scale` 注释）。
/// 起止同乘同加：终点侧探针（`temp/probe/gt_probe_end.py`）独立实测的斜率与起点侧一致
/// （pierro 0.056% vs 0.054%、glupov 0.474% vs 0.525%），故两端共用同一变换。
pub fn apply_ref_calibration(refs: &mut [RefEntry], cfg: &CaseCfg) {
    if cfg.ref_scale == 0.0 && cfg.ref_offset == 0.0 {
        return;
    }
    for e in refs.iter_mut() {
        e.start = e.start * (1.0 + cfg.ref_scale) + cfg.ref_offset;
        e.end = e.end * (1.0 + cfg.ref_scale) + cfg.ref_offset;
    }
}

/// 语料页选区（page=corpus, video=source）
pub fn corpus_regions(key: &str) -> Vec<OcrRegionInput> {
    match key {
        "moon_sisters" => vec![
            OcrRegionInput { start: 37.53445753177658, end: 41.23095359134309, x1: 0.2, y1: 0.3964285714285714, x2: 0.8, y2: 0.5964285714285715 },
            OcrRegionInput { start: 43.23414776576048, end: 150.36933797398578, x1: 0.15479910714285716, y1: 0.7669642857142857, x2: 0.8477120535714286, y2: 0.9625 },
            OcrRegionInput { start: 153.4213611285289, end: 156.6089174844994, x1: 0.20502232142857144, y1: 0.3875, x2: 0.8050223214285714, y2: 0.5875 },
        ],
        "glupov" => vec![
            OcrRegionInput { start: 2.244384779128154, end: 30.971196990244636, x1: 0.1924665178571429, y1: 0.6784708057, x2: 0.792466517857143, y2: 0.8738084614 },
            OcrRegionInput { start: 46.19806263843079, end: 76.41020246274422, x1: 0.20251116071428577, y1: 0.6833312086, x2: 0.8025111607142857, y2: 0.8786688643 },
        ],
        "pierro_questions" => vec![
            OcrRegionInput { start: 0.0, end: 1272.3, x1: 0.16484375, y1: 0.7601340682, x2: 0.8326450892857143, y2: 0.9522546738 },
        ],
        _ => unreachable!("未知案例: {key}"),
    }
}

/// 嵌字选区（page=asr, video=clip）
pub fn hardsub_regions(key: &str) -> Vec<OcrRegionInput> {
    match key {
        "moon_sisters" => vec![
            OcrRegionInput { start: 1.6250005275973716, end: 6.498949253557576, x1: 0.2627790179, y1: 0.38303571428571426, x2: 0.8301339286, y2: 0.5830357142857143 },
            OcrRegionInput { start: 7.984131234376179, end: 147.97936019415172, x1: 0.2577566964, y1: 0.7446428571, x2: 0.8326450893, y2: 0.9357142857142856 },
            OcrRegionInput { start: 150.36933797398578, end: 185.13431614757792, x1: 0.2577566964, y1: 0.7446428571, x2: 0.8401785714, y2: 0.9178571429 },
            OcrRegionInput { start: 214.6356887998845, end: 262.4167854574098, x1: 0.2552455357, y1: 0.7401785714, x2: 0.8200892857, y2: 0.9401785714 },
            OcrRegionInput { start: 263.8815155779389, end: 270.4365261181527, x1: 0.2627790179, y1: 0.3875, x2: 0.8025111607142857, y2: 0.5875 },
        ],
        "glupov" => vec![
            OcrRegionInput { start: 69.19615992506358, end: 232.6610104886491, x1: 0.2627790179, y1: 0.7267857143, x2: 0.8301339286, y2: 0.9 },
            OcrRegionInput { start: 290.58328455542073, end: 588.3363037178857, x1: 0.2577566964, y1: 0.7401785714, x2: 0.8226004464, y2: 0.9178571429 },
        ],
        "pierro_questions" => vec![
            OcrRegionInput { start: 49.566388194397724, end: 2946.326729, x1: 0.2602678571, y1: 0.7401785714, x2: 0.8426897321, y2: 0.9267857143 },
        ],
        _ => unreachable!("未知案例: {key}"),
    }
}

pub fn default_ocr_params() -> OcrRunParams {
    OcrRunParams {
        // 2026-09-18：0.25s 试验后回退 0.5s。0.25s 的收益（嵌字 +2.3~+10）不足以抵消代价：
        // 语料回归（glupov 97.1→96.1，细网格更早采到打字机过渡帧）、耗时 +34%~+57%。
        // 合并层门限已改为绝对秒下限，与网格解耦，回退不改变合并语义。
        frame_interval: 0.5,
        dhash_threshold: 3,
        batch_size: 16,
        merge_similarity: 0.3,
    }
}

// ── 素材/环境探测（缺失时优雅跳过，不 panic）──

pub fn require_file(path: &Path, what: &str) -> bool {
    if path.is_file() {
        true
    } else {
        eprintln!("[跳过] 缺少{what}：{}", path.display());
        false
    }
}

/// 构造真实 OCR 环境（镜像 ocr_e2e.rs 模式）；未就绪返回 None。
pub fn build_ocr_manager() -> Option<OcrManager> {
    let runtime_dir = repo_root().join("runtime");
    if !runtime_dir.join("config.json").is_file() {
        eprintln!("[跳过] 缺少 runtime/config.json，请先跑 scripts/bootstrap_ocr.ps1");
        return None;
    }
    let config = match RuntimeConfig::load(&runtime_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[跳过] 读取 runtime 配置失败: {e}");
            return None;
        }
    };
    let manager = OcrManager::new(config, runtime_dir);
    if !manager.with_provider(|p| p.is_ready()) {
        eprintln!("[跳过] OCR 环境未就绪");
        return None;
    }
    Some(manager)
}

/// 跑一次完整 OCR 流水线，返回 (段列表, 耗时秒)。
///
/// 契约：**素材与运行环境的缺失由调用方提前判定并跳过**（`require_file` /
/// `build_ocr_manager`）。因此走到这里仍失败，就是流水线的真实故障——
/// 直接 panic 让基准测试失败，避免"回归被当成跳过"的假绿。
pub fn run_ocr(
    manager: &OcrManager,
    video: &Path,
    regions: &[OcrRegionInput],
) -> (Vec<OcrSegment>, f64) {
    let video_str = video.to_string_lossy().into_owned();
    let meta = get_video_metadata(video_str.clone())
        .unwrap_or_else(|e| panic!("读取视频元数据失败（{video_str}）: {e}"));
    let t0 = Instant::now();
    let segments = run_ocr_pipeline(
        manager,
        &video_str,
        meta.width,
        meta.height,
        regions,
        &default_ocr_params(),
        meta.fps,
        |_, _, _, _| {},
    )
    .unwrap_or_else(|e| panic!("OCR 流水线失败（{video_str}）: {e}"));
    (segments, t0.elapsed().as_secs_f64())
}

/// 基准主观评估产物：把 OCR 段写成 SRT 到 `temp/bench_output/<case>_<kind>.srt`。
///
/// 复用产品侧 `export::write_segments_srt`（同一 SRT 格式化与 UTF-8 BOM 约定），
/// 供导入剪辑软件、对照实况视频逐条目视——分数之外的定性判断。返回 (路径, 条数)。
pub fn write_bench_srt(
    case: &str,
    kind: &str,
    segments: &[OcrSegment],
) -> Result<(String, usize), String> {
    let dir = repo_root().join("temp").join("bench_output");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let path = dir.join(format!("{case}_{kind}.srt"));
    let path_str = path.to_string_lossy().into_owned();
    let n = ai_game_subtitle_assistant_lib::export::write_segments_srt(segments, &path_str)?;
    Ok((path_str, n))
}

/// 语料提取：移植 src/stores/project.ts pushCorpusTexts 语义
/// （trim → 丢空串 → 与既有+批内精确去重）。改装配规则须与前端同步（project.ts:803-820）。
pub fn corpus_from_segments(segments: &[OcrSegment]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for s in segments {
        let t = s.text.trim();
        if t.is_empty() || !seen.insert(t.to_string()) {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

// ── 参考文本解析 ──
// 格式：`HH:MM:SS:FF - HH:MM:SS:FF` 时间码行 + 多行文本块，空行分隔条目。
// 裸 \r（5min 参考的行内说话人粘连）先归一为 \n。

#[derive(Debug, Clone)]
pub struct RefEntry {
    pub start: f64,
    pub end: f64,
    /// 文本块各行以 \n 连接（保留说话人名行——OCR 选区同样拍到它们）
    pub text: String,
}

pub fn parse_timecode(s: &str, fps: f64) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    let f: f64 = parts[3].parse().ok()?;
    // 帧号必须落在 [0, fps)：越界说明参考文本的基准帧率与配置不符
    // （曾出现 ref_fps=30 而数据为 60fps 帧号 57 的静默算错）
    if !(0.0..fps).contains(&f) {
        return None;
    }
    Some(h * 3600.0 + m * 60.0 + sec + f / fps)
}

/// 疑似时间码行：含 ≥3 个冒号且含 `-`（合法时码为 `HH:MM:SS:FF`，分隔为 ` - `）。
/// 用于把"看起来是时间码但解析失败"的行判为真值数据错误，而不是静默并入上一条正文——
/// 覆盖帧号越界、缺少空格分隔（`…:FF-…`）、段数不足等形态。
fn looks_like_timecode_line(line: &str) -> bool {
    line.matches(':').count() >= 3 && line.contains('-')
}

fn split_timecode_line(line: &str, fps: f64) -> Option<(f64, f64)> {
    let (a, b) = line.split_once(" - ")?;
    Some((parse_timecode(a, fps)?, parse_timecode(b, fps)?))
}

pub fn parse_reference(path: &Path, fps: f64) -> Result<Vec<RefEntry>, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("读取参考文本失败: {e}"))?;
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut entries = Vec::new();
    let mut cur: Option<(f64, f64, Vec<String>)> = None;
    let flush = |cur: &mut Option<(f64, f64, Vec<String>)>, entries: &mut Vec<RefEntry>| {
        if let Some((s, e, lines)) = cur.take() {
            entries.push(RefEntry { start: s, end: e, text: lines.join("\n") });
        }
    };
    for (idx, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut cur, &mut entries);
        } else if let Some((s, e)) = split_timecode_line(t, fps) {
            flush(&mut cur, &mut entries);
            cur = Some((s, e, Vec::new()));
        } else if looks_like_timecode_line(t) {
            // 疑似时间码却解析失败（帧号越界 / 分隔或段数有误）→ 参考文本真值有问题，
            // 报错并附行号，而不是静默并入上一条正文
            return Err(format!(
                "第 {} 行：时间码行无效（帧号须小于基准帧率 {fps}，且分隔/段数须为 `HH:MM:SS:FF - HH:MM:SS:FF`，请核对 ref_fps 与参考文本）：{t}",
                idx + 1
            ));
        } else if let Some((_, _, lines)) = cur.as_mut() {
            lines.push(t.to_string());
        }
        // 时间码行之前出现正文（无条目头）→ 忽略
    }
    flush(&mut cur, &mut entries);
    Ok(entries)
}

// ── 文本归一化与编辑距离 ──

/// 严格归一化：仅去空白
pub fn normalize_strict(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 宽松归一化（主指标）：全角→半角折叠后仅保留字母数字（含 CJK/假名），
/// 即去除全部标点与空白——抹平 OCR 与人工参考的标点/全半角差异。
pub fn normalize_lenient(s: &str) -> String {
    s.chars()
        .map(|c| {
            if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            } else {
                c
            }
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// 编辑距离；超过 cap 时提前返回 cap+1（配对筛选用，保证 ≥PAIR_MIN_SIM 的
/// 配对距离精确，低于阈值的距离只需"知道它低于阈值"）。
pub fn levenshtein_capped(a: &[char], b: &[char], cap: usize) -> usize {
    if a.len().abs_diff(b.len()) > cap {
        return cap + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let v = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
            cur[j + 1] = v;
            row_min = row_min.min(v);
        }
        if row_min > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 宽松归一化相似度（0..1）。配对阈值筛选用：低于 PAIR_MIN_SIM 的返回值
/// 不保证精确，只保证 < PAIR_MIN_SIM。
pub fn similarity(a: &str, b: &str) -> f64 {
    let na: Vec<char> = normalize_lenient(a).chars().collect();
    let nb: Vec<char> = normalize_lenient(b).chars().collect();
    let maxlen = na.len().max(nb.len());
    if maxlen == 0 {
        return 1.0;
    }
    let cap = ((1.0 - PAIR_MIN_SIM) * maxlen as f64).ceil() as usize;
    let d = levenshtein_capped(&na, &nb, cap).min(maxlen);
    1.0 - d as f64 / maxlen as f64
}

/// 精确 CER（宽松归一化）：编辑距离 / 参考侧长度
pub fn exact_cer(reference: &str, produced: &str) -> f64 {
    let na: Vec<char> = normalize_lenient(reference).chars().collect();
    let nb: Vec<char> = normalize_lenient(produced).chars().collect();
    let reflen = na.len().max(1);
    levenshtein_capped(&na, &nb, usize::MAX) as f64 / reflen as f64
}

// ── 语料序列对齐（顺序保持的最大加权匹配，LCS 式 DP）──

#[derive(Debug, Clone, Copy)]
pub struct Pairing {
    pub exp: usize,
    pub prod: usize,
    pub sim: f64,
}

/// 返回 (配对列表[按期望顺序], 期望未配对下标, 产出多余下标)
pub fn align_sequences(expected: &[String], produced: &[String]) -> (Vec<Pairing>, Vec<usize>, Vec<usize>) {
    let n = expected.len();
    let m = produced.len();
    let mut sim = vec![vec![0.0f64; m]; n];
    for (i, e) in expected.iter().enumerate() {
        for (j, p) in produced.iter().enumerate() {
            sim[i][j] = similarity(e, p);
        }
    }
    // dp[i][j] = expected[..i] 与 produced[..j] 的最大 Σsim
    let mut dp = vec![vec![0.0f64; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            let mut best = dp[i - 1][j].max(dp[i][j - 1]);
            if sim[i - 1][j - 1] >= PAIR_MIN_SIM {
                best = best.max(dp[i - 1][j - 1] + sim[i - 1][j - 1]);
            }
            dp[i][j] = best;
        }
    }
    // 回溯
    let mut pairs = Vec::new();
    let mut exp_matched = vec![false; n];
    let mut prod_matched = vec![false; m];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let s = sim[i - 1][j - 1];
        if s >= PAIR_MIN_SIM && (dp[i][j] - (dp[i - 1][j - 1] + s)).abs() < 1e-9 {
            pairs.push(Pairing { exp: i - 1, prod: j - 1, sim: s });
            exp_matched[i - 1] = true;
            prod_matched[j - 1] = true;
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    let missed: Vec<usize> = (0..n).filter(|k| !exp_matched[*k]).collect();
    let extra: Vec<usize> = (0..m).filter(|k| !prod_matched[*k]).collect();
    (pairs, missed, extra)
}

// ── 嵌字时间轴对齐（重叠 ≥ 0.5×较短段时长；产出段至多归入一个期望条目）──

#[derive(Debug, Default)]
pub struct HardsubAlign {
    /// 1:1 配对 (期望下标, 产出下标)
    pub one_to_one: Vec<(usize, usize)>,
    /// 碎片化：一条期望被多条产出覆盖 (期望下标, 产出下标列表)
    pub fragmented: Vec<(usize, Vec<usize>)>,
    /// 被吞并：期望与邻近产出段重叠过半但产出已归属别的期望 (期望下标, 产出下标)
    pub merged: Vec<(usize, usize)>,
    /// 缺失：期望无任何产出覆盖
    pub missed: Vec<usize>,
    /// 噪音：产出未归入任何期望
    pub spurious: Vec<usize>,
}

pub fn align_temporal(expected: &[RefEntry], produced: &[OcrSegment]) -> HardsubAlign {
    let n = expected.len();
    let m = produced.len();
    // 候选 (score, exp, prod)：score = 重叠/较短段时长
    let mut cands: Vec<(f64, usize, usize)> = Vec::new();
    for (ei, e) in expected.iter().enumerate() {
        let ed = e.end - e.start;
        if ed <= 0.0 {
            continue;
        }
        for (pi, p) in produced.iter().enumerate() {
            let pd = p.end - p.start;
            if pd <= 0.0 {
                continue;
            }
            let ov = (e.end.min(p.end) - e.start.max(p.start)).max(0.0);
            let min_dur = ed.min(pd);
            if ov >= 0.5 * min_dur {
                cands.push((ov / min_dur, ei, pi));
            }
        }
    }
    // 贪心：分数降序；每个产出段只归入一个期望，期望可累积多个产出段（碎片化）
    cands.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap()
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    let mut prod_to: Vec<Option<usize>> = vec![None; m];
    let mut exp_parts: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(_, ei, pi) in &cands {
        if prod_to[pi].is_some() {
            continue;
        }
        prod_to[pi] = Some(ei);
        exp_parts[ei].push(pi);
    }
    let mut align = HardsubAlign::default();
    for (ei, parts) in exp_parts.into_iter().enumerate() {
        match parts.len() {
            0 => {
                // 无归属产出：被某条归属他处的产出段过半覆盖 → 判"被吞并"
                let mut merged_into = None;
                for (pi, p) in produced.iter().enumerate() {
                    let ed = expected[ei].end - expected[ei].start;
                    if ed <= 0.0 {
                        break;
                    }
                    let ov = (expected[ei].end.min(p.end) - expected[ei].start.max(p.start)).max(0.0);
                    if ov >= 0.5 * ed {
                        merged_into = Some(pi);
                        break;
                    }
                }
                match merged_into {
                    Some(pi) => align.merged.push((ei, pi)),
                    None => align.missed.push(ei),
                }
            }
            1 => align.one_to_one.push((ei, parts[0])),
            _ => {
                let mut parts = parts;
                parts.sort_by(|&a, &b| {
                    produced[a]
                        .start
                        .partial_cmp(&produced[b].start)
                        .unwrap()
                });
                align.fragmented.push((ei, parts));
            }
        }
    }
    align.spurious = (0..m).filter(|pi| prod_to[*pi].is_none()).collect();
    align
}

// ── 扣分制评分 ──

#[derive(Debug, Default)]
pub struct CorpusScore {
    pub score: f64,
    pub correct: usize,
    pub minor: usize,
    pub severe: usize,
    pub missing: usize,
    pub noise: usize,
    pub cer_mean: f64,
    pub cer_p95: f64,
    pub cer_max: f64,
}

/// 语料扣分：正确 0；轻度 (1−sim)；严重 1；缺失 1。
pub fn score_corpus(
    expected: &[String],
    produced: &[String],
    pairs: &[Pairing],
    n_missed: usize,
    n_extra: usize,
) -> CorpusScore {
    let mut ded = 0.0f64;
    let mut correct = 0;
    let mut minor = 0;
    let mut severe = 0;
    let mut cers = Vec::new();
    for p in pairs {
        if p.sim >= SIM_CORRECT {
            correct += 1;
        } else if p.sim >= SIM_SEVERE {
            minor += 1;
            ded += 1.0 - p.sim;
        } else {
            severe += 1;
            ded += 1.0;
        }
        cers.push(exact_cer(&expected[p.exp], &produced[p.prod]));
    }
    ded += n_missed as f64;
    let n = expected.len().max(1);
    CorpusScore {
        score: (100.0 - 100.0 * ded / n as f64).max(0.0),
        correct,
        minor,
        severe,
        missing: n_missed,
        noise: n_extra,
        cer_mean: cers.iter().sum::<f64>() / cers.len().max(1) as f64,
        cer_p95: percentile(cers.clone(), 0.95),
        cer_max: cers.iter().cloned().fold(0.0, f64::max),
    }
}

#[derive(Debug, Default)]
pub struct HardsubScore {
    pub score: f64,
    pub n_scored: usize,
    pub n_out_of_region: usize,
    pub one_to_one: usize,
    pub fragmented: usize,
    pub merged: usize,
    pub missed: usize,
    pub spurious: usize,
    pub dstart_p95: f64,
    pub dend_p95: f64,
    /// 1:1 配对中 d=max(|Δstart|,|Δend|) ≤ 容差的占比（0..1）
    pub within_tolerance: f64,
    /// 期望时长被产出覆盖的比例均值（0..1）
    pub coverage: f64,
    /// 1:1 配对文本相似度均值（仅报告，不计分）
    pub text_sim_mean: f64,

    // ── 结构轴（碎片化规模）——与时间轴并列报告，避免被复合评分掩盖 ──
    /// 碎片化多出的段数 Σ(N−1)（一条期望被拆成 N 段）
    pub extra_segments: usize,
    /// 碎片化段数直方图 [恰好 2 段, 恰好 3 段, ≥4 段]
    pub frag_hist: [usize; 3],

    // ── 时间轴（仅 1:1 配对；符号约定 Δ = 产出 − 参考，正 = 偏晚/过伸）──
    /// Δstart 带符号中位数（正 = 起点偏晚）
    pub dstart_median_signed: f64,
    /// Δend 带符号中位数（正 = 终点过伸）
    pub dend_median_signed: f64,
    /// max|Δstart|、max|Δend|（显式给出，避免小样本 p95=max 被误读为普遍现象）
    pub dstart_max: f64,
    pub dend_max: f64,
    /// 起点偏晚 / 偏早的配对数
    pub dstart_late: usize,
    pub dstart_early: usize,
    /// 终点过伸 / 欠伸的配对数
    pub dend_over: usize,
    pub dend_under: usize,
    /// |Δstart| ≤ 容差、|Δend| ≤ 容差 的占比（分轴达标率；d=max 会掩盖是哪一轴不达标）
    pub within_tol_start: f64,
    pub within_tol_end: f64,
}

/// 时间轴扣分（1:1 与碎片并集共用同一公式）：
/// d = max(|Δstart|,|Δend|)；d ≤ 0.5×容差 → 0；(0.5×容差, 容差] → 线性 0..1；> 容差 → 1.0
fn timing_penalty(start: f64, end: f64, ref_start: f64, ref_end: f64, tolerance: f64) -> f64 {
    let d = (ref_start - start).abs().max((ref_end - end).abs());
    let t_half = 0.5 * tolerance;
    if d <= t_half {
        0.0
    } else if d <= tolerance {
        (d - t_half) / (tolerance - t_half).max(1e-9)
    } else {
        1.0
    }
}

/// 嵌字扣分（按期望条目；d = max(|Δstart|,|Δend|)）：
/// d ≤ 0.5×容差 0；(0.5×容差, 容差] 线性扣 0..1；> 容差 扣 1；
/// 碎片化 (N−1)×0.5，并按**并集首尾补算时间分**（与 1:1 同公式）；被吞并 1；缺失 1。
///
/// 口径变更（2026-09-15，**破坏性**）：旧口径碎片条目完全豁免时间扣分（连 ② 时间轴
/// 也只报 1:1 配对），碎片被合并成完整段后才开始计时，造成"完整段反而低于碎片段"
/// 的反转（D9 实测 glupov 11.4→6.8）。并集相同时合并后扣分必不高于碎片时，
/// 恢复单调（合并永不亏）；负分可接受（缺陷求和口径，不封顶）。
/// 复合分历史数字不可跨此口径对比，见 review-reports/BENCH_HARDSUB_SCORE_UNION_TIMING.md。
pub fn score_hardsub(
    expected: &[RefEntry],
    produced: &[OcrSegment],
    align: &HardsubAlign,
    n_out_of_region: usize,
    tolerance: f64,
) -> HardsubScore {
    let mut total = 0.0f64;
    let mut dstarts = Vec::new();
    let mut dends = Vec::new();
    // 带符号偏差（Δ = 产出 − 参考）：正 = 偏晚/过伸，用于分辨"过伸"与"欠伸"
    let mut dsigned = Vec::new();
    let mut designed = Vec::new();
    let mut covered = Vec::new();
    let mut sims = Vec::new();
    for &(ei, pi) in &align.one_to_one {
        let e = &expected[ei];
        let p = &produced[pi];
        let ds = (e.start - p.start).abs();
        let de = (e.end - p.end).abs();
        total += timing_penalty(p.start, p.end, e.start, e.end, tolerance);
        dstarts.push(ds);
        dends.push(de);
        dsigned.push(p.start - e.start);
        designed.push(p.end - e.end);
        sims.push(similarity(&e.text, &p.text));
        let ed = (e.end - e.start).max(1e-9);
        let ov = (e.end.min(p.end) - e.start.max(p.start)).max(0.0);
        covered.push((ov / ed).min(1.0));
    }
    for (ei, parts) in &align.fragmented {
        // 碎片并集首尾补算时间分（与 1:1 同公式）：旧口径碎片完全豁免计时、
        // 合并成完整段后才开始计时 → "完整段反而低于碎片段"的反转；
        // 并集相同时合并后扣分必不高于碎片时，合并永不亏（单调）。
        let e = &expected[*ei];
        let union_start = parts
            .iter()
            .map(|&pi| produced[pi].start)
            .fold(f64::INFINITY, f64::min);
        let union_end = parts
            .iter()
            .map(|&pi| produced[pi].end)
            .fold(f64::NEG_INFINITY, f64::max);
        total += timing_penalty(union_start, union_end, e.start, e.end, tolerance);
        total += (parts.len() - 1) as f64 * 0.5;
        let ed = (e.end - e.start).max(1e-9);
        let mut cov = 0.0;
        for &pi in parts {
            let p = &produced[pi];
            cov += (e.end.min(p.end) - e.start.max(p.start)).max(0.0);
        }
        covered.push((cov / ed).min(1.0));
    }
    for _ in &align.merged {
        total += 1.0;
        covered.push(1.0);
    }
    for _ in &align.missed {
        total += 1.0;
        covered.push(0.0);
    }
    let n_scored = align.one_to_one.len() + align.fragmented.len() + align.merged.len() + align.missed.len();
    let within = if dstarts.is_empty() {
        0.0
    } else {
        let hit = dstarts
            .iter()
            .zip(dends.iter())
            .filter(|(ds, de)| ds.max(**de) <= tolerance)
            .count();
        hit as f64 / dstarts.len() as f64
    };

    // ── 结构轴统计 ──
    let mut extra_segments = 0usize;
    let mut frag_hist = [0usize; 3];
    for (_, parts) in &align.fragmented {
        extra_segments += parts.len() - 1;
        match parts.len() {
            2 => frag_hist[0] += 1,
            3 => frag_hist[1] += 1,
            _ => frag_hist[2] += 1,
        }
    }

    // ── 时间轴分轴统计（带符号方向 + 分轴达标率）──
    let signed_median = |v: &[f64]| -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        s[s.len() / 2]
    };
    let frac_within = |v: &[f64]| -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.iter().filter(|x| x.abs() <= tolerance).count() as f64 / v.len() as f64
    };
    let dstart_late = dsigned.iter().filter(|d| **d > 0.0).count();
    let dstart_early = dsigned.iter().filter(|d| **d < 0.0).count();
    let dend_over = designed.iter().filter(|d| **d > 0.0).count();
    let dend_under = designed.iter().filter(|d| **d < 0.0).count();

    // 先算极值再消费向量（percentile 取得所有权）
    let dstart_max = dstarts.iter().cloned().fold(0.0f64, f64::max);
    let dend_max = dends.iter().cloned().fold(0.0f64, f64::max);
    let dstart_p95 = percentile(dstarts, 0.95);
    let dend_p95 = percentile(dends, 0.95);

    HardsubScore {
        score: if n_scored == 0 {
            100.0
        } else {
            // 不封顶（2026-09-15 随碎片并集计时一并放开）：缺陷求和口径下
            // 单条目扣分可超 1（如碎片 时间 1.0 + 结构 0.5），负分如实反映
            // 计时/结构双差，且保住深度缺陷区的区分度（钳 0 会把 D9 前后
            // −25.0/−15.9 压成同值 0，单调性仍在但分辨率尽失）
            100.0 - 100.0 * total / n_scored as f64
        },
        n_scored,
        n_out_of_region,
        one_to_one: align.one_to_one.len(),
        fragmented: align.fragmented.len(),
        merged: align.merged.len(),
        missed: align.missed.len(),
        spurious: align.spurious.len(),
        dstart_p95,
        dend_p95,
        within_tolerance: within,
        coverage: covered.iter().sum::<f64>() / covered.len().max(1) as f64,
        text_sim_mean: sims.iter().sum::<f64>() / sims.len().max(1) as f64,
        extra_segments,
        frag_hist,
        dstart_median_signed: signed_median(&dsigned),
        dend_median_signed: signed_median(&designed),
        dstart_max,
        dend_max,
        dstart_late,
        dstart_early,
        dend_over,
        dend_under,
        within_tol_start: frac_within(&dsigned),
        within_tol_end: frac_within(&designed),
    }
}

pub fn percentile(mut v: Vec<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

/// 打印一行可粘贴的 markdown 表格数据行（前两列日期/commit 留给人工填写）
pub fn print_md_row(cells: &[String]) {
    println!("| {} |", cells.join(" | "));
}

// ── 自测：对齐与扣分逻辑的小样本断言 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timecode_frames() {
        let t = parse_timecode("00:01:09:11", 60000.0 / 1001.0).unwrap();
        assert!((t - (69.0 + 11.0 / (60000.0 / 1001.0))).abs() < 1e-9);
        let t = parse_timecode("00:00:36:16", 30.0).unwrap();
        assert!((t - 36.53333333).abs() < 1e-4);
        assert!(parse_timecode("bad", 30.0).is_none());
    }

    #[test]
    fn parse_rejects_frame_index_at_or_above_fps() {
        // 帧号必须 < 基准帧率：30fps 下 45 非法，60fps 下合法
        assert!(parse_timecode("00:00:01:45", 30.0).is_none());
        assert!(parse_timecode("00:00:01:45", 60.0).is_some());
        assert!(parse_timecode("00:00:01:30", 30.0).is_none()); // 边界：等于 fps 也非法

        // 参考文件中出现形态合法但帧号越界的行 → 解析报错，而非静默并入上一条正文
        let dir = std::env::temp_dir().join(format!("gsa_bench_tc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ref_tc.txt");
        std::fs::write(&path, "00:00:01:45 - 00:00:02:00\n台词\n\n").unwrap();
        let err = parse_reference(&path, 30.0).unwrap_err();
        assert!(err.contains("时间码行无效"), "实际: {err}");
        // 同数据在 60fps 基准下正常解析
        assert_eq!(parse_reference(&path, 60.0).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_rejects_malformed_timecode_lines() {
        // 各类"疑似时码但解析失败"的形态：都不允许静默并入上一条正文
        let cases = [
            "00:00:01:00-00:00:02:00",   // 缺空格分隔
            "00:00:01 - 00:00:02:00",    // 起始段数不足
            "00:00:01:45 - 00:00:02:00", // 帧号越界（30fps）
        ];
        let dir = std::env::temp_dir().join(format!("gsa_bench_bad_tc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (i, bad) in cases.iter().enumerate() {
            let path = dir.join(format!("ref_bad_{i}.txt"));
            std::fs::write(&path, format!("{bad}\n台词\n\n")).unwrap();
            let err = parse_reference(&path, 30.0).unwrap_err();
            assert!(
                err.contains("时间码行无效") && err.contains("第 1 行"),
                "case {bad} 实际: {err}"
            );
        }

        // 正常正文（冒号不足 3 个）不受影响
        let path = dir.join("ref_ok.txt");
        std::fs::write(&path, "00:00:01:00 - 00:00:02:00\n12:30 - 第 2 幕，A-B 路线\n\n").unwrap();
        let refs = parse_reference(&path, 30.0).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].text, "12:30 - 第 2 幕，A-B 路线");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_reference_multiline_and_bare_cr() {
        let dir = std::env::temp_dir().join(format!("gsa_bench_ut_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ref_ut.txt");
        // 含裸 \r（说话人粘连）、多行文本块、空行分隔
        std::fs::write(&path, "00:00:01:00 - 00:00:02:00\r\n甲\r乙台词一\r\n\r\n00:00:03:00 - 00:00:04:00\r\n台词二\r\n\r\n").unwrap();
        let refs = parse_reference(&path, 30.0).unwrap();
        assert_eq!(refs.len(), 2);
        assert!((refs[0].start - 1.0).abs() < 1e-9);
        assert!((refs[0].end - 2.0).abs() < 1e-9);
        assert_eq!(refs[0].text, "甲\n乙台词一");
        assert_eq!(refs[1].text, "台词二");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normalize_levels() {
        assert_eq!(normalize_strict("派蒙： 你好"), "派蒙：你好");
        // 宽松：全角折叠 + 去标点（「」[]！：。全被剔除，全角字母折叠）
        assert_eq!(normalize_lenient("「编玛瑙]必须不断，磨练！"), "编玛瑙必须不断磨练");
        assert_eq!(normalize_lenient("ＡＢＣ１２３"), "ABC123");
    }

    #[test]
    fn levenshtein_basics() {
        let a: Vec<char> = "kitten".chars().collect();
        let b: Vec<char> = "sitting".chars().collect();
        assert_eq!(levenshtein_capped(&a, &b, 10), 3);
        assert_eq!(levenshtein_capped(&a, &b, 2), 3); // 超 cap 提前返回 cap+1
        assert_eq!(levenshtein_capped(&[], &[], 0), 0);
    }

    #[test]
    fn align_sequences_matches_and_extras() {
        let expected = vec!["ABCDEFGH".to_string(), "XYZ".to_string()];
        let produced = vec!["ABCDE".to_string(), "XYZQ".to_string()];
        let (pairs, missed, extra) = align_sequences(&expected, &produced);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].exp, 0);
        assert!(pairs[0].sim >= PAIR_MIN_SIM);
        assert!(missed.is_empty());
        assert!(extra.is_empty());

        // 一条期望被拆成两半：只配对最好的一半，另一半计为噪音
        let expected = vec!["ABCDEFGH".to_string()];
        let produced = vec!["ABCD".to_string(), "EFGH".to_string()];
        let (pairs, missed, extra) = align_sequences(&expected, &produced);
        assert_eq!(pairs.len(), 1);
        assert!(missed.is_empty());
        assert_eq!(extra.len(), 1);
    }

    #[test]
    fn align_temporal_classes() {
        let e = |s: f64, en: f64, t: &str| RefEntry { start: s, end: en, text: t.to_string() };
        let p = |s: f64, en: f64| OcrSegment { start: s, end: en, text: "t".into(), confidence: 1.0 };

        // 碎片化：一条期望被两条产出覆盖
        let expected = vec![e(0.0, 10.0, "a")];
        let produced = vec![p(0.0, 5.0), p(5.0, 10.0)];
        let al = align_temporal(&expected, &produced);
        assert_eq!(al.fragmented.len(), 1);
        assert_eq!(al.fragmented[0].1.len(), 2);

        // 被吞并：一条产出跨两条期望
        let expected = vec![e(0.0, 10.0, "a"), e(10.0, 20.0, "b")];
        let produced = vec![p(0.0, 20.0)];
        let al = align_temporal(&expected, &produced);
        assert_eq!(al.merged.len(), 1, "{al:?}");

        // 缺失与噪音
        let expected = vec![e(0.0, 10.0, "a")];
        let produced = vec![p(100.0, 110.0)];
        let al = align_temporal(&expected, &produced);
        assert_eq!(al.missed.len(), 1);
        assert_eq!(al.spurious.len(), 1);

        // 1:1
        let expected = vec![e(0.0, 10.0, "a")];
        let produced = vec![p(0.2, 10.1)];
        let al = align_temporal(&expected, &produced);
        assert_eq!(al.one_to_one.len(), 1);
    }

    #[test]
    fn scoring_deductions() {
        // 语料：3 条期望，1 正确 + 1 轻度("BBBX" vs "BBBB" sim 0.75 → 扣 0.25) + 1 缺失(扣 1)
        // → 100 − 125/3 ≈ 58.33
        let expected = vec!["AAAA".to_string(), "BBBB".to_string(), "CCCC".to_string()];
        let produced = vec!["AAAA".to_string(), "BBBX".to_string()];
        let (pairs, missed, extra) = align_sequences(&expected, &produced);
        assert_eq!(missed.len(), 1);
        let sc = score_corpus(&expected, &produced, &pairs, missed.len(), extra.len());
        assert_eq!(sc.correct, 1);
        assert_eq!(sc.minor, 1);
        assert_eq!(sc.missing, 1);
        assert!((sc.score - 175.0 / 3.0).abs() < 1e-6, "score={}", sc.score);

        // 嵌字：2 条期望均 1:1，一条 Δ=0.6s（容差 1 → 扣 0.2），一条精确
        let e = |s: f64, en: f64| RefEntry { start: s, end: en, text: "t".to_string() };
        let expected = vec![e(10.0, 12.0), e(20.0, 22.0)];
        let produced = vec![
            OcrSegment { start: 10.6, end: 12.0, text: "t".into(), confidence: 1.0 },
            OcrSegment { start: 20.0, end: 22.0, text: "t".into(), confidence: 1.0 },
        ];
        let align = align_temporal(&expected, &produced);
        assert_eq!(align.one_to_one.len(), 2);
        let sc = score_hardsub(&expected, &produced, &align, 0, 1.0);
        assert!((sc.score - 90.0).abs() < 1e-6, "score={}", sc.score);
        // d=0.6s 在容差 1.0s 内（进入线性扣分段但仍属"容差内"）→ 两条都算
        assert!((sc.within_tolerance - 1.0).abs() < 1e-6);
    }

    /// 分轴诊断：结构轴（碎片规模）+ 时间轴（带符号方向、分轴达标率）
    #[test]
    fn hardsub_axes_diagnostics() {
        let e = |s: f64, en: f64, t: &str| RefEntry { start: s, end: en, text: t.to_string() };
        let p = |s: f64, en: f64, t: &str| OcrSegment { start: s, end: en, text: t.into(), confidence: 1.0 };

        let expected = vec![e(10.0, 20.0, "aaa"), e(30.0, 40.0, "bbb"), e(50.0, 60.0, "ccc")];
        // e0：起点偏晚 +0.6、终点准确；e1：被拆成两段（多出 1 段）；e2：起点偏晚 +2.0、终点欠伸 −1.0
        let produced = vec![
            p(10.6, 20.0, "aaa"),
            p(30.0, 34.0, "bbb"),
            p(34.0, 40.5, "bbb"),
            p(52.0, 59.0, "ccc"),
        ];
        let align = align_temporal(&expected, &produced);
        assert_eq!(align.one_to_one.len(), 2);
        assert_eq!(align.fragmented.len(), 1);
        assert_eq!(align.fragmented[0].1.len(), 2);

        let sc = score_hardsub(&expected, &produced, &align, 0, 0.3);

        // 结构轴
        assert_eq!(sc.extra_segments, 1, "被拆成 2 段 → 多出 1 段");
        assert_eq!(sc.frag_hist, [1, 0, 0], "恰好 2 段的分组 1 个");

        // 时间轴（仅 1:1：e0-p0 与 e2-p3）
        assert_eq!(sc.dstart_late, 2, "两对起点都偏晚");
        assert_eq!(sc.dstart_early, 0);
        assert_eq!(sc.dend_over, 0);
        assert_eq!(sc.dend_under, 1, "一对终点欠伸");
        assert!((sc.dend_median_signed - 0.0).abs() < 1e-9, "Δend 中位 {:+}", sc.dend_median_signed);
        assert!((sc.dstart_max - 2.0).abs() < 1e-9);
        assert!((sc.dend_max - 1.0).abs() < 1e-9);
        // 分轴达标率：起点 0/2（0.6 与 2.0 均超 0.3）；终点 1/2（0.0 达标，−1.0 不达标）
        assert!((sc.within_tol_start - 0.0).abs() < 1e-9, "start {}", sc.within_tol_start);
        assert!((sc.within_tol_end - 0.5).abs() < 1e-9, "end {}", sc.within_tol_end);
        // 复合达标率：两对的 d 都不 ≤0.3
        assert!((sc.within_tolerance - 0.0).abs() < 1e-9);
    }

    /// 评分口径（2026-09-15 破坏性变更）：碎片条目按并集首尾补算时间分——
    /// 同一条目合并后扣分必不高于碎片时（合并永不亏），消除"完整段反而低分"反转。
    #[test]
    fn hardsub_fragmented_union_timing_monotonic() {
        let e = |s: f64, en: f64, t: &str| RefEntry { start: s, end: en, text: t.to_string() };
        let p = |s: f64, en: f64, t: &str| OcrSegment { start: s, end: en, text: t.into(), confidence: 1.0 };
        let expected = vec![e(10.0, 20.0, "a")];

        // 干净拆分（并集 10..20 无偏差）：时间 0 + 结构 0.5 → 50.0；合并成一段 → 100.0
        let frag = vec![p(10.0, 15.0, "a"), p(15.0, 20.0, "a")];
        let af = align_temporal(&expected, &frag);
        assert_eq!(af.fragmented.len(), 1);
        let sf = score_hardsub(&expected, &frag, &af, 0, 0.3);
        assert!((sf.score - 50.0).abs() < 1e-6, "碎片: {}", sf.score);

        let merged = vec![p(10.0, 20.0, "a")];
        let am = align_temporal(&expected, &merged);
        assert_eq!(am.one_to_one.len(), 1);
        let sm = score_hardsub(&expected, &merged, &am, 0, 0.3);
        assert!((sm.score - 100.0).abs() < 1e-6, "合并: {}", sm.score);
        assert!(sm.score >= sf.score, "合并不得低于碎片时");

        // 并集偏晚 0.6s（> 容差 0.3）：碎片 = 时间 1.0 + 结构 0.5 → −50.0；合并 = 时间 1.0 → 0.0
        let frag_late = vec![p(10.6, 15.0, "a"), p(15.0, 20.0, "a")];
        let afl = align_temporal(&expected, &frag_late);
        assert_eq!(afl.fragmented.len(), 1);
        let sfl = score_hardsub(&expected, &frag_late, &afl, 0, 0.3);
        assert!((sfl.score - (-50.0)).abs() < 1e-6, "碎片(偏晚): {}", sfl.score);

        let merged_late = vec![p(10.6, 20.0, "a")];
        let aml = align_temporal(&expected, &merged_late);
        assert_eq!(aml.one_to_one.len(), 1);
        let sml = score_hardsub(&expected, &merged_late, &aml, 0, 0.3);
        assert!((sml.score - 0.0).abs() < 1e-6, "合并(偏晚): {}", sml.score);
        assert!(sml.score >= sfl.score, "偏晚形态下合并同样不得低于碎片时");

        // 多段碎片（3 段干净拆分）：结构 1.0 → 0.0；不封顶口径允许负分
        let frag3 = vec![p(10.0, 13.0, "a"), p(13.0, 16.0, "a"), p(16.0, 20.0, "a")];
        let af3 = align_temporal(&expected, &frag3);
        assert_eq!(af3.fragmented.len(), 1);
        let sf3 = score_hardsub(&expected, &frag3, &af3, 0, 0.3);
        assert!((sf3.score - 0.0).abs() < 1e-6, "3 段碎片: {}", sf3.score);
    }
}
