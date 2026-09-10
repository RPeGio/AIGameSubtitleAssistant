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
}

pub const MOON_SISTERS: CaseCfg = CaseCfg {
    key: "moon_sisters",
    ref_file: "quality_bench_test(voiced)_5min_reference.txt",
    // 素材实测 r_frame_rate=60/1（ffprobe）——参考时间码按 60fps 帧号书写（帧号最大 57）
    ref_fps: 60.0,
    corpus_video: "quality_bench_test_corpus(voiced)_5min.mp4",
    clip_video: "quality_bench_test(voiced)_5min.mp4",
};

pub const GLUPOV: CaseCfg = CaseCfg {
    key: "glupov",
    ref_file: "quality_bench_test(non-voiced)_11min_reference.txt",
    ref_fps: 60000.0 / 1001.0,
    corpus_video: "quality_bench_test_corpus(non-voiced)_11min.mp4",
    clip_video: "quality_bench_test(non-voiced)_11min.mp4",
};

pub const PIERRO_QUESTIONS: CaseCfg = CaseCfg {
    key: "pierro_questions",
    ref_file: "quality_bench_test(voiced)_48min_reference.txt",
    ref_fps: 60000.0 / 1001.0,
    corpus_video: "quality_bench_test_corpus(voiced)_48min.mp4",
    clip_video: "quality_bench_test(voiced)_48min.mp4",
};

/// 语料页选区（page=corpus, video=source）
pub fn corpus_regions(key: &str) -> Vec<OcrRegionInput> {
    match key {
        "moon_sisters" => vec![
            OcrRegionInput { start: 37.53445753177658, end: 41.23095359134309, x1: 0.2, y1: 0.3964285714285714, x2: 0.8, y2: 0.5964285714285715 },
            OcrRegionInput { start: 43.23414776576048, end: 150.36933797398578, x1: 0.15479910714285716, y1: 0.7267857142857143, x2: 0.8477120535714286, y2: 0.9625 },
            OcrRegionInput { start: 153.4213611285289, end: 156.6089174844994, x1: 0.20502232142857144, y1: 0.3875, x2: 0.8050223214285714, y2: 0.5875 },
        ],
        "glupov" => vec![
            OcrRegionInput { start: 2.244384779128154, end: 30.971196990244636, x1: 0.1924665178571429, y1: 0.6598214285714286, x2: 0.792466517857143, y2: 0.8598214285714286 },
            OcrRegionInput { start: 46.19806263843079, end: 76.41020246274422, x1: 0.20251116071428577, y1: 0.6553571428571429, x2: 0.8025111607142857, y2: 0.8553571428571429 },
        ],
        "pierro_questions" => vec![
            OcrRegionInput { start: 0.0, end: 1272.3, x1: 0.2, y1: 0.7, x2: 0.8, y2: 0.86875 },
        ],
        _ => unreachable!("未知案例: {key}"),
    }
}

/// 嵌字选区（page=asr, video=clip）
pub fn hardsub_regions(key: &str) -> Vec<OcrRegionInput> {
    match key {
        "moon_sisters" => vec![
            OcrRegionInput { start: 1.6250005275973716, end: 6.498949253557576, x1: 0.2728236607142857, y1: 0.38303571428571426, x2: 0.8, y2: 0.5830357142857143 },
            OcrRegionInput { start: 7.984131234376179, end: 147.97936019415172, x1: 0.27784598214285716, y1: 0.713392857142857, x2: 0.8050223214285714, y2: 0.9357142857142856 },
            OcrRegionInput { start: 150.36933797398578, end: 185.13431614757792, x1: 0.2, y1: 0.7, x2: 0.8, y2: 0.9 },
            OcrRegionInput { start: 214.6356887998845, end: 262.4167854574098, x1: 0.26529017857142856, y1: 0.7, x2: 0.8, y2: 0.9401785714285714 },
            OcrRegionInput { start: 263.8815155779389, end: 270.4365261181527, x1: 0.27282366071428577, y1: 0.3875, x2: 0.8025111607142857, y2: 0.5875 },
        ],
        "glupov" => vec![
            OcrRegionInput { start: 69.19615992506358, end: 232.6610104886491, x1: 0.27533482142857146, y1: 0.7401785714285714, x2: 0.8301339285714286, y2: 0.9 },
            OcrRegionInput { start: 290.58328455542073, end: 588.3363037178857, x1: 0.2, y1: 0.7, x2: 0.8, y2: 0.9 },
        ],
        "pierro_questions" => vec![
            OcrRegionInput { start: 49.566388194397724, end: 2946.326729, x1: 0.27533482142857146, y1: 0.7267857142857143, x2: 0.810044642857143, y2: 0.9401785714285714 },
        ],
        _ => unreachable!("未知案例: {key}"),
    }
}

pub fn default_ocr_params() -> OcrRunParams {
    OcrRunParams {
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

/// 跑一次完整 OCR 流水线；失败时 eprintln 并返回 None。
pub fn run_ocr(
    manager: &OcrManager,
    video: &Path,
    regions: &[OcrRegionInput],
) -> Option<(Vec<OcrSegment>, f64)> {
    let meta = match get_video_metadata(video.to_string_lossy().into_owned()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[跳过] 读取视频元数据失败: {e}");
            return None;
        }
    };
    let t0 = Instant::now();
    match run_ocr_pipeline(
        manager,
        &video.to_string_lossy(),
        meta.width,
        meta.height,
        regions,
        &default_ocr_params(),
        meta.fps,
        |_, _, _, _| {},
    ) {
        Ok(segments) => Some((segments, t0.elapsed().as_secs_f64())),
        Err(e) => {
            eprintln!("[跳过] OCR 流水线失败: {e}");
            None
        }
    }
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

/// 时间码行形态检测：`N:N:N:N - N:N:N:N`（各段为纯数字，不校验帧率合法性）
fn looks_like_timecode_line(line: &str) -> bool {
    let Some((a, b)) = line.split_once(" - ") else {
        return false;
    };
    let is_tc = |s: &str| {
        let parts: Vec<&str> = s.trim().split(':').collect();
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    };
    is_tc(a) && is_tc(b)
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
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            flush(&mut cur, &mut entries);
        } else if let Some((s, e)) = split_timecode_line(t, fps) {
            flush(&mut cur, &mut entries);
            cur = Some((s, e, Vec::new()));
        } else if looks_like_timecode_line(t) {
            // 形态是时间码却解析失败（帧号越界等）→ 参考文本真值有问题，报错而非静默并入正文
            return Err(format!(
                "时间码行无效（帧号须小于基准帧率 {fps}，请核对 ref_fps 与参考文本）：{t}"
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
}

/// 嵌字扣分（按期望条目；d = max(|Δstart|,|Δend|)）：
/// d ≤ 0.5×容差 0；(0.5×容差, 容差] 线性扣 0..1；> 容差 扣 1；
/// 碎片化 (N−1)×0.5；被吞并 1；缺失 1。
pub fn score_hardsub(
    expected: &[RefEntry],
    produced: &[OcrSegment],
    align: &HardsubAlign,
    n_out_of_region: usize,
    tolerance: f64,
) -> HardsubScore {
    let t_half = 0.5 * tolerance;
    let mut total = 0.0f64;
    let mut dstarts = Vec::new();
    let mut dends = Vec::new();
    let mut covered = Vec::new();
    let mut sims = Vec::new();
    for &(ei, pi) in &align.one_to_one {
        let e = &expected[ei];
        let p = &produced[pi];
        let ds = (e.start - p.start).abs();
        let de = (e.end - p.end).abs();
        let d = ds.max(de);
        total += if d <= t_half {
            0.0
        } else if d <= tolerance {
            (d - t_half) / (tolerance - t_half).max(1e-9)
        } else {
            1.0
        };
        dstarts.push(ds);
        dends.push(de);
        sims.push(similarity(&e.text, &p.text));
        let ed = (e.end - e.start).max(1e-9);
        let ov = (e.end.min(p.end) - e.start.max(p.start)).max(0.0);
        covered.push((ov / ed).min(1.0));
    }
    for (ei, parts) in &align.fragmented {
        total += (parts.len() - 1) as f64 * 0.5;
        let e = &expected[*ei];
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
    HardsubScore {
        score: if n_scored == 0 {
            100.0
        } else {
            (100.0 - 100.0 * total / n_scored as f64).max(0.0)
        },
        n_scored,
        n_out_of_region,
        one_to_one: align.one_to_one.len(),
        fragmented: align.fragmented.len(),
        merged: align.merged.len(),
        missed: align.missed.len(),
        spurious: align.spurious.len(),
        dstart_p95: percentile(dstarts, 0.95),
        dend_p95: percentile(dends, 0.95),
        within_tolerance: within,
        coverage: covered.iter().sum::<f64>() / covered.len().max(1) as f64,
        text_sim_mean: sims.iter().sum::<f64>() / sims.len().max(1) as f64,
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
}
