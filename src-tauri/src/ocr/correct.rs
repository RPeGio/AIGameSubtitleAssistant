use serde::{Deserialize, Serialize};
use super::params::{OcrSegment, PunctuationNorm};

/// 一条待审批的文本纠正（S2 术语表 / S3 一致性纠错共用）。
///
/// **Rust 侧只标记、不改文本**（2026-09-24 用户决策）：产出文本保持"标点归一化后、
/// 未精化"的形态，纠正以 `Diff` 形式交给前端；用户在前端逐条审批——
/// - **采纳**：对该次 OCR 的所有事件文本执行 `old → new` 替换，条目出队；
/// - **放弃**：不改文本，条目出队。
///
/// 这样做的理由：纠正的"是否正确"本质上需用户判断（用户明确指出"多数派不一定正确"），
/// 而后端静默改写会让用户无从察觉；改为审批后，误报无害（点放弃即可）、判据也不必收紧。
///
/// `old` 用集合语义：同一目标可能对应多种误读形态（`编玛瑙` / `编玛脑`），
/// 全部收进同一条目，采纳时一并替换。底层用 `HashSet` 去重，返回前 `collect` 为 `Vec`
/// （JSON 无 set 类型，且 `Vec` 顺序稳定便于调试）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diff {
    /// 待替换的原文形态（去重后）
    pub old: Vec<String>,
    /// 替换目标（术语表词条，或一致性纠错的多数派写法）
    pub new: String,
}

/// 采纳替换规则的**单一实现**（产品命令 `approve_corpus_diff` 与基准
/// `tests/common::apply_diffs_to_segments` 共用，PR34 审查 P2-1 收敛：
/// 此前是前端 TS / 基准 Rust 两份实现靠注释同步，现机制上同源）。
///
/// 逐个误读形态对全文替换（`str::replace` ≡ 前端此前的 `split(old).join(new)`，
/// R3 已用真实数据逐位对齐验证）。
///
/// 防御：跳过空/单字符 `old`——空串 replace 会在每字符间插入 `new`（灾难性）。
/// 两个 Diff 来源本就保证 ≥2 字符（`collect_glossary_hits` 与
/// `find_consistency_hints` 均只收长度 ≥2 的词条/片段），此处按公共边界再守一道。
pub fn replace_diff_forms(text: &str, old_forms: &[String], new: &str) -> String {
    let mut out = text.to_string();
    for old in old_forms {
        if old.chars().count() >= 2 {
            out = out.replace(old.as_str(), new);
        }
    }
    out
}

/// 括号字符 → 是左括号还是右括号（`None` 表示不是括号）
pub(crate) fn bracket_side(c: char) -> Option<bool> {
    match c {
        '（' | '【' | '[' | '{' | '〔' | '〖' | '〈' | '《' | '『' | '「' => Some(true),
        '）' | '】' | ']' | '}' | '〕' | '〗' | '〉' | '》' | '』' | '」' => Some(false),
        _ => None,
    }
}

/// 省略号类字符（连续出现 ≥2 个、或本身是 `…` 时，视为一个省略号）
pub(crate) fn is_ellipsis_char(c: char) -> bool {
    matches!(c, '.' | '．' | '。' | '·' | '・' | '‧' | '…')
}

/// 修复"标点被识别成拉丁字母"（S1 补充，2026-09-24）。
///
/// 依据字符精度诊断：英文嵌字里 `「」` 被读成行尾的 `j`（实测 9 处，占三案例全部字符错误的
/// 约 20%）——与 `]`/`【` 同源，都是引号退化，但字母形态未被括号归一化覆盖。
///
/// **上下文约束**（避免误伤正常单词）：仅当该字母**紧邻行尾或已是行尾**、且**本行不含其它
/// 未配对的括号**时才替换。实测形态：`Sonnet"` / `...sidej` / `The Jesterj`。
pub(crate) fn fix_misread_punct_letters(text: &str, cfg: &PunctuationNorm) -> String {
    let mut out = String::with_capacity(text.len());
    for (li, line) in text.lines().enumerate() {
        if li > 0 {
            out.push('\n');
        }
        let chars: Vec<char> = line.chars().collect();
        // 行内已配对括号数：有括号说明标点识别正常，不做字母替换
        let has_bracket = chars.iter().any(|c| bracket_side(*c).is_some());
        let trim_end = chars.len();
        let mut buf: Vec<char> = Vec::with_capacity(chars.len());
        for (i, &c) in chars.iter().enumerate() {
            // j/J 位于行尾（后面只剩空白）且本行无其它括号 → 视为右括号退化
            let is_tail = chars[i + 1..trim_end].iter().all(|x| x.is_whitespace());
            if !has_bracket && is_tail && matches!(c, 'j' | 'J') {
                buf.push(cfg.close_bracket);
            } else {
                buf.push(c);
            }
        }
        out.extend(buf);
    }
    out
}

/// 标点/省略号归一化（作用于最终段文本；不改动任何合并判据——判据走 `norm_chars`，
/// 本就忽略标点，故此归一化对分段与计时零影响）。
pub(crate) fn normalize_punctuation(text: &str, cfg: &PunctuationNorm) -> String {
    let src = if cfg.fix_misread_letters {
        fix_misread_punct_letters(text, cfg)
    } else {
        text.to_string()
    };
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(is_open) = bracket_side(c) {
            if is_open {
                out.push(cfg.open_bracket);
            } else {
                out.push(cfg.close_bracket);
            }
            i += 1;
            continue;
        }
        let ell: Vec<char> = cfg.ellipsis.chars().collect();
        if !ell.is_empty() && c == ell[0] {
            // 目标省略号本身：多个连续一律收敛为一个
            out.push_str(&cfg.ellipsis);
            i += 1;
            while i < chars.len() && chars[i] == ell[0] {
                i += 1;
            }
            continue;
        }
        if is_ellipsis_char(c) {
            // 连续 ≥2 个（如 `...`、`。。`、`···`）才算省略号；单个 `.`/`。` 原样保留
            let mut j = i;
            while j < chars.len() && is_ellipsis_char(chars[j]) && chars[j] != ell.first().copied().unwrap_or('\0') {
                j += 1;
            }
            if j - i >= 2 {
                out.push_str(&cfg.ellipsis);
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 术语匹配的"词字符"判定：字母/数字/汉字，外加**词内可能出现的符号**（`.`、`-`、`_`、`·`）。
///
/// 关键约束（2026-09-24 回归修复）：匹配窗口**只能是连续词字符**——否则窗口会跨过标点，
/// 替换时把相邻标点吞掉。实测反例：词条 `NO.0217` 在 `抱歉，NO.0217，前面…` 上匹配到
/// `，NO.0217`（含逗号），替换后变成 `抱歉NO.02177，`（逗号丢失 + 长度错位）。
///
/// `.`/`-` 等必须算词字符，否则 `NO.0217` 会被切成 `NO`+`0217` 两个片段而永远匹配不上；
/// 而**全角标点与空白**（`，。！？「」` 等）不算，保证窗口不跨句读标点。
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '·' | '/' | ':')
}

/// 术语表纠错的模糊匹配容差（编辑距离比例）。
///
/// 依据字符精度诊断（D4，2026-09-24）：主要是**形近字**误读（`缟`→`编` 等），
/// 词条长度通常 ≥3 字符，1 个字符差的比例远小于此值，故 0.34 能覆盖单字误读，
/// 又不会把两个不同的短词判为同一个（如 3 字符词差 2 个字符 = 0.67 > 0.34）。
pub(crate) const GLOSSARY_MATCH_TOL: f64 = 0.34;

/// 术语表纠错（S2，可选精度策略，由用户提供词表）。
///
/// 机制：把文本切成**连续词字符**的片段（标点/空白作为不可跨越的分隔），
/// 在每个片段内按**窗口**滑过（窗口长度 = 词条长度 ±1）与词条做模糊比较；
/// 术语表标记（S2）：扫出与词条形近的片段，**只记录、不改文本**（2026-09-24 用户决策）。
///
/// 返回 (去重后的原文形态集合, 命中的词条)；`None` 表示无命中。
/// 调用方把结果并入 `Diff`（同一词条可能对应多种误读形态，全部收进同一条目）。
///
/// 与标点归一化的关系：**归一化是必须的前置**（用户 2026-09-24 决策）——词条与产出
/// 必须同形才能匹配（`[编玛瑙】` 无法匹配 `「缟玛瑙」`），故本函数在归一化之后调用。
///
/// 安全性：① 窗口不跨标点（见 `is_word_char`）；② 只认与词条不同但足够相近的窗口；
/// ③ 要求**首字符相同**或**仅 1 字符不同**；④ 忽略单字符词条。
pub(crate) fn collect_glossary_hits(
    text: &str,
    glossary: &[String],
) -> Option<(std::collections::HashSet<String>, String)> {
    if glossary.is_empty() {
        return None;
    }
    // 预编译词条（去空白后按字符切），并过滤过短词条（1 字符词条误伤面太大）
    let terms: Vec<Vec<char>> = glossary
        .iter()
        .map(|t| t.chars().filter(|c| !c.is_whitespace()).collect::<Vec<char>>())
        .filter(|t| t.len() >= 2)
        .collect();
    if terms.is_empty() {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut hits: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    // `cursor` = 已扫到原文的哪个位置。命中后前移到窗口末，避免同一处重复计入。
    let mut cursor = 0;
    while cursor < chars.len() {
        if !is_word_char(chars[cursor]) {
            cursor += 1;
            continue;
        }
        // 当前片段 = 连续词字符 [cursor, seg_end)
        let seg_end = {
            let mut e = cursor;
            while e < chars.len() && is_word_char(chars[e]) {
                e += 1;
            }
            e
        };
        // 片段内滑窗匹配（窗口不得越过 seg_end）
        let mut best: Option<(usize, usize, &Vec<char>, usize)> = None; // (起点, 窗口长, 词条, 词条长)
        for t in &terms {
            let tl = t.len();
            // **只允许等长窗口**：词条纠错针对的是"形近字替换"（长度不变），
            // 允许 ±1 长度差会在替换时增减字符——实测产生 `NO.02177`（残留数字）、
            // `米提亚长`（丢掉「队」）这类破坏性改写
            for wl in [tl] {
                let mut s = cursor;
                while s + wl <= seg_end {
                    let win = &chars[s..s + wl];
                    let d = levenshtein_chars(win, t);
                    let ratio = d as f64 / tl.max(wl) as f64;
                    // 命中条件（**普遍规则，不为个别词条做长度特判**）：
                    // ① 等长窗口（见上）；
                    // ② 有差异且编辑距离比例 ≤ 容差；
                    // ③ **首字符相同**（同词内后续字误读，如 米提哑→米提亚）
                    //    或 **仅 1 字符不同**（任意位置的单字形近误读，如 编玛瑙→缟玛瑙）。
                    //    后者是术语表的主要价值来源：用户先跑一次自动纠错、发现被 vote
                    //    多数的词整体是错的（含首字），再用术语表替换。不加长度特判——
                    //    曾为单个长短语加的特判已回退，长短语误伤由用户词表自身规避
                    //    （实测 `天空岛的造物` 会改写 `天空岛的防线`，故不入词表）。
                    let one_char_near = d == 1;
                    if d > 0
                        && ratio <= GLOSSARY_MATCH_TOL
                        && (win.first() == t.first() || one_char_near)
                    {
                        // 择优：起点更靠前优先；同起点时更长的词条优先
                        let better = match best {
                            None => true,
                            Some((bs, _, _, btl)) => s < bs || (s == bs && tl > btl),
                        };
                        if better {
                            best = Some((s, wl, t, tl));
                        }
                    }
                    s += 1;
                }
            }
        }
        match best {
            Some((s, wl, t, _)) => {
                // 记录命中的原文形态（集合去重）与目标词条；**不改文本**
                let hit: String = chars[s..s + wl].iter().collect();
                hits.entry(t.iter().collect::<String>())
                    .or_default()
                    .insert(hit);
                cursor = s + wl; // 前移，避免同一处重复计入
            }
            None => {
                cursor += 1;
            }
        }
    }
    if hits.is_empty() {
        return None;
    }
    // 多条词条命中时取命中形态最多的一条（其余下次运行仍会被标出）
    let (new, olds) = hits
        .into_iter()
        .max_by_key(|(_, v)| v.len())
        .expect("hits 非空");
    Some((olds, new))
}

/// 字符级编辑距离（术语表匹配用；文本短，直接 DP）
pub(crate) fn levenshtein_chars(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 一致性纠错（S3）：候选变体的最少总出现次数。
///
/// 出现太少（如仅 1~2 次）不足以判断哪个是"多数"，且容易把正常的不同词误判为变体。
pub(crate) const CONSISTENCY_MIN_TOTAL: usize = 3;

/// 一致性纠错（S3）：主导变体的最低占比。
///
/// 只有明显多数才算候选（如 11:0）。用户明确指出"多数派不一定正确"，故本阈值**不用于
/// 自动改写**，仅用于筛选"值得提请用户注意"的变体；且最终是否采纳由用户决定。
pub(crate) const CONSISTENCY_DOMINANCE: f64 = 0.7;

/// 一致性纠错的输入形态：从产出文本中切出的**词片段**（连续词字符），带出现次数。
///
/// 用于发现"同一位置的形近变体"——例如 `编玛瑙` 出现 11 次而 `缟玛瑙` 0 次，
/// 说明 OCR 对该词存在**系统性误读**（此时"多数"恰恰是错的，故只建议、不自动改）。
#[derive(Debug, Clone, PartialEq)]
pub struct ConsistencyHint {
    /// 少数派写法（疑似误读）
    pub from: String,
    /// 多数派写法（疑似正确）
    pub to: String,
    /// 少数派出现次数
    pub from_count: usize,
    /// 多数派出现次数
    pub to_count: usize,
}

/// 一致性纠错（S3）：扫描全部产出文本，找出**形近变体对**并给出建议（不修改文本）。
///
/// 判据（保守，宁可漏报不可误报）：
/// ① 两个片段**等长**、编辑距离恰为 1（形近误读的典型形态）；
/// ② 二者合计出现次数 ≥ `CONSISTENCY_MIN_TOTAL`；
/// ③ 主导方占比 ≥ `CONSISTENCY_DOMINANCE`。
///
/// **只返回建议**——由用户在前端确认后再写入术语表（复用 S2 的纠错通道）。
/// 这样既避免"多数派是错的"时自动改坏文本，也把决定权留给用户。
pub fn find_consistency_hints(segments: &[OcrSegment]) -> Vec<ConsistencyHint> {
    // 统计词片段出现次数（跨全部条目）
    let mut counts: std::collections::HashMap<Vec<char>, usize> = std::collections::HashMap::new();
    for seg in segments {
        for line in seg.text.lines() {
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if !is_word_char(chars[i]) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < chars.len() && is_word_char(chars[i]) {
                    i += 1;
                }
                // 只统计长度 ≥2 的片段（单字无法判断）
                if i - start >= 2 {
                    *counts.entry(chars[start..i].to_vec()).or_insert(0) += 1;
                }
            }
        }
    }
    // 找出形近变体对（等长 + 编辑距离 1）
    let keys: Vec<Vec<char>> = counts.keys().cloned().collect();
    let mut hints: Vec<ConsistencyHint> = Vec::new();
    let mut paired: std::collections::HashSet<Vec<char>> = std::collections::HashSet::new();
    for a in &keys {
        if paired.contains(a) {
            continue;
        }
        for b in &keys {
            if a == b || a.len() != b.len() || paired.contains(b) {
                continue;
            }
            if levenshtein_chars(a, b) != 1 {
                continue;
            }
            let ca = counts[a];
            let cb = counts[b];
            let total = ca + cb;
            if total < CONSISTENCY_MIN_TOTAL {
                continue;
            }
            let (major, minor, cmaj, cmin) = if ca >= cb { (a, b, ca, cb) } else { (b, a, cb, ca) };
            if (cmaj as f64) < CONSISTENCY_DOMINANCE * (total as f64) {
                continue;
            }
            paired.insert(a.clone());
            paired.insert(b.clone());
            hints.push(ConsistencyHint {
                from: minor.iter().collect(),
                to: major.iter().collect(),
                from_count: cmin,
                to_count: cmaj,
            });
            break;
        }
    }
    // 按"少数派出现次数"降序（更值得注意的排前面），再按字面稳定排序
    hints.sort_by(|x, y| {
        y.from_count
            .cmp(&x.from_count)
            .then_with(|| x.from.cmp(&y.from))
    });
    hints
}

/// 把一致性提示转成待审批 `Diff`（S3：`old = {少数派}`、`new = 多数派`）。
///
/// **Rust 侧不改文本**——与术语表一致，纠正交由用户在前端审批（用户指出"多数派不一定
/// 正确"：OCR 系统性偏移时多数派恰恰是错的，故必须保留用户否决权）。
pub(crate) fn consistency_diffs(segments: &[OcrSegment]) -> Vec<Diff> {
    find_consistency_hints(segments)
        .into_iter()
        .map(|h| Diff {
            old: vec![h.from],
            new: h.to,
        })
        .collect()
}

/// 把一批命中形态并入 Diff 列表：同 `new` 的条目合并（`old` 取并集）。
///
/// 目的：同一词条在一次运行中可能命中多种误读形态（`编玛瑙`、`编玛脑`…），
/// 用户只应审批**一条**（`→ 缟玛瑙`），故按 `new` 聚合。
pub(crate) fn merge_diff(
    diffs: &mut Vec<Diff>,
    olds: std::collections::HashSet<String>,
    new: String,
) {
    if let Some(d) = diffs.iter_mut().find(|d| d.new == new) {
        let mut set: std::collections::HashSet<String> = d.old.iter().cloned().collect();
        set.extend(olds);
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort(); // 顺序稳定，便于调试与快照比较
        d.old = v;
    } else {
        let mut v: Vec<String> = olds.into_iter().collect();
        v.sort();
        diffs.push(Diff { old: v, new });
    }
}

