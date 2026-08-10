// ─── LLM 推理 CLI Provider ───────────────────────────────
// 一次性子进程：`llama-cli -m <model> -p <prompt> -st -n 256 --no-display-prompt`。
// stdout 输出回答文本，stderr 是日志（错误时取尾部报错）。
// 0.5B 模型加载极快（~0.3s），相对生成耗时可忽略，故不做常驻进程（3B 后再评估）。

use crate::ai_runtime::{resolve_path, LlmError, LlmProvider, RuntimeConfig};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

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
    fn probe(&self) -> Result<(), LlmError> {
        let output = Command::new(&self.binary)
            .arg("-m")
            .arg(&self.model)
            .arg("-p")
            .arg("hi")
            .arg("-st")
            .arg("-n")
            .arg("1")
            .output()
            .map_err(|e| LlmError::Worker(format!("无法启动 llama-cli: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LlmError::Worker(format!(
                "llama-cli 探测推理失败（{}）：{}",
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

    /// 执行一次单轮推理（阻塞；由编排层放后台线程）。
    /// 温度固定 0.2：纠错/去重/格式化等确定性任务，低温减少幻觉。
    fn run_complete(&self, prompt: &str) -> Result<String, LlmError> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-p")
            .arg(prompt)
            // 单轮：回答后立即退出，不进入交互循环
            .arg("-st")
            // 生成上限，防止异常情况无限生成
            .arg("-n")
            .arg("256")
            // stdout 只留回答文本，prompt 不回显
            .arg("--no-display-prompt")
            .arg("--temp")
            .arg("0.2");
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
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

    fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        if !self.is_ready() {
            return Err(LlmError::NotReady);
        }
        let result = self.run_complete(prompt);
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
        assert!(matches!(provider.complete("hi"), Err(LlmError::NotReady)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
