// ─── AI 融合模块（Phase 4）────────────────────────────────
// OCR 文本（可靠的剧情录屏字幕，可能含角色名前缀）与游戏内容时间轴文本
// （切片视频上带时间轴的"转写侧"：游戏语音 ASR 段，或无配音处的画面内嵌
// 字幕 OCR 段，通常与 OCR 文本不同语言）交由 LLM 跨语言语义对齐，
// 一步完成匹配 + 纠错 + 去重。时间轴以转写侧段为准。
//
// 纯文本契约（2026-10-02 用户决策）：本模块只做"编号映射 + 文本搬运"——
// 命中段逐字复制 OCR 原文（含换行与角色名行），未命中段保留转写侧原文；
// 两者都不改写、不剥离前缀、不做换行归一；**本模块不产出说话人/角色名**。
// 转写侧的说话人由前端预处理写成文本首行（与语料的"名字行 + 正文"同形态），
// 只作为文本的一部分随原文透传；说话人语义的正式契约待设计重评审，见
// benchmark/FUSE_PIPELINE_DEFECTS.md 的 F1/F2。
//
// 分批策略：OCR 文本全量放入每批 prompt（语义匹配需要全局视野，Qwen 32K
// 上下文足够），转写侧段每批 30 条。主播语音轨由前端过滤，不进入本模块。

use crate::ai_runtime::LlmManager;
use crate::llm::{LlmProgress, LLM_PROGRESS_EVENT};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 单批最多转写侧段数
const BATCH_SIZE: usize = 30;
/// 融合批生成上限：30 段 JSON 输出（index/ocr_index）需要余量；
/// 仅作上限，正常输出远小于此
const MAX_TOKENS: u32 = 4096;
/// 补全缺失外层花括号的上限：防把垃圾输出"补"成看似合法的 JSON
const MAX_BRACE_REPAIR: usize = 4;

/// 输入：一个游戏内容时间轴段（转写侧，index = 输入顺序，LLM 输出按此对应）。
/// 来源可为 game-ASR 游戏语音，或画面内嵌字幕 OCR（embed_ocr，无配音场景）。
/// 文本**原样**进入 prompt（保留换行）：前端预处理会把 ASR 的说话人标签写成
/// 首行（如 "Sonnet\n…"），与语料/嵌字的"名字行 + 正文"形态对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseAsrInput {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 融合结果段（时间轴沿用转写侧段；纯文本，无说话人字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusedSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// 是否匹配到 OCR 文本；false = 保留转写侧原文本
    pub matched: bool,
}

/// 融合统计（前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseStats {
    pub total: usize,
    pub matched: usize,
    /// JSON 解析失败的批数（该批全部段保留转写原文，整体计入这里）
    pub failed_batches: usize,
    /// 模型未给出判定（缺 index）的段数——仅统计解析成功的批；
    /// 解析失败的批已整体计入 failed_batches，不重复计入这里
    pub missing_segments: usize,
}

/// 融合完整结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseResult {
    pub segments: Vec<FusedSegment>,
    pub stats: FuseStats,
}

/// 构建单批 prompt：指令 + OCR 全量编号列表 + 本批 GC 编号列表。
/// 编号带前缀区分（OCR[n] / GC[n]）：批 ≥2 时 GC 编号是全局顺序，
/// 无前缀会让小模型混淆两套编号，把 OCR 编号误当 GC 编号输出。
/// 跨语言语义对齐：LLM 只输出对应关系（ocr_index），
/// 最终文本由代码从 OCR 列表逐字复制——实测小模型无法可靠"复制文本"。
///
/// 文本**原样**写入（保留换行）：条目可能跨多行（首行常是角色名/说话人标签），
/// 折行归一会让模型丢失该结构，且与"逐字复制"的目标矛盾。
fn build_prompt(ocr_texts: &[String], batch: &[FuseAsrInput]) -> String {
    let mut p = String::new();
    p.push_str(
        "你是游戏字幕融合助手。下面是可靠的剧情字幕文本（OCR）和游戏内容时间轴文本（GC，来源可为游戏语音转写或画面内嵌字幕 OCR，语言可能与字幕不同）。\n\
         请把每条 GC 与语义相同的 OCR 字幕对应（跨语言对应）：找到对应就填该 OCR 字幕的编号，找不到就填 0。\n\
         条目可能不止一行（首行常是角色名或说话人标签），条目一律按行首的 OCR[编号] / GC[编号] 标记划分。\n\
         严格只输出 JSON，严禁输出任何其他内容，格式：{\"segments\":[{\"index\":GC编号,\"ocr_index\":OCR编号或0}]}\n\
         index 和 ocr_index 都是纯数字（如 17），不要写成 \"GC[17]\"。\n\
         示例（GC[3] 与 OCR[1] 语义相同）：{\"segments\":[{\"index\":3,\"ocr_index\":1}]}\n\n",
    );
    // 条数上限（实测必需）：保留换行后条目跨多行，小模型会失去条数感并跑飞成
    // 数百条编号（撞 MAX_TOKENS 截断 → 整批降级，花括号修复救不回）。显式条数
    // 约束可压住：同构建 A/B 中"保留换行且无上限" 2/3 失败，"加条数上限" 3/3 稳定。
    p.push_str(&format!(
        "本次 GC 共 {} 条：最多只输出 {} 条，不要输出 GC 编号以外的内容。\n\n",
        batch.len(),
        batch.len()
    ));
    p.push_str("== 字幕文本（OCR）==\n");
    for (i, t) in ocr_texts.iter().enumerate() {
        p.push_str(&format!("OCR[{}] {}\n", i + 1, t.trim()));
    }
    p.push_str("\n== 游戏内容时间轴文本（GC）==\n");
    for s in batch {
        p.push_str(&format!("GC[{}] {}\n", s.index, s.text.trim()));
    }
    p
}

/// LLM 输出的单段（index = GC 输入编号；ocr_index = 对应的 OCR 编号，0/缺省=无对应）
#[derive(Debug, Deserialize)]
struct RawFusedSegment {
    #[serde(deserialize_with = "deser_index")]
    index: usize,
    #[serde(default, deserialize_with = "deser_index_opt")]
    ocr_index: Option<usize>,
}

/// index 字段宽容反序列化：3B 模型实测会把编号原样回显成 "GC[17]"
/// （带前缀的字符串），纯 usize 解析会整批失败降级。兼容数字/字符串。
fn deser_index<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct IndexVisitor;
    impl<'de> serde::de::Visitor<'de> for IndexVisitor {
        type Value = usize;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "数字或含数字的字符串（如 17、\"GC[17]\"）")
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<usize, E> {
            usize::try_from(v).map_err(|_| E::custom("index 过大"))
        }
        fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<usize, E> {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            digits
                .parse::<usize>()
                .map_err(|_| E::custom(format!("index 无法解析: {s:?}")))
        }
        // null（模型输出 "ocr_index":null）：opt 场景视为无对应；index 场景=0 会被 merge 忽略
        fn visit_unit<E: serde::de::Error>(self) -> Result<usize, E> {
            Ok(0)
        }
    }
    deserializer.deserialize_any(IndexVisitor)
}

/// ocr_index 的宽松解析：0、空串视为无对应（None）
fn deser_index_opt<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deser_index(deserializer).map(|v| (v != 0).then_some(v))
}

/// LLM 输出整体结构：{"segments":[...]}
#[derive(Debug, Deserialize)]
struct RawFusionOutput {
    segments: Vec<RawFusedSegment>,
}

/// 解析 LLM 输出，三级尝试：
/// 1. 严格 JSON；
/// 2. 截取首个 `{` 到最后一个 `}`（模型夹带 ```json 围栏或前后说明文字）；
/// 3. 补全缺失的外层花括号（实测 3B 模型在 30 段批次下会稳定漏掉最外层 `}`，
///    输出形如 `{"segments":[…]` 缺尾括号——属可确定性修复的形态）。
fn parse_fusion_output(raw: &str) -> Result<Vec<RawFusedSegment>, String> {
    let trimmed = raw.trim();
    let attempt = |s: &str| -> Result<Vec<RawFusedSegment>, String> {
        let out: RawFusionOutput =
            serde_json::from_str(s).map_err(|e| format!("JSON 解析失败: {}", e))?;
        Ok(out.segments)
    };
    if let Ok(v) = attempt(trimmed) {
        return Ok(v);
    }
    let mut candidates: Vec<String> = Vec::new();
    if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if close > open {
            candidates.push(trimmed[open..=close].to_string());
        }
    }
    let (opens, closes) = (trimmed.matches('{').count(), trimmed.matches('}').count());
    if opens > closes && opens - closes <= MAX_BRACE_REPAIR {
        // 只补不删：截到最后一个 `]` 再补齐缺失的 `}`（避免把尾部说明文字也包进来）
        if let (Some(open), Some(last_bracket)) = (trimmed.find('{'), trimmed.rfind(']')) {
            if last_bracket > open {
                candidates.push(format!(
                    "{}{}",
                    &trimmed[open..=last_bracket],
                    "}".repeat(opens - closes)
                ));
            }
        }
    }
    for c in &candidates {
        if let Ok(v) = attempt(c) {
            return Ok(v);
        }
    }
    Err(format!("无法从输出中解析 JSON: {}", &trimmed.chars().take(120).collect::<String>()))
}

/// 本批中"模型未给出判定"的输入编号（解析成功的批才有意义）：
/// 返回条目里缺 index 的段会被 merge 当作未命中而保留转写原文，
/// 但那次降级此前完全静默——故单列出来供统计与排查。
fn missing_indexes(batch: &[FuseAsrInput], raw: &[RawFusedSegment]) -> Vec<usize> {
    let got: std::collections::HashSet<usize> = raw.iter().map(|r| r.index).collect();
    batch
        .iter()
        .map(|s| s.index)
        .filter(|i| !got.contains(i))
        .collect()
}

/// 把 LLM 解析结果合并回输入序列（纯文本搬运）：
/// - ocr_index 在 1..=ocr_texts.len()：text 逐字取 OCR 原文（含换行/角色名行）；
/// - 未命中/越界：保留转写侧原文本；
/// - 重复 index 以后者为准。
fn merge_results(
    ocr_texts: &[String],
    inputs: &[FuseAsrInput],
    raw: Vec<RawFusedSegment>,
) -> Vec<FusedSegment> {
    let mut by_index: std::collections::HashMap<usize, RawFusedSegment> = std::collections::HashMap::new();
    for r in raw {
        by_index.insert(r.index, r);
    }
    inputs
        .iter()
        .map(|s| {
            // 边界过滤：模型可能幻觉越界编号，越界视为未命中
            let hit = by_index
                .get(&s.index)
                .and_then(|r| r.ocr_index)
                .filter(|&oi| (1..=ocr_texts.len()).contains(&oi));
            match hit {
                Some(oi) => FusedSegment {
                    start: s.start,
                    end: s.end,
                    text: ocr_texts[oi - 1].clone(),
                    matched: true,
                },
                None => FusedSegment {
                    start: s.start,
                    end: s.end,
                    text: s.text.clone(),
                    matched: false,
                },
            }
        })
        .collect()
}

/// 串起完整融合流水线（分批调用 LLM）。
/// `on_progress(progress, message)` 每批推进调用一次；批间串行（避免并发
/// 多进程抢 GPU）。
pub fn fuse_pipeline<F>(
    manager: &LlmManager,
    ocr_texts: &[String],
    inputs: &[FuseAsrInput],
    mut on_progress: F,
) -> Result<FuseResult, String>
where
    F: FnMut(f64, String),
{
    let ready = manager.with_provider(|p| p.is_ready());
    if !ready {
        return Err("LLM 运行环境未就绪".into());
    }
    if ocr_texts.is_empty() {
        return Err("没有可用的 OCR 字幕文本".into());
    }
    if inputs.is_empty() {
        return Err("没有可用的游戏内容段（ASR 或画面嵌字 OCR）".into());
    }

    let total_batches = inputs.len().div_ceil(BATCH_SIZE);
    let mut segments: Vec<FusedSegment> = Vec::with_capacity(inputs.len());
    let mut matched = 0usize;
    let mut failed_batches = 0usize;
    let mut missing_segments = 0usize;

    for (b, chunk) in inputs.chunks(BATCH_SIZE).enumerate() {
        on_progress(
            (b as f64 + 0.1) / total_batches as f64,
            format!("融合批次 {}/{}", b + 1, total_batches),
        );
        let prompt = build_prompt(ocr_texts, chunk);
        // 解析失败重试一次；仍失败则整批降级为转写原文，不阻塞流水线
        let mut parsed: Result<Vec<RawFusedSegment>, String> =
            manager.with_provider(|p| p.complete(&prompt, MAX_TOKENS)).map_err(|e| e.to_string()).and_then(|raw| parse_fusion_output(&raw));
        if parsed.is_err() {
            parsed = manager
                .with_provider(|p| p.complete(&prompt, MAX_TOKENS))
                .map_err(|e| e.to_string())
                .and_then(|raw| parse_fusion_output(&raw));
        }
        match parsed {
            Ok(raw) => {
                // 解析成功但整批缺 index（实测常见：长批次漏首条）：单列计数，不掩盖
                let missing = missing_indexes(chunk, &raw);
                if !missing.is_empty() {
                    missing_segments += missing.len();
                    eprintln!(
                        "[fuse] 批次 {} 模型未判定 {} 段（保留转写原文）: {:?}",
                        b + 1,
                        missing.len(),
                        missing
                    );
                }
                let out = merge_results(ocr_texts, chunk, raw);
                matched += out.iter().filter(|s| s.matched).count();
                segments.extend(out);
            }
            Err(e) => {
                failed_batches += 1;
                eprintln!("[fuse] 批次 {} 解析失败，保留转写原文: {}", b + 1, e);
                segments.extend(merge_results(ocr_texts, chunk, Vec::new()));
            }
        }
    }

    on_progress(
        1.0,
        format!(
            "融合完成，共 {} 段（命中 {}，未判定 {}）",
            segments.len(),
            matched,
            missing_segments
        ),
    );
    Ok(FuseResult {
        segments,
        stats: FuseStats {
            total: inputs.len(),
            matched,
            failed_batches,
            missing_segments,
        },
    })
}

/// Tauri 命令：执行 AI 融合（OCR 文本 + 游戏内容 ASR 段 → LLM → fused 段）。
/// 在后台线程跑（LLM 分批调用可能耗时），通过 `llm-progress` 事件上报每批进度。
#[tauri::command]
pub async fn run_fuse(
    app: AppHandle,
    ocr_texts: Vec<String>,
    asr_segments: Vec<FuseAsrInput>,
) -> Result<FuseResult, String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<LlmManager>();
        fuse_pipeline(&manager, &ocr_texts, &asr_segments, |progress, message| {
            let _ = app_handle.emit(LLM_PROGRESS_EVENT, LlmProgress { progress, message });
        })
    })
    .await
    .map_err(|e| format!("AI 融合任务内部错误: {}", e))?
}

// ─── 单元测试（纯逻辑，不依赖真实 LLM）──────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_inputs(n: usize) -> Vec<FuseAsrInput> {
        (1..=n)
            .map(|i| FuseAsrInput {
                index: i,
                start: i as f64,
                end: i as f64 + 1.0,
                text: format!("语音{}", i),
            })
            .collect()
    }

    #[test]
    fn test_build_prompt_contains_all_ocr_and_batch() {
        let ocr = vec!["派蒙：旅行者你来了".into(), "前方有敌人".into()];
        let batch = sample_inputs(2);
        let p = build_prompt(&ocr, &batch);
        assert!(p.contains("OCR[1] 派蒙：旅行者你来了"));
        assert!(p.contains("OCR[2] 前方有敌人"));
        assert!(p.contains("GC[1] 语音1"));
        assert!(p.contains("GC[2] 语音2"));
        // 标签统一为 GC（来源可含 embed_ocr，不再用 "ASR" 误导）
        assert!(!p.contains("ASR[1]"));
        // 纯文本契约：不再要求模型输出角色名
        assert!(!p.contains("character"));
        // 条数上限：保留换行后必需（否则实测模型跑飞成数百条编号）
        assert!(p.contains("本次 GC 共 2 条：最多只输出 2 条"));
    }

    #[test]
    fn test_build_prompt_keeps_newlines() {
        // 条目可能跨多行（首行常是角色名/说话人标签）：换行必须原样进 prompt
        let ocr = vec!["派蒙\n旅行者你来了".into()];
        let mut batch = sample_inputs(1);
        batch[0].text = "Sonnet\nhe big deal!".into();
        let p = build_prompt(&ocr, &batch);
        assert!(p.contains("OCR[1] 派蒙\n旅行者你来了\n"), "OCR 侧换行须保留：{p}");
        assert!(p.contains("GC[1] Sonnet\nhe big deal!\n"), "GC 侧换行须保留：{p}");
    }

    #[test]
    fn test_parse_fusion_output_plain_json() {
        let raw = r#"{"segments":[{"index":1,"ocr_index":1}]}"#;
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].index, 1);
        assert_eq!(v[0].ocr_index, Some(1));
    }

    #[test]
    fn test_parse_fusion_output_with_noise() {
        // 模型夹带围栏/说明文字：截取花括号片段
        let raw = "好的，结果如下：\n```json\n{\"segments\":[{\"index\":2,\"ocr_index\":0}]}\n```";
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v[0].index, 2);
        assert_eq!(v[0].ocr_index, None, "ocr_index 0 视为无对应");
    }

    #[test]
    fn test_parse_fusion_output_string_index() {
        // 3B 模型实测会回显 "ASR[1]" 带前缀字符串：宽容提取数字
        let raw = r#"{"segments":[{"index":"ASR[1]","ocr_index":"OCR[2]"},{"index":"2","ocr_index":null}]}"#;
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].index, 1);
        assert_eq!(v[0].ocr_index, Some(2));
        assert_eq!(v[1].index, 2);
        assert_eq!(v[1].ocr_index, None, "null 视为无对应");
    }

    #[test]
    fn test_parse_fusion_output_missing_outer_brace() {
        // 真实 30 段批次实测形态（4/4 复现）：模型漏掉最外层 `}`，补全后必须可解析
        let raw = r#"{"segments":[{"index":2,"ocr_index":2},{"index":3,"ocr_index":0}]"#;
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].ocr_index, None);
    }

    #[test]
    fn test_parse_fusion_output_garbage_errors() {
        assert!(parse_fusion_output("完全不是 JSON").is_err());
        assert!(parse_fusion_output("{\"broken\": [").is_err());
    }

    #[test]
    fn test_merge_results_verbatim_copy() {
        // 命中段逐字复制 OCR 原文（含角色名行/换行），不做任何剥离
        let ocr = vec!["卡侬\n桑娜妲。仔细想想。".to_string(), "前方有敌人".to_string()];
        let inputs = sample_inputs(3);
        let raw = vec![
            RawFusedSegment { index: 1, ocr_index: Some(1) },
            RawFusedSegment { index: 3, ocr_index: None },
        ];
        let out = merge_results(&ocr, &inputs, raw);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "卡侬\n桑娜妲。仔细想想。");
        assert!(out[0].matched);
        // index 2 未匹配：保留转写原文本
        assert_eq!(out[1].text, "语音2");
        assert!(!out[1].matched);
        // index 3：无对应 OCR → 转写原文本
        assert_eq!(out[2].text, "语音3");
        assert!(!out[2].matched);
    }

    #[test]
    fn test_merge_results_ignores_out_of_range_and_dup() {
        let ocr = vec!["派蒙：旅行者你来了".to_string()];
        let inputs = sample_inputs(2);
        let raw = vec![
            RawFusedSegment { index: 1, ocr_index: Some(1) },
            RawFusedSegment { index: 1, ocr_index: Some(2) },
            RawFusedSegment { index: 99, ocr_index: Some(1) },
        ];
        let out = merge_results(&ocr, &inputs, raw);
        assert_eq!(out[0].text, "语音1", "重复 index 后者覆盖；其 ocr_index=2 越界 → 保留原文");
        assert_eq!(out[1].text, "语音2", "越界 index 不影响");
    }

    #[test]
    fn test_missing_indexes() {
        let batch = sample_inputs(3);
        let raw = vec![RawFusedSegment { index: 1, ocr_index: None }];
        assert_eq!(missing_indexes(&batch, &raw), vec![2, 3]);
        assert_eq!(missing_indexes(&batch, &[]), vec![1, 2, 3], "整批无输出=全部未判定");
    }
}
