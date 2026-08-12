// ─── AI 运行时模块 ───────────────────────────────────────
// 提供统一的 AI Provider 抽象，支持可插拔实现：
//   - OCR：PaddleOCR（Python Worker，2.2 接入）/ 未来 ONNX / 云端 API
//   - ASR：MOSS（Phase 3）
//   - 文本处理：Local LLM（Phase 5）
//
// 本阶段（2.1）只搭骨架：trait + 管理器 + 配置 + 占位 provider。

pub mod config;
pub mod dhash;
pub mod llm;
pub mod moss;
pub mod paddle;

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

pub use config::RuntimeConfig;

// ─── 通用工具 ─────────────────────────────────────────────

/// 相对 runtime 的路径（机器无关）join runtime 目录；绝对路径（老配置）原样使用
pub(crate) fn resolve_path(p: &str, runtime_dir: &Path) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        runtime_dir.join(p)
    }
}

// ─── OCR 数据与错误 ──────────────────────────────────────

/// 单张帧图像的 OCR 识别结果
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// 附加说明（如未就绪的原因），默认空
    fn describe(&self) -> String {
        String::new()
    }
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

/// 创建失败的 provider —— 携带失败原因，供状态探测展示
pub struct BrokenProvider {
    name: String,
    message: String,
}

impl OcrProvider for BrokenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        self.message.clone()
    }

    fn recognize_batch(&self, _image_paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
        Err(OcrError::Runtime(Box::new(std::io::Error::other(
            self.message.clone(),
        ))))
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
/// Paddle 创建失败时返回 BrokenProvider（携带原因），不 panic
pub fn create_ocr_provider(
    kind: OcrProviderKind,
    config: &RuntimeConfig,
    runtime_dir: &PathBuf,
) -> Box<dyn OcrProvider> {
    match kind {
        OcrProviderKind::None => Box::new(NoneProvider),
        OcrProviderKind::Paddle => match paddle::PaddleProvider::spawn(config, runtime_dir) {
            Ok(p) => Box::new(p),
            Err(e) => Box::new(BrokenProvider {
                name: "paddle".into(),
                message: e.to_string(),
            }),
        },
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
    runtime_dir: PathBuf,
    provider: Mutex<Box<dyn OcrProvider>>,
}

impl OcrManager {
    pub fn new(config: RuntimeConfig, runtime_dir: PathBuf) -> Self {
        // 已配置 worker 脚本则走 Paddle；否则用 None 占位
        let kind = if config.worker_script.is_empty() {
            OcrProviderKind::None
        } else {
            OcrProviderKind::Paddle
        };
        let provider = create_ocr_provider(kind, &config, &runtime_dir);
        Self {
            config,
            runtime_dir,
            provider: Mutex::new(provider),
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn runtime_dir(&self) -> &PathBuf {
        &self.runtime_dir
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
        let detail = self.with_provider(|p| p.describe());
        let message = if ready {
            format!("OCR 运行时就绪（provider: {}）", name)
        } else if name == "none" {
            "未配置 OCR 运行环境".into()
        } else if detail.is_empty() {
            format!("OCR 运行环境未就绪（provider: {}）", name)
        } else {
            format!("OCR 运行环境未就绪（provider: {}）：{}", name, detail)
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
    fn test_manager_status_not_ready() {
        let manager = OcrManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        let status = manager.status();
        assert_eq!(status.provider, "none");
        assert!(!status.ready);
    }

    #[test]
    fn test_broken_provider_reports_reason() {
        let p = BrokenProvider {
            name: "paddle".into(),
            message: "python 不存在".into(),
        };
        assert_eq!(p.name(), "paddle");
        assert!(!p.is_ready());
        assert_eq!(p.describe(), "python 不存在");
        assert!(p.recognize_batch(&[]).is_err());
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
            runtime_dir: PathBuf::from("runtime"),
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

// ─── ASR 数据与错误 ──────────────────────────────────────

/// ASR 语音识别的一段结果（provider 原始输出，不含 id/character）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// 说话人标签（如 "S01"）；provider 无说话人数据时为 None
    pub speaker: Option<String>,
    /// 识别置信度 (0.0 ~ 1.0)；provider 无置信度数据时为 None
    pub confidence: Option<f64>,
}

/// ASR 错误类型 —— 尽量保留底层错误链
#[derive(Debug)]
pub enum AsrError {
    /// 运行环境未就绪（可执行文件/模型缺失）
    NotReady,
    /// 环境层面错误：进程启动失败、依赖缺失等
    Runtime(Box<dyn Error + Send + Sync>),
    /// 转写进程返回的错误
    Worker(String),
    /// IO 错误
    Io(std::io::Error),
}

impl AsrError {
    pub fn runtime<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        AsrError::Runtime(Box::new(err))
    }
}

impl From<std::io::Error> for AsrError {
    fn from(e: std::io::Error) -> Self {
        AsrError::Io(e)
    }
}

impl fmt::Display for AsrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsrError::NotReady => write!(f, "ASR 运行环境未就绪"),
            AsrError::Runtime(err) => write!(f, "ASR 运行环境错误: {}", err),
            AsrError::Worker(msg) => write!(f, "ASR worker 错误: {}", msg),
            AsrError::Io(err) => write!(f, "ASR IO 错误: {}", err),
        }
    }
}

impl std::error::Error for AsrError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            AsrError::Runtime(err) => Some(err.as_ref()),
            AsrError::Io(err) => Some(err),
            _ => None,
        }
    }
}

// ─── ASR Provider 抽象 ───────────────────────────────────

/// ASR 提供者。整段音频一次转写为第一接口（与 MOSS CLI 的 JSON 输出对应）。
/// 只要求 `Send`，管理器外套 Mutex 提供 `Sync`。
pub trait AsrProvider: Send {
    /// provider 名称（"none" / "moss" 等）
    fn name(&self) -> &str;
    /// 运行环境是否就绪
    fn is_ready(&self) -> bool;
    /// 附加说明（如未就绪的原因），默认空
    fn describe(&self) -> String {
        String::new()
    }
    /// 转写整段音频（WAV 文件路径），返回带说话人标签的时间轴段
    fn transcribe(&self, audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError>;
}

/// 占位 provider —— 环境未就绪时使用，保证管线可编译
pub struct AsrNoneProvider;

impl AsrProvider for AsrNoneProvider {
    fn name(&self) -> &str {
        "none"
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn transcribe(&self, _audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        Err(AsrError::NotReady)
    }
}

/// 创建失败的 provider —— 携带失败原因，供状态探测展示
pub struct AsrBrokenProvider {
    name: String,
    message: String,
}

impl AsrProvider for AsrBrokenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        self.message.clone()
    }

    fn transcribe(&self, _audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        Err(AsrError::Runtime(Box::new(std::io::Error::other(
            self.message.clone(),
        ))))
    }
}

// ─── ASR Provider 工厂 ───────────────────────────────────

/// 支持的 ASR provider 类型（可扩展）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsrProviderKind {
    None,
    /// MOSS 转写 CLI（Phase 3 接入）
    Moss,
}

/// 根据类型与配置创建 provider 实例
/// Moss 创建失败时返回 BrokenProvider（携带原因），不 panic
pub fn create_asr_provider(
    kind: AsrProviderKind,
    config: &RuntimeConfig,
    runtime_dir: &PathBuf,
) -> Box<dyn AsrProvider> {
    match kind {
        AsrProviderKind::None => Box::new(AsrNoneProvider),
        AsrProviderKind::Moss => match moss::MossProvider::spawn(config, runtime_dir) {
            Ok(p) => Box::new(p),
            Err(e) => Box::new(AsrBrokenProvider {
                name: "moss".into(),
                message: e.to_string(),
            }),
        },
    }
}

// ─── ASR 管理器（Tauri 托管状态）──────────────────────────

/// 给前端展示的 ASR 运行时状态
#[derive(Clone, Serialize)]
pub struct AsrRuntimeStatus {
    pub provider: String,
    pub ready: bool,
    pub message: String,
}

/// 全局 ASR 管理器 —— 持有配置与当前 provider
pub struct AsrManager {
    config: RuntimeConfig,
    runtime_dir: PathBuf,
    provider: Mutex<Box<dyn AsrProvider>>,
}

impl AsrManager {
    pub fn new(config: RuntimeConfig, runtime_dir: PathBuf) -> Self {
        // 已配置 MOSS 可执行文件则走 Moss；否则用 None 占位
        let kind = if config.moss_binary.is_empty() {
            AsrProviderKind::None
        } else {
            AsrProviderKind::Moss
        };
        let provider = create_asr_provider(kind, &config, &runtime_dir);
        Self {
            config,
            runtime_dir,
            provider: Mutex::new(provider),
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn runtime_dir(&self) -> &PathBuf {
        &self.runtime_dir
    }

    /// 作用域化访问 provider：内部加锁，避免把 MutexGuard 泄漏到调用方。
    /// 锁被污染时退化为使用被污染的 guard 内的值，保证调用不中断。
    pub fn with_provider<R>(&self, f: impl FnOnce(&dyn AsrProvider) -> R) -> R {
        let guard = self
            .provider
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&**guard)
    }

    /// 汇总当前运行时状态（供 check_asr_runtime 命令）
    pub fn status(&self) -> AsrRuntimeStatus {
        let name = self.with_provider(|p| p.name().to_string());
        let ready = self.with_provider(|p| p.is_ready());
        let detail = self.with_provider(|p| p.describe());
        let message = if ready {
            format!("ASR 运行时就绪（provider: {}）", name)
        } else if name == "none" {
            "未配置 ASR 运行环境".into()
        } else if detail.is_empty() {
            format!("ASR 运行环境未就绪（provider: {}）", name)
        } else {
            format!("ASR 运行环境未就绪（provider: {}）：{}", name, detail)
        };
        AsrRuntimeStatus {
            provider: name,
            ready,
            message,
        }
    }
}

// ─── ASR 单元测试 ─────────────────────────────────────────

#[cfg(test)]
mod asr_tests {
    use super::*;

    #[test]
    fn test_asr_none_provider_not_ready() {
        let p = AsrNoneProvider;
        assert_eq!(p.name(), "none");
        assert!(!p.is_ready());
        let result = p.transcribe(Path::new("a.wav"));
        assert!(matches!(result, Err(AsrError::NotReady)));
    }

    #[test]
    fn test_asr_manager_status_not_ready() {
        let manager = AsrManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        let status = manager.status();
        assert_eq!(status.provider, "none");
        assert!(!status.ready);
    }

    #[test]
    fn test_asr_manager_selects_moss_kind() {
        let mut config = RuntimeConfig::default();
        config.moss_binary = "bin/moss-transcribe.exe".into();
        config.moss_model = "models/moss/model.gguf".into();
        let manager = AsrManager::new(config, PathBuf::from("no_such_runtime_xyz"));
        let status = manager.status();
        assert_eq!(status.provider, "moss");
        // runtime 目录不存在 → spawn 失败 → broken（describe 非空）
        assert!(!status.ready);
        assert!(!status.message.is_empty());
    }

    #[test]
    fn test_asr_broken_provider_reports_reason() {
        let p = AsrBrokenProvider {
            name: "moss".into(),
            message: "可执行文件不存在".into(),
        };
        assert_eq!(p.name(), "moss");
        assert!(!p.is_ready());
        assert_eq!(p.describe(), "可执行文件不存在");
        assert!(p.transcribe(Path::new("a.wav")).is_err());
    }

    #[test]
    fn test_asr_manager_survives_poisoned_lock() {
        // 构造一个已污染的 Mutex：持有 guard 时 panic
        let mutex = Mutex::new(Box::new(AsrNoneProvider) as Box<dyn AsrProvider>);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            std::panic::panic_any("poison");
        }));
        let mgr = AsrManager {
            config: RuntimeConfig::default(),
            runtime_dir: PathBuf::from("runtime"),
            provider: mutex,
        };
        // with_provider 应退化为使用被污染 guard 内的值，不 panic
        let status = mgr.status();
        assert_eq!(status.provider, "none");
        assert!(!status.ready);
    }
}

// ─── ASR Tauri 命令 ──────────────────────────────────────

/// Tauri 命令：探测 ASR 运行环境是否就绪
#[tauri::command]
pub fn check_asr_runtime(state: tauri::State<'_, AsrManager>) -> AsrRuntimeStatus {
    state.status()
}

// ─── LLM 数据与错误 ──────────────────────────────────────

/// LLM 推理错误类型 —— 尽量保留底层错误链
#[derive(Debug)]
pub enum LlmError {
    /// 运行环境未就绪（可执行文件/模型缺失）
    NotReady,
    /// 环境层面错误：进程启动失败、依赖缺失等
    Runtime(Box<dyn Error + Send + Sync>),
    /// 推理进程返回的错误
    Worker(String),
    /// IO 错误
    Io(std::io::Error),
}

impl LlmError {
    pub fn runtime<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        LlmError::Runtime(Box::new(err))
    }
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        LlmError::Io(e)
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::NotReady => write!(f, "LLM 运行环境未就绪"),
            LlmError::Runtime(err) => write!(f, "LLM 运行环境错误: {}", err),
            LlmError::Worker(msg) => write!(f, "LLM worker 错误: {}", msg),
            LlmError::Io(err) => write!(f, "LLM IO 错误: {}", err),
        }
    }
}

impl std::error::Error for LlmError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LlmError::Runtime(err) => Some(err.as_ref()),
            LlmError::Io(err) => Some(err),
            _ => None,
        }
    }
}

// ─── LLM Provider 抽象 ───────────────────────────────────

/// LLM 提供者。单轮推理（prompt → 文本）为第一接口（与 llama-cli 单轮模式对应）。
/// 只要求 `Send`，管理器外套 Mutex 提供 `Sync`。
pub trait LlmProvider: Send {
    /// provider 名称（"none" / "llama" 等）
    fn name(&self) -> &str;
    /// 运行环境是否就绪
    fn is_ready(&self) -> bool;
    /// 附加说明（如未就绪的原因），默认空
    fn describe(&self) -> String {
        String::new()
    }
    /// 执行一次单轮推理，返回回答文本。
    /// `max_tokens` 为生成上限（llama-cli -n 参数）
    fn complete(&self, prompt: &str, max_tokens: u32) -> Result<String, LlmError>;
}

/// 占位 provider —— 环境未就绪时使用，保证管线可编译
pub struct LlmNoneProvider;

impl LlmProvider for LlmNoneProvider {
    fn name(&self) -> &str {
        "none"
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn complete(&self, _prompt: &str, _max_tokens: u32) -> Result<String, LlmError> {
        Err(LlmError::NotReady)
    }
}

/// 创建失败的 provider —— 携带失败原因，供状态探测展示
pub struct LlmBrokenProvider {
    name: String,
    message: String,
}

impl LlmProvider for LlmBrokenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        self.message.clone()
    }

    fn complete(&self, _prompt: &str, _max_tokens: u32) -> Result<String, LlmError> {
        Err(LlmError::Runtime(Box::new(std::io::Error::other(
            self.message.clone(),
        ))))
    }
}

// ─── LLM Provider 工厂 ───────────────────────────────────

/// 支持的 LLM provider 类型（可扩展）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmProviderKind {
    None,
    /// llama.cpp CLI（Phase 5 接入）
    Llama,
}

/// 根据类型与配置创建 provider 实例
/// Llama 创建失败时返回 BrokenProvider（携带原因），不 panic
pub fn create_llm_provider(
    kind: LlmProviderKind,
    config: &RuntimeConfig,
    runtime_dir: &PathBuf,
) -> Box<dyn LlmProvider> {
    match kind {
        LlmProviderKind::None => Box::new(LlmNoneProvider),
        LlmProviderKind::Llama => match llm::LlamaProvider::spawn(config, runtime_dir) {
            Ok(p) => Box::new(p),
            Err(e) => Box::new(LlmBrokenProvider {
                name: "llama".into(),
                message: e.to_string(),
            }),
        },
    }
}

// ─── LLM 管理器（Tauri 托管状态）──────────────────────────

/// 给前端展示的 LLM 运行时状态
#[derive(Clone, Serialize)]
pub struct LlmRuntimeStatus {
    pub provider: String,
    pub ready: bool,
    pub message: String,
}

/// 全局 LLM 管理器 —— 持有配置与当前 provider
pub struct LlmManager {
    config: RuntimeConfig,
    runtime_dir: PathBuf,
    provider: Mutex<Box<dyn LlmProvider>>,
}

impl LlmManager {
    pub fn new(config: RuntimeConfig, runtime_dir: PathBuf) -> Self {
        // 已配置 llama-cli 可执行文件则走 Llama；否则用 None 占位
        let kind = if config.llm_binary.is_empty() {
            LlmProviderKind::None
        } else {
            LlmProviderKind::Llama
        };
        let provider = create_llm_provider(kind, &config, &runtime_dir);
        Self {
            config,
            runtime_dir,
            provider: Mutex::new(provider),
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn runtime_dir(&self) -> &PathBuf {
        &self.runtime_dir
    }

    /// 作用域化访问 provider：内部加锁，避免把 MutexGuard 泄漏到调用方。
    /// 锁被污染时退化为使用被污染的 guard 内的值，保证调用不中断。
    pub fn with_provider<R>(&self, f: impl FnOnce(&dyn LlmProvider) -> R) -> R {
        let guard = self
            .provider
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&**guard)
    }

    /// 汇总当前运行时状态（供 check_llm_runtime 命令）
    pub fn status(&self) -> LlmRuntimeStatus {
        let name = self.with_provider(|p| p.name().to_string());
        let ready = self.with_provider(|p| p.is_ready());
        let detail = self.with_provider(|p| p.describe());
        let message = if ready {
            format!("LLM 运行时就绪（provider: {}）", name)
        } else if name == "none" {
            "未配置 LLM 运行环境".into()
        } else if detail.is_empty() {
            format!("LLM 运行环境未就绪（provider: {}）", name)
        } else {
            format!("LLM 运行环境未就绪（provider: {}）：{}", name, detail)
        };
        LlmRuntimeStatus {
            provider: name,
            ready,
            message,
        }
    }
}

// ─── LLM 单元测试 ─────────────────────────────────────────

#[cfg(test)]
mod llm_tests {
    use super::*;

    #[test]
    fn test_llm_none_provider_not_ready() {
        let p = LlmNoneProvider;
        assert_eq!(p.name(), "none");
        assert!(!p.is_ready());
        let result = p.complete("hi", 256);
        assert!(matches!(result, Err(LlmError::NotReady)));
    }

    #[test]
    fn test_llm_manager_status_not_ready() {
        let manager = LlmManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        let status = manager.status();
        assert_eq!(status.provider, "none");
        assert!(!status.ready);
    }

    #[test]
    fn test_llm_manager_selects_llama_kind() {
        let mut config = RuntimeConfig::default();
        config.llm_binary = "bin/llama-cli.exe".into();
        config.llm_model = "models/qwen/model.gguf".into();
        let manager = LlmManager::new(config, PathBuf::from("no_such_runtime_xyz"));
        let status = manager.status();
        assert_eq!(status.provider, "llama");
        // runtime 目录不存在 → spawn 失败 → broken（describe 非空）
        assert!(!status.ready);
        assert!(!status.message.is_empty());
    }
}

// ─── LLM Tauri 命令 ──────────────────────────────────────

/// Tauri 命令：探测 LLM 运行环境是否就绪。
/// async + spawn_blocking：推理进行中会短暂占用 provider 锁，
/// sync 命令跑在主线程会等锁导致整窗冻结（换 3B 模型后更明显）。
#[tauri::command]
pub async fn check_llm_runtime(app: tauri::AppHandle) -> LlmRuntimeStatus {
    tauri::async_runtime::spawn_blocking(move || app.state::<LlmManager>().status())
        .await
        .unwrap_or_else(|e| LlmRuntimeStatus {
            provider: "unknown".into(),
            ready: false,
            message: format!("LLM 运行环境探测失败: {}", e),
        })
}
