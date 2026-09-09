use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tauri::Manager; // 提供 app.path() 等方法
#[cfg(test)]
use uuid::Uuid;

// ─── 数据模型 ─────────────────────────────────────────────

// ─── 时间轴事件（tagged union）────────────────────────────
// TimelineEvent 是所有轨道事件的基础抽象。
// 通过 serde(tag = "type") 序列化为带类型标签的 JSON，
// 反序列化时根据 "type" 字段自动分发到对应的变体。
// JSON 示例: { "type": "ocr_region", "id": "...", "start": 0, "end": 90, "x1": 0.1, ... }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TimelineEvent {
    /// OCR 识别到的游戏内对话文本
    #[serde(rename = "ocr_text")]
    OcrText(OcrTextEvent),
    /// 切片视频内嵌字幕 OCR —— 主播画面中游戏字幕的识别文本（嵌字轴，供融合）。
    /// 结构同 OcrText，字段复用 OcrTextEvent
    #[serde(rename = "embed_ocr")]
    EmbedOcr(OcrTextEvent),
    /// OCR 区域选框 —— 标记视频中字幕出现的矩形区域
    #[serde(rename = "ocr_region")]
    OcrRegion(OcrRegionEvent),
    /// ASR 语音识别结果，含说话人分离信息
    #[serde(rename = "asr")]
    Asr(AsrEvent),
    /// AI 融合结果 —— OCR 文本与 ASR 融合后的最终字幕（Phase 4）
    #[serde(rename = "fused")]
    Fused(FusedEvent),
    /// 人工手动创建/编辑的字幕事件
    #[serde(rename = "manual")]
    Manual(ManualEvent),
}

/// OCR 文本事件 —— 从视频截图中识别出的一句台词
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrTextEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    /// 识别出的原始文本
    pub text: String,
    /// OCR 置信度 (0.0 ~ 1.0)
    pub confidence: f64,
}

/// OCR 区域事件 —— 标记视频中某段时间内字幕出现的矩形位置
/// 坐标使用归一化值 (0.0 ~ 1.0)，相对于视频画面的宽高
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrRegionEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    /// 矩形左上角 X（归一化）
    pub x1: f64,
    /// 矩形左上角 Y（归一化）
    pub y1: f64,
    /// 矩形右下角 X（归一化）
    pub x2: f64,
    /// 矩形右下角 Y（归一化）
    pub y2: f64,
}

/// ASR 语音识别事件 —— 由说话人分离模型输出的带时间轴的台词
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    /// 识别的文本内容
    pub text: String,
    /// 说话人标签（如 "S01", "S02"）
    pub speaker: String,
    /// 解析后的角色名（如 "派蒙"），可能为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    /// ASR 置信度 (0.0 ~ 1.0)
    pub confidence: f64,
}

/// AI 融合事件 —— LLM 将 OCR 文本与 ASR 段融合后的最终字幕（Phase 4）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusedEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    /// 融合后的最终文本（优先 OCR 文本，无匹配时保留 ASR 原文本）
    pub text: String,
    /// 角色名（LLM 从 OCR 文本提取），可能为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
}

/// 手动创建的字幕事件 —— 用户手工添加或编辑的台词
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualEvent {
    pub id: String,
    pub start: f64,
    pub end: f64,
    /// 台词文本
    pub text: String,
    /// 角色名（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
}

/// 轨道 —— 一组有序的时间轴事件
/// 一个项目可以有多个轨道（角色字幕轨、主播语音轨、OCR 区域轨等）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// 轨道唯一标识
    pub id: String,
    /// 用户自定义的轨道显示名称（如 "角色字幕", "主播语音"）
    pub name: String,
    /// 轨道类型: "ocr_region" | "ocr_text" | "asr" | "manual" | "translation"
    #[serde(rename = "type")]
    pub track_type: String,
    /// 轨道内容属性（仅 asr 轨道使用）：
    /// "streamer" 主播语音 | "game" 游戏内容。
    /// 缺省视为 "game"：游戏内容轨是常态，主播语音轨由用户显式标记
    #[serde(default = "default_track_role")]
    pub track_role: String,
    /// 轨道角色："control" 控制轨（页面工作状态，如 OCR 选区）| "output" 产物轨（字幕数据）。
    /// 缺省视为 "output"：既有轨道都是产物轨，控制轨由页面显式创建
    #[serde(default = "default_track_scope")]
    pub scope: String,
    /// 控制轨归属页面（仅 scope=control 使用）：
    /// "corpus" | "asr" | "fuse" | "editor"。缺省空串
    #[serde(default)]
    pub page: String,
    /// 轨道绑定的视频源："source" 剧情录屏（文本源）| "clip" 切片（时间轴基准）。
    /// 缺省视为 "clip"：时间轴产物默认挂在切片视频上
    #[serde(default = "default_track_video")]
    pub video: String,
    /// 是否在预览窗口中显示该轨道字幕（纯显示偏好，随项目保存）
    #[serde(default = "default_true")]
    pub preview_visible: bool,
    /// 轨道内的事件列表，按时间排序
    #[serde(default)]
    pub events: Vec<TimelineEvent>,
}

fn default_track_role() -> String {
    "game".into()
}

fn default_track_scope() -> String {
    "output".into()
}

fn default_track_video() -> String {
    "clip".into()
}

fn default_true() -> bool {
    true
}

/// 项目 —— 顶层容器，保存整个字幕项目的元数据和所有轨道
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// .gsa 项目文件的绝对路径 —— 项目身份（同一目录可有多个项目文件）。
    /// open/set_project_video 时以实际操作的文件路径覆写，文件内保存的旧值不具权威性
    pub path: String,
    /// 切片视频文件路径（相对于项目文件所在文件夹或绝对路径）——时间轴基准
    pub video: String,
    /// 剧情录屏视频路径（文本源，OCR 语料用）；缺省空串
    #[serde(default)]
    pub source_video: String,
    /// 项目名称
    pub name: String,
    /// 可靠文本语料集合（独立于轨道，供 LLM 融合消费）；缺省空
    #[serde(default)]
    pub corpus: Vec<CorpusItem>,
    /// 所有轨道
    pub tracks: Vec<Track>,
    pub created_at: String,
    pub updated_at: String,
}

/// 语料条目 —— 一条可靠的游戏内文本（来源可多样，无时间轴语义）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusItem {
    pub id: String,
    /// 文本内容
    pub text: String,
    /// 来源："paste" 手动粘贴 | "image_ocr" 截图 OCR | "ocr_track" 从 OCR 轨提取
    pub source: String,
    pub created_at: String,
}

/// 最近项目列表中的条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub updated_at: String,
}

// ─── 常量和文件名 ─────────────────────────────────────────

/// 项目文件魔数头：文件首行为 `GSA-PROJECT v1`，其后是 JSON 主体。
/// 自定义扩展名 + 魔数头让项目文件不会被误认成普通 JSON 配置文件
const PROJECT_MAGIC: &str = "GSA-PROJECT";
/// 项目文件格式版本（解析时拒绝更高版本，提示升级应用）
const PROJECT_FORMAT_VERSION: u32 = 1;
/// 项目文件扩展名（不带点）
const PROJECT_EXT: &str = "gsa";
/// 项目文件名主体长度上限（字符数），防止超长文件名触发 Windows 路径问题
const MAX_NAME_CHARS: usize = 100;
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
#[allow(unused)]
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

// ─── 项目文件格式（.gsa：魔数头 + JSON 主体）──────────────
// 外部形态：<项目名>.gsa，内部 JSON 结构与字段完全不变。
// 首行携带魔数与格式版本，便于识别文件类型并支持未来格式演进。

/// 将项目名清理为合法的 Windows 文件名主体（不含扩展名）：
/// 非法字符 `\ / : * ? " < > |` 与控制符替换为 '_'；去首尾空白与结尾的点；
/// Windows 保留设备名（CON/NUL/COM1…）追加 '_'；按字符数截断；清空后回退 "project"
fn sanitize_project_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut s = replaced.trim().trim_end_matches('.').to_string();

    // 保留设备名检查针对“基础名”（首个点之前），CON.txt 同样是保留名
    let upper = s.to_ascii_uppercase();
    let stem = upper.split('.').next().unwrap_or("");
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem) {
        s.push('_');
    }

    if s.chars().count() > MAX_NAME_CHARS {
        s = s.chars().take(MAX_NAME_CHARS).collect();
        s = s.trim_end().trim_end_matches('.').to_string();
    }
    if s.is_empty() {
        s = "project".to_string();
    }
    s
}

/// 项目文件完整路径：项目目录 + sanitize(项目名) + ".gsa"
fn project_file_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{}", sanitize_project_name(name), PROJECT_EXT))
}

/// 扩展名是否为 .gsa（不区分大小写）
fn has_project_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(PROJECT_EXT))
        .unwrap_or(false)
}

/// 在项目目录中查找唯一的项目文件（*.gsa，扩展名不区分大小写）。
/// 目录不可读、找不到或存在多个时均返回 Err。
/// 仅用于 open_project 的"目录兼容分支"（旧最近项目列表存的是目录路径）；
/// 项目身份本身 = .gsa 文件路径，同目录允许多个项目共存。
fn find_project_file(dir: &Path) -> Result<PathBuf, String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("无法读取项目目录 {}: {}", dir.display(), e))?;
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("无法读取项目目录: {}", e))?.path();
        if path.is_file() && has_project_ext(&path) {
            found.push(path);
        }
    }
    match found.len() {
        0 => Err(format!(
            "目录中未找到项目文件（*.{}）: {}",
            PROJECT_EXT,
            dir.display()
        )),
        1 => Ok(found.remove(0)),
        _ => Err(format!(
            "目录中存在多个项目文件，无法确定要打开的项目: {}",
            found
                .iter()
                .map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("、")
        )),
    }
}

/// 序列化为项目文件全文：`GSA-PROJECT v1` 头一行 + JSON 主体
fn serialize_project(project: &Project) -> Result<String, String> {
    let json = serde_json::to_string_pretty(project)
        .map_err(|e| format!("项目序列化失败: {}", e))?;
    Ok(format!(
        "{} v{}\n{}\n",
        PROJECT_MAGIC, PROJECT_FORMAT_VERSION, json
    ))
}

/// 解析项目文件全文（容忍 UTF-8 BOM 与 CRLF）；JSON 主体结构不变
fn parse_project(text: &str) -> Result<Project, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let (header, body) = text
        .split_once('\n')
        .ok_or_else(|| "不是有效的 GSA 项目文件（缺少文件头）".to_string())?;
    let version = header
        .trim_end_matches('\r')
        .trim()
        .strip_prefix(PROJECT_MAGIC)
        .and_then(|rest| rest.trim().strip_prefix('v'))
        .and_then(|v| v.trim().parse::<u32>().ok())
        .ok_or_else(|| "不是有效的 GSA 项目文件（文件头格式错误）".to_string())?;
    if version > PROJECT_FORMAT_VERSION {
        return Err(format!(
            "项目文件版本过新（v{}），请升级应用后再打开",
            version
        ));
    }
    serde_json::from_str(body).map_err(|e| format!("项目文件解析失败: {}", e))
}

/// 原子写项目文件：先写同目录临时文件，成功后 rename 覆盖目标。
/// 自动保存高频触发，直接覆盖写一旦被崩溃/断电打断会留下半截文件
fn write_project_file(path: &Path, project: &Project) -> Result<(), String> {
    let content = serialize_project(project)?;
    let tmp = path.with_extension(format!("{}.tmp", PROJECT_EXT));
    fs::write(&tmp, &content).map_err(|e| format!("无法写入项目文件: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("无法写入项目文件: {}", e)
    })?;
    Ok(())
}

// ─── 命令实现 ─────────────────────────────────────────────

/// 创建一个新的字幕项目
///
/// 在指定的 `path` 目录下创建以项目名命名的 `<项目名>.gsa` 项目文件，
/// 并将该项目添加到最近项目列表中。同一目录允许共存多个项目，
/// 仅当同名项目文件已存在时拒绝。
///
/// - `name`: 项目名称（同时决定项目文件名，非法字符会被替换为 '_'）
/// - `path`: 项目文件夹的绝对路径
#[tauri::command]
pub fn create_project(app: AppHandle, name: String, path: String) -> Result<Project, String> {
    let project = create_project_in(&PathBuf::from(&path), &name)?;
    append_recent_project(&app, &project)?;
    Ok(project)
}

/// create_project 的纯文件系统实现（无 AppHandle，便于测试）。
/// 返回的 `Project.path` = 新建 .gsa 文件的绝对路径（项目身份 = 文件）
fn create_project_in(dir: &Path, name: &str) -> Result<Project, String> {
    // 确保目录存在（用户可能提前创建了目录，也可能没有）
    fs::create_dir_all(dir).map_err(|e| format!("无法创建项目目录: {}", e))?;

    let project_file = project_file_path(dir, name);
    // 仅同名项目文件冲突即拒绝；目录内其它项目不受影响
    if project_file.exists() {
        return Err(format!("同名项目文件已存在: {}", project_file.display()));
    }

    let now = now_iso();
    let project = Project {
        path: project_file.to_string_lossy().to_string(),
        video: String::new(),
        source_video: String::new(),
        name: name.to_string(),
        corpus: Vec::new(),
        tracks: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    };

    write_project_file(&project_file, &project)?;
    Ok(project)
}

/// 打开一个已有的项目
///
/// `path` 支持两种形态：
/// - `.gsa` 项目文件的绝对路径 → 直接打开（同目录可有多个项目，项目身份 = 文件）
/// - 目录路径 → 目录内查找唯一的项目文件（兼容旧最近项目列表存的目录）
///
/// 成功后返回的 `Project.path` 统一为项目文件的绝对路径。
#[tauri::command]
pub fn open_project(app: AppHandle, path: String) -> Result<Project, String> {
    let project = open_project_at(&path)?;
    append_recent_project(&app, &project)?;
    Ok(project)
}

/// open_project 的纯文件系统实现（无 AppHandle，便于测试）
fn open_project_at(path: &str) -> Result<Project, String> {
    let raw = PathBuf::from(path);
    let project_file = if raw.is_dir() {
        find_project_file(&raw)?
    } else if raw.is_file() && has_project_ext(&raw) {
        raw
    } else {
        return Err(format!(
            "路径不是 .{} 项目文件，也不是包含唯一项目文件的目录: {}",
            PROJECT_EXT,
            raw.display()
        ));
    };

    let text = fs::read_to_string(&project_file)
        .map_err(|e| format!("无法读取项目文件: {}", e))?;
    let mut project = parse_project(&text)?;

    // 确保 path 字段是项目文件的绝对路径（项目身份 = 文件）
    project.path = project_file.to_string_lossy().to_string();

    Ok(project)
}

/// 保存项目，返回带新 updated_at 的 Project
///
/// 项目身份 = `project.path` 指向的 .gsa 文件：永远写回该文件。
/// 项目改名只改 JSON 内的 name 字段，文件名保持创建时的名字
/// （同一目录可共存多个项目，不做任何跨文件清理）。
///
/// - `project`: 要保存的项目对象
#[tauri::command]
pub fn save_project(project: Project) -> Result<Project, String> {
    let project_file = PathBuf::from(&project.path);
    if !has_project_ext(&project_file) {
        return Err(format!(
            "项目路径不是 .{} 项目文件: {}",
            PROJECT_EXT,
            project_file.display()
        ));
    }

    let mut updated = project;
    updated.updated_at = now_iso();

    write_project_file(&project_file, &updated)?;

    Ok(updated)
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

// ─── 视频路径更新 ─────────────────────────────────────────

/// 设置项目的视频文件路径并保存
///
/// `project_path` 为 .gsa 项目文件路径（项目身份 = 文件）
#[tauri::command]
pub fn set_project_video(project_path: String, video_path: String) -> Result<Project, String> {
    let project_file = PathBuf::from(&project_path);
    if !has_project_ext(&project_file) {
        return Err(format!(
            "项目路径不是 .{} 项目文件: {}",
            PROJECT_EXT,
            project_file.display()
        ));
    }

    let text = fs::read_to_string(&project_file)
        .map_err(|e| format!("无法读取项目文件: {}", e))?;
    let mut project = parse_project(&text)?;

    project.video = video_path;
    project.path = project_path;
    project.updated_at = now_iso();

    write_project_file(&project_file, &project)?;

    Ok(project)
}

// ─── 项目重命名 / 最近项目条目管理 ─────────────────────────

/// 重命名项目：重命名 .gsa 项目文件，并同步更新项目内 name 与最近项目列表。
/// 项目身份 = 文件，重命名即把项目身份迁移到新文件
///
/// - `project_path`: 现有 .gsa 项目文件路径
/// - `new_name`: 新项目名（同时决定新文件名，非法字符会被替换为 '_'）
#[tauri::command]
pub fn rename_project(
    app: AppHandle,
    project_path: String,
    new_name: String,
) -> Result<RecentProject, String> {
    let project = rename_project_in(Path::new(&project_path), &new_name)?;
    let entry = RecentProject {
        path: project.path.clone(),
        name: project.name.clone(),
        updated_at: project.updated_at.clone(),
    };
    let recent =
        replace_recent_entry(list_recent_projects(app.clone()), &project_path, entry.clone());
    save_recent_projects(&app, &recent)?;
    Ok(entry)
}

/// rename_project 的纯文件系统实现（无 AppHandle，便于测试）
fn rename_project_in(old_file: &Path, new_name: &str) -> Result<Project, String> {
    if !has_project_ext(old_file) {
        return Err(format!(
            "项目路径不是 .{} 项目文件: {}",
            PROJECT_EXT,
            old_file.display()
        ));
    }
    let text = fs::read_to_string(old_file).map_err(|e| format!("无法读取项目文件: {}", e))?;
    let mut project = parse_project(&text)?;

    let dir = old_file.parent().unwrap_or(Path::new("."));
    let new_file = project_file_path(dir, new_name);
    // sanitize 后仍指向同一文件（名字没变）：无操作
    if new_file == old_file {
        return Ok(project);
    }
    if new_file.exists() {
        return Err(format!("同名项目文件已存在: {}", new_file.display()));
    }

    project.name = sanitize_project_name(new_name);
    project.path = new_file.to_string_lossy().to_string();
    project.updated_at = now_iso();

    // 先写新文件、成功后再删旧文件：写失败时旧项目文件不受影响
    write_project_file(&new_file, &project)?;
    fs::remove_file(old_file).map_err(|e| format!("重命名项目失败: {}", e))?;

    Ok(project)
}

/// 从最近项目列表中移除一个条目（仅列表移除，不删除项目文件本身）
#[tauri::command]
pub fn remove_recent_project(app: AppHandle, project_path: String) -> Result<(), String> {
    let recent = remove_recent_entry(list_recent_projects(app.clone()), &project_path);
    save_recent_projects(&app, &recent)
}

// ─── 文本文件读取（语料导入用）────────────────────────────

/// 读取用户选择的 txt 文本文件（语料页"手动提供文本"导入）。
/// 仅接受 UTF-8：GBK 等编码直接报错让用户转存，不做编码探测。
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    const MAX_SIZE: u64 = 10 * 1024 * 1024;
    let meta = fs::metadata(&path).map_err(|e| format!("无法读取文件: {}", e))?;
    if meta.len() > MAX_SIZE {
        return Err("文件超过 10 MB，请确认选择的是文本文件".into());
    }
    let bytes = fs::read(&path).map_err(|e| format!("无法读取文件: {}", e))?;
    let mut text = String::from_utf8(bytes)
        .map_err(|_| "仅支持 UTF-8 编码的 txt 文件，请先转存为 UTF-8".to_string())?;
    // Windows Notepad 存的 txt 常带 UTF-8 BOM（\u{FEFF}）；在此显式剥离，
    // 避免残留到语料首行（不依赖前端 trim() 的隐式兜底）
    if text.starts_with('\u{FEFF}') {
        text.remove(0);
    }
    Ok(text)
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

    save_recent_projects(app, &recent)
}

/// 用新条目原位替换最近项目列表中 old_path 对应的条目；不在列表中则原样返回
fn replace_recent_entry(
    mut recent: Vec<RecentProject>,
    old_path: &str,
    entry: RecentProject,
) -> Vec<RecentProject> {
    if let Some(slot) = recent.iter_mut().find(|rp| rp.path == old_path) {
        *slot = entry;
    }
    recent
}

/// 移除最近项目列表中指定路径的条目；不在列表中则原样返回
fn remove_recent_entry(mut recent: Vec<RecentProject>, path: &str) -> Vec<RecentProject> {
    recent.retain(|rp| rp.path != path);
    recent
}

/// 将最近项目列表写回 app data 目录下的存储文件
fn save_recent_projects(app: &AppHandle, recent: &[RecentProject]) -> Result<(), String> {
    let file = get_app_data_file(app, RECENT_PROJECTS_FILE);
    let json = serde_json::to_string_pretty(recent)
        .map_err(|e| format!("序列化最近项目列表失败: {}", e))?;
    fs::write(&file, &json).map_err(|e| format!("写入最近项目列表失败: {}", e))?;
    Ok(())
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track() -> Track {
        Track {
            id: "t1".into(),
            name: "主播语音".into(),
            track_type: "asr".into(),
            track_role: "streamer".into(),
            scope: "output".into(),
            page: String::new(),
            video: "clip".into(),
            preview_visible: true,
            events: vec![],
        }
    }

    #[test]
    fn test_track_roundtrip_preserves_role() {
        let json = serde_json::to_string(&sample_track()).unwrap();
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back.track_role, "streamer");
    }

    #[test]
    fn test_track_legacy_json_defaults_to_game() {
        // 文件缺 track_role 字段（旧数据）→ 反序列化默认 "game"
        let json = r#"{"id":"t1","name":"游戏角色","type":"asr","events":[]}"#;
        let track: Track = serde_json::from_str(json).unwrap();
        assert_eq!(track.track_role, "game");
    }

    #[test]
    fn test_track_missing_events_defaults_empty() {
        // events 缺省（旧数据）也不应炸
        let json = r#"{"id":"t1","name":"轨","type":"manual"}"#;
        let track: Track = serde_json::from_str(json).unwrap();
        assert!(track.events.is_empty());
    }

    #[test]
    fn test_track_legacy_json_preview_visible_defaults_true() {
        // 文件缺 preview_visible（旧数据）→ 默认 true（预览不遗漏旧轨道）
        let json = r#"{"id":"t1","name":"游戏角色","type":"asr","events":[]}"#;
        let track: Track = serde_json::from_str(json).unwrap();
        assert!(track.preview_visible);
    }

    #[test]
    fn test_track_preview_visible_roundtrip() {
        let mut track = sample_track();
        track.preview_visible = false;
        let json = serde_json::to_string(&track).unwrap();
        let back: Track = serde_json::from_str(&json).unwrap();
        assert!(!back.preview_visible);
    }

    #[test]
    fn test_fused_event_roundtrip() {
        // fused 事件：tagged union 序列化/反序列化往返
        let ev = TimelineEvent::Fused(FusedEvent {
            id: "f1".into(),
            start: 1.5,
            end: 4.2,
            text: "旅行者，你来了".into(),
            character: Some("派蒙".into()),
        });
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"fused\""));
        let back: TimelineEvent = serde_json::from_str(&json).unwrap();
        match back {
            TimelineEvent::Fused(f) => {
                assert_eq!(f.id, "f1");
                assert_eq!(f.text, "旅行者，你来了");
                assert_eq!(f.character.as_deref(), Some("派蒙"));
            }
            _ => panic!("类型标签分发错误"),
        }
    }

    #[test]
    fn test_fused_event_character_omitted_when_none() {
        let ev = TimelineEvent::Fused(FusedEvent {
            id: "f2".into(),
            start: 0.0,
            end: 1.0,
            text: "未匹配文本".into(),
            character: None,
        });
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("character"));
    }

    #[test]
    fn test_embed_ocr_event_roundtrip() {
        // embed_ocr（切片内嵌字幕 OCR）：tagged union 往返 + 字段完整保留
        let ev = TimelineEvent::EmbedOcr(OcrTextEvent {
            id: "e1".into(),
            start: 1.5,
            end: 4.2,
            text: "旅行者，你来了".into(),
            confidence: 0.92,
        });
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"embed_ocr\""));
        let back: TimelineEvent = serde_json::from_str(&json).unwrap();
        match back {
            TimelineEvent::EmbedOcr(e) => {
                assert_eq!(e.id, "e1");
                assert_eq!(e.text, "旅行者，你来了");
                assert!((e.confidence - 0.92).abs() < 1e-9);
            }
            _ => panic!("类型标签分发错误"),
        }
    }

    #[test]
    fn test_track_legacy_json_defaults_scope_page_video() {
        // 文件缺 scope/page/video（旧数据）→ 默认 output / 空 / clip（既有轨道视为产物轨，挂切片）
        let json = r#"{"id":"t1","name":"游戏角色","type":"asr","events":[]}"#;
        let track: Track = serde_json::from_str(json).unwrap();
        assert_eq!(track.scope, "output");
        assert_eq!(track.page, "");
        assert_eq!(track.video, "clip");
    }

    #[test]
    fn test_track_scope_page_video_roundtrip() {
        let mut track = sample_track();
        track.scope = "control".into();
        track.page = "corpus".into();
        track.video = "source".into();
        let json = serde_json::to_string(&track).unwrap();
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back.scope, "control");
        assert_eq!(back.page, "corpus");
        assert_eq!(back.video, "source");
    }

    #[test]
    fn test_project_legacy_json_defaults_source_video_corpus() {
        // 文件缺 source_video/corpus（旧数据）→ 默认空串 / 空语料
        let json = r#"{"path":"C:/proj","video":"clip.mp4","name":"示例","tracks":[],"created_at":"1","updated_at":"2"}"#;
        let project: Project = serde_json::from_str(json).unwrap();
        assert_eq!(project.source_video, "");
        assert!(project.corpus.is_empty());
        assert_eq!(project.video, "clip.mp4");
    }

    #[test]
    fn test_project_source_video_corpus_roundtrip() {
        let mut project = Project {
            path: "C:/proj".into(),
            video: "clip.mp4".into(),
            source_video: "source.mp4".into(),
            name: "示例".into(),
            corpus: vec![CorpusItem {
                id: "c1".into(),
                text: "旅行者，你来了".into(),
                source: "paste".into(),
                created_at: "1".into(),
            }],
            tracks: vec![],
            created_at: "1".into(),
            updated_at: "2".into(),
        };
        project.corpus.push(CorpusItem {
            id: "c2".into(),
            text: "前方有敌人".into(),
            source: "ocr_track".into(),
            created_at: "3".into(),
        });
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_video, "source.mp4");
        assert_eq!(back.corpus.len(), 2);
        assert_eq!(back.corpus[0].text, "旅行者，你来了");
        assert_eq!(back.corpus[0].source, "paste");
        assert_eq!(back.corpus[1].source, "ocr_track");
    }

    // ── read_text_file ──

    /// 在系统临时目录造一个唯一文件，返回路径；测试结束自动清理由调用方
    /// 通过返回的路径做 remove_file（简单场景直接在测试内处理）
    fn temp_file(name: &str, contents: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{}_{}", Uuid::new_v4(), name));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn test_read_text_file_utf8_roundtrip() {
        let path = temp_file("corpus.txt", "旅行者，你来了\n派蒙：\n".as_bytes());
        let text = read_text_file(path.to_string_lossy().to_string()).unwrap();
        assert_eq!(text, "旅行者，你来了\n派蒙：\n");
        fs::remove_file(&path).ok();
    }

    #[test]
    fn test_read_text_file_rejects_non_utf8() {
        let path = temp_file("gbk.bin", &[0xC4, 0xE3, 0xBA, 0xC3, 0xFF]);
        let err = read_text_file(path.to_string_lossy().to_string()).unwrap_err();
        assert!(err.contains("UTF-8"), "报错应提示 UTF-8 编码问题，实际: {}", err);
        fs::remove_file(&path).ok();
    }

    // ── 项目文件格式（.gsa）──

    fn sample_project() -> Project {
        Project {
            path: "C:/proj".into(),
            video: "clip.mp4".into(),
            source_video: "source.mp4".into(),
            name: "示例项目".into(),
            corpus: vec![CorpusItem {
                id: "c1".into(),
                text: "旅行者，你来了".into(),
                source: "paste".into(),
                created_at: "1".into(),
            }],
            tracks: vec![sample_track()],
            created_at: "1".into(),
            updated_at: "2".into(),
        }
    }

    #[test]
    fn test_sanitize_replaces_illegal_chars() {
        assert_eq!(
            sanitize_project_name(r#"a/b\c:d*e?f"g<h>i|j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        // 控制字符同样替换
        assert_eq!(sanitize_project_name("a\u{0007}b"), "a_b");
    }

    #[test]
    fn test_sanitize_trims_dots_and_fallback() {
        assert_eq!(sanitize_project_name("  我的项目...  "), "我的项目");
        // 清理后为空（如只剩点/空白）才回退 "project"；"___" 是合法文件名不回退
        assert_eq!(sanitize_project_name("..."), "project");
        assert_eq!(sanitize_project_name("   "), "project");
        assert_eq!(sanitize_project_name(""), "project");
        assert_eq!(sanitize_project_name("///"), "___");
        // Windows 保留设备名（含带扩展名形态）追加 '_'
        assert_eq!(sanitize_project_name("CON"), "CON_");
        assert_eq!(sanitize_project_name("nul.txt"), "nul.txt_");
    }

    #[test]
    fn test_sanitize_truncates_long_name() {
        let long = "字".repeat(150);
        let sanitized = sanitize_project_name(&long);
        assert_eq!(sanitized.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn test_project_file_path_uses_sanitized_name() {
        let dir = PathBuf::from("C:/proj");
        assert_eq!(
            project_file_path(&dir, "我的项目:第一话"),
            dir.join("我的项目_第一话.gsa")
        );
    }

    #[test]
    fn test_project_file_roundtrip_with_header() {
        let project = sample_project();
        let text = serialize_project(&project).unwrap();
        assert!(text.starts_with("GSA-PROJECT v1\n"), "应含魔数头，实际: {}", &text[..text.len().min(40)]);
        let back = parse_project(&text).unwrap();
        assert_eq!(back.name, "示例项目");
        assert_eq!(back.video, "clip.mp4");
        assert_eq!(back.tracks.len(), 1);
        assert_eq!(back.corpus[0].text, "旅行者，你来了");
    }

    #[test]
    fn test_parse_rejects_plain_json_without_header() {
        // 纯 JSON（如旧 project.json 或普通配置文件）不再被接受
        let legacy = r#"{"path":"C:/proj","video":"clip.mp4","name":"示例","tracks":[],"created_at":"1","updated_at":"2"}"#;
        let err = parse_project(legacy).unwrap_err();
        assert!(err.contains("GSA 项目文件"), "实际: {}", err);
    }

    #[test]
    fn test_parse_rejects_newer_version() {
        let err = parse_project("GSA-PROJECT v2\n{}\n").unwrap_err();
        assert!(err.contains("版本过新"), "实际: {}", err);
    }

    #[test]
    fn test_parse_tolerates_bom_and_crlf() {
        let project = sample_project();
        let text = serialize_project(&project).unwrap();
        let crlf = format!("\u{FEFF}{}", text.replace('\n', "\r\n"));
        let back = parse_project(&crlf).unwrap();
        assert_eq!(back.name, "示例项目");
    }

    #[test]
    fn test_find_project_file_zero_one_many() {
        let dir = std::env::temp_dir().join(format!("gsa_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        // 0 个：未找到
        assert!(find_project_file(&dir).is_err());

        // 1 个：命中（扩展名不区分大小写）
        fs::write(dir.join("a.gsa"), "x").unwrap();
        assert_eq!(find_project_file(&dir).unwrap(), dir.join("a.gsa"));
        fs::write(dir.join("b.GSA"), "x").unwrap();

        // 多个：报错
        assert!(find_project_file(&dir).is_err());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_write_project_file_atomic_tmp_cleaned() {
        let dir = std::env::temp_dir().join(format!("gsa_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("p.gsa");
        write_project_file(&target, &sample_project()).unwrap();

        // 目标文件有魔数头，同目录不残留 .tmp
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.starts_with("GSA-PROJECT v1"));
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留临时文件");
        fs::remove_dir_all(&dir).ok();
    }

    // ── 多项目共存（项目身份 = .gsa 文件路径）──

    fn empty_project(name: &str) -> Project {
        Project {
            path: String::new(),
            video: String::new(),
            source_video: String::new(),
            name: name.into(),
            corpus: vec![],
            tracks: vec![],
            created_at: "1".into(),
            updated_at: "2".into(),
        }
    }

    fn new_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gsa_{}_{}", tag, Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_create_allows_sibling_projects_same_name_rejected() {
        let dir = new_test_dir("sibling");
        let a = create_project_in(&dir, "甲").unwrap();
        assert_eq!(
            a.path,
            project_file_path(&dir, "甲").to_string_lossy().to_string()
        );

        // 同名 → 拒绝；异名 → 允许共存
        assert!(create_project_in(&dir, "甲").is_err());
        let b = create_project_in(&dir, "乙").unwrap();
        assert_ne!(b.path, a.path);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_open_by_file_and_by_dir_legacy() {
        let dir = new_test_dir("open_modes");
        let a = create_project_in(&dir, "甲").unwrap();
        create_project_in(&dir, "乙").unwrap();

        // 按文件路径打开：命中对应项目，path = 文件
        let opened = open_project_at(&a.path).unwrap();
        assert_eq!(opened.name, "甲");
        assert_eq!(opened.path, a.path);

        // 目录内多个 .gsa → 目录形态打开报错（需给具体文件）
        let err = open_project_at(dir.to_string_lossy().as_ref()).unwrap_err();
        assert!(err.contains("多个"), "实际: {}", err);

        // 目录兼容分支：目录内唯一 .gsa → 打开成功（旧最近项目列表形态）
        let dir2 = new_test_dir("open_dir_legacy");
        let only = create_project_in(&dir2, "独苗").unwrap();
        let opened = open_project_at(dir2.to_string_lossy().as_ref()).unwrap();
        assert_eq!(opened.name, "独苗");
        assert_eq!(opened.path, only.path);
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&dir2).ok();
    }

    #[test]
    fn test_save_writes_own_file_only_no_cross_cleanup() {
        // P1-1 回归：同目录两个项目，保存（含改名）只写自己的文件，不动他者
        let dir = new_test_dir("save_isolation");
        let a = create_project_in(&dir, "甲").unwrap();
        create_project_in(&dir, "乙").unwrap();

        let mut renamed = open_project_at(&a.path).unwrap();
        renamed.name = "丙".into(); // 改名：只改 JSON 内字段，文件名不变
        let saved = save_project(renamed).unwrap();

        // 仍写回原文件（path 未变），乙项目文件原样保留
        assert_eq!(saved.path, a.path);
        let reopened = open_project_at(&saved.path).unwrap();
        assert_eq!(reopened.name, "丙");
        let sibling = open_project_at(
            &project_file_path(&dir, "乙").to_string_lossy().to_string()
        )
        .unwrap();
        assert_eq!(sibling.name, "乙");
        // 目录中仍恰好两个 .gsa，无新增无残留
        let count = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| has_project_ext(&e.path()))
            .count();
        assert_eq!(count, 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_save_and_set_video_reject_non_gsa_path() {
        let dir = new_test_dir("reject_non_gsa");
        // 旧语义（目录路径）混入 → 明确报错，而不是把项目写到错误目标
        let mut p = empty_project("甲");
        p.path = dir.to_string_lossy().to_string();
        assert!(save_project(p).is_err());

        let err =
            set_project_video(dir.to_string_lossy().to_string(), "v.mp4".into()).unwrap_err();
        assert!(err.contains(".gsa"), "实际: {}", err);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_set_project_video_by_file() {
        let dir = new_test_dir("set_video");
        let a = create_project_in(&dir, "甲").unwrap();
        let updated = set_project_video(a.path.clone(), "clip.mp4".into()).unwrap();
        assert_eq!(updated.video, "clip.mp4");
        assert_eq!(updated.path, a.path);
        let reopened = open_project_at(&a.path).unwrap();
        assert_eq!(reopened.video, "clip.mp4");
        fs::remove_dir_all(&dir).ok();
    }

    // ── 项目重命名 / 最近项目条目管理 ──

    fn recent_entry(path: &str) -> RecentProject {
        RecentProject {
            path: path.into(),
            name: path.into(),
            updated_at: "1".into(),
        }
    }

    #[test]
    fn test_rename_project_in_moves_file_and_updates_fields() {
        let dir = new_test_dir("rename_move");
        let a = create_project_in(&dir, "旧名").unwrap();
        let old_file = PathBuf::from(&a.path);

        let renamed = rename_project_in(&old_file, "新名").unwrap();
        let new_file = PathBuf::from(&renamed.path);

        assert!(!old_file.exists(), "旧项目文件应已移除");
        assert!(new_file.exists(), "新项目文件应已创建");
        assert_eq!(renamed.name, "新名");
        assert_eq!(
            renamed.path,
            project_file_path(&dir, "新名").to_string_lossy().to_string()
        );

        // 重命名后的文件可正常打开，name 已更新
        let reopened = open_project_at(&renamed.path).unwrap();
        assert_eq!(reopened.name, "新名");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_rename_project_in_rejects_existing_target() {
        let dir = new_test_dir("rename_conflict");
        create_project_in(&dir, "甲").unwrap();
        create_project_in(&dir, "乙").unwrap();
        let a_file = project_file_path(&dir, "甲");

        assert!(rename_project_in(&a_file, "乙").is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_rename_project_in_same_name_is_noop() {
        let dir = new_test_dir("rename_noop");
        let a = create_project_in(&dir, "甲").unwrap();

        let renamed = rename_project_in(Path::new(&a.path), "甲").unwrap();
        assert_eq!(renamed.path, a.path);
        assert!(Path::new(&a.path).exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_replace_recent_entry_replaces_in_place() {
        let recent = vec![recent_entry("p1"), recent_entry("p2")];
        let updated = replace_recent_entry(recent.clone(), "p2", recent_entry("p2x"));
        assert_eq!(updated.len(), 2);
        assert_eq!(updated[0].path, "p1"); // 原位替换，顺序不变
        assert_eq!(updated[1].path, "p2x");

        // 不在列表中：原样返回
        let untouched = replace_recent_entry(recent, "missing", recent_entry("x"));
        assert_eq!(untouched.len(), 2);
        assert_eq!(untouched[0].path, "p1");
        assert_eq!(untouched[1].path, "p2");
    }

    #[test]
    fn test_remove_recent_entry_removes_only_target() {
        let updated = remove_recent_entry(vec![recent_entry("p1"), recent_entry("p2")], "p1");
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].path, "p2");

        // 不在列表中：原样返回
        let untouched = remove_recent_entry(vec![recent_entry("p1")], "missing");
        assert_eq!(untouched.len(), 1);
    }
}
