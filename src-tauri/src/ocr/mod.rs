// ─── OCR 字幕生成流水线 ────────────────────────────────────
// 纯函数部分：
//   ocr_pass    —— 只 OCR 变化帧，未变化帧顺延文本，产出每帧的 FrameText
//   merge_frames —— 连续相同文本合并成 OcrSegment（事件）
// 编排命令：
//   run_ocr     —— 串联 扫描→抽帧→窗口精化→OCR→合并，后台线程跑并上报进度
//
// 依赖：
//   video::scan_frame_hashes     → Vec<(time, dhash)>（零落盘变化检测，网格+精化共用）
//   video::extract_frames_bytes  → mjpeg 管道内存帧（仅变化帧保留字节，零落盘）
//   2.4 change_flags             → Vec<(is_changed, base_hash)>
//   2.2 OcrProvider              → 批量识别（images 元素 = base64 JPEG，IPC 不经磁盘）

use crate::ai_runtime::dhash::FrameChange;
use crate::ai_runtime::{OcrError, OcrManager, OcrResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// OCR 进度事件名（前端 `OCR_PROGRESS_EVENT` 需保持一致）
pub const OCR_PROGRESS_EVENT: &str = "ocr-progress";

/// 合并后的一段字幕事件（不含 id，写轨道时由 store 分配）
#[derive(Debug, Clone, Serialize)]
pub struct OcrSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f64,
}

/// 一帧的文本（顺延后的完整序列）。
/// text 用 `Arc<str>` 共享，未变化帧只做引用计数递增，不逐帧拷贝字符串。
/// `sample_time` = 该帧的网格采样时刻（图像的实际采样点）；帧级差分注入的
/// 时刻覆写只改 `time`（有效边界），采样覆盖仍以 `sample_time` 计。
#[derive(Debug, Clone)]
pub struct FrameText {
    pub time: f64,
    pub sample_time: f64,
    pub text: Arc<str>,
    pub confidence: f64,
}

/// JPEG 字节 → base64（OCR IPC 的图像载荷格式）
fn b64(data: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD as B64_ENGINE;
    use base64::Engine as _;
    B64_ENGINE.encode(data)
}

/// 只 OCR 变化帧，未变化帧顺延上一次 OCR 的文本。
///
/// 契约：`changes` 必须按帧时间升序；`jpegs` 与 `changes` 中 is_changed 帧
/// 按出现顺序一一对齐。
///
/// - 变化帧的 JPEG 字节 base64 编码后按顺序批量交给 `recognize`（每次调用只处理一批），
///   worker 在内存中解码为 ndarray，全程不经磁盘
/// - 传 `recognize` 闭包而非 provider 引用，是为了让调用方把锁的持有范围缩小到单次批量调用
/// - 文本在入口处 `trim` 归一化（空检测与后续合并语义保持一致）
/// - 若返回结果数量与请求不符，视为 worker 错误并终止（不让部分失败悄悄污染字幕）
/// - `on_progress` 每处理一批回调一次：(已处理帧数, 总帧数)
pub fn ocr_pass(
    changes: &[FrameChange],
    jpegs: &[Vec<u8>],
    mut recognize: impl FnMut(&[String]) -> Result<Vec<OcrResult>, OcrError>,
    batch_size: usize,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Vec<FrameText>, OcrError> {
    let bs = batch_size.max(1);
    let total = changes.len();
    let changed_total = changes.iter().filter(|c| c.is_changed).count();
    if jpegs.len() != changed_total {
        return Err(OcrError::Worker(format!(
            "图像数据与变化帧数量不符：{} 变化帧，{} 张 JPEG",
            changed_total,
            jpegs.len()
        )));
    }
    let mut jpeg_iter = jpegs.iter();
    let mut texts = Vec::with_capacity(total);
    // 当前顺延的 (text, confidence)；Arc 共享避免未变化帧反复分配
    let mut current: Option<(Arc<str>, f64)> = None;

    for batch in changes.chunks(bs) {
        // 变化帧在此批内的索引
        let changed: Vec<usize> = batch
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_changed)
            .map(|(i, _)| i)
            .collect();
        let images: Vec<String> = changed
            .iter()
            .map(|_| {
                let jpeg = jpeg_iter
                    .next()
                    .ok_or_else(|| OcrError::Worker("图像数据少于变化帧".into()))?;
                Ok(b64(jpeg))
            })
            .collect::<Result<Vec<String>, OcrError>>()?;

        let results = if images.is_empty() {
            Vec::new()
        } else {
            recognize(&images)?
        };

        // 数量校验：短结果不能静默吞掉，避免顺延状态被污染
        if results.len() != changed.len() {
            return Err(OcrError::Worker(format!(
                "OCR 结果数量不匹配：请求 {} 帧，返回 {} 帧",
                changed.len(),
                results.len()
            )));
        }

        let mut res_iter = results.into_iter();
        for change in batch.iter() {
            if change.is_changed {
                let r = res_iter
                    .next()
                    .ok_or_else(|| OcrError::Worker("OCR 结果缺失".into()))?;
                // trim 归一化：保证空检测与合并时的一致性
                current = Some((Arc::from(r.text.trim()), r.confidence));
            }
            let (text, confidence) = match &current {
                Some((t, c)) => (Arc::clone(t), *c),
                None => (Arc::from(""), 0.0),
            };
            texts.push(FrameText {
                time: change.time,
                sample_time: change.sample_time,
                text,
                confidence,
            });
        }
        on_progress(texts.len(), total);
    }
    Ok(texts)
}

/// 把变化标志与 mjpeg 流实际帧装配成 ocr_pass 的输入（changes 与 jpegs 严格对齐）。
///
/// 背景：网格帧数按 ceil(时长/间隔) 估算，而 ffmpeg fps 滤镜实际输出按 round 舍入，
/// 实测 dur=5s、interval=0.7s 时估算 8 帧、实际输出 7 帧。多出的"幻影"网格帧
/// （号 ≥ total）在 extract_frames_bytes 中无实际帧匹配、被静默丢弃。
///
/// 装配规则：
/// - 实际帧（0..total）直接装配；`kept_jpegs` 必须与其中变化帧一一对应（显式校验，
///   违反即报错而非静默错位）
/// - 幻影网格帧若判为变化，说明片段尾部确有字幕切换：经 `recover_frame` 按网格时间
///   补抽单帧；补抽失败则放弃该帧（changes/jpegs 同步跳过，对齐不变）
fn assemble_grid_frames(
    flags: &[(bool, Option<u64>)],
    kept_jpegs: Vec<Vec<u8>>,
    total: usize,
    start: f64,
    interval: f64,
    mut recover_frame: impl FnMut(f64) -> Option<Vec<u8>>,
) -> Result<(Vec<FrameChange>, Vec<Vec<u8>>), OcrError> {
    // 对齐校验：kept 即全部 <total 的变化帧字节（extract_frames_bytes 按 keep 升序保留）
    let changed_below = (0..total.min(flags.len())).filter(|&k| flags[k].0).count();
    if kept_jpegs.len() != changed_below {
        return Err(OcrError::Worker(format!(
            "网格帧装配错位：{} 个变化帧，{} 张 JPEG",
            changed_below,
            kept_jpegs.len()
        )));
    }
    let mut changes: Vec<FrameChange> = (0..total)
        .map(|k| {
            let (is_changed, base_hash) = flags.get(k).copied().unwrap_or((false, None));
            let time = start + (k as f64) * interval;
            FrameChange {
                time,
                sample_time: time,
                is_changed,
                base_hash,
            }
        })
        .collect();
    let mut jpegs = kept_jpegs;
    // 幻影网格帧回收：补抽的帧号大于所有实际帧，追加在末尾保持升序对齐
    for (k, &(is_changed, base_hash)) in flags.iter().enumerate().skip(total) {
        if !is_changed {
            continue;
        }
        let time = start + (k as f64) * interval;
        if let Some(jpeg) = recover_frame(time) {
            changes.push(FrameChange {
                time,
                sample_time: time,
                is_changed: true,
                base_hash,
            });
            jpegs.push(jpeg);
        }
    }
    Ok((changes, jpegs))
}

/// 计算两个字符串的归一化编辑距离比例（0=相同，1=完全不同）
fn edit_distance_ratio(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let (m, n) = (ac.len(), bc.len());
    if m == 0 && n == 0 {
        return 0.0;
    }
    let max_len = m.max(n) as f64;
    if max_len == 0.0 {
        return 1.0;
    }
    // 一维 DP 求莱文斯坦距离
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = if ac[i - 1] == bc[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n] as f64 / max_len
}

/// 行边界切断：较短文本的各行与较长文本**同序号行**归一化相等，且较长文本多出后续行。
///
/// 纯文本上，这两种语义**形态相同**，必须靠 `is_stable_line_boundary_cut` 的时长门区分：
/// - **独立新字幕**：姓名框/头衔态稳定显示数秒后，下一条对话行整行出现。
///   实测 glupov 嵌字：`Anton / Former Acting Captain,"Ninth Company"`（≈40 字符）
///   稳定 3.95s 后出现 `No news. But perhaps...`（参考亦把二者记为两条，Δe 曾达 +10.40s）。
/// - **同一条目延续**：打字机/换行续写，下一行在 <1.5s 内接上。
///   实测 glupov 语料：`…第九连队的编制保住`（3 行）→ 0.47s 后补出换行第二行
///   `了。我们没有让祖先与陛下蒙羞。`（4 行），属同一条字幕。
///
/// 注意：英文嵌字的姓名框+长头衔占比 ≈0.44 > 1/3，会顶破 `similar_text` 的 3× 前缀规则。
fn is_line_boundary_cut(short: &str, long: &str) -> bool {
    let s: Vec<String> = short
        .lines()
        .map(|l| norm_chars(l).into_iter().collect())
        .collect();
    let l: Vec<String> = long
        .lines()
        .map(|l| norm_chars(l).into_iter().collect())
        .collect();
    if s.is_empty() || l.len() <= s.len() {
        return false;
    }
    if !s
        .iter()
        .zip(l.iter())
        .all(|(a, b)| edit_distance_ratio(a, b) <= LINE_PREFIX_EDIT_TOLERANCE)
    {
        return false;
    }
    // 多出的行须是**新字幕的对话行**，而非换行余尾：以短态末行长度为尺度取 1/4 下限
    // （新行常刚从打字机冒头，实测 glupov 首帧仅 `No news. But perha`＝14 字符 / 末行 31
    // → 0.45 ✓；moon 换行余尾 `moon.`＝4 字符 / 末行 78 → 0.05 ✗ 不判切断，保持同一条目
    // ——moon 参考把换行续写并入同一条，如 #16 [235.77 → 252.18]）。
    let new_line_len = l[s.len()].chars().count();
    let short_last_len = s[s.len() - 1].chars().count();
    new_line_len * 4 >= short_last_len
}

/// 行边界切断判为"独立新字幕"所需的最短稳定时长（秒）。
///
/// 取 **2.0s**（> D10 保险丝 1.5s）——实测两侧兼容：
/// - glupov 嵌字姓名框态稳定 **3.95s**（#18）与 **≈2.0s**（#21）→ 参考记为独立条目；
/// - moon 嵌字姓名框态仅 **1.50s** → moon 参考把它并入对话行条目
///   （`[20.50 → 33.95]` 条目文本就含姓名行），故不拆。
const LINE_BOUNDARY_STABLE_SEC: f64 = 2.0;

/// 触发行边界切断所需的**新文本帧**最低 OCR 置信度。
///
/// 低置信度帧多出的"一行"更可能是识别抖动而非真实新字幕：实测 moon 一帧
/// `conf=0.64` 的乱码把已稳定 12.7s 的段落顶开成两段（1:1 18→15、碎片 1→4）。
const LINE_BOUNDARY_MIN_CONF: f64 = 0.8;

/// 稳定时长门 + 置信度门 + 行边界切断：三者同时成立才判为独立新字幕。
fn is_stable_line_boundary_cut(
    short: &str,
    long: &str,
    short_dur: f64,
    long_conf: f64,
) -> bool {
    long_conf >= LINE_BOUNDARY_MIN_CONF
        && short_dur >= LINE_BOUNDARY_STABLE_SEC
        && is_line_boundary_cut(short, long)
}

/// 判断两段文本是否属于同一条字幕的延续
///
/// - 完全相等：是
/// - 前缀互相覆盖（打字机/渐进文本）：是
/// - 编辑距离比例 ≤ 阈值（OCR 抖动/半句）：是
///
/// 注：行边界切断（姓名框态 → 下一条）**不在此处判定**——它需要短态持续时间，
/// 由调用方用 `is_stable_line_boundary_cut` 拦截（`merge_frames` 的 run 分组与
/// `merge_similar_adjacent` 各自持有时间信息）。
fn similar_text(a: &str, b: &str, threshold: f64) -> bool {
    if a == b {
        return true;
    }
    // 前缀互相覆盖（打字机/渐进文本）：仅当较短文本足够长（≥ 较长文本 1/3）时，
    // 避免把"派蒙"与"派蒙：旅行者你来了"这类独立两行误合并（约占 1/4 的前缀）。
    //
    // 判据走**归一化**字符（去空白/标点/大小写），与 `is_line_prefix`、
    // `is_line_boundary_cut`、`norm_chars` 系判据保持一致：实测嵌字里
    // `Onyx Agate` 与 `OnyxAgate`（少一个空格）曾因此判为"非前缀"而漏并
    // （glupov 4→5、8→9；pierro 4→5、17→18、21→22 共 5 处碎片）。
    let (short, long) = if a.chars().count() <= b.chars().count() {
        (a, b)
    } else {
        (b, a)
    };
    let sn = norm_chars(short);
    let ln = norm_chars(long);
    let short_len = sn.len();
    if short_len > 0 && ln.starts_with(&sn) && short_len * 3 >= ln.len() {
        return true;
    }
    edit_distance_ratio(a, b) <= threshold
}

/// 判定"同一文本"的近似阈值（OCR 抖动：全半角括号、标点差异等）
const SAME_TEXT_TOLERANCE: f64 = 0.1;

/// 渐进补全偏好：更长（更完整）候选的承载门槛。
///
/// 两道通道之一即可：
/// 1. **≥2 帧承载**——打字机补全通常会被后续网格/保险丝多次采到；
/// 2. **是 run 的末态**（与 `texts.last()` 近似同文）——显示期的最终形态即该行完整文本；
///    实测 glupov 语料 `…能力显得更关键。` 因 9×8 dHash 对尾部亚阈值变化不敏感而漏检，
///    仅由 1.5s 保险丝在段尾补采到 1 帧，走此通道才认得出。
///
/// 单帧幻觉尾巴（`test_vote_ignores_noise_long_text` 的乱码行）虽可能是末态，但会
/// 被下面的置信度门挡掉。
const PROGRESSIVE_COMPLETE_MIN_SUPPORT: usize = 2;

/// 渐进补全偏好：更长候选的平均置信度不得低于 medoid 胜者超过此幅度。
///
/// 守门依据（2026-09-18 语料实测）：**幻觉尾巴帧的 OCR 置信度显著更低**——
/// moon 语料 `卡侬 / oo / 桑娜妲。…` 的承载帧 conf 0.65/0.70、`…也没什么意 / 见。o`
/// 的承载帧 conf 0.50/0.63；而真实渐进补全（glupov `…所有事务，任务瞬间复杂了起来。`）
/// 的完整态 conf 0.86 反而**高于**残缺态 0.80。故以"置信度不劣于胜者"为第二道门，
/// 避免"更长者优先"把 D4 污染文本翻案。
const PROGRESSIVE_COMPLETE_CONF_MARGIN: f64 = 0.10;

/// 渐进补全偏好：更长候选相对 medoid 胜者的**最小归一化字符增量**。
///
/// 取值依据（2026-09-18 语料实测）：真实打字机补全的增量远大于此——
/// `…所有事务，任务` → `…所有事务，任务瞬间复杂了起来。` 增量 9 字符、
/// `…任务排期的能` → `…任务排期的能力显得更关键。` 增量 6 字符；
/// 而 OCR 抖动变体（全半角括号、个别字错读）增量仅 0~1 字符。
/// 0.5s 网格 A/B 实测：无此门时把一条原本"正确"（sim≥0.98）的条目换成了略偏的长变体
/// （语料 97.1→96.9）；加门后不误换。
const PROGRESSIVE_COMPLETE_MIN_GAIN: usize = 3;

/// 从 run 内所有相似帧文本做多数投票：选与其它帧总相似度最高者（medoid）。
///
/// 替代"更长者胜出"：被噪声污染的更长文本与多数帧差异大，不会被选中；
/// 完全相同的候选平局时取更长（与旧行为兼容）。
///
/// **渐进补全偏好（D10，2026-09-18）**：medoid 在"打字机渐进链" P1 ⊂ P2 ⊂ P3 上
/// **数学上必然落在中间态**（中间态到各态的平均距离最小），于是 run 内明明采到了
/// 完整态，胜出的却是残缺文本。0.5s 网格下链短、恰好常落在完整态；网格降到 0.25s
/// 后链变长，glupov 语料因此出现两条截断配对（sim 0.778/0.811，扣 0.41 = 全部回归）。
/// 故在 medoid 之后补一步：若存在"严格更长、近似包含 medoid 胜者、且被 ≥2 帧承载"
/// 的候选，取其中最长者——即该行显示期间的**最终完整态**（语料/翻译所需的形态）。
fn vote_text(texts: &[(Arc<str>, f64)]) -> (Arc<str>, f64) {
    let mut best: &(Arc<str>, f64) = &texts[0];
    let mut best_score = f64::MIN;
    for cand in texts {
        let score: f64 = texts
            .iter()
            .map(|(t, _)| 1.0 - edit_distance_ratio(&cand.0, t))
            .sum();
        let is_longer = cand.0.chars().count() > best.0.chars().count();
        if score > best_score + 1e-9 || ((score - best_score).abs() <= 1e-9 && is_longer) {
            best_score = score;
            best = cand;
        }
    }
    // 渐进补全偏好：medoid 之后再看是否存在"更完整的同一句"
    let bn = norm_chars(&best.0);
    if !bn.is_empty() {
        let winner_conf = best.1;
        let mut winner: Option<&(Arc<str>, f64)> = None;
        for cand in texts {
            let cn = norm_chars(&cand.0);
            if cn.len() < bn.len() + PROGRESSIVE_COMPLETE_MIN_GAIN || !is_subsequence(&bn, &cn) {
                continue;
            }
            let support = texts
                .iter()
                .filter(|(t, _)| edit_distance_ratio(t, &cand.0) <= SAME_TEXT_TOLERANCE)
                .count();
            // 通道 2：候选是 run 末态（显示期最终形态）
            let is_final_state = texts
                .last()
                .is_some_and(|(t, _)| edit_distance_ratio(t, &cand.0) <= SAME_TEXT_TOLERANCE);
            if support < PROGRESSIVE_COMPLETE_MIN_SUPPORT && !is_final_state {
                continue;
            }
            // 置信度门：候选帧自身的置信度不得明显低于胜者帧（幻觉尾巴实测低 0.15~0.48）
            if cand.1 + PROGRESSIVE_COMPLETE_CONF_MARGIN < winner_conf {
                continue;
            }
            let longer = winner.map_or(true, |w| norm_chars(&w.0).len() < cn.len());
            if longer {
                winner = Some(cand);
            }
        }
        if let Some(w) = winner {
            return (w.0.clone(), w.1);
        }
    }
    (best.0.clone(), best.1)
}

/// 把连续相同（或高度相似）文本的帧合并成字幕事件。
///
/// 契约：`frames` 必须按 time 升序，且文本已 trim 归一化（由 `ocr_pass` 保证）。
///
/// - 空文本是边界：不产生事件，且打断 run（连续空帧短于容错窗口时视为抖动不打断）
/// - 相似文本（`merge_similarity`）视为同一句的过渡帧：合并，flush 时多数投票取最终文本
/// - 事件 end = 该段最后一帧的覆盖中点（`last + interval×0.5`，把段尾量化误差的
///   期望压到 0 —— 旧值 `last + interval` 系统性 +0.5×interval，超 0.3s 容差），
///   并对 clip 结尾截断
/// - 整段落在 clip 之外（start >= clip_end）时丢弃，不产生越界时间
/// - confidence 取投票胜出文本对应帧的置信度
pub fn merge_frames(
    frames: Vec<FrameText>,
    interval: f64,
    clip_end: f64,
    merge_similarity: f64,
) -> Vec<OcrSegment> {
    // 空帧容错窗口：连续空帧短于此值视为 OCR 抖动，不打断 run
    let empty_gap = (interval * 1.5).max(0.8);

    struct Run {
        start: f64,
        /// run 内所有相似帧的 (文本, 置信度)；flush 时多数投票取最终文本
        texts: Vec<(Arc<str>, f64)>,
        last: f64,
        /// 当前连续空帧的起点时间（容错窗口内不打断 run）
        empty_since: Option<f64>,
    }

    fn flush(segments: &mut Vec<OcrSegment>, run: &Run, interval: f64, clip_end: f64) {
        // 整段在 clip 之外：丢弃，避免零长度/越界时间
        if run.start >= clip_end {
            return;
        }
        let end = (run.last + interval * 0.5).min(clip_end).max(run.start);
        let (text, confidence) = vote_text(&run.texts);
        segments.push(OcrSegment {
            start: run.start,
            end,
            text: text.to_string(),
            confidence,
        });
    }

    let mut segments = Vec::new();
    let mut run: Option<Run> = None;

    for f in frames {
        if f.text.is_empty() {
            // 空帧容错：累计空帧时长 < empty_gap 时视为 OCR 抖动，run 继续
            if let Some(r) = run.as_mut() {
                let es = *r.empty_since.get_or_insert(f.time);
                if f.time - es < empty_gap {
                    continue;
                }
            }
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            continue;
        }

        // 相似基准 = run 内最近一帧文本（渐进时是超集，比较稳定）
        let is_same = matches!(&run, Some(r) if {
            let base: &str = r.texts.last().map(|(t, _)| t.as_ref()).unwrap_or("");
            // 稳定 ≥2.0s 的姓名框/头衔态之后出现整行新文本 = 独立新字幕（D11），run 在此断开
            let stable_boundary = is_stable_line_boundary_cut(
                base,
                f.text.as_ref(),
                f.time - r.start,
                f.confidence,
            );
            !stable_boundary && similar_text(base, f.text.as_ref(), merge_similarity)
        });
        if is_same {
            if let Some(r) = run.as_mut() {
                // 覆盖推进用采样时刻：帧级差分注入帧的有效时刻早于采样时刻，
                // 段尾按采样覆盖计算（否则注入帧的段尾会提前，破坏下游拼接）
                r.last = f.sample_time;
                r.empty_since = None;
                r.texts.push((f.text, f.confidence));
            }
        } else {
            if let Some(r) = run.take() {
                flush(&mut segments, &r, interval, clip_end);
            }
            run = Some(Run {
                start: f.time,
                texts: vec![(f.text, f.confidence)],
                last: f.sample_time,
                empty_since: None,
            });
        }
    }
    if let Some(r) = run.take() {
        flush(&mut segments, &r, interval, clip_end);
    }
    segments
}

/// 归一化：小写 + 仅留 Unicode 字母数字（含 CJK），剔除空白与标点
/// —— 用于段间"包含关系"判定（大小写不敏感，兼容英文实况素材）
fn norm_chars(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 判断 a 的字符是否按序出现在 b 中（近似前缀/渐进片段匹配，双指针）
fn is_subsequence(a: &[char], b: &[char]) -> bool {
    let mut j = 0;
    for &c in a {
        while j < b.len() && b[j] != c {
            j += 1;
        }
        if j >= b.len() {
            return false;
        }
        j += 1;
    }
    true
}

/// 渐进碎片的时间跨度上限（秒，**绝对秒下限**）：超过即视为"短暂但完整的独立条目"，不拼接。
///
/// 取值依据（按基准诊断）：打字机中间态只活约 1 个网格间隔；独立条目（如单独显示的
/// 角色名条）寿命通常数秒。实测真实碎片对最长 1.057s（0.5s 间隔下），故 2.5×0.5s = 1.25s
/// 既容纳真实碎片、又守住独立条目。
///
/// 2026-09-18 改为**绝对秒下限** `max(本值, factor × interval)`：网格降为 0.25s 后，
/// 显示行为（碎片寿命）不随之减半，门限若同步减半会误伤真实碎片。
const PROGRESSIVE_MAX_SPAN_SEC: f64 = 1.25;
const PROGRESSIVE_MAX_SPAN_FACTOR: f64 = 2.5;

/// 方向 2 长残尾档的时长上限（秒，**绝对秒下限**）：D1 第 2 步定稿（2026-09-18）。
///
/// 取值依据（pre-merge 段序诊断，moon 嵌字 clip 2/5）：延迟重读残尾在 merge 输入
/// 的真实形态是 `[01:52.067 → 01:53.734]`＝**1.667s**——最终事件里显示的 1.467s 是
/// `clamp_segment_times` 压掉与后段重叠后的假象（曾据此误判门值）。独立姓名框态
/// 实测 1.483s，与残尾仅差 0.18s，**时长门无法区分二者**（1.5s 门曾吞姓名框造成
/// glupov 语料缺失 1）。故本档改由"**字尾匹配**"判别（残尾 ≈ 前段归一化文本的
/// 等长尾窗：重读到句尾；姓名框是字头重复、尾窗比率 ≈1 被拒），时长门放到
/// 4.0×0.5s = 2.0s 仅作上界；同样改为绝对秒下限（网格 0.25s 时仍为 2.0s，
/// 否则 moon 那条 1.667s 残尾会因门限降到 1.0s 而重新变成碎片）。
const RESIDUAL_MAX_SPAN_SEC: f64 = 2.0;
const RESIDUAL_MAX_SPAN_FACTOR: f64 = 4.0;

/// 相邻段拼接的时间邻接窗（秒，**绝对秒下限**）：两段间空档大于此值即视为不同条目。
/// 0.5s = 原 0.5s 网格下一格；网格降为 0.25s 后仍保持 0.5s（显示行为的绝对尺度）。
const CONTIGUOUS_MAX_GAP_SEC: f64 = 0.5;

/// 阶段 2 短字幕召回的时长下限（秒）：短于此视为逐帧闪烁/噪声，不值得召回。
///
/// 背景：每次召回 = 一次 ffmpeg 单帧抽取（~0.19s）+ 一次单图 OCR IPC（~0.42s）。
/// pierro 嵌字跑实测在「恰 2 边界」窗口共触发 878 次候选、无门限时精化 546s；
/// 绝大多数候选只持续 1~2 帧（≈33ms），是打字机/画面动效的亚阈值抖动而非真短字幕。
///
/// 取值依据（2026-09-14 门限网格 + 同会话受控四跑，pierro 候选窗 878、贴边被拒 2）：
/// | 门限 | 召回 | 精化 | clip 总计 | 复合 | Δend p95 |
/// | 无   | 878 | 519s | 2567s | 15.5 | 9.80s |
/// | .10/.05 | 586 | 339s | 2371s | 16.1 | 12.32s |
/// | .15/.075| 299 | 179s | 2213s | 17.4 | 10.77s |
/// | .20/.10 | 113 |  71s | 2157s | 18.8 | 10.48s |
/// 边际质量代价随剪枝加深而递增（0.0021 / 0.0045 / 0.0075 分·召回⁻¹）。取 0.15/0.075：
/// 精化 −65%、总耗时 −13.8%，且新三轴报告（结构/时间/覆盖）全面不劣于 0.10/0.05。
const RECALL_MIN_SPAN_SEC: f64 = 0.15;
/// 召回候选两侧（A/C 状态）各自的持续时长下限（秒）：排除"短字幕恰好贴住
/// 窗口边缘"的伪影（真正的短字幕出现在窗口内部时，两侧都有垫底状态）。
const RECALL_MIN_SIDE_SEC: f64 = 0.075;

/// 静态超时强制采样（D8 1b + C）：网格距最近一次 OCR 超过该秒数 → 强制 OCR 一帧。
///
/// 背景：9×8 dHash 对"画面静止 + 同位置整行文字替换"的分辨力不足（实测 pierro
/// 语料 [74]↔[75] 距离仅 2~4 位 ≤ 阈值 3），且渐进显示的逐步变化同样低于阈值——
/// 变化检测整句漏检时，静态超时帧可读到完整文本，由合并层吸收文本（时间轴不动）。
///
/// 取值（2026-09-16 由 4.0 → 1.5，C 方案）：保险丝锚点是 last_ocr_t = 字幕的
/// 帧级切换时刻（1a 注入覆写），故 T 即"该字幕至少每隔 T 秒被重读一次"——
/// 打字机通常 ~1s 内补全，T=1.5 保证寿命 >1.5s 的字幕至少采到一次完整态
/// （glupov [卡佳和科利亚] 句寿命 3.1s < 旧 T=4.0，完整态窗口 2.2s 从未被采）。
/// 代价：静态段 OCR 频率 ×2.7（同源模拟 glupov 语料两窗 20→24 / 32→35；
/// pierro 语料静默约 961s，约 +400 帧 ≈ +3 分钟）。env：GSA_OCR_STALE_TIMEOUT_SEC（≤0 关闭）。
const STALE_OCR_TIMEOUT_SEC: f64 = 1.5;

/// 阶段 2 召回候选判定：窗口内 `[m, s)` 为疑似短字幕（boundaries.len()==2 时）。
///
/// - 区间至少两帧（对应旧条件 `s > m + 1`）
/// - 候选可见时长 ≥ `min_span`
/// - 候选前后（A 侧 `[0, m)`、C 侧 `[s, wlen)`）各持续 ≥ `min_side`
fn recall_eligible(
    m: usize,
    s: usize,
    wlen: usize,
    dense_interval: f64,
    min_span: f64,
    min_side: f64,
) -> bool {
    if s <= m + 1 || s >= wlen {
        return false;
    }
    let span = (s - m) as f64 * dense_interval;
    let a_side = m as f64 * dense_interval;
    let c_side = (wlen - s) as f64 * dense_interval;
    span >= min_span && a_side >= min_side && c_side >= min_side
}

/// 相邻段"包含关系拼接"：把同一句被拆开的分片合成一段（修复碎片化）。
///
/// 判据（较短者记 S、较长者记 L，两个方向共用）：
/// 1. **时间相邻**：`L.start − S.end ≤ interval`（允许重叠）—— 排除跨空档的远距离误并
/// 2. **归一化包含**：`norm(S)` 按序出现在 `norm(L)` 中（大小写/标点/换行不敏感）
/// 3. **更长者胜**：`norm(L).len > norm(S).len`
/// 4. **较短者短暂**：S 时长 ≤ `PROGRESSIVE_MAX_SPAN_FACTOR × interval`
///
/// 两个方向：
/// - 前段是后段的渐进片段（打字机逐字显现）→ 取后段完整文本，保留前段起点
/// - 后段是前段的残留片段（尾部残缺）→ 并入前段，时间取并集，文本不变
///
/// 备注：方向 1 **不设置信度守卫**，一律取更长（更完整）文本——语料基准 A/B
/// 证明守卫会在更长文本置信度仅略低时保留较短残片，使完整句从语料消失
/// （glupov 语料 97.3→83.6 即由此引起，去守卫后恢复）。
///
/// 方向 1 的第二档判据（`is_line_prefix`，D9）：残片因漏检被拖长到数秒（超过
/// `PROGRESSIVE_MAX_SPAN_SEC` 护栏）时，若前段各行为后段对应行的逐行归一化前缀、
/// 且末行是严格前缀（句中切断），仍判为同一句补全而拼接——glupov 嵌字 "…alive, I h"
/// 撑 3.2s 即此类；末行整行相等 = 行边界切断（对话行清空只剩姓名框的独立状态），
/// 不并入下一条。
pub fn merge_contained_adjacent(segments: Vec<OcrSegment>, interval: f64) -> Vec<OcrSegment> {
    // 绝对秒下限与"网格相对项"取较大者：0.5s 网格下等价于原行为；更细网格下门限不随
    // 采样率下降（碎片寿命是显示行为，与采样率无关）
    let max_span = PROGRESSIVE_MAX_SPAN_SEC.max(interval * PROGRESSIVE_MAX_SPAN_FACTOR);
    let residual_span = RESIDUAL_MAX_SPAN_SEC.max(interval * RESIDUAL_MAX_SPAN_FACTOR);
    let mut out: Vec<OcrSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = out.last_mut() {
            let contiguous = seg.start - last.end <= CONTIGUOUS_MAX_GAP_SEC.max(interval);
            let ln = norm_chars(&last.text);
            let sn = norm_chars(&seg.text);
            if contiguous && ln.len() >= 2 && sn.len() >= 2 {
                // 方向 1：前段是后段的渐进片段 → 取完整文本（不设置信度守卫，依据见函数注释）。
                // 两档判据（任一命中即拼接）：短寿命残片+子序列（原护栏）；或
                // 行级字面前缀（不限时长，见 is_line_prefix 注释）。
                if ln.len() < sn.len()
                    && (is_line_prefix(&last.text, &seg.text)
                        || (last.end - last.start <= max_span && is_subsequence(&ln, &sn)))
                {
                    last.text = seg.text.clone();
                    last.confidence = seg.confidence;
                    // 并集而非赋值（D11）：段序按 start 排序，后段可能**嵌套**在前段跨度内
                    // （打字机残片滞后入列），赋值会缩短已累积跨度
                    last.end = last.end.max(seg.end);
                    continue;
                }
                // 方向 2：后段是前段的残留片段 → 并入前段（文本不变，时间取并集）。
                // 两档判据（D1 第 2 步）：
                // - 短残尾（≤ 2.5×interval）+ 归一化子序列；
                // - 长残尾（≤ 4.0×interval）+ **字尾匹配**：残尾归一化串与前段归一化
                //   文本的**等长尾窗**比对（编辑距离比 ≤ 0.2），即"重读到句尾"。
                //   时长门不能区分长残尾与独立姓名框态（1.667s vs 1.483s），位置可以：
                //   姓名框是字头重复（尾窗比率 ≈1）被拒。
                let residual_tail_match = sn.len() < ln.len()
                    && seg.end - seg.start <= residual_span
                    && {
                        let ltail: String = ln[ln.len() - sn.len()..].iter().collect();
                        let sn_str: String = sn.iter().collect();
                        edit_distance_ratio(&sn_str, &ltail) <= LINE_PREFIX_EDIT_TOLERANCE
                    };
                if sn.len() < ln.len()
                    && ((seg.end - seg.start <= max_span && is_subsequence(&sn, &ln))
                        || residual_tail_match)
                {
                    // 并集而非赋值（D11）：嵌套残片（seg.end < last.end）赋值会把
                    // 长段截断——pierro 实测 62.9s 静态段被 0.25s 嵌套残片截成 0.5s
                    last.end = last.end.max(seg.end);
                    continue;
                }
            }
        }
        out.push(seg);
    }
    out
}

/// 行级字面前缀的行间容差（D1 第 1 步，模糊化）：共享区 OCR 错位（替换 1~2 字符）
/// 不再阻断同句判定。取值依据（glupov/pierro 嵌字事件取证）：已知错位残句对的
/// 归一化编辑距离比 0.067~0.15（"nt they…"/"at they…"、"Dne…"/"one…" 等），
/// 0.2 留 1.3~3 倍余量；不同句子的整行差异通常远超 0.2，误并由"逐行独立+
/// 前缀结构+时间连续"三重约束兜底。极短行（≤4 字符）1 字符错位比率 ≥0.25
/// 天然超阈，保守拒绝。
const LINE_PREFIX_EDIT_TOLERANCE: f64 = 0.2;

/// 行级字面前缀：`prev` 的各行（归一化）**模糊**匹配 `next` 对应行，且末行须为
/// **严格前缀**（句中切断）。用于方向 1 的第二档（不限时长）。
///
/// - 需 `prev` ≥ 2 行：挡住"派蒙"/"派蒙：旅行者你来了"这类单行独立短条
///   （那是完整独立条目，由 2.5×interval 护栏保护，不得放宽）
/// - 前 `prev` 行数 −1 行逐行模糊相等：`edit_distance_ratio ≤ 0.2`（OCR 共享区
///   错位/漏检 1 字符不再阻断，D1 已证案例 "nt they…"/"at they…" 等）；
/// - 末行：`next` 对应行取**前 plast.len() 字符窗口**与 `plast` 比对，距离比 ≤ 0.2，
///   且 `plast` 严格更短（句中切断/打字机补全中）；
/// - 末行整行（模糊比 0）相等 = `prev` 恰在行边界结束 → 对话行清空的独立状态，
///   与下一条是不同条目，不拼接（glupov 语料 [……] 省略号条即此类）。
fn is_line_prefix(prev: &str, next: &str) -> bool {
    let p: Vec<String> = prev
        .lines()
        .map(|l| norm_chars(l).into_iter().collect())
        .collect();
    let n: Vec<String> = next
        .lines()
        .map(|l| norm_chars(l).into_iter().collect())
        .collect();
    if p.len() < 2 || n.is_empty() || p.len() > n.len() {
        return false;
    }
    for (a, b) in p.iter().zip(n.iter()).take(p.len() - 1) {
        if edit_distance_ratio(a, b) > LINE_PREFIX_EDIT_TOLERANCE {
            return false;
        }
    }
    let plast = &p[p.len() - 1];
    let nlast = &n[p.len() - 1];
    if plast.is_empty() || plast.len() >= nlast.len() {
        return false;
    }
    // 末行模糊前缀：`next` 对应行取与 plast 等长的前缀窗口比对（共享区 1~2 字符
    // 错位只需 1 次替换即可对齐，插入/删失导致的整体偏移会抬高比率而保守拒绝）
    let nhead: String = nlast.chars().take(plast.chars().count()).collect();
    let tail_len_ok = nlast.chars().count() > plast.chars().count();
    tail_len_ok && edit_distance_ratio(plast, &nhead) <= LINE_PREFIX_EDIT_TOLERANCE
}

/// 字幕预估最短长度的**默认值**（秒）——前端可调项 `OcrRunParams::min_subtitle_sec` 的缺省。
///
/// 依据（2026-09-18 主观评审 + 基准实测）：真实字幕寿命通常 ≥1s（与 D10 保险丝 1.5s、
/// D9 的 2.5×interval 护栏同源），短于此的产出段几乎必为**分段错误**——实测嵌字碎片成因：
/// 打字机首帧、OCR 误读（`You've`→`Du've`、`ll the`→`u the`）、共享区少一个空格
/// （`Onyx Agate`/`OnyxAgate`）、姓名框态、淡出残留、画面图案误识别。
///
/// 取 **0.7s**（保守值，实测标定）：

/// | 门 | 语料 glupov / moon | 嵌字 glupov / moon | 碎片 |
/// |---|---|---|---|
/// | 0.7s（默认） | 97.1 / 100.0 ✓ | 58.5 / 50.7 | 各 1 |
/// | 1.3s | 97.1 / 100.0 ✓ | 58.5 / 50.7（未生效） | 各 1 |
/// | 1.6s | 92.5 / 94.1（各缺失 1 ✗） | 60.8 / 53.4 | 各 0 |
///
/// 语料里存在 1.48s 姓名框/头衔态，与 moon 1.50s 姓名框态**结构同形但参考语义相反**
/// （语料要求独立、moon 要求合并）→ 纯文本/时长判据无法两全，故默认取保守值；
/// 前端把本值作为"字幕预估最短长度"暴露给用户自行权衡。
pub const DEFAULT_MIN_SUBTITLE_SEC: f64 = 0.7;

/// 短碎片与其后一条允许的时间间隔上限（秒）：实测存在 0.5~1.5s 的检测空洞
/// （pierro 4→5、21→22 段间有空间隔，被既有 0.5s 邻接门挡掉而漏并）。
const SHORT_FRAGMENT_GAP_MAX_SEC: f64 = 2.0;

/// 末行模糊前缀容差：覆盖 OCR 误读 1~2 字符（长句下比率远小于此）
const SHORT_FRAGMENT_PREFIX_TOL: f64 = 0.3;

/// 末行字符重叠率下限：覆盖乱序/替换型误读
const SHORT_FRAGMENT_OVERLAP: f64 = 0.5;

/// 字符多重集重叠率：`Σ min(count_a, count_b) / |a|`
fn overlap_ratio(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    let mut used = vec![false; b.len()];
    let mut hit = 0usize;
    for &c in a {
        if let Some(i) = b.iter().enumerate().position(|(j, &d)| !used[j] && d == c) {
            used[i] = true;
            hit += 1;
        }
    }
    hit as f64 / a.len() as f64
}

/// 短碎片与后一条的**弱关联**判定（只比较碎片末行与后一条对应行）。
///
/// 三条任一成立即算关联：归一化子序列（含空白/标点差异）、模糊前缀（OCR 误读 1~2 字符）、
/// 字符重叠率 ≥0.5（乱序/替换型误读）。
///
/// **护栏**：要求碎片末行归一化后**非空**——这条保住 D9 的独立短条：
/// glupov 语料 `[……]` 条末行归一化为空串，绝不能被并入下一条
/// （否则语料出现"缺失"，是六项基准的长期硬门）。
fn short_fragment_related(frag: &str, next: &str) -> bool {
    let fl: Vec<Vec<char>> = frag.lines().map(norm_chars).collect();
    let nl: Vec<Vec<char>> = next.lines().map(norm_chars).collect();
    if fl.is_empty() || nl.is_empty() {
        return false;
    }
    let flast = &fl[fl.len() - 1];
    if flast.len() < 2 {
        return false;
    }
    let idx = (fl.len() - 1).min(nl.len() - 1);
    let nline = &nl[idx];
    if nline.is_empty() {
        return false;
    }
    if is_subsequence(flast, nline) {
        return true;
    }
    let head: Vec<char> = nline.iter().copied().take(flast.len()).collect();
    let hs: String = flast.iter().collect();
    let hh: String = head.iter().collect();
    if edit_distance_ratio(&hs, &hh) <= SHORT_FRAGMENT_PREFIX_TOL {
        return true;
    }
    overlap_ratio(flast, nline) >= SHORT_FRAGMENT_OVERLAP
}

/// 短碎片激进合并：时长 < `min_subtitle_sec` 的段若与其后一条弱关联，
/// 并入后一条——**保留碎片起点**（该显示的真实起点，常比后条检测到的起点更准）
/// 与后条终点/文本（用户 2026-09-18 主观评审提案）。
///
/// `min_subtitle_sec` 由 `OcrRunParams::min_subtitle_sec` 传入（前端可调，默认 0.7s，
/// ≤0 关闭本 pass）。调大能减少碎片，但会提高"误吞真实短句"的概率——实测 1.6s 时
/// 语料出现缺失（见 `SHORT_FRAGMENT_MAX_SEC` 注释的标定表）。
///
/// 与既有合并的分工：
/// - `merge_contained_adjacent`：严格判据（子序列/行前缀）+ 1.25s 跨度门；
/// - 本 pass：**放宽到弱关联** + `min_subtitle_sec` 时长门 + 2.0s 间隔门（跨检测空洞）；
/// - D12 的稳定姓名框态（≥2.0s）不在本 pass 范围，仍保持独立（实测参考亦记为独立条目）。
fn merge_short_fragments_into_next(
    segments: Vec<OcrSegment>,
    min_subtitle_sec: f64,
) -> Vec<OcrSegment> {
    if min_subtitle_sec <= 0.0 {
        return segments;
    }
    let mut out: Vec<OcrSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        let merge = out.last().is_some_and(|last| {
            last.end - last.start < min_subtitle_sec
                && seg.start - last.end <= SHORT_FRAGMENT_GAP_MAX_SEC
                && short_fragment_related(&last.text, &seg.text)
        });
        if merge {
            if let Some(last) = out.pop() {
                out.push(OcrSegment {
                    start: last.start,
                    ..seg
                });
                continue;
            }
        }
        out.push(seg);
    }
    out
}

/// 段尾精化（B-ii）：把 `end = 末采样 + interval×0.5` 的**量化估计**替换为密帧实测的切换时刻。
///
/// 依据（2026-09-18 边界形态诊断）：Δend p95 达 0.87s（glupov）/0.62s（moon），而容差 0.5s。
/// `末采样 + interval×0.5` 只是"覆盖率中点"估计（D2′），切换发生在采样点之后即欠伸、
/// 采样点紧贴切换即过伸。而**切换是突变**（整行新字幕），密帧能逐帧定位。
///
/// 与阶段 1 起点精化同源同数据（`scan_stream` 内存密帧，零抽帧零落盘）：以末采样处的哈希为
/// 基准，在**其后 1.5×interval 窗口**内找首个越阈帧（= 下一条字幕的切换帧），取该时刻为段尾。
/// 该判据与变化检测同阈值同语义，故不会引入采样层看不到的新边界。
/// 窗口内无越阈（本段延续到 clip 末尾）→ 保留原估计。
fn refine_segment_ends(
    segments: Vec<OcrSegment>,
    dense: &[(f64, u64)],
    threshold: u32,
    interval: f64,
    clip_end: f64,
) -> Vec<OcrSegment> {
    if dense.is_empty() {
        return segments;
    }
    // 取时间上最接近 `t` 的密帧哈希（密帧间隔 ~1/src_fps，同一显示状态）
    let hash_near = |t: f64| -> Option<u64> {
        let i = dense.partition_point(|(dt, _)| *dt < t);
        let cand = [i.checked_sub(1), (i < dense.len()).then_some(i)];
        cand.into_iter()
            .flatten()
            .min_by(|&a, &b| {
                (dense[a].0 - t)
                    .abs()
                    .partial_cmp(&(dense[b].0 - t).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|k| dense[k].1)
    };
    segments
        .into_iter()
        .map(|mut seg| {
            let last_sample = seg.end - interval * 0.5;
            if let Some(base) = hash_near(last_sample) {
                let hi = last_sample + interval * 1.5;
                if let Some(&(t, _)) = dense
                    .iter()
                    .find(|(dt, h)| *dt > last_sample && *dt <= hi
                        && crate::ai_runtime::dhash::hamming_distance(base, *h) > threshold)
                {
                    seg.end = t.clamp(seg.start, clip_end);
                }
            }
            seg
        })
        .collect()
}

/// 修正相邻段时间重叠：段 `end` 不得超过下一段 `start`。
///
/// 阶段 1 的窗口精化会把 changed 帧 `start` 提前到帧级边界，而 `end` 仍按
/// 网格 `last + interval` 计算，可能产生交叉/重叠 → 统一 clamp 保证时间单调。
fn clamp_segment_times(mut segments: Vec<OcrSegment>) -> Vec<OcrSegment> {
    for i in 1..segments.len() {
        if segments[i - 1].end > segments[i].start {
            segments[i - 1].end = segments[i].start;
        }
    }
    segments
}

/// 相邻段文本相似（相等/前缀/编辑距离 ≤ 阈值）→ 合并为一段（取更长文本、时间取并集）。
///
/// 用于阶段 2 短字幕召回后：dHash 判为变化但 OCR 文本与相邻段相同的"伪短字幕"
/// （字幕视觉抖动/OCR 抖动）会与相邻段相似，在此合并，避免同一句被拆成碎片。
fn merge_similar_adjacent(segments: Vec<OcrSegment>, threshold: f64) -> Vec<OcrSegment> {
    let mut out: Vec<OcrSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(last) = out.last_mut() {
            // 稳定 ≥2.0s 的姓名框/头衔态与下一条对话行不并（D11）
            let stable_boundary = is_stable_line_boundary_cut(
                &last.text,
                &seg.text,
                last.end - last.start,
                seg.confidence,
            );
            if !stable_boundary && similar_text(&last.text, &seg.text, threshold) {
                last.end = last.end.max(seg.end);
                // 更长且置信度不低于原段才替换文本，避免用低置信度长文本覆盖清晰短文本
                if seg.text.chars().count() > last.text.chars().count()
                    && seg.confidence >= last.confidence
                {
                    last.text = seg.text;
                    last.confidence = seg.confidence;
                }
                continue;
            }
        }
        out.push(seg);
    }
    out
}

/// 对每个 changed 帧做窗口精化：从零落盘扫描序列中切出窗口密帧哈希，
/// 逐帧相对基准 A 判定，计算帧级主边界（阶段 1）+ 召回窗口内短字幕（阶段 2）。
///
/// 返回 `(精化后的 changes, 召回的短字幕段)`。
/// 单独成函数便于集成测试/基准直接调用，不依赖 Tauri 命令栈。
///
/// 性能策略：哈希全部来自扫描阶段的内存序列（`scan_frame_hashes`），
/// 本函数零抽帧零落盘；仅阶段 2 召回时按需抽取一张中间帧 JPEG
/// （OCR worker 消费文件路径，每窗口至多一次）。
///
/// `on_refine(done, total, frac, message)`：每批次帧进度回调一次（聚合上限 ~50 次），
/// 用于消除精化阶段的长静默（pierro 曾 546s 无输出）。
#[allow(clippy::too_many_arguments)]
pub fn refine_window_changes(
    changes: &[crate::ai_runtime::dhash::FrameChange],
    scan_stream: &[(f64, u64)],
    video_path: &str,
    clip: &OcrRegionInput,
    video_w: u32,
    video_h: u32,
    src_fps: f64,
    dhash_threshold: u32,
    recall_dir: &Path,
    manager: &crate::ai_runtime::OcrManager,
    dev_debug: bool,
    mut on_refine: impl FnMut(usize, usize, f64, String),
) -> (Vec<crate::ai_runtime::dhash::FrameChange>, Vec<OcrSegment>) {
    let dense_interval = 1.0 / src_fps;
    let mut short_segments: Vec<OcrSegment> = Vec::new();
    let mut refined: Vec<crate::ai_runtime::dhash::FrameChange> =
        Vec::with_capacity(changes.len());
    // 进度聚合步长：全片至多 ~50 次回调，避免逐帧刷屏
    let total = changes.len();
    let step = (total / 50).max(1);
    // P1 预筛门限：env 可覆盖（调参用），默认取常量
    let min_span = std::env::var("GSA_RECALL_MIN_SPAN_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RECALL_MIN_SPAN_SEC);
    let min_side = std::env::var("GSA_RECALL_MIN_SIDE_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RECALL_MIN_SIDE_SEC);

    for idx in 0..changes.len() {
        let mut fc = changes[idx].clone();
        if fc.is_changed && idx > 0 {
            if let Some(base) = fc.base_hash {
                let lo = changes[idx - 1].time;
                let hi = fc.time;
                if hi > lo {
                    // 窗口 = 扫描序列的时间切片（与旧"整段密帧落盘后切片"同一帧集合，
                    // 哈希同源：网格哈希也取自该序列，交叉比较基准一致）
                    let window: Vec<(f64, u64)> = scan_stream
                        .iter()
                        .filter(|(t, _)| *t >= lo && *t < hi)
                        .copied()
                        .collect();
                    let hashes: Vec<u64> = window.iter().map(|(_, h)| *h).collect();
                    let boundaries =
                        crate::ai_runtime::dhash::boundary_indices(&hashes, base, dhash_threshold);
                    // 下一段（f_k）的真实边界 = 最后一个变化帧
                    if let Some(&last) = boundaries.last() {
                        // 用窗口内实际帧时间（而非 lo + last*interval），窗口首帧
                        // 未必恰在 lo，更精确也避免假设
                        let new_time = window[last].0;
                        if new_time < hi {
                            if dev_debug {
                                eprintln!(
                                    "[ocr]   精化边界 [{} → {}]",
                                    fmt_time(fc.time),
                                    fmt_time(new_time)
                                );
                            }
                            fc.time = new_time;
                        }
                        // 阶段 2：恰好两次变化 → [main, sub[0]) 为短字幕候选。
                        // 注意：只召回恰好 2 个边界的窗口（A→短字幕→C）。遍历全部边界对
                        // 会过度召回（每次召回一次慢速 OCR IPC，实测事件数翻倍、耗时×3），
                        // 故不做泛化，多突变/多短字幕留给后续更精确的判定。
                        if boundaries.len() == 2 {
                            let (m, s) = (boundaries[0], boundaries[1]);
                            // P1 预筛：候选须足够时长且两侧状态持续，
                            // 过滤逐帧闪烁，避免无收益的 ffmpeg 抽帧 + OCR IPC
                            if recall_eligible(
                                m,
                                s,
                                window.len(),
                                dense_interval,
                                min_span,
                                min_side,
                            ) {
                                // 区间中点帧做 OCR（避开切换过渡帧）；
                                // 单帧 mjpeg 管道取内存字节 → base64，不落盘
                                let mid = m + (s - m) / 2;
                                let start = lo + m as f64 * dense_interval;
                                let end = lo + s as f64 * dense_interval;
                                if let Some(&(mid_time, _)) = window.get(mid) {
                                    if let Ok(jpeg) = crate::video::extract_single_frame_bytes(
                                        video_path,
                                        mid_time,
                                        clip.x1,
                                        clip.y1,
                                        clip.x2,
                                        clip.y2,
                                        video_w,
                                        video_h,
                                    ) {
                                        if dev_debug {
                                            // dev 模式留一份召回帧供人工检查
                                            let dump = recall_dir
                                                .join(format!("recall_{idx}.jpg"));
                                            let _ = std::fs::write(dump, &jpeg);
                                        }
                                        let image = b64(&jpeg);
                                        if let Ok(res) = manager.with_provider(|p| {
                                            p.recognize_batch(std::slice::from_ref(&image))
                                        }) {
                                            if let Some(r) = res.into_iter().next() {
                                                let text = r.text.trim().to_string();
                                                if !text.is_empty() {
                                                    short_segments.push(OcrSegment {
                                                        start,
                                                        end,
                                                        confidence: r.confidence,
                                                        text,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        refined.push(fc);
        if idx % step == 0 || idx + 1 == total {
            on_refine(
                idx + 1,
                total,
                (idx + 1) as f64 / total.max(1) as f64,
                format!("精化边界 {}/{} 帧", idx + 1, total),
            );
        }
    }
    (refined, short_segments)
}

// ─── 编排命令 ─────────────────────────────────────────────

/// 前端传入的 ocr_region 选区（归一化坐标 + 时间段）
#[derive(Deserialize)]
pub struct OcrRegionInput {
    pub start: f64,
    pub end: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// OCR 运行参数
#[derive(Deserialize)]
pub struct OcrRunParams {
    /// 帧间隔（秒），默认 1.0
    pub frame_interval: f64,
    /// dHash 变化检测阈值，默认 3
    pub dhash_threshold: u32,
    /// OCR 批大小，默认 16
    pub batch_size: usize,
    /// 合并相似度阈值（编辑距离比例，默认 0.3）
    pub merge_similarity: f64,
    /// 字幕预估最短长度（秒，默认 0.7）：短于此的产出段若与后一条弱关联，则并入后一条
    /// （保留碎片起点 + 后条终点/文本）。**前端可调**——调大能减少碎片，但会提高
    /// "误吞真实短句"的概率（基准语料硬门会暴露）；实测 1.6s 会使语料出现缺失。
    pub min_subtitle_sec: f64,
}

/// 进度事件载荷
#[derive(Clone, Serialize)]
pub struct OcrProgress {
    pub clip_index: usize,
    pub clip_count: usize,
    /// 整体进度 0.0 ~ 1.0
    pub progress: f64,
    pub message: String,
}

/// RAII 临时目录守卫：任何退出路径（含 panic 展开、JoinHandle 错误）都会清理。
/// `keep` 为 true 时保留目录（dev_debug 用于人工检查帧）。
struct TempDirGuard {
    path: PathBuf,
    keep: bool,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// 格式化时间 mm:ss.mmm
/// 秒 → "MM:SS.mmm"（集成测试打印结果用）
pub fn fmt_time(s: f64) -> String {
    let total_ms = (s * 1000.0).round() as i64;
    let ms = total_ms % 1000;
    let sec = (total_ms / 1000) % 60;
    let min = total_ms / 60000;
    format!("{:02}:{:02}.{:03}", min, sec, ms)
}

/// 串起完整 OCR 流水线（抽帧 → 变化检测 → 窗口精化 → OCR → 合并）。
///
/// 抽取为独立函数便于集成测试直接调用（不依赖 Tauri 命令栈）。
/// `src_fps`：源视频帧率，>0 时启用切换窗口的帧级主边界精化（阶段 1）。
/// `on_progress(clip_index, clip_count, progress, message)` 每次阶段推进调用一次。
#[allow(clippy::too_many_arguments)] // 参数为流水线依赖的显式事实，不聚合为结构体以保持可测性
pub fn run_ocr_pipeline<F>(
    manager: &OcrManager,
    video_path: &str,
    video_w: u32,
    video_h: u32,
    region_clips: &[OcrRegionInput],
    params: &OcrRunParams,
    src_fps: f64,
    mut on_progress: F,
) -> Result<Vec<OcrSegment>, String>
where
    F: FnMut(usize, usize, f64, String),
{
    let clip_count = region_clips.len();
    let ready = manager.with_provider(|p| p.is_ready());
    if !ready {
        return Err("OCR 运行环境未就绪".into());
    }
    if clip_count == 0 {
        return Err("没有可处理的 OCR 选区".into());
    }

    let dev_debug = manager.config().dev_debug;
    let frame_interval = if params.frame_interval > 0.0 {
        params.frame_interval
    } else {
        1.0
    };
    let dhash_threshold = params.dhash_threshold;
    let batch_size = params.batch_size.max(1);
    let merge_similarity = params.merge_similarity;
    // 字幕预估最短长度：≤0 视为关闭短碎片激进合并
    let min_subtitle_sec = params.min_subtitle_sec;
    // D8(1b) 静态超时保险丝：env 可覆盖，≤0 关闭
    let stale_timeout = std::env::var("GSA_OCR_STALE_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(STALE_OCR_TIMEOUT_SEC);

    // 临时目录：dev 时写仓库根 temp/ocr/<uuid> 并保留；否则系统临时目录 + 自动清理
    let base_dir = if dev_debug {
        let repo = manager
            .runtime_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir());
        repo.join("temp").join("ocr").join(Uuid::new_v4().to_string())
    } else {
        std::env::temp_dir().join(format!("gsa_ocr_{}", Uuid::new_v4()))
    };
    let _guard = TempDirGuard {
        path: base_dir.clone(),
        keep: dev_debug,
    };
    std::fs::create_dir_all(&base_dir).map_err(|e| format!("无法创建临时目录: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&base_dir, std::fs::Permissions::from_mode(0o700));
    }
    if dev_debug {
        eprintln!("[ocr] 帧输出目录: {}", base_dir.display());
    }

    let mut emit = |clip_index: usize, progress: f64, message: String| {
        on_progress(clip_index, clip_count, progress, message);
    };

    let mut all_segments = Vec::new();
    for (i, clip) in region_clips.iter().enumerate() {
        let clip_started = std::time::Instant::now();
        let clip_dir = base_dir.join(format!("clip_{}", i));
        std::fs::create_dir_all(&clip_dir).map_err(|e| format!("无法创建 clip 目录: {}", e))?;

        // ── 零落盘扫描：一次得到全 clip 的 dHash 序列 ──
        // 网格变化检测与窗口精化共用同一序列（哈希同源，交叉比较基准才一致）。
        // 扫描密度取源帧率（精化需要帧级时间）；源帧率未知时退化为网格密度。
        let scan_interval = if src_fps > 0.0 {
            (1.0 / src_fps).min(frame_interval)
        } else {
            frame_interval
        };
        emit(
            i,
            i as f64 / clip_count as f64,
            format!("扫描变化 {}/{}", i + 1, clip_count),
        );
        let scan_started = std::time::Instant::now();
        let scan_stream = crate::video::scan_frame_hashes(
            video_path,
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            video_w,
            video_h,
            scan_interval,
        )
        .map_err(|e| format!("扫描失败（clip {}）: {}", i, e))?;
        let scan_dt = scan_started.elapsed();

        // ── 网格变化标志：网格帧哈希 = 扫描序列中时间最近帧 ──
        // （网格与精化窗口的哈希同源，boundary_indices 交叉比较基准才一致）
        let last_scan = scan_stream.len().saturating_sub(1);
        let grid_frame_count = ((clip.end - clip.start) / frame_interval).ceil() as usize;
        let grid_hashes: Vec<u64> = (0..grid_frame_count)
            .map(|k| {
                let idx =
                    ((k as f64 * frame_interval / scan_interval).round() as usize).min(last_scan);
                scan_stream[idx].1
            })
            .collect();
        let grid_times: Vec<f64> = (0..grid_frame_count)
            .map(|k| clip.start + (k as f64) * frame_interval)
            .collect();
        // D8：变化检测 + 补漏（帧级差分注入 + 静态超时），语义见 dhash::change_flags_rescued
        let (flags, time_overrides) = crate::ai_runtime::dhash::change_flags_rescued(
            &grid_hashes,
            &grid_times,
            &scan_stream,
            dhash_threshold,
            stale_timeout,
        );
        // mjpeg 抽帧只为变化帧保留字节；未变化帧在流中读到即丢（零落盘）
        let keep: Vec<usize> = flags
            .iter()
            .enumerate()
            .filter(|(_, (c, _))| *c)
            .map(|(k, _)| k)
            .collect();

        // ── 网格抽帧（mjpeg 管道 → 内存字节，OCR IPC 载荷）──
        emit(
            i,
            (i as f64 + 0.4) / clip_count as f64,
            format!("提取帧 {}/{}", i + 1, clip_count),
        );
        let extract_started = std::time::Instant::now();
        let grid = crate::video::extract_frames_bytes(
            video_path,
            clip.start,
            clip.end,
            clip.x1,
            clip.y1,
            clip.x2,
            clip.y2,
            video_w,
            video_h,
            frame_interval,
            &keep,
        )
        .map_err(|e| format!("抽帧失败（clip {}）: {}", i, e))?;
        let extract_dt = extract_started.elapsed();
        if dev_debug {
            // dev 模式把保留的帧写盘供人工检查（生产模式全程零落盘）
            for fb in &grid.kept {
                let dump = clip_dir.join(format!("frame_{:05}.jpg", fb.index + 1));
                let _ = std::fs::write(dump, &fb.jpeg);
            }
            eprintln!(
                "[ocr] clip {}/{}：扫描 {} 帧，网格 {} 帧，变化 {} 帧 → OCR",
                i + 1,
                clip_count,
                scan_stream.len(),
                grid.total,
                grid.kept.len()
            );
        }

        // 装配 changes/jpegs：以 mjpeg 流实际帧数为准，幻影变化帧按网格时间补抽单帧
        let (mut changes, jpegs) = assemble_grid_frames(
            &flags,
            grid.kept.iter().map(|f| f.jpeg.clone()).collect(),
            grid.total,
            clip.start,
            frame_interval,
            |time| {
                crate::video::extract_single_frame_bytes(
                    video_path,
                    time,
                    clip.x1,
                    clip.y1,
                    clip.x2,
                    clip.y2,
                    video_w,
                    video_h,
                )
                .ok()
            },
        )
        .map_err(|e| format!("网格帧装配失败（clip {}）: {}", i, e))?;
        // D8(1a)：帧级差分注入的变化帧，有效时刻覆写为切换帧时刻（帧级精确段首）；
        // 采样覆盖（sample_time）保留原网格时刻，段尾推进不受段首前移影响。
        // 精化的窗口边界读取 changes[].time，覆写须在精化前完成。
        for (k, t) in time_overrides.iter().enumerate() {
            if let (Some(t), Some(ch)) = (t, changes.get_mut(k)) {
                ch.sample_time = ch.time;
                ch.time = *t;
            }
        }

        // ── 窗口精化：帧级主边界（阶段 1）+ 短字幕召回（阶段 2）────
        // 窗口 = 扫描序列的时间切片（零抽帧零落盘）：
        // - 真正的下一段边界 = 最后一个变化帧（而非首个 main，规避 A→短字幕→C 误判）
        // - 窗口内恰好两次变化时，中间区间为短字幕：≥2 帧则按需抽中间帧召回为独立段
        let refine_started = std::time::Instant::now();
        let (changes, mut short_segments) = if src_fps > 0.0 {
            emit(
                i,
                (i as f64 + 0.55) / clip_count as f64,
                format!("精化边界 {}/{}", i + 1, clip_count),
            );
            refine_window_changes(
                &changes,
                &scan_stream,
                video_path,
                clip,
                video_w,
                video_h,
                src_fps,
                dhash_threshold,
                &clip_dir,
                manager,
                dev_debug,
                // 精化阶段整体占用该 clip 进度 [0.55, 0.60]，随帧聚合上报（消除长静默）
                |done, total, _frac, msg| {
                    let frac = if total == 0 { 0.0 } else { done as f64 / total as f64 };
                    let progress = (i as f64 + 0.55 + frac * 0.05) / clip_count as f64;
                    emit(i, progress, msg);
                },
            )
        } else {
            (changes, Vec::new())
        };
        let refine_dt = refine_started.elapsed();


        emit(
            i,
            (i as f64 + 0.6) / clip_count as f64,
            format!("OCR 识别 {}/{}", i + 1, clip_count),
        );

        let total_frames = changes.len();
        let ocr_started = std::time::Instant::now();
        let mut ipc_time = std::time::Duration::ZERO;
        let mut ipc_batches = 0usize;
        let texts = ocr_pass(
            &changes,
            &jpegs,
            |images| {
                let t = std::time::Instant::now();
                let r = manager.with_provider(|p| p.recognize_batch(images));
                ipc_time += t.elapsed();
                ipc_batches += 1;
                r
            },
            batch_size,
            |done, total| {
                let overall = (i as f64 + done as f64 / total.max(1) as f64) / clip_count as f64;
                emit(
                    i,
                    overall,
                    format!("OCR 识别 {}/{}（{}/{} 帧）", i + 1, clip_count, done, total_frames),
                );
            },
        )
        .map_err(|e| format!("OCR 失败（clip {}）: {}", i, e))?;
        let ocr_dt = ocr_started.elapsed();
        if dev_debug {
            eprintln!(
                "[ocr] clip {}/{}：耗时 共{:.2}s = 扫描{:.2} + 抽帧{:.2} + 精化{:.2} + OCR{:.2}（IPC {:.2}s，{} 批）",
                i + 1,
                clip_count,
                clip_started.elapsed().as_secs_f64(),
                scan_dt.as_secs_f64(),
                extract_dt.as_secs_f64(),
                refine_dt.as_secs_f64(),
                ocr_dt.as_secs_f64(),
                ipc_time.as_secs_f64(),
                ipc_batches
            );
        }

        if dev_debug {
            // 打印每个变化帧的 OCR 结果
            for (change, ft) in changes.iter().zip(texts.iter()) {
                if change.is_changed && !ft.text.is_empty() {
                    eprintln!(
                        "[ocr]   {} conf={:.2} \"{}\"",
                        fmt_time(ft.time),
                        ft.confidence,
                        ft.text
                    );
                }
            }
        }

        let mut segments = merge_frames(texts, frame_interval, clip.end, merge_similarity);
        // 阶段 2：并入窗口内召回的短字幕段，按时间排序
        segments.append(&mut short_segments);
        segments.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // 第二遍：包含关系拼接（同一句的字幕分片合成一段，含误召回的抖动短字幕）
        let segments = merge_contained_adjacent(segments, frame_interval);
        // 第三遍：相邻文本相似合并（消除阶段 2 召回的同句碎片/伪短字幕）
        let segments = merge_similar_adjacent(segments, merge_similarity);
        // 第四遍：短碎片激进合并（弱关联 + 时长/间隔门；保留碎片起点，见函数注释）
        let segments = merge_short_fragments_into_next(segments, min_subtitle_sec);
        // 第五遍（B-ii）：段尾精化——用密帧实测切换时刻替换"末采样+半间隔"的量化估计
        let segments =
            refine_segment_ends(segments, &scan_stream, dhash_threshold, frame_interval, clip.end);
        // 第六遍：精化可能让 start 提前 → clamp 相邻段时间，保证单调不重叠
        let segments = clamp_segment_times(segments);
        if dev_debug {
            eprintln!("[ocr] clip {}/{}：合并 {} 条事件", i + 1, clip_count, segments.len());
            for seg in &segments {
                eprintln!(
                    "[ocr]   事件 [{} → {}] \"{}\"",
                    fmt_time(seg.start),
                    fmt_time(seg.end),
                    seg.text
                );
            }
        }
        all_segments.extend(segments);

        // ── 每 clip 增量清理：本 clip 的帧已全部消费完，及时释放磁盘 ──
        // dev_debug 保留整个目录供人工检查；运行级 TempDirGuard 兜底失败路径
        if !dev_debug {
            let _ = std::fs::remove_dir_all(&clip_dir);
        }
    }

    Ok(all_segments)
}

/// Tauri 命令：串联完整 OCR 流水线。
///
/// 在后台线程跑（不阻塞 UI），过程中通过 `ocr-progress` 事件上报进度，
/// 返回按时间排序的 OcrSegment 列表，由前端写入 ocr_text 轨道。
#[tauri::command]
pub async fn run_ocr(
    app: AppHandle,
    video_path: String,
    video_w: u32,
    video_h: u32,
    region_clips: Vec<OcrRegionInput>,
    params: OcrRunParams,
    // 源视频帧率：窗口精化用。前端已获取元数据，直接传入避免重复探测
    src_fps: f64,
) -> Result<Vec<OcrSegment>, String> {
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<OcrManager>();
        run_ocr_pipeline(
            &manager,
            &video_path,
            video_w,
            video_h,
            &region_clips,
            &params,
            src_fps,
            |clip_index, clip_count, progress, message| {
                let _ = app_handle.emit(
                    OCR_PROGRESS_EVENT,
                    OcrProgress {
                        clip_index,
                        clip_count,
                        progress,
                        message,
                    },
                );
            },
        )
    })
    .await
    .map_err(|e| format!("OCR 任务内部错误: {}", e))?
}

/// 把 OCR 结果拆成语料文本行：每张图的整图文本按换行拆分，
/// trim 后丢弃空行并批内去重（精确匹配）。
/// 行级清洗（白名单/低置信度/重复行）已由 Python worker 完成，这里只做拆分归一。
pub fn lines_from_results(results: &[OcrResult]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for result in results {
        for line in result.text.split('\n') {
            let line = line.trim();
            if line.is_empty() || !seen.insert(line.to_string()) {
                continue;
            }
            lines.push(line.to_string());
        }
    }
    lines
}

/// Tauri 命令：直接识别用户选择的剧情文本截图（语料来源，不经过视频抽帧流水线）。
///
/// 直接读取文件字节并 base64 编码交给 worker（worker 内存解码 ndarray）——
/// 不经临时目录，也根除了 cv2 读图在 Windows 上对非 ASCII 路径不可靠的问题。
#[tauri::command]
pub async fn run_ocr_images(
    app: AppHandle,
    image_paths: Vec<String>,
    batch_size: usize,
) -> Result<Vec<String>, String> {
    if image_paths.is_empty() {
        return Err("未选择图片".into());
    }
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<OcrManager>();
        let ready = manager.with_provider(|p| p.is_ready());
        if !ready {
            return Err("OCR 运行环境未就绪".into());
        }

        let mut images = Vec::with_capacity(image_paths.len());
        for src in &image_paths {
            let bytes =
                std::fs::read(src).map_err(|e| format!("读取图片失败（{}）: {}", src, e))?;
            images.push(b64(&bytes));
        }

        let bs = batch_size.max(1);
        let total_batches = images.len().div_ceil(bs);
        let mut results = Vec::new();
        for (i, chunk) in images.chunks(bs).enumerate() {
            let batch = manager
                .with_provider(|p| p.recognize_batch(chunk))
                .map_err(|e| format!("OCR 失败: {}", e))?;
            results.extend(batch);
            let _ = app_handle.emit(
                OCR_PROGRESS_EVENT,
                OcrProgress {
                    // 与 run_ocr 的 clip_index（1 基）及消息文本 {i+1} 保持一致
                    clip_index: i + 1,
                    clip_count: total_batches,
                    progress: (i + 1) as f64 / total_batches as f64,
                    message: format!("识别截图 {}/{}", i + 1, total_batches),
                },
            );
        }
        Ok(lines_from_results(&results))
    })
    .await
    .map_err(|e| format!("OCR 任务内部错误: {}", e))?
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::dhash::FrameChange;
    use crate::ai_runtime::{OcrProvider, OcrResult, OcrError};

    /// 按调用顺序消费预置结果的 mock provider。
    /// 结果耗尽后返回"短向量"（比请求少），用于触发数量不匹配的错误路径。
    struct MockProvider {
        results: Vec<OcrResult>,
        cursor: std::sync::Mutex<usize>,
    }

    impl OcrProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        fn is_ready(&self) -> bool {
            true
        }
        fn recognize_batch(&self, paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
            let mut cursor = self.cursor.lock().unwrap();
            let remaining = self.results.get(*cursor..).unwrap_or(&[]);
            let count = remaining.len().min(paths.len());
            let out = remaining[..count].to_vec();
            *cursor += count;
            Ok(out)
        }
    }

    fn mock(results: Vec<OcrResult>) -> MockProvider {
        MockProvider {
            results,
            cursor: std::sync::Mutex::new(0),
        }
    }

    fn frame(time: f64, changed: bool) -> FrameChange {
        FrameChange {
            time,
            sample_time: time,
            is_changed: changed,
            base_hash: None,
        }
    }

    fn ft(time: f64, text: &str, confidence: f64) -> FrameText {
        FrameText {
            time,
            sample_time: time,
            text: Arc::from(text),
            confidence,
        }
    }

    // ── ocr_pass ──

    #[test]
    fn test_ocr_pass_carries_forward() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: "B".into(), confidence: 0.8 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, false), frame(2.0, true), frame(3.0, false)];
        let jpegs = vec![b"J1".to_vec(), b"J2".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {}).unwrap();
        assert_eq!(texts.len(), 4);
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[1].text.as_ref(), "A"); // 顺延
        assert_eq!(texts[2].text.as_ref(), "B");
        assert_eq!(texts[3].text.as_ref(), "B"); // 顺延
        assert!((texts[0].confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_ocr_pass_trims_text() {
        let provider = mock(vec![OcrResult { text: "  A  ".into(), confidence: 0.9 }]);
        let changes = vec![frame(0.0, true)];
        let jpegs = vec![b"J".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A"); // 已 trim
    }

    #[test]
    fn test_ocr_pass_empty_change_resets() {
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: String::new(), confidence: 0.0 },
        ]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, false)];
        let jpegs = vec![b"J1".to_vec(), b"J2".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 1, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[1].text.as_ref(), ""); // 变化帧识别为空 → 清空
        assert_eq!(texts[2].text.as_ref(), ""); // 顺延空
    }

    #[test]
    fn test_ocr_pass_batch_mixed_changed() {
        let provider = mock(vec![OcrResult { text: "X".into(), confidence: 0.7 }]);
        let changes = vec![frame(0.0, false), frame(1.0, true)];
        let jpegs = vec![b"J".to_vec()];
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 2, |_, _| {}).unwrap();
        assert_eq!(texts[0].text.as_ref(), ""); // 首帧未变化且无 previous → 空
        assert_eq!(texts[1].text.as_ref(), "X");
    }

    #[test]
    fn test_ocr_pass_result_mismatch_errors() {
        // 预置结果比变化帧少 → 应返回错误而非静默空文本；
        // 图像数据与变化帧数量不符 → 在调用 recognize 前即报错
        let provider = mock(vec![OcrResult { text: "A".into(), confidence: 0.9 }]);
        let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, true)];
        let jpegs = vec![b"J".to_vec()];
        assert!(ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 1, |_, _| {}).is_err());
    }

    // ── assemble_grid_frames（幻影网格帧回收）──

    /// 8 个网格标志：k=0/3/7 为变化帧。模拟 ceil 估算 8 帧、mjpeg 流实际 7 帧。
    fn phantom_flags() -> Vec<(bool, Option<u64>)> {
        let mut flags = vec![(false, None); 8];
        flags[0] = (true, None);
        flags[3] = (true, Some(0xA));
        flags[7] = (true, Some(0xB));
        flags
    }

    #[test]
    fn test_assemble_grid_phantom_changed_recovered() {
        let flags = phantom_flags();
        // 实际 7 帧，kept 只含 <7 的变化帧字节（k=0、k=3）
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let recorded = calls.clone();
        let mut recover = move |t: f64| {
            recorded.borrow_mut().push(t);
            Some(b"J7".to_vec())
        };
        let (changes, jpegs) =
            assemble_grid_frames(&flags, vec![b"J0".to_vec(), b"J3".to_vec()], 7, 10.0, 0.7, &mut recover)
                .unwrap();
        // 7 个实际帧 + 1 个回收的幻影帧
        assert_eq!(changes.len(), 8);
        assert_eq!(jpegs.len(), 3);
        // changes/jpegs 与变化帧按序对齐：k=0、k=3（实际）、k=7（幻影）
        let changed: Vec<usize> = changes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_changed)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(changed, vec![0, 3, 7]);
        assert_eq!(changes[7].time, 10.0 + 7.0 * 0.7);
        assert_eq!(changes[7].base_hash, Some(0xB));
        assert_eq!(jpegs[2], b"J7".to_vec());
        // 幻影帧按网格时间补抽，且只补抽一次
        assert_eq!(calls.borrow().clone(), vec![10.0 + 7.0 * 0.7]);
    }

    #[test]
    fn test_assemble_grid_phantom_unchanged_skipped() {
        let mut flags = phantom_flags();
        flags[7] = (false, None); // 幻影帧未变化 → 无需补抽
        let (changes, jpegs) = assemble_grid_frames(
            &flags,
            vec![b"J0".to_vec(), b"J3".to_vec()],
            7,
            0.0,
            0.7,
            |_| panic!("未变化帧不应触发补抽"),
        )
        .unwrap();
        assert_eq!(changes.len(), 7);
        assert_eq!(jpegs.len(), 2);
    }

    #[test]
    fn test_assemble_grid_recovery_failure_keeps_alignment() {
        let flags = phantom_flags();
        // 补抽失败 → 幻影帧的 change 与 jpeg 同步跳过，对齐不被破坏
        let (changes, jpegs) = assemble_grid_frames(
            &flags,
            vec![b"J0".to_vec(), b"J3".to_vec()],
            7,
            0.0,
            0.7,
            |_| None,
        )
        .unwrap();
        assert_eq!(changes.len(), 7);
        let changed: Vec<usize> = changes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_changed)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(changed, vec![0, 3]);
        assert_eq!(jpegs.len(), 2);
    }

    #[test]
    fn test_assemble_grid_total_matches_no_recovery() {
        let flags = phantom_flags();
        // 估算与实际一致（total=8）→ 无幻影帧，不触发补抽
        let (changes, jpegs) = assemble_grid_frames(
            &flags,
            vec![b"J0".to_vec(), b"J3".to_vec(), b"J7".to_vec()],
            8,
            0.0,
            0.7,
            |_| panic!("total 覆盖全部网格帧时不应触发补抽"),
        )
        .unwrap();
        assert_eq!(changes.len(), 8);
        assert_eq!(jpegs.len(), 3);
    }

    #[test]
    fn test_assemble_grid_mismatch_errors() {
        let flags = phantom_flags();
        // kept 数量与变化帧不符 → 显式报错而非静默错位
        assert!(assemble_grid_frames(&flags, vec![b"J0".to_vec()], 7, 0.0, 0.7, |_| None).is_err());
    }

    #[test]
    fn test_assemble_grid_output_feeds_ocr_pass() {
        // 装配结果（含回收的幻影帧）满足 ocr_pass 契约：数量对齐、文本按序回填
        let flags = phantom_flags();
        let (changes, jpegs) = assemble_grid_frames(
            &flags,
            vec![b"J0".to_vec(), b"J3".to_vec()],
            7,
            0.0,
            0.7,
            |_| Some(b"J7".to_vec()),
        )
        .unwrap();
        let provider = mock(vec![
            OcrResult { text: "A".into(), confidence: 0.9 },
            OcrResult { text: "B".into(), confidence: 0.8 },
            OcrResult { text: "C".into(), confidence: 0.7 },
        ]);
        let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {})
            .unwrap();
        assert_eq!(texts.len(), 8);
        assert_eq!(texts[0].text.as_ref(), "A");
        assert_eq!(texts[3].text.as_ref(), "B");
        assert_eq!(texts[7].text.as_ref(), "C"); // 幻影帧文本正常回填
    }

    // ── merge_frames ──

    #[test]
    fn test_merge_consecutive_same() {
        let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "A", 0.9), ft(3.0, "A", 0.9)];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 3.5).abs() < 1e-9); // 3.0 + 0.5×1.0（覆盖中点）
        assert_eq!(segs[0].text, "A");
    }

    #[test]
    fn test_merge_empty_breaks_and_skips() {
        let frames = vec![
            ft(0.0, "", 0.0),
            ft(1.0, "A", 0.9),
            ft(2.0, "A", 0.9),
            ft(3.0, "", 0.0),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 2.5).abs() < 1e-9); // 2.0 + 0.5
    }

    #[test]
    fn test_merge_text_change_split() {
        let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "B", 0.8), ft(3.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 2);
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[0].end - 1.5).abs() < 1e-9); // 1.0 + 0.5
        assert!((segs[1].start - 2.0).abs() < 1e-9);
        assert!((segs[1].end - 3.5).abs() < 1e-9); // 3.0 + 0.5
    }

    #[test]
    fn test_merge_clamp_to_clip_end() {
        let frames = vec![ft(4.0, "B", 0.8), ft(5.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 5.5, 0.3);
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start - 4.0).abs() < 1e-9);
        assert!((segs[0].end - 5.5).abs() < 1e-9); // min(5+0.5, 5.5)
    }

    #[test]
    fn test_merge_skips_run_beyond_clip() {
        // 段起始已越过 clip_end → 丢弃
        let frames = vec![ft(6.0, "B", 0.8), ft(7.0, "B", 0.8)];
        let segs = merge_frames(frames, 1.0, 5.5, 0.3);
        assert!(segs.is_empty());
    }

    // ── 相似度合并 ──

    #[test]
    fn test_edit_distance_ratio() {
        assert!((edit_distance_ratio("", "") - 0.0).abs() < 1e-9);
        assert!((edit_distance_ratio("abc", "abc") - 0.0).abs() < 1e-9);
        assert!((edit_distance_ratio("abc", "abd") - 1.0 / 3.0).abs() < 1e-9);
        assert!((edit_distance_ratio("abc", "xyz") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_similar_prefix_extends() {
        // 打字机/渐进文本：后续帧是前缀的超集 → 合并为一条，保留最长文本
        let frames = vec![
            ft(1.0, "旅行者，你来了", 0.9),
            ft(2.0, "旅行者，你来了。前方似乎有东西在等待。", 0.9),
            ft(3.0, "旅行者，你来了。前方似乎有东西在等待。", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "旅行者，你来了。前方似乎有东西在等待。");
        assert!((segs[0].end - 3.5).abs() < 1e-9); // 3.0 + 0.5
    }

    #[test]
    fn test_merge_similar_rejects_namebox_line_boundary() {
        // D11 回归：姓名框/头衔态**稳定 ≥2.0s** 后，高置信度帧给出下一条对话行 → 两条独立字幕。
        // 实测形态（glupov 嵌字 #18）：`Anton / Former Acting Captain,"Ninth Company"`
        // ≈40 字符 vs 含对话行的 ≈90 字符，占比 0.44 > 1/3 会顶破 3× 前缀规则
        let namebox = "Anton\nFormer Acting Captain,\"Ninth Company";
        let full = "Anton\nFormer Acting Captain,\"Ninth Company\nNo news. But perhaps... no news is the best news.";
        let frames = vec![
            ft(1.0, namebox, 0.97),
            ft(2.0, namebox, 0.97),
            ft(2.8, namebox, 0.97),
            ft(3.0, full, 0.96),
            ft(4.0, full, 0.96),
        ];
        let segs = merge_frames(frames, 0.5, 30.0, 0.3);
        assert_eq!(segs.len(), 2, "稳定姓名框态与对话行应是两条独立字幕");
        assert_eq!(segs[0].text, namebox);
        assert_eq!(segs[1].text, full);
    }

    #[test]
    fn test_merge_similar_keeps_wrapped_tail_line() {
        // 反向守卫：多出的行是**换行余尾**（远短于短态末行）→ 不判行边界切断，保持同一条目。
        // 实测形态（moon 嵌字 #16）：`runaway princess back to the` → 补出 `moon.`（4 字符）
        let base = "Aria\nlike the villain in a human novel who tries to bring the runaway princess back to the";
        let wrapped = "Aria\nlike the villain in a human novel who tries to bring the runaway princess back to the\nmoon.";
        let frames = vec![
            ft(1.0, base, 0.95),
            ft(3.0, base, 0.95),
            ft(5.0, base, 0.95),
            ft(5.5, wrapped, 0.95),
            ft(6.5, wrapped, 0.95),
        ];
        let segs = merge_frames(frames, 0.5, 30.0, 0.3);
        assert_eq!(segs.len(), 1, "换行余尾应保持同一条目");
        assert_eq!(segs[0].text, wrapped);
    }

    #[test]
    fn test_merge_similar_ignores_low_conf_extra_line() {
        // 置信度门：低置信度帧多出的一行是识别抖动，不触发行边界切断
        // （实测 moon：一帧 conf=0.64 的乱码把已稳定 12.7s 的段落顶开）
        let base = "Sonnet\nthe big deal! We can just leave the work to those Moon Envoys";
        let garbled = "Sonnet\nthe big deal! We can just leave the work to those Moon Envoys\nr";
        let frames = vec![
            ft(1.0, base, 0.97),
            ft(2.0, base, 0.97),
            ft(4.0, base, 0.97),
            ft(4.5, garbled, 0.64),
        ];
        let segs = merge_frames(frames, 0.5, 30.0, 0.3);
        assert_eq!(segs.len(), 1, "低置信度多行不应拆段");
    }

    #[test]
    fn test_merge_similar_keeps_transient_line_boundary() {
        // 反向守卫：同形态但短态**只持续 0.47s**（打字机换行续写）→ 仍属同一条字幕。
        // 实测形态（glupov 语料）：`…第九连队的编制保住`（3 行）→ 补出换行第二行（4 行）
        let partial = "安东\n原「第九连队」临时连长\n层岩巨渊的经历让他们吃了不少苦头，不过万幸，第九连队的编制保住";
        let wrapped = "安东\n原「第九连队」临时连长\n层岩巨渊的经历让他们吃了不少苦头，不过万幸，第九连队的编制保住\n了。我们没有让祖先与陛下蒙羞。";
        let frames = vec![
            ft(1.00, partial, 0.92),
            ft(1.25, partial, 0.93),
            ft(1.50, wrapped, 0.91),
            ft(1.75, wrapped, 0.92),
        ];
        let segs = merge_frames(frames, 0.25, 30.0, 0.3);
        assert_eq!(segs.len(), 1, "打字机换行续写应保持同一条");
        assert_eq!(segs[0].text, wrapped);
    }

    #[test]
    fn test_merge_similar_keeps_partial_last_line() {
        // 末行只写了一半（严格前缀）→ 仍是同一句的渐进补全，应合并为一条
        let frames = vec![
            ft(1.0, "派蒙\n前方似乎有", 0.9),
            ft(2.0, "派蒙\n前方似乎有什么东西在等待。", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "派蒙\n前方似乎有什么东西在等待。");
    }

    #[test]
    fn test_merge_similar_noise_keeps_longest() {
        // OCR 抖动：同一句少一字 → 相似合并，保留更长
        let frames = vec![
            ft(1.0, "前方似乎有什么东西在等待", 0.9),
            ft(2.0, "前方似乎有什么东西在等待。", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "前方似乎有什么东西在等待。");
    }

    #[test]
    fn test_merge_dissimilar_splits() {
        // 完全不同 → 分两段
        let frames = vec![ft(1.0, "甲", 0.9), ft(2.0, "乙", 0.8)];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 2);
    }

    // ── 多数投票 ──

    #[test]
    fn test_vote_ignores_noise_long_text() {
        // 噪声帧更长但与多数帧差异大 → 投票选多数一致文本，不被更长噪声污染
        let frames = vec![
            ft(1.0, "前方似乎有什么东西在等待。", 0.9),
            ft(2.0, "前方似乎有什么东西在等待。", 0.9),
            ft(3.0, "前方似乎有什么东西在等待。前方似乎有什么东X乱码Z", 0.6),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "前方似乎有什么东西在等待。");
    }

    #[test]
    fn test_vote_prefers_progressive_completion() {
        // D10 回归：复刻 glupov 语料实测段（事件 [8.41 → 10.87]）——run 内每个网格帧都
        // 入表、未变化帧顺延上一文本，故三态权重为 2/4/4；medoid 在此形状下落在**中间态**
        // P2（残缺），而完整态 P3 就在同一 run 内、被 4 帧承载且置信度更高（实测 0.86 vs 0.80）
        let p1 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安";
        let p2 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安排我主管一支连队的所有事务，任务";
        let p3 = "斯捷潘尼扬\n「编玛瑙」\n嗯，最近上头安排我主管一支连队的所有事务，任务瞬间复杂了起来。";
        let mut frames = Vec::new();
        let mut t = 1.0;
        for (text, n, cf) in [(p1, 2, 0.83), (p2, 4, 0.80), (p3, 4, 0.86)] {
            for _ in 0..n {
                frames.push(ft(t, text, cf));
                t += 0.25;
            }
        }
        let segs = merge_frames(frames, 0.25, 30.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, p3);
    }

    #[test]
    fn test_vote_progressive_completion_needs_support() {
        // 完整态只被 1 帧承载**且不是 run 末态**（其后还有残缺态帧）→ 不启用渐进偏好，
        // 输出仍是 medoid 胜者（中间态）。守门理由：单帧的"更长"不可信
        let p1 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安";
        let p2 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安排我主管一支连队的所有事务，任务";
        let p3 = "斯捷潘尼扬\n「编玛瑙」\n嗯，最近上头安排我主管一支连队的所有事务，任务瞬间复杂了起来。";
        let mut frames = Vec::new();
        let mut t = 1.0;
        for (text, n, cf) in [(p1, 2, 0.83), (p2, 4, 0.80), (p3, 1, 0.86), (p2, 2, 0.80)] {
            for _ in 0..n {
                frames.push(ft(t, text, cf));
                t += 0.25;
            }
        }
        let segs = merge_frames(frames, 0.25, 30.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, p2);
    }

    #[test]
    fn test_vote_accepts_final_state_completion() {
        // 完整态只被 1 帧承载但**就是 run 末态**（实测 glupov 语料 `…能力显得更关键。`：
        // 尾部亚阈值变化漏检，仅 1.5s 保险丝在段尾补采到一帧，conf 0.86 > 残缺态 0.85）
        let p2 = "斯捷潘尼扬\n「编玛瑙]\n战斗技巧退居到了次要位置，全局意识和任务排期的能";
        let p3 = "斯捷潘尼扬\n「编玛瑙】\n战斗技巧退居到了次要位置，全局意识和任务排期的能力显得更关键。";
        let frames = vec![
            ft(1.00, "斯捷潘尼扬\n「编玛瑙]\n战斗技巧退居到", 0.81),
            ft(1.25, "斯捷潘尼扬\n「编玛瑙]\n战斗技巧退居到了次要位置，全局意", 0.82),
            ft(1.50, p2, 0.85),
            ft(1.75, p2, 0.85),
            ft(4.00, p3, 0.86),
        ];
        let segs = merge_frames(frames, 0.25, 30.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, p3);
    }

    #[test]
    fn test_vote_rejects_low_confidence_completion() {
        // 低置信度"更长候选"被拒（moon 语料实测：`卡侬 / oo / 桑娜妲。…` 承载帧
        // conf 0.65/0.70，而干净态 0.85）——支持度 ≥2 也要过置信度门
        let clean = "卡侬\n桑娜妲。仔细想想，最近这些年，你丢下工作，偷偷跑出去找人类玩的次数，好像越来越多了。";
        let tailed = "卡侬\noo\n桑娜妲。仔细想想，最近这些年，你丢下工作，偷偷跑出去找人类玩的次数，好像越来越多了。";
        let frames = vec![
            ft(1.0, clean, 0.98),
            ft(1.25, clean, 0.85),
            ft(1.5, tailed, 0.65),
            ft(1.75, tailed, 0.70),
            ft(2.0, clean, 0.85),
        ];
        let segs = merge_frames(frames, 0.25, 30.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, clean);
    }

    // ── 短碎片激进合并 ──

    #[test]
    fn test_short_fragment_merges_across_gap() {
        // 实测形态（pierro 4→5 / 21→22）：碎片 0.4s、与其后一条间隔 1.2s（> 既有 0.5s 邻接门），
        // 末行与后条对应行弱关联（少一个空格）→ 并入后条，保留碎片起点
        let segs = vec![
            OcrSegment {
                start: 10.0,
                end: 10.4,
                text: "Stepanyan\nOnyx Agate\nHm? Oh, it's yo".into(),
                confidence: 0.9,
            },
            OcrSegment {
                start: 11.6,
                end: 16.0,
                text: "Stepanyan\nOnyx Agate\nHm? Oh, it's you... Glad to meet you again.".into(),
                confidence: 0.95,
            },
        ];
        let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
        assert_eq!(out.len(), 1);
        assert!((out[0].start - 10.0).abs() < 1e-9, "保留碎片起点");
        assert!((out[0].end - 16.0).abs() < 1e-9, "保留后条终点");
        assert!(out[0].text.contains("Glad to meet you"));
    }

    #[test]
    fn test_short_fragment_keeps_unrelated() {
        // 碎片与其后一条**无关联**（不同句子）→ 不并（保护真实短条不被吞掉）
        let segs = vec![
            OcrSegment { start: 10.0, end: 10.4, text: "派蒙\n嗯".into(), confidence: 0.9 },
            OcrSegment {
                start: 10.6,
                end: 14.0,
                text: "旅行者\n前方似乎有什么东西在等待。".into(),
                confidence: 0.95,
            },
        ];
        let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_short_fragment_keeps_ellipsis_entry() {
        // D9 护栏：末行归一化后为空（纯省略号条）→ 绝不并入下一条，
        // 否则语料出现"缺失"（glupov 语料 [……] 条即此类）
        let segs = vec![
            OcrSegment {
                start: 531.0,
                end: 531.4,
                text: "安东\n原「第九连队」临时连长\n……".into(),
                confidence: 0.9,
            },
            OcrSegment {
                start: 534.0,
                end: 545.0,
                text: "安东\n原「第九连队」临时连长\n没有消息。但也许……没有消息就是最好的消息。".into(),
                confidence: 0.95,
            },
        ];
        let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
        assert_eq!(out.len(), 2, "纯省略号条必须保持独立");
    }

    #[test]
    fn test_similar_text_prefix_ignores_whitespace() {
        // 判据 bug 回归：前缀判据走归一化后，`Onyx Agate` 与 `OnyxAgate`（少一个空格）
        // 仍判为同句延续（实测 glupov 4→5、8→9 与 pierro 4→5、17→18、21→22 因原始
        // starts_with 而漏并）
        let short = "Stepanyan\nOnyx Agate\n嗯？是你啊…居然有幸";
        let long = "Stepanyan\nOnyxAgate\n嗯？是你啊…居然有幸再见面了。";
        assert!(similar_text(short, long, 0.3), "空白差异不应阻断前缀判定");
    }

    #[test]
    fn test_short_fragment_keeps_ocr_misread_head_beyond_gate() {
        // 已知未覆盖（glupov #4，1.25s 且合并时刻时长 >1.3s）：首帧 OCR 把 `ll the` 误读成
        // `u the`，弱关联判据认得出，但时长门取保守值 0.7s → 保持独立。
        // 提门到 1.6s 可合并（嵌字 glupov 58.5→60.8），但会使语料 97.1→92.5（缺失 1）→ 不可接受。
        let segs = vec![
            OcrSegment {
                start: 94.04,
                end: 95.29,
                text: "Stepanyan\nOnyx Agate\nu the".into(),
                confidence: 0.82,
            },
            OcrSegment {
                start: 95.29,
                end: 106.77,
                text: "Stepanyan\nOnyxAgate\nll the tedious, drawn-out paperwork and keeping a unit running smoothly".into(),
                confidence: 0.96,
            },
        ];
        let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
        assert_eq!(out.len(), 2, "超出保守时长门 → 保持独立（已知残留）");
    }

    #[test]
    fn test_short_fragment_keeps_namebox_state_beyond_gate() {
        // 已知未覆盖（moon #5，1.50s 姓名框态）：与语料里 1.48s 姓名框/头衔态**结构完全同形**，
        // 但参考语义相反（moon 参考要求合并、语料参考要求独立）→ 纯文本/时长判据无法两全，
        // 取保守门后两者都保持独立（语料硬门优先）。
        let segs = vec![
            OcrSegment { start: 56.23, end: 57.73, text: "Sonnet\"".into(), confidence: 1.0 },
            OcrSegment {
                start: 57.98,
                end: 68.55,
                text: "Sonnet\n've been having a great time playing around too, Canon!".into(),
                confidence: 0.96,
            },
        ];
        let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
        assert_eq!(out.len(), 2, "姓名框态超保守门 → 保持独立（已知残留）");
    }

    #[test]
    fn test_short_fragment_keeps_stable_namebox() {
        // 3.52s 姓名框态（glupov #18）：超出 1.6s 时长门 → 保持独立（参考亦记为独立条目）
        let segs = vec![
            OcrSegment {
                start: 531.08,
                end: 534.60,
                text: "Anton\nFormer Acting Captain,\"Ninth Company".into(),
                confidence: 0.97,
            },
            OcrSegment {
                start: 535.03,
                end: 545.33,
                text: "Anton\nFormer Acting Captain,\"Ninth Company\nNo news. But perhaps... no news is the best news.".into(),
                confidence: 0.96,
            },
        ];
        let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
        assert_eq!(out.len(), 2, "稳定姓名框态（3.52s）应保持独立");
    }

    // ── 段尾精化（B-ii）──

    #[test]
    fn test_refine_segment_ends_uses_dense_switch() {
        // 段尾估计 = 末采样(10.0) + interval×0.5 = 10.25；密帧实测切换在 10.10 → 取 10.10
        let segs = vec![OcrSegment { start: 8.0, end: 10.25, text: "X".into(), confidence: 0.9 }];
        let a = 0x0u64;
        let b = 0xFFFF_FFFF_FFFF_FFFFu64; // 汉明距离 64 > 阈值
        let dense = vec![(9.0, a), (10.0, a), (10.1, b), (10.2, b)];
        let out = refine_segment_ends(segs, &dense, 3, 0.5, 30.0);
        assert!((out[0].end - 10.1).abs() < 1e-9, "实得 {}", out[0].end);
    }

    #[test]
    fn test_refine_segment_ends_keeps_estimate_without_switch() {
        // 窗口内无越阈（本段延续到 clip 末尾）→ 保留原估计
        let segs = vec![OcrSegment { start: 8.0, end: 10.25, text: "X".into(), confidence: 0.9 }];
        let a = 0x0u64;
        let dense = vec![(9.0, a), (10.0, a), (11.0, a)];
        let out = refine_segment_ends(segs, &dense, 3, 0.5, 30.0);
        assert!((out[0].end - 10.25).abs() < 1e-9, "实得 {}", out[0].end);
    }

    // ── 空帧容错 ──

    #[test]
    fn test_merge_empty_tolerance_keeps_run() {
        // 单帧空（interval 0.5 → 容错窗口 0.8s）视为 OCR 抖动，不打断 run
        let frames = vec![
            ft(1.0, "A", 0.9),
            ft(2.0, "A", 0.9),
            ft(2.5, "", 0.0),
            ft(3.0, "A", 0.9),
        ];
        let segs = merge_frames(frames, 0.5, 10.0, 0.3);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "A");
        assert!((segs[0].end - 3.25).abs() < 1e-9); // last=3.0 + 0.5×interval 0.25
    }

    #[test]
    fn test_merge_empty_beyond_tolerance_breaks() {
        // 连续空帧超过容错窗口（1.0s 间隔 → 窗口 1.5s）→ 打断
        let frames = vec![
            ft(1.0, "A", 0.9),
            ft(2.0, "A", 0.9),
            ft(3.0, "", 0.0),
            ft(4.0, "", 0.0),
            ft(5.0, "", 0.0),
            ft(6.0, "B", 0.9),
        ];
        let segs = merge_frames(frames, 1.0, 10.0, 0.3);
        assert_eq!(segs.len(), 2);
    }

    // ── 相邻段包含关系拼接（碎片化修复）──

    #[test]
    fn test_merge_contained_absorb_progressive() {
        // 渐进中间态短段在前、完整句在后 → 并入（诊断案例：卡侬·那是我本职工）
        let segs = vec![
            OcrSegment { start: 1.0, end: 2.0, text: "卡侬\n·那是我本职工".into(), confidence: 0.8 },
            OcrSegment { start: 2.0, end: 9.0, text: "卡侬\n…那是我本职工作的一部分。他们的主祭呼唤我的名字。".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.starts_with("卡侬\n…那是我本职工作的一部分"));
        assert!((out[0].start - 1.0).abs() < 1e-9);
        assert!((out[0].end - 9.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_contained_absorb_residual() {
        // 完整句在前、尾部残缺短段在后 → 并入（文本不变，时间取并集）
        let segs = vec![
            OcrSegment { start: 1.0, end: 8.0, text: "卡侬\n…那是我本职工作的一部分。他们的主祭呼唤我的名字。".into(), confidence: 0.9 },
            OcrSegment { start: 8.0, end: 9.0, text: "呼唤我的名字".into(), confidence: 0.8 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert!((out[0].end - 9.0).abs() < 1e-9);
    }

    #[test]
    fn test_merge_contained_absorb_long_residual_tail_match() {
        // D1 第 2 步定稿正例：moon clip 2/5 的 **pre-merge 真实形态**（段序 dump）——
        // 残尾 [01:52.067→01:53.734] 时长 1.667s（> 2.5×interval，最终事件显示的
        // 1.467s 是 clamp 假象），字尾窗比率 0.094 → 走长残尾档并入
        let segs = vec![
            OcrSegment { start: 97.001, end: 112.234, text: "Aria\nng to be an ordinary human girl and dancing in front of your own statue also\none of your duties? That's news to me.".into(), confidence: 0.9 },
            OcrSegment { start: 112.067, end: 113.734, text: "Aria\none of\nyour duties? That's news to me.".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1, "1.667s 句尾重读残尾（≤2.0s 门 + 字尾匹配）应并入");
        assert!(out[0].text.contains("dancing in front of your own"), "文本不变");
        assert!((out[0].end - 113.734).abs() < 1e-9, "时间取并集");
    }

    #[test]
    fn test_merge_contained_nested_residual_does_not_shrink() {
        // D11 回归：pierro 实测段序——残片 #81 经方向 1 吸收完整长段 #82（并集
        // [484.55, 549.32]）后，又来了一个**嵌套在长段内**的短残片 #83
        // （[486.73, 486.98]）；方向 2 若用赋值 `last.end = seg.end` 会把
        // 64.8s 跨度截断成 2.4s（产出 62.5s 空窗、参考 65.7s 条目满扣）
        let segs = vec![
            OcrSegment { start: 484.55, end: 486.32, text: "Mitya\nre of the curse seems to be something like... when m".into(), confidence: 0.9 },
            OcrSegment { start: 486.45, end: 549.32, text: "Mitya\nire of the curse seems to be something like... when moments of technological progress occur, all life in the vicinity is taken away.".into(), confidence: 0.95 },
            OcrSegment { start: 486.73, end: 486.98, text: "Mitya\nire of the curse seems to be something like... when".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1, "三段应并成一段");
        assert!(
            (out[0].end - 549.32).abs() < 1e-9,
            "嵌套残片不得截断已累积跨度（实得 {}）",
            out[0].end
        );
        assert!((out[0].start - 484.55).abs() < 1e-9, "保留最早起点");
    }

    #[test]
    fn test_merge_contained_rejects_namebox_state_as_residual() {
        // 独立姓名框态（对话行清空后只剩姓名框）不得被方向 2 吞并：其文本是任何带
        // 姓名框长段的子序列。glupov 语料 [18]（……省略号条的匹配项）即靠该独立态
        // 配对——放宽时长门曾把它吞成缺失 1（2026-09-16 实证，见 D1 节第 2 步）
        let segs = vec![
            OcrSegment { start: 1.0, end: 9.0, text: "Sonnet\nI've taken a shine to our new master, sisters, and they're willing to send subordinates to share our work".into(), confidence: 0.9 },
            OcrSegment { start: 9.1, end: 12.2, text: "Sonnet".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2, "独立姓名框态保持独立（3.1s 超 2.0s 上界）");
    }

    #[test]
    fn test_merge_contained_rejects_namebox_short_state() {
        // 关键守护：glupov 语料 P17/P18 实测对（1.483s ≤ 2.0s 上界，**时长门放行**）
        // ——必须由"字尾匹配"拒绝：姓名框是前段的**字头**重复，尾窗比率 ≈1。这条
        // 测试即第 2 步首版（纯时长门放宽）吞掉 [18] 造成缺失 1 的回归守卫
        let segs = vec![
            OcrSegment { start: 60.7, end: 64.5, text: "安东\n原「第九连队」临时连长\n等大家恢复了精神，我们会随时准备迎接新的指令。直到陛下的宏愿实现，我们也许会死去，但不会被击垮。".into(), confidence: 0.9 },
            OcrSegment { start: 64.465, end: 65.948, text: "安东\n原「第九连队」临时连长".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2, "姓名框字头态须由字尾匹配拒绝（[18] 依赖它配对）");
    }

    #[test]
    fn test_merge_contained_case_insensitive_progressive() {
        // 实况日志真实碎片对（pierro 44:01.556）：英文大小写/标点差异下仍应拼接
        let segs = vec![
            OcrSegment { start: 1.0, end: 1.484, text: "Paimon\n\"AL\"".into(), confidence: 0.9 },
            OcrSegment { start: 1.484, end: 3.435, text: "Paimon\n\"Allies\"?".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("Allies"));
        assert!((out[0].start - 1.0).abs() < 1e-9);
        assert!((out[0].end - 3.435).abs() < 1e-9);
    }

    #[test]
    fn test_merge_contained_accepts_span_within_2_5x() {
        // 1.057s（2.11×interval）的真实碎片在 2.5×interval 护栏内 → 应拼接
        let segs = vec![
            OcrSegment { start: 1.0, end: 2.057, text: "Paimon\nal!? Uh..A—Actually, maybe it's best no".into(), confidence: 0.9 },
            OcrSegment { start: 2.057, end: 15.0, text: "Paimon\nal!? Uh... A—Actually, maybe it's best not to...? What if Ronova suddenly pop up again? She might take this chance to".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1, "1.057s ≤ 1.25s 护栏应拼接");
    }

    #[test]
    fn test_merge_contained_rejects_cross_gap() {
        // 跨 2.86s 空档（远超 interval）即使文本包含也不拼接（决策：不覆盖此类碎片）
        let segs = vec![
            OcrSegment { start: 1.0, end: 2.029, text: "The Jester\nnthe circumstances, the Fat".into(), confidence: 0.9 },
            OcrSegment { start: 4.886, end: 30.0, text: "The Jester\nn the circumstances, the Fatui need not hide anything from our allies.".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2, "空档 2.857s > 0.5s 应拒绝拼接");
    }

    #[test]
    fn test_merge_contained_keeps_complete_brief_entry() {
        // 短暂但完整的独立条目：包含关系成立但时长 3s 超护栏 → 保留（护栏取代 1/3 长度比守卫）
        let segs = vec![
            OcrSegment { start: 1.0, end: 4.0, text: "派蒙".into(), confidence: 0.9 },
            OcrSegment { start: 4.0, end: 9.0, text: "派蒙：旅行者你来了".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2, "3s > 1.25s 护栏应保留为独立条目");
    }

    #[test]
    fn test_merge_contained_keeps_shared_prefix_distinct() {
        // 共享前缀但非包含的相邻短条 → 不并（由包含判据本身拒绝）
        let segs = vec![
            OcrSegment { start: 1.0, end: 1.4, text: "派蒙\n嗯？".into(), confidence: 0.9 },
            OcrSegment { start: 1.4, end: 1.8, text: "派蒙\n最近上头安排我主管一支连队".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_merge_contained_takes_full_text() {
        // 无置信度守卫：方向 1 一律取更长（更完整）文本——语料 A/B 证明守卫会在更长
        // 文本置信度仅略低时保留较短残片（0.93 vs 0.94 属 OCR 噪声），使完整句从语料消失
        let segs = vec![
            OcrSegment { start: 1.0, end: 1.5, text: "Paimon\nto challe".into(), confidence: 0.95 },
            OcrSegment { start: 1.5, end: 3.0, text: "Paimon\nto challenge the Ruler of Death".into(), confidence: 0.60 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Paimon\nto challenge the Ruler of Death", "方向 1 取完整文本");
        assert!((out[0].end - 3.0).abs() < 1e-9, "时间取并集");
    }

    #[test]
    fn test_merge_contained_keeps_distinct_short() {
        // 真短句（非包含）不并入
        let segs = vec![
            OcrSegment { start: 1.0, end: 2.0, text: "嗯？".into(), confidence: 0.9 },
            OcrSegment { start: 2.0, end: 9.0, text: "我们出发吧。".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_is_line_prefix_cases() {
        // 逐行前缀 + 末行严格前缀（大小写/空白/标点容忍）
        assert!(is_line_prefix("A\nb", "a\nbc"));
        assert!(is_line_prefix("A\nbC", "a\nbcd e"));
        // D1 第 1 步：共享区 1 字符错位（编辑距离比 0.067~0.15）不再阻断
        // 已证案例（glupov/pierro 嵌字）：末行错位（替换）
        assert!(is_line_prefix(
            "Anton\ncompany\nnt they deserted t",
            "Anton\ncompany\nat they deserted the Fatui"
        ));
        // 已证案例：先前行的错位不再阻断整段判定
        assert!(is_line_prefix(
            "Anton\ncompany\nMitya\nDne has recovered their strength",
            "Anton\ncompany\nMitya\none has recovered their strength, we will be ready"
        ));
        // 末行整行相等 = 行边界切断（对话行清空状态）→ 否
        assert!(!is_line_prefix("a\nbc", "a\nbc\nde"));
        // 中段行差异过大（1/1 = 1.0 > 0.2）→ 否
        assert!(!is_line_prefix("a\nx\nbc", "a\ny\nbcd"));
        // 中段行差异过大（不同句，比率 ≈1.0 > 0.2）→ 否
        assert!(!is_line_prefix(
            "a\n今天天气很好",
            "a\n明天应该会下雨吧再说\n continued text here"
        ));
        // 前段行数多于后段 → 否
        assert!(!is_line_prefix("a\nb\nc", "a\nbc"));
        // 单行（派蒙 型独立短条）→ 否
        assert!(!is_line_prefix("派蒙", "派蒙：旅行者你来了"));
        // 极短末行 1 字符错位（比率 0.5 > 0.2）→ 保守拒绝
        assert!(!is_line_prefix("a\nbc", "a\nxcdef"));
        // 空行/空文本 → 否
        assert!(!is_line_prefix("a\n", "a\nb"));
    }

    #[test]
    fn test_merge_contained_long_fragment_line_prefix() {
        // D9：glupov 嵌字型——残片被漏检拖长到 3.2s（超 2.5×interval 护栏），
        // 但末行是完整段末行的严格前缀 → 仍判为同一句补全，拼接
        let segs = vec![
            OcrSegment { start: 0.0, end: 3.2, text: "Anton\nFormer Acting Captain,\"Ninth Company\nIf they are still alive, I h".into(), confidence: 0.9 },
            OcrSegment { start: 3.41, end: 4.2, text: "Anton\nFormer Acting Captain,\"Ninth Company'\nIf they are still alive, I hope they never come back here.".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1, "行级前缀应拼掉 3.2s 残片");
        assert!((out[0].start - 0.0).abs() < 1e-9, "保留前段起点");
        assert!((out[0].end - 4.2).abs() < 1e-9, "时间取并集");
        assert!(out[0].text.contains("I hope they never come back here."), "取完整文本");
    }

    #[test]
    fn test_merge_contained_wrap_extra_line() {
        // 完整段因换行多出一行（3 段 vs 4 段）：末行仍为严格前缀 → 拼接
        let segs = vec![
            OcrSegment { start: 0.0, end: 1.0, text: "Anton\nFormer Acting Captain,\"Ninth Company\nlurderofBirds. Inever thought I'".into(), confidence: 0.9 },
            OcrSegment { start: 1.2, end: 2.0, text: "Anton\nFormer Acting Captain,\"Ninth Company\nlurderofBirds. I never thought I'd see you again here in Snezhnograd. What a\npleasant surprise.".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("pleasant surprise."));
    }

    #[test]
    fn test_merge_contained_line_boundary_keeps_distinct() {
        // 对话行清空、只剩姓名框的独立状态：末行整行相等 → 行边界切断，不并入
        let segs = vec![
            OcrSegment { start: 0.0, end: 3.2, text: "安东\n原「第九连队」临时连长".into(), confidence: 0.9 },
            OcrSegment { start: 3.41, end: 4.2, text: "安东\n原「第九连队」临时连长\n没有消息。但也许……没有消息就是最好的消息。".into(), confidence: 0.9 },
        ];
        let out = merge_contained_adjacent(segs, 0.5);
        assert_eq!(out.len(), 2, "独立姓名框状态应保留");
    }

    // ── 相邻段时间 clamp（精化后防重叠）──

    #[test]
    fn test_recall_eligible_gates() {
        // 60fps 密集采样：dt = 1/60 ≈ 0.0167s，min_span=0.2s（12 帧），min_side=0.1s（6 帧）
        let dt = 1.0 / 60.0;
        // 窗口过窄：s 或 m 贴边
        assert!(!recall_eligible(1, 2, 30, dt, 0.2, 0.1)); // s == m+1
        assert!(!recall_eligible(1, 3, 30, dt, 0.2, 0.1)); // span = 0.033 < 0.2
        assert!(!recall_eligible(3, 15, 30, dt, 0.2, 0.1)); // a_side = 0.05 < 0.1
        assert!(!recall_eligible(8, 28, 30, dt, 0.2, 0.1)); // c_side = 0.033 < 0.1
        assert!(!recall_eligible(7, 28, 28, dt, 0.2, 0.1)); // s == wlen 贴尾
        // 三边都够：a=0.117, span=0.2, c=0.15 → 可召回
        assert!(recall_eligible(7, 19, 28, dt, 0.2, 0.1));
        // 30fps：span 0.2s = 6 帧
        let dt30 = 1.0 / 30.0;
        assert!(!recall_eligible(7, 12, 28, dt30, 0.2, 0.1)); // span = 0.167 < 0.2
        assert!(recall_eligible(7, 14, 28, dt30, 0.2, 0.1)); // span = 0.233 √
    }

    #[test]
    fn test_clamp_segment_times_no_overlap() {
        // 前段 end 与后段 start 交叉 → clamp 到后段 start
        let segs = vec![
            OcrSegment { start: 1.0, end: 5.0, text: "A".into(), confidence: 0.9 },
            OcrSegment { start: 3.0, end: 8.0, text: "B".into(), confidence: 0.9 },
        ];
        let out = clamp_segment_times(segs);
        assert_eq!(out.len(), 2);
        assert!((out[0].end - 3.0).abs() < 1e-9);
        assert!((out[1].start - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_clamp_segment_times_keeps_gap() {
        // 本就无重叠 → 时间不变
        let segs = vec![
            OcrSegment { start: 1.0, end: 3.0, text: "A".into(), confidence: 0.9 },
            OcrSegment { start: 5.0, end: 9.0, text: "B".into(), confidence: 0.9 },
        ];
        let out = clamp_segment_times(segs);
        assert!((out[0].end - 3.0).abs() < 1e-9);
        assert!((out[1].end - 9.0).abs() < 1e-9);
    }

    // ── 相邻相似合并（阶段 2 去伪短字幕）──

    #[test]
    fn test_merge_similar_adjacent_merges_same_text() {
        // 完全相同文本的相邻段 → 合并为一段（时间取并集）
        let segs = vec![
            OcrSegment { start: 1.0, end: 3.0, text: "卡侬\n…那是我本职工作的一部分。".into(), confidence: 0.9 },
            OcrSegment { start: 3.0, end: 3.2, text: "卡侬\n…那是我本职工作的一部分。".into(), confidence: 0.9 },
        ];
        let out = merge_similar_adjacent(segs, 0.3);
        assert_eq!(out.len(), 1);
        assert!((out[0].end - 3.2).abs() < 1e-9);
    }

    #[test]
    fn test_merge_similar_adjacent_keeps_distinct() {
        // 文本差异大的相邻段 → 不合并
        let segs = vec![
            OcrSegment { start: 1.0, end: 3.0, text: "卡侬".into(), confidence: 0.9 },
            OcrSegment { start: 3.0, end: 5.0, text: "获得".into(), confidence: 0.9 },
        ];
        let out = merge_similar_adjacent(segs, 0.3);
        assert_eq!(out.len(), 2);
    }

    // ── lines_from_results ──

    #[test]
    fn test_lines_from_results_split_and_dedup() {
        // 拆行、trim、丢空行；批内及跨图去重（精确匹配）
        let results = vec![
            OcrResult {
                text: "旅行者，你来了\n\n  派蒙：  \n旅行者，你来了".into(),
                confidence: 0.9,
            },
            OcrResult {
                text: "  第二张的行  \n旅行者，你来了".into(),
                confidence: 0.8,
            },
        ];
        let lines = lines_from_results(&results);
        assert_eq!(lines, vec!["旅行者，你来了", "派蒙：", "第二张的行"]);
    }

    #[test]
    fn test_lines_from_results_empty() {
        assert!(lines_from_results(&[]).is_empty());
        let blank = vec![OcrResult { text: "  \n\n".into(), confidence: 0.5 }];
        assert!(lines_from_results(&blank).is_empty());
    }
}
