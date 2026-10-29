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

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// PATH 里有没有这个命令（Windows 的 CreateProcess 会自动搜 PATH）
fn runs_on_path(stem: &str) -> bool {
    Command::new(stem)
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
}
