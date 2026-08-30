// ─── ASR 编排模块 ─────────────────────────────────────────
// 串起完整 ASR 流水线（提取音频 → MOSS 转写），仿 ocr 模块的
// run_ocr 结构：后台线程 + 进度事件 + 返回段列表由前端写入轨道。

use crate::ai_runtime::moss::SEGMENT_SECONDS;
use crate::ai_runtime::{AsrManager, AsrSegment, AsrTranscribeOptions};
use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// ASR 进度事件名（前端需保持一致）
pub const ASR_PROGRESS_EVENT: &str = "asr-progress";

/// ASR 运行参数（前端配置面板传入）
#[derive(Deserialize)]
pub struct AsrRunParams {
    /// 引擎："funasr" | "moss"
    pub engine: String,
    /// 说话人上限：None = 自动估计（仅 funasr+diarize 生效；moss 自动估计）
    pub max_speakers: Option<u32>,
    /// 识别语言：None/空 = 自动检测（仅 funasr 生效；moss 自动识别）
    pub language: Option<String>,
}

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

/// 串起完整 ASR 流水线（提取音频 → 按面板参数选择引擎转写）。
///
/// 抽取为独立函数便于集成测试直接调用（不依赖 Tauri 命令栈）。
/// `on_progress(progress, message)` 每次阶段推进调用一次；转写为
/// 单次阻塞调用（CLI 无进度输出），中途只发一次"转写中"提示。
pub fn run_asr_pipeline<F>(
    manager: &AsrManager,
    video_path: &str,
    params: &AsrRunParams,
    mut on_progress: F,
) -> Result<Vec<AsrSegment>, String>
where
    F: FnMut(f64, String),
{
    let engine = if params.engine == "moss" {
        "moss"
    } else {
        "funasr"
    };
    let ready = manager.with_engine(engine, |p| p.is_ready());
    if !ready {
        return Err(format!("ASR 运行环境未就绪（{}）", engine));
    }
    // 后端重入守卫：Arc 去锁后无天然串行，防并发转写互相覆盖 active_child /
    // 互相清理对方子进程（前端 asrRunning 已防 UI 路径，此为兜底）
    if !manager.try_begin_transcribe() {
        return Err("已有 ASR 转写在进行中，请等待完成".into());
    }
    struct TranscribeGuard<'a>(&'a AsrManager);
    impl Drop for TranscribeGuard<'_> {
        fn drop(&mut self) {
            self.0.finish_transcribe();
        }
    }
    let _guard = TranscribeGuard(manager);

    // 一次完整转写任务开始：清除引擎上一次的取消残留
    // （MOSS 分段转写逐段调用 transcribe，取消标志按任务重置，见 AsrProvider::reset）
    manager.with_engine(engine, |p| p.reset());

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

    on_progress(0.2, format!("{} 转写中…（可能需要数分钟）", engine));
    let options = AsrTranscribeOptions {
        language: params.language.clone(),
        max_speakers: params.max_speakers,
    };
    // MOSS 长音频按固定 5 分钟分段转写：规避单次 prefill 显存 OOM 与
    // max_new=5120 截断，且逐段上报进度（已完成段数/总段数，供前端显示）。
    // FunASR 单次转写（worker 自带分段），进度保持 0.2 直至完成。
    let segments = if engine == "moss" {
        let seg_seconds = crate::ai_runtime::moss::SEGMENT_SECONDS as f64;
        let info = crate::ai_runtime::moss::wav_info(&audio);
        let total = info.as_ref().map(|i| i.seconds).unwrap_or(0.0);
        if total <= seg_seconds {
            // 短音频单段直转
            manager
                .with_engine(engine, |p| p.transcribe(&audio, &options))
                .map_err(|e| format!("ASR 转写失败: {}", e))?
        } else {
            // 长音频分段：切片 → 逐段转写 → 段内偏移合并 → 段完成上报进度
            let info = info.ok_or("MOSS 长音频分段需要可解析的 PCM WAV 头")?;
            // total 为 f64 秒，ceil 后按整秒切段，避免丢弃尾部的亚秒音频
            let n_segs = (total.ceil() as u32).div_ceil(SEGMENT_SECONDS);
            let mut all: Vec<AsrSegment> = Vec::new();
            for seg in 0..n_segs {
                let start = seg * SEGMENT_SECONDS;
                let end = ((seg + 1) * SEGMENT_SECONDS).min(total.ceil() as u32);
                let seg_wav = base_dir.join(format!("moss_seg_{}.wav", seg));
                crate::ai_runtime::moss::slice_wav(&audio, &info, start, end, &seg_wav)
                    .map_err(|e| e.to_string())?;
                if dev_debug {
                    eprintln!("[asr] MOSS 分段 {}/{}: [{}-{}s]", seg + 1, n_segs, start, end);
                }
                let segs = manager
                    .with_engine(engine, |p| p.transcribe(&seg_wav, &options))
                    .map_err(|e| format!("ASR 转写失败（第 {} 段）: {}", seg + 1, e))?;
                let _ = std::fs::remove_file(&seg_wav);
                // 段完成即上报：已完成段数/总段数（0.25~0.95 区间）
                on_progress(
                    0.25 + 0.70 * ((seg + 1) as f64 / n_segs as f64),
                    format!("MOSS 分段转写 {}/{}（第 {} 段 {}-{}s）", seg + 1, n_segs, seg + 1, start, end),
                );
                for mut s in segs {
                    s.start += start as f64;
                    s.end += start as f64;
                    all.push(s);
                }
            }
            all
        }
    } else {
        manager
            .with_engine(engine, |p| p.transcribe(&audio, &options))
            .map_err(|e| format!("ASR 转写失败: {}", e))?
    };
    drop(options);

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
    params: AsrRunParams,
) -> Result<Vec<AsrSegment>, String> {
    let app_handle = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let manager = app_handle.state::<AsrManager>();
        run_asr_pipeline(&manager, &video_path, &params, |progress, message| {
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
        let params = AsrRunParams {
            engine: "funasr".into(),
            max_speakers: None,
            language: None,
        };
        let mut called = false;
        let result = run_asr_pipeline(&manager, "dummy.mp4", &params, |_, _| called = true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("未就绪"));
        assert!(!called, "未就绪时不应触发进度回调");
    }
}
