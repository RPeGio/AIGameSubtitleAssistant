// ─── AI Runtime 运行配置 ──────────────────────────────────
// runtime/ 目录存放运行环境相关的非代码资源：
//   runtime/config.json          → 本文件持久化
//   runtime/worker/ocr_worker.py → 2.2 的 Python OCR 桥接脚本
//   runtime/deps                 → pip install --target 安装的依赖（复用系统 Python）
//   runtime/models/paddleocr     → PP-OCR 模型文件
//
// runtime 目录解析顺序：
//   1. 环境变量 GSA_RUNTIME_DIR
//   2. 从可执行文件所在目录逐级向上，找含 runtime/config.json 的目录（开发期=仓库根）
//   3. 兜底：应用数据目录下的 runtime

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 运行时配置 —— 描述 Python/依赖/模型所在位置
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// 系统 Python 可执行文件路径（开发期直接用系统环境，不内嵌）
    pub python_path: String,
    /// OCR worker 脚本路径（runtime/worker/ocr_worker.py）
    pub worker_script: String,
    /// pip install --target 安装的依赖目录（runtime/deps）
    pub deps_dir: String,
    /// PP-OCR 模型目录（runtime/models/paddleocr）
    pub model_dir: String,
    /// 识别语言，默认 "ch"
    pub language: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            python_path: "python".into(),
            worker_script: String::new(),
            deps_dir: String::new(),
            model_dir: String::new(),
            language: "ch".into(),
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
pub fn resolve_runtime_dir(fallback: Option<PathBuf>) -> PathBuf {
    if let Ok(env_dir) = std::env::var("GSA_RUNTIME_DIR") {
        let p = PathBuf::from(env_dir);
        if p.exists() {
            return p;
        }
    }
    if let Some(d) = runtime_dir_from_exe() {
        return d;
    }
    fallback.unwrap_or_else(|| PathBuf::from("runtime"))
}

impl RuntimeConfig {
    /// 从 runtime 目录读取 config.json；文件缺失或损坏时返回默认配置
    pub fn load(runtime_dir: &Path) -> Self {
        let path = runtime_dir.join("config.json");
        if let Ok(json) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<RuntimeConfig>(&json) {
                return cfg;
            }
        }
        Self::default()
    }

    /// 写入 config.json 到 runtime 目录（目录不存在则创建）
    pub fn save(&self, runtime_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(runtime_dir).map_err(|e| format!("无法创建 runtime 目录: {}", e))?;
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("配置序列化失败: {}", e))?;
        fs::write(runtime_dir.join("config.json"), json)
            .map_err(|e| format!("无法写入配置: {}", e))
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
        };
        cfg.save(&dir).unwrap();

        let loaded = RuntimeConfig::load(&dir);
        assert_eq!(loaded.python_path, cfg.python_path);
        assert_eq!(loaded.worker_script, cfg.worker_script);
        assert_eq!(loaded.deps_dir, cfg.deps_dir);
        assert_eq!(loaded.model_dir, cfg.model_dir);
        assert_eq!(loaded.language, cfg.language);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_returns_default() {
        let dir = temp_dir("cfg_missing");
        let cfg = RuntimeConfig::load(&dir);
        assert_eq!(cfg.python_path, "python");
        assert_eq!(cfg.language, "ch");
        assert!(cfg.worker_script.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_corrupted_returns_default() {
        let dir = temp_dir("cfg_corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{ not valid json ").unwrap();
        let cfg = RuntimeConfig::load(&dir);
        assert_eq!(cfg.language, "ch");
        let _ = fs::remove_dir_all(&dir);
    }
}
