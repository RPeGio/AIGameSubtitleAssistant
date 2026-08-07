// ─── MOSS 转写 CLI Provider ───────────────────────────────
// 一次性子进程：`moss-transcribe transcribe <model.gguf> <audio.wav> --format json`。
// stdout 输出 JSON 段数组，stderr 是日志（量小，可整体收集）。
// 模型每次转写加载一次（~1.4s），相对长音频推理（数分钟）可忽略，故不做常驻进程。

use crate::ai_runtime::{AsrError, AsrProvider, AsrSegment, RuntimeConfig};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// MOSS CLI 输出的单段（JSON 数组元素）。多余字段（id）由 serde 默认忽略。
#[derive(Deserialize)]
struct RawSegment {
    start: f64,
    end: f64,
    speaker: String,
    text: String,
}

/// 解析 `--format json` 的 stdout：JSON 段数组 → AsrSegment 列表。
/// MOSS 不输出置信度，confidence 置 None。
fn parse_segments(json: &str) -> Result<Vec<AsrSegment>, AsrError> {
    let raw: Vec<RawSegment> = serde_json::from_str(json)
        .map_err(|e| AsrError::Worker(format!("MOSS 输出解析失败: {}", e)))?;
    Ok(raw
        .into_iter()
        .map(|s| AsrSegment {
            start: s.start,
            end: s.end,
            text: s.text,
            speaker: Some(s.speaker),
            confidence: None,
        })
        .collect())
}

/// 相对 runtime 的路径（机器无关）join runtime 目录；绝对路径（老配置）原样使用
fn resolve_path(p: &str, runtime_dir: &Path) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        runtime_dir.join(p)
    }
}

/// MOSS 转写 provider —— 每次 transcribe 启动一次性子进程
pub struct MossProvider {
    binary: PathBuf,
    model: PathBuf,
    /// 推理线程数：0 = 不设置 MTD_THREADS（CLI 默认全核）
    threads: u32,
    ready: AtomicBool,
    /// 最近一次错误（供 describe() 向状态探测展示）
    last_error: Mutex<Option<String>>,
}

impl MossProvider {
    /// 构造 provider 并用 `info <model>` 探测 GGUF 可加载。探测失败不视为致命，仅 ready=false。
    pub fn spawn(config: &RuntimeConfig, runtime_dir: &Path) -> Result<Self, AsrError> {
        let binary = resolve_path(&config.moss_binary, runtime_dir);
        let model = resolve_path(&config.moss_model, runtime_dir);

        if !binary.is_file() {
            return Err(AsrError::Worker(format!(
                "MOSS 可执行文件不存在: {}",
                binary.display()
            )));
        }
        if !model.is_file() {
            return Err(AsrError::Worker(format!(
                "MOSS 模型不存在: {}",
                model.display()
            )));
        }

        let provider = MossProvider {
            binary,
            model,
            threads: config.moss_threads,
            ready: AtomicBool::new(false),
            last_error: Mutex::new(None),
        };
        if let Err(e) = provider.probe() {
            provider.set_error(&e.to_string());
        } else {
            provider.ready.store(true, Ordering::SeqCst);
        }
        Ok(provider)
    }

    /// 探测：`info <model>` 成功说明 GGUF 元数据可加载
    fn probe(&self) -> Result<(), AsrError> {
        let output = Command::new(&self.binary)
            .arg("info")
            .arg(&self.model)
            .output()
            .map_err(|e| AsrError::Worker(format!("无法启动 MOSS: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AsrError::Worker(format!(
                "MOSS info 探测失败（{}）：{}",
                output.status,
                stderr.trim()
            )));
        }
        Ok(())
    }

    fn set_error(&self, msg: &str) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(msg.to_string());
        }
    }

    /// 执行一次转写（阻塞，长音频可达数分钟；由编排层放后台线程）
    fn run_transcribe(&self, audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("transcribe").arg(&self.model).arg(audio_path);
        if self.threads > 0 {
            cmd.env("MTD_THREADS", self.threads.to_string());
        }
        let output = cmd
            .output()
            .map_err(|e| AsrError::Worker(format!("无法启动 MOSS 转写: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AsrError::Worker(format!(
                "MOSS 转写失败（{}）：{}",
                output.status,
                stderr.trim()
            )));
        }
        parse_segments(&String::from_utf8_lossy(&output.stdout))
    }
}

impl AsrProvider for MossProvider {
    fn name(&self) -> &str {
        "moss"
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        self.last_error.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
    }

    fn transcribe(&self, audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        if !self.is_ready() {
            return Err(AsrError::NotReady);
        }
        let result = self.run_transcribe(audio_path);
        if let Err(e) = &result {
            self.set_error(&e.to_string());
        }
        result
    }
}

// ─── 单元测试（纯协议逻辑，不依赖真实 exe/模型）──────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_segments_ok() {
        let json = r#"[
            {"id":"seg_0001","start":0.03,"end":0.45,"speaker":"S01","text":"The road."},
            {"id":"seg_0002","start":0.81,"end":1.35,"speaker":"S02","text":"Gee."}
        ]"#;
        let segs = parse_segments(json).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start, 0.03);
        assert_eq!(segs[0].end, 0.45);
        assert_eq!(segs[0].text, "The road.");
        assert_eq!(segs[0].speaker.as_deref(), Some("S01"));
        assert_eq!(segs[0].confidence, None);
        assert_eq!(segs[1].speaker.as_deref(), Some("S02"));
    }

    #[test]
    fn test_parse_segments_empty_array() {
        let segs = parse_segments("[]").unwrap();
        assert!(segs.is_empty());
    }

    #[test]
    fn test_parse_segments_invalid_json() {
        let result = parse_segments("not json");
        assert!(matches!(result, Err(AsrError::Worker(_))));
    }

    #[test]
    fn test_parse_segments_missing_field_errors() {
        let json = r#"[{"start":0.1,"end":0.5,"speaker":"S01"}]"#;
        assert!(parse_segments(json).is_err());
    }

    #[test]
    fn test_resolve_path() {
        let runtime = Path::new("runtime");
        assert_eq!(
            resolve_path("bin/moss-transcribe.exe", runtime),
            runtime.join("bin/moss-transcribe.exe")
        );
        let abs = if cfg!(windows) {
            "C:\\x\\moss-transcribe.exe"
        } else {
            "/x/moss-transcribe.exe"
        };
        assert_eq!(resolve_path(abs, runtime), PathBuf::from(abs));
    }
}
