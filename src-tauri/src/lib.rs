// ─── 模块声明 ─────────────────────────────────────────────
// 每个模块对应一个核心功能域，Phase 0 先建立骨架
pub mod project;       // 项目管理（创建/打开/保存）
pub mod video;         // 视频处理（Phase 1 实现）
pub mod timeline;      // 时间轴（Phase 1 实现）
pub mod subtitle;      // 字幕（Phase 2 实现）
pub mod ai_runtime;    // AI 运行时（Phase 2 起逐步实现）
pub mod export;        // 导出（Phase 6 实现）

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

        // ─── 命令注册 ────────────────────────────────────
        // 每个命令函数在此列出，前端才能调用
        .invoke_handler(tauri::generate_handler![
            // project 模块
            project::create_project,
            project::open_project,
            project::save_project,
            project::list_recent_projects,
        ])
        .run(tauri::generate_context!())
        .expect("启动应用失败");
}
