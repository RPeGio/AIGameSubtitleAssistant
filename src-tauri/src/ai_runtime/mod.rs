// ─── AI 运行时模块 ───────────────────────────────────────
// 提供统一的 AI Provider 抽象，支持可插拔实现：
//   - OCR：PaddleOCR（Python Worker，2.2 接入）/ 未来 ONNX / 云端 API
//   - ASR：MOSS（Phase 3）
//   - 文本处理：Local LLM（Phase 5）
//
// 本阶段（2.1）只搭骨架：trait + 管理器 + 配置 + 占位 provider。

pub mod config;
pub mod dhash;
pub mod funasr;
pub mod llm;
pub mod moss;
pub mod paddle;

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

/// 单次转写的运行时参数（前端配置面板传入；引擎不支持的字段会被忽略）。
/// 引擎差异：FunASR 用 language/max_speakers，MOSS 全部忽略（自动估计）。
#[derive(Clone, Debug, Default)]
pub struct AsrTranscribeOptions {
    /// 识别语言（"auto"/"zh"/"en"/"ja"；空 = 自动检测）
    pub language: Option<String>,
    /// 说话人上限（None = 自动估计）
    pub max_speakers: Option<u32>,
}

/// ASR 提供者。整段音频一次转写为第一接口（与 MOSS CLI 的 JSON 输出对应）。
/// 要求 `Send + Sync`：transcribe 是分钟级阻塞调用，管理器以 `Arc` 共享、
/// 不加锁——cancel()（取消/超时/退出钩子）必须能与转写并发执行。
pub trait AsrProvider: Send + Sync {
    /// provider 名称（"none" / "moss" 等）
    fn name(&self) -> &str;
    /// 运行环境是否就绪
    fn is_ready(&self) -> bool;
    /// 附加说明（如未就绪的原因），默认空
    fn describe(&self) -> String {
        String::new()
    }
    /// 转写整段音频（WAV 文件路径），返回带说话人标签的时间轴段。
    /// options 为本次运行参数（语言/说话人上限等），不支持的引擎应忽略。
    fn transcribe(
        &self,
        audio_path: &Path,
        options: &AsrTranscribeOptions,
    ) -> Result<Vec<AsrSegment>, AsrError>;
    /// 开始一次新的转写任务前调用：清除上一次的取消/错误残留（默认无操作）。
    /// 分段转写中 transcribe() 会被逐段调用，取消标志必须按"任务"而非"段"
    /// 重置——否则段间窗口内的取消会在下一段入口被吞掉。
    fn reset(&self) {}
    /// 取消当前转写（默认无操作；MOSS 实现为终止活动子进程）。
    /// 供前端"取消 ASR"、超时与应用退出钩子调用。
    fn cancel(&self) {}
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

    fn transcribe(
        &self,
        _audio_path: &Path,
        _options: &AsrTranscribeOptions,
    ) -> Result<Vec<AsrSegment>, AsrError> {
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

    fn transcribe(
        &self,
        _audio_path: &Path,
        _options: &AsrTranscribeOptions,
    ) -> Result<Vec<AsrSegment>, AsrError> {
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
    /// FunASR Python worker（Fun-ASR-Nano + VAD + cam++ 说话人 + 标点）
    FunAsr,
}

/// 根据类型与配置创建 provider 实例
/// 创建失败时返回 BrokenProvider（携带原因），不 panic
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
        AsrProviderKind::FunAsr => match funasr::FunAsrProvider::spawn(config, runtime_dir) {
            Ok(p) => Box::new(p),
            Err(e) => Box::new(AsrBrokenProvider {
                name: "funasr".into(),
                message: e.to_string(),
            }),
        },
    }
}

// ─── ASR 管理器（Tauri 托管状态）──────────────────────────

/// 给前端展示的 ASR 运行时状态（兼容旧 check_asr_runtime 命令，汇总全部引擎）
#[derive(Clone, Serialize)]
pub struct AsrRuntimeStatus {
    pub provider: String,
    pub ready: bool,
    pub message: String,
}

/// 单个 ASR 引擎的状态（供配置面板禁用不可用引擎）
#[derive(Clone, Serialize)]
pub struct AsrEngineStatus {
    pub engine: String,
    pub ready: bool,
    pub message: String,
}

/// 全局 ASR 管理器 —— 同时持有 funasr / moss 两个引擎（各自按配置就绪），
/// 由前端配置面板在每次运行时选择。provider 以 Arc 共享、无互斥锁：
/// transcribe 阻塞调用与 cancel（取消/超时/退出钩子）需要并发，锁会把
/// 取消拖到转写结束（失效）。
pub struct AsrManager {
    config: RuntimeConfig,
    runtime_dir: PathBuf,
    funasr: Arc<dyn AsrProvider>,
    moss: Arc<dyn AsrProvider>,
    /// 转写进行中标志（test-and-set）：Arc 去锁后不再天然串行转写，
    /// 该标志是后端兜底，防并发转写互相覆盖 active_child / 互相清理
    transcribing: AtomicBool,
}

impl AsrManager {
    pub fn new(config: RuntimeConfig, runtime_dir: PathBuf) -> Self {
        // 双引擎并存：funasr 需三路径字段齐全（worker/deps/model_dir），
        // moss 需 binary + model；未配置的引擎用 None 占位
        let funasr = if !config.funasr_worker.is_empty()
            && !config.funasr_deps.is_empty()
            && !config.funasr_model_dir.is_empty()
        {
            create_asr_provider(AsrProviderKind::FunAsr, &config, &runtime_dir)
        } else {
            Box::new(AsrNoneProvider)
        };
        let moss = if !config.moss_binary.is_empty() && !config.moss_model.is_empty() {
            create_asr_provider(AsrProviderKind::Moss, &config, &runtime_dir)
        } else {
            Box::new(AsrNoneProvider)
        };
        Self {
            config,
            runtime_dir,
            funasr: Arc::from(funasr),
            moss: Arc::from(moss),
            transcribing: AtomicBool::new(false),
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn runtime_dir(&self) -> &PathBuf {
        &self.runtime_dir
    }

    /// 访问指定引擎的 provider（无锁：provider 内部自带同步，transcribe 与 cancel 可并发）
    pub fn with_engine<R>(&self, key: &str, f: impl FnOnce(&dyn AsrProvider) -> R) -> R {
        match key {
            "moss" => f(&*self.moss),
            _ => f(&*self.funasr),
        }
    }

    /// 两引擎状态列表（供 check_asr_engines 命令展示面板可用性）
    pub fn engines(&self) -> Vec<AsrEngineStatus> {
        ["funasr", "moss"]
            .iter()
            .map(|key| {
                let (ready, message) = self.with_engine(key, |p| (p.is_ready(), p.describe()));
                AsrEngineStatus {
                    engine: (*key).into(),
                    ready,
                    message,
                }
            })
            .collect()
    }

    /// 尝试开始一次转写（test-and-set）：已有转写在进行时返回 false。
    /// 成功后必须调用 finish_transcribe 释放。
    pub fn try_begin_transcribe(&self) -> bool {
        self.transcribing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// 结束转写（成功/失败/取消后调用，配合 try_begin_transcribe）
    pub fn finish_transcribe(&self) {
        self.transcribing.store(false, Ordering::SeqCst);
    }

    /// 汇总当前运行时状态（供 check_asr_runtime 命令；任一引擎就绪即视为可用）
    pub fn status(&self) -> AsrRuntimeStatus {
        let engines = self.engines();
        let ready: Vec<_> = engines.iter().filter(|e| e.ready).collect();
        if !ready.is_empty() {
            let names = ready
                .iter()
                .map(|e| e.engine.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            AsrRuntimeStatus {
                provider: names.clone(),
                ready: true,
                message: format!("ASR 运行时就绪（provider: {}）", names),
            }
        } else {
            let parts: Vec<String> = engines
                .iter()
                .filter(|e| !e.message.is_empty())
                .map(|e| format!("{}: {}", e.engine, e.message))
                .collect();
            AsrRuntimeStatus {
                provider: engines
                    .iter()
                    .map(|e| e.engine.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                ready: false,
                message: if parts.is_empty() {
                    "未配置 ASR 运行环境".into()
                } else {
                    parts.join("；")
                },
            }
        }
    }

    /// 请求取消当前转写（两引擎同时取消，幂等：无活动任务时无副作用）
    pub fn cancel(&self) {
        self.funasr.cancel();
        self.moss.cancel();
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
        let result = p.transcribe(Path::new("a.wav"), &AsrTranscribeOptions::default());
        assert!(matches!(result, Err(AsrError::NotReady)));
    }

    #[test]
    fn test_asr_manager_status_not_ready() {
        let manager = AsrManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        let status = manager.status();
        assert!(!status.ready);
    }

    #[test]
    fn test_asr_manager_selects_moss_kind() {
        let mut config = RuntimeConfig::default();
        config.moss_binary = "bin/moss-transcribe.exe".into();
        config.moss_model = "models/moss/model.gguf".into();
        let manager = AsrManager::new(config, PathBuf::from("no_such_runtime_xyz"));
        let engines = manager.engines();
        let moss = engines.iter().find(|e| e.engine == "moss").unwrap();
        assert_eq!(moss.engine, "moss");
        // runtime 目录不存在 → spawn 失败 → broken（ready=false，message 非空）
        assert!(!moss.ready);
        assert!(!moss.message.is_empty());
        // funasr 未配置 → none 占位
        assert!(!engines.iter().find(|e| e.engine == "funasr").unwrap().ready);
    }

    #[test]
    fn test_asr_manager_selects_funasr_kind() {
        // funasr 三路径齐全 → funasr 引擎（即使 moss 也配置了）
        let mut config = RuntimeConfig::default();
        config.funasr_worker = "worker/funasr_worker.py".into();
        config.funasr_deps = "deps_funasr".into();
        config.funasr_model_dir = "models/funasr".into();
        config.moss_binary = "bin/moss-transcribe.exe".into();
        let manager = AsrManager::new(config, PathBuf::from("no_such_runtime_xyz"));
        let engines = manager.engines();
        assert_eq!(
            engines.iter().find(|e| e.engine == "funasr").unwrap().engine,
            "funasr"
        );
        assert_eq!(
            engines.iter().find(|e| e.engine == "moss").unwrap().engine,
            "moss"
        );
    }

    #[test]
    fn test_asr_manager_engines_both_available_when_configured() {
        // 双引擎并存：funasr 与 moss 都配置时互不影响
        let mut config = RuntimeConfig::default();
        config.funasr_worker = "worker/funasr_worker.py".into();
        config.funasr_deps = "deps_funasr".into();
        config.funasr_model_dir = "models/funasr".into();
        config.moss_binary = "bin/moss-transcribe.exe".into();
        config.moss_model = "models/moss/model.gguf".into();
        let manager = AsrManager::new(config, PathBuf::from("no_such_runtime_xyz"));
        assert_eq!(
            manager.with_engine("funasr", |p| p.name().to_string()),
            "funasr"
        );
        assert_eq!(manager.with_engine("moss", |p| p.name().to_string()), "moss");
    }

    #[test]
    fn test_asr_manager_partial_funasr_keeps_moss() {
        // 只填了 funasr_worker（半配置）→ funasr 引擎不启用，moss 引擎正常
        let mut config = RuntimeConfig::default();
        config.funasr_worker = "worker/funasr_worker.py".into();
        config.moss_binary = "bin/moss-transcribe.exe".into();
        config.moss_model = "models/moss/model.gguf".into();
        let manager = AsrManager::new(config, PathBuf::from("no_such_runtime_xyz"));
        assert_eq!(
            manager.with_engine("funasr", |p| p.name().to_string()),
            "none"
        );
        assert_eq!(manager.with_engine("moss", |p| p.name().to_string()), "moss");
    }

    #[test]
    fn test_asr_manager_blocks_concurrent_transcribe() {
        // 后端重入守卫：第二次 try_begin 应失败，finish 后恢复
        let mgr = AsrManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        assert!(mgr.try_begin_transcribe());
        assert!(!mgr.try_begin_transcribe(), "并发转写应被拒绝");
        mgr.finish_transcribe();
        assert!(mgr.try_begin_transcribe(), "finish 后应恢复可用");
        mgr.finish_transcribe();
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
        assert!(
            p.transcribe(Path::new("a.wav"), &AsrTranscribeOptions::default())
                .is_err()
        );
    }

    #[test]
    fn test_asr_manager_shares_provider_without_lock() {
        // provider 以 Arc 共享（无互斥锁）：transcribe 阻塞期间 cancel 可并发；
        // 无活动任务时 cancel 无副作用、不 panic
        let mgr = AsrManager::new(RuntimeConfig::default(), PathBuf::from("runtime"));
        assert_eq!(mgr.with_engine("funasr", |p| p.name().to_string()), "none");
        mgr.cancel();
        let status = mgr.status();
        assert!(!status.ready);
    }
}

// ─── ASR Tauri 命令 ──────────────────────────────────────

/// Tauri 命令：探测 ASR 运行环境是否就绪
#[tauri::command]
pub fn check_asr_runtime(state: tauri::State<'_, AsrManager>) -> AsrRuntimeStatus {
    state.status()
}

/// Tauri 命令：探测两个 ASR 引擎（funasr/moss）各自的状态（供配置面板）
#[tauri::command]
pub fn check_asr_engines(state: tauri::State<'_, AsrManager>) -> Vec<AsrEngineStatus> {
    state.engines()
}

/// Tauri 命令：取消当前 ASR 转写（终止活动的 MOSS 子进程）
#[tauri::command]
pub fn asr_cancel(state: tauri::State<'_, AsrManager>) {
    state.cancel();
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
