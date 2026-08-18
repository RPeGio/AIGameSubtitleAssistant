// ─── ASR 编排模块 ─────────────────────────────────────────
// 串起完整 ASR 流水线（提取音频 → MOSS 转写），仿 ocr 模块的
// run_ocr 结构：后台线程 + 进度事件 + 返回段列表由前端写入轨道。

use crate::ai_runtime::{AsrManager, AsrSegment};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// ASR 进度事件名（前端需保持一致）
pub const ASR_PROGRESS_EVENT: &str = "asr-progress";

/// 进度事件载荷。ASR 为单次转写，无 clip 概念，只有整体进度 0.0 ~ 1.0。
#[derive(Clone, Serialize)]
pub struct AsrProgress {
    /// 整体进度 0.0 ~ 1.0
    pub progress: f64,
    pub message: String,
}

/// RAII 临时目录守卫：任何退出路径（含 panic 展开、JoinHandle 错误）都会清理。
/// `keep` 为 true 时保留目录（dev_debug 用于人工检查音频文件）。
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

/// 串起完整 ASR 流水线（提取音频 → MOSS 转写）。
///
/// 抽取为独立函数便于集成测试直接调用（不依赖 Tauri 命令栈）。
/// `on_progress(progress, message)` 每次阶段推进调用一次；转写为
/// 单次阻塞调用（CLI 无进度输出），中途只发一次"转写中"提示。
pub fn run_asr_pipeline<F>(
    manager: &AsrManager,
    video_path: &str,
    mut on_progress: F,
) -> Result<Vec<AsrSegment>, String>
where
    F: FnMut(f64, String),
{
    let ready = manager.with_provider(|p| p.is_ready());
    if !ready {
        return Err("ASR 运行环境未就绪".into());
    }

    let dev_debug = manager.config().dev_debug;

    // 临时目录：dev 时写仓库根 temp/asr/<uuid> 并保留；否则系统临时目录 + 自动清理
    let base_dir = if dev_debug {
        let repo = manager
            .runtime_dir()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir());
        repo.join("temp")
            .join("asr")
            .join(Uuid::new_v4().to_string())
    } else {
        std::env::temp_dir().join(format!("gsa_asr_{}", Uuid::new_v4()))
    };
    let _guard = TempDirGuard {
        path: base_dir.clone(),
        keep: dev_debug,
    };
    std::fs::create_dir_all(&base_dir).map_err(|e| format!("无法创建临时目录: {}", e))?;
    if dev_debug {
        eprintln!("[asr] 音频目录: {}", base_dir.display());
    }

    on_progress(0.05, "提取音频…".into());
    let audio = crate::video::extract_audio(video_path, &base_dir)
        .map_err(|e| format!("音频提取失败: {}", e))?;
    if dev_debug {
        eprintln!("[asr] 音频: {}", audio.display());
    }

    on_progress(0.2, "ASR 转写中…（可能需要数分钟）".into());
    let segments = manager
        .with_provider(|p| p.transcribe(&audio))
        .map_err(|e| format!("ASR 转写失败: {}", e))?;

    if dev_debug {
        eprintln!("[asr] 转写完成，共 {} 段：", segments.len());
        for seg in &segments {
            eprintln!(
                "[asr]   [{} → {}] <{}> \"{}\"",
                crate::ocr::fmt_time(seg.start),
                crate::ocr::fmt_time(seg.end),
                seg.speaker.as_deref().unwrap_or("-"),
                seg.text
            );
        }
    }

    on_progress(1.0, format!("转写完成，共 {} 段", segments.len()));
    Ok(segments)
}

/// Tauri 命令：串联完整 ASR 流水线。
///
/// 在后台线程跑（不阻塞 UI），过程中通过 `asr-progress` 事件上报进度，
/// 返回按时间排序的 AsrSegment 列表，由前端写入 asr 轨道。
#[tauri::command]
pub async fn run_asr(
    app: AppHandle,
    video_path: String,
) -> Result<Vec<AsrSegment>, String> {
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<AsrManager>();
        run_asr_pipeline(&manager, &video_path, |progress, message| {
            let _ = app_handle.emit(ASR_PROGRESS_EVENT, AsrProgress { progress, message });
        })
    })
    .await
    .map_err(|e| format!("ASR 任务内部错误: {}", e))?
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::RuntimeConfig;
    use std::path::PathBuf;

    #[test]
    fn test_pipeline_not_ready_errors() {
        let manager = AsrManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        let mut called = false;
        let result = run_asr_pipeline(&manager, "dummy.mp4", |_, _| called = true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未就绪"));
        assert!(!called, "未就绪时不应触发进度回调");
    }
}
