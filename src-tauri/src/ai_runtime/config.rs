// ─── AI Runtime 运行配置 ──────────────────────────────────
// runtime/ 目录存放运行环境相关的非代码资源：
//   runtime/config.json          → 本文件持久化
//   runtime/worker/ocr_worker.py → 2.2 的 Python OCR 桥接脚本
//   runtime/deps                 → pip install --target 安装的依赖（内嵌解释器安装，cp312）
//   runtime/models/paddleocr     → PP-OCR 模型文件
//
// runtime 目录解析顺序：
//   1. 环境变量 GSA_RUNTIME_DIR（须含 config.json 才被接受）
//   2. 从可执行文件所在目录逐级向上，找含 runtime/config.json 的目录（开发期=仓库根）
//   3. 兜底：应用数据目录下的 runtime

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

/// 运行时配置 —— 描述 Python/依赖/模型所在位置
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Python 可执行文件路径：相对 runtime（如 "python/python.exe"，机器无关）或绝对路径（老配置）
    pub python_path: String,
    /// OCR worker 脚本路径（相对 runtime 目录，如 worker/ocr_worker.py）
    pub worker_script: String,
    /// pip install --target 安装的依赖目录（相对 runtime，如 deps）
    pub deps_dir: String,
    /// PP-OCR 模型目录（相对 runtime，如 models/paddleocr）
    pub model_dir: String,
    /// 识别语言，默认 "ch"
    pub language: String,
    /// OCR 模型档位："mobile"（快，默认）| "server"（慢，更准）
    #[serde(default = "default_ocr_model")]
    pub ocr_model: String,
    /// OCR 推理设备：空 = CPU/自动（默认）| "gpu:0" 等（走 CUDA 版 paddle，需 runtime/deps_gpu）。
    /// 环境变量 GSA_OCR_DEVICE 优先于此字段（开发期临时覆盖用）
    #[serde(default)]
    pub ocr_device: String,
    /// 开发期调试：帧输出到仓库根 temp/ 并打印各环节日志（默认关闭；
    /// 调试时在 runtime/config.json 手动设 true，普通运行写入系统临时目录并自动清理）
    #[serde(default = "default_dev_debug")]
    pub dev_debug: bool,
    /// MOSS 推理可执行文件：相对 runtime（如 "bin/moss-transcribe.exe"，机器无关）或绝对路径；空 = 未配置
    #[serde(default)]
    pub moss_binary: String,
    /// MOSS GGUF 模型文件（相对 runtime，如 "models/moss/moss-transcribe-q5_k.gguf"）；空 = 未配置
    #[serde(default)]
    pub moss_model: String,
    /// MOSS 推理线程数：0 = 不设置（CLI 默认全核）。实测 i7-14650HX（24 逻辑核）上全核最快，
    /// 文档建议的 8 线程反而慢 ~72%（解码带宽受限，甜点依机器而异）
    #[serde(default)]
    pub moss_threads: u32,
    /// MOSS 推理后端设备：空 = 不设置（ggml 自动选最优：有 CUDA/Vulkan DLL 即 GPU，否则 CPU）
    /// | "cuda" | "cpu" | "vulkan"。由 build_moss.ps1 -Backend 构建对应 DLL 后生效
    #[serde(default)]
    pub moss_device: String,
    /// MOSS 转写超时（分钟）：0 = 不限。防御模型/音频异常导致的卡死，
    /// 超时后终止子进程并报错（正常长音频按需调大）
    #[serde(default)]
    pub moss_timeout_minutes: u32,
    /// LLM 推理可执行文件（llama-cli）：相对 runtime（如 "bin/llama-cli.exe"，机器无关）或绝对路径；空 = 未配置
    #[serde(default)]
    pub llm_binary: String,
    /// Qwen GGUF 模型文件（相对 runtime，如 "models/qwen/Qwen3-4B-Q4_K_M.gguf"）；空 = 未配置
    #[serde(default)]
    pub llm_model: String,
    /// LLM 推理线程数：0 = 不设置（llama-cli 默认全核），沿用 MOSS 的实测结论
    #[serde(default)]
    pub llm_threads: u32,
    /// ASR provider 选择："moss" | "funasr" | 空（自动：funasr 配置存在则 funasr，否则 moss）
    #[serde(default)]
    pub asr_provider: String,
    /// FunASR worker 脚本路径（相对 runtime，如 worker/funasr_worker.py）；空 = 未配置
    #[serde(default)]
    pub funasr_worker: String,
    /// FunASR 依赖目录（相对 runtime，如 deps_funasr，pip install --target 安装）
    #[serde(default)]
    pub funasr_deps: String,
    /// FunASR 模型缓存目录（相对 runtime，如 models/funasr，MODELSCOPE_CACHE 指向）
    #[serde(default)]
    pub funasr_model_dir: String,
    /// FunASR 推理设备："cuda"（默认，不可用时 worker 自动回退 cpu）| "cpu"
    #[serde(default = "default_funasr_device")]
    pub funasr_device: String,
    /// FunASR 识别语言：空 = 自动检测（默认）。Fun-ASR-Nano-2512 支持 中文/英文/日文
    #[serde(default = "default_funasr_language")]
    pub funasr_language: String,
    /// FunASR 转写超时（分钟）：0 = 不限。含模型加载时间（约 20-40s）
    #[serde(default)]
    pub funasr_timeout_minutes: u32,
    /// 融合 worker 脚本路径（相对 runtime，如 worker/embed_worker.py）；空 = 未配置向量融合
    #[serde(default)]
    pub embed_worker: String,
    /// 融合嵌入模型 ONNX 路径（相对 runtime，如 models/embed/multilingual-e5-small/model_qint8_avx512_vnni.onnx）
    #[serde(default)]
    pub embed_model: String,
    /// 融合 E5 tokenizer.json 路径（相对 runtime）
    #[serde(default)]
    pub embed_tokenizer: String,
    /// 融合嵌入依赖目录（相对 runtime，如 deps_embed，pip --target 安装，作为 PYTHONPATH）
    #[serde(default)]
    pub embed_deps_dir: String,
    /// 下载源策略："auto"（默认，清单顺序：ESA 镜像 → GitHub）| "mirror"（强制走镜像前缀）
    /// | "direct"（只用清单里的直链）。只在安装/更新运行时时生效
    #[serde(default = "default_download_source")]
    pub download_source: String,
    /// 分段下载的段大小（MB）：0 = 关闭分段（单流）。默认 64；镜像对 Range 的支持是
    /// "整包重取再切片"，所以脚本对镜像前缀只做单流兜底，这个值只影响直链/CDN 源
    #[serde(default = "default_segment_mb")]
    pub segment_mb: u32,
    /// 首启准备页是否自动开跑安装（默认 true）。关闭后准备页只列清单、等用户点「开始准备」
    #[serde(default = "default_setup_auto")]
    pub setup_auto: bool,
}

fn default_download_source() -> String {
    "auto".into()
}

fn default_segment_mb() -> u32 {
    64
}

fn default_setup_auto() -> bool {
    true
}

fn default_ocr_model() -> String {
    "mobile".into()
}

fn default_dev_debug() -> bool {
    false
}

fn default_funasr_device() -> String {
    "cuda".into()
}

fn default_funasr_language() -> String {
    // 空 = auto：外网游戏切片英/日居多，硬编码"中文"会让 Nano 的 prompt 语言提示错误
    String::new()
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            // 标准布局里的解释器路径。历史默认值 "python" 指向的是目录，
            // 校验与运行期都要多一层回退，容易踩坑（真机已踩过）。
            python_path: super::OCR_PYTHON_REL.into(),
            worker_script: String::new(),
            deps_dir: String::new(),
            model_dir: String::new(),
            language: "ch".into(),
            ocr_model: default_ocr_model(),
        ocr_device: String::new(),
            dev_debug: default_dev_debug(),
            moss_binary: String::new(),
            moss_model: String::new(),
            moss_threads: 0,
            moss_device: String::new(),
            moss_timeout_minutes: 0,
            llm_binary: String::new(),
            llm_model: String::new(),
            llm_threads: 0,
            asr_provider: String::new(),
            funasr_worker: String::new(),
            funasr_deps: String::new(),
            funasr_model_dir: String::new(),
            funasr_device: default_funasr_device(),
            funasr_language: default_funasr_language(),
            funasr_timeout_minutes: 0,
            embed_worker: String::new(),
            embed_model: String::new(),
            embed_tokenizer: String::new(),
            embed_deps_dir: String::new(),
            download_source: default_download_source(),
            segment_mb: default_segment_mb(),
            setup_auto: default_setup_auto(),
        }
    }
}

/// 从可执行文件向上查找含 runtime/config.json 的目录
fn runtime_dir_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    loop {
        let candidate = dir.join("runtime");
        if candidate.join("config.json").exists() {
            return Some(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
}

/// 运行时目录指针文件名（位于 app_data_dir 下，不在 runtime 内部）
///
/// 之所以要独立文件：设置里把 runtime 整体搬到别的盘之后，原路径就没东西可读了，
/// 指针必须留在"永远存在"的应用数据目录里。文件内容见 [`RuntimeLocation`]。
pub const LOCATION_FILE: &str = "runtime-location.json";

/// runtime 目录位置记录
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuntimeLocation {
    /// 当前 runtime 目录（绝对路径）
    pub runtime_dir: String,
    /// 搬家前的旧目录（供"验证无误后一键删除"用；删完清空）
    #[serde(default)]
    pub previous: Option<String>,
    /// 最近一次搬家的时间（Unix 秒，仅供展示）
    #[serde(default)]
    pub moved_at: Option<u64>,
}

/// 读指针文件，**不校验目录是否存在**（设置页要用它提示"指针指向的目录没了"）
fn read_location_raw(base: &Path) -> Option<RuntimeLocation> {
    let path = base.join(LOCATION_FILE);
    let text = fs::read_to_string(&path).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match serde_json::from_str::<RuntimeLocation>(text) {
        Ok(loc) if !loc.runtime_dir.trim().is_empty() => Some(loc),
        Ok(_) => None,
        Err(e) => {
            eprintln!("[ai_runtime] 解析 {} 失败: {e}", path.display());
            None
        }
    }
}

/// 读指针文件；目录不存在时返回 None 并告警（外接盘掉线就属于这种）
fn read_location(base: &Path) -> Option<RuntimeLocation> {
    let loc = read_location_raw(base)?;
    let dir = PathBuf::from(&loc.runtime_dir);
    if dir.is_dir() {
        Some(loc)
    } else {
        eprintln!(
            "[ai_runtime] runtime 指针指向的目录不存在（外接盘未挂载？）: {}",
            dir.display()
        );
        None
    }
}

/// 写指针文件（原子写）
pub fn write_location(base: &Path, loc: &RuntimeLocation) -> Result<(), String> {
    fs::create_dir_all(base).map_err(|e| format!("无法创建应用数据目录: {e}"))?;
    let json = serde_json::to_string_pretty(loc).map_err(|e| format!("指针序列化失败: {e}"))?;
    let tmp = base.join(format!("{LOCATION_FILE}.tmp"));
    fs::write(&tmp, json).map_err(|e| format!("无法写入指针临时文件: {e}"))?;
    fs::rename(&tmp, base.join(LOCATION_FILE)).map_err(|e| format!("无法落盘指针: {e}"))
}

/// 解析 runtime 目录（指针文件 → 环境变量 → 向上查找 → 兜底 app_data/runtime）
///
/// 参数是**应用数据目录**（不是 runtime 目录本身）：指针文件放在那里，
/// 兜底时再拼 `runtime`。
pub fn resolve_runtime_dir(app_data: Option<PathBuf>) -> PathBuf {
    // 1. 设置里搬过家：指针优先（目录存在才采纳，见 read_location 的告警）
    if let Some(base) = &app_data {
        if let Some(loc) = read_location(base) {
            return PathBuf::from(loc.runtime_dir);
        }
    }
    // 2. 环境变量指定的目录必须真的含 config.json，否则视为误配置并告警
    if let Ok(env_dir) = std::env::var("GSA_RUNTIME_DIR") {
        let p = PathBuf::from(env_dir);
        if p.join("config.json").exists() {
            return p;
        }
        eprintln!(
            "[ai_runtime] GSA_RUNTIME_DIR 指向的目录缺少 config.json: {}",
            p.display()
        );
    }
    if let Some(d) = runtime_dir_from_exe() {
        return d;
    }
    app_data
        .map(|d| d.join("runtime"))
        .unwrap_or_else(|| PathBuf::from("runtime"))
}

/// 词法规范化路径：解析 "." 与 ".."，不访问文件系统
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 判断 path 是否位于 base 目录内（词法判断，考虑 ".."）
fn is_within(path: &Path, base: &Path) -> bool {
    normalize_lexical(path).starts_with(normalize_lexical(base))
}

impl RuntimeConfig {
    /// 从 runtime 目录读取 config.json。
    /// 文件缺失或损坏时返回 Err（携带原因），由调用方决定告警/回退。
    pub fn load(runtime_dir: &Path) -> Result<Self, String> {
        let path = runtime_dir.join("config.json");
        let json = fs::read_to_string(&path).map_err(|e| {
            format!("读取 {} 失败: {}", path.display(), e)
        })?;
        // 兼容带 UTF-8 BOM 的配置（如某些编辑器/脚本写出的）
        let json = json.strip_prefix('\u{feff}').unwrap_or(&json);
        serde_json::from_str::<RuntimeConfig>(json)
            .map_err(|e| format!("解析 {} 失败: {}", path.display(), e))
    }

    /// 写入 config.json（原子写：先写临时文件再 rename，避免中途写坏导致配置丢失）
    pub fn save(&self, runtime_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(runtime_dir).map_err(|e| format!("无法创建 runtime 目录: {}", e))?;
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("配置序列化失败: {}", e))?;
        let tmp = runtime_dir.join("config.json.tmp");
        fs::write(&tmp, &json).map_err(|e| format!("无法写入临时配置: {}", e))?;
        fs::rename(&tmp, runtime_dir.join("config.json"))
            .map_err(|e| format!("无法落盘配置: {}", e))
    }

    /// 读配置；**文件不存在时按默认值起步**。
    /// 一键安装只解包、不写 config.json，此时"保存设置"必须能从默认值开始，
    /// 而不是报"读取失败"（真机 2026-11-02 就是这样：面板里改任何一项都失败）。
    /// 文件存在但解析失败仍然报错——那是坏配置，不能静默用默认值覆盖。
    fn load_or_default(runtime_dir: &Path) -> Result<RuntimeConfig, String> {
        if !runtime_dir.join("config.json").is_file() {
            return Ok(RuntimeConfig::default());
        }
        RuntimeConfig::load(runtime_dir)
    }

    /// 校验各路径的合法性：
    /// - worker_script / deps_dir / model_dir / moss_binary / moss_model / llm_binary / llm_model
    ///   / funasr_worker / funasr_deps / funasr_model_dir
    ///   / embed_worker / embed_model / embed_tokenizer / embed_deps_dir 必须解析在 runtime 目录内（防路径穿越）
    /// - python_path 解析后（相对则按 runtime 目录解析）必须存在
    /// - moss_binary / llm_binary 非空时必须存在（缺失时对应运行时不可用，由 provider 报告原因）
    /// - moss_model / llm_model 不做存在性检查（模型缺失时由 provider 报告"未就绪"原因）
    /// - embed_* 四个字段**整组必配或整组为空**（半配状态视为配置错误，避免运行期才失败）
    pub fn validate(&self, runtime_dir: &Path) -> Result<(), String> {
        self.validate_schema(runtime_dir)?;
        self.validate_paths(runtime_dir)
    }

    /// 只校验"形状"：路径不越出 runtime、embed 整组、设备白名单。
    ///
    /// 设置面板改的都是**非路径**字段（设备 / 线程 / 下载源…），不该因为"运行时还没装、
    /// python.exe 不存在"就整个保存失败——真机踩过：搬家后 `python_path` 指向目录，
    /// 于是引擎组任何一项都存不进去。存在性检查留给 [`Self::validate_paths`]。
    pub fn validate_schema(&self, runtime_dir: &Path) -> Result<(), String> {
        // 可执行文件 / worker 脚本 / 依赖目录必须留在 runtime 内：它们与同目录的 DLL、
        // 包进来的 python 绑定，挪出去只会"文件在但跑不起来"。
        for (name, value) in [
            ("worker_script", &self.worker_script),
            ("deps_dir", &self.deps_dir),
            ("moss_binary", &self.moss_binary),
            ("llm_binary", &self.llm_binary),
            ("funasr_worker", &self.funasr_worker),
            ("funasr_deps", &self.funasr_deps),
            ("embed_worker", &self.embed_worker),
            ("embed_deps_dir", &self.embed_deps_dir),
        ] {
            let pb = PathBuf::from(value);
            let joined = if pb.is_absolute() {
                pb
            } else {
                runtime_dir.join(pb)
            };
            if !is_within(&joined, runtime_dir) {
                return Err(format!("{} 不能越出 runtime 目录: {}", name, value));
            }
        }

        // 纯只读数据（模型目录 / 模型文件）允许绝对路径指向 runtime 之外：C 盘紧张的机器
        // 可以把几百 MB 的模型搬到别的盘，config.json 里写绝对路径即可。相对路径仍必须
        // 落在 runtime 内（防路径穿越）。
        for (name, value) in [
            ("model_dir", &self.model_dir),
            ("moss_model", &self.moss_model),
            ("llm_model", &self.llm_model),
            ("funasr_model_dir", &self.funasr_model_dir),
            ("embed_model", &self.embed_model),
            ("embed_tokenizer", &self.embed_tokenizer),
        ] {
            let pb = PathBuf::from(value);
            if pb.is_absolute() {
                continue;
            }
            if !is_within(&runtime_dir.join(pb), runtime_dir) {
                return Err(format!("{} 不能越出 runtime 目录: {}", name, value));
            }
        }

        // embed_* 整组必配或整组为空（半配状态视为配置错误，避免运行期才失败）
        let embed_set: [(&str, &String); 4] = [
            ("embed_worker", &self.embed_worker),
            ("embed_model", &self.embed_model),
            ("embed_tokenizer", &self.embed_tokenizer),
            ("embed_deps_dir", &self.embed_deps_dir),
        ];
        let configured = embed_set.iter().filter(|(_, v)| !v.is_empty()).count();
        if configured != 0 && configured != embed_set.len() {
            let missing: Vec<&str> = embed_set
                .iter()
                .filter(|(_, v)| v.is_empty())
                .map(|(name, _)| *name)
                .collect();
            return Err(format!(
                "embed_* 配置必须整组同时存在（缺：{}）",
                missing.join(", ")
            ));
        }

        // moss_device 白名单：空（自动）/ cpu / cuda / vulkan
        if !matches!(
            self.moss_device.as_str(),
            "" | "cpu" | "cuda" | "vulkan"
        ) {
            return Err(format!(
                "moss_device 取值非法: {}（可选 空/cpu/cuda/vulkan）",
                self.moss_device
            ));
        }
        // funasr_device 白名单：空（历史默认，等价 cuda）/ cpu / cuda。
        // FunASR 走 PyTorch，没有 Vulkan 后端；worker 侧 cuda 不可用会自动回退
        // CPU（scripts/funasr_worker.py 的 pick_device），所以这里不拦"选了 cuda
        // 但本机没有 CUDA"——那是软回退，不是错误。
        if !matches!(self.funasr_device.as_str(), "" | "cpu" | "cuda") {
            return Err(format!(
                "funasr_device 取值非法: {}（可选 空/cpu/cuda）",
                self.funasr_device
            ));
        }
        // ocr_device 同样要有白名单：面板走 normalize_ocr_device，但手改 config.json
        // 会绕过它。这里放行「空 / cpu / gpu:N（npu/xpu 同理）」，其余判非法（2026-11-16 审查 P2-3）。
        normalize_ocr_device(&self.ocr_device).map_err(|e| {
            format!("{e}（配置文件里手写的值也会被挡，请改成合法设备串）")
        })?;
        Ok(())
    }

    /// 只做存在性检查（python.exe / moss / llm 二进制）。与 [`Self::validate_schema`] 分开，
    /// 是因为"运行时还没装完"不该阻塞设置保存。
    ///
    /// 判定口径与运行期一致：`ocr_paths` / `embed_paths` 的 `pick()` 在配置值解析不到时
    /// 会回退标准布局，所以这里也接受"配置值失效但标准布局存在"——否则老配置里
    /// `python_path: "python"`（指向目录）会让每次启动都报一次校验失败。
    pub fn validate_paths(&self, runtime_dir: &Path) -> Result<(), String> {
        let pp = PathBuf::from(&self.python_path);
        let joined = if pp.is_absolute() {
            pp
        } else {
            runtime_dir.join(&pp)
        };
        if !joined.is_file() && !runtime_dir.join(super::OCR_PYTHON_REL).is_file() {
            return Err(format!(
                "python_path 指向的文件不存在: {}（按 {} 解析）",
                self.python_path,
                joined.display()
            ));
        }
        ensure_binary_exists("moss_binary", &self.moss_binary, runtime_dir)?;
        ensure_binary_exists("llm_binary", &self.llm_binary, runtime_dir)?;
        Ok(())
    }
}

/// 可执行文件存在性检查：空值（未配置）直接通过；
/// 非空时按 runtime 目录解析（绝对路径原样使用），文件缺失返回 Err。
fn ensure_binary_exists(name: &str, value: &str, runtime_dir: &Path) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    let pb = PathBuf::from(value);
    let joined = if pb.is_absolute() {
        pb
    } else {
        runtime_dir.join(pb)
    };
    if !joined.is_file() {
        return Err(format!(
            "{} 指向的文件不存在: {}（按 {} 解析）",
            name,
            value,
            joined.display()
        ));
    }
    Ok(())
}

// ─── 推理设备设置（前端设备下拉）─────────────────────────

/// 设备设置结果（MOSS / FunASR 共用；写盘成功）
#[derive(Debug, Clone, Serialize)]
pub struct DeviceSaveResult {
    /// 实际写入的值（moss 的 "" = auto；funasr 恒为 cpu/cuda）
    pub device: String,
    /// 面向用户的结果说明（含是否已立即生效）
    pub message: String,
}

/// 规范化设备取值："" / "auto" → ""（auto）；只接受 ""/cpu/cuda/vulkan
pub fn normalize_moss_device(raw: &str) -> Result<String, String> {
    let v = raw.trim().to_ascii_lowercase();
    match v.as_str() {
        "" | "auto" => Ok(String::new()),
        "cpu" | "cuda" | "vulkan" => Ok(v),
        other => Err(format!(
            "moss_device 取值非法: {}（可选 空/auto/cpu/cuda/vulkan）",
            other
        )),
    }
}

/// 规范化 FunASR 设备："" / "auto" / "cuda" → "cuda"，"cpu" → "cpu"。
/// worker 侧 cuda 不可用会自动回退 CPU，所以没有第三种取值。
pub fn normalize_funasr_device(raw: &str) -> Result<String, String> {
    let v = raw.trim().to_ascii_lowercase();
    match v.as_str() {
        "" | "auto" | "cuda" => Ok("cuda".into()),
        "cpu" => Ok("cpu".into()),
        other => Err(format!(
            "funasr_device 取值非法: {}（可选 cpu/cuda）",
            other
        )),
    }
}

/// 让运行中的 ASR 管理器重读配置、重建单个引擎的 provider（切设备立即生效）。
/// 放后台线程：MOSS 重建要跑一次 `info` 实测，不能卡住命令线程。
async fn reload_asr_engine(app: &tauri::AppHandle, engine: &'static str) -> Result<String, String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || match handle.try_state::<super::AsrManager>() {
        Some(mgr) => mgr.reload_engine(engine),
        None => Err("ASR 管理器未托管".to_string()),
    })
    .await
    .unwrap_or_else(|e| Err(format!("重新加载失败: {e}")))
}

/// Tauri 命令：设置 MOSS 推理设备（写 runtime/config.json 并立即生效）。
///
/// 只改 moss_device 一个字段并原子落盘；随后重建 moss provider —— 转写进行中
/// 时重建会被拒绝（配置仍已落盘，重启后生效），避免中途换后端导致同一批素材
/// 前后不一致。
#[tauri::command]
pub async fn set_moss_device(
    app: tauri::AppHandle,
    device: String,
) -> Result<DeviceSaveResult, String> {
    let runtime_dir = resolve_runtime_dir(app.path().app_data_dir().ok());
    let normalized = normalize_moss_device(&device)?;
    let mut cfg = RuntimeConfig::load_or_default(&runtime_dir)?;
    cfg.moss_device = normalized.clone();
    cfg.save(&runtime_dir)?;
    let shown = if normalized.is_empty() {
        "空（auto）".to_string()
    } else {
        normalized.clone()
    };
    let message = match reload_asr_engine(&app, "moss").await {
        Ok(note) => format!("已保存 moss_device={}（{}）", shown, note),
        Err(e) => format!("已保存 moss_device={}（{}；重启后生效）", shown, e),
    };
    Ok(DeviceSaveResult {
        device: normalized,
        message,
    })
}

/// Tauri 命令：设置 FunASR 推理设备（写 runtime/config.json 并立即生效）。
/// 与 MOSS 不同：选中 cuda 但本机 CUDA 不可用时 worker 会自动回退 CPU，
/// 不构成错误，因此这里不做硬校验。
#[tauri::command]
pub async fn set_funasr_device(
    app: tauri::AppHandle,
    device: String,
) -> Result<DeviceSaveResult, String> {
    let runtime_dir = resolve_runtime_dir(app.path().app_data_dir().ok());
    let normalized = normalize_funasr_device(&device)?;
    let mut cfg = RuntimeConfig::load_or_default(&runtime_dir)?;
    cfg.funasr_device = normalized.clone();
    cfg.save(&runtime_dir)?;
    let message = match reload_asr_engine(&app, "funasr").await {
        Ok(note) => format!("已保存 funasr_device={}（{}）", normalized, note),
        Err(e) => format!("已保存 funasr_device={}（{}；重启后生效）", normalized, e),
    };
    Ok(DeviceSaveResult {
        device: normalized,
        message,
    })
}

// ─── 设置面板（第 19 项）────────────────────────────────────
//
// 设计取舍：
// - **配置落点不新增文件**：引擎/线程/超时/下载源都写 `runtime/config.json`（`RuntimeConfig`），
//   它已经是 Rust 侧唯一配置源，且 `validate()` 已在把关；
// - **只有"runtime 目录在哪"单独存指针**（`runtime-location.json` 在 app_data 下），
//   因为目录搬走之后原位置就没东西可读了；
// - 界面偏好（主题/缩放/默认策略/快捷键）属于前端 localStorage，不经过这里。

/// 设置面板读到的完整快照（一次 IPC 拿全，避免前端串行问十个命令）
#[derive(Clone, Debug, Serialize)]
pub struct SettingsSnapshot {
    /// 当前 runtime 目录（绝对路径，可能是搬过家的位置）
    pub runtime_dir: String,
    /// 应用数据目录（指针文件所在处）
    pub app_data_dir: String,
    /// 日志目录（`runtime/logs`）
    pub log_dir: String,
    /// runtime 所在盘可用字节（查不到为 null）
    pub free_bytes: Option<u64>,
    /// 搬家前留下的旧目录（供"验证无误后一键删除"；没有则为 null）
    pub previous_runtime_dir: Option<String>,
    /// 运行时清单是否找到
    pub manifest_found: bool,
    pub ocr_device: String,
    pub moss_threads: u32,
    pub moss_timeout_minutes: u32,
    pub dev_debug: bool,
    pub download_source: String,
    pub segment_mb: u32,
    pub setup_auto: bool,
    /// 这两个只读展示（改它们要走 set_moss_device / set_funasr_device，会重建 provider）
    pub moss_device: String,
    pub funasr_device: String,
    /// 指针指向的目录不存在时的告警（已回退到默认目录；面板要把它显出来）
    pub location_warning: Option<String>,
}

/// 局部更新：只改传了的字段
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub ocr_device: Option<String>,
    #[serde(default)]
    pub moss_threads: Option<u32>,
    #[serde(default)]
    pub moss_timeout_minutes: Option<u32>,
    #[serde(default)]
    pub dev_debug: Option<bool>,
    #[serde(default)]
    pub download_source: Option<String>,
    #[serde(default)]
    pub segment_mb: Option<u32>,
    #[serde(default)]
    pub setup_auto: Option<bool>,
}

/// 搬家结果
#[derive(Clone, Debug, Serialize)]
pub struct MoveResult {
    pub ok: bool,
    pub message: String,
    pub old_dir: String,
    pub new_dir: String,
    /// 恒为 true：正在运行的进程锁着 bin/*.dll 与 python.exe，只能重启生效
    pub needs_restart: bool,
}

/// 下载源归一化："auto"（默认）| "mirror" | "direct"
pub fn normalize_download_source(raw: &str) -> Result<String, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => Ok("auto".into()),
        "mirror" => Ok("mirror".into()),
        "direct" => Ok("direct".into()),
        other => Err(format!(
            "download_source 取值非法: {other}（可选 auto/mirror/direct）"
        )),
    }
}

/// 分段大小归一化：0（关闭分段）~ 1024 MB
pub fn normalize_segment_mb(raw: u32) -> Result<u32, String> {
    if raw > 1024 {
        return Err(format!("segment_mb 取值非法: {raw}（0–1024，0 表示关闭分段）"));
    }
    Ok(raw)
}

/// MOSS 推理线程数归一化：0（不设置 MTD_THREADS，CLI 默认全核）~ 256。
///
/// 与 `segment_mb`/`download_source` 同一套契约——**超范围就报错，不静默钳位**。
/// 之前这里是 `v.min(256)`，用户填 999 会被告知成功、实际存成 256，
/// 面板上看到的又还是 256，谁也说不清到底生没生效（2026-11-16 审查 P2-2）。
pub fn normalize_moss_threads(raw: u32) -> Result<u32, String> {
    if raw > 256 {
        return Err(format!(
            "moss_threads 取值非法: {raw}（0–256，0 表示用 CLI 默认全核）"
        ));
    }
    Ok(raw)
}

/// MOSS 单段超时归一化：0（不超时）~ 1440 分钟。契约同 `normalize_moss_threads`。
pub fn normalize_moss_timeout_minutes(raw: u32) -> Result<u32, String> {
    if raw > 24 * 60 {
        return Err(format!(
            "moss_timeout_minutes 取值非法: {raw}（0–1440 分钟，0 表示不超时）"
        ));
    }
    Ok(raw)
}

/// OCR 设备归一化：空（交给 paddlex 自动选）| "cpu" | "gpu:<n>"（也可 npu/xpu）。
///
/// 与 `moss_device`/`funasr_device` 一致做白名单校验：非法串以前会照存，
/// 直到 OCR 真跑起来才被 worker 静默退回 CPU，用户以为在用 GPU（2026-11-16 审查 P2-3）。
/// 这里只挡「明显不是设备串」的输入（如 `gpu:abc`、`cuda`、`gup:0`）；
/// `npu`/`xpu` 仍放行——worker 侧明确把它交给 paddle 自己判断，此处不越权臆测。
fn normalize_ocr_device(raw: &str) -> Result<String, String> {
    let v = raw.trim().to_ascii_lowercase();
    if v.is_empty() || v == "cpu" {
        return Ok(v);
    }
    let (kind, index) = match v.split_once(':') {
        Some((k, n)) => (k, Some(n)),
        None => (v.as_str(), None),
    };
    let known = matches!(kind, "gpu" | "npu" | "xpu");
    let index_ok = match index {
        None => known,
        Some(n) => known && n.parse::<u32>().is_ok(),
    };
    if !index_ok {
        return Err(format!(
            "ocr_device 取值非法: {raw}（可选空 / cpu / gpu:0 这类「设备:序号」，也支持 npu/xpu）"
        ));
    }
    Ok(v)
}

fn settings_snapshot(
    app: &tauri::AppHandle,
    runtime_dir: &Path,
    app_data: &Path,
    config: &RuntimeConfig,
) -> SettingsSnapshot {
    // 指针按"原样"读，才能区分「没搬过家」与「搬过家但目标目录没了」
    let raw_loc = read_location_raw(app_data);
    let location_warning = match &raw_loc {
        Some(l) if !PathBuf::from(&l.runtime_dir).is_dir() => Some(format!(
            "运行时目录指针指向的 {} 不存在，本次已回退到 {}。若是移动硬盘没插，插上后重启即可；\
             若是手动删掉了那个目录，请用「移动到…」重新指定。",
            l.runtime_dir,
            runtime_dir.display()
        )),
        _ => None,
    };
    let manifest_found =
        super::backend_probe::find_manifest(&super::backend_probe::manifest_candidates(app, runtime_dir))
            .is_some();
    SettingsSnapshot {
        runtime_dir: runtime_dir.display().to_string(),
        app_data_dir: app_data.display().to_string(),
        log_dir: runtime_dir.join("logs").display().to_string(),
        free_bytes: super::runtime_install::query_free_bytes(runtime_dir),
        previous_runtime_dir: raw_loc.and_then(|l| l.previous),
        manifest_found,
        ocr_device: config.ocr_device.clone(),
        moss_threads: config.moss_threads,
        moss_timeout_minutes: config.moss_timeout_minutes,
        dev_debug: config.dev_debug,
        download_source: if config.download_source.trim().is_empty() {
            "auto".into()
        } else {
            config.download_source.clone()
        },
        segment_mb: config.segment_mb,
        setup_auto: config.setup_auto,
        moss_device: config.moss_device.clone(),
        funasr_device: config.funasr_device.clone(),
        location_warning,
    }
}

/// Tauri 命令：设置面板快照
#[tauri::command]
pub async fn runtime_settings(app: tauri::AppHandle) -> Result<SettingsSnapshot, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取应用数据目录失败: {e}"))?;
    let app_for_snapshot = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime_dir = resolve_runtime_dir(Some(app_data.clone()));
        let config = RuntimeConfig::load_or_default(&runtime_dir).unwrap_or_default();
        settings_snapshot(&app_for_snapshot, &runtime_dir, &app_data, &config)
    })
    .await
    .map_err(|e| format!("读取设置失败: {e}"))
}

/// Tauri 命令：局部更新设置（写 runtime/config.json）
///
/// 只改传了的字段；改完先 `validate()` 再落盘（非法路径不留半份坏配置）。
/// `ocr_device` 变化会顺带重建 OCR provider（否则要重启才生效）。
#[tauri::command]
pub async fn update_runtime_settings(
    app: tauri::AppHandle,
    patch: SettingsPatch,
) -> Result<SettingsSnapshot, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取应用数据目录失败: {e}"))?;

    // 读配置 → 校验 → 写盘整段都在阻塞线程里：慢盘/杀软扫盘时 fs 调用能卡几百毫秒，
    // 放在 async 线程上会连带拖住同一 runtime 里的其它命令（`runtime_settings` 就是这么写的）。
    // 闭包顺带把 `ocr_device` 的旧值返出来，供下面判断是否要重建 OCR。
    // `app_data` 后面那半段（拼快照）还要用，所以只把副本 move 进写盘闭包
    let app_data_for_write = app_data.clone();
    let (config, runtime_dir, ocr_device_before) =
        tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
            let runtime_dir = resolve_runtime_dir(Some(app_data_for_write));
            let mut config = RuntimeConfig::load_or_default(&runtime_dir)?;
            let ocr_device_before = config.ocr_device.clone();

            if let Some(v) = patch.ocr_device {
                config.ocr_device = normalize_ocr_device(&v)?;
            }
            if let Some(v) = patch.moss_threads {
                config.moss_threads = normalize_moss_threads(v)?;
            }
            if let Some(v) = patch.moss_timeout_minutes {
                config.moss_timeout_minutes = normalize_moss_timeout_minutes(v)?;
            }
            if let Some(v) = patch.dev_debug {
                config.dev_debug = v;
            }
            if let Some(v) = patch.download_source {
                config.download_source = normalize_download_source(&v)?;
            }
            if let Some(v) = patch.segment_mb {
                config.segment_mb = normalize_segment_mb(v)?;
            }
            if let Some(v) = patch.setup_auto {
                config.setup_auto = v;
            }

            // 设置面板改的都是非路径字段：这里只校验"形状"，不因运行时未装完而拒绝保存
            // （真机踩过：搬家后 python_path 失效，引擎组任何一项都存不进去）。
            config.validate_schema(&runtime_dir)?;
            config.save(&runtime_dir)?;
            Ok((config, runtime_dir, ocr_device_before))
        })
        .await
        .map_err(|e| format!("保存设置失败: {e}"))??;

    // OCR 设备换了就重建 worker（它把设备当环境变量传给 python，热改只能重启子进程）
    if config.ocr_device != ocr_device_before {
        let handle = app.clone();
        let note = tauri::async_runtime::spawn_blocking(move || {
            match handle.try_state::<super::OcrManager>() {
                Some(mgr) => mgr.reload(),
                None => Err("OCR 管理器未托管".to_string()),
            }
        })
        .await
        .unwrap_or_else(|e| Err(format!("重新加载失败: {e}")));
        if let Err(e) = note {
            eprintln!("[ai_runtime] OCR 重建失败（下次启动生效）: {e}");
        }
    }

    // 快照里有 `find_manifest`（读清单）和 `query_free_bytes`（spawn 一次 powershell
    // 查 Get-PSDrive），同样不能占着 async 线程跑——这正是 `runtime_settings` 的做法。
    let app_for_snapshot = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        settings_snapshot(&app_for_snapshot, &runtime_dir, &app_data, &config)
    })
    .await
    .map_err(|e| format!("读取设置失败: {e}"))
}

/// 递归复制目录（跳过 `.downloads` 临时缓存：它随时可重建，复制它只是浪费时间）
///
/// 顺带把「拷过去的每个文件的相对路径 + 字节数」记进 `copied`，供搬家后的对账直接复用。
/// 原来对账要再整树走两遍（`rel_paths(&current)` + `rel_paths(target)`），而 runtime 动辄
/// 几 GB、上万个文件，这两遍纯属白花时间；顺便还能顺手比大小，而不只是比路径集合。
fn copy_dir_all(
    root: &Path,
    src: &Path,
    dst: &Path,
    copied: &mut Vec<(String, u64)>,
) -> Result<u64, String> {
    let mut total = 0u64;
    let entries = fs::read_dir(src).map_err(|e| format!("读取 {} 失败: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("遍历 {} 失败: {e}", src.display()))?;
        let name = entry.file_name();
        // `.downloads` 在任何层级都是下载器的临时缓存，一律不搬。
        // 对账清单也走同一个判断，否则就成了「复制按 A 跳过、对账按 B 要求」的自相矛盾。
        if name == ".downloads" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let meta = entry
            .metadata()
            .map_err(|e| format!("读取 {} 属性失败: {e}", from.display()))?;
        if meta.is_dir() {
            fs::create_dir_all(&to).map_err(|e| format!("创建 {} 失败: {e}", to.display()))?;
            total += copy_dir_all(root, &from, &to, copied)?;
        } else {
            let n = fs::copy(&from, &to)
                .map_err(|e| format!("复制 {} → {} 失败: {e}", from.display(), to.display()))?;
            let rel = from
                .strip_prefix(root)
                .unwrap_or(&from)
                .to_string_lossy()
                .to_string();
            copied.push((rel, n));
            total += n;
        }
    }
    Ok(total)
}

fn dir_entry_count(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|it| it.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

/// 收集目录下的全部相对路径 → 字节数（目录记 0，跳过任意层级的 `.downloads`），用于搬家后对账。
///
/// 这里**不允许**再静默跳过读不动的目录：以前写的是 `let Ok(entries) = fs::read_dir(dir)
/// else { return }`，于是「某个子目录没权限」会变成「对账通过」，而那批文件根本没搬过去，
/// 指针照样被改写 —— 正好踩中「半搬状态比不搬更糟」。读失败一律往上抛。
fn rel_paths(root: &Path) -> Result<std::collections::BTreeMap<String, u64>, String> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut std::collections::BTreeMap<String, u64>,
    ) -> Result<(), String> {
        let entries = fs::read_dir(dir).map_err(|e| format!("读取 {} 失败: {e}", dir.display()))?;
        for e in entries {
            let e = e.map_err(|e| format!("遍历 {} 失败: {e}", dir.display()))?;
            let name = e.file_name();
            if name == ".downloads" {
                continue;
            }
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string();
            let meta = e
                .metadata()
                .map_err(|err| format!("读取 {} 属性失败: {err}", p.display()))?;
            if meta.is_dir() {
                out.insert(rel, 0);
                walk(root, &p, out)?;
            } else {
                out.insert(rel, meta.len());
            }
        }
        Ok(())
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

/// 搬家前确认目标盘装得下：`need` 是本次要写的字节数，`free` 是该盘可用字节。
///
/// 留 10% 余量（日志、指针文件、NTFS 元数据都要占地方）。`free` 为 None 表示查不到
/// 可用空间（非 Windows 盘符路径、`Get-PSDrive` 失败等），此时**不拦**——宁可让复制过程
/// 自己失败并回滚，也不能因为探测失败就把路堵死。
fn ensure_enough_space(need: u64, free: Option<u64>) -> Result<(), String> {
    let Some(free) = free else { return Ok(()) };
    let margin = need / 10;
    if free < need.saturating_add(margin) {
        return Err(format!(
            "目标盘空间不足：需要 {} 复制 + 10% 余量，可用 {}（先清理磁盘或换个位置）",
            fmt_bytes(need),
            fmt_bytes(free)
        ));
    }
    Ok(())
}

/// 把字节数写成人类可读形式（GB/MB），用于报错信息
fn fmt_bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if n as f64 >= 1024.0 * MB {
        format!("{:.2} GB", n as f64 / (1024.0 * MB))
    } else {
        format!("{:.0} MB", n as f64 / MB)
    }
}

/// 判断某个目录「看起来像 runtime」：给「删除旧目录」兜底用。
///
/// 接受两种证据：目录里有 `config.json`（正常 runtime 必有），或目录名里带 `runtime`
/// （`D:\_GSA_RUNTIME` 这类用户自选的目标名，可能还没落 config.json 就被搬到别处了）。
/// 这当然挡不住所有误删，但足以挡住「手改指针文件指向 C:\Windows」这种事故。
fn looks_like_runtime(dir: &Path) -> bool {
    if dir.join("config.json").is_file() {
        return true;
    }
    dir.file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase().contains("runtime"))
        .unwrap_or(false)
}

/// 搬家实现（拆出来便于单测，不依赖 AppHandle）
pub(crate) fn move_runtime_dir_impl(app_data: &Path, target: &Path) -> Result<MoveResult, String> {
    let current = resolve_runtime_dir(Some(app_data.to_path_buf()));
    let cur_norm = normalize_lexical(&current);
    let tgt_norm = normalize_lexical(target);

    if !current.is_dir() {
        return Err(format!("当前 runtime 目录不存在: {}", current.display()));
    }
    if tgt_norm == cur_norm {
        return Err("目标目录与当前 runtime 目录相同".into());
    }
    if tgt_norm.starts_with(&cur_norm) {
        return Err("目标目录不能位于当前 runtime 目录内部".into());
    }
    if cur_norm.starts_with(&tgt_norm) {
        return Err("目标目录不能是当前 runtime 目录的上级".into());
    }
    if tgt_norm.parent().is_none() {
        return Err("不能直接搬到盘符根目录".into());
    }
    // 设计要求（docs/SETTINGS_PANEL_SCOPE.md §3 第 5 条）：禁止目标是正在使用的 app_data 子路径。
    // 理由不只是"防递归复制"——runtime 搬进 app_data 后，日志、`runtime-location.json` 指针、
    // `recent_projects.json` 会和它会互相干扰，且下次启动 `resolve_runtime_dir` 的兜底
    // (`app_data.join("runtime")`) 与指针可能指向同一棵树，删旧目录时风险放大。
    let app_norm = normalize_lexical(app_data);
    if tgt_norm.starts_with(&app_norm) {
        return Err(format!(
            "目标目录不能放在应用数据目录内部: {}（会与日志/指针文件互相干扰，请选盘上另一个位置）",
            app_data.display()
        ));
    }
    if target.is_dir() && dir_entry_count(target) > 0 {
        return Err(format!("目标目录非空: {}", target.display()));
    }
    // 搬家是要把几 GB 的 runtime 整份写进去：空间不够时提前报错，别等复制到一半
    // 才 ENOSPC（那时只能靠回滚收拾，还得让用户重来一遍）。这里的字节数用
    // `rel_paths` 现算——它和复制共用同一套跳过口径，算出来的就是要写的量。
    let need: u64 = rel_paths(&current)?.values().copied().sum();
    ensure_enough_space(need, super::runtime_install::query_free_bytes(target))?;

    // 记住这个目录是不是「本次自己新建」的：只有自己新建的才敢在失败时删掉。
    // 用户若指定了一个已存在的空目录，那是他的目录，失败时不能替人删。
    let created_target = !target.is_dir();
    fs::create_dir_all(target).map_err(|e| format!("创建目标目录失败: {e}"))?;

    // 复制或对账失败一律回滚（设计口径见 docs/SETTINGS_PANEL_SCOPE.md §3.4「失败回滚」、
    // docs/RUNTIME_BOOTSTRAP_CONTRACT.md「半搬状态比不搬更糟」）：不回滚会留下半个目标目录，
    // 而用户按提示「可重试」重来时，上面那条「目标目录非空」立刻把路堵死，只能去资源管理器手删。
    let rollback = |created: bool| {
        if created {
            let _ = fs::remove_dir_all(target);
            return;
        }
        // 目标目录是用户原有的空目录：目录本身留着（那是他的目录），只清掉本次拷进去的内容，
        // 否则同样会留下半边目录、重试撞「非空」。上面已确认它进门前是空的，清掉不会误删用户数据。
        if let Ok(entries) = fs::read_dir(target) {
            for p in entries.flatten().map(|e| e.path()) {
                let _ = if p.is_dir() {
                    fs::remove_dir_all(&p)
                } else {
                    fs::remove_file(&p)
                };
            }
        }
    };

    // 复制时顺手记下每个文件的相对路径与字节数，对账直接用它，不必再扫源树一遍
    let mut copied: Vec<(String, u64)> = Vec::new();
    let bytes = match copy_dir_all(&current, &current, target, &mut copied) {
        Ok(n) => n,
        Err(e) => {
            rollback(created_target);
            return Err(format!("{e}（已回滚，旧目录与指针未改动）"));
        }
    };
    // 复制完按「路径 + 大小」逐条对账，再写指针 —— 少一半文件、或拷了半个文件就搬过去，
    // 都比不搬更糟（用户会以为搬完了，回头把旧目录删了）
    let dst_paths = match rel_paths(target) {
        Ok(m) => m,
        Err(e) => {
            rollback(created_target);
            return Err(format!("{e}（已回滚，旧目录与指针未改动）"));
        }
    };
    for (rel, size) in &copied {
        match dst_paths.get(rel) {
            Some(got) if got == size => {}
            Some(got) => {
                rollback(created_target);
                return Err(format!(
                    "复制不完整：{rel} 大小不符（源 {size} 字节，目标 {got} 字节）（已回滚，旧目录与指针未改动）"
                ));
            }
            None => {
                rollback(created_target);
                return Err(format!(
                    "复制不完整：目标目录缺少 {rel}（已回滚，旧目录与指针未改动）"
                ));
            }
        }
    }

    let loc = RuntimeLocation {
        runtime_dir: target.display().to_string(),
        previous: Some(current.display().to_string()),
        moved_at: Some(now_epoch()),
    };
    write_location(app_data, &loc)?;

    Ok(MoveResult {
        ok: true,
        message: format!(
            "已复制 {:.2} GB 到 {}，重启应用后生效（旧目录保留，验证无误后可一键删除）",
            bytes as f64 / 1024.0 / 1024.0 / 1024.0,
            target.display()
        ),
        old_dir: current.display().to_string(),
        new_dir: target.display().to_string(),
        needs_restart: true,
    })
}

/// 删除旧目录实现
pub(crate) fn delete_old_runtime_dir_impl(app_data: &Path) -> Result<String, String> {
    // 这里要的是「指针文件里记了什么」，而不是「指针指向的新目录此刻在不在」：
    // 用 read_location 的话，新目录缺失（外接盘没插 / 被手动删）时会返回 None，
    // 明明指针文件好好的却报「没有搬家记录（指针文件缺失）」，清理旧目录这条路也永远走不到。
    // 2026-11-07 真机就是这么卡住的（D:\_GSA_RUNTIME 少打一个下划线，旧目录反而删不掉）。
    let mut loc =
        read_location_raw(app_data).ok_or_else(|| "没有搬家记录（指针文件缺失）".to_string())?;
    let previous = loc
        .previous
        .clone()
        .ok_or_else(|| "没有待删除的旧目录".to_string())?;
    let old = PathBuf::from(&previous);
    let current = resolve_runtime_dir(Some(app_data.to_path_buf()));
    if normalize_lexical(&old) == normalize_lexical(&current) {
        return Err("旧目录就是当前 runtime 目录，拒绝删除".into());
    }
    // 唯一的护栏不能只是「不等于当前目录」——指针文件是纯文本，手改成 `C:\Windows`
    // 就会把整盘删了。这里要求它「看起来像 runtime」：要么写着 config.json，
    // 要么目录名里带 runtime（搬家目标由用户自选，不保证有 config.json）。
    if old.exists() && !looks_like_runtime(&old) {
        return Err(format!(
            "{} 不像 runtime 目录（没有 config.json，目录名也不含 runtime），拒绝删除；\
             请手动确认后自行清理",
            old.display()
        ));
    }
    // 新目录不在时给一句人话，别让用户以为搬家成功过
    let new_dir_missing = !PathBuf::from(&loc.runtime_dir).is_dir();
    let hint = if new_dir_missing {
        "（注意：指针指向的新目录当前不存在，删完旧目录后只剩悬空指针）"
    } else {
        ""
    };
    if !old.exists() {
        loc.previous = None;
        write_location(app_data, &loc)?;
        return Ok(format!("旧目录已不存在：{previous}（已清理记录）{hint}"));
    }
    fs::remove_dir_all(&old).map_err(|e| {
        format!("删除 {previous} 失败: {e}（可能有文件正被占用，先退出所有转写/OCR 任务再试）")
    })?;
    loc.previous = None;
    write_location(app_data, &loc)?;
    Ok(format!("已删除旧目录：{previous}{hint}"))
}

/// Tauri 命令：把 runtime 目录整体搬到别处（重启生效）
#[tauri::command]
pub async fn move_runtime_dir(app: tauri::AppHandle, target: String) -> Result<MoveResult, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取应用数据目录失败: {e}"))?;
    let target = PathBuf::from(target.trim());
    if target.as_os_str().is_empty() {
        return Err("目标目录不能为空".into());
    }
    tauri::async_runtime::spawn_blocking(move || move_runtime_dir_impl(&app_data, &target))
        .await
        .map_err(|e| format!("搬家失败: {e}"))?
}

/// Tauri 命令：删除搬家前的旧目录（用户确认新目录验证无误后手动触发）
#[tauri::command]
pub async fn delete_old_runtime_dir(app: tauri::AppHandle) -> Result<String, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取应用数据目录失败: {e}"))?;
    tauri::async_runtime::spawn_blocking(move || delete_old_runtime_dir_impl(&app_data))
        .await
        .map_err(|e| format!("删除旧目录失败: {e}"))?
}

/// Tauri 命令：打开日志目录（装不上运行时，用户第一件事就是找 install.log）
#[tauri::command]
pub fn open_log_dir(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;

    let runtime_dir = resolve_runtime_dir(app.path().app_data_dir().ok());
    let log_dir = runtime_dir.join("logs");
    fs::create_dir_all(&log_dir).map_err(|e| format!("创建日志目录失败: {e}"))?;
    app.opener()
        .open_path(log_dir.display().to_string(), None::<&str>)
        .map_err(|e| format!("打开日志目录失败: {e}"))?;
    Ok(log_dir.display().to_string())
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gsa_{}_{}", name, std::process::id()))
    }

    #[test]
    fn test_config_roundtrip() {
        let dir = temp_dir("cfg_roundtrip");
        let cfg = RuntimeConfig {
            python_path: "C:/Python310/python.exe".into(),
            worker_script: "worker/ocr_worker.py".into(),
            deps_dir: "deps".into(),
            model_dir: "models/paddleocr".into(),
            language: "ch".into(),
            ocr_model: "mobile".into(),
        ocr_device: String::new(),
            dev_debug: true,
            moss_binary: "bin/moss-transcribe.exe".into(),
            moss_model: "models/moss/moss-transcribe-q5_k.gguf".into(),
            moss_threads: 8,
            moss_device: "cuda".into(),
            moss_timeout_minutes: 30,
            llm_binary: "bin/llama-cli.exe".into(),
            llm_model: "models/qwen/Qwen3-4B-Q4_K_M.gguf".into(),
            llm_threads: 4,
            asr_provider: "funasr".into(),
            funasr_worker: "worker/funasr_worker.py".into(),
            funasr_deps: "deps_funasr".into(),
            funasr_model_dir: "models/funasr".into(),
            funasr_device: "cpu".into(),
            funasr_language: "英文".into(),
            funasr_timeout_minutes: 45,
            embed_worker: "worker/embed_worker.py".into(),
            embed_model: "models/embed/multilingual-e5-small/model_qint8_avx512_vnni.onnx".into(),
            embed_tokenizer: "models/embed/multilingual-e5-small/tokenizer.json".into(),
            embed_deps_dir: "deps_embed".into(),
            download_source: "auto".into(),
            segment_mb: 64,
            setup_auto: true,
        };
        cfg.save(&dir).unwrap();

        let loaded = RuntimeConfig::load(&dir).unwrap();
        assert_eq!(loaded.python_path, cfg.python_path);
        assert_eq!(loaded.worker_script, cfg.worker_script);
        assert_eq!(loaded.deps_dir, cfg.deps_dir);
        assert_eq!(loaded.model_dir, cfg.model_dir);
        assert_eq!(loaded.language, cfg.language);
        assert_eq!(loaded.ocr_model, "mobile");
        assert!(loaded.dev_debug);
        assert_eq!(loaded.moss_binary, "bin/moss-transcribe.exe");
        assert_eq!(loaded.moss_model, "models/moss/moss-transcribe-q5_k.gguf");
        assert_eq!(loaded.moss_threads, 8);
        assert_eq!(loaded.moss_device, "cuda");
        assert_eq!(loaded.moss_timeout_minutes, 30);
        assert_eq!(loaded.llm_binary, "bin/llama-cli.exe");
        assert_eq!(loaded.llm_model, "models/qwen/Qwen3-4B-Q4_K_M.gguf");
        assert_eq!(loaded.llm_threads, 4);
        assert_eq!(loaded.asr_provider, "funasr");
        assert_eq!(loaded.funasr_worker, "worker/funasr_worker.py");
        assert_eq!(loaded.funasr_deps, "deps_funasr");
        assert_eq!(loaded.funasr_model_dir, "models/funasr");
        assert_eq!(loaded.funasr_device, "cpu");
        assert_eq!(loaded.funasr_language, "英文");
        assert_eq!(loaded.funasr_timeout_minutes, 45);
        assert_eq!(loaded.embed_worker, "worker/embed_worker.py");
        assert_eq!(
            loaded.embed_model,
            "models/embed/multilingual-e5-small/model_qint8_avx512_vnni.onnx"
        );
        assert_eq!(
            loaded.embed_tokenizer,
            "models/embed/multilingual-e5-small/tokenizer.json"
        );
        assert_eq!(loaded.embed_deps_dir, "deps_embed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_returns_err() {
        let dir = temp_dir("cfg_missing");
        assert!(RuntimeConfig::load(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_corrupted_returns_err() {
        let dir = temp_dir("cfg_corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{ not valid json ").unwrap();
        assert!(RuntimeConfig::load(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_paths_inside_runtime_ok() {
        let dir = temp_dir("cfg_valid");
        fs::create_dir_all(&dir).unwrap();
        // 相对 python_path 需解析到 runtime 内且文件存在
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        fs::write(dir.join("moss-transcribe.exe"), b"dummy").unwrap();
        fs::write(dir.join("llama-cli.exe"), b"dummy").unwrap();
        let cfg = RuntimeConfig {
            python_path: "python.exe".into(),
            worker_script: "worker/ocr_worker.py".into(),
            deps_dir: "deps".into(),
            model_dir: "models/paddleocr".into(),
            language: "ch".into(),
            ocr_model: "mobile".into(),
        ocr_device: String::new(),
            dev_debug: true,
            moss_binary: "moss-transcribe.exe".into(),
            moss_model: "models/moss/moss-transcribe-q5_k.gguf".into(),
            moss_threads: 8,
            moss_device: "cpu".into(),
            moss_timeout_minutes: 0,
            llm_binary: "llama-cli.exe".into(),
            llm_model: "models/qwen/Qwen3-4B-Q4_K_M.gguf".into(),
            llm_threads: 4,
            asr_provider: String::new(),
            funasr_worker: String::new(),
            funasr_deps: String::new(),
            funasr_model_dir: String::new(),
            funasr_device: "cuda".into(),
            funasr_language: String::new(), // 空 = auto 自动检测
            funasr_timeout_minutes: 0,
            embed_worker: String::new(),
            embed_model: String::new(),
            embed_tokenizer: String::new(),
            embed_deps_dir: String::new(),
            download_source: "auto".into(),
            segment_mb: 64,
            setup_auto: true,
        };
        assert!(cfg.validate(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_missing_python() {
        let dir = temp_dir("cfg_missing_py");
        fs::create_dir_all(&dir).unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python/python.exe".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_with_bom() {
        // 兼容带 UTF-8 BOM 的配置文件
        let dir = temp_dir("cfg_bom");
        fs::create_dir_all(&dir).unwrap();
        let json = r#"{"python_path":"python","worker_script":"","deps_dir":"","model_dir":"","language":"ch"}"#;
        let with_bom = format!("\u{feff}{}", json);
        fs::write(dir.join("config.json"), with_bom).unwrap();
        let cfg = RuntimeConfig::load(&dir).unwrap();
        assert_eq!(cfg.language, "ch");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_traversal() {
        let dir = temp_dir("cfg_traversal");
        let mut cfg = RuntimeConfig::default();
        cfg.worker_script = "../evil.py".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_legacy_config_without_moss_fields() {
        // 老配置（Phase 2）没有 moss 字段 → 应能加载且 moss 字段取默认值
        let dir = temp_dir("cfg_legacy");
        fs::create_dir_all(&dir).unwrap();
        let json = r#"{"python_path":"python","worker_script":"worker/ocr_worker.py","deps_dir":"deps","model_dir":"models/paddleocr","language":"ch","ocr_model":"mobile","dev_debug":true}"#;
        fs::write(dir.join("config.json"), json).unwrap();
        let cfg = RuntimeConfig::load(&dir).unwrap();
        assert_eq!(cfg.moss_binary, "");
        assert_eq!(cfg.moss_model, "");
        assert_eq!(cfg.moss_threads, 0);
        assert_eq!(cfg.moss_device, ""); // 老配置无此字段 → 空 = 自动选后端
        assert_eq!(cfg.llm_binary, "");
        assert_eq!(cfg.llm_model, "");
        assert_eq!(cfg.llm_threads, 0);
        assert_eq!(cfg.moss_timeout_minutes, 0);
        // funasr 字段（Phase 之后）也应取默认值
        assert_eq!(cfg.asr_provider, "");
        assert_eq!(cfg.funasr_worker, "");
        assert_eq!(cfg.funasr_deps, "");
        assert_eq!(cfg.funasr_model_dir, "");
        assert_eq!(cfg.funasr_device, "cuda");
        assert_eq!(cfg.funasr_language, ""); // 空 = auto 自动检测
        assert_eq!(cfg.funasr_timeout_minutes, 0);
        // embed 字段（向量融合，T4 之后）也应取默认值：空 = 未配置向量融合
        assert_eq!(cfg.embed_worker, "");
        assert_eq!(cfg.embed_model, "");
        assert_eq!(cfg.embed_tokenizer, "");
        assert_eq!(cfg.embed_deps_dir, "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_moss_traversal() {
        let dir = temp_dir("cfg_moss_traversal");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.moss_binary = "../evil.exe".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_embed_traversal() {
        let dir = temp_dir("cfg_embed_traversal");
        let mut cfg = RuntimeConfig::default();
        cfg.embed_worker = "worker/embed_worker.py".into();
        cfg.embed_model = "../evil.onnx".into();
        cfg.embed_tokenizer = "tokenizer.json".into();
        cfg.embed_deps_dir = "deps_embed".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_partial_embed_config() {
        // embed_* 四个字段整组必配或整组为空：只配两个是配置错误
        let dir = temp_dir("cfg_embed_partial");
        let mut cfg = RuntimeConfig::default();
        cfg.embed_worker = "worker/embed_worker.py".into();
        cfg.embed_deps_dir = "deps_embed".into();
        let err = cfg.validate(&dir).unwrap_err();
        assert!(err.contains("整组"), "错误信息应说明整组配置要求：{}", err);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_full_embed_config_ok_when_paths_in_runtime() {
        // 全组非空 + 都在 runtime 内 → 校验通过（不做存在性检查，由 worker 报告原因）
        let dir = temp_dir("cfg_embed_full");
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        cfg.embed_worker = "worker/embed_worker.py".into();
        cfg.embed_model = "models/embed/multilingual-e5-small/model_qint8_avx512_vnni.onnx".into();
        cfg.embed_tokenizer = "models/embed/multilingual-e5-small/tokenizer.json".into();
        cfg.embed_deps_dir = "deps_embed".into();
        assert!(cfg.validate(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_missing_moss_binary() {
        let dir = temp_dir("cfg_missing_moss");
        fs::create_dir_all(&dir).unwrap();
        // python 就绪，确保错误来自 moss_binary 分支
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.moss_binary = "bin/moss-transcribe.exe".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_moss_model_missing_ok() {
        // moss_model 不做存在性检查：模型缺失时由 provider 报告"未就绪"原因
        let dir = temp_dir("cfg_moss_model_missing");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        fs::write(dir.join("moss-transcribe.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.moss_binary = "moss-transcribe.exe".into();
        cfg.moss_model = "models/moss/missing.gguf".into();
        assert!(cfg.validate(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_empty_moss_binary_ok() {
        // moss 未配置（空）→ 合法，ASR 由 NoneProvider 兜底
        let dir = temp_dir("cfg_no_moss");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        assert!(cfg.validate(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_llm_traversal() {
        let dir = temp_dir("cfg_llm_traversal");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.llm_binary = "../evil.exe".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_missing_llm_binary() {
        let dir = temp_dir("cfg_missing_llm");
        fs::create_dir_all(&dir).unwrap();
        // python 就绪，确保错误来自 llm_binary 分支
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.llm_binary = "bin/llama-cli.exe".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_allows_absolute_model_outside_runtime() {
        // 模型是纯只读数据：C 盘紧张的机器可以把 gguf 放到别的盘，写绝对路径即可
        let dir = temp_dir("cfg_abs_model_outside");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin/moss-transcribe.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.moss_binary = "bin/moss-transcribe.exe".into();
        cfg.moss_model = "D:/models/moss/moss-transcribe-q5_k.gguf".into();
        assert!(cfg.validate(&dir).is_ok(), "{:?}", cfg.validate(&dir));

        // 但二进制不行：它要跟同目录的 ggml/cuda DLL 一起被加载
        cfg.moss_binary = "D:/tools/moss-transcribe.exe".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_schema_ignores_missing_python() {
        // 真机踩过：搬家后 config.json 是默认值（python_path: "python"），设置面板
        // 改任何一项都被 validate() 的存在性检查拦下。形状校验必须放行，
        // 存在性只归 validate_paths，且标准布局在位时也算通过。
        let dir = temp_dir("cfg_schema_ignores_python");
        fs::create_dir_all(dir.join("python")).unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python".into();
        assert!(cfg.validate_schema(&dir).is_ok());
        assert!(cfg.validate_paths(&dir).is_err(), "标准布局也没有 python 时应报错");
        assert!(cfg.validate(&dir).is_err());

        // 标准布局 python/python.exe 在位时，失效的配置值不应再让校验失败
        fs::write(dir.join("python/python.exe"), b"dummy").unwrap();
        assert!(cfg.validate_paths(&dir).is_ok(), "{:?}", cfg.validate_paths(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_llm_model_missing_ok() {
        // llm_model 不做存在性检查：模型缺失时由 provider 报告"未就绪"原因
        let dir = temp_dir("cfg_llm_model_missing");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        fs::write(dir.join("llama-cli.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.llm_binary = "llama-cli.exe".into();
        cfg.llm_model = "models/qwen/missing.gguf".into();
        assert!(cfg.validate(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_empty_llm_binary_ok() {
        // llm 未配置（空）→ 合法，LLM 由 NoneProvider 兜底
        let dir = temp_dir("cfg_no_llm");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        assert!(cfg.validate(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_rejects_bad_moss_device() {
        // moss_device 只允许 空/cpu/cuda/vulkan
        let dir = temp_dir("cfg_bad_moss_device");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.moss_device = "../evil".into();
        assert!(cfg.validate(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_validate_accepts_all_moss_devices() {
        let dir = temp_dir("cfg_ok_moss_devices");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        for device in ["", "cpu", "cuda", "vulkan"] {
            let mut cfg = RuntimeConfig::default();
            cfg.python_path = "python.exe".into();
            cfg.moss_device = device.into();
            assert!(cfg.validate(&dir).is_ok(), "moss_device={:?} 应通过校验", device);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_normalize_moss_device() {
        // auto 归一为空串（配置里存 ""），其余小写化并拒绝非法值
        assert_eq!(normalize_moss_device("").unwrap(), "");
        assert_eq!(normalize_moss_device("auto").unwrap(), "");
        assert_eq!(normalize_moss_device("AUTO").unwrap(), "");
        assert_eq!(normalize_moss_device(" CUDA ").unwrap(), "cuda");
        assert_eq!(normalize_moss_device("vulkan").unwrap(), "vulkan");
        assert_eq!(normalize_moss_device("cpu").unwrap(), "cpu");
        assert!(normalize_moss_device("gpu").is_err());
        assert!(normalize_moss_device("../evil").is_err());
    }

    #[test]
    fn test_normalize_funasr_device() {
        // funasr 只有 cpu/cuda 两种落点；空与 auto 都等价 cuda（worker 侧再软回退）
        assert_eq!(normalize_funasr_device("").unwrap(), "cuda");
        assert_eq!(normalize_funasr_device("auto").unwrap(), "cuda");
        assert_eq!(normalize_funasr_device(" CUDA ").unwrap(), "cuda");
        assert_eq!(normalize_funasr_device("cpu").unwrap(), "cpu");
        assert!(normalize_funasr_device("vulkan").is_err(), "FunASR 无 Vulkan 后端");
        assert!(normalize_funasr_device("../evil").is_err());
    }

    #[test]
    fn test_validate_rejects_bad_funasr_device() {
        let dir = temp_dir("cfg_bad_funasr_device");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("python.exe"), b"dummy").unwrap();
        let mut cfg = RuntimeConfig::default();
        cfg.python_path = "python.exe".into();
        cfg.funasr_device = "vulkan".into();
        let err = cfg.validate(&dir).unwrap_err();
        assert!(err.contains("funasr_device"), "错误应点名 funasr_device：{err}");
        // 旧配置的空串与两个合法值都放行
        for ok in ["", "cpu", "cuda"] {
            cfg.funasr_device = ok.into();
            assert!(cfg.validate(&dir).is_ok(), "funasr_device={ok:?} 应通过校验");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    // ── 设置面板：下载源 / 分段 / runtime 目录搬家 ──────────

    #[test]
    fn test_load_or_default_starts_from_default_when_missing() {
        // 目录名不能和 test_load_missing_returns_err 的 "cfg_missing" 撞车：
        // 两个测试并行跑，一边 create 一边 remove_dir_all 就会互相踩（实测偶发失败）。
        let dir = temp_dir("cfg_load_or_default");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 没有 config.json ⇒ 默认值，不报错（一键安装后就是这个状态）
        let cfg = RuntimeConfig::load_or_default(&dir).unwrap();
        assert_eq!(cfg.download_source, "auto");
        assert_eq!(cfg.segment_mb, 64);
        assert!(cfg.setup_auto);
        // 有文件但内容坏了 ⇒ 仍然报错，不能静默用默认值覆盖
        fs::write(dir.join("config.json"), b"{ not json").unwrap();
        assert!(RuntimeConfig::load_or_default(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_normalize_download_source_and_segment() {
        for (raw, want) in [("", "auto"), ("auto", "auto"), ("MIRROR", "mirror"), ("direct", "direct")] {
            assert_eq!(normalize_download_source(raw).unwrap(), want);
        }
        assert!(normalize_download_source("cdn").is_err());
        assert_eq!(normalize_segment_mb(0).unwrap(), 0);
        assert_eq!(normalize_segment_mb(64).unwrap(), 64);
        assert!(normalize_segment_mb(2048).is_err());
    }

    #[test]
    fn test_normalize_ocr_device_whitelist() {
        // P2-3：非法设备串以前照存，直到 OCR 跑起来才被 worker 静默退回 CPU
        for (raw, want) in [
            ("", ""),
            ("  CPU ", "cpu"),
            ("gpu:0", "gpu:0"),
            ("GPU:1", "gpu:1"),
            ("npu", "npu"),
            ("xpu:0", "xpu:0"),
        ] {
            assert_eq!(normalize_ocr_device(raw).unwrap(), want, "输入 {raw:?}");
        }
        for bad in ["gpu:abc", "cuda", "gup:0", "gpu:", "gpu:-1", "cpu:0"] {
            assert!(
                normalize_ocr_device(bad).is_err(),
                "{bad:?} 应被白名单挡下（错误串要能告诉用户可选值）"
            );
        }
    }

    #[test]
    fn test_normalize_threads_and_timeout_reject_instead_of_clamping() {
        // P2-2：同一个 patch 接口不能有的字段静默钳位、有的字段报错。
        // 超范围一律报错，且错误串里要带上允许区间（用户得知道填多少才合法）。
        assert_eq!(normalize_moss_threads(0).unwrap(), 0);
        assert_eq!(normalize_moss_threads(16).unwrap(), 16);
        assert_eq!(normalize_moss_threads(256).unwrap(), 256);
        let err = normalize_moss_threads(999).unwrap_err();
        assert!(err.contains("999") && err.contains("256"), "错误串应带原值与区间: {err}");

        assert_eq!(normalize_moss_timeout_minutes(0).unwrap(), 0);
        assert_eq!(normalize_moss_timeout_minutes(1440).unwrap(), 1440);
        let err = normalize_moss_timeout_minutes(1441).unwrap_err();
        assert!(
            err.contains("1441") && err.contains("1440"),
            "错误串应带原值与区间: {err}"
        );
    }

    #[test]
    fn test_location_pointer_overrides_fallback() {
        let base = temp_dir("loc_base");
        let target = temp_dir("loc_target");
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).unwrap();

        // 没有指针时按兜底走（开发机 exe 向上能找到仓库 runtime/，所以只断言"不是目标目录"）
        assert_ne!(resolve_runtime_dir(Some(base.clone())), target);

        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: target.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();
        assert_eq!(resolve_runtime_dir(Some(base.clone())), target);
        assert_eq!(read_location(&base).unwrap().previous, None);

        // 指针指向的目录没了（外接盘掉线）⇒ 退回兜底，而不是返回一个死路径
        fs::remove_dir_all(&target).unwrap();
        assert_ne!(resolve_runtime_dir(Some(base.clone())), target);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_move_runtime_dir_copies_then_delete_old() {
        let base = temp_dir("move_base");
        let cur = temp_dir("move_cur");
        let dst = temp_dir("move_dst");
        for d in [&base, &cur, &dst] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(cur.join("bin")).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        fs::write(cur.join("bin").join("moss.exe"), b"x").unwrap();
        // .downloads 是临时缓存，不该被复制
        fs::create_dir_all(cur.join(".downloads")).unwrap();
        fs::write(cur.join(".downloads").join("part"), b"y").unwrap();

        // 用指针把"当前目录"指到临时目录，避免动到仓库里的真 runtime
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();

        let res = move_runtime_dir_impl(&base, &dst).unwrap();
        assert!(res.ok && res.needs_restart);
        assert!(dst.join("config.json").is_file());
        assert!(dst.join("bin").join("moss.exe").is_file());
        assert!(!dst.join(".downloads").exists(), "临时缓存不应被搬走");
        assert!(cur.exists(), "旧目录必须保留（用户验证后再删）");

        let loc = read_location(&base).unwrap();
        assert_eq!(loc.previous.as_deref(), Some(cur.to_string_lossy().as_ref()));
        assert_eq!(resolve_runtime_dir(Some(base.clone())), dst);

        // 一键删旧目录
        let msg = delete_old_runtime_dir_impl(&base).unwrap();
        assert!(msg.contains("已删除"), "删除结果：{msg}");
        assert!(!cur.exists());
        assert!(read_location(&base).unwrap().previous.is_none());
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn test_move_rejects_non_empty_target() {
        let base = temp_dir("move2_base");
        let cur = temp_dir("move2_cur");
        let dst = temp_dir("move2_dst");
        for d in [&base, &cur, &dst] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(&cur).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(dst.join("occupied.txt"), b"z").unwrap();
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();

        let err = move_runtime_dir_impl(&base, &dst).unwrap_err();
        assert!(err.contains("非空"), "错误应点名目标非空：{err}");
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&cur);
        let _ = fs::remove_dir_all(&dst);
    }

    /// P1-2：复制失败时必须回滚。不回滚会留下半个目标目录，而用户按提示「重试」时
    /// 会立刻撞上「目标目录非空」——提示与实现互相打脸。
    ///
    /// 造假方式：把源目录里的一个文件**独占打开**（`share_mode(0)`），随后的 `fs::copy`
    /// 必然拿到「另一个程序正在使用此文件」。比依赖磁盘满/拔盘这类偶发条件稳定得多。
    /// （原先这里靠「子目录里的 `.downloads`」制造对账缺失，P2-12 把两处跳过口径统一后，
    /// 那个套路不再产生缺失，反而成了正常搬家的用例，见 `test_move_succeeds_with_nested_downloads`。）
    #[test]
    fn test_move_rolls_back_created_target_when_copy_incomplete() {
        let base = temp_dir("rb_base");
        let cur = temp_dir("rb_cur");
        let dst = temp_dir("rb_dst");
        for d in [&base, &cur, &dst] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(cur.join("bin")).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        let locked = cur.join("bin").join("locked.bin");
        fs::write(&locked, b"z").unwrap();
        let _hold = lock_exclusive(&locked);
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();

        let err = move_runtime_dir_impl(&base, &dst).unwrap_err();
        assert!(err.contains("已回滚"), "错误里应说明已回滚：{err}");
        assert!(
            !dst.exists(),
            "本次新建的目标目录必须被删掉，否则重试会撞「目标目录非空」"
        );

        // 指针与旧目录都不许动
        let loc = read_location(&base).unwrap();
        assert_eq!(loc.runtime_dir, cur.display().to_string());
        assert!(loc.previous.is_none(), "失败时不该写 previous");
        assert_eq!(resolve_runtime_dir(Some(base.clone())), cur);
        assert!(cur.join("config.json").is_file(), "旧目录必须完好");

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&cur);
    }

    /// P1-2 的另一半：目标目录若是用户原本就有的空目录，失败时**不能删这个目录本身**
    /// （那是别人的目录），但要把本次拷进去的内容清干净，否则同样把「重试」堵死。
    #[test]
    fn test_move_rolls_back_into_existing_empty_target() {
        let base = temp_dir("rb2_base");
        let cur = temp_dir("rb2_cur");
        let dst = temp_dir("rb2_dst");
        for d in [&base, &cur, &dst] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(cur.join("bin")).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        let locked = cur.join("bin").join("locked.bin");
        fs::write(&locked, b"z").unwrap();
        let _hold = lock_exclusive(&locked);
        fs::create_dir_all(&dst).unwrap(); // 用户选中的已存在空目录
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();

        let err = move_runtime_dir_impl(&base, &dst).unwrap_err();
        assert!(err.contains("已回滚"), "错误里应说明已回滚：{err}");
        assert!(dst.is_dir(), "用户原有的目标目录不该被删掉");
        assert_eq!(dir_entry_count(&dst), 0, "但我们拷进去的内容必须清干净");

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&cur);
        let _ = fs::remove_dir_all(&dst);
    }

    /// P1-3：新目录不存在时（外接盘没插 / 被手动删掉），「删除旧目录」必须仍然可用。
    /// 修复前这里走 `read_location`（要求指针目标 `is_dir()`），会直接报
    /// 「没有搬家记录（指针文件缺失）」——明明指针文件在，文案是假的，而且清理路径不可达。
    #[test]
    fn test_delete_old_dir_works_when_new_dir_missing() {
        let base = temp_dir("del_base");
        let old = temp_dir("del_old");
        let new = temp_dir("del_new_missing");
        for d in [&base, &old, &new] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("config.json"), b"{}").unwrap();
        // 指针指向一个**不存在**的新目录，previous 记着旧目录
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: new.display().to_string(),
                previous: Some(old.display().to_string()),
                moved_at: Some(now_epoch()),
            },
        )
        .unwrap();
        assert!(!new.exists());

        let msg = delete_old_runtime_dir_impl(&base).unwrap();
        assert!(msg.contains("已删除"), "删除结果：{msg}");
        assert!(!old.exists(), "旧目录应被真正删掉");
        assert!(read_location_raw(&base).unwrap().previous.is_none(), "previous 应被清掉");

        // 连「已不存在」的清记录路径也要能走通
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: new.display().to_string(),
                previous: Some(temp_dir("del_old2").display().to_string()),
                moved_at: Some(now_epoch()),
            },
        )
        .unwrap();
        let msg = delete_old_runtime_dir_impl(&base).unwrap();
        assert!(msg.contains("已不存在"), "删除结果：{msg}");
        assert!(read_location_raw(&base).unwrap().previous.is_none());

        let _ = fs::remove_dir_all(&base);
    }

    /// P2-15：「删除旧目录」不能只靠「不等于当前目录」这一道护栏。
    /// 指针文件是纯文本，手改成 `C:\Windows` 就会把整盘删掉 —— 要求目标「看起来像 runtime」。
    #[test]
    fn test_delete_refuses_dir_that_does_not_look_like_runtime() {
        let base = temp_dir("lk_base");
        let cur = temp_dir("lk_cur");
        let bogus = temp_dir("lk_bogus");
        for d in [&base, &cur, &bogus] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(&cur).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        fs::create_dir_all(bogus.join("deeper")).unwrap();
        fs::write(bogus.join("deeper").join("keep.txt"), b"important").unwrap();
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: Some(bogus.display().to_string()),
                moved_at: Some(now_epoch()),
            },
        )
        .unwrap();

        let err = delete_old_runtime_dir_impl(&base).unwrap_err();
        assert!(err.contains("不像 runtime"), "错误串：{err}");
        assert!(
            bogus.join("deeper").join("keep.txt").is_file(),
            "被拒时绝不能动里面的文件"
        );

        // 名字里带 runtime 的目录应当放行（用户自选的搬家目标）
        let named = temp_dir("lk_rt_RUNTIME");
        let _ = fs::remove_dir_all(&named);
        fs::create_dir_all(&named).unwrap();
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: Some(named.display().to_string()),
                moved_at: Some(now_epoch()),
            },
        )
        .unwrap();
        let msg = delete_old_runtime_dir_impl(&base).unwrap();
        assert!(msg.contains("已删除"), "结果：{msg}");
        assert!(!named.exists());

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&cur);
        let _ = fs::remove_dir_all(&bogus);
    }

    /// P2-14：搬家目标不允许落在 app_data 内部（设计文档 §3 第 5 条）。
    /// 以前只挡了「目标在当前 runtime 内 / 是其上级 / 盘符根」，把 `%APPDATA%\gsa-rt` 这种
    /// 位置放过去了 —— 那会和日志、指针文件同树，且把源目录变成"待删旧目录"后风险更大。
    #[test]
    fn test_move_rejects_target_inside_app_data() {
        let base = temp_dir("ad_base");
        let cur = temp_dir("ad_cur");
        for d in [&base, &cur] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(&cur).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();

        // 目标就在 app_data 里面（且 它是 app_data 本身，不是当前 runtime 的子路径）
        let inside = base.join("rt-inside");
        let err = move_runtime_dir_impl(&base, &inside).unwrap_err();
        assert!(err.contains("应用数据目录内部"), "错误串：{err}");
        assert!(!inside.exists(), "被拒时不该建出目录");

        // 目录外的正常目标不受影响（这里只验证前面那道闸放行到下一步）
        let ok_target = temp_dir("ad_ok");
        let _ = fs::remove_dir_all(&ok_target);
        let res = move_runtime_dir_impl(&base, &ok_target).expect("app_data 之外的目标应放行");
        assert!(res.ok);

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&cur);
        let _ = fs::remove_dir_all(&ok_target);
    }

    /// P2-13：空间校验留 10% 余量；探测不到可用空间时不拦。
    #[test]
    fn test_ensure_enough_space_leaves_margin_and_never_blocks_on_unknown() {
        // 探测失败（None）一律放行：宁可让复制自己失败回滚，也不因探测不到就堵死
        assert!(ensure_enough_space(1 << 40, None).is_ok());

        // 刚好够、但没有余量 ⇒ 拦（10% 余量）
        assert!(ensure_enough_space(1000, Some(1000)).is_err());
        assert!(ensure_enough_space(1000, Some(1099)).is_err()); // 1099 < 1000 + 100
        assert!(ensure_enough_space(1000, Some(1100)).is_ok());

        // 0 字节目录不该因为"余量"被误拦
        assert!(ensure_enough_space(0, Some(0)).is_ok());

        // 报错要带上两个数字，让用户一眼看出差多少
        let err = ensure_enough_space(2 * 1024 * 1024 * 1024, Some(1024 * 1024 * 1024))
            .unwrap_err();
        assert!(err.contains("空间不足"), "错误串：{err}");
        assert!(err.contains("GB"), "错误串应带人类可读的量：{err}");
    }

    /// 独占打开（Windows `share_mode(0)`，其它平台 chmod 0），让随后的复制必然失败。
    fn lock_exclusive(path: &Path) -> std::fs::File {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap()
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let f = std::fs::File::open(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
            f
        }
    }

    /// P2-12 的正面用例：`.downloads` 在任意层级都不搬，而且**不会**被对账判成缺失。
    /// 修复前 `copy_dir_all` 跳任意层级、`rel_paths` 只跳根层，于是子目录里只要有一个
    /// 下载缓存，搬家就会永远失败在「目标目录缺少 bin\\.downloads」（且已回滚）。
    #[test]
    fn test_move_succeeds_with_nested_downloads() {
        let base = temp_dir("nd_base");
        let cur = temp_dir("nd_cur");
        let dst = temp_dir("nd_dst");
        for d in [&base, &cur, &dst] {
            let _ = fs::remove_dir_all(d);
        }
        fs::create_dir_all(cur.join("bin").join(".downloads")).unwrap();
        fs::write(cur.join("config.json"), b"{}").unwrap();
        fs::write(cur.join("bin").join(".downloads").join("part"), b"y").unwrap();
        fs::write(cur.join("bin").join("worker.exe"), b"ok").unwrap();
        write_location(
            &base,
            &RuntimeLocation {
                runtime_dir: cur.display().to_string(),
                previous: None,
                moved_at: None,
            },
        )
        .unwrap();

        let res = move_runtime_dir_impl(&base, &dst).expect("子目录里的下载缓存不该让搬家失败");
        assert!(res.ok);
        assert!(dst.join("config.json").is_file(), "配置要搬过去");
        assert!(dst.join("bin").join("worker.exe").is_file(), "可执行文件要搬过去");
        assert!(
            !dst.join("bin").join(".downloads").exists(),
            "下载缓存不搬"
        );

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&cur);
        let _ = fs::remove_dir_all(&dst);
    }

    /// P2-12：`.downloads` 在**任意层级**都算临时缓存 —— 复制不搬它，对账也必须不要求它。
    /// 修复前 `copy_dir_all` 跳任意层级、`rel_paths` 只跳根层，两套口径互相矛盾：
    /// 要么对账把「没搬的缓存」判成缺失（搬家永远失败），要么反过来漏检。
    #[test]
    fn test_rel_paths_skips_downloads_at_any_depth_and_records_size() {
        let root = temp_dir("rp_root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a").join("b")).unwrap();
        fs::create_dir_all(root.join(".downloads")).unwrap();
        fs::create_dir_all(root.join("a").join(".downloads")).unwrap();
        fs::write(root.join(".downloads").join("x"), b"x").unwrap();
        fs::write(root.join("a").join(".downloads").join("y"), b"yy").unwrap();
        fs::write(root.join("config.json"), b"{}").unwrap();
        fs::write(root.join("a").join("b").join("w.bin"), b"0123456789").unwrap();

        let map = rel_paths(&root).unwrap();
        assert!(
            map.keys().all(|k| !k.contains(".downloads")),
            "任意层级的 .downloads 都不该进对账清单：{:?}",
            map.keys().collect::<Vec<_>>()
        );
        assert_eq!(map.get("config.json"), Some(&2), "文件要记字节数：{map:?}");
        let nested = format!(
            "a{0}b{0}w.bin",
            std::path::MAIN_SEPARATOR
        );
        assert_eq!(map.get(&nested), Some(&10), "嵌套文件的相对路径与大小：{map:?}");
        assert!(map.contains_key("a"), "目录本身也要在清单里：{map:?}");

        let _ = fs::remove_dir_all(&root);
    }

    /// P2-12：读不动的目录必须报错，绝不能静默当成「没有文件」。
    /// 修复前是 `let Ok(entries) = fs::read_dir(dir) else { return }`，于是
    /// 「某子目录没权限 / 外接盘掉线」会让对账通过 —— 而那批文件根本没搬过去，
    /// 指针照样被改写，正好踩中「半搬状态比不搬更糟」。
    #[test]
    fn test_rel_paths_propagates_unreadable_dir() {
        let missing = temp_dir("rp_missing");
        let _ = fs::remove_dir_all(&missing);
        let err = rel_paths(&missing).unwrap_err();
        assert!(err.contains("失败"), "错误里应说清哪个目录读不了：{err}");
    }
}
