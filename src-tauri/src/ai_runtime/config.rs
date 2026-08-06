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
    /// 开发期调试：帧输出到仓库根 temp/ 并打印各环节日志（release 前关闭）
    #[serde(default = "default_dev_debug")]
    pub dev_debug: bool,
}

fn default_ocr_model() -> String {
    "mobile".into()
}

fn default_dev_debug() -> bool {
    true
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            python_path: "python".into(),
            worker_script: String::new(),
            deps_dir: String::new(),
            model_dir: String::new(),
            language: "ch".into(),
            ocr_model: default_ocr_model(),
            dev_debug: default_dev_debug(),
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

/// 解析 runtime 目录（环境变量 → 向上查找 → 兜底）
///
/// TODO(分发): 打包后需把 runtime/**/* 加入 tauri.conf.json 的 bundle.resources，
/// 并让此处识别 Windows 资源目录布局（installer 下 resources/runtime/config.json）。
pub fn resolve_runtime_dir(fallback: Option<PathBuf>) -> PathBuf {
    // 环境变量指定的目录必须真的含 config.json，否则视为误配置并告警
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
    fallback.unwrap_or_else(|| PathBuf::from("runtime"))
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

    /// 校验各路径的合法性：
    /// - worker_script / deps_dir / model_dir 必须解析在 runtime 目录内（防路径穿越）
    /// - python_path 解析后（相对则按 runtime 目录解析）必须存在
    pub fn validate(&self, runtime_dir: &Path) -> Result<(), String> {
        for (name, value) in [
            ("worker_script", &self.worker_script),
            ("deps_dir", &self.deps_dir),
            ("model_dir", &self.model_dir),
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

        let pp = PathBuf::from(&self.python_path);
        let joined = if pp.is_absolute() {
            pp
        } else {
            runtime_dir.join(&pp)
        };
        if !joined.is_file() {
            return Err(format!(
                "python_path 指向的文件不存在: {}（按 {} 解析）",
                self.python_path,
                joined.display()
            ));
        }
        Ok(())
    }
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
            dev_debug: true,
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
        let cfg = RuntimeConfig {
            python_path: "python.exe".into(),
            worker_script: "worker/ocr_worker.py".into(),
            deps_dir: "deps".into(),
            model_dir: "models/paddleocr".into(),
            language: "ch".into(),
            ocr_model: "mobile".into(),
            dev_debug: true,
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
}
