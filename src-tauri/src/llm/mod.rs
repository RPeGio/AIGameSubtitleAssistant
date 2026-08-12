// ─── LLM 编排模块 ─────────────────────────────────────────
// 串起一次 LLM 推理（prompt → llama-cli 回答），仿 asr 模块的 run_asr 结构：
// 后台线程 + 进度事件 + 返回文本。
// 进度事件先保留观察：0.5B 秒级完成，但换 3B 后推理耗时可观。

use crate::ai_runtime::LlmManager;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// LLM 进度事件名（前端需保持一致）
pub const LLM_PROGRESS_EVENT: &str = "llm-progress";

/// 测试台默认生成上限（融合批在 fuse 模块另行指定更大值）
const MAX_TOKENS: u32 = 256;

/// 进度事件载荷。LLM 为单次推理，无 clip 概念，只有整体进度 0.0 ~ 1.0。
#[derive(Clone, Serialize)]
pub struct LlmProgress {
    /// 整体进度 0.0 ~ 1.0
    pub progress: f64,
    pub message: String,
}

/// 执行一次 LLM 推理。
///
/// 抽取为独立函数便于集成测试直接调用（不依赖 Tauri 命令栈）。
/// `on_progress(progress, message)` 在推理前/后各调用一次。
pub fn run_llm_pipeline<F>(manager: &LlmManager, prompt: &str, mut on_progress: F) -> Result<String, String>
where
    F: FnMut(f64, String),
{
    let ready = manager.with_provider(|p| p.is_ready());
    if !ready {
        return Err("LLM 运行环境未就绪".into());
    }

    on_progress(0.1, "推理中…".into());
    let answer = manager
        .with_provider(|p| p.complete(prompt, MAX_TOKENS))
        .map_err(|e| format!("LLM 推理失败: {}", e))?;

    on_progress(1.0, "完成".into());
    Ok(answer)
}

/// Tauri 命令：执行一次 LLM 推理。
///
/// 在后台线程跑（不阻塞 UI），过程中通过 `llm-progress` 事件上报进度，
/// 返回回答文本，由前端展示。
#[tauri::command]
pub async fn run_llm(app: AppHandle, prompt: String) -> Result<String, String> {
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<LlmManager>();
        run_llm_pipeline(&manager, &prompt, |progress, message| {
            let _ = app_handle.emit(LLM_PROGRESS_EVENT, LlmProgress { progress, message });
        })
    })
    .await
    .map_err(|e| format!("LLM 任务内部错误: {}", e))?
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::RuntimeConfig;
    use std::path::PathBuf;

    #[test]
    fn test_pipeline_not_ready_errors() {
        let manager = LlmManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        let mut called = false;
        let result = run_llm_pipeline(&manager, "hi", |_, _| called = true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未就绪"));
        assert!(!called, "未就绪时不应触发进度回调");
    }
}
