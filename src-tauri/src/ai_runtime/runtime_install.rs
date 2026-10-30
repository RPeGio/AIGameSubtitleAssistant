//! runtime 安装器（bootstrap）的 Rust 侧基础设施
//!
//! 契约见 `docs/RUNTIME_BOOTSTRAP_CONTRACT.md`：
//! - 脚本：`scripts/bootstrap_runtime.ps1`（stdout 只输出 JSONL 事件，stderr 是人类诊断）
//! - 本模块只做四件事：**磁盘预检** → **起子进程** → **逐行解析 JSONL 并转发成 Tauri 事件**
//!   → **取消（杀进程树）**
//!
//! 为什么要 Rust 侧包一层，而不是让前端直接调脚本：
//! 1. 前端没有 exec 权限（没有 shell 插件），也不该有；
//! 2. 安装要能取消——取消必须落到"杀掉子进程"这个动作上，只有 Rust 握得住 `Child`；
//! 3. 磁盘预检要在**下载任何东西之前**拦下（契约 §5 退出码 1），不能等 powershell 跑起来；
//! 4. 事件流要能被前端 `listen` 到，且不阻塞 UI 线程（`spawn_blocking`）。

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Manager;
use tauri::Emitter;

/// 安装进度事件名（前端 `listen("runtime-install-progress", cb)`）
pub const INSTALL_PROGRESS_EVENT: &str = "runtime-install-progress";

/// 安装器脚本的仓库内相对路径
pub const BOOTSTRAP_SCRIPT_REL: &str = "scripts/bootstrap_runtime.ps1";

/// 磁盘需求的余量系数：下载 zip + 解压展开（经验约 2 倍）+ 固定余量。
/// 与 `scripts/bootstrap_runtime.ps1` 里的 `$downloadBytes * 3 + 100MB` 保持同一口径，
/// 两边都算一遍是刻意的：脚本是权威（它会真的失败并退出码 1），Rust 侧提前拦是为了
/// 在**等 powershell 启动之前**就把话说明白。
const EXPANSION_FACTOR: u64 = 3;
const SAFETY_MARGIN: u64 = 100 * 1024 * 1024;

// ─── 协议事件 ─────────────────────────────────────────────

/// 脚本 stdout 的一行事件。
///
/// 字段与契约 §3 冻结的 schema 一一对应；缺省字段用 `Option` + `skip_serializing_if`，
/// 转发给前端时保持"缺省即省略"，前端只需按 `event` 分支。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct InstallEvent {
    /// start / progress / item_done / item_skip / item_fail / error / done
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavor: Option<String>,
    /// download / verify / extract / configure（仅 progress）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// 0–100（仅 progress）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 以下三个只出现在 done 事件
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<Vec<String>>,
}

impl InstallEvent {
    /// 事件名是否在冻结集合内（脚本若被改坏、吐出别的事件名，转发但记一条诊断）
    pub fn is_known_event(&self) -> bool {
        matches!(
            self.event.as_str(),
            "start" | "progress" | "item_done" | "item_skip" | "item_fail" | "error" | "done"
        )
    }
}

/// 解析一行 stdout。空行、非 JSON、缺 `event` 字段一律返回 `None`
/// （契约要求 stdout 只有 JSONL，但脚本的 stderr 混进管道、被安全软件插一行之类的事故
/// 不该让整次安装失败，宁可丢这一行并留诊断）。
pub fn parse_event_line(line: &str) -> Option<InstallEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let evt: InstallEvent = serde_json::from_str(trimmed).ok()?;
    if evt.event.is_empty() {
        return None;
    }
    Some(evt)
}

/// 一次安装的结果（给前端做收尾展示；明细在事件流里）
#[derive(Debug, Clone, Default, Serialize)]
pub struct InstallSummary {
    /// 退出码 0（全成功）才为 true
    pub ok: bool,
    /// 契约 §5 的退出码：0/1/2/3/4
    pub exit_code: i32,
    /// 已安装条目；取消时为已完成的那些
    pub installed: Vec<String>,
    /// 跳过条目（已装且版本一致）
    pub skipped: Vec<String>,
    /// 失败条目明细（退出码 3 的来源）
    pub failed: Vec<String>,
    /// 被用户取消
    pub cancelled: bool,
    /// 汇总文案（直接给 UI 用，不要求前端拼字符串）
    pub message: String,
}

// ─── 磁盘预检 ─────────────────────────────────────────────

/// 预检结论（前端可在点安装前先查一次，避免下完 600 MB 才发现没空间）
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiskPrecheck {
    pub ok: bool,
    /// manifest 里找到的条目数（0 = 清单没找到/里没有这些 id）
    pub manifest_entries: usize,
    /// 估算需要的字节数（含展开余量）
    pub required_bytes: u64,
    /// runtime 所在盘的可用字节；查不到为 None（此时不拦，交给脚本兜底）
    pub free_bytes: Option<u64>,
    pub message: String,
}

/// 按清单条目估算磁盘需求并判断是否够用（纯函数，便于单测）。
///
/// - `flavor`：`auto` 时取该 id 所有 flavor 的**最大** size（保守估计）；
/// - `free_bytes` 为 None 表示查不到可用空间 → 不拦（脚本的预检仍是权威）。
pub fn precheck_disk_with(
    manifest_text: Option<&str>,
    free_bytes: Option<u64>,
    id: &str,
    flavor: &str,
) -> DiskPrecheck {
    let Some(text) = manifest_text else {
        return DiskPrecheck {
            ok: true,
            manifest_entries: 0,
            required_bytes: 0,
            free_bytes,
            message: "未找到安装清单，跳过磁盘预检（安装脚本会再查一次）".into(),
        };
    };
    let entries: Vec<super::installed_state::ManifestEntry> =
        super::installed_state::parse_manifest_items(text)
            .into_iter()
            .filter(|e| e.id == id)
            .collect();
    if entries.is_empty() {
        return DiskPrecheck {
            ok: true,
            manifest_entries: 0,
            required_bytes: 0,
            free_bytes,
            message: format!("清单里没有条目 {id}，跳过磁盘预检"),
        };
    }

    let want_auto = flavor.is_empty() || flavor.eq_ignore_ascii_case("auto");
    let pick = |e: &super::installed_state::ManifestEntry| -> u64 {
        // 有 unpacked_size 就按「下载 + 解压 + 解压」算：解压要同时存在于临时目录与
        // dest 两份，峰值 = 2×解压 + 下载。没有真值时退回 size × 3 的经验值。
        match e.unpacked_size {
            Some(u) if u > 0 => e.size.saturating_add(u).saturating_add(u),
            _ => e.size.saturating_mul(EXPANSION_FACTOR),
        }
    };
    let required = if want_auto {
        // auto：保守取该 id 所有 flavor 里最贵的那个
        entries.iter().map(pick).max().unwrap_or(0)
    } else {
        entries
            .iter()
            .find(|e| e.flavor.eq_ignore_ascii_case(flavor))
            .map(pick)
            .unwrap_or(0)
    }
    .saturating_add(SAFETY_MARGIN);
    let size = if want_auto {
        entries.iter().map(|e| e.size).max().unwrap_or(0)
    } else {
        entries
            .iter()
            .find(|e| e.flavor.eq_ignore_ascii_case(flavor))
            .map(|e| e.size)
            .unwrap_or(0)
    };

    let ok = match free_bytes {
        Some(free) => free >= required,
        None => true,
    };
    let message = match free_bytes {
        Some(free) if !ok => format!(
            "磁盘空间不足：需要约 {:.2} GB（下载 {:.2} GB + 解压余量），可用 {:.2} GB",
            required as f64 / 1024.0 / 1024.0 / 1024.0,
            size as f64 / 1024.0 / 1024.0 / 1024.0,
            free as f64 / 1024.0 / 1024.0 / 1024.0
        ),
        Some(free) => format!(
            "磁盘空间充足：需要约 {:.2} GB，可用 {:.2} GB",
            required as f64 / 1024.0 / 1024.0 / 1024.0,
            free as f64 / 1024.0 / 1024.0 / 1024.0
        ),
        None => format!(
            "无法读取可用空间，跳过判定（需要约 {:.2} GB，安装脚本会再查一次）",
            required as f64 / 1024.0 / 1024.0 / 1024.0
        ),
    };

    DiskPrecheck {
        ok,
        manifest_entries: entries.len(),
        required_bytes: required,
        free_bytes,
        message,
    }
}

/// 查某个路径所在盘的可用字节。
///
/// 只用盘符查 `Get-PSDrive`，不拼接路径（避免引号/中文路径的转义问题）；
/// 查不到返回 None（不阻塞安装，脚本里还有一次权威预检）。
fn query_free_bytes(path: &Path) -> Option<u64> {
    let raw = path.to_string_lossy();
    let mut chars = raw.chars();
    let letter = chars.next()?;
    if chars.next()? != ':' || !letter.is_ascii_alphabetic() {
        return None;
    }
    let script = format!(
        "$d = Get-PSDrive -Name {} -ErrorAction SilentlyContinue; if ($d) {{ $d.Free }}",
        letter
    );
    let out = crate::subprocess::hidden("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().parse::<u64>().ok()
}

// ─── 安装器状态（可取消） ─────────────────────────────────

/// 托管在 Tauri 里的安装器状态。
///
/// 同一时刻只允许一个安装任务（`running` 抢不到就直接拒绝）：脚本会写同一个
/// `runtime/` 目录，并发跑必然互相踩。
#[derive(Default)]
pub struct RuntimeInstaller {
    child: Mutex<Option<Child>>,
    running: AtomicBool,
    cancelled: AtomicBool,
}

impl RuntimeInstaller {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 终止正在跑的安装（无活动进程时仅置取消标志，无副作用）
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.child.lock() {
            if let Some(mut child) = slot.take() {
                // 先杀进程树：powershell 自己是根，但它会等子进程（下载/解压）结束，
                // 只杀根可能留下悬挂的 IO；/T 覆盖整棵树
                #[cfg(windows)]
                {
                    let _ = crate::subprocess::hidden("taskkill")
                        .args(["/PID", &child.id().to_string(), "/T", "/F"])
                        .output();
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// `running` 的 RAII 归位：任何 return 路径（含 panic 前的 unwind）都要放开锁，
/// 否则一次失败会让"已有安装任务在进行"永久卡住。
struct RunningGuard<'a>(&'a RuntimeInstaller);

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
    }
}

// ─── 脚本定位 ─────────────────────────────────────────────

/// 找 `scripts/bootstrap_runtime.ps1` 的候选路径（第一个存在的胜出）。
///
/// 脚本**随安装包发布**：`pnpm build` 先跑 `scripts/sync_bundle_resources.mjs`
/// 把它拷进 `src-tauri/resources/scripts/`，再由 `bundle.resources` 带进安装包，
/// 最终落在 `<exe目录>/resources/scripts/bootstrap_runtime.ps1`（见候选列表里的
/// `dir/resources/...`）。Windows 上 `resource_dir()` 就是 exe 所在目录。
/// 开发期则直接命中仓库里的 `scripts/`。
pub fn bootstrap_candidates(runtime_dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("GSA_BOOTSTRAP_SCRIPT") {
        if !p.trim().is_empty() {
            v.push(PathBuf::from(p));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.join(BOOTSTRAP_SCRIPT_REL));
        // dev 形态：cwd 是 src-tauri，脚本在上一级
        v.push(cwd.join("..").join(BOOTSTRAP_SCRIPT_REL));
    }
    // runtime 目录就在仓库根下时，往上两级也找一遍
    v.push(runtime_dir.join(BOOTSTRAP_SCRIPT_REL));
    if let Some(parent) = runtime_dir.parent() {
        v.push(parent.join(BOOTSTRAP_SCRIPT_REL));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join(BOOTSTRAP_SCRIPT_REL));
            v.push(dir.join("..").join(BOOTSTRAP_SCRIPT_REL));
            v.push(dir.join("resources").join(BOOTSTRAP_SCRIPT_REL));
        }
    }
    v
}

/// 按候选顺序找脚本（第一个存在的胜出）
pub fn find_bootstrap_script(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

// ─── Tauri 命令 ───────────────────────────────────────────

/// Tauri 命令：只做磁盘预检（不安装）。前端可在展示"安装"按钮前先查。
#[tauri::command]
pub async fn precheck_runtime_disk(app: tauri::AppHandle) -> Result<DiskPrecheck, String> {
    tauri::async_runtime::spawn_blocking(move || precheck_impl(&app))
        .await
        .map_err(|e| format!("磁盘预检内部错误: {}", e))?
}

/// Tauri 命令：跑一次安装。
///
/// - `items`：逗号分隔条目 id（默认 `moss`）
/// - `flavor`：`auto` / `cpu` / `cuda` / `vulkan`（默认 `auto`）
/// - `force`：忽略已装标记
///
/// 过程通过 `runtime-install-progress` 事件上报；返回汇总。取消请调
/// `cancel_runtime_install`（会杀掉 powershell 进程树）。
#[tauri::command]
pub async fn install_runtime(
    app: tauri::AppHandle,
    items: Option<String>,
    flavor: Option<String>,
    force: Option<bool>,
) -> Result<InstallSummary, String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let installer = handle.state::<RuntimeInstaller>();
        run_install(&handle, &installer, items, flavor, force.unwrap_or(false))
    })
    .await
    .map_err(|e| format!("安装任务内部错误: {}", e))?
}

/// Tauri 命令：取消安装（杀进程树）。
#[tauri::command]
pub fn cancel_runtime_install(app: tauri::AppHandle) -> bool {
    let installer = app.state::<RuntimeInstaller>();
    if !installer.is_running() {
        return false;
    }
    installer.cancel();
    true
}

// ─── 实现 ─────────────────────────────────────────────────

/// runtime 目录 + 清单（与 `probe_moss_install` 同一套解析规则）
fn runtime_paths(app: &tauri::AppHandle) -> (PathBuf, Option<PathBuf>) {
    let runtime_dir = super::config::resolve_runtime_dir(
        app.path().app_data_dir().ok().map(|d| d.join("runtime")),
    );
    let manifest = super::backend_probe::find_manifest(&super::backend_probe::manifest_candidates(
        app,
        &runtime_dir,
    ));
    (runtime_dir, manifest)
}

fn precheck_impl(app: &tauri::AppHandle) -> Result<DiskPrecheck, String> {
    let (runtime_dir, manifest) = runtime_paths(app);
    let text = manifest
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let free = query_free_bytes(&runtime_dir);
    Ok(precheck_disk_with(text.as_deref(), free, "moss", "auto"))
}

/// 子进程输出里保留的最后一段（给失败时拼错误文案）
const STDERR_TAIL_LIMIT: usize = 4000;

fn push_tail(buf: &mut String, text: &str) {
    if !buf.is_empty() {
        buf.push('\n');
    }
    buf.push_str(text);
    if buf.len() > STDERR_TAIL_LIMIT {
        // 只留尾部：真正有用的报错通常在最后几行
        let keep_from = buf.len() - STDERR_TAIL_LIMIT;
        let safe = buf
            .char_indices()
            .map(|(i, _)| i)
            .find(|i| *i >= keep_from)
            .unwrap_or(buf.len());
        *buf = buf[safe..].to_string();
    }
}

/// 把这次安装的诊断落到 `runtime/logs/install.log`（每次安装覆盖上一次）。
///
/// 为什么要文件：打包形态下 powershell 子进程**没有可见控制台**（我们刻意加了
/// CREATE_NO_WINDOW），安装失败时用户只看到一句 toast，没处看原因——真机反馈
/// 2026-10-29 正是卡在"命令行一闪而过、看不到报错"上。失败文案会带上这个路径。
fn write_install_log(
    runtime_dir: &Path,
    script: &Path,
    flavor: &str,
    outcome: &StreamOutcome,
) -> Option<PathBuf> {
    let dir = runtime_dir.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("install.log");
    let status = outcome
        .status
        .as_ref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "已取消（没有退出码）".into());
    let stderr = outcome.stderr_tail.trim();
    let body = format!(
        "脚本: {}\nflavor: {}\n退出码: {}\n\n── stderr（尾部 {STDERR_TAIL_LIMIT} 字符）──\n{}\n",
        script.display(),
        flavor,
        status,
        if stderr.is_empty() { "（空）" } else { stderr }
    );
    std::fs::write(&path, body).ok()?;
    Some(path)
}

fn run_install(
    app: &tauri::AppHandle,
    installer: &RuntimeInstaller,
    items: Option<String>,
    flavor: Option<String>,
    force: bool,
) -> Result<InstallSummary, String> {
    // 单实例：抢不到 running 直接拒绝（脚本写同一个 runtime 目录，并发必打架）
    if installer
        .running
        .swap(true, Ordering::SeqCst)
    {
        return Err("已有安装任务在进行中".into());
    }
    let _guard = RunningGuard(installer);
    installer.cancelled.store(false, Ordering::SeqCst);

    let (runtime_dir, manifest) = runtime_paths(app);

    // 1) 磁盘预检：不通过就当场结束（等价脚本的退出码 1），一个字节都不下
    let text = manifest
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let want_flavor = flavor.clone().unwrap_or_else(|| "auto".into());
    let pre = precheck_disk_with(text.as_deref(), query_free_bytes(&runtime_dir), "moss", &want_flavor);
    if !pre.ok {
        emit_event(
            app,
            InstallEvent {
                event: "error".into(),
                item: Some("moss".into()),
                detail: Some(pre.message.clone()),
                ..Default::default()
            },
        );
        emit_event(
            app,
            InstallEvent {
                event: "done".into(),
                ok: Some(false),
                installed: Some(vec![]),
                skipped: Some(vec![]),
                failed: Some(vec![pre.message.clone()]),
                ..Default::default()
            },
        );
        return Ok(InstallSummary {
            ok: false,
            exit_code: 1,
            cancelled: false,
            message: pre.message,
            ..Default::default()
        });
    }

    // 2) 找脚本
    let script = find_bootstrap_script(&bootstrap_candidates(&runtime_dir))
        .ok_or_else(|| format!("找不到安装脚本 {BOOTSTRAP_SCRIPT_REL}（打包形态下脚本未随 app 发布）"))?;

    // 3) 起子进程：stdout 是 JSONL 协议流，stderr 是人类诊断
    let mut cmd = crate::subprocess::hidden("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-Items")
        .arg(items.unwrap_or_else(|| "moss".into()))
        .arg("-Flavor")
        .arg(&want_flavor)
        .arg("-RuntimeDir")
        .arg(&runtime_dir);
    if let Some(m) = manifest.as_deref() {
        // 用 Rust 侧解析到的同一份清单，避免脚本按 cwd 猜出另一份
        cmd.arg("-Manifest").arg(m);
    }
    if force {
        cmd.arg("-Force");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    // 控制台编码对齐脚本开头的 UTF-8 设定，避免中文 detail 在管道里变 GBK
    cmd.env("PYTHONIOENCODING", "utf-8");

    let child = cmd
        .spawn()
        .map_err(|e| format!("无法启动安装脚本: {e}"))?;

    // 4) 起进程后的一切（读管道 / 等结束 / 可取消）交给 stream_child，
    //    它不依赖 Tauri，便于用真实脚本做端到端单测
    let app_for_events = app.clone();
    let outcome = stream_child(
        child,
        &installer.child,
        &installer.cancelled,
        move |evt| emit_event(&app_for_events, evt),
        &|| installer.cancel(),
    )?;
    // 落一份诊断日志：失败时把它写进文案，用户不用再猜"刚才那个黑窗口报了什么"
    let log_path = write_install_log(&runtime_dir, &script, &want_flavor, &outcome);
    let mut summary = summarize(outcome);
    if !summary.ok {
        if let Some(p) = log_path {
            summary.message = format!("{}（日志：{}）", summary.message, p.display());
        }
    }
    Ok(summary)
}

/// 一次运行的原始产物（未套退出码语义）
struct StreamOutcome {
    /// None = 被取消，没拿到退出码
    status: Option<std::process::ExitStatus>,
    /// stdout 里最后一条 done 事件（脚本没吐 done 时为默认值）
    done: InstallEvent,
    /// stderr 尾部（失败时拼文案用）
    stderr_tail: String,
}

/// 读子进程管道 + 等结束，全程可被取消。
///
/// 单独抽出来的理由：它不碰 Tauri（事件用闭包回传），所以能用**真实脚本**
/// 做端到端单测——这是"JSONL 进度 + 取消"这套基建唯一值得用真实子进程钉住的部分。
fn stream_child<F>(
    mut child: Child,
    child_slot: &Mutex<Option<Child>>,
    cancelled: &AtomicBool,
    on_event: F,
    on_cancel: &dyn Fn(),
) -> Result<StreamOutcome, String>
where
    F: Fn(InstallEvent) + Send + 'static,
{
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // stdout 读线程：逐行解析事件 → 回传 + 记下 done
    let done_evt: Arc<Mutex<Option<InstallEvent>>> = Arc::new(Mutex::new(None));
    let done_slot = done_evt.clone();
    let out_thread = stdout.map(|s| {
        std::thread::spawn(move || {
            for line in BufReader::new(s).lines() {
                let Ok(line) = line else { break };
                match parse_event_line(&line) {
                    Some(evt) => {
                        if !evt.is_known_event() {
                            eprintln!("[runtime_install] 未知事件名: {}", evt.event);
                        }
                        if evt.event == "done" {
                            if let Ok(mut slot) = done_slot.lock() {
                                *slot = Some(evt.clone());
                            }
                        }
                        on_event(evt);
                    }
                    None => {
                        // 非协议行：不进事件流，落到 stderr 供排查
                        eprintln!("[runtime_install] stdout 非协议行: {}", line.trim());
                    }
                }
            }
        })
    });

    // stderr 读线程：实时转 eprintln，并留尾部供失败文案用
    let err_tail: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let tail_slot = err_tail.clone();
    let err_thread = stderr.map(|s| {
        std::thread::spawn(move || {
            let mut reader = BufReader::new(s);
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            let text = String::from_utf8_lossy(&buf).into_owned();
            for line in text.lines() {
                eprintln!("[runtime_install] {line}");
            }
            if let Ok(mut tail) = tail_slot.lock() {
                for line in text.lines() {
                    push_tail(&mut tail, line);
                }
            }
        })
    });

    {
        let mut slot = child_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(child);
    }

    // 等结束：可被 cancel 打断
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            // cancel 可能落在子进程登记前的窗口（flag 已置但没 kill），幂等再调一次
            on_cancel();
            break None;
        }
        let mut slot = child_slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(Some(st)) => {
                    *slot = None;
                    break Some(st);
                }
                Ok(None) => {}
                Err(e) => {
                    *slot = None;
                    return Err(format!("安装进程异常: {e}"));
                }
            },
            None => break None,
        }
        drop(slot);
        std::thread::sleep(Duration::from_millis(120));
    };

    if let Some(h) = out_thread {
        let _ = h.join();
    }
    if let Some(h) = err_thread {
        let _ = h.join();
    }

    let done = done_evt
        .lock()
        .map(|slot| slot.clone())
        .unwrap_or(None)
        .unwrap_or_default();
    let stderr_tail = err_tail.lock().map(|t| t.clone()).unwrap_or_default();

    Ok(StreamOutcome {
        status,
        done,
        stderr_tail,
    })
}

/// 把原始产物套上契约 §5 的退出码语义（纯函数，便于单测）
fn summarize(outcome: StreamOutcome) -> InstallSummary {
    let StreamOutcome {
        status,
        done,
        stderr_tail,
    } = outcome;

    let Some(status) = status else {
        // 取消：已完成的条目可能已写进 .installed.json，如实回传
        return InstallSummary {
            ok: false,
            exit_code: 4,
            installed: done.installed.clone().unwrap_or_default(),
            skipped: done.skipped.clone().unwrap_or_default(),
            failed: done.failed.clone().unwrap_or_default(),
            cancelled: true,
            message: "安装已取消".into(),
        };
    };

    let code = status.code().unwrap_or(4);
    // done 事件与退出码必须一致；若脚本自相矛盾，以退出码为准（进程怎么退出是事实）
    let ok = code == 0 && done.ok.unwrap_or(true);
    let message = if ok {
        format!(
            "安装完成：新增 {} 项，跳过 {} 项",
            done.installed.as_ref().map(|v| v.len()).unwrap_or(0),
            done.skipped.as_ref().map(|v| v.len()).unwrap_or(0)
        )
    } else {
        let detail = done
            .failed
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| v.join("；"))
            .unwrap_or_else(|| {
                stderr_tail
                    .lines()
                    .last()
                    .unwrap_or("未知错误")
                    .to_string()
            });
        format!("安装失败（退出码 {code}）：{detail}")
    };

    InstallSummary {
        ok,
        exit_code: code,
        installed: done.installed.unwrap_or_default(),
        skipped: done.skipped.unwrap_or_default(),
        failed: done.failed.unwrap_or_default(),
        cancelled: false,
        message,
    }
}

fn emit_event(app: &tauri::AppHandle, evt: InstallEvent) {
    let _ = app.emit(INSTALL_PROGRESS_EVENT, evt);
}

// ─── 单测 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "version": "1",
        "items": [
            {"id":"moss","flavor":"cpu","url":"u1","sha256":"a","size":1121692,"dest":"bin","backend_marker":"cpu"},
            {"id":"moss","flavor":"vulkan","url":"u2","sha256":"b","size":16139894,"dest":"bin","backend_marker":"vulkan"},
            {"id":"moss","flavor":"cuda","url":"u3","sha256":"c","size":651834262,"dest":"bin","backend_marker":"cuda"}
        ]
    }"#;

    #[test]
    fn parse_progress_line() {
        let evt = parse_event_line(
            r#"{"event":"progress","item":"moss","phase":"download","percent":42.3,"detail":"下载中"}"#,
        )
        .expect("应解析成功");
        assert_eq!(evt.event, "progress");
        assert_eq!(evt.item.as_deref(), Some("moss"));
        assert_eq!(evt.phase.as_deref(), Some("download"));
        assert_eq!(evt.percent, Some(42.3));
        assert!(evt.flavor.is_none(), "缺省字段应为 None，转发时被省略");
        assert!(evt.is_known_event());
    }

    #[test]
    fn parse_done_line_keeps_arrays() {
        let evt = parse_event_line(
            r#"{"event":"done","ok":true,"installed":["moss"],"skipped":[],"failed":[]}"#,
        )
        .expect("应解析成功");
        assert_eq!(evt.ok, Some(true));
        assert_eq!(evt.installed.as_deref(), Some(&["moss".to_string()][..]));
        assert_eq!(evt.skipped.as_deref(), Some(&[][..]));
        assert!(evt.is_known_event());
    }

    #[test]
    fn parse_rejects_noise_and_unknown() {
        assert!(parse_event_line("").is_none());
        assert!(parse_event_line("   ").is_none());
        // stderr 混进管道的人类文本
        assert!(parse_event_line("manifest: D:\\x\\manifest.json (version=1)").is_none());
        // 合法 JSON 但不是协议事件
        assert!(parse_event_line(r#"{"foo":1}"#).is_none());
        assert!(parse_event_line(r#"{"event":""}"#).is_none());
        let odd = parse_event_line(r#"{"event":"weird"}"#).unwrap();
        assert!(!odd.is_known_event(), "未知事件名要能被识别出来并记诊断");
    }

    #[test]
    fn parse_ignores_trailing_cr() {
        // Windows 管道里可能出现 \r\n，行尾 \r 不能破坏解析
        let evt = parse_event_line("{\"event\":\"start\",\"item\":\"moss\"}\r").unwrap();
        assert_eq!(evt.event, "start");
    }

    #[test]
    fn precheck_cuda_needs_three_times_plus_margin() {
        let pre = precheck_disk_with(Some(MANIFEST), Some(u64::MAX), "moss", "cuda");
        assert!(pre.ok);
        assert_eq!(pre.manifest_entries, 3);
        assert_eq!(
            pre.required_bytes,
            651_834_262 * EXPANSION_FACTOR + SAFETY_MARGIN
        );
    }

    #[test]
    fn precheck_uses_unpacked_size_when_declared() {
        // OCR 的包带 unpacked_size：峰值 = 下载 + 解压(临时) + 解压(dest) + 余量，
        // 不能用 size × 3 —— 那样对 24k 个文件的 deps 会低估。
        let manifest = r#"{"items":[{"id":"deps-ocr","flavor":"cpu","sha256":"a","size":1000,"unpacked_size":9000}]}"#;
        let pre = precheck_disk_with(Some(manifest), Some(u64::MAX), "deps-ocr", "cpu");
        assert_eq!(pre.required_bytes, 1000 + 9000 + 9000 + SAFETY_MARGIN);
    }

    #[test]
    fn precheck_falls_back_when_unpacked_size_absent_or_zero() {
        let manifest = r#"{"items":[{"id":"deps-ocr","flavor":"cpu","sha256":"a","size":1000,"unpacked_size":0}]}"#;
        let pre = precheck_disk_with(Some(manifest), Some(u64::MAX), "deps-ocr", "cpu");
        assert_eq!(
            pre.required_bytes,
            1000 * EXPANSION_FACTOR + SAFETY_MARGIN,
            "unpacked_size 为 0 应退回经验值"
        );
    }

    #[test]
    fn precheck_auto_takes_largest_flavor() {
        let pre = precheck_disk_with(Some(MANIFEST), None, "moss", "auto");
        assert!(pre.ok, "查不到可用空间时不拦");
        assert_eq!(
            pre.required_bytes,
            651_834_262 * EXPANSION_FACTOR + SAFETY_MARGIN,
            "auto 要按最大 flavor 估算，否则会放过 cuda 的空间不足"
        );
        assert!(pre.free_bytes.is_none());
        assert!(pre.message.contains("跳过判定"));
    }

    #[test]
    fn precheck_fails_when_free_is_short() {
        let pre = precheck_disk_with(Some(MANIFEST), Some(1024 * 1024 * 1024), "moss", "cuda");
        assert!(!pre.ok);
        assert!(pre.message.contains("磁盘空间不足"));
    }

    #[test]
    fn precheck_without_manifest_does_not_block() {
        let pre = precheck_disk_with(None, Some(0), "moss", "cpu");
        assert!(pre.ok, "清单都读不到时不该拦用户，交给脚本报错");
        assert_eq!(pre.manifest_entries, 0);
        assert!(pre.message.contains("跳过磁盘预检"));
    }

    #[test]
    fn precheck_unknown_id_does_not_block() {
        let pre = precheck_disk_with(Some(MANIFEST), Some(0), "deps-ocr", "cpu");
        assert!(pre.ok);
        assert_eq!(pre.manifest_entries, 0);
        assert!(pre.message.contains("deps-ocr"));
    }

    #[test]
    fn precheck_bad_json_does_not_block() {
        // 清单损坏 → parse_manifest_items 返回空 → 放行，由脚本给出真实的清单错误
        let pre = precheck_disk_with(Some("{not json"), Some(0), "moss", "cpu");
        assert!(pre.ok);
        assert_eq!(pre.manifest_entries, 0);
    }

    #[test]
    fn bootstrap_candidates_contain_repo_layouts() {
        let cands = bootstrap_candidates(Path::new("F:\\repo\\runtime"));
        assert!(
            cands
                .iter()
                .any(|p| p.ends_with("scripts\\bootstrap_runtime.ps1")),
            "候选里必须含仓库布局"
        );
        // 开发形态：cwd 是 src-tauri，要能找到上一级的 scripts
        assert!(cands.iter().any(|p| p.to_string_lossy().contains("..") || p.exists()
            || p.to_string_lossy().contains("bootstrap_runtime.ps1")));
    }

    #[test]
    fn find_bootstrap_script_picks_first_existing() {
        let dir = std::env::temp_dir().join(format!("gsa_boot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.ps1");
        let real = dir.join("real.ps1");
        std::fs::write(&real, "# x").unwrap();
        let found = find_bootstrap_script(&[missing, real.clone()]);
        assert_eq!(found.as_deref(), Some(real.as_path()));
        assert!(find_bootstrap_script(&[dir.join("none.ps1")]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_tail_keeps_last_part_only() {
        let mut buf = String::new();
        for _ in 0..1000 {
            push_tail(&mut buf, "0123456789");
        }
        assert!(buf.len() <= STDERR_TAIL_LIMIT + 12, "尾部缓冲不能无限增长");
        assert!(buf.ends_with("0123456789"));
    }

    #[test]
    fn installer_rejects_concurrent_runs() {
        // 直接验证 running 语义，不起子进程
        let installer = RuntimeInstaller::default();
        assert!(!installer.is_running());
        assert!(installer.running.swap(true, Ordering::SeqCst) == false);
        assert!(installer.is_running());
        {
            let _g = RunningGuard(&installer);
        }
        assert!(!installer.is_running(), "guard 落地后必须放开");
    }

    // ─── 汇总（纯函数） ───────────────────────────────────

    fn exit_status(code: i32) -> std::process::ExitStatus {
        // 造 ExitStatus 只能靠真进程：起一个必定以该码退出的 powershell 太慢，
        // 这里借 cmd 的 /c exit 快速拿到真实 ExitStatus
        #[cfg(windows)]
        {
            crate::subprocess::hidden("cmd")
                .args(["/c", "exit", &code.to_string()])
                .status()
                .expect("cmd 应可用")
        }
        #[cfg(not(windows))]
        {
            crate::subprocess::hidden("sh")
                .args(["-c", &format!("exit {code}")])
                .status()
                .expect("sh 应可用")
        }
    }

    #[test]
    fn summarize_success_counts_items() {
        let s = summarize(StreamOutcome {
            status: Some(exit_status(0)),
            done: InstallEvent {
                event: "done".into(),
                ok: Some(true),
                installed: Some(vec!["moss".into()]),
                skipped: Some(vec![]),
                failed: Some(vec![]),
                ..Default::default()
            },
            stderr_tail: String::new(),
        });
        assert!(s.ok);
        assert_eq!(s.exit_code, 0);
        assert_eq!(s.installed, vec!["moss".to_string()]);
        assert!(s.message.contains("新增 1 项"));
    }

    #[test]
    fn summarize_partial_failure_uses_failed_list() {
        let s = summarize(StreamOutcome {
            status: Some(exit_status(3)),
            done: InstallEvent {
                event: "done".into(),
                ok: Some(false),
                failed: Some(vec!["deps-ocr".into()]),
                ..Default::default()
            },
            stderr_tail: "无关的尾巴".into(),
        });
        assert!(!s.ok);
        assert_eq!(s.exit_code, 3);
        assert!(s.message.contains("退出码 3"));
        assert!(s.message.contains("deps-ocr"), "明细要取 done.failed 而不是 stderr");
    }

    #[test]
    fn summarize_falls_back_to_stderr_tail() {
        // 脚本被外力杀掉：没有 done 事件，只能拿 stderr 最后一行说话
        let s = summarize(StreamOutcome {
            status: Some(exit_status(4)),
            done: InstallEvent::default(),
            stderr_tail: "第一行\n最后一行是真话".into(),
        });
        assert!(!s.ok);
        assert_eq!(s.exit_code, 4);
        assert!(s.message.contains("最后一行是真话"));
    }

    #[test]
    fn summarize_cancel_reports_cancelled() {
        let s = summarize(StreamOutcome {
            status: None,
            done: InstallEvent {
                event: "done".into(),
                installed: Some(vec!["moss".into()]),
                ..Default::default()
            },
            stderr_tail: String::new(),
        });
        assert!(s.cancelled);
        assert!(!s.ok);
        assert_eq!(s.installed, vec!["moss".to_string()], "取消前装完的条目要如实回传");
    }

    #[test]
    fn summarize_trusts_exit_code_over_event_claim() {
        // 脚本若"说自己成功但退出码非 0"，以退出码为准
        let s = summarize(StreamOutcome {
            status: Some(exit_status(2)),
            done: InstallEvent {
                event: "done".into(),
                ok: Some(true),
                ..Default::default()
            },
            stderr_tail: String::new(),
        });
        assert!(!s.ok);
        assert_eq!(s.exit_code, 2);
    }

    // ─── 端到端：真脚本 + 真子进程（离线，不下载） ─────────

    /// 仓库里的安装脚本路径（`src-tauri/../scripts/bootstrap_runtime.ps1`）
    fn repo_script() -> Option<PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join(BOOTSTRAP_SCRIPT_REL);
        p.is_file().then_some(p)
    }

    /// 跑真实脚本，用一个"清单里没有的条目"让它在**不联网**的前提下走完
    /// 解析 → 选条目 → item_fail → done 全流程（退出码 3）。
    /// 钉住的是：进程能起、stdout 逐行能解、事件顺序合理、退出码映射正确。
    #[test]
    #[cfg(windows)]
    fn end_to_end_bogus_item_reports_failure() {
        let Some(script) = repo_script() else {
            eprintln!("跳过：仓库里没有 bootstrap_runtime.ps1");
            return;
        };
        let tag = format!("gsa_rt_e2e_{}", std::process::id());
        let dir = std::env::temp_dir().join(&tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        let manifest = dir.join("manifest.json");
        std::fs::write(&manifest, MANIFEST).unwrap();

        let child = crate::subprocess::hidden("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .args(["-Items", "deps-ocr", "-Flavor", "cpu", "-RuntimeDir"])
            .arg(dir.join("runtime"))
            .arg("-Manifest")
            .arg(&manifest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .expect("powershell 应可启动");

        let events: Arc<Mutex<Vec<InstallEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let slot: Mutex<Option<Child>> = Mutex::new(None);
        let cancelled = AtomicBool::new(false);

        let outcome = stream_child(
            child,
            &slot,
            &cancelled,
            move |evt| sink.lock().unwrap().push(evt),
            &|| {},
        )
        .expect("读管道不应失败");

        let summary = summarize(outcome);
        let seen = events.lock().unwrap().clone();
        let kinds: Vec<&str> = seen.iter().map(|e| e.event.as_str()).collect();

        assert_eq!(summary.exit_code, 3, "清单里没有该条目 → 契约退出码 3");
        assert!(!summary.ok);
        assert!(summary.failed.contains(&"deps-ocr".to_string()));
        assert_eq!(kinds.last(), Some(&"done"), "最后一条必须是 done");
        assert!(
            kinds.iter().any(|k| *k == "item_fail"),
            "应出现 item_fail，实际：{kinds:?}"
        );
        assert!(
            summary.message.contains("deps-ocr"),
            "失败文案要带条目名：{}",
            summary.message
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 取消路径：起一个要睡 5 秒的子进程冒充"安装中"，在它跑完之前置取消标志 + kill，
    /// 验证 stream_child 的等待循环会立刻退出、且如实返回"没有退出码"。
    /// （不用真实脚本：真脚本要么秒退、要么真去下载，两个都不适合钉这条时序。）
    #[test]
    #[cfg(windows)]
    fn cancel_stops_waiting() {
        let Some(_script) = repo_script() else {
            eprintln!("跳过：仓库里没有 bootstrap_runtime.ps1");
            return;
        };
        // 用一个睡 5 秒的子进程模拟"安装中"，验证 cancel 后 stream_child 立即返回
        let child = crate::subprocess::hidden("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 5"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .expect("powershell 应可启动");

        let slot: Mutex<Option<Child>> = Mutex::new(None);
        let slot_ref = &slot;
        let cancelled = AtomicBool::new(false);

        // 150ms 后置取消标志（模拟用户点"取消"）
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(150));
                cancelled.store(true, Ordering::SeqCst);
                if let Ok(mut g) = slot_ref.lock() {
                    if let Some(c) = g.as_mut() {
                        let _ = c.kill();
                    }
                }
            });
            let started = std::time::Instant::now();
            let outcome = stream_child(
                child,
                slot_ref,
                &cancelled,
                |_evt| {},
                &|| {},
            )
            .expect("读管道不应失败");
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_secs(4),
                "取消后应立即返回，实际等了 {:?}",
                elapsed
            );
            assert!(outcome.status.is_none(), "被取消时拿不到退出码");
        });
    }
}
