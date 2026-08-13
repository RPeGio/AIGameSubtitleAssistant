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
/// 融合批生成上限：30 段 JSON 输出（text + character）需要大余量；
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
/// 跨语言语义对齐：LLM 自己建立对应关系，只输出 JSON。
fn build_prompt(ocr_texts: &[String], batch: &[FuseAsrInput]) -> String {
    let mut p = String::new();
    p.push_str(
        "你是游戏字幕融合助手。下面是可靠的剧情字幕文本（OCR）和游戏语音转写文本（ASR，语言可能与字幕不同）。\n\
         请把每条 ASR 文本与语义相同的 OCR 字幕文本对应（跨语言对应），输出最终字幕文本：\n\
         - 找到对应字幕：最终文本用 OCR 字幕文本（去掉角色名前缀，如“派蒙：”），并提取角色名；\n\
         - 找不到对应：最终文本保留 ASR 原文，角色名为空。\n\
         - 最终输出文本语言与字幕文本语言一致。\n\
         修正 OCR/ASR 中的明显识别错误，如对于 OCR 文本中明显为被截断的，且后面紧跟着完整文本的对话，合并二者时间为一段并采用其中完整的那段对话。\n\
         严格只输出 JSON，严禁输出任何其他内容，格式：{\"segments\":[{\"index\":ASR编号,\"text\":\"最终文本\",\"character\":\"角色名或空\"}]}\n\
         index 必须是 ASR[n] 里的编号。\n\n",
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

/// LLM 输出的单段（index = ASR 输入编号）
#[derive(Debug, Deserialize)]
struct RawFusedSegment {
    index: usize,
    text: String,
    #[serde(default)]
    character: Option<String>,
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

/// 把 LLM 解析结果合并回输入序列：未输出/越界 index 保留 ASR 原文本；
/// 重复 index 以后者为准。
fn merge_results(inputs: &[FuseAsrInput], raw: Vec<RawFusedSegment>) -> Vec<FusedSegment> {
    let mut by_index: std::collections::HashMap<usize, RawFusedSegment> = std::collections::HashMap::new();
    for r in raw {
        by_index.insert(r.index, r);
    }
    inputs
        .iter()
        .map(|s| match by_index.get(&s.index) {
            Some(r) => FusedSegment {
                start: s.start,
                end: s.end,
                text: r.text.trim().to_string(),
                character: r
                    .character
                    .as_ref()
                    .filter(|c| !c.trim().is_empty())
                    .cloned(),
                matched: true,
            },
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
                let out = merge_results(chunk, raw);
                matched += out.iter().filter(|s| s.matched).count();
                segments.extend(out);
            }
            Err(e) => {
                failed_batches += 1;
                eprintln!("[fuse] 批次 {} 解析失败，保留 ASR 原文本: {}", b + 1, e);
                segments.extend(merge_results(chunk, Vec::new()));
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
        let raw = r#"{"segments":[{"index":1,"text":"旅行者，你来了","character":"派蒙"}]}"#;
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].index, 1);
        assert_eq!(v[0].text, "旅行者，你来了");
        assert_eq!(v[0].character.as_deref(), Some("派蒙"));
    }

    #[test]
    fn test_parse_fusion_output_with_noise() {
        // 模型夹带围栏/说明文字：截取花括号片段
        let raw = "好的，结果如下：\n```json\n{\"segments\":[{\"index\":2,\"text\":\"你好\",\"character\":\"\"}]}\n```";
        let v = parse_fusion_output(raw).unwrap();
        assert_eq!(v[0].index, 2);
        assert_eq!(v[0].character, Some(String::new()));
    }

    #[test]
    fn test_parse_fusion_output_garbage_errors() {
        assert!(parse_fusion_output("完全不是 JSON").is_err());
        assert!(parse_fusion_output("{\"broken\": [").is_err());
    }

    #[test]
    fn test_merge_results_partial_match() {
        let inputs = sample_inputs(3);
        let raw = vec![
            RawFusedSegment { index: 1, text: " 派蒙台词 ".into(), character: Some("派蒙".into()) },
            RawFusedSegment { index: 3, text: "第三条".into(), character: None },
        ];
        let out = merge_results(&inputs, raw);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "派蒙台词");
        assert_eq!(out[0].character.as_deref(), Some("派蒙"));
        assert!(out[0].matched);
        // index 2 未匹配：保留 ASR 原文本
        assert_eq!(out[1].text, "语音2");
        assert!(!out[1].matched);
        assert_eq!(out[1].character, None);
        assert_eq!(out[2].text, "第三条");
        assert!(out[2].matched);
    }

    #[test]
    fn test_merge_results_ignores_out_of_range_and_dup() {
        let inputs = sample_inputs(2);
        let raw = vec![
            RawFusedSegment { index: 1, text: "第一".into(), character: None },
            RawFusedSegment { index: 1, text: "第一覆盖".into(), character: None },
            RawFusedSegment { index: 99, text: "越界".into(), character: None },
        ];
        let out = merge_results(&inputs, raw);
        assert_eq!(out[0].text, "第一覆盖", "重复 index 后者覆盖");
        assert_eq!(out[1].text, "语音2", "越界 index 不影响");
    }
}
