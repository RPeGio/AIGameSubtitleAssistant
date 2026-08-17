// ─── FunASR 转写 Worker Provider ───────────────────────────
// 一次性子进程：`python funasr_worker.py <audio.wav>`，stdout 输出 JSON 段数组。
// 复用 runtime/python 解释器，依赖装在独立目录 deps_funasr（避免与 paddleocr
// 的 deps 版本冲突），模型缓存 models/funasr（ModelScope 源，bootstrap 预下载）。
// 模型每次转写加载一次（~20-40s），相对 GPU 推理时间可忽略，故不做常驻进程。
//
// 进程生命周期与 MOSS 一致（复制 moss.rs 模式）：转写子进程登记在
// active_child，支持 cancel()（kill 子进程）、超时与应用退出钩子。
//
// 防残留叠加：每次转写前清理系统上残留的 funasr_worker 孤儿进程。
// 注意进程是 python.exe（与 OCR worker 同名），必须按命令行含
// funasr_worker.py 过滤，不能按映像名清理（否则会误杀 OCR worker）。

use crate::ai_runtime::{resolve_path, AsrError, AsrProvider, AsrSegment, RuntimeConfig};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// FunASR worker 输出的单段（JSON 数组元素）
#[derive(Deserialize)]
struct RawSegment {
    start: f64,
    end: f64,
    speaker: String,
    text: String,
}

/// 解析 worker 的 stdout：JSON 段数组 → AsrSegment 列表（时间戳为 VAD 段边界）。
/// FunASR 不输出置信度，confidence 置 None。
fn parse_segments(json: &str) -> Result<Vec<AsrSegment>, AsrError> {
    let raw: Vec<RawSegment> = serde_json::from_str(json)
        .map_err(|e| AsrError::Worker(format!("FunASR 输出解析失败: {}", e)))?;
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

/// FunASR 转写 provider —— 每次 transcribe 启动一次性 python 子进程
pub struct FunAsrProvider {
    python: PathBuf,
    worker: PathBuf,
    deps: PathBuf,
    model_dir: PathBuf,
    device: String,
    language: String,
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

impl FunAsrProvider {
    /// 构造 provider。就绪探测只做文件/依赖存在性检查（轻量、不启动 python：
    /// import torch/funasr 需要数秒，模型可加载性由转写时验证并记录错误）。
    pub fn spawn(config: &RuntimeConfig, runtime_dir: &Path) -> Result<Self, AsrError> {
        let python = resolve_path(&config.python_path, runtime_dir);
        let worker = resolve_path(&config.funasr_worker, runtime_dir);
        let deps = resolve_path(&config.funasr_deps, runtime_dir);
        let model_dir = resolve_path(&config.funasr_model_dir, runtime_dir);

        if !python.is_file() {
            return Err(AsrError::Worker(format!(
                "Python 解释器不存在: {}",
                python.display()
            )));
        }
        if !worker.is_file() {
            return Err(AsrError::Worker(format!(
                "FunASR worker 脚本不存在: {}（请先跑 scripts/bootstrap_funasr.ps1）",
                worker.display()
            )));
        }

        let provider = FunAsrProvider {
            python,
            worker,
            deps,
            model_dir,
            device: config.funasr_device.clone(),
            language: config.funasr_language.clone(),
            timeout_minutes: config.funasr_timeout_minutes,
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

    /// 轻量探测：deps 目录存在且含 funasr 包（pip --target 安装后的布局）
    fn probe(&self) -> Result<(), AsrError> {
        if !self.deps.is_dir() {
            return Err(AsrError::Worker(format!(
                "FunASR 依赖目录不存在: {}（请先跑 scripts/bootstrap_funasr.ps1）",
                self.deps.display()
            )));
        }
        let has_funasr = std::fs::read_dir(&self.deps)
            .map(|mut it| {
                it.any(|e| {
                    e.map(|e| e.file_name().to_string_lossy().starts_with("funasr"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if !has_funasr {
            return Err(AsrError::Worker(
                "FunASR 依赖未安装（deps 目录缺少 funasr 包，请先跑 scripts/bootstrap_funasr.ps1）"
                    .into(),
            ));
        }
        Ok(())
    }

    fn set_error(&self, msg: &str) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(msg.to_string());
        }
    }

    /// 解析 `wmic process where name='python.exe' get ... /format:csv` 输出，
    /// 提取命令行含 `marker` 的 python 进程 PID。
    /// CSV 行形如：`"HOST","python.exe","python funasr_worker.py C:\a.wav","12345"`
    /// （CommandLine 含逗号时 wmic 不转义，故只按 marker 过滤 + 取行尾 PID）
    fn parse_wmic_pids(output: &str, marker: &str) -> Vec<u32> {
        output
            .lines()
            .filter(|l| l.contains(marker) && l.contains("python"))
            .filter_map(|l| {
                l.rsplit(',')
                    .next()?
                    .trim_matches('"')
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
            .collect()
    }

    /// 终止系统上历史残留的 funasr_worker 孤儿进程（本次子进程尚未 spawn，
    /// 不会误杀自己）。python.exe 与 OCR worker 同名，故按命令行过滤。
    /// wmic 在 Win11 24H2+/Server 2025 已被移除，失败时回退 PowerShell
    /// Get-CimInstance（同一过滤语义）。
    #[cfg(windows)]
    fn cleanup_stale_funasr_processes(&self) {
        let wmic_out = Command::new("wmic")
            .args([
                "process",
                "where",
                "name='python.exe'",
                "get",
                "ProcessId,CommandLine",
                "/format:csv",
            ])
            .output();
        match wmic_out {
            Ok(o) if o.status.success() => {
                for pid in Self::parse_wmic_pids(&String::from_utf8_lossy(&o.stdout), "funasr_worker.py") {
                    Self::kill_pid(pid);
                }
            }
            _ => {
                // PowerShell 按命令行过滤（不含引号嵌套，Rust 侧 argv 直传无 shell 展开）
                let script = "Get-CimInstance Win32_Process -Filter \"Name='python.exe'\" | Where-Object { $_.CommandLine -like '*funasr_worker.py*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }";
                if let Err(e) = Command::new("powershell")
                    .args(["-NoProfile", "-Command", script])
                    .output()
                {
                    if self.dev_debug {
                        eprintln!("[funasr] 残留清理失败（wmic 与 powershell 均不可用）: {}", e);
                    }
                } else if self.dev_debug {
                    eprintln!("[funasr] 已用 PowerShell 清理残留转写进程");
                }
            }
        }
    }

    /// taskkill 单个进程（幂等：进程已不存在时忽略错误）
    #[cfg(windows)]
    fn kill_pid(pid: u32) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }

    /// 非 Windows 平台：无 wmic/taskkill，跳过清理（当前项目仅 Windows 部署）
    #[cfg(not(windows))]
    fn cleanup_stale_funasr_processes(&self) {}

    /// 执行一次转写（阻塞，含模型加载 20-40s；由编排层放后台线程）。
    /// 子进程登记在 active_child：轮询等待（可被 cancel 打断）+ 超时终止。
    fn run_transcribe(&self, audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        // 防残留叠加：清理历史孤儿进程（本次子进程尚未 spawn，不误杀自己）
        self.cleanup_stale_funasr_processes();
        let mut cmd = Command::new(&self.python);
        cmd.arg(&self.worker)
            .arg(audio_path)
            // PYTHONPATH 指向独立 deps 目录；MODELSCOPE_CACHE 指向模型缓存
            .env("PYTHONPATH", &self.deps)
            .env("MODELSCOPE_CACHE", &self.model_dir)
            .env("GSA_FUNASR_DEVICE", &self.device)
            .env("GSA_FUNASR_LANGUAGE", &self.language)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| AsrError::Worker(format!("无法启动 FunASR 转写: {}", e)))?;

        // 分离管道交给读线程：防止管道缓冲写满阻塞 python 进程
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
                return Err(AsrError::Worker("FunASR 转写已取消".into()));
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
                        return Err(AsrError::Worker(format!("FunASR 进程异常: {}", e)));
                    }
                },
                None => return Err(AsrError::Worker("FunASR 转写已取消".into())),
            }
            drop(slot);
            if let Some(d) = deadline {
                if Instant::now() >= d {
                    self.cancel();
                    return Err(AsrError::Worker(format!(
                        "FunASR 转写超时（{} 分钟），已终止子进程",
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
                "FunASR 转写失败（{}）：{}",
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

impl AsrProvider for FunAsrProvider {
    fn name(&self) -> &str {
        "funasr"
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
        // 每次转写重置取消标志：上一次取消不污染本次
        self.cancelled.store(false, Ordering::SeqCst);
        let result = self.run_transcribe(audio_path);
        if let Err(e) = &result {
            self.set_error(&e.to_string());
        }
        result
    }

    fn cancel(&self) {
        FunAsrProvider::cancel(self);
    }
}

// ─── 单元测试（纯协议逻辑，不依赖真实 python/模型）──────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_segments_ok() {
        let json = r#"[
            {"start":0.5,"end":2.3,"speaker":"SPK0","text":"你好，世界。"},
            {"start":2.8,"end":4.1,"speaker":"SPK1","text":"Hello!"}
        ]"#;
        let segs = parse_segments(json).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start, 0.5);
        assert_eq!(segs[0].end, 2.3);
        assert_eq!(segs[0].text, "你好，世界。");
        assert_eq!(segs[0].speaker.as_deref(), Some("SPK0"));
        assert_eq!(segs[0].confidence, None);
        assert_eq!(segs[1].speaker.as_deref(), Some("SPK1"));
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
        let json = r#"[{"start":0.1,"end":0.5,"speaker":"SPK0"}]"#;
        assert!(parse_segments(json).is_err());
    }

    #[test]
    fn test_parse_wmic_pids_filters_by_marker() {
        // 命中 funasr_worker 的行取行尾 PID；其他 python 进程（OCR worker）忽略
        let out = concat!(
            "Node,CommandLine,ProcessId\r\n",
            "\"HOST\",\"python.exe\",\"python C:\\\\runtime\\\\worker\\\\funasr_worker.py C:\\\\a.wav\",\"1111\"\r\n",
            "\"HOST\",\"python.exe\",\"python C:\\\\runtime\\\\worker\\\\ocr_worker.py\",\"2222\"\r\n",
            "\"HOST\",\"python.exe\",\"python funasr_worker.py\",\"3333\"\r\n"
        );
        assert_eq!(
            FunAsrProvider::parse_wmic_pids(out, "funasr_worker.py"),
            vec![1111, 3333]
        );
    }

    #[test]
    fn test_parse_wmic_pids_empty_and_malformed() {
        // 无进程/异常行：不应误解析出 PID
        assert!(FunAsrProvider::parse_wmic_pids("", "funasr_worker.py").is_empty());
        let header = "Node,CommandLine,ProcessId\r\n";
        assert!(FunAsrProvider::parse_wmic_pids(header, "funasr_worker.py").is_empty());
        // 坏 PID 行应跳过而非崩溃
        let bad = "\"HOST\",\"python.exe\",\"python funasr_worker.py\",\"abc\"\r\n";
        assert!(FunAsrProvider::parse_wmic_pids(bad, "funasr_worker.py").is_empty());
    }
}
