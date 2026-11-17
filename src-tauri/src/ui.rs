//! 界面级设置（窗口 / WebView 相关，与业务无关）。

use tauri::Manager;

/// 设置 WebView 的缩放比例（限制在 0.5–2.0，防止被传成 0 或负数把界面缩没）。
///
/// 用 WebView2 自己的 zoom，而不是 CSS 的 `zoom` 属性：
/// CSS zoom 只缩放内容，`100vh` / `100dvh` 这类视口单位不跟着变 —— 缩到 90%
/// 时底部会空出一条，放大到 125% 时内容溢出、文字被挤着换行。
/// WebView 缩放改的是 CSS 像素与物理像素的换算比，视口单位随之正确。
#[tauri::command]
pub fn set_ui_zoom(app: tauri::AppHandle, scale: f64) -> Result<(), String> {
    let scale = scale.clamp(0.5, 2.0);
    let Some(win) = app.get_webview_window("main") else {
        return Err("找不到主窗口，界面缩放未生效".into());
    };
    win.set_zoom(scale)
        .map_err(|e| format!("设置界面缩放失败: {e}"))
}
