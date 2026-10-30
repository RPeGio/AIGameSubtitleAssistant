// ─── MOSS 运行时安装状态比对（.installed.json × manifest）──
// 契约：docs/RUNTIME_BOOTSTRAP_CONTRACT.md（安装器写、启动侧读）
//   安装器只负责解包 + 写入 runtime/.installed.json：
//     {"version":1,"items":{"moss":{"flavor":"vulkan","sha256":"…",
//       "source_version":"…","installed_at":"…"}}}
//   启动侧（本模块）比对"已装 vs 清单声明"，回答三个问题：
//     ① 装的东西还在吗（文件被删/目录被移）
//     ② 装的和清单是同一份吗（sha256；清单换版 ⇒ 提示更新）
//     ③ 装的是清单里还支持的 flavor 吗（某后端被下线）
//
// 为什么只认 sha256：zip 不是字节可复现的（docs/engines_vulkan_design.md §8.4，
// 包内 SOURCE.txt 记构建时刻），版本号相同不代表内容相同，哈希才可核对。
// 本模块纯函数 + 只读文件，不写任何东西（写 .installed.json 是安装器的事）。

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// 安装器写的状态文件名（runtime 目录下）
pub const INSTALLED_FILE: &str = ".installed.json";

/// 清单里模型条目的 id。
/// 模型与后端无关（cpu/cuda/vulkan 共用同一份 gguf），所以在清单里是独立条目、
/// 不带 flavor；二进制才是按 flavor 分三条的那个 `moss`。
pub const MODEL_ID: &str = "models-moss";

/// 清单里向量融合（embed）运行时的条目 id。
/// 与 flavor 无关：这 26 个包是纯 CPU 的 onnxruntime，没有 CUDA/DirectML 变体，
/// 所以全平台只有一条；它复用 OCR 包带来的 `python/python.exe`。
pub const EMBED_ID: &str = "deps-embed";

/// `.installed.json` 里单个条目的内容
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct InstalledItem {
    /// 装的是哪个后端（"cpu" / "cuda" / "vulkan"）
    pub flavor: String,
    /// 解包用的 zip 的 sha256（小写 64 位十六进制）
    pub sha256: String,
    /// 上游源码版本（安装器抄自 manifest/构建记录，可空）
    pub source_version: String,
    /// 安装时刻（ISO 8601，可空）
    pub installed_at: String,
}

/// 清单里一条可安装项（只取比对需要的字段）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ManifestEntry {
    pub id: String,
    pub flavor: String,
    pub sha256: String,
    pub size: u64,
    /// 解压后字节数（清单可选字段）。OCR 的 `deps` 有 2.4 万个文件、压缩率只有
    /// ~3 倍以内，用 `size × 3` 估预检会低估到装不下 —— 有真值就用真值。
    pub unpacked_size: Option<u64>,
}

/// 比对结论（给欢迎页/自检卡片展示）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallStatus {
    /// ok | update_available | not_installed | file_missing
    /// | flavor_not_in_manifest | manifest_missing | unknown
    pub state: String,
    /// 本地安装记录（没有记录时为 None）
    pub installed: Option<InstalledItem>,
    /// 清单里同 flavor 条目声明的 sha256 / size
    pub declared_sha256: Option<String>,
    pub declared_size: Option<u64>,
    /// 用户可读结论（前端直接显示，已含处置建议）
    pub message: String,
}

impl InstallStatus {
    fn new(state: &str, message: String) -> Self {
        InstallStatus {
            state: state.to_string(),
            installed: None,
            declared_sha256: None,
            declared_size: None,
            message,
        }
    }
}

// ─── 解析（容错，纯函数）─────────────────────────────────

/// 去掉 UTF-8 BOM：这两个文件都是"工具写出来的"（manifest 由发布脚本生成、
/// `.installed.json` 由安装器写），而 serde_json 见到 BOM 会直接判语法错误
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// 解析 `.installed.json`：`items` 是对象，键为 id。
/// 坏 JSON / 缺字段一律当"没有记录"，不报错（这类文件是安装器写的，
/// 启动侧不该因为一个字段缺失就让用户没法用 ASR）。
pub fn parse_installed(text: &str) -> BTreeMap<String, InstalledItem> {
    let mut out = BTreeMap::new();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(strip_bom(text)) else {
        return out;
    };
    let Some(items) = json.get("items").and_then(|v| v.as_object()) else {
        return out;
    };
    for (id, v) in items {
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        out.insert(
            id.clone(),
            InstalledItem {
                flavor: s("flavor").to_ascii_lowercase(),
                sha256: s("sha256").to_ascii_lowercase(),
                source_version: s("source_version"),
                installed_at: s("installed_at"),
            },
        );
    }
    out
}

/// 解析 manifest 的 `items` 数组（id / flavor / sha256 / size 四字段）
pub fn parse_manifest_items(text: &str) -> Vec<ManifestEntry> {
    let mut out = Vec::new();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(strip_bom(text)) else {
        return out;
    };
    let Some(items) = json.get("items").and_then(|v| v.as_array()) else {
        return out;
    };
    for item in items {
        let s = |k: &str| {
            item.get(k)
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        out.push(ManifestEntry {
            id: s("id"),
            flavor: s("flavor").to_ascii_lowercase(),
            sha256: s("sha256").to_ascii_lowercase(),
            size: item.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
            unpacked_size: item.get("unpacked_size").and_then(|v| v.as_u64()),
        });
    }
    out
}

// ─── 比对 ───────────────────────────────────────────────

/// 比对已装运行时与清单声明。
///
/// - `runtime_dir`：运行时的根目录（`.installed.json` 在此）
/// - `binary`：解析后的 moss 可执行文件（判断"装的东西还在不在"）
/// - `model`：解析后的 gguf 路径（None = 不检查模型）
/// - `manifest_path`：找到的清单路径（None = 没找到）
/// - `id`：清单里的条目 id（本项目固定 "moss"）
///
/// 检查顺序按"修起来的代价"排：先"没有可执行文件"，再"没有模型"（两者都等于跑不了），
/// 然后才是版本类结论（清单缺失 / 后端下线 / 有更新）——后者都能跑，只是需要提示。
pub fn evaluate(
    runtime_dir: &Path,
    binary: &Path,
    model: Option<&Path>,
    manifest_path: Option<&Path>,
    id: &str,
) -> InstallStatus {
    let installed_text = std::fs::read_to_string(runtime_dir.join(INSTALLED_FILE)).ok();
    let installed = installed_text
        .as_deref()
        .map(parse_installed)
        .unwrap_or_default()
        .get(id)
        .cloned();

    let binary_ok = binary.is_file();

    let Some(inst) = installed else {
        // 没有安装记录：文件在 ⇒ 手工安装/旧安装器（无从比对）；不在 ⇒ 真没装
        let mut st = if binary_ok {
            InstallStatus::new(
                "unknown",
                format!(
                    "未找到 {}（可能是手工安装或旧版安装器所装），无法比对版本",
                    INSTALLED_FILE
                ),
            )
        } else {
            InstallStatus::new("not_installed", "MOSS 运行时未安装".to_string())
        };
        st.declared_sha256 = declared(manifest_path, id, "").map(|e| e.sha256);
        st.declared_size = declared(manifest_path, id, "").map(|e| e.size);
        if !binary_ok {
            return st;
        }
        // 二进制在但模型没了，同样是跑不了——不能因为"没安装记录"就只报 unknown
        return with_model_check(st, model, "找到 MOSS 二进制（无安装记录）");
    };

    let declared_entry = declared(manifest_path, id, &inst.flavor);
    let mut st = InstallStatus {
        state: String::new(),
        installed: Some(inst.clone()),
        declared_sha256: declared_entry.as_ref().map(|e| e.sha256.clone()),
        declared_size: declared_entry.as_ref().map(|e| e.size),
        message: String::new(),
    };

    if !binary_ok {
        st.state = "file_missing".to_string();
        st.message = format!(
            "安装记录为 {} 版，但 {} 不存在：运行时目录被移动或文件被删，请重新安装 MOSS 运行时",
            flavor_label(&inst.flavor),
            binary.display()
        );
        return with_marker_note(st, binary, &inst);
    }

    // 二进制在、模型没了：状态上"已安装"但一说句话就炸，必须先报这个
    st = with_model_check(
        st,
        model,
        &format!("MOSS 运行时已装（{} 版）", flavor_label(&inst.flavor)),
    );
    if st.state == "model_missing" {
        return with_marker_note(st, binary, &inst);
    }

    if manifest_path.is_none() {
        st.state = "manifest_missing".to_string();
        st.message = format!(
            "已装 {} 版 MOSS 运行时，但没找到清单，无法核对是否有更新",
            flavor_label(&inst.flavor)
        );
        return with_marker_note(st, binary, &inst);
    }

    let Some(entry) = declared_entry else {
        st.state = "flavor_not_in_manifest".to_string();
        st.message = format!(
            "已装的是 {} 版，但清单里没有这个后端（该后端可能已下线），建议在设置里改选其它后端",
            flavor_label(&inst.flavor)
        );
        return with_marker_note(st, binary, &inst);
    };

    if entry.sha256.is_empty() || inst.sha256.is_empty() {
        st.state = "unknown".to_string();
        st.message = format!(
            "已装 {} 版 MOSS 运行时，但安装记录/清单缺 sha256，无法核对",
            flavor_label(&inst.flavor)
        );
        return with_marker_note(st, binary, &inst);
    }

    if entry.sha256 == inst.sha256 {
        st.state = "ok".to_string();
        st.message = format!(
            "MOSS 运行时已就绪（{} 版，与清单一致）",
            flavor_label(&inst.flavor)
        );
    } else {
        st.state = "update_available".to_string();
        st.message = format!(
            "MOSS 运行时是旧版（已装 {} 版，清单声明了更新的一份），建议重新安装以获取更新",
            flavor_label(&inst.flavor)
        );
    }
    with_marker_note(st, binary, &inst)
}

/// 模型缺失时把状态改成 `model_missing`（二进制在、模型不在 = 跑不了）
fn with_model_check(st: InstallStatus, model: Option<&Path>, ctx: &str) -> InstallStatus {
    let Some(model) = model else { return st };
    if model.is_file() {
        return st;
    }
    InstallStatus {
        state: "model_missing".to_string(),
        message: format!(
            "{ctx}，但模型文件不存在：{}。请重新安装运行时补全模型（清单条目 {}），或把模型放回该路径",
            model.display(),
            MODEL_ID
        ),
        ..st
    }
}

/// 在清单里找同 id（且同 flavor，flavor 空 = 任意）的条目
fn declared(manifest_path: Option<&Path>, id: &str, flavor: &str) -> Option<ManifestEntry> {
    let text = std::fs::read_to_string(manifest_path?).ok()?;
    parse_manifest_items(&text)
        .into_iter()
        .find(|e| e.id == id && (flavor.is_empty() || e.flavor.eq_ignore_ascii_case(flavor)))
}

/// `.backend` 标记与安装记录不符时补一句提醒（手工覆盖过文件才会出现）
fn with_marker_note(mut st: InstallStatus, binary: &Path, inst: &InstalledItem) -> InstallStatus {
    let dir = binary.parent().unwrap_or_else(|| Path::new("."));
    let marker = std::fs::read_to_string(dir.join(".backend"))
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    if let Some(m) = marker {
        if !inst.flavor.is_empty() && m != inst.flavor {
            st.message.push_str(&format!(
                "（注意：.backend 标记为 {m}，与安装记录的 {} 不符，可能被手工替换过文件）",
                inst.flavor
            ));
        }
    }
    st
}

/// flavor 给人看的中文名
fn flavor_label(flavor: &str) -> &str {
    match flavor {
        "cuda" => "CUDA",
        "vulkan" => "Vulkan",
        "cpu" => "CPU",
        "" => "(未知)",
        other => other,
    }
}

// ─── Tauri 命令 ─────────────────────────────────────────

/// Tauri 命令：已装 MOSS 运行时的状态比对。
/// 欢迎页/设置页据此提示"有更新 / 文件缺失 / 未安装"（只读，不改任何文件）。
#[tauri::command]
pub fn probe_moss_install(app: tauri::AppHandle) -> InstallStatus {
    use tauri::Manager;

    let runtime_dir = super::config::resolve_runtime_dir(
        app.path().app_data_dir().ok().map(|d| d.join("runtime")),
    );
    let config = super::RuntimeConfig::load(&runtime_dir).unwrap_or_default();
    // 路径解析与 provider 同源（空配置回退标准布局），否则会出现
    // "卡片说未安装、转写其实能跑"这种两套说法
    let (binary, model) = super::moss_paths(&config, &runtime_dir);
    let manifest = super::backend_probe::find_manifest(&super::backend_probe::manifest_candidates(
        &app,
        &runtime_dir,
    ));
    evaluate(
        &runtime_dir,
        &binary,
        Some(&model),
        manifest.as_deref(),
        "moss",
    )
}

/// Tauri 命令：已装 embed（向量融合）运行时的状态比对。
///
/// "二进制"取 `python/python.exe`（embed 复用 OCR 包的解释器），"模型"取 onnx 文件：
/// 这两者任一缺失都等于向量融合跑不了，所以一次比对就能给出可读结论。
#[tauri::command]
pub fn probe_embed_install(app: tauri::AppHandle) -> InstallStatus {
    use tauri::Manager;

    let runtime_dir = super::config::resolve_runtime_dir(
        app.path().app_data_dir().ok().map(|d| d.join("runtime")),
    );
    let config = super::RuntimeConfig::load(&runtime_dir).unwrap_or_default();
    let (python, _worker, _deps, model, _tokenizer) =
        super::embed_paths(&config, &runtime_dir);
    let manifest = super::backend_probe::find_manifest(&super::backend_probe::manifest_candidates(
        &app,
        &runtime_dir,
    ));
    evaluate(
        &runtime_dir,
        &python,
        Some(&model),
        manifest.as_deref(),
        EMBED_ID,
    )
}

// ─── 单测 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个临时 runtime 目录（`.installed.json` + 假 exe + `.backend`）
    fn setup(tag: &str, installed: Option<&str>, marker: Option<&str>, exe: bool) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("gsa_inst_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if let Some(t) = installed {
            std::fs::write(root.join(INSTALLED_FILE), t).unwrap();
        }
        if exe {
            std::fs::write(root.join("moss-transcribe.exe"), b"x").unwrap();
        }
        if let Some(m) = marker {
            std::fs::write(root.join(".backend"), m).unwrap();
        }
        root
    }

    fn manifest_with(tag: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("gsa_inst_manifest_{tag}_{}.json", std::process::id()));
        std::fs::write(&p, body).unwrap();
        p
    }

    const INSTALLED_VK: &str = r#"{"version":1,"items":{"moss":{"flavor":"vulkan",
        "sha256":"e848847289982b8322262341aa133e40b8f152ee563b8af63e3412e4eb6a89b5",
        "source_version":"190a569c","installed_at":"2026-10-24T03:00:00Z"}}}"#;

    #[test]
    fn parse_installed_reads_items_map() {
        let m = parse_installed(INSTALLED_VK);
        let it = m.get("moss").expect("moss 条目");
        assert_eq!(it.flavor, "vulkan");
        assert_eq!(it.source_version, "190a569c");
        assert!(it.sha256.starts_with("e8488472"));
    }

    #[test]
    fn parse_installed_tolerates_garbage() {
        assert!(parse_installed("{not json").is_empty());
        assert!(parse_installed(r#"{"items":[]}"#).is_empty());
        // 缺字段：条目仍在，字段为空串（后续会判 unknown，不会 panic）
        let m = parse_installed(r#"{"items":{"moss":{}}}"#);
        assert_eq!(m.get("moss").unwrap().flavor, "");
    }

    #[test]
    fn evaluate_ok_when_sha_matches() {
        let dir = setup("ok", Some(INSTALLED_VK), Some("vulkan"), true);
        let mf = manifest_with(
            "ok",
            r#"{"version":"1","items":[{"id":"moss","flavor":"vulkan","url":"u",
                "sha256":"e848847289982b8322262341aa133e40b8f152ee563b8af63e3412e4eb6a89b5",
                "size":16139894,"dest":"bin"}]}"#,
        );
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, Some(&mf), "moss");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "ok");
        assert_eq!(st.declared_size, Some(16139894));
        assert!(st.message.contains("已就绪"), "{}", st.message);
    }

    #[test]
    fn evaluate_update_available_when_sha_differs() {
        let dir = setup("upd", Some(INSTALLED_VK), Some("vulkan"), true);
        let mf = manifest_with(
            "upd",
            r#"{"items":[{"id":"moss","flavor":"vulkan","sha256":"deadbeef","size":2}]}"#,
        );
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, Some(&mf), "moss");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "update_available");
        assert_eq!(st.declared_sha256.as_deref(), Some("deadbeef"));
        assert!(st.message.contains("旧版"), "{}", st.message);
    }

    #[test]
    fn evaluate_file_missing_beats_version_check() {
        // 记录说装了，但 exe 不在了（目录被删/被移动）⇒ 最该修的是"装回来"
        let dir = setup("miss", Some(INSTALLED_VK), Some("vulkan"), false);
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, None, "moss");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(st.state, "file_missing");
        assert!(st.message.contains("重新安装"), "{}", st.message);
    }

    #[test]
    fn evaluate_not_installed_when_no_record_and_no_exe() {
        let dir = setup("none", None, None, false);
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, None, "moss");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(st.state, "not_installed");
    }

    #[test]
    fn evaluate_unknown_when_exe_present_but_no_record() {
        // 手工解压/旧安装器：文件在、没记录 ⇒ 不谎报"未安装"
        let dir = setup("manual", None, Some("cpu"), true);
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, None, "moss");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(st.state, "unknown");
        assert!(st.message.contains(INSTALLED_FILE), "{}", st.message);
    }

    #[test]
    fn evaluate_flavor_not_in_manifest() {
        let dir = setup("flav", Some(INSTALLED_VK), Some("vulkan"), true);
        let mf = manifest_with(
            "flav",
            r#"{"items":[{"id":"moss","flavor":"cpu","sha256":"aa","size":1}]}"#,
        );
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, Some(&mf), "moss");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "flavor_not_in_manifest");
        assert!(st.message.contains("Vulkan"), "{}", st.message);
    }

    #[test]
    fn evaluate_manifest_missing_still_reports_installed() {
        let dir = setup("nomf", Some(INSTALLED_VK), Some("vulkan"), true);
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, None, "moss");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(st.state, "manifest_missing");
        assert_eq!(st.installed.as_ref().unwrap().flavor, "vulkan");
    }

    #[test]
    fn marker_mismatch_is_reported_in_message() {
        // 安装记录说 cuda，.backend 却是 vulkan ⇒ 提示被手工替换过
        let dir = setup("mark", Some(INSTALLED_VK), Some("cuda"), true);
        let mf = manifest_with(
            "mark",
            r#"{"items":[{"id":"moss","flavor":"vulkan","sha256":"e848847289982b8322262341aa133e40b8f152ee563b8af63e3412e4eb6a89b5","size":1}]}"#,
        );
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, Some(&mf), "moss");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "ok");
        assert!(st.message.contains(".backend 标记为 cuda"), "{}", st.message);
    }

    // ─── 模型缺失态（清单条目 models-moss 落地后新增）────────

    /// 造一份"二进制与记录都正常、清单也一致"的现场，返回 (dir, manifest)
    fn setup_healthy(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = setup(tag, Some(INSTALLED_VK), Some("vulkan"), true);
        let mf = manifest_with(
            tag,
            r#"{"version":"1","items":[{"id":"moss","flavor":"vulkan","url":"u",
                "sha256":"e848847289982b8322262341aa133e40b8f152ee563b8af63e3412e4eb6a89b5",
                "size":16139894,"dest":"bin"}]}"#,
        );
        (dir, mf)
    }

    #[test]
    fn evaluate_model_missing_beats_version_ok() {
        // 这正是修复前那个"装完显示 ok、一说话就炸"的现场
        let (dir, mf) = setup_healthy("modmiss");
        let model = dir.join("models").join("moss").join("moss-transcribe-q5_k.gguf");
        let st = evaluate(
            &dir,
            &dir.join("moss-transcribe.exe"),
            Some(&model),
            Some(&mf),
            "moss",
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "model_missing", "模型缺失必须压过 ok");
        assert!(st.message.contains(MODEL_ID), "文案要指出清单条目：{}", st.message);
        assert!(st.message.contains("重新安装"), "{}", st.message);
    }

    #[test]
    fn evaluate_model_present_keeps_ok() {
        let (dir, mf) = setup_healthy("modok");
        let model_dir = dir.join("models").join("moss");
        std::fs::create_dir_all(&model_dir).unwrap();
        let model = model_dir.join("moss-transcribe-q5_k.gguf");
        std::fs::write(&model, b"gguf").unwrap();
        let st = evaluate(
            &dir,
            &dir.join("moss-transcribe.exe"),
            Some(&model),
            Some(&mf),
            "moss",
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "ok");
    }

    #[test]
    fn evaluate_model_missing_without_record_still_reports() {
        // 手工解包（无 .installed.json）但模型没跟过来：也要说"模型缺失"，别只说 unknown
        let dir = setup("modnorec", None, Some("cpu"), true);
        let model = dir.join("models").join("moss").join("x.gguf");
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), Some(&model), None, "moss");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(st.state, "model_missing");
    }

    #[test]
    fn evaluate_no_model_check_when_none() {
        // model=None 表示调用方不关心模型（例如只查二进制）——不能凭空报缺失
        let (dir, mf) = setup_healthy("modnone");
        let st = evaluate(&dir, &dir.join("moss-transcribe.exe"), None, Some(&mf), "moss");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&mf);
        assert_eq!(st.state, "ok");
    }

    #[test]
    fn moss_paths_fall_back_to_standard_layout() {
        // 卡片安装只解包、不写 config.json：空配置必须解析到标准布局，
        // 否则"装完"会被判成 file_missing
        let cfg = super::super::RuntimeConfig::default();
        let (bin, model) = super::super::moss_paths(&cfg, Path::new("R:/rt"));
        assert_eq!(
            bin,
            Path::new("R:/rt").join(super::super::MOSS_BINARY_REL)
        );
        assert_eq!(
            model,
            Path::new("R:/rt").join(super::super::MOSS_MODEL_REL)
        );
    }

    /// 随包 manifest 与"清单里 cpu 条目"的一致性是安装器契约的一部分；
    /// 这里只校验解析器能吃下真实产物形态（flavor 大写也认）。
    #[test]
    fn parse_manifest_items_is_case_insensitive_for_flavor() {
        let v = parse_manifest_items(
            r#"{"items":[{"id":"moss","flavor":"Vulkan","sha256":"ABC","size":7}]}"#,
        );
        assert_eq!(v[0].flavor, "vulkan");
        assert_eq!(v[0].sha256, "abc");
        assert_eq!(v[0].size, 7);
    }

    /// 两个文件都是"工具写出来的"：PowerShell 5.1 的 `Set-Content -Encoding UTF8`
    /// 会加 BOM，而 serde_json 见到 BOM 直接判语法错误（会退化成"什么都没装"）
    #[test]
    fn parsers_tolerate_utf8_bom() {
        let installed = "\u{feff}{\"version\":1,\"items\":{\"moss\":{\"flavor\":\"cpu\",\"sha256\":\"ab\"}}}";
        let map = parse_installed(installed);
        assert_eq!(map.get("moss").map(|i| i.flavor.as_str()), Some("cpu"));

        let manifest = "\u{feff}{\"items\":[{\"id\":\"models-moss\",\"sha256\":\"cd\",\"size\":9}]}";
        let items = parse_manifest_items(manifest);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "models-moss");
        assert_eq!(items[0].flavor, "", "无 flavor 的条目应保留为空串");
    }
}
