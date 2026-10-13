// ─── AI 融合模块（Phase 4 → T4 向量召回路线）──────────────

// OCR 文本（可靠的剧情录屏字幕，可能含角色名前缀）与游戏内容时间轴文本
// （切片视频上带时间轴的"转写侧"：游戏语音 ASR 段，或无配音处的画面内嵌
// 字幕 OCR 段，通常与 OCR 文本不同语言）交由**向量召回 + 统一转移 DP**做跨
// 语言语义对齐。时间轴以转写侧段为准。
//
// 纯文本契约（2026-10-02 用户决策，仍适用）：本模块只做"下标映射 + 文本搬运"——
// 命中段逐字复制 OCR 原文（含换行与角色名行），未命中段保留转写侧原文；
// 两者都不改写、不剥离前缀、不做换行归一；**本模块不产出说话人/角色名**。
// T4/R2 起改用 runtime/worker/embed_worker.py 一次性子进程（ai_runtime/embed.rs
// 客户端），LLM 对齐路径已删除（见 FUSE_PIPELINE_DEFECTS.md F13）。
//
// **「同台词、跨说话人」缺口（F14，登记于 2026-10-15 待适配）**：从前端
// `withSpeakerLine` 的 speaker 注入随本体删除，导致一条语料由多说话人富含、
// 而另一些说话人重复说同一句话时，句子向量相同 ⇒ 相似度矩阵无法区分哪些
// 语料行属于哪一个说话人。改走模态路径之前，务必先适配 F14（结构化说话人
// 输入或按钮分说话人块），在登记可用之前，发布版本**不要启用向量融合**。

use crate::ai_runtime::embed::{
    run_embed_worker, EmbedRequest, EmbedSegmentIn, EmbedSegmentOut,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 输入：一个游戏内容时间轴段（转写侧，index = 输入顺序，worker 输出按此对应）。
/// 来源可为 game-ASR 游戏语音，或画面内嵌字幕 OCR（embed_ocr，无配音场景）。
/// 文本**原样**录入（保留换行），不加说话人前缀（F14 见上文）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseAsrInput {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 候选名单条目（1-based 语料下标 + 该行的相似度）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FuseCandidate {
    pub ocr_index: usize,
    pub score: f64,
}

/// 前端逐段审计用的融合元数据。
/// 所有字段都由 embed worker 产出（R2 只做透传），`score` **不是**正确性信号
/// （任何阈值下精确率 ≈ 错误基准率 4.2%，见 FUSE_THRESHOLD_CALIBRATION.md §7.9），
/// 审计表必须依赖 transition / reused / masked / matched 这些结构化信号。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FuseMeta {
    /// 命中的语料下标（1-based）；0 = 不配
    pub ocr_index: usize,
    /// 命中行的余弦相似度；不配时为 0.0
    pub score: f64,
    /// 候选名单（相似度降序，含命中的那条）
    pub candidates: Vec<FuseCandidate>,
    /// 该段被「该不配」判据整行封死
    pub masked: bool,
    /// forward / repeat / reset / unmatched
    pub transition: String,
    /// 该段命中的语料行被 >=2 段命中
    pub reused: bool,
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
    /// 「该不配」掩码命中数（该段被判必不配；是精确判据，假阳性实测 0）
    pub masked_segments: usize,
    /// 「候选语料行被重复使用」的段数（同一段语料被 ≥2 段命中）
    pub reused_segments: usize,
}

/// 融合完整结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuseResult {
    pub segments: Vec<FusedSegment>,
    /// 与 segments 一一对应的审计元数据（若 worker 输出不含则 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metas: Option<Vec<FuseMeta>>,
    pub stats: FuseStats,
}

/// 将 worker 输出（EmbedSegmentOut 列表）映射为 FusedSegment（纯文本搬运）。
/// `ocr_index` 1-based；0/-1 = 不配。越界视为 worker 异常，返回 Err 而非静默降级
/// （防假绿：LLM 时代"越界当未命中"静默级联建的坑，向量路径不再复制它）。
fn map_worker_segments(
    ocr_texts: &[String],
    out: &[EmbedSegmentOut],
) -> Result<Vec<FusedSegment>, String> {
    let n = ocr_texts.len();
    out.iter()
        .map(|s| {
            let oi = s.ocr_index;
            let hit = oi >= 1 && (oi as usize) <= n;
            if oi > 0 && !hit {
                return Err(format!(
                    "worker 返回越界语料下标（{}）大于语料条数 {}",
                    oi, n
                ));
            }
            Ok(FusedSegment {
                start: s.start,
                end: s.end,
                text: if hit {
                    ocr_texts[(oi - 1) as usize].clone()
                } else {
                    s.product_text.clone()
                },
                matched: hit,
            })
        })
        .collect()
}

/// 元数据结构化：Rust 侧不再重算任何相似度/掩码逻辑，worker 已给答案。
fn map_worker_meta(out: &EmbedSegmentOut) -> FuseMeta {
    FuseMeta {
        // 0 = 不配（worker 用 -1），前端和 Rust 立即取 0 语义
        ocr_index: out.ocr_index.max(0) as usize,
        score: out.score,
        candidates: out
            .candidates
            .iter()
            .map(|(idx, score)| FuseCandidate {
                ocr_index: (*idx).max(0) as usize,
                score: *score,
            })
            .collect(),
        masked: out.masked,
        transition: out.transition.clone(),
        reused: out.reused,
    }
}

/// 串起完整融合流水线（一次性子进程调用向量 worker）。
/// `on_progress(progress, message)` 进度目前只上报两步（启动 / 完成），因为
/// 端到端 <3s，分段进度没意义；保留接口与旧前端节奏兼容。
pub fn fuse_pipeline<F>(
    config: &crate::ai_runtime::RuntimeConfig,
    runtime_dir: &std::path::Path,
    ocr_texts: &[String],
    inputs: &[FuseAsrInput],
    unmatch_filter: bool,
    mut on_progress: F,
) -> Result<FuseResult, String>
where
    F: FnMut(f64, String),
{
    if ocr_texts.is_empty() {
        return Err("没有可用的 OCR 字幕文本".into());
    }
    if inputs.is_empty() {
        return Err("没有可用的游戏内容段（ASR 或画面嵌字 OCR）".into());
    }

    on_progress(0.05, "准备向量融合…".into());
    let req = EmbedRequest {
        corpus: ocr_texts.to_vec(),
        segments: inputs
            .iter()
            .map(|s| EmbedSegmentIn {
                index: s.index,
                start: s.start,
                end: s.end,
                text: s.text.clone(),
            })
            .collect(),
        unmatch_filter,
        top_k: 5,
    };
    let (segments, stats) = run_embed_worker(config, runtime_dir, &req)
        .map_err(|e| format!("向量融合失败：{}", e))?;
    on_progress(0.95, "融合完成，组装中…".into());

    if segments.len() != inputs.len() {
        return Err(format!(
            "worker 输出段数与输入不符（{} != {}）",
            segments.len(),
            inputs.len()
        ));
    }
    let mapped = map_worker_segments(ocr_texts, &segments)?;
    let metas: Vec<FuseMeta> = segments.iter().map(map_worker_meta).collect();
    let matched = mapped.iter().filter(|s| s.matched).count();
    // 掩码/复用计数直接用 worker 的单一真源统计（这里只做重排验证）
    let masked_segments = stats.masked_rows;
    let reused_segments = stats.reused_rows;

    on_progress(
        1.0,
        format!(
            "融合完成，共 {} 段（命中 {}，掩码抑制 {}）",
            mapped.len(),
            matched,
            masked_segments
        ),
    );
    Ok(FuseResult {
        segments: mapped,
        metas: Some(metas),
        stats: FuseStats {
            total: inputs.len(),
            matched,
            masked_segments,
            reused_segments,
        },
    })
}

/// Tauri 命令：执行 AI 融合（OCR 文本 + 游戏内容段 → 向量召回 → fused 段）。
/// 在后台线程跑（一次子进程秒级，未来扩常驻也不走事件循环），
/// 通过 `llm-progress` 事件上报进度（沿用既有事件名与 payload 结构）。
#[tauri::command]
pub async fn run_fuse(
    app: AppHandle,
    ocr_texts: Vec<String>,
    asr_segments: Vec<FuseAsrInput>,
    unmatch_filter: Option<bool>,
) -> Result<FuseResult, String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<crate::ai_runtime::EmbedManager>();
        fuse_pipeline(
            manager.config(),
            manager.runtime_dir(),
            &ocr_texts,
            &asr_segments,
            unmatch_filter.unwrap_or(false),
            |progress, message| {
                let _ = app_handle.emit(
                    crate::llm::LLM_PROGRESS_EVENT,
                    crate::llm::LlmProgress { progress, message },
                );
            },
        )
    })
    .await
    .map_err(|e| format!("AI 融合任务内部错误: {}", e))?
}
