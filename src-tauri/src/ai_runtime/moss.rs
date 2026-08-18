// ─── MOSS 转写 CLI Provider ───────────────────────────────
// 一次性子进程：`moss-transcribe transcribe <model.gguf> <audio.wav> --format json`。
// stdout 输出 JSON 段数组，stderr 是日志（量小，可整体收集）。
// 模型每次转写加载一次（~1.4s），相对长音频推理（数分钟）可忽略，故不做常驻进程。
//
// 进程生命周期：转写子进程登记在 active_child，支持：
//   - cancel()：取消转写（kill 子进程），供前端"取消 ASR"与超时使用
//   - 应用退出钩子调用 cancel()，避免关应用后孤儿 moss 进程残留占满 CPU
//
// 防残留叠加：每次转写前用 tasklist/taskkill 清理系统上历史残留的
// moss-transcribe 孤儿进程（异常崩溃等可能绕过退出钩子，导致多实例抢 CPU）。

use crate::ai_runtime::{
    resolve_path, AsrError, AsrProvider, AsrSegment, AsrTranscribeOptions, RuntimeConfig,
};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

/// MOSS 转写 provider —— 每次 transcribe 启动一次性子进程
pub struct MossProvider {
    binary: PathBuf,
    model: PathBuf,
    /// 推理线程数：0 = 不设置 MTD_THREADS（CLI 默认全核）
    threads: u32,
    /// 转写超时（分钟）：0 = 不限；超时终止子进程并报错
    timeout_minutes: u32,
    ready: AtomicBool,
    /// 取消标志：cancel() 置位，transcribe 轮询检测后返回"已取消"
    cancelled: AtomicBool,
    /// 最近一次错误（供 describe() 向状态探测展示）
    last_error: Mutex<Option<String>>,
    /// 当前活动的转写子进程（供取消/退出钩子终止）
    active_child: Mutex<Option<Child>>,
    /// 开发调试：打印运行时细节日志
    dev_debug: bool,
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
            timeout_minutes: config.moss_timeout_minutes,
            ready: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            last_error: Mutex::new(None),
            active_child: Mutex::new(None),
            dev_debug: config.dev_debug,
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

    /// 解析 `tasklist /FO CSV /NH` 输出，提取 moss-transcribe 的 PID 列表。
    /// 无进程时 tasklist 输出本地化提示行（非 CSV 格式），不匹配前缀天然跳过。
    /// CSV 行形如：`"moss-transcribe.exe","12345","Console","1","1,234 K"`
    fn parse_tasklist_pids(output: &str) -> Vec<u32> {
        output
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("\"moss-transcribe.exe\""))
            .filter_map(|l| {
                let mut fields = l.split(',');
                fields.next()?; // 映像名
                let pid = fields.next()?.trim_matches('"').parse::<u32>().ok()?;
                Some(pid)
            })
            .collect()
    }

    /// 终止系统上历史残留的 moss-transcribe 孤儿进程（本次子进程尚未
    /// spawn，不会误杀自己）。异常崩溃/被杀软强杀等路径可能绕过退出钩子，
    /// 残留进程会与下次转写叠加抢占 CPU，故每次转写前清理。
    #[cfg(windows)]
    fn cleanup_stale_moss_processes(&self) {
        let output = match Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq moss-transcribe.exe"])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                if self.dev_debug {
                    eprintln!("[moss] tasklist 调用失败，跳过残留清理: {}", e);
                }
                return;
            }
        };
        for pid in Self::parse_tasklist_pids(&String::from_utf8_lossy(&output.stdout)) {
            if self.dev_debug {
                eprintln!("[moss] 清理残留转写进程 PID {}", pid);
            }
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }
    }

    /// 非 Windows 平台：无 tasklist/taskkill，跳过清理（当前项目仅 Windows 部署）
    #[cfg(not(windows))]
    fn cleanup_stale_moss_processes(&self) {}

    /// 执行一次转写（阻塞，长音频可达数分钟；由编排层放后台线程）。
    /// 子进程登记在 active_child：轮询等待（可被 cancel 打断）+ 超时终止。
    fn run_transcribe(&self, audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        // 防残留叠加：清理历史孤儿进程（本次子进程尚未 spawn，不误杀自己）
        self.cleanup_stale_moss_processes();
        let mut cmd = Command::new(&self.binary);
        cmd.arg("transcribe")
            .arg(&self.model)
            .arg(audio_path)
            // 默认输出是原始流格式，--format json 才会输出结构化段
            .arg("--format")
            .arg("json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.threads > 0 {
            cmd.env("MTD_THREADS", self.threads.to_string());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AsrError::Worker(format!("无法启动 MOSS 转写: {}", e)))?;

        // 分离管道交给读线程：防止管道缓冲写满阻塞 moss 进程
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = stdout.map(|mut s| {
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                buf
            })
        });
        let err_reader = stderr.map(|mut s| {
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                buf
            })
        });

        {
            let mut slot = self
                .active_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(child);
        }

        // 轮询等待：可被 cancel 打断；timeout_minutes > 0 时超时终止
        let deadline = (self.timeout_minutes > 0)
            .then(|| Instant::now() + Duration::from_secs(self.timeout_minutes as u64 * 60));
        let status = loop {
            if self.cancelled.load(Ordering::SeqCst) {
                // cancel 可能落在子进程登记前的窗口（flag 已置但没 kill），
                // 此处幂等再调一次确保子进程被终止
                self.cancel();
                return Err(AsrError::Worker("MOSS 转写已取消".into()));
            }
            let mut slot = self
                .active_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match slot.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(Some(status)) => {
                        *slot = None;
                        break status;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        *slot = None;
                        return Err(AsrError::Worker(format!("MOSS 进程异常: {}", e)));
                    }
                },
                None => return Err(AsrError::Worker("MOSS 转写已取消".into())),
            }
            drop(slot);
            if let Some(d) = deadline {
                if Instant::now() >= d {
                    self.cancel();
                    return Err(AsrError::Worker(format!(
                        "MOSS 转写超时（{} 分钟），已终止子进程",
                        self.timeout_minutes
                    )));
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        let stdout = out_reader
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = err_reader
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        if !status.success() {
            return Err(AsrError::Worker(format!(
                "MOSS 转写失败（{}）：{}",
                status,
                stderr.trim()
            )));
        }
        parse_segments(&stdout)
    }

    /// 终止当前转写子进程（无活动进程时仅置取消标志，无副作用）。
    /// 供前端"取消 ASR"、超时与应用退出钩子调用。
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.active_child.lock() {
            if let Some(child) = slot.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
                *slot = None;
            }
        }
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

    fn transcribe(
        &self,
        audio_path: &Path,
        // MOSS 内置说话人分离与多语言识别，无语言/说话人上限参数，忽略 options
        _options: &AsrTranscribeOptions,
    ) -> Result<Vec<AsrSegment>, AsrError> {
        if !self.is_ready() {
            return Err(AsrError::NotReady);
        }
        // 每次转写重置取消标志：上一次取消不污染本次
        self.cancelled.store(false, Ordering::SeqCst);
        let result = self.run_transcribe(audio_path);
        match &result {
            Err(e) => self.set_error(&e.to_string()),
            // 成功时清除历史错误：避免取消/失败的残留一直显示在状态探测里
            Ok(_) => {
                if let Ok(mut slot) = self.last_error.lock() {
                    *slot = None;
                }
            }
        }
        result
    }

    fn cancel(&self) {
        MossProvider::cancel(self);
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

    #[test]
    fn test_parse_tasklist_pids_extracts_csv_rows() {
        // 多个残留实例：都应被识别（含内存列带千分位逗号的情况）
        let out = concat!(
            "\"moss-transcribe.exe\",\"12345\",\"Console\",\"1\",\"1,234 K\"\r\n",
            "\"moss-transcribe.exe\",\"67890\",\"Console\",\"1\",\"5,678 K\"\r\n"
        );
        assert_eq!(MossProvider::parse_tasklist_pids(out), vec![12345, 67890]);
    }

    #[test]
    fn test_parse_tasklist_pids_skips_empty_and_foreign() {
        // 无进程时 tasklist 输出本地化提示行（非 CSV），不应误解析出 PID
        let localized = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert!(MossProvider::parse_tasklist_pids(localized).is_empty());
        assert!(MossProvider::parse_tasklist_pids("").is_empty());
        // 其他映像的行不属于 moss，忽略
        let foreign = "\"chrome.exe\",\"999\",\"Console\",\"1\",\"9,999 K\"\r\n";
        assert!(MossProvider::parse_tasklist_pids(foreign).is_empty());
        // 坏 PID 行（理论不会出现）应跳过而非崩溃
        let malformed = "\"moss-transcribe.exe\",\"abc\",\"Console\",\"1\",\"1,234 K\"\r\n";
        assert!(MossProvider::parse_tasklist_pids(malformed).is_empty());
    }
}
