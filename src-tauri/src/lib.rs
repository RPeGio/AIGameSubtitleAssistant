// ─── 模块声明 ─────────────────────────────────────────────
// 每个模块对应一个核心功能域，Phase 0 先建立骨架
pub mod project;       // 项目管理（创建/打开/保存）
pub mod video;         // 视频处理（Phase 1 实现）
pub mod timeline;      // 时间轴（Phase 1 实现）
pub mod subtitle;      // 字幕（Phase 2 实现）
pub mod ai_runtime;    // AI 运行时（Phase 2 起逐步实现）
pub mod ocr;           // OCR 字幕生成流水线（Phase 2）
pub mod export;        // 导出（Phase 6 实现）

use ai_runtime::{config::resolve_runtime_dir, OcrManager, RuntimeConfig};
use tauri::Manager; // app.path() 等

/// Tauri 应用入口
///
/// 在这里注册所有插件和 Tauri 命令（IPC handlers）。
/// 每个 `#[tauri::command]` 函数都需要在 `invoke_handler` 中注册，
/// 前端才能通过 `invoke("command_name", { args })` 调用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // ─── 插件注册 ────────────────────────────────────
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())

        // ─── 全局状态初始化 ───────────────────────────────
        // 解析 runtime 目录（环境变量 → 向上查找 → app_data 兜底），
        // 加载运行配置（失败时告警并回退默认）并托管 OcrManager。
        .setup(|app| {
            let fallback = app
                .path()
                .app_data_dir()
                .ok()
                .map(|d| d.join("runtime"));
            let runtime_dir = resolve_runtime_dir(fallback);
            let config = match RuntimeConfig::load(&runtime_dir) {
                Ok(cfg) => {
                    if let Err(msg) = cfg.validate(&runtime_dir) {
                        eprintln!("[ai_runtime] 配置校验失败: {} @ {}", msg, runtime_dir.display());
                    }
                    cfg
                }
                Err(msg) => {
                    eprintln!("[ai_runtime] 读取配置失败（使用默认）: {}", msg);
                    RuntimeConfig::default()
                }
            };
            app.manage(OcrManager::new(config, runtime_dir));
            Ok(())
        })

        // ─── 命令注册 ────────────────────────────────────
        // 每个命令函数在此列出，前端才能调用
        .invoke_handler(tauri::generate_handler![
            // project 模块
            project::create_project,
            project::open_project,
            project::save_project,
            project::set_project_video,
            project::list_recent_projects,
            // video 模块
            video::get_video_metadata,
            // ai_runtime 模块
            ai_runtime::check_ocr_runtime,
        ])
        .run(tauri::generate_context!())
        .expect("启动应用失败");
}
