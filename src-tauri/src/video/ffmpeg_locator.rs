// ─── ffmpeg / ffprobe 定位 ─────────────────────────────────
//
// 查找顺序：**内嵌优先 → 系统 PATH**。
// 内嵌优先的理由：ffmpeg 是硬依赖（元数据 / 抽帧 / 抽音 / 波形全走它），
// 版本必须由发布包锁定；用户机器上有没有装 ffmpeg 不该影响可用性。
// PATH 只作兜底：dev 环境没跑 fetch 脚本、或用户自己想用更新的版本。
//
// 打包形态下 `bundle.resources` 是数组形式 ⇒ **保留目录结构**，
// 内嵌二进制落在 `$RESOURCES/resources/ffmpeg/`，不是 `$RESOURCES/ffmpeg/`。
// dev 形态下 exe 在 `src-tauri/target/<profile>/`，向上两级即 `src-tauri`，
// 对应仓库里的 `src-tauri/resources/ffmpeg`。

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

/// 内嵌二进制在 resources 下的子目录名
const BUNDLED_SUBDIR: &str = "ffmpeg";

/// 资源根目录（由 `init()` 在 app setup 时注入）
static RESOURCE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 命中缓存的 ffmpeg（只缓存成功结果；没找到时下次调用会重新探测）
static FFMPEG: OnceLock<PathBuf> = OnceLock::new();
/// 命中缓存的 ffprobe
static FFPROBE: OnceLock<PathBuf> = OnceLock::new();

/// 初始化资源目录。
///
/// 只允许设置一次（Tauri setup 里调用）；重复调用返回 `false` 并被忽略。
pub fn init(resource_dir: Option<PathBuf>) -> bool {
    RESOURCE_DIR.set(resource_dir).is_ok()
}

fn resource_dir() -> Option<&'static Path> {
    RESOURCE_DIR.get().and_then(|d| d.as_deref())
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// 可执行文件名（Windows 带 .exe）
fn file_name(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", stem)
    } else {
        stem.to_string()
    }
}

/// 计算内嵌二进制的候选路径（纯函数，便于单测）。
///
/// 从 exe 所在目录向上逐级探测 `resources/ffmpeg/<name>`（最多 4 级）：
///   - `<exe>/resources/ffmpeg/…`        —— 打包布局（数组形式保留目录结构）
///   - `<exe>/../../resources/ffmpeg/…`  —— dev 布局（target/debug 向上两级 = src-tauri）
///   - `<exe>/../../../resources/…`      —— cargo test 的 exe 在 target/debug/deps
/// 再用 Tauri 给的 resource_dir 兜两种形态（部分平台它直接是安装根 / resources 被 map 到别处）。
pub fn candidates_for(exe_dir: Option<&Path>, resource_dir: Option<&Path>, name: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(dir) = exe_dir {
        for ancestor in dir.ancestors().take(4) {
            out.push(ancestor.join("resources").join(BUNDLED_SUBDIR).join(name));
        }
    }
    if let Some(dir) = resource_dir {
        out.push(dir.join("resources").join(BUNDLED_SUBDIR).join(name));
        out.push(dir.join(BUNDLED_SUBDIR).join(name));
    }
    out
}

/// 该路径能不能真的跑起来（`-version` 退出码为 0）。
///
/// 不能只判 `exists()`：解压中断、被杀掉的半个文件、架构不符的二进制都会「存在」但跑不动。
fn runs(path: &Path) -> bool {
    crate::subprocess::hidden(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// PATH 里有没有这个命令（Windows 的 CreateProcess 会自动搜 PATH）
fn runs_on_path(stem: &str) -> bool {
    crate::subprocess::hidden(stem)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 按「内嵌 → PATH」的顺序定位一个二进制
fn locate(stem: &str) -> Option<PathBuf> {
    let name = file_name(stem);
    let exe = exe_dir();
    for candidate in candidates_for(exe.as_deref(), resource_dir(), &name) {
        if candidate.is_file() && runs(&candidate) {
            return Some(candidate);
        }
    }
    if runs_on_path(stem) {
        return Some(PathBuf::from(stem));
    }
    None
}

/// ffmpeg 路径（内嵌优先）
pub fn ffmpeg() -> Option<PathBuf> {
    if let Some(p) = FFMPEG.get() {
        return Some(p.clone());
    }
    let found = locate("ffmpeg")?;
    let _ = FFMPEG.set(found.clone());
    Some(found)
}

/// ffprobe 路径（内嵌优先）
pub fn ffprobe() -> Option<PathBuf> {
    if let Some(p) = FFPROBE.get() {
        return Some(p.clone());
    }
    let found = locate("ffprobe")?;
    let _ = FFPROBE.set(found.clone());
    Some(found)
}

// ─── 自检命令（欢迎页 FFmpeg 卡）────────────────────────────

/// 自检结论：前端只读这份结构，不自己拼路径也不自己判版本
#[derive(Debug, Clone, Serialize)]
pub struct FfmpegStatus {
    /// ffmpeg 与 ffprobe 都可用
    pub ready: bool,
    /// "bundled"（内嵌目录）/ "path"（系统 PATH）/ "missing"
    pub source: String,
    /// 解析到的可执行文件：内嵌时是绝对路径，走 PATH 时就是命令名
    pub ffmpeg: String,
    pub ffprobe: String,
    /// 形如 "n8.1.3-25-g611cec792a"；取不到为空串
    pub version: String,
    /// 给用户看的一句话（未就绪时说明该怎么办）
    pub message: String,
}

/// 取版本号：`ffmpeg -version` 首行形如
/// `ffmpeg version n8.1.3-25-g611cec792a-20261028 Copyright (c) 2000-2026 the FFmpeg developers`
fn version_of(path: &Path) -> String {
    let out = match crate::subprocess::hidden(path)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()
        .and_then(|line| line.split("version ").nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("")
        .to_string()
}

/// 欢迎页 FFmpeg 自检卡的数据源。
///
/// 只读探测，不改配置、不下载任何东西：内嵌目录优先，其次系统 PATH。
/// **必须 async**：里面要跑两次 `-version`（冷启动时杀毒软件扫描可上百毫秒），
/// 同步命令跑在主线程上会把欢迎页卡住（真机反馈 2026-10-29）。
#[tauri::command]
pub async fn check_ffmpeg() -> FfmpegStatus {
    tauri::async_runtime::spawn_blocking(ffmpeg_status)
        .await
        .unwrap_or_else(|e| FfmpegStatus {
            ready: false,
            source: "missing".into(),
            ffmpeg: String::new(),
            ffprobe: String::new(),
            version: String::new(),
            message: format!("ffmpeg 自检失败: {e}"),
        })
}

/// 自检的实际实现（阻塞版，供命令与单测复用）
pub fn ffmpeg_status() -> FfmpegStatus {
    let ff = ffmpeg();
    let fp = ffprobe();
    let mut status = FfmpegStatus {
        ready: false,
        source: "missing".into(),
        ffmpeg: String::new(),
        ffprobe: String::new(),
        version: String::new(),
        message: String::new(),
    };

    let (Some(ff), Some(fp)) = (ff, fp) else {
        status.message =
            "未找到 ffmpeg/ffprobe：安装包应自带 resources/ffmpeg/；源码检出请先跑 scripts/fetch_ffmpeg.ps1"
                .into();
        return status;
    };

    // locate() 对内嵌命中返回绝对路径，走 PATH 兜底时返回的是裸命令名
    let bundled = ff.is_absolute();
    status.ready = true;
    status.source = if bundled { "bundled" } else { "path" }.into();
    status.ffmpeg = ff.display().to_string();
    status.ffprobe = fp.display().to_string();
    status.version = version_of(&ff);
    status.message = if bundled {
        "使用随包内嵌的 ffmpeg".to_string()
    } else {
        "随包内嵌的 ffmpeg 缺失，已回退到系统 PATH 上的 ffmpeg".to_string()
    };
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_bundle_layout_first() {
        // 打包形态：exe 与 resources 同级，数组形式保留目录结构
        let exe_dir = Path::new("C:\\app");
        let list = candidates_for(Some(exe_dir), Some(Path::new("C:\\app")), "ffmpeg.exe");
        assert_eq!(list[0], Path::new("C:\\app\\resources\\ffmpeg\\ffmpeg.exe"));
    }

    #[test]
    fn candidates_dev_layout_included() {
        // dev: exe 在 src-tauri/target/debug，向上两级即 src-tauri
        let exe_dir = Path::new("F:\\repo\\src-tauri\\target\\debug");
        let list = candidates_for(Some(exe_dir), None, "ffprobe.exe");
        assert_eq!(
            list[0],
            Path::new("F:\\repo\\src-tauri\\target\\debug\\resources\\ffmpeg\\ffprobe.exe")
        );
        assert_eq!(list[2], Path::new("F:\\repo\\src-tauri\\resources\\ffmpeg\\ffprobe.exe"));
    }

    #[test]
    fn candidates_cover_cargo_test_layout() {
        // cargo test 的 exe 在 target/debug/deps，要再向上多走一级才到 src-tauri
        let exe_dir = Path::new("F:\\repo\\src-tauri\\target\\debug\\deps");
        let list = candidates_for(Some(exe_dir), None, "ffmpeg.exe");
        assert!(list.contains(&Path::new("F:\\repo\\src-tauri\\resources\\ffmpeg\\ffmpeg.exe").to_path_buf()));
    }

    #[test]
    fn candidates_fall_back_to_resource_dir() {
        let list = candidates_for(None, Some(Path::new("D:\\res")), "ffmpeg.exe");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Path::new("D:\\res\\resources\\ffmpeg\\ffmpeg.exe"));
        assert_eq!(list[1], Path::new("D:\\res\\ffmpeg\\ffmpeg.exe"));
    }

    #[test]
    fn broken_binary_is_not_usable() {
        // 文件存在但跑不起来（冒充二进制的文本）必须判为不可用
        let bogus = std::env::temp_dir().join(format!("gsa_fake_{}.exe", std::process::id()));
        std::fs::write(&bogus, b"not a real exe").unwrap();
        assert!(!runs(&bogus));
        let _ = std::fs::remove_file(&bogus);
    }

    /// 仓库里已跑过 `scripts/fetch_ffmpeg.ps1` 时，locate 必须命中内嵌目录而不是 PATH。
    /// 没跑过（二进制不入库）则跳过——这条测试的职责是「内嵌优先真的生效」。
    #[test]
    fn bundled_binary_takes_priority() {
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(BUNDLED_SUBDIR)
            .join(file_name("ffmpeg"));
        if !bundled.is_file() {
            return;
        }
        let found = ffmpeg().expect("内嵌 ffmpeg 存在时必须能定位到");
        assert!(found.is_absolute(), "内嵌命中应返回绝对路径，实际 {found:?}");
        assert!(found.ends_with(&file_name("ffmpeg")));
    }

    #[test]
    fn version_parses_from_bundled_binary() {
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(BUNDLED_SUBDIR)
            .join(file_name("ffmpeg"));
        if !bundled.is_file() {
            return;
        }
        let v = version_of(&bundled);
        assert!(!v.is_empty(), "版本号解析为空");
        assert!(v.contains("n8.1") || v.contains("8.1"), "意外的版本号：{v}");
    }
}
