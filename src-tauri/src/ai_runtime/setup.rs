//! 首启准备页的判定层。
//!
//! 背景（真机反馈 2026-10-31）：应用启动时运行时可能还没装（一键安装发生在启动之后），
//! 于是四张自检卡全是「未就绪」，用户只能重启。这个模块提供**一次调用拿全**的判定：
//! 哪些条目已就绪、哪些还要下、要下多少、磁盘够不够、安装脚本在不在。
//!
//! 判定口径（用户 2026-11-01 拍板）：
//! - **必需**：`moss`（CPU 或 Vulkan 版本皆可，CUDA 属可选增强）、`models-moss`、
//!   `deps-ocr`、`deps-embed`；
//! - **可选**：`deps-ocr-gpu`、`deps-ocr-cuda-libs`（没有就走 CPU，不阻塞放行）；
//! - 已就绪的老用户**完全不出准备页**（`ready == true` 时前端直接渲染主界面）。

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Manager;

use super::backend_probe::{find_manifest, manifest_candidates, probe_machine, MachineCaps};
use super::config::{resolve_runtime_dir, RuntimeConfig};
use super::installed_state::{parse_installed, parse_manifest_items, ManifestEntry, INSTALLED_FILE};
use super::runtime_install::DiskPrecheck;

/// 必需条目：缺任何一项都不能进主界面
pub const REQUIRED_IDS: [&str; 4] = ["moss", "models-moss", "deps-ocr", "deps-embed"];

/// 开发期跳过开关：设了 `GSA_SKIP_SETUP=1` 就永远当"已就绪"（调试与截图要用）
const SKIP_ENV: &str = "GSA_SKIP_SETUP";

/// 准备页清单里的一行
#[derive(Debug, Clone, Serialize)]
pub struct SetupItem {
    /// 清单条目 id（`moss` / `models-moss` / `deps-ocr` / ...）
    pub id: String,
    /// 用户可读名称
    pub label: String,
    /// 选中要装的后端（无 flavor 条目为空串）
    pub flavor: String,
    /// 该条目的下载体积（字节）
    pub size: u64,
    /// ready（已就绪）/ update（文件在但版本不同）/ missing（缺）
    pub state: String,
    /// 缺了就不放行
    pub required: bool,
    /// 补充说明（可选条目的处置建议）
    pub note: String,
}

/// 启动门控的判定结果
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    /// 必需项全部就绪（准备页可以不放行）
    pub ready: bool,
    /// 开发期跳过（`GSA_SKIP_SETUP=1`）
    pub skip: bool,
    /// 运行时目录（展示用，用户排查时要能照着找）
    pub runtime_dir: String,
    /// 安装脚本路径；`None` = 打包形态下脚本没随包发布，准备页只能报错
    pub script: Option<String>,
    /// 清单条目（含已就绪的，前端按状态分区显示）
    pub items: Vec<SetupItem>,
    /// 缺的必需条目 id
    pub missing_required: Vec<String>,
    /// 还要下载的字节数（含可选条目；不含解压展开余量）
    pub pending_bytes: u64,
    /// 磁盘预检（按 pending 估算，已扣掉已装项）
    pub disk: DiskPrecheck,
    /// 判定本身失败时的原因（此时 `ready = false`，前端按失败态展示）
    pub error: Option<String>,
}

impl RuntimeStatus {
    /// 判定失败时的兜底：宁可停在准备页报错，也别悄悄放行进主界面
    pub fn failed(message: String) -> Self {
        RuntimeStatus {
            ready: false,
            skip: false,
            runtime_dir: String::new(),
            script: None,
            items: Vec::new(),
            missing_required: Vec::new(),
            pending_bytes: 0,
            disk: DiskPrecheck::default(),
            error: Some(message),
        }
    }
}

// ─── 判定 ─────────────────────────────────────────────────

fn label_of(id: &str) -> &str {
    match id {
        "moss" => "MOSS 转写运行时",
        "models-moss" => "MOSS 语音模型",
        "deps-ocr" => "OCR 运行时",
        "deps-ocr-gpu" => "OCR GPU 加速库",
        "deps-ocr-cuda-libs" => "CUDA 运行库",
        "deps-embed" => "向量融合运行时",
        other => other,
    }
}

fn dir_has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// 条目在磁盘上是否真的可用（语义判据，不看 `.installed.json` 的 sha）
///
/// 只看 sha 不够：安装器换了版本会留下旧记录，而文件被删了记录还在。两边都看才靠谱。
fn exists_on_disk(id: &str, config: &RuntimeConfig, runtime_dir: &Path) -> bool {
    match id {
        "moss" => super::moss_paths(config, runtime_dir).0.is_file(),
        "models-moss" => super::moss_paths(config, runtime_dir).1.is_file(),
        "deps-ocr" => {
            let (_, deps, _, worker) = super::ocr_paths(config, runtime_dir);
            worker.is_file() && dir_has_entries(&deps)
        }
        // GPU 加速库是可选增量：只看目录有没有东西
        "deps-ocr-gpu" => dir_has_entries(&super::resolve_path(super::OCR_DEPS_GPU_REL, runtime_dir)),
        // pip 装的 CUDA 运行库：8 个轮子解出来是一堆 nvidia-* 目录，>= 4 个才算装了
        "deps-ocr-cuda-libs" => {
            let dir = super::resolve_path(super::OCR_DEPS_GPU_REL, runtime_dir).join("nvidia");
            std::fs::read_dir(dir)
                .map(|it| it.filter(|e| e.as_ref().map(|e| e.path().is_dir()).unwrap_or(false)).count() >= 4)
                .unwrap_or(false)
        }
        "deps-embed" => {
            let (_, worker, deps, model, tokenizer) = super::embed_paths(config, runtime_dir);
            worker.is_file() && dir_has_entries(&deps) && model.is_file() && tokenizer.is_file()
        }
        _ => false,
    }
}

/// 同一 id 在清单里可能有多条（moss 的 cpu/vulkan/cuda）。
/// 选哪条：已装过就用已装的那个 flavor，否则按本机能力 cuda → vulkan → cpu。
fn pick_entry<'a>(entries: &'a [ManifestEntry], installed_flavor: Option<&str>, caps: &MachineCaps) -> Option<&'a ManifestEntry> {
    if let Some(f) = installed_flavor {
        if let Some(e) = entries.iter().find(|e| e.flavor.eq_ignore_ascii_case(f)) {
            return Some(e);
        }
    }
    let pref: &[&str] = if caps.cuda_runnable() {
        &["cuda", "vulkan", "cpu"]
    } else if caps.vulkan {
        &["vulkan", "cpu"]
    } else {
        &["cpu"]
    };
    for want in pref {
        if let Some(e) = entries.iter().find(|e| e.flavor.eq_ignore_ascii_case(want)) {
            return Some(e);
        }
    }
    entries.first()
}

fn note_of(id: &str, caps: &MachineCaps) -> String {
    match id {
        "deps-ocr-gpu" | "deps-ocr-cuda-libs" if !caps.cuda => "本机没有 NVIDIA 显卡，可跳过".into(),
        "deps-ocr-gpu" | "deps-ocr-cuda-libs" => "可选：让 OCR 走 GPU".into(),
        "moss" if caps.cuda_runnable() => "可选：换成 CUDA 版本更快".into(),
        _ => String::new(),
    }
}

/// 纯函数版判定（不依赖 Tauri，便于单测）
pub fn evaluate_status(
    runtime_dir: &Path,
    config: &RuntimeConfig,
    manifest_text: Option<&str>,
    installed_text: Option<&str>,
    caps: &MachineCaps,
    script: Option<&Path>,
) -> RuntimeStatus {
    let entries = manifest_text.map(parse_manifest_items).unwrap_or_default();
    let installed = installed_text.map(parse_installed).unwrap_or_default();

    // 按 id 分组，保持清单顺序（清单是发布脚本按依赖顺序生成的：先运行时后模型）
    let mut ids: Vec<String> = Vec::new();
    for e in &entries {
        if !ids.contains(&e.id) {
            ids.push(e.id.clone());
        }
    }

    let mut items = Vec::new();
    let mut missing_required = Vec::new();
    let mut pending_bytes = 0u64;

    for id in &ids {
        let group: Vec<ManifestEntry> = entries.iter().filter(|e| &e.id == id).cloned().collect();
        let rec = installed.get(id);
        let picked = pick_entry(&group, rec.map(|r| r.flavor.as_str()), caps);
        let Some(picked) = picked else { continue };

        let required = REQUIRED_IDS.contains(&id.as_str());
        let on_disk = exists_on_disk(id, config, runtime_dir);
        // 记录里的 sha 与清单一致 = 已是最新；文件在但 sha 不同 = 有更新
        let sha_match = rec
            .map(|r| !r.sha256.trim().is_empty() && r.sha256.trim().eq_ignore_ascii_case(picked.sha256.trim()))
            .unwrap_or(false);
        let state = if on_disk && (sha_match || picked.sha256.trim().is_empty()) {
            "ready"
        } else if on_disk {
            "update"
        } else {
            "missing"
        };

        if state != "ready" {
            pending_bytes = pending_bytes.saturating_add(picked.size);
            if required {
                missing_required.push(id.clone());
            }
        }

        items.push(SetupItem {
            id: id.clone(),
            label: label_of(id).to_string(),
            flavor: picked.flavor.clone(),
            size: picked.size,
            state: state.to_string(),
            required,
            note: note_of(id, caps),
        });
    }

    let script_path = script.map(|p| p.display().to_string());
    // 缺脚本时无法自动补齐：判定成"未就绪"并让前端把原因说清楚
    let ready = missing_required.is_empty() && script_path.is_some();

    let disk = super::runtime_install::precheck_pending(manifest_text, runtime_dir, &ids, "auto", false);

    RuntimeStatus {
        ready,
        skip: false,
        runtime_dir: runtime_dir.display().to_string(),
        script: script_path,
        items,
        missing_required,
        pending_bytes,
        disk,
        error: None,
    }
}

// ─── Tauri 命令 ───────────────────────────────────────────

fn skip_requested() -> bool {
    std::env::var(SKIP_ENV)
        .map(|v| !v.trim().is_empty() && v.trim() != "0")
        .unwrap_or(false)
}

fn status_impl(app: &tauri::AppHandle) -> RuntimeStatus {
    let runtime_dir = resolve_runtime_dir(app.path().app_data_dir().ok().map(|d| d.join("runtime")));
    let config = RuntimeConfig::load(&runtime_dir).unwrap_or_default();
    let manifest_path: Option<PathBuf> =
        find_manifest(&manifest_candidates(app, &runtime_dir));
    let manifest_text = manifest_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok());
    let installed_text = std::fs::read_to_string(runtime_dir.join(INSTALLED_FILE)).ok();
    let script = super::runtime_install::find_bootstrap_script(
        &super::runtime_install::bootstrap_candidates(&runtime_dir),
    );
    let caps = probe_machine();

    let mut status = evaluate_status(
        &runtime_dir,
        &config,
        manifest_text.as_deref(),
        installed_text.as_deref(),
        &caps,
        script.as_deref(),
    );
    // 清单都没有时上面会得到空条目表（ready 会因缺脚本或空 missing 而误判），显式拦一道
    if manifest_text.is_none() {
        status.ready = false;
        status.error = Some("找不到运行时清单（resources/runtime/manifest.json）".into());
    }
    status.skip = skip_requested();
    status
}

/// 启动门控判定：一次调用拿全「能不能进主界面 / 还要下什么 / 磁盘够不够」。
#[tauri::command]
pub async fn runtime_status(app: tauri::AppHandle) -> RuntimeStatus {
    let handle = app.clone();
    match tauri::async_runtime::spawn_blocking(move || status_impl(&handle)).await {
        Ok(s) => s,
        Err(e) => RuntimeStatus::failed(format!("运行时判定失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(cuda: bool, vulkan: bool) -> MachineCaps {
        MachineCaps {
            cuda,
            vulkan,
            cuda_cc: if cuda { Some(89) } else { None },
        }
    }

    /// 造一个"全装好"的临时 runtime 目录，返回 (根目录, 清单文本)
    fn full_runtime(name: &str) -> (PathBuf, String) {
        let root = std::env::temp_dir().join(format!("gsa_setup_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for rel in [
            "bin/moss-transcribe.exe",
            "models/moss/moss-transcribe-q5_k.gguf",
            "worker/ocr_worker.py",
            "deps/onnxruntime/keep.txt",
            "worker/embed_worker.py",
            "deps_embed/onnxruntime/keep.txt",
            "models/embed/multilingual-e5-small/model_qint8_avx512_vnni.onnx",
            "models/embed/multilingual-e5-small/tokenizer.json",
        ] {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        let manifest = r#"{"version":"1","items":[
            {"id":"moss","flavor":"cuda","url":"u","sha256":"aaa","size":100,"dest":"bin"},
            {"id":"models-moss","url":"u","sha256":"bbb","size":200,"dest":"models/moss"},
            {"id":"deps-ocr","flavor":"cpu","url":"u","sha256":"ccc","size":300,"dest":"."},
            {"id":"deps-ocr-gpu","flavor":"cuda","url":"u","sha256":"ddd","size":400,"dest":"."},
            {"id":"deps-embed","url":"u","sha256":"eee","size":500,"dest":"."}
        ]}"#
        .to_string();
        let installed = r#"{"version":1,"items":{
            "moss":{"flavor":"cuda","sha256":"aaa"},
            "models-moss":{"flavor":"","sha256":"bbb"},
            "deps-ocr":{"flavor":"cpu","sha256":"ccc"},
            "deps-embed":{"flavor":"","sha256":"eee"}
        }}"#;
        std::fs::write(root.join(INSTALLED_FILE), installed).unwrap();
        (root, manifest)
    }

    #[test]
    fn ready_when_required_items_present() {
        let (root, manifest) = full_runtime("ready");
        let status = evaluate_status(
            &root,
            &RuntimeConfig::default(),
            Some(&manifest),
            Some(&std::fs::read_to_string(root.join(INSTALLED_FILE)).unwrap()),
            &caps(true, true),
            Some(Path::new("scripts/bootstrap_runtime.ps1")),
        );
        assert!(status.ready, "必需项齐了应当 ready: {:?}", status.missing_required);
        assert!(status.missing_required.is_empty());
        // 可选条目缺失只算 pending，不该阻塞
        let gpu = status.items.iter().find(|i| i.id == "deps-ocr-gpu").unwrap();
        assert_eq!(gpu.state, "missing");
        assert!(!gpu.required);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_model_blocks_ready() {
        let (root, manifest) = full_runtime("nomodel");
        std::fs::remove_file(root.join("models/moss/moss-transcribe-q5_k.gguf")).unwrap();
        let status = evaluate_status(
            &root,
            &RuntimeConfig::default(),
            Some(&manifest),
            Some(&std::fs::read_to_string(root.join(INSTALLED_FILE)).unwrap()),
            &caps(true, true),
            Some(Path::new("s.ps1")),
        );
        assert!(!status.ready);
        assert_eq!(status.missing_required, vec!["models-moss".to_string()]);
        let item = status.items.iter().find(|i| i.id == "models-moss").unwrap();
        assert_eq!(item.state, "missing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn flavor_follows_installed_record() {
        let (root, manifest) = full_runtime("flavor");
        // 记录里是 vulkan，但机器支持 cuda：应沿用已装的 vulkan（不能凭空换成 cuda）
        let installed = r#"{"version":1,"items":{"moss":{"flavor":"vulkan","sha256":"aaa"}}}"#;
        let manifest_v = manifest.replace("\"cuda\",\"url\"", "\"vulkan\",\"url\"");
        let status = evaluate_status(
            &root,
            &RuntimeConfig::default(),
            Some(&manifest_v),
            Some(installed),
            &caps(true, true),
            Some(Path::new("s.ps1")),
        );
        let moss = status.items.iter().find(|i| i.id == "moss").unwrap();
        assert_eq!(moss.flavor, "vulkan");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_script_is_not_ready() {
        let (root, manifest) = full_runtime("noscript");
        let status = evaluate_status(
            &root,
            &RuntimeConfig::default(),
            Some(&manifest),
            Some(&std::fs::read_to_string(root.join(INSTALLED_FILE)).unwrap()),
            &caps(false, false),
            None,
        );
        assert!(!status.ready, "没有安装脚本时不能放行（缺了就补不上）");
        assert!(status.script.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
