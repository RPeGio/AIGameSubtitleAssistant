// ─── LLM 推理 CLI Provider ───────────────────────────────
// 一次性子进程：`llama-cli -m <model> -p <prompt> -st -n 256 --no-display-prompt`。
// stdout 输出回答文本，stderr 是日志（错误时取尾部报错）。
// 0.5B 模型加载极快（~0.3s），相对生成耗时可忽略，故不做常驻进程（3B 后再评估）。

use crate::ai_runtime::{resolve_path, LlmError, LlmProvider, RuntimeConfig};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// llama-cli Provider —— 每次 complete 启动一次性子进程
pub struct LlamaProvider {
    binary: PathBuf,
    model: PathBuf,
    /// 推理线程数：0 = 不设置（llama-cli 默认全核）
    threads: u32,
    ready: AtomicBool,
    /// 最近一次错误（供 describe() 向状态探测展示）
    last_error: Mutex<Option<String>>,
}

impl LlamaProvider {
    /// 构造 provider 并用最小推理（`-st -n 1`）探测模型可加载。
    /// 探测失败不视为致命，仅 ready=false。
    pub fn spawn(config: &RuntimeConfig, runtime_dir: &Path) -> Result<Self, LlmError> {
        let binary = resolve_path(&config.llm_binary, runtime_dir);
        let model = resolve_path(&config.llm_model, runtime_dir);

        if !binary.is_file() {
            return Err(LlmError::Worker(format!(
                "LLM 可执行文件不存在: {}",
                binary.display()
            )));
        }
        if !model.is_file() {
            return Err(LlmError::Worker(format!(
                "LLM 模型不存在: {}",
                model.display()
            )));
        }

        let provider = LlamaProvider {
            binary,
            model,
            threads: config.llm_threads,
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

    /// 探测：`-st -n 1` 最小推理（加载模型 + 生成 1 token）。
    /// 成功说明模型可加载且推理链路通；比仅验元数据更真实（能暴露 CUDA 初始化等问题）。
    /// 60s 超时保护：模型加载/GPU 初始化卡住时不能拖死应用启动。
    fn probe(&self) -> Result<(), LlmError> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-p")
            .arg("hi")
            .arg("-st")
            .arg("-n")
            .arg("1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if self.threads > 0 {
            cmd.arg("-t").arg(self.threads.to_string());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| LlmError::Worker(format!("无法启动 llama-cli: {}", e)))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(LlmError::Worker(format!(
                        "llama-cli 探测推理失败（{}）",
                        status
                    )));
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(LlmError::Worker("llama-cli 探测超时（60s）".into()));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    return Err(LlmError::Worker(format!("llama-cli 探测进程异常: {}", e)));
                }
            }
        }
    }

    fn set_error(&self, msg: &str) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(msg.to_string());
        }
    }

    /// 执行一次单轮推理（阻塞；由编排层放后台线程）。
    /// 温度固定 0.2：纠错/去重/格式化等确定性任务，低温减少幻觉。
    fn run_complete(&self, prompt: &str, max_tokens: u32) -> Result<String, LlmError> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-p")
            .arg(prompt)
            // 单轮：回答后立即退出，不进入交互循环
            .arg("-st")
            // 生成上限，防止异常情况无限生成（融合批输出 JSON 需要更大上限）
            .arg("-n")
            .arg(max_tokens.to_string())
            // stdout 只留回答文本，prompt 不回显
            .arg("--no-display-prompt")
            .arg("--temp")
            .arg("0.2")
            // 强制 ANSI 色块：见 parse_answer（b10333 把 UI 全输出到 stdout，靠色块定位回答）
            .arg("--color")
            .arg("on");
        if self.threads > 0 {
            cmd.arg("-t").arg(self.threads.to_string());
        }
        let output = cmd
            .output()
            .map_err(|e| LlmError::Worker(format!("无法启动 llama-cli: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LlmError::Worker(format!(
                "llama-cli 推理失败（{}）：{}",
                output.status,
                stderr.trim()
            )));
        }
        Ok(parse_answer(&String::from_utf8_lossy(&output.stdout)))
    }
}

/// 从 llama-cli stdout 提取回答文本。
///
/// b10333 的 `-st` 模式把 banner、模型信息、提示符、性能统计全部输出到 stdout
/// （stderr 反而为空），且无法用参数关闭。强制 `--color on` 后输出有稳定结构：
/// prompt 回显夹在绿色块（`\x1b[32m` … `\x1b[0m`）内，回答为无色文本，
/// 性能统计以品红块（`\x1b[35m`）开始：
///   ... `\x1b[1m\x1b[32m` `> prompt...` `\x1b[0m` <回答> `\x1b[35m` [ Prompt: ... ] `\x1b[0m` Exiting...
/// 故取第一个 `\x1b[0m` 之后、第一个 `\x1b[35m` 之前的内容。
/// 色块缺失（版本/参数变化）时回退返回整段 trim，靠 e2e 断言防污染回归。
fn parse_answer(stdout: &str) -> String {
    let reset = "\x1b[0m";
    let magenta = "\x1b[35m";
    match stdout.find(reset) {
        Some(start) => {
            let tail = &stdout[start + reset.len()..];
            let end = tail.find(magenta).unwrap_or(tail.len());
            tail[..end].trim().to_string()
        }
        None => stdout.trim().to_string(),
    }
}

impl LlmProvider for LlamaProvider {
    fn name(&self) -> &str {
        "llama"
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        self.last_error
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default()
    }

    fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String, LlmError> {
        if !self.is_ready() {
            return Err(LlmError::NotReady);
        }
        let result = self.run_complete(prompt, max_tokens);
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
    fn test_parse_answer_multiline_prompt() {
        // b10333 实测输出结构：绿色块回显 prompt → \x1b[0m 回答 → \x1b[35m 性能统计
        let stdout = format!(
            "Loading model... \nbanner\n\x1b[1m\x1b[32m\n> line one\nline two\n2+2=?\n\x1b[0m2+2=4\n\x1b[35m\n[ Prompt: 2251.0 t/s | Generation: 305.8 t/s ]\n\x1b[0m\n\nExiting...\n"
        );
        assert_eq!(parse_answer(&stdout), "2+2=4");
    }

    #[test]
    fn test_parse_answer_single_line_prompt() {
        let stdout = format!(
            "banner\n\x1b[1m\x1b[32m\n> 2+2=?\n\x1b[0m2+2 equals 4.\n\x1b[35m\n[ Prompt: 1 t/s ]\n\x1b[0m\nExiting...\n"
        );
        assert_eq!(parse_answer(&stdout), "2+2 equals 4.");
    }

    #[test]
    fn test_parse_answer_multiline_response() {
        // 回答多行：\x1b[0m 后的全部行直到品红块
        let stdout = format!(
            "\x1b[1m\x1b[32m\n> hi\n\x1b[0mline one\nline two\n\x1b[35m\n[ Prompt: 1 t/s ]\n\x1b[0m\n"
        );
        assert_eq!(parse_answer(&stdout), "line one\nline two");
    }

    #[test]
    fn test_parse_answer_no_color_blocks_fallback() {
        // 色块缺失（版本变化）→ 回退整段 trim
        assert_eq!(parse_answer("just an answer\n"), "just an answer");
    }

    #[test]
    fn test_parse_answer_empty() {
        assert_eq!(parse_answer(""), "");
    }

    #[test]
    fn test_spawn_missing_binary_errors() {
        let mut config = RuntimeConfig::default();
        config.llm_binary = "bin/llama-cli.exe".into();
        config.llm_model = "models/qwen/model.gguf".into();
        let result = LlamaProvider::spawn(&config, Path::new("no_such_runtime_xyz"));
        assert!(matches!(result, Err(LlmError::Worker(_))));
    }

    #[test]
    fn test_spawn_missing_model_errors() {
        let dir = std::env::temp_dir().join(format!("gsa_llm_missing_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let binary = dir.join("llama-cli.exe");
        std::fs::write(&binary, b"dummy").unwrap();
        let mut config = RuntimeConfig::default();
        config.llm_binary = binary.to_string_lossy().into_owned();
        config.llm_model = "models/qwen/missing.gguf".into();
        let result = LlamaProvider::spawn(&config, &dir);
        assert!(matches!(result, Err(LlmError::Worker(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_spawn_dummy_files_broken_but_ok() {
        // 文件存在但不可执行 → spawn 返回 Ok(provider)，ready=false 且 describe 有原因
        let dir = std::env::temp_dir().join(format!("gsa_llm_dummy_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let binary = dir.join("llama-cli.exe");
        let model = dir.join("model.gguf");
        std::fs::write(&binary, b"dummy").unwrap();
        std::fs::write(&model, b"dummy").unwrap();
        let mut config = RuntimeConfig::default();
        config.llm_binary = binary.to_string_lossy().into_owned();
        config.llm_model = model.to_string_lossy().into_owned();
        let provider = LlamaProvider::spawn(&config, &dir).unwrap();
        assert!(!provider.is_ready());
        assert!(!provider.describe().is_empty());
        assert!(matches!(
            provider.complete("hi", 256),
            Err(LlmError::NotReady)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
