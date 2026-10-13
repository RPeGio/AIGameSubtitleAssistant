// ─── 向量融合 Worker Provider（T4/R2）──────────────────────
// 一次性子进程：`python embed_worker.py <input.json> <output.json>`。
// 复用 runtime/python 解释器；依赖装在独立目录 deps_embed（onnxruntime CPU +
// tokenizers，pip --target），模型 runtime/models/embed（bootstrap_embed 预下载）。
// 四案例端到端（含模型加载）实测 ~2s，融合不是交互式高频操作，故不做常驻进程、
// 不做取消（参见 funasr.rs 头注释的取舍理由，本模块是它的精简版）。
//
// 与 LLM 路线的关键差异：结果走**输出文件**而非 stdout——stdout 在中文 Windows
// 上可能混入 GBK 噪音（funasr.rs 已踩过的坑），文件交换没有这个失效面。
//
// 为什么放 ai_runtime 而不是 fuse/：与其他 worker provider（paddle/moss/funasr/llm）
// 并列，服从「ai_runtime = 进程编排层」的既有分层，fuse/ 保持为纯装配+协议层。

use crate::ai_runtime::RuntimeConfig;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// worker 输入段（转写侧一段；index = 输入顺序 1-based，worker 输出按此对应）
#[derive(Debug, Clone, Serialize)]
pub struct EmbedSegmentIn {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// worker 输入请求体
#[derive(Debug, Clone, Serialize)]
pub struct EmbedRequest {
    pub corpus: Vec<String>,
    pub segments: Vec<EmbedSegmentIn>,
    /// 「该不配」行掩码开关（按素材，默认关）
    pub unmatch_filter: bool,
    /// 候选名单长度
    pub top_k: usize,
    /// 嵌入模型目录名（`runtime/models/embed/` 下；缺省/null = worker 内置默认，
    /// PR43 审查 P1-1：从 `RuntimeConfig::embed_model` 路径拆出，兑现配置契约）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 该目录下的 ONNX 文件名；缺省/null = worker 内置默认
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onnx: Option<String>,
}

/// worker 输出：每段的对齐结果与结构化审计信号
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbedSegmentOut {
    pub index: usize,
    pub start: f64,
    pub end: f64,
    /// 命中的语料下标（1-based；-1 = 不配）
    pub ocr_index: i64,
    /// 命中行的余弦相似度；不配时为 0.0
    pub score: f64,
    /// 融合时刻的转写原文（R3 审计表「不配/替换」的回退真值；旧 worker 无此字段时为空）
    #[serde(default)]
    pub source_text: String,
    /// 产物文本：命中 = 语料原文逐字；不配 = 转写侧原文
    pub product_text: String,
    /// 候选名单 [[语料下标(1-based), 相似度]]，相似度降序
    pub candidates: Vec<(i64, f64)>,
    /// 该段被「该不配」判据整行封死（判据命中 ⇔ 该段必为不配）
    pub masked: bool,
    /// 转移类型：forward / repeat / reset / unmatched
    pub transition: String,
    /// 该段命中的语料行被 >=2 段命中（语料行被重复使用）
    pub reused: bool,
}

/// worker 输出统计
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbedStats {
    pub matched: usize,
    pub unmatched: usize,
    pub masked_rows: usize,
    pub masked_cells: usize,
    pub reused_rows: usize,
    pub transition: std::collections::HashMap<String, usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EmbedResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    stats: Option<EmbedStats>,
    #[serde(default)]
    segments: Option<Vec<EmbedSegmentOut>>,
}

/// 融合失败错误（fuse 模块专用错误类型，与 AsrError 等并列分层）
#[derive(Debug)]
pub enum EmbedError {
    /// 运行环境未就绪（配置缺失/文件缺失）
    NotReady(String),
    /// worker 执行失败（启动失败/非零退出/输出解析失败）
    Worker(String),
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedError::NotReady(m) => write!(f, "{}", m),
            EmbedError::Worker(m) => write!(f, "{}", m),
        }
    }
}

/// 一次性子进程客户端：装配输入 JSON → spawn python → 读 output.json → 反序列化。
pub fn run_embed_worker(
    config: &RuntimeConfig,
    runtime_dir: &Path,
    req: &EmbedRequest,
) -> Result<(Vec<EmbedSegmentOut>, EmbedStats), EmbedError> {
    let configured = !config.embed_worker.is_empty();
    if !configured {
        return Err(EmbedError::NotReady(
            "向量融合未配置（runtime/config.json 缺 embed_* 字段）".into(),
        ));
    }
    let python = crate::ai_runtime::resolve_path(&config.python_path, runtime_dir);
    let worker = crate::ai_runtime::resolve_path(&config.embed_worker, runtime_dir);
    let deps = crate::ai_runtime::resolve_path(&config.embed_deps_dir, runtime_dir);
    if !python.is_file() {
        return Err(EmbedError::NotReady(format!(
            "Python 解释器不存在: {}",
            python.display()
        )));
    }
    if !worker.is_file() {
        return Err(EmbedError::NotReady(format!(
            "fuse worker 脚本不存在: {}（请先跑 scripts/bootstrap_embed.ps1）",
            worker.display()
        )));
    }

    // 输入/输出走系统临时目录（命名隔离，不会污染工程目录）：
    // 写输入 JSON 必须先于 spawn（worker 启动即读文件）。
    // 子目录按 pid+纳秒唯一（一次一目录）：同进程两次并发调用不得互写对方文件，
    // 也不读上一次调用遗留的陈旧输出（PR43 审查 P2-1）
    let tmp = std::env::temp_dir().join(format!(
        "gsa_embed_fuse_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp)
        .map_err(|e| EmbedError::Worker(format!("无法创建临时目录: {}", e)))?;
    let in_path = tmp.join("input.json");
    let out_path = tmp.join("output.json");
    let input_json = serde_json::to_vec_pretty(req)
        .map_err(|e| EmbedError::Worker(format!("输入序列化失败: {}", e)))?;
    std::fs::write(&in_path, &input_json)
        .map_err(|e| EmbedError::Worker(format!("无法写输入文件: {}", e)))?;

    // 输入/输出走临时文件（temp_dir 由 RuntimeConfig::temp_dir 提供，不会污染工程目录）
    let mut cmd = Command::new(&python);
    cmd.arg(&worker)
        .arg(&in_path)
        .arg(&out_path)
        // PYTHONPATH 指向独立 deps 目录（含 numpy/onnxruntime/tokenizers）；
        // fuse_core 自身也带兜底自举，双保险（不用 PROJECT 模型的 redirect 环境变量）
        .env("PYTHONPATH", &deps)
        .env("PYTHONIOENCODING", "utf-8");
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| EmbedError::Worker(format!("无法启动 fuse worker: {}", e)))?;

    // 两个管道各交给读线程抽干，防缓冲写满阻塞 worker
    let (stdout_tx, stderr_tx) = (child.stdout.take(), child.stderr.take());
    let out_reader = stdout_tx.map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });
    let err_reader = stderr_tx.map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            buf
        })
    });

    // 轮询等待（非阻塞 try_wait）：onnxruntime 卡死等异常不能把融合命令永久挂起
    //（一次成功的端到端约 2s，120s 兜底已十倍余量；els onnx 卡死时 kill + 报错，
    // 前端由 run_fuse 的错误路径接住——PR43 审查 P2-2）
    const EMBED_WAIT_TIMEOUT_SECS: u64 = 120;
    let started = std::time::Instant::now();
    let mut status = child
        .try_wait()
        .map_err(|e| EmbedError::Worker(format!("fuse worker 异常: {}", e)))?;
    while status.is_none() {
        if started.elapsed().as_secs() >= EMBED_WAIT_TIMEOUT_SECS {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(EmbedError::Worker(format!(
                "fuse worker 超过 {}s 未退出，已终止（疑似 onnxruntime 卡死）",
                EMBED_WAIT_TIMEOUT_SECS
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        status = child
            .try_wait()
            .map_err(|e| EmbedError::Worker(format!("fuse worker 异常: {}", e)))?;
    }
    let status = status.expect("循环退出条件的谓词保证 Some");
    let stdout = String::from_utf8_lossy(
        &out_reader
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default(),
    )
    .into_owned();
    let stderr = String::from_utf8_lossy(
        &err_reader
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default(),
    )
    .into_owned();
    let _ = stdout; // worker stdout 只打一行摘要，留日志排查用
    if !status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(EmbedError::Worker(format!(
            "fuse worker 退出码非零（{}）：{}",
            status,
            stderr.trim()
        )));
    }

    let resp: EmbedResponse = serde_json::from_reader(std::fs::File::open(&out_path).map_err(
        |e| EmbedError::Worker(format!("无法读取 worker 输出: {}", e)),
    )?)
    .map_err(|e| {
        let _ = std::fs::remove_dir_all(&tmp);
        EmbedError::Worker(format!("worker 输出解析失败: {}", e))
    })?;
    let out = match resp {
        EmbedResponse {
            ok: true,
            stats: Some(stats),
            segments: Some(segments),
            ..
        } => Ok((segments, stats)),
        EmbedResponse {
            ok: false,
            error: Some(err),
            ..
        } => Err(EmbedError::Worker(format!("fuse worker 报错: {}", err))),
        _ => Err(EmbedError::Worker(
            "fuse worker 输出结构不完整（ok/stats/segments）".into(),
        )),
    };
    let _ = std::fs::remove_dir_all(&tmp); // 一次性目录用完即清（成功/失败都走到这）
    out
}
