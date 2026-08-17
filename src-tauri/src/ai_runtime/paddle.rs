// ─── PaddleOCR Python Worker Provider ─────────────────────
// 常驻子进程：内嵌 Python（runtime/python）+ runtime/worker/ocr_worker.py，JSON lines over stdio。
// 模型只加载一次；`ping` 用于启动探测（也顺带触发首次模型下载）。

use crate::ai_runtime::{OcrError, OcrProvider, OcrResult, RuntimeConfig};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// 与 worker 的 stdio 通道
struct WorkerIo {
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

/// PaddleOCR provider —— 内部用 Mutex 串行化子进程通信
pub struct PaddleProvider {
    child: Mutex<Child>,
    io: Mutex<WorkerIo>,
    ready: AtomicBool,
    /// 最近一次错误（供 describe() 向状态探测展示）
    last_error: Mutex<Option<String>>,
}

impl Drop for PaddleProvider {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ─── 协议纯函数（可单测）──────────────────────────────────

#[derive(Serialize)]
struct Request<'a> {
    id: u32,
    images: &'a [String],
}

/// 构造批量识别请求行
fn build_request(id: u32, images: &[String]) -> String {
    serde_json::to_string(&Request { id, images }).unwrap_or_default()
}

#[derive(Deserialize)]
struct WorkerResponse {
    // id 用于协议关联，单请求串行下暂不校验
    #[allow(dead_code)]
    id: Option<u32>,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    results: Option<Vec<OcrResult>>,
}

/// 解析 worker 响应行；ok=false 时返回错误
fn parse_response(line: &str) -> Result<Vec<OcrResult>, OcrError> {
    let resp: WorkerResponse = serde_json::from_str(line)
        .map_err(|e| OcrError::Worker(format!("响应解析失败: {}", e)))?;
    if !resp.ok {
        return Err(OcrError::Worker(
            resp.error.unwrap_or_else(|| "未知 worker 错误".into()),
        ));
    }
    Ok(resp.results.unwrap_or_default())
}

// ─── Provider 实现 ────────────────────────────────────────

impl PaddleProvider {
    /// 启动 worker 子进程并探测就绪。探测失败不视为致命，仅 ready=false。
    pub fn spawn(config: &RuntimeConfig, runtime_dir: &Path) -> Result<Self, OcrError> {
        let worker_script = runtime_dir.join(&config.worker_script);
        let deps_dir = runtime_dir.join(&config.deps_dir);
        let model_dir = runtime_dir.join(&config.model_dir);

        // python_path 支持相对 runtime 的写法（如 "python/python.exe"，机器无关）；
        // 绝对路径（老配置）原样使用
        let python = if Path::new(&config.python_path).is_absolute() {
            PathBuf::from(&config.python_path)
        } else {
            runtime_dir.join(&config.python_path)
        };

        let mut cmd = Command::new(&python);
        cmd.arg(&worker_script)
            .env("PYTHONPATH", &deps_dir)
            // paddleocr 3.x 基于 paddlex，模型缓存目录走 PADDLE_PDX_CACHE_HOME
            .env("PADDLE_PDX_CACHE_HOME", &model_dir)
            // 模型档位：mobile（默认/快）| server（慢/准）
            .env("GSA_OCR_MODEL", &config.ocr_model)
            // 跳过启动时的模型源连通性检查，加速就绪
            .env("PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK", "True")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            OcrError::Worker(format!(
                "无法启动 worker（python: {}，脚本: {}）：{}",
                python.display(),
                worker_script.display(),
                e
            ))
        })?;

        // 排空 stderr，避免管道填满阻塞子进程；顺带把日志打到 eprintln 便于调试
        let stderr = child.stderr.take().ok_or_else(|| {
            OcrError::Worker("无法获取 worker stderr".into())
        })?;
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(l) = line {
                    eprintln!("[ocr_worker] {}", l);
                }
            }
        });

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| OcrError::Worker("无法获取 worker stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OcrError::Worker("无法获取 worker stdout".into()))?;

        let provider = PaddleProvider {
            child: Mutex::new(child),
            io: Mutex::new(WorkerIo {
                stdin: BufWriter::new(stdin),
                stdout: BufReader::new(stdout),
            }),
            ready: AtomicBool::new(false),
            last_error: Mutex::new(None),
        };

        // 启动探测：ping 成功说明 worker 已加载 PaddleOCR（含模型）
        if let Err(e) = provider.ping() {
            provider.set_error(&e.to_string());
        } else {
            provider.ready.store(true, Ordering::SeqCst);
        }
        Ok(provider)
    }

    fn set_error(&self, msg: &str) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(msg.to_string());
        }
    }

    fn ping(&self) -> Result<(), OcrError> {
        let mut io = self
            .io
            .lock()
            .map_err(|_| OcrError::Worker("worker io 锁被污染".into()))?;
        io.stdin.write_all(b"{\"id\":0,\"cmd\":\"ping\"}\n")?;
        io.stdin.flush()?;

        let mut line = String::new();
        let n = io.stdout.read_line(&mut line)?;
        if n == 0 {
            return Err(OcrError::Worker("worker 已退出（ping 无响应）".into()));
        }

        let v: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| OcrError::Worker(format!("ping 响应解析失败: {}", e)))?;
        if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            let err = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("ping 返回失败");
            return Err(OcrError::Worker(err.to_string()));
        }
        Ok(())
    }
}

impl OcrProvider for PaddleProvider {
    fn name(&self) -> &str {
        "paddle"
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        self.last_error
            .lock()
            .map(|e| e.clone().unwrap_or_default())
            .unwrap_or_default()
    }

    fn recognize_batch(&self, image_paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
        if !self.is_ready() {
            return Err(OcrError::NotReady);
        }
        let mut io = self
            .io
            .lock()
            .map_err(|_| OcrError::Worker("worker io 锁被污染".into()))?;

        let req = build_request(1, image_paths);
        io.stdin.write_all(req.as_bytes())?;
        io.stdin.write_all(b"\n")?;
        io.stdin.flush()?;

        let mut line = String::new();
        let n = io.stdout.read_line(&mut line)?;
        if n == 0 {
            self.ready.store(false, Ordering::SeqCst);
            self.set_error("worker 进程已退出");
            return Err(OcrError::Worker("worker 进程已退出".into()));
        }

        match parse_response(&line) {
            Ok(results) => Ok(results),
            Err(e) => {
                self.set_error(&e.to_string());
                Err(e)
            }
        }
    }
}

// ─── 单元测试（纯协议逻辑，不依赖 Python）──────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_request_json() {
        let req = build_request(1, &["a.jpg".into(), "b.jpg".into()]);
        let v: serde_json::Value = serde_json::from_str(&req).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["images"][0], "a.jpg");
        assert_eq!(v["images"][1], "b.jpg");
    }

    #[test]
    fn test_parse_response_ok() {
        let line = r#"{"id":1,"ok":true,"results":[{"text":"旅行者，你来了","confidence":0.98},{"text":"","confidence":0.0}]}"#;
        let results = parse_response(line).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].text, "旅行者，你来了");
        assert!((results[0].confidence - 0.98).abs() < 1e-9);
        assert_eq!(results[1].text, "");
    }

    #[test]
    fn test_parse_response_error() {
        let line = r#"{"id":1,"ok":false,"error":"文件不存在"}"#;
        let err = parse_response(line).unwrap_err();
        assert!(matches!(err, OcrError::Worker(_)));
        assert!(err.to_string().contains("文件不存在"));
    }

    #[test]
    fn test_parse_response_malformed() {
        let err = parse_response("not json").unwrap_err();
        assert!(matches!(err, OcrError::Worker(_)));
    }

    #[test]
    fn test_runtime_dir_paths_join() {
        let runtime = PathBuf::from("C:/x/runtime");
        let cfg = RuntimeConfig {
            python_path: "python".into(),
            worker_script: "worker/ocr_worker.py".into(),
            deps_dir: "deps".into(),
            model_dir: "models/paddleocr".into(),
            language: "ch".into(),
            ocr_model: "mobile".into(),
            dev_debug: true,
            moss_binary: String::new(),
            moss_model: String::new(),
            moss_threads: 0,
            moss_timeout_minutes: 0,
            llm_binary: String::new(),
            llm_model: String::new(),
            llm_threads: 0,
        };
        assert_eq!(
            runtime.join(&cfg.worker_script),
            PathBuf::from("C:/x/runtime/worker/ocr_worker.py")
        );
        assert_eq!(runtime.join(&cfg.deps_dir), PathBuf::from("C:/x/runtime/deps"));
    }
}
