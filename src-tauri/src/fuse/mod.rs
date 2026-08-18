// ─── AI 融合模块（Phase 4）────────────────────────────────
// OCR 文本（可靠的剧情录屏字幕，可能含角色名前缀）与游戏内容 ASR 段
// （切片视频的游戏语音，通常与 OCR 文本不同语言）交由 LLM 跨语言语义对齐，
// 一步完成匹配 + 纠错 + 去重。时间轴以 ASR 为准。
//
// 分批策略：OCR 文本全量放入每批 prompt（语义匹配需要全局视野，Qwen 32K
// 上下文足够），ASR 段每批 30 条。主播语音轨由前端过滤，不进入本模块。

use crate::ai_runtime::LlmManager;
use crate::llm::{LlmProgress, LLM_PROGRESS_EVENT};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 单批最多 ASR 段数
const BATCH_SIZE: usize = 30;
/// 融合批生成上限：30 段 JSON 输出（index/ocr_index/character）需要余量；
/// 仅作上限，正常输出远小于此
const MAX_TOKENS: u32 = 4096;

/// 输入：一个游戏内容 ASR 段（index = 输入顺序，LLM 输出按此对应）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseAsrInput {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 融合结果段（时间轴沿用 ASR 段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusedSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// LLM 从 OCR 文本提取的角色名；无匹配段为 None
    pub character: Option<String>,
    /// 是否匹配到 OCR 文本；false = 保留 ASR 原文本
    pub matched: bool,
}

/// 融合统计（前端展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseStats {
    pub total: usize,
    pub matched: usize,
    /// JSON 解析失败的批数（该批段保留 ASR 原文本）
    pub failed_batches: usize,
}

/// 融合完整结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseResult {
    pub segments: Vec<FusedSegment>,
    pub stats: FuseStats,
}

/// 构建单批 prompt：指令 + OCR 全量编号列表 + 本批 ASR 编号列表。
/// 编号带前缀区分（OCR[n] / ASR[n]）：批 ≥2 时 ASR 编号是全局顺序，
/// 无前缀会让小模型混淆两套编号，把 OCR 编号误当 ASR 编号输出。
/// 跨语言语义对齐：LLM 只输出对应关系（ocr_index + character），
/// 最终文本由代码从 OCR 列表逐字复制——实测小模型无法可靠"复制文本"。
fn build_prompt(ocr_texts: &[String], batch: &[FuseAsrInput]) -> String {
    let mut p = String::new();
    p.push_str(
        "你是游戏字幕融合助手。下面是可靠的剧情字幕文本（OCR）和游戏语音转写文本（ASR，语言可能与字幕不同）。\n\
         请把每条 ASR 文本与语义相同的 OCR 字幕文本对应（跨语言对应）：\n\
         - 找到对应字幕：ocr_index 填该 OCR 字幕的编号，character 从该字幕开头的角色名前缀提取（如 OCR 文本“派蒙：旅行者你来了”→ character 为“派蒙”）；\n\
         - 找不到对应：ocr_index 填 0，character 留空。\n\
         若 OCR 中同一句对话同时存在被截断版和完整版，选择完整版对应的编号。\n\
         严格只输出 JSON，严禁输出任何其他内容，格式：{\"segments\":[{\"index\":ASR编号,\"ocr_index\":OCR编号或0,\"character\":\"角色名或空\"}]}\n\
         index 和 ocr_index 都是纯数字（如 17），不要写成 \"ASR[17]\"。\n\
         示例（OCR[1] 是“派蒙：旅行者你来了”，ASR[3] 是“トラベラー来たな”，两处对应）：\n\
         {\"segments\":[{\"index\":3,\"ocr_index\":1,\"character\":\"派蒙\"}]}\n\n",
    );
    p.push_str("== 字幕文本（OCR）==\n");
    for (i, t) in ocr_texts.iter().enumerate() {
        let t = t.replace('\n', " ");
        p.push_str(&format!("OCR[{}] {}\n", i + 1, t));
    }
    p.push_str("\n== 游戏语音转写（ASR）==\n");
    for s in batch {
        let t = s.text.replace('\n', " ");
        p.push_str(&format!("ASR[{}] {}\n", s.index, t));
    }
    p
}

/// LLM 输出的单段（index = ASR 输入编号；ocr_index = 对应的 OCR 编号，0/缺省=无对应）
#[derive(Debug, Deserialize)]
struct RawFusedSegment {
    #[serde(deserialize_with = "deser_index")]
    index: usize,
    #[serde(default, deserialize_with = "deser_index_opt")]
    ocr_index: Option<usize>,
    #[serde(default)]
    character: Option<String>,
}

/// index 字段宽容反序列化：3B 模型实测会把编号原样回显成 "ASR[17]"
/// （带前缀的字符串），纯 usize 解析会整批失败降级。兼容数字/字符串。
fn deser_index<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct IndexVisitor;
    impl<'de> serde::de::Visitor<'de> for IndexVisitor {
        type Value = usize;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "数字或含数字的字符串（如 17、\"ASR[17]\"）")
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

/// 去掉 OCR 文本开头的"角色名："前缀（中英文冒号都支持）
fn strip_prefix(text: &str, character: Option<&str>) -> String {
    let trimmed = text.trim();
    if let Some(name) = character.filter(|c| !c.trim().is_empty()) {
        let name = name.trim();
        for sep in [':', '：'] {
            if let Some(rest) = trimmed.strip_prefix(&format!("{name}{sep}")) {
                return rest.trim().to_string();
            }
        }
    }
    trimmed.to_string()
}

/// LLM 输出整体结构：{"segments":[...]}
#[derive(Debug, Deserialize)]
struct RawFusionOutput {
    segments: Vec<RawFusedSegment>,
}

/// 解析 LLM 输出：先严格 JSON，失败时尝试提取首尾花括号片段。
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
    // 模型可能夹带 ```json 围栏或前后说明文字：截取首个 { 到最后一个 }
    if let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if close > open {
            if let Ok(v) = attempt(&trimmed[open..=close]) {
                return Ok(v);
            }
        }
    }
    Err(format!("无法从输出中解析 JSON: {}", &trimmed.chars().take(120).collect::<String>()))
}

/// 把 LLM 解析结果合并回输入序列：
/// - 命中对应 OCR（ocr_index 在 1..=ocr_texts.len()）：text 逐字取 OCR 文本并去角色名前缀；
/// - 未命中/越界：保留 ASR 原文本；
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
        .map(|s| match by_index.get(&s.index) {
            Some(r) => {
                // 边界过滤：模型可能幻觉越界编号，越界视为未命中（文本与 matched 共用同一条件）
                let in_range = r.ocr_index.filter(|&oi| (1..=ocr_texts.len()).contains(&oi));
                let text = match in_range {
                    Some(oi) => strip_prefix(&ocr_texts[oi - 1], r.character.as_deref()),
                    None => s.text.trim().to_string(),
                };
                FusedSegment {
                    start: s.start,
                    end: s.end,
                    text,
                    character: r
                        .character
                        .as_ref()
                        .filter(|c| !c.trim().is_empty())
                        .cloned(),
                    matched: in_range.is_some(),
                }
            }
            None => FusedSegment {
                start: s.start,
                end: s.end,
                text: s.text.clone(),
                character: None,
                matched: false,
            },
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
        return Err("没有可用的游戏内容 ASR 段".into());
    }

    let total_batches = inputs.len().div_ceil(BATCH_SIZE);
    let mut segments: Vec<FusedSegment> = Vec::with_capacity(inputs.len());
    let mut matched = 0usize;
    let mut failed_batches = 0usize;

    for (b, chunk) in inputs.chunks(BATCH_SIZE).enumerate() {
        on_progress(
            (b as f64 + 0.1) / total_batches as f64,
            format!("融合批次 {}/{}", b + 1, total_batches),
        );
        let prompt = build_prompt(ocr_texts, chunk);
        // 解析失败重试一次；仍失败则整批降级为 ASR 原文本，不阻塞流水线
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
                let out = merge_results(ocr_texts, chunk, raw);
                matched += out.iter().filter(|s| s.matched).count();
                segments.extend(out);
            }
            Err(e) => {
                failed_batches += 1;
                eprintln!("[fuse] 批次 {} 解析失败，保留 ASR 原文本: {}", b + 1, e);
                segments.extend(merge_results(ocr_texts, chunk, Vec::new()));
            }
        }
    }

    on_progress(1.0, format!("融合完成，共 {} 段", segments.len()));
    Ok(FuseResult {
        segments,
        stats: FuseStats {
            total: inputs.len(),
            matched,
            failed_batches,
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
        assert!(p.contains("ASR[1] 语音1"));
        assert!(p.contains("ASR[2] 语音2"));
    }

    #[test]
    fn test_build_prompt_strips_newlines() {
        let ocr = vec!["第一行\n第二行".into()];
        let p = build_prompt(&ocr, &sample_inputs(1));
        assert!(p.contains("第一行 第二行"));
    }

    #[test]
    fn test_parse_fusion_output_plain_json() {
        let raw = r#"{"segments":[{"index":1,"ocr_index":1,"character":"派蒙"}]}"#;
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].index, 1);
        assert_eq!(v[0].ocr_index, Some(1));
        assert_eq!(v[0].character.as_deref(), Some("派蒙"));
    }

    #[test]
    fn test_parse_fusion_output_with_noise() {
        // 模型夹带围栏/说明文字：截取花括号片段
        let raw = "好的，结果如下：\n```json\n{\"segments\":[{\"index\":2,\"ocr_index\":0,\"character\":\"\"}]}\n```";
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v[0].index, 2);
        assert_eq!(v[0].ocr_index, None, "ocr_index 0 视为无对应");
        assert_eq!(v[0].character, Some(String::new()));
    }

    #[test]
    fn test_parse_fusion_output_string_index() {
        // 3B 模型实测会回显 "ASR[1]" 带前缀字符串：宽容提取数字
        let raw = r#"{"segments":[{"index":"ASR[1]","ocr_index":"OCR[2]","character":"派蒙"},{"index":"2","ocr_index":null,"character":""}]}"#;
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].index, 1);
        assert_eq!(v[0].ocr_index, Some(2));
        assert_eq!(v[1].index, 2);
        assert_eq!(v[1].ocr_index, None, "null 视为无对应");
    }

    #[test]
    fn test_parse_fusion_output_garbage_errors() {
        assert!(parse_fusion_output("完全不是 JSON").is_err());
        assert!(parse_fusion_output("{\"broken\": [").is_err());
    }

    #[test]
    fn test_strip_prefix() {
        assert_eq!(strip_prefix("派蒙：旅行者你来了", Some("派蒙")), "旅行者你来了");
        assert_eq!(strip_prefix("Paimon: hello", Some("Paimon")), "hello");
        // 角色名与文本无前缀/不匹配：原样保留
        assert_eq!(strip_prefix("旅行者你来了", Some("派蒙")), "旅行者你来了");
        assert_eq!(strip_prefix("派蒙：旅行者你来了", None), "派蒙：旅行者你来了");
    }

    #[test]
    fn test_merge_results_partial_match() {
        let ocr = vec!["派蒙：旅行者你来了".to_string(), "安柏：前方有敌人".to_string()];
        let inputs = sample_inputs(3);
        let raw = vec![
            RawFusedSegment { index: 1, ocr_index: Some(1), character: Some("派蒙".into()) },
            RawFusedSegment { index: 3, ocr_index: None, character: None },
        ];
        let out = merge_results(&ocr, &inputs, raw);
        assert_eq!(out.len(), 3);
        // index 1 → OCR[1] 去角色名前缀
        assert_eq!(out[0].text, "旅行者你来了");
        assert_eq!(out[0].character.as_deref(), Some("派蒙"));
        assert!(out[0].matched);
        // index 2 未匹配：保留 ASR 原文本
        assert_eq!(out[1].text, "语音2");
        assert!(!out[1].matched);
        assert_eq!(out[1].character, None);
        // index 3：无对应 OCR → ASR 原文本
        assert_eq!(out[2].text, "语音3");
        assert!(!out[2].matched);
    }

    #[test]
    fn test_merge_results_ignores_out_of_range_and_dup() {
        let ocr = vec!["派蒙：旅行者你来了".to_string()];
        let inputs = sample_inputs(2);
        let raw = vec![
            RawFusedSegment { index: 1, ocr_index: Some(1), character: Some("派蒙".into()) },
            RawFusedSegment { index: 1, ocr_index: Some(2), character: None },
            RawFusedSegment { index: 99, ocr_index: Some(1), character: None },
        ];
        let out = merge_results(&ocr, &inputs, raw);
        assert_eq!(out[0].text, "语音1", "重复 index 后者覆盖；其 ocr_index=2 越界 → 保留 ASR 原文");
        assert_eq!(out[1].text, "语音2", "越界 index 不影响");
    }
}
