// ─── 视频处理模块 ─────────────────────────────────────────
//
// 当前阶段 (Phase 1):
//   get_video_metadata: 通过 ffprobe 获取视频时长、分辨率、帧率等信息
//
// 后续阶段会加入：
//   - 按帧率截帧（为 OCR 提供图片）
//   - 提取音频流（为 ASR 提供 PCM/WAV）
//   - 字幕区域裁剪

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

// ─── 公开数据结构 ─────────────────────────────────────────

/// 视频元数据 —— 前端展示和后续处理的基础信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    /// 视频文件的绝对路径
    pub path: String,
    /// 视频总时长（秒）
    pub duration: f64,
    /// 画面宽度（像素）
    pub width: u32,
    /// 画面高度（像素）
    pub height: u32,
    /// 帧率（fps，如 29.97）
    pub fps: f64,
    /// 视频编码格式（如 h264, hevc）
    pub codec: String,
    /// 音频编码格式（如 aac, mp3），无音轨时为 None
    pub audio_codec: Option<String>,
}

// ─── ffprobe JSON 反序列化结构 ────────────────────────────
// 只反序列化我们需要的字段，忽略其余

#[derive(Deserialize)]
struct FfprobeOutput {
    streams: Vec<FfprobeStream>,
    format: FfprobeFormat,
}

#[derive(Deserialize)]
struct FfprobeStream {
    codec_type: String,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    /// ffprobe 返回的帧率是分数形式的字符串，如 "30000/1001"
    r_frame_rate: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

// ─── ffprobe 查找 ─────────────────────────────────────────
// 优先级：
//   1. 系统 PATH（用户自己装了 ffmpeg）
//   2. ffmpeg-sidecar 管理目录（auto_download 下载的位置）
//   3. [预留] 用户自定义路径（后续由启动时的下载器写入 config）

/// 在系统上定位可用的 ffprobe 二进制文件
fn resolve_ffprobe() -> Option<PathBuf> {
    // 策略 1：系统 PATH
    // 直接尝试运行 "ffprobe -version"
    // Windows 的 CreateProcess 会自动搜索 PATH 中的所有目录
    let mut probe_cmd = Command::new("ffprobe");
    probe_cmd
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if probe_cmd.output().is_ok() {
        return Some(PathBuf::from("ffprobe"));
    }

    // 策略 2：ffmpeg-sidecar 的 sidecar 目录
    // auto_download() 会把 ffmpeg/ffprobe/ffplay 下载到 sidecar_dir 中
    if let Ok(dir) = ffmpeg_sidecar::paths::sidecar_dir() {
        let probe_path = if cfg!(target_os = "windows") {
            dir.join("ffprobe.exe")
        } else {
            dir.join("ffprobe")
        };
        if probe_path.exists() {
            return Some(probe_path);
        }
    }

    None
}

// ─── 辅助函数 ─────────────────────────────────────────────

/// 将 ffprobe 返回的分数帧率（如 "30000/1001"）解析为浮点数
fn parse_fraction_fps(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    let numerator: f64 = parts[0].parse().ok()?;
    let denominator: f64 = parts[1].parse().ok()?;
    if denominator == 0.0 {
        return None;
    }
    Some(numerator / denominator)
}

// ─── 核心逻辑 ─────────────────────────────────────────────

/// 将反序列化后的 ffprobe JSON 转换为 VideoMetadata
fn ffprobe_output_to_metadata(parsed: &FfprobeOutput, path: &str) -> Result<VideoMetadata, String> {
    let video_stream = parsed
        .streams
        .iter()
        .find(|s| s.codec_type == "video")
        .ok_or_else(|| "未找到视频流".to_string())?;

    let fps = video_stream
        .r_frame_rate
        .as_deref()
        .and_then(parse_fraction_fps)
        .unwrap_or(0.0);

    let audio_codec = parsed
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .and_then(|s| s.codec_name.clone());

    let duration = parsed
        .format
        .duration
        .as_deref()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    Ok(VideoMetadata {
        path: path.to_string(),
        duration,
        width: video_stream.width.unwrap_or(0),
        height: video_stream.height.unwrap_or(0),
        fps,
        codec: video_stream.codec_name.clone().unwrap_or_default(),
        audio_codec,
    })
}

/// 调用 ffprobe 并解析其 JSON 输出，提取视频元数据
fn probe_video(path: &str, ffprobe: &PathBuf) -> Result<VideoMetadata, String> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("无法执行 ffprobe: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe 执行失败: {}", stderr));
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("ffprobe JSON 解析失败: {}", e))?;

    ffprobe_output_to_metadata(&parsed, path)
}

// ─── Tauri 命令 ───────────────────────────────────────────

/// Tauri 命令：获取视频文件元数据
///
/// # 参数
/// - `path`: 视频文件的绝对路径
///
/// # 返回值
/// - `Ok(VideoMetadata)`: 视频元数据
/// - `Err("FFMPEG_NOT_FOUND")`: 系统未安装 ffmpeg/ffprobe
#[tauri::command]
pub fn get_video_metadata(path: String) -> Result<VideoMetadata, String> {
    let ffprobe = resolve_ffprobe().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;
    probe_video(&path, &ffprobe)
}
