// ─── AI 运行时模块 ───────────────────────────────────────
// 提供统一的 AI Provider 抽象，支持可插拔实现：
//   - OCR：PaddleOCR（Python Worker，2.2 接入）/ 未来 ONNX / 云端 API
//   - ASR：MOSS（Phase 3）
//   - 文本处理：Local LLM（Phase 5）
//
// 本阶段（2.1）只搭骨架：trait + 管理器 + 配置 + 占位 provider。

pub mod backend_probe;
pub mod config;
pub mod dhash;
pub mod embed;
pub mod funasr;
pub mod installed_state;
pub mod llm;
pub mod moss;
pub mod paddle;
pub mod runtime_install;

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
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

/// 安装器约定的 MOSS 标准布局（与 manifest 条目的 `dest` 一致）
pub(crate) const MOSS_BINARY_REL: &str = "bin/moss-transcribe.exe";
pub(crate) const MOSS_MODEL_REL: &str = "models/moss/moss-transcribe-q5_k.gguf";

/// 解析 MOSS 的二进制与模型路径。
///
/// 为什么空字段要回退到标准布局：一键安装（欢迎页自检卡片）只解包文件、**不写
/// config.json**，所以"刚装完、配置还没生成"是正常状态。此时若把空字段当成
/// runtime 目录本身（`resolve_path("")` 的结果），就会把已装好的运行时判成
/// "文件缺失"——用户看到的是装完反而坏了。用户自定义过路径时 config 里一定有值，
/// 回退不会被触发。
pub(crate) fn moss_paths(config: &RuntimeConfig, runtime_dir: &Path) -> (PathBuf, PathBuf) {
    let binary = if config.moss_binary.trim().is_empty() {
        resolve_path(MOSS_BINARY_REL, runtime_dir)
    } else {
        resolve_path(&config.moss_binary, runtime_dir)
    };
    let model = if config.moss_model.trim().is_empty() {
        resolve_path(MOSS_MODEL_REL, runtime_dir)
    } else {
        resolve_path(&config.moss_model, runtime_dir)
    };
    (binary, model)
}

/// 安装器约定的 OCR 标准布局（与 manifest 条目 `deps-ocr` 的 `dest: "."` 一致）
pub(crate) const OCR_PYTHON_REL: &str = "python/python.exe";
pub(crate) const OCR_DEPS_REL: &str = "deps";
pub(crate) const OCR_DEPS_GPU_REL: &str = "deps_gpu";
pub(crate) const OCR_MODELS_REL: &str = "models/paddleocr";
pub(crate) const OCR_WORKER_REL: &str = "worker/ocr_worker.py";

/// OCR 运行时的四个关键路径：(python 可执行, 依赖目录, 模型目录, worker 脚本)。
///
/// 与 `moss_paths` 同一动机：一键安装只解包、不写 config.json，所以配置为空时
/// 必须回退到标准布局，否则会把刚装好的运行时判成"缺失"。
///
/// 这里比 `moss_paths` 多一条判据：**配置值指向的东西在磁盘上不存在时也回退**。
/// 原因是 `RuntimeConfig::default()` 的 `python_path` 是历史占位值 "python"
/// （不是 "python/python.exe"），按字面 join 会得到一个目录；安装器又不会写
/// config.json。存在性判定能让"刚装完"和"老的占位配置"都落到标准布局。
/// 用户自定义过路径时那个路径通常存在，回退不会被触发。
/// 注意 `deps_gpu` 是**可选**增量（只有 NVIDIA 机器会有），不参与这里的解析。
pub(crate) fn ocr_paths(config: &RuntimeConfig, runtime_dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    fn pick(value: &str, rel: &str, runtime_dir: &Path, want_dir: bool) -> PathBuf {
        let std_path = resolve_path(rel, runtime_dir);
        let value = value.trim();
        if value.is_empty() {
            return std_path;
        }
        let custom = resolve_path(value, runtime_dir);
        let ok = if want_dir {
            custom.is_dir()
        } else {
            custom.is_file()
        };
        if ok {
            custom
        } else {
            std_path
        }
    }
    (
        pick(&config.python_path, OCR_PYTHON_REL, runtime_dir, false),
        pick(&config.deps_dir, OCR_DEPS_REL, runtime_dir, true),
        pick(&config.model_dir, OCR_MODELS_REL, runtime_dir, true),
        pick(&config.worker_script, OCR_WORKER_REL, runtime_dir, false),
    )
}

/// 安装器约定的向量融合（embed）标准布局（与 manifest 条目 `deps-embed` 的 `dest: "."` 一致）
pub(crate) const EMBED_WORKER_REL: &str = "worker/embed_worker.py";
pub(crate) const EMBED_DEPS_REL: &str = "deps_embed";
pub(crate) const EMBED_MODEL_REL: &str =
    "models/embed/multilingual-e5-small/model_qint8_avx512_vnni.onnx";
pub(crate) const EMBED_TOKENIZER_REL: &str =
    "models/embed/multilingual-e5-small/tokenizer.json";

/// embed 运行时的关键路径：(python, worker, 依赖目录, 模型, tokenizer)。
///
/// 与 `moss_paths` / `ocr_paths` 同一动机：一键安装只解包、不写 config.json，
/// 所以配置为空时必须回退到标准布局——否则"刚装完"会被判成"未配置"，
/// 用户看到的是装好了却不能用（PR43 前 MOSS/OCR 都踩过这个坑）。
///
/// embed 复用 OCR 包带来的 `python/python.exe`（不单独分发解释器），
/// 因此 `deps-embed` 的安装前置是 `deps-ocr` 已装。
pub(crate) fn embed_paths(
    config: &RuntimeConfig,
    runtime_dir: &Path,
) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    fn pick(value: &str, rel: &str, runtime_dir: &Path, want_dir: bool) -> PathBuf {
        let std_path = resolve_path(rel, runtime_dir);
        let value = value.trim();
        if value.is_empty() {
            return std_path;
        }
        let custom = resolve_path(value, runtime_dir);
        let ok = if want_dir {
            custom.is_dir()
        } else {
            custom.is_file()
        };
        if ok {
            custom
        } else {
            std_path
        }
    }
    (
        pick(&config.python_path, OCR_PYTHON_REL, runtime_dir, false),
        pick(&config.embed_worker, EMBED_WORKER_REL, runtime_dir, false),
        pick(&config.embed_deps_dir, EMBED_DEPS_REL, runtime_dir, true),
        pick(&config.embed_model, EMBED_MODEL_REL, runtime_dir, false),
        pick(&config.embed_tokenizer, EMBED_TOKENIZER_REL, runtime_dir, false),
    )
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
        // 已配置 worker 脚本、或标准布局里已有 worker（一键安装后没有 config.json）
        // 都走 Paddle；都没有才用 None 占位
        let worker = ocr_paths(&config, &runtime_dir).3;
        let kind = if config.worker_script.is_empty() && !worker.is_file() {
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

    /// 从磁盘重建 provider（一键安装/更新运行时之后调用，让新装的 worker 立刻生效）。
    ///
    /// 与 `new()` 用同一套判定（含「空配置回退标准布局」），所以「装完必须重启」
    /// 这条边界不存在了 —— 真机 2026-10-31 的「脚本跑完 4 个都未就绪」就是它。
    pub fn reload(&self) -> Result<String, String> {
        let config =
            RuntimeConfig::load(&self.runtime_dir).unwrap_or_else(|_| self.config.clone());
        let worker = ocr_paths(&config, &self.runtime_dir).3;
        let kind = if config.worker_script.is_empty() && !worker.is_file() {
            OcrProviderKind::None
        } else {
            OcrProviderKind::Paddle
        };
        let provider = create_ocr_provider(kind, &config, &self.runtime_dir);
        let (name, ready, detail) = (
            provider.name().to_string(),
            provider.is_ready(),
            provider.describe(),
        );
        {
            let mut guard = self.provider.lock().unwrap_or_else(|e| e.into_inner());
            *guard = provider;
        }
        Ok(if ready {
            format!("OCR 已重新加载（provider: {name}）")
        } else if detail.is_empty() {
            format!("OCR 已重新加载但未就绪（provider: {name}）")
        } else {
            format!("OCR 已重新加载但未就绪（provider: {name}）：{detail}")
        })
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
    fn test_ocr_reload_reports_not_ready_without_worker() {
        // 一键装完运行时后要能"热加载"：目录里没有 worker 时如实说未就绪，而不是报错或 panic
        let dir = std::env::temp_dir().join(format!("gsa_ocr_reload_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manager = OcrManager::new(RuntimeConfig::default(), dir.clone());
        let msg = manager.reload().expect("没有 config.json 也应能重建（回退默认配置）");
        assert!(msg.contains("未就绪"), "reload 应如实说未就绪：{msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ocr_paths_fall_back_to_standard_layout() {
        // 安装器只解包、不写 config.json，且 default() 里 python_path 是历史占位值
        // "python"（目录，不是 python/python.exe）——两种情形都必须落到标准布局
        let dir = std::env::temp_dir().join(format!("gsa_ocr_paths_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (python, deps, models, worker) = ocr_paths(&RuntimeConfig::default(), &dir);
        assert_eq!(python, dir.join(OCR_PYTHON_REL));
        assert_eq!(deps, dir.join(OCR_DEPS_REL));
        assert_eq!(models, dir.join(OCR_MODELS_REL));
        assert_eq!(worker, dir.join(OCR_WORKER_REL));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ocr_paths_keep_existing_custom_paths() {
        // 自定义路径存在时必须用它（不能被回退逻辑吃掉）
        let dir = std::env::temp_dir().join(format!("gsa_ocr_custom_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("py")).unwrap();
        std::fs::create_dir_all(dir.join("dep")).unwrap();
        std::fs::create_dir_all(dir.join("mdl")).unwrap();
        std::fs::write(dir.join("py\\python.exe"), "x").unwrap();
        std::fs::write(dir.join("w.py"), "x").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "py/python.exe".into();
        cfg.deps_dir = "dep".into();
        cfg.model_dir = "mdl".into();
        cfg.worker_script = "w.py".into();
        let (python, deps, models, worker) = ocr_paths(&cfg, &dir);
        assert_eq!(python, dir.join("py\\python.exe"));
        assert_eq!(deps, dir.join("dep"));
        assert_eq!(models, dir.join("mdl"));
        assert_eq!(worker, dir.join("w.py"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embed_paths_fall_back_to_standard_layout() {
        // 与 moss/ocr 同一个坑：一键安装不写 config.json，default() 的字段全是空的
        let dir = std::env::temp_dir().join(format!("gsa_embed_paths_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (python, worker, deps, model, tokenizer) =
            embed_paths(&RuntimeConfig::default(), &dir);
        assert_eq!(python, dir.join(OCR_PYTHON_REL));
        assert_eq!(worker, dir.join(EMBED_WORKER_REL));
        assert_eq!(deps, dir.join(EMBED_DEPS_REL));
        assert_eq!(model, dir.join(EMBED_MODEL_REL));
        assert_eq!(tokenizer, dir.join(EMBED_TOKENIZER_REL));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embed_paths_keep_existing_custom_paths() {
        let dir = std::env::temp_dir().join(format!("gsa_embed_custom_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dep")).unwrap();
        std::fs::write(dir.join("py.exe"), "x").unwrap();
        std::fs::write(dir.join("w.py"), "x").unwrap();
        std::fs::write(dir.join("m.onnx"), "x").unwrap();
        std::fs::write(dir.join("t.json"), "x").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "py.exe".into();
        cfg.embed_worker = "w.py".into();
        cfg.embed_deps_dir = "dep".into();
        cfg.embed_model = "m.onnx".into();
        cfg.embed_tokenizer = "t.json".into();
        let (python, worker, deps, model, tokenizer) = embed_paths(&cfg, &dir);
        assert_eq!(python, dir.join("py.exe"));
        assert_eq!(worker, dir.join("w.py"));
        assert_eq!(deps, dir.join("dep"));
        assert_eq!(model, dir.join("m.onnx"));
        assert_eq!(tokenizer, dir.join("t.json"));
        let _ = std::fs::remove_dir_all(&dir);
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

/// 运行时整体重载的结果（一句话/条，直接可显示）
#[derive(Clone, Serialize)]
pub struct RuntimeReloadSummary {
    pub asr: Vec<String>,
    pub ocr: String,
}

/// Tauri 命令：安装/更新运行时后重建 ASR 与 OCR 的 provider。
///
/// 为什么需要它：provider 是 setup 阶段按当时的文件情况一次性构造的（缺文件时
/// 会退化成 None/Broken 占位），一键安装只是往磁盘写文件，进程内的对象不会
/// 自动换新 —— 于是"装完不重启就还是未就绪"。重建比重启应用便宜得多。
#[tauri::command]
pub async fn reload_runtimes(app: tauri::AppHandle) -> RuntimeReloadSummary {
    // MOSS 重建要跑 `info`（数秒），放命令线程上会把窗口拖住
    tauri::async_runtime::spawn_blocking(move || {
        let mut asr = Vec::new();
        match app.try_state::<AsrManager>() {
            Some(manager) => {
                for key in ["moss", "funasr"] {
                    asr.push(match manager.reload_engine(key) {
                        Ok(m) => m,
                        Err(e) => format!("{key}: {e}"),
                    });
                }
            }
            None => asr.push("ASR 管理器未初始化".into()),
        }
        let ocr = match app.try_state::<OcrManager>() {
            Some(manager) => manager
                .reload()
                .unwrap_or_else(|e| format!("OCR 重新加载失败：{e}")),
            None => "OCR 管理器未初始化".into(),
        };
        RuntimeReloadSummary { asr, ocr }
    })
    .await
    .unwrap_or_else(|e| RuntimeReloadSummary {
        asr: vec![format!("重新加载失败：{e}")],
        ocr: format!("重新加载失败：{e}"),
    })
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
    /// 后台预热（启动后由管理器的后台线程调用一次）。
    ///
    /// 默认空实现——探测成本高的引擎（MOSS 要跑 `info` 加载 GGUF 元数据）在这里做，
    /// **不要**放进构造函数，否则会把 setup 阶段的主线程卡住（真机反馈 2026-10-29）。
    fn warm_up(&self) {}
    /// 后台重试预热（默认等同 `warm_up`）。
    ///
    /// 由状态轮询（`check_asr_engines`）在「未就绪」时触发：装了运行时/驱动就绪后
    /// 不必重启应用即可自愈；实现方需自带失败冷冻期，避免每次轮询重起进程。
    fn retry_warm_up(&self) {
        self.warm_up();
    }
    /// 实测生效的推理后端（如 "CUDA0"/"Vulkan1"/"CPU"），默认空。
    /// 仅 MOSS 有实证来源（spawn 时 info 的 [mt I] backend 行）；其余引擎
    /// 实测需拉起重依赖子进程，欢迎页探针不做。
    fn effective_backend(&self) -> String {
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
    /// 配置的推理设备：moss 为 ""(=auto，ggml 自动选优)/cpu/cuda/vulkan，funasr 为 cpu/cuda。
    /// 展示的是配置值而非实测后端——实测需拉起子进程，欢迎页探针只做存在性检查。
    pub device: String,
    /// 实测生效后端（仅 moss 有值，来自 spawn 时 `info` 的 `[mt I] backend:` 行，
    /// 如 "CUDA0"/"Vulkan1"/"CPU"）。与 device 不同：这是硬判据——device=vulkan
    /// 时 ggml 静默降级后实测可能仍是 CUDA0。None = 未实证（引擎未配置/探测失败）。
    pub effective_backend: Option<String>,
}

/// 全局 ASR 管理器 —— 同时持有 funasr / moss 两个引擎（各自按配置就绪），
/// 由前端配置面板在每次运行时选择。provider 以 Arc 共享、无互斥锁：
/// transcribe 阻塞调用与 cancel（取消/超时/退出钩子）需要并发，锁会把
/// 取消拖到转写结束（失效）。这里的 `RwLock` 只护住"换 provider 的那一瞬"
/// （见 `reload_engine`）：取用时先克隆 Arc、立刻放锁，转写期间不会持锁。
pub struct AsrManager {
    /// 当前生效的配置快照：切换设备后整体替换，因此只能克隆、不能借出引用
    config: RwLock<RuntimeConfig>,
    runtime_dir: PathBuf,
    funasr: RwLock<Arc<dyn AsrProvider>>,
    moss: RwLock<Arc<dyn AsrProvider>>,
    /// 转写进行中标志（test-and-set）：Arc 去锁后不再天然串行转写，
    /// 该标志是后端兜底，防并发转写互相覆盖 active_child / 互相清理
    transcribing: AtomicBool,
}

/// 按配置构造 funasr provider：三路径字段任一为空即视为未配置（None 占位）
fn build_funasr(config: &RuntimeConfig, runtime_dir: &PathBuf) -> Box<dyn AsrProvider> {
    if config.funasr_worker.is_empty()
        || config.funasr_deps.is_empty()
        || config.funasr_model_dir.is_empty()
    {
        return Box::new(AsrNoneProvider);
    }
    create_asr_provider(AsrProviderKind::FunAsr, config, runtime_dir)
}

/// 按配置构造 moss provider。
///
/// **空字段不等于"未配置"**：一键安装（欢迎页自检卡）只解包文件、不写 `config.json`，
/// 所以"装完了、配置还没生成"是正常状态。路径统一由 `moss_paths()` 解析（空字段回退
/// 安装器约定的标准布局，见 `:52` 的注释）；真的没装时 `MossProvider::spawn` 会带着
/// 具体路径报错，前端因此能显示"哪个文件不在"而不是一句空白的"未就绪"。
fn build_moss(config: &RuntimeConfig, runtime_dir: &PathBuf) -> Box<dyn AsrProvider> {
    create_asr_provider(AsrProviderKind::Moss, config, runtime_dir)
}

impl AsrManager {
    pub fn new(config: RuntimeConfig, runtime_dir: PathBuf) -> Self {
        // 双引擎并存：各自按路径字段是否齐全决定是否真的拉起 provider
        let funasr = build_funasr(&config, &runtime_dir);
        let moss = build_moss(&config, &runtime_dir);
        let funasr: Arc<dyn AsrProvider> = Arc::from(funasr);
        let moss: Arc<dyn AsrProvider> = Arc::from(moss);
        // 预热放到后台：MOSS 的 `info` 要加载几百 MB GGUF 元数据，放在构造里会在
        // setup 阶段冻住主线程（窗口出来一片死寂、连拖动都卡）。这里起一个线程跑
        // warm_up()，命令与转写各自另有兜底，就绪状态照旧如实上报。
        {
            let providers = vec![Arc::clone(&funasr), Arc::clone(&moss)];
            std::thread::spawn(move || {
                for p in providers {
                    p.warm_up();
                }
            });
        }
        Self {
            config: RwLock::new(config),
            runtime_dir,
            funasr: RwLock::new(funasr),
            moss: RwLock::new(moss),
            transcribing: AtomicBool::new(false),
        }
    }

    /// 当前生效的配置快照（克隆）
    pub fn config(&self) -> RuntimeConfig {
        self.read_config()
    }

    /// 读配置快照；锁中毒时取回内部值——一次 panic 不该让整个管理器永久不可用
    fn read_config(&self) -> RuntimeConfig {
        match self.config.read() {
            Ok(g) => g.clone(),
            Err(e) => e.into_inner().clone(),
        }
    }

    /// 取指定引擎的 provider（克隆 Arc 后立即放锁，转写期间不持锁）
    fn provider_of(&self, key: &str) -> Arc<dyn AsrProvider> {
        let slot = if key == "moss" { &self.moss } else { &self.funasr };
        match slot.read() {
            Ok(g) => Arc::clone(&g),
            Err(e) => Arc::clone(&e.into_inner()),
        }
    }

    /// 重读盘上配置并重建**单个**引擎的 provider（切换设备后立即生效）。
    /// 另一个引擎不动：FunASR worker 拉起成本高，改 MOSS 设备没理由重启它。
    /// 返回面向用户的一句话（成功=已重新加载 / 成功但不可用=原因）。
    pub fn reload_engine(&self, key: &str) -> Result<String, String> {
        if key != "moss" && key != "funasr" {
            return Err(format!("未知 ASR 引擎: {key}"));
        }
        // 换 provider 会丢弃正在跑的子进程，转写期间直接拒绝（配置已落盘，重启后仍生效）
        if self.transcribing.load(Ordering::SeqCst) {
            return Err("转写进行中，完成后再切换".into());
        }
        let config = RuntimeConfig::load(&self.runtime_dir)?;
        config.validate(&self.runtime_dir)?;
        let provider: Box<dyn AsrProvider> = if key == "moss" {
            build_moss(&config, &self.runtime_dir)
        } else {
            build_funasr(&config, &self.runtime_dir)
        };
        let (name, ready, message) = (
            provider.name().to_string(),
            provider.is_ready(),
            provider.describe().to_string(),
        );
        let slot = if key == "moss" { &self.moss } else { &self.funasr };
        match slot.write() {
            Ok(mut g) => *g = Arc::from(provider),
            Err(e) => *e.into_inner() = Arc::from(provider),
        }
        match self.config.write() {
            Ok(mut g) => *g = config,
            Err(e) => *e.into_inner() = config,
        }
        Ok(if ready {
            format!("{name} 已重新加载")
        } else {
            format!("{name} 已重新加载但不可用：{message}")
        })
    }

    pub fn runtime_dir(&self) -> &PathBuf {
        &self.runtime_dir
    }

    /// 访问指定引擎的 provider（无锁：provider 内部自带同步，transcribe 与 cancel 可并发）
    pub fn with_engine<R>(&self, key: &str, f: impl FnOnce(&dyn AsrProvider) -> R) -> R {
        let provider = self.provider_of(key);
        f(&*provider)
    }

    /// 两引擎状态列表（供 check_asr_engines 命令展示面板可用性）
    pub fn engines(&self) -> Vec<AsrEngineStatus> {
        let config = self.read_config();
        ["funasr", "moss"]
            .iter()
            .map(|key| {
                let (ready, message, effective) = self.with_engine(key, |p| {
                    (p.is_ready(), p.describe(), p.effective_backend())
                });
                // 设备值直接取配置：welcome 页展示"当前配置的 moss device"，
                // 空 = auto（ggml 自动选优，实测偏好独显），由前端转成可读文案
                let device = match *key {
                    "moss" => config.moss_device.clone(),
                    _ => config.funasr_device.clone(),
                };
                // 未就绪就让引擎自己在后台再探一次（带失败冷冻期）：
                // 装完运行时/驱动就绪后，前端每次刷新状态就能自愈，不必重启应用
                if !ready {
                    let provider = self.provider_of(key);
                    std::thread::spawn(move || provider.retry_warm_up());
                }
                AsrEngineStatus {
                    engine: (*key).into(),
                    ready,
                    message,
                    device,
                    // 实测后端：moss 来自 spawn 时的 L2 实证；空则不展示
                    effective_backend: if effective.is_empty() { None } else { Some(effective) },
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
        self.provider_of("funasr").cancel();
        self.provider_of("moss").cancel();
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

// ─── 向量融合运行时（T4/R2）───────────────────────────────

/// 全局向量融合作管理器 —— 持有配置（不做 provider 抽象：只有 worker 一种实现）
pub struct EmbedManager {
    config: RuntimeConfig,
    runtime_dir: PathBuf,
}

impl EmbedManager {
    /// 构造。**不做就绪探测**（不 spawn python：worker 加载模型需秒级，
    /// 可加载性由真正融合时验证并返回具体错误）。
    pub fn new(config: RuntimeConfig, runtime_dir: PathBuf) -> Self {
        Self {
            config,
            runtime_dir,
        }
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn runtime_dir(&self) -> &PathBuf {
        &self.runtime_dir
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

// ─── 向量融合 Tauri 命令 ─────────────────────────────────

/// 向量融合运行时状态（就绪 = 四个 embed_* 配置齐 + worker 脚本真实存在）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedRuntimeStatus {
    pub ready: bool,
    pub message: String,
}

impl EmbedManager {
    /// 就绪判定**只做静态检查**（关键路径都真实存在），不 spawn python——
    /// 模型可加载性由真正融合时验证并给出具体错误（与 FunAsr/Llm 就绪探测同哲学）。
    ///
    /// 路径一律走 `embed_paths()`：一键安装只解包、不写 `config.json`，空字段必须回退到
    /// 安装器约定的标准布局（与 `moss_paths` / `ocr_paths` 同一口径）。这里曾经判"配置
    /// 字符串为空 = 未配置"，于是真机上 worker、模型、依赖三样都在盘上，卡片仍报
    /// "缺 embed_worker / embed_model / embed_tokenizer / embed_deps_dir"。
    pub fn status(&self) -> EmbedRuntimeStatus {
        let (python, worker, deps, model, tokenizer) =
            embed_paths(&self.config, &self.runtime_dir);
        // 逐项检查而不是只看配置字符串：报错要能指出"到底哪个文件不在"
        for (label, path, want_dir) in [
            ("Python 解释器", &python, false),
            ("fuse worker 脚本", &worker, false),
            ("向量依赖目录", &deps, true),
            ("嵌入模型", &model, false),
            ("tokenizer", &tokenizer, false),
        ] {
            let exists = if want_dir { path.is_dir() } else { path.is_file() };
            if !exists {
                return EmbedRuntimeStatus {
                    ready: false,
                    message: format!(
                        "{label}不存在: {}（点「安装运行时」可补齐，或先跑 scripts/bootstrap_embed.ps1）",
                        path.display()
                    ),
                };
            }
        }
        EmbedRuntimeStatus {
            ready: true,
            message: "向量融合运行时就绪".into(),
        }
    }
}

/// Tauri 命令：探测向量融合运行环境是否就绪（供 FuseView 就绪卡）
#[tauri::command]
pub fn check_embed_runtime(state: tauri::State<'_, EmbedManager>) -> EmbedRuntimeStatus {
    state.status()
}
