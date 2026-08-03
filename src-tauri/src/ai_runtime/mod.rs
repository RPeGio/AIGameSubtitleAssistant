// ─── AI 运行时模块 ───────────────────────────────────────
// 提供统一的 AI Provider 抽象，支持可插拔实现：
//   - OCR：PaddleOCR（Python Worker，2.2 接入）/ 未来 ONNX / 云端 API
//   - ASR：MOSS（Phase 3）
//   - 文本处理：Local LLM（Phase 5）
//
// 本阶段（2.1）只搭骨架：trait + 管理器 + 配置 + 占位 provider。

pub mod config;

use serde::Serialize;
use std::error::Error;
use std::fmt;
use std::sync::Mutex;

pub use config::RuntimeConfig;

// ─── OCR 数据与错误 ──────────────────────────────────────

/// 单张帧图像的 OCR 识别结果
#[derive(Debug, Clone, Serialize)]
pub struct OcrResult {
    pub text: String,
    /// 识别置信度 (0.0 ~ 1.0)
    pub confidence: f64,
}

/// OCR 错误类型 —— 尽量保留底层错误链
#[derive(Debug)]
pub enum OcrError {
    /// 运行环境未就绪（python/模型缺失）
    NotReady,
    /// 已配置但尚未实现的 provider
    Unimplemented(String),
    /// 环境层面错误：python 启动失败、依赖缺失等
    Runtime(Box<dyn Error + Send + Sync>),
    /// worker 进程返回的错误
    Worker(String),
    /// IO 错误
    Io(std::io::Error),
}

impl OcrError {
    pub fn runtime<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        OcrError::Runtime(Box::new(err))
    }
}

impl From<std::io::Error> for OcrError {
    fn from(e: std::io::Error) -> Self {
        OcrError::Io(e)
    }
}

impl fmt::Display for OcrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OcrError::NotReady => write!(f, "OCR 运行环境未就绪"),
            OcrError::Unimplemented(name) => write!(f, "OCR provider '{}' 尚未实现", name),
            OcrError::Runtime(err) => write!(f, "OCR 运行环境错误: {}", err),
            OcrError::Worker(msg) => write!(f, "OCR worker 错误: {}", msg),
            OcrError::Io(err) => write!(f, "OCR IO 错误: {}", err),
        }
    }
}

impl std::error::Error for OcrError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            OcrError::Runtime(err) => Some(err.as_ref()),
            OcrError::Io(err) => Some(err),
            _ => None,
        }
    }
}

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

/// "已配置但未实现" 的 provider —— 保留配置的 name，便于与"未初始化"区分
pub struct PendingProvider {
    name: String,
}

impl OcrProvider for PendingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn recognize_batch(&self, _image_paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
        Err(OcrError::Unimplemented(self.name.clone()))
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
        OcrProviderKind::Paddle => Box::new(PendingProvider { name: "paddle".into() }),
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

    /// 作用域化访问 provider：内部加锁，避免把 MutexGuard 泄漏到调用方。
    /// 锁被污染时退化为使用被污染的 guard 内的值，保证调用不中断。
    pub fn with_provider<R>(&self, f: impl FnOnce(&dyn OcrProvider) -> R) -> R {
        let guard = self
            .provider
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&**guard)
    }

    /// 汇总当前运行时状态（供 check_ocr_runtime 命令）
    pub fn status(&self) -> OcrRuntimeStatus {
        let name = self.with_provider(|p| p.name().to_string());
        let ready = self.with_provider(|p| p.is_ready());
        let message = if ready {
            format!("OCR 运行时就绪（provider: {}）", name)
        } else if name == "none" {
            "未配置 OCR 运行环境".into()
        } else {
            format!("OCR 运行环境未就绪（provider: {}）", name)
        };
        OcrRuntimeStatus {
            provider: name,
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
    fn test_pending_provider_reports_configured_name() {
        let p = PendingProvider { name: "paddle".into() };
        assert_eq!(p.name(), "paddle");
        assert!(!p.is_ready());
        let result = p.recognize_batch(&["a.jpg".into()]);
        assert!(matches!(result, Err(OcrError::Unimplemented(_))));
    }

    #[test]
    fn test_manager_status_not_ready() {
        let manager = OcrManager::new(RuntimeConfig::default());
        let status = manager.status();
        assert_eq!(status.provider, "none");
        assert!(!status.ready);
    }

    #[test]
    fn test_manager_survives_poisoned_lock() {
        // 构造一个已污染的 Mutex：持有 guard 时 panic
        let mutex = Mutex::new(Box::new(NoneProvider) as Box<dyn OcrProvider>);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            std::panic::panic_any("poison");
        }));
        let mgr = OcrManager {
            config: RuntimeConfig::default(),
            provider: mutex,
        };
        // with_provider 应退化为使用被污染 guard 内的值，不 panic
        let status = mgr.status();
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
