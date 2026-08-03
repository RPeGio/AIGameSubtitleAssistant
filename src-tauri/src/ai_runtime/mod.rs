// ─── AI 运行时模块 ───────────────────────────────────────
// 提供统一的 AI Provider 抽象，支持可插拔实现：
//   - OCR：PaddleOCR（Python Worker，2.2 接入）/ 未来 ONNX / 云端 API
//   - ASR：MOSS（Phase 3）
//   - 文本处理：Local LLM（Phase 5）
//
// 本阶段（2.1）只搭骨架：trait + 管理器 + 配置 + NoneProvider 占位。

pub mod config;

use serde::Serialize;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

pub use config::RuntimeConfig;

// ─── OCR 数据与错误 ──────────────────────────────────────

/// 单张帧图像的 OCR 识别结果
#[derive(Debug, Clone, Serialize)]
pub struct OcrResult {
    pub text: String,
    /// 识别置信度 (0.0 ~ 1.0)
    pub confidence: f64,
}

/// OCR 错误类型
#[derive(Debug)]
pub enum OcrError {
    /// 运行环境未就绪（python/模型缺失）
    NotReady,
    /// 环境层面问题：python 启动失败、依赖缺失等
    Runtime(String),
    /// worker 进程返回的错误
    Worker(String),
    /// IO 错误
    Io(String),
}

impl fmt::Display for OcrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OcrError::NotReady => write!(f, "OCR 运行环境未就绪"),
            OcrError::Runtime(msg) => write!(f, "OCR 运行环境错误: {}", msg),
            OcrError::Worker(msg) => write!(f, "OCR worker 错误: {}", msg),
            OcrError::Io(msg) => write!(f, "OCR IO 错误: {}", msg),
        }
    }
}

impl std::error::Error for OcrError {}

// ─── OCR Provider 抽象 ───────────────────────────────────

/// OCR 提供者。批量识别为第一接口（与 worker 的 JSON 协议一一对应）。
/// 只要求 `Send`，管理器外套 Mutex 提供 `Sync`。
pub trait OcrProvider: Send {
    /// provider 名称（"none" / "paddle" 等）
    fn name(&self) -> &str;
    /// 运行环境是否就绪
    fn is_ready(&self) -> bool;
    /// 批量识别帧图像（文件路径），按输入顺序返回结果
    fn recognize_batch(&self, image_paths: &[String]) -> Result<Vec<OcrResult>, OcrError>;
}

/// 占位 provider —— 环境未就绪时使用，保证管线可编译
pub struct NoneProvider;

impl OcrProvider for NoneProvider {
    fn name(&self) -> &str {
        "none"
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn recognize_batch(&self, _image_paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
        Err(OcrError::NotReady)
    }
}

// ─── Provider 工厂 ───────────────────────────────────────

/// 支持的 OCR provider 类型（可扩展）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrProviderKind {
    None,
    /// PaddleOCR Python Worker（2.2 实现）
    Paddle,
}

/// 根据类型与配置创建 provider 实例
pub fn create_ocr_provider(kind: OcrProviderKind, _config: &RuntimeConfig) -> Box<dyn OcrProvider> {
    match kind {
        OcrProviderKind::None => Box::new(NoneProvider),
        // TODO(2.2): 实现 PaddleProvider（Python Worker 封装）
        OcrProviderKind::Paddle => Box::new(NoneProvider),
    }
}

// ─── 管理器（Tauri 托管状态）──────────────────────────────

/// 给前端展示的运行时状态
#[derive(Clone, Serialize)]
pub struct OcrRuntimeStatus {
    pub provider: String,
    pub ready: bool,
    pub message: String,
}

/// 全局 OCR 管理器 —— 持有配置与当前 provider
pub struct OcrManager {
    config: RuntimeConfig,
    provider: Mutex<Box<dyn OcrProvider>>,
}

impl OcrManager {
    pub fn new(config: RuntimeConfig) -> Self {
        // 2.1 阶段固定使用 NoneProvider；2.2 起按 config 决定
        let provider = create_ocr_provider(OcrProviderKind::None, &config);
        Self {
            config,
            provider: Mutex::new(provider),
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn provider(&self) -> MutexGuard<'_, Box<dyn OcrProvider>> {
        self.provider
            .lock()
            .expect("OCR provider 锁被污染")
    }

    /// 汇总当前运行时状态（供 check_ocr_runtime 命令）
    pub fn status(&self) -> OcrRuntimeStatus {
        let provider = self.provider();
        let ready = provider.is_ready();
        let message = if ready {
            format!("OCR 运行时就绪（provider: {}）", provider.name())
        } else {
            "OCR 运行环境未就绪：请先运行环境引导脚本".into()
        };
        OcrRuntimeStatus {
            provider: provider.name().into(),
            ready,
            message,
        }
    }
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_none_provider_not_ready() {
        let p = NoneProvider;
        assert_eq!(p.name(), "none");
        assert!(!p.is_ready());
        let result = p.recognize_batch(&["a.jpg".into()]);
        assert!(matches!(result, Err(OcrError::NotReady)));
    }

    #[test]
    fn test_manager_status_not_ready() {
        let manager = OcrManager::new(RuntimeConfig::default());
        let status = manager.status();
        assert_eq!(status.provider, "none");
        assert!(!status.ready);
    }
}

// ─── Tauri 命令 ───────────────────────────────────────────

/// Tauri 命令：探测 OCR 运行环境是否就绪
#[tauri::command]
pub fn check_ocr_runtime(state: tauri::State<'_, OcrManager>) -> OcrRuntimeStatus {
    state.status()
}
