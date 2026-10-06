use std::sync::Arc;
use super::params::{FrameText, OcrSegment};

/// 计算两个字符串的归一化编辑距离比例（0=相同，1=完全不同）
pub(crate) fn edit_distance_ratio(a: &str, b: &str) -> f64 {
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
pub(crate) fn is_line_boundary_cut(short: &str, long: &str) -> bool {
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
pub(crate) const LINE_BOUNDARY_STABLE_SEC: f64 = 2.0;

/// 触发行边界切断所需的**新文本帧**最低 OCR 置信度。
///
/// 低置信度帧多出的"一行"更可能是识别抖动而非真实新字幕：实测 moon 一帧
/// `conf=0.64` 的乱码把已稳定 12.7s 的段落顶开成两段（1:1 18→15、碎片 1→4）。
pub(crate) const LINE_BOUNDARY_MIN_CONF: f64 = 0.8;

/// run 内文本是否仍在**净增长**（打字机链）：末帧的归一化长度严格大于首帧。
///
/// 用途（D12 收口）：`is_stable_line_boundary_cut` 只看 (短态, 长态) 这一文本对与短态时长，
/// 而"打字机正打到一半"的短态同样满足它——实测 pierro 嵌字 `[1167.99 → 1170.50]`
/// 的 run 归一化长度为 `23→38→55→65→65`（对话行仍在逐字补全），此时完整段
/// （4 行 / 150 字符 / conf 0.954）出现即被误判为独立新字幕，同一条字幕产出两段
/// （`[1167.77, 1170.67]` + `[1170.67, 1187.28]`），下游跨语言对齐因此整体错位。
///
/// 与 D12 正当用例的判别（本护栏唯一的判别依据）：glupov #17 的姓名框/头衔态在 run 内
/// **逐帧同形**——实测归一化长度 `36×8`（OCR 把 `conf=0.000` 的「……」行丢弃后只剩姓名框），
/// 净增长为 0 → 本护栏不生效，时长门照旧判独立（正确）。
///
/// 取"首帧 → 末帧**净**增长"而不取"存在长度不同的帧"：OCR 抖动会让个别帧多/少一两个字符，
/// 只有真实打字机链才会在 run 首尾之间留下净增量。方向偏保守——返回 false 时行为与
/// 引入本护栏之前**逐位相同**（门槛照旧生效）。
///
/// 仅 `merge_frames` 能用：它的 `Run.texts` 是 run 内全部帧文本；`merge_similar_adjacent`
/// 手里只有已定型的事件序列，没有 run 内增长轨迹。
pub(crate) fn run_text_growing(texts: &[(Arc<str>, f64)]) -> bool {
    match (texts.first(), texts.last()) {
        (Some((first, _)), Some((last, _))) => norm_chars(last).len() > norm_chars(first).len(),
        _ => false,
    }
}

/// 稳定时长门 + 置信度门 + 行边界切断：三者同时成立才判为独立新字幕。
pub(crate) fn is_stable_line_boundary_cut(
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
pub(crate) fn similar_text(a: &str, b: &str, threshold: f64) -> bool {
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
pub(crate) const SAME_TEXT_TOLERANCE: f64 = 0.1;

/// 渐进补全偏好：更长（更完整）候选的承载门槛。
///
/// 两道通道之一即可：
/// 1. **≥2 帧承载**——打字机补全通常会被后续网格/保险丝多次采到；**实测唯一真正放行过候选的通道**；
/// 2. **是 run 的末态**（与 `texts.last()` 近似同文）——显示期的最终形态即该行完整文本。
///    ⚠ **通道 2 目前无实证（A5，2026-09-29 复核）**：曾以 glupov 语料 `…能力显得更关键。`
///    为例（称该行尾部变化被 9×8 dHash 漏检、仅保险丝补采 1 帧、只能走此通道），该例已证伪——
///    默认 0.5s 网格下该 run（13.38–15.49s，n=4）的 **medoid 本身就是完整态**，无需偏好介入；
///    且台账 D10 残留记录显示 0.25s 下保险丝那一帧读到的仍是残缺态。A5 通道计数：glupov 语料
///    25 个 run 全部 KEEP medoid（0 次放行）；嵌字侧实际放行均由通道 1 完成（实测 support=2）。
///    故通道 2 仅作"末态兜底"保留，必要性由 0.25s 网格受控 A/B（A5-P4）裁决。
///
/// 单帧幻觉尾巴（`test_vote_ignores_noise_long_text` 的乱码行）虽可能是末态，但会
/// 被下面的置信度门挡掉。
pub(crate) const PROGRESSIVE_COMPLETE_MIN_SUPPORT: usize = 2;

/// 渐进补全偏好：更长候选的平均置信度不得低于 medoid 胜者超过此幅度。
///
/// 守门依据（2026-09-18 语料实测）：**幻觉尾巴帧的 OCR 置信度显著更低**——
/// moon 语料 `卡侬 / oo / 桑娜妲。…` 的承载帧 conf 0.65/0.70、`…也没什么意 / 见。o`
/// 的承载帧 conf 0.50/0.63；而真实渐进补全（glupov `…所有事务，任务瞬间复杂了起来。`）
/// 的完整态 conf 0.86 反而**高于**残缺态 0.80。故以"置信度不劣于胜者"为第二道门，
/// 避免"更长者优先"把 D4 污染文本翻案。
///
/// 网格归属（A5 补注，2026-09-29）：上述数值取自 **0.25s 细网格实验期**（本偏好即为该网格而加）；
/// 默认 0.5s 网格下本门从未被触发——glupov 语料 25 个 run 的候选全部先被 `MIN_GAIN` 挡在门外。
/// 另：A5 曾假设"渐进门换文本时带走了候选帧置信度、经 `merge_similar_adjacent` 的 conf 守卫级联
/// 翻转相邻合并"，该假设已被受控实验**证伪**（只把置信度锚点换回 medoid 帧、文本不变，
/// moon 产物逐字节相同）。
pub(crate) const PROGRESSIVE_COMPLETE_CONF_MARGIN: f64 = 0.10;

/// 渐进补全偏好：更长候选相对 medoid 胜者的**最小归一化字符增量**。
///
/// 取值依据（2026-09-18 语料实测）：真实打字机补全的增量远大于此——
/// `…所有事务，任务` → `…所有事务，任务瞬间复杂了起来。` 增量 9 字符、
/// `…任务排期的能` → `…任务排期的能力显得更关键。` 增量 6 字符；
/// 而 OCR 抖动变体（全半角括号、个别字错读）增量仅 0~1 字符。
/// 0.5s 网格 A/B 实测：无此门时把一条原本"正确"（sim≥0.98）的条目换成了略偏的长变体
/// （语料 97.1→96.9）；加门后不误换。
///
/// A5 复核（2026-09-29）：本门**承重且已单测钉住**——
/// `test_vote_progressive_min_gain_guards_noise_variant` 用真实素材（glupov `原「第九连队」临时连长`
/// 被 OCR 读成 `原「第九连队J临时连长`）做正反两向锁死；临时把本常量改为 0，该用例立即变红并
/// 复现语料 97.1→96.9 的那一条。默认 0.5s 网格下它是**唯一**真正拦下候选的门。
pub(crate) const PROGRESSIVE_COMPLETE_MIN_GAIN: usize = 3;

/// 从 run 内所有相似帧文本做多数投票：选与其它帧总相似度最高者（medoid）。
///
/// 替代"更长者胜出"：被噪声污染的更长文本与多数帧差异大，不会被选中；
/// 完全相同的候选平局时取更长（与旧行为兼容）。
///
/// **渐进补全偏好（D10，2026-09-18）**：medoid 在"打字机渐进链" P1 ⊂ P2 ⊂ P3 上
/// **数学上必然落在中间态**（中间态到各态的平均距离最小），于是 run 内明明采到了
/// 完整态，胜出的却是残缺文本。0.5s 网格下链短，medoid 常恰好落在完整态——**A5 实测该网格
/// 下本偏好在语料侧完全惰性**（glupov 25 个 run 全部 KEEP medoid；三案例语料产出与关闭偏好
/// 时逐字节相同）；网格降到 0.25s 后链变长，glupov 语料因此出现两条截断配对
/// （sim 0.778/0.811，扣 0.41 = 全部回归）。
/// 故在 medoid 之后补一步：若存在"严格更长、近似包含 medoid 胜者、且被 ≥2 帧承载"
/// 的候选，取其中最长者——即该行显示期间的**最终完整态**（语料/翻译所需的形态）。
/// 渐进补全偏好的**调试开关**：`GSA_OCR_PROGRESSIVE_COMPLETE=0/false/off/no` 关闭本步。
///
/// 用途（A5）：**同一次构建**下做受控 A/B——关闭时 `vote_text` 直接返回 medoid 胜者，行为
/// 等同本偏好引入之前的状态，避免以往"两臂代码不同、结论被别的改动污染"的问题。
/// 默认开启。解析逻辑抽成纯函数 `progressive_complete_enabled_from`，便于单测（不去改
/// 进程环境变量，避免测试并发竞态）。
pub(crate) fn progressive_complete_enabled_from(v: Option<&str>) -> bool {
    !matches!(
        v.unwrap_or("").trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

pub(crate) fn progressive_complete_enabled() -> bool {
    progressive_complete_enabled_from(std::env::var("GSA_OCR_PROGRESSIVE_COMPLETE").ok().as_deref())
}

pub(crate) fn vote_text(texts: &[(Arc<str>, f64)]) -> (Arc<str>, f64) {
    vote_text_with(texts, progressive_complete_enabled())
}

/// `vote_text` 的实现体：`progressive` = 是否启用渐进补全偏好（生产路径取环境开关）。
pub(crate) fn vote_text_with(texts: &[(Arc<str>, f64)], progressive: bool) -> (Arc<str>, f64) {
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
    // （可用 GSA_OCR_PROGRESSIVE_COMPLETE=0 关闭，供同构建受控 A/B）
    let bn = norm_chars(&best.0);
    if !bn.is_empty() && progressive {
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
            let longer = winner.is_none_or(|w| norm_chars(&w.0).len() < cn.len());
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
            // 稳定 ≥2.0s 的姓名框/头衔态之后出现整行新文本 = 独立新字幕（D11），run 在此断开。
            // 但 run 内文本仍在增长（打字机链）时短态**不是**稳定态：多出的行是同一句正被
            // 逐字补全的后半段，不切断（D12 收口，判据见 `run_text_growing`）。
            let stable_boundary = is_stable_line_boundary_cut(
                base,
                f.text.as_ref(),
                f.time - r.start,
                f.confidence,
            ) && !run_text_growing(&r.texts);
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
pub(crate) fn norm_chars(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 判断 a 的字符是否按序出现在 b 中（近似前缀/渐进片段匹配，双指针）
pub(crate) fn is_subsequence(a: &[char], b: &[char]) -> bool {
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
///
/// 网格归属（A5 补注）：默认 0.5s 网格下两档**恰好相等**（1.25 = 2.5×0.5），即 factor 项
/// 在当前发布配置下不改变任何判定，只为 0.25s 细网格预留——读代码时不必怀疑它在 0.5s 下的作用。
pub(crate) const PROGRESSIVE_MAX_SPAN_SEC: f64 = 1.25;
pub(crate) const PROGRESSIVE_MAX_SPAN_FACTOR: f64 = 2.5;

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
///
/// 网格归属（A5 补注）：与上一常量同理，默认 0.5s 下两档恰好相等（2.0 = 4.0×0.5），
/// factor 项仅在 0.25s 细网格生效。
pub(crate) const RESIDUAL_MAX_SPAN_SEC: f64 = 2.0;
pub(crate) const RESIDUAL_MAX_SPAN_FACTOR: f64 = 4.0;

/// 相邻段拼接的时间邻接窗（秒，**绝对秒下限**）：两段间空档大于此值即视为不同条目。
/// 0.5s = 原 0.5s 网格下一格；网格降为 0.25s 后仍保持 0.5s（显示行为的绝对尺度）。
pub(crate) const CONTIGUOUS_MAX_GAP_SEC: f64 = 0.5;

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
pub(crate) const LINE_PREFIX_EDIT_TOLERANCE: f64 = 0.2;

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
pub(crate) fn is_line_prefix(prev: &str, next: &str) -> bool {
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

/// 短碎片与其后一条允许的时间间隔上限（秒）：实测存在 0.5~1.5s 的检测空洞
/// （pierro 4→5、21→22 段间有空间隔，被既有 0.5s 邻接门挡掉而漏并）。
pub(crate) const SHORT_FRAGMENT_GAP_MAX_SEC: f64 = 2.0;

/// 末行模糊前缀容差：覆盖 OCR 误读 1~2 字符（长句下比率远小于此）
pub(crate) const SHORT_FRAGMENT_PREFIX_TOL: f64 = 0.3;

/// 末行字符重叠率下限：覆盖乱序/替换型误读
pub(crate) const SHORT_FRAGMENT_OVERLAP: f64 = 0.5;

/// 字符多重集重叠率：`Σ min(count_a, count_b) / |a|`
pub(crate) fn overlap_ratio(a: &[char], b: &[char]) -> f64 {
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

/// 短碎片与后一条的**弱关联**判定（比较行层面，另加前置行护栏）。
///
/// 三条任一成立即算关联：归一化子序列（含空白/标点差异）、模糊前缀（OCR 误读 1~2 字符）、
/// 字符重叠率 ≥0.5（乱序/替换型误读）。比较行 = 碎片中**最后一个归一化长度 ≥2 的行**
/// （尾部单字符行——OCR 把界面数字读成的 `0`/`O`/`A`——不参与比较），并用它在碎片中的
/// **位置**定位后条的对应行。
///
/// **两道护栏**：
/// 1. 前置行一致性：比较行之前的各行须与后条同序号行模糊相等（见函数内注释）——
///    否则"换了说话人"的两条无关短句会被比较行的字符重叠率误判为续写；
/// 2. 碎片**字面末行**归一化为空串（纯标点条，如 `……`）→ 直接否决，且碎片至少要有一行
///    归一化长度 ≥2——这条保住 D9 的独立短条：glupov 语料 `[……]` 条绝不能被并入下一条
///    （否则语料出现"缺失"，是六项基准的长期硬门）。
pub(crate) fn short_fragment_related(frag: &str, next: &str) -> bool {
    let fl: Vec<Vec<char>> = frag.lines().map(norm_chars).collect();
    let nl: Vec<Vec<char>> = next.lines().map(norm_chars).collect();
    if fl.is_empty() || nl.is_empty() {
        return false;
    }
    // ── 比较行选择（P1，2026-09-28）──
    // 旧实现取**字面末行**并要求其归一化长度 ≥2。问题是 OCR 会把界面数字/字母读成单字符
    // 行并落在末尾（实测 pierro `MurderofBirds`/`0`(0.484s)、`Paimon`/`Huh?`/`0`(0.383s)、
    // `Mitya`/`…seems to be so`/`O`(0.901s)），这些**前缀碎片**在弱关联判据之前就被否决，
    // 因而无法被本 pass 吸收（用户 2026-09-28 导入 Premiere 时暴露：`0` 那一行还被解析器
    // 当成字幕序号，导致其后全部被截断）。
    //
    // 现改为：**尾部长度 <2 的行不参与比较**（它们不可能独立承载一条字幕），取**最后一个
    // 长度 ≥2 的行**作为比较行，并用**它在下标中的位置**定位后条的对应行。
    //
    // **D9 护栏原意保持不变**：字面末行归一化为**空串**（纯标点条，如 `……`）时直接否决
    // ——这类条必须独立留存，绝不能被并入下一条（否则语料出现"缺失"，长期硬门）。
    // 只有"长度恰为 1"的末行（如 moon 语料真实句尾 `了。`）不再由本门决定去留，
    // 改由时长门（`min_subtitle_sec`）与弱关联判据处理。
    if fl.last().is_none_or(|l| l.is_empty()) {
        return false; // D9：纯省略号/纯标点条 → 绝不并入
    }
    let Some(fidx) = (0..fl.len()).rev().find(|&i| fl[i].len() >= 2) else {
        return false; // 碎片全为单字符/垃圾行，无从比较
    };
    let flast = &fl[fidx];
    // ── 前置行一致性护栏（关键）──
    // 碎片除末行外的各行须与后条**同序号行**模糊相等：字幕的前置行是**姓名框/头衔块**，
    // 同一条字幕的渐进显示里它逐字不变。若不校验它，末行的"字符重叠率"会把**换了说话人**
    // 的两条无关短句误判为续写——实测（2026-09-26，按素材标定 min_subtitle_sec 时暴露）：
    // pierro `「丑角」/可以。`(2.83s) → `派蒙/欸！真的可以吗！…`（重叠 0.50）、
    // moon `卡侬/…艾莉亚。` → `艾莉亚/卡侬妹妹。`（重叠 1.00）、
    // glupov `斯捷潘尼扬/「缟玛瑙」/嗯？是你啊…` → 另一句（重叠 0.55）均为此类误判。
    // 加上本护栏后这些对手全部被判为无关；glupov 那条真·姓名框态（前置行同为
    // `安东/原「第九连队」临时连长`）仍被正确认定。
    for (a, b) in fl.iter().zip(nl.iter()).take(fidx) {
        if a.is_empty() {
            continue; // OCR 丢弃的不可识别行（如「……」）视为通配
        }
        let s: String = a.iter().collect();
        let t: String = b.iter().copied().take(a.len()).collect();
        if edit_distance_ratio(&s, &t) > SHORT_FRAGMENT_PREFIX_TOL {
            return false;
        }
    }
    let idx = fidx.min(nl.len() - 1);
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
/// `min_subtitle_sec` 由 `OcrRunParams::min_subtitle_sec` 传入（**前端可调，产品默认
/// 1.5s**，见 `DEFAULT_MIN_SUBTITLE_SEC`；≤0 关闭本 pass）。调大能减少碎片，但会提高
/// "误吞真实短句"的概率——该门与素材相关：实况嵌字的字幕寿命天然长于剧情语料，
/// 基准按素材分别标定（`tests/common::CaseCfg::hardsub_min_subtitle_sec`）。
///
/// 与既有合并的分工：
/// - `merge_contained_adjacent`：严格判据（子序列/行前缀）+ 1.25s 跨度门；
/// - 本 pass：**放宽到弱关联** + `min_subtitle_sec` 时长门 + 2.0s 间隔门（跨检测空洞）；
/// - D12 的稳定姓名框态（≥2.0s）不在本 pass 范围，仍保持独立（实测参考亦记为独立条目）。
pub(crate) fn merge_short_fragments_into_next(
    segments: Vec<OcrSegment>,
    min_subtitle_sec: f64,
) -> Vec<OcrSegment> {
    if min_subtitle_sec <= 0.0 {
        return segments;
    }
    let mut out: Vec<OcrSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        let merge = out.last().is_some_and(|last| {
            last.end - last.start <= min_subtitle_sec
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

/// 相邻段文本相似（相等/前缀/编辑距离 ≤ 阈值）→ 合并为一段（取更长文本、时间取并集）。
///
/// 用于阶段 2 短字幕召回后：dHash 判为变化但 OCR 文本与相邻段相同的"伪短字幕"
/// （字幕视觉抖动/OCR 抖动）会与相邻段相似，在此合并，避免同一句被拆成碎片。
pub(crate) fn merge_similar_adjacent(segments: Vec<OcrSegment>, threshold: f64) -> Vec<OcrSegment> {
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

