use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri::Manager; // 提供 app.path() 等方法

// ─── 数据模型 ─────────────────────────────────────────────

/// 字幕事件 —— 核心数据单元
/// 一条字幕事件代表一句台词，包含起止时间、文本内容、说话人等
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker: Option<String>,
    pub character: Option<String>,
    /// 来源: "ocr" | "asr" | "manual"
    pub source: String,
    /// 置信度 (0.0 ~ 1.0)
    pub confidence: f64,
}

/// 轨道 —— 一组有序的字幕事件
/// 一个项目可以有多个轨道（角色字幕轨、主播语音轨、翻译轨等）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// 轨道类型: "subtitle" | "voice" | "translation"
    #[serde(rename = "type")]
    pub track_type: String,
    pub events: Vec<SubtitleEvent>,
}

/// 项目 —— 顶层容器，保存整个字幕项目的元数据和所有轨道
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// 项目文件夹的绝对路径
    pub path: String,
    /// 视频文件路径（相对于项目文件夹或绝对路径）
    pub video: String,
    /// 项目名称
    pub name: String,
    /// 所有轨道
    pub tracks: Vec<Track>,
    pub created_at: String,
    pub updated_at: String,
}

/// 最近项目列表中的条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub updated_at: String,
}

// ─── 常量和文件名 ─────────────────────────────────────────

/// 项目元数据文件名
const PROJECT_FILE: &str = "project.json";
/// 最近项目列表文件名（存储在 app data 目录下）
const RECENT_PROJECTS_FILE: &str = "recent_projects.json";

// ─── 辅助函数 ─────────────────────────────────────────────

/// 生成当前时间的 ISO 8601 字符串
fn now_iso() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // 使用 chrono 会更规范，但为了减少依赖，简单用 Unix timestamp
    format!("{}", secs)
}

/// 在当前时间戳基础上生成一个简短的唯一 ID
fn generate_id() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // 取毫秒级时间戳作为 ID 的一部分
    format!("evt_{}", duration.as_millis())
}

/// 获取 app data 目录下的文件路径（用于存储最近项目列表等全局数据）
fn get_app_data_file(app: &AppHandle, filename: &str) -> PathBuf {
    // Tauri 2 中通过 app.path() 获取 AppPathResolver
    // app_data_dir() 返回应用数据目录（跨平台，如 Windows 上的 %APPDATA%/<bundle-identifier>/）
    let data_dir = app
        .path()
        .app_data_dir()
        .expect("无法获取 app data 目录");
    // 确保目录存在
    fs::create_dir_all(&data_dir).expect("无法创建 app data 目录");
    data_dir.join(filename)
}

// ─── 命令实现 ─────────────────────────────────────────────

/// 创建一个新的字幕项目
///
/// 在指定的 `path` 目录下创建 `project.json` 文件，
/// 并将该项目添加到最近项目列表中。
///
/// - `name`: 项目名称
/// - `path`: 项目文件夹的绝对路径
#[tauri::command]
pub fn create_project(app: AppHandle, name: String, path: String) -> Result<Project, String> {
    let project_path = PathBuf::from(&path);

    // 检查目标目录是否已存在 project.json，避免误覆盖已有项目
    let project_file = project_path.join(PROJECT_FILE);
    if project_file.exists() {
        return Err(format!(
            "目标目录已包含项目文件: {}",
            project_file.display()
        ));
    }

    // 确保目录存在（用户可能提前创建了目录，也可能没有）
    fs::create_dir_all(&project_path)
        .map_err(|e| format!("无法创建项目目录: {}", e))?;

    let now = now_iso();
    let project = Project {
        path: project_path.to_string_lossy().to_string(),
        video: String::new(),
        name,
        tracks: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    };

    // 序列化为 JSON 并写入文件
    let json = serde_json::to_string_pretty(&project)
        .map_err(|e| format!("项目序列化失败: {}", e))?;
    fs::write(&project_file, &json)
        .map_err(|e| format!("无法写入项目文件: {}", e))?;

    // 更新最近项目列表
    append_recent_project(&app, &project)?;

    Ok(project)
}

/// 打开一个已有的项目
///
/// 从指定目录读取 `project.json` 并反序列化。
///
/// - `path`: 项目文件夹的绝对路径
#[tauri::command]
pub fn open_project(app: AppHandle, path: String) -> Result<Project, String> {
    let project_file = PathBuf::from(&path).join(PROJECT_FILE);

    let json = fs::read_to_string(&project_file)
        .map_err(|e| format!("无法读取项目文件: {}", e))?;
    let mut project: Project = serde_json::from_str(&json)
        .map_err(|e| format!("项目文件解析失败: {}", e))?;

    // 确保 path 字段是完整的绝对路径
    project.path = PathBuf::from(&path).to_string_lossy().to_string();

    // 更新最近项目列表
    append_recent_project(&app, &project)?;

    Ok(project)
}

/// 保存项目（写入 project.json）
///
/// - `project`: 要保存的项目对象
#[tauri::command]
pub fn save_project(project: Project) -> Result<(), String> {
    let project_path = PathBuf::from(&project.path);
    let project_file = project_path.join(PROJECT_FILE);

    let mut updated = project;
    updated.updated_at = now_iso();

    let json = serde_json::to_string_pretty(&updated)
        .map_err(|e| format!("项目序列化失败: {}", e))?;
    fs::write(&project_file, &json)
        .map_err(|e| format!("无法写入项目文件: {}", e))?;

    Ok(())
}

/// 获取最近项目列表
#[tauri::command]
pub fn list_recent_projects(app: AppHandle) -> Vec<RecentProject> {
    let file = get_app_data_file(&app, RECENT_PROJECTS_FILE);

    // 如果文件不存在，返回空列表
    if !file.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&file) {
        Ok(json) => {
            serde_json::from_str(&json).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    }
}

// ─── 内部辅助 ─────────────────────────────────────────────

/// 将项目添加到最近项目列表的最前面
///
/// 如果列表已存在同名路径，则移到最前面。
/// 列表最多保留 20 条。
fn append_recent_project(app: &AppHandle, project: &Project) -> Result<(), String> {
    let mut recent = list_recent_projects(app.clone());

    // 移除已存在的同名条目
    recent.retain(|rp| rp.path != project.path);

    // 将当前项目插入到最前面
    recent.insert(
        0,
        RecentProject {
            path: project.path.clone(),
            name: project.name.clone(),
            updated_at: project.updated_at.clone(),
        },
    );

    // 限制列表长度
    if recent.len() > 20 {
        recent.truncate(20);
    }

    let file = get_app_data_file(app, RECENT_PROJECTS_FILE);
    let json = serde_json::to_string_pretty(&recent)
        .map_err(|e| format!("序列化最近项目列表失败: {}", e))?;
    fs::write(&file, &json)
        .map_err(|e| format!("写入最近项目列表失败: {}", e))?;

    Ok(())
}
