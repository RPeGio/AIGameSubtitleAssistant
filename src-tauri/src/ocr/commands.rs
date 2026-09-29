use tauri::{AppHandle, Emitter, Manager};
use crate::project::CorpusItem;
use crate::ai_runtime::OcrManager;
use crate::ai_runtime::OcrResult;
use super::correct::{Diff, replace_diff_forms};
use super::params::{OcrProgress, OcrRegionInput, OcrRunParams, OcrSegment};
use super::pipeline::{b64, run_ocr_pipeline};

/// OCR 进度事件名（前端 `OCR_PROGRESS_EVENT` 需保持一致）
pub const OCR_PROGRESS_EVENT: &str = "ocr-progress";

/// Tauri 命令：串联完整 OCR 流水线。
///
/// 在后台线程跑（不阻塞 UI），过程中通过 `ocr-progress` 事件上报进度；
/// 返回 (按时间排序的 OcrSegment 列表, 待审批的文本纠正 Diff 列表)。
/// 文本保持"标点归一化后、未精化"形态——纠正由前端审批后执行（2026-09-24 用户决策）。
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
) -> Result<(Vec<OcrSegment>, Vec<Diff>), String> {
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

        // 逐批读取图片字节并 base64 编码：几十张高分辨率截图一次性读入会推高
        // 峰值内存（base64 再放大 ~1.33×），按 IPC 批消费，单批用完即弃
        let bs = batch_size.max(1);
        let total_batches = image_paths.len().div_ceil(bs);
        let mut results = Vec::new();
        for (i, paths) in image_paths.chunks(bs).enumerate() {
            let images: Vec<String> = paths
                .iter()
                .map(|src| {
                    std::fs::read(src)
                        .map(|bytes| b64(&bytes))
                        .map_err(|e| format!("读取图片失败（{}）: {}", src, e))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let batch = manager
                .with_provider(|p| p.recognize_batch(&images))
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

/// Tauri 命令：采纳一条语料纠正（前端逐条审批后调用）。
///
/// 对**全部**语料条目执行「误读形态 → 纠正文本」替换，并按入库口径去重
/// ——替换可能与既有正确条目撞成同文（如语料里本就有「缟玛瑙…」），
/// 去重规则：文本全等、保留首现、保序（原前端逻辑下沉至此，与 `pushCorpusTexts`
/// 的入库去重同口径）。
///
/// 待审批列表（`Project.corpus_ocr_diffs`）仍由前端管理：采纳/放弃都由前端移除条目
/// （放弃无文本变更，不需要命令），后端只负责文本替换这一步。
/// 同步命令即可：纯 CPU、量级 = 条数 × old 数次 replace，微秒级。
#[tauri::command]
pub fn approve_corpus_diff(
    corpus: Vec<CorpusItem>,
    diff: Diff,
) -> Vec<CorpusItem> {
    let mut out = corpus;
    for item in &mut out {
        item.text = replace_diff_forms(&item.text, &diff.old, &diff.new);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|item| seen.insert(item.text.clone()));
    out
}

// ─── 单元测试 ─────────────────────────────────────────────

