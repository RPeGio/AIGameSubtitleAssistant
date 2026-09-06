use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
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
    /// 项目文件夹的绝对路径
    pub path: String,
    /// 切片视频文件路径（相对于项目文件夹或绝对路径）——时间轴基准
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
        source_video: String::new(),
        name,
        corpus: Vec::new(),
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

/// 保存项目（写入 project.json），返回带新 updated_at 的 Project
///
/// - `project`: 要保存的项目对象
#[tauri::command]
pub fn save_project(project: Project) -> Result<Project, String> {
    let project_path = PathBuf::from(&project.path);
    let project_file = project_path.join(PROJECT_FILE);

    let mut updated = project;
    updated.updated_at = now_iso();

    let json = serde_json::to_string_pretty(&updated)
        .map_err(|e| format!("项目序列化失败: {}", e))?;
    fs::write(&project_file, &json)
        .map_err(|e| format!("无法写入项目文件: {}", e))?;

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
#[tauri::command]
pub fn set_project_video(project_path: String, video_path: String) -> Result<Project, String> {
    let project_file = PathBuf::from(&project_path).join(PROJECT_FILE);

    let json = fs::read_to_string(&project_file)
        .map_err(|e| format!("无法读取项目文件: {}", e))?;
    let mut project: Project = serde_json::from_str(&json)
        .map_err(|e| format!("项目文件解析失败: {}", e))?;

    project.video = video_path;
    project.path = project_path;
    project.updated_at = now_iso();

    let new_json = serde_json::to_string_pretty(&project)
        .map_err(|e| format!("项目序列化失败: {}", e))?;
    fs::write(&project_file, &new_json)
        .map_err(|e| format!("无法写入项目文件: {}", e))?;

    Ok(project)
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

    let file = get_app_data_file(app, RECENT_PROJECTS_FILE);
    let json = serde_json::to_string_pretty(&recent)
        .map_err(|e| format!("序列化最近项目列表失败: {}", e))?;
    fs::write(&file, &json)
        .map_err(|e| format!("写入最近项目列表失败: {}", e))?;

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
        // 旧 project.json 无 track_role 字段 → 反序列化默认 "game"
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
        // 旧 project.json 无 preview_visible → 默认 true（预览不遗漏旧轨道）
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
    fn test_track_legacy_json_defaults_scope_page_video() {
        // 旧 project.json 无 scope/page/video → 默认 output / 空 / clip（既有轨道视为产物轨，挂切片）
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
        // 旧 project.json 无 source_video/corpus → 默认空串 / 空语料
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
}
