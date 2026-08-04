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
use std::fs;
use std::path::{Path, PathBuf};
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

// ─── 帧提取（OCR 用）──────────────────────────────────────

/// 在系统上定位可用的 ffmpeg 二进制文件（与 resolve_ffprobe 同策略）
fn resolve_ffmpeg() -> Option<PathBuf> {
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if cmd.output().is_ok() {
        return Some(PathBuf::from("ffmpeg"));
    }

    if let Ok(dir) = ffmpeg_sidecar::paths::sidecar_dir() {
        let path = if cfg!(target_os = "windows") {
            dir.join("ffmpeg.exe")
        } else {
            dir.join("ffmpeg")
        };
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// 归一化选区坐标 → 整数 crop 框 (x, y, w, h)
/// 做坐标钳制、最小 1px 尺寸、边界约束
fn crop_geometry(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    video_w: u32,
    video_h: u32,
) -> Result<(u32, u32, u32, u32), String> {
    if video_w == 0 || video_h == 0 {
        return Err("视频宽高无效".into());
    }
    let x1 = x1.clamp(0.0, 1.0);
    let y1 = y1.clamp(0.0, 1.0);
    let x2 = x2.clamp(0.0, 1.0);
    let y2 = y2.clamp(0.0, 1.0);
    if x2 <= x1 || y2 <= y1 {
        return Err("无效的选区（宽或高为 0）".into());
    }

    let x = (x1 * video_w as f64).round() as u32;
    let y = (y1 * video_h as f64).round() as u32;
    let x2_px = (x2 * video_w as f64).round() as u32;
    let y2_px = (y2 * video_h as f64).round() as u32;
    let w = x2_px.saturating_sub(x).max(1);
    let h = y2_px.saturating_sub(y).max(1);
    let x = x.min(video_w.saturating_sub(w));
    let y = y.min(video_h.saturating_sub(h));
    Ok((x, y, w, h))
}

/// 提取出的一帧
#[derive(Debug, Clone)]
pub struct ExtractedFrame {
    pub path: PathBuf,
    /// 该帧在视频中的时间点（秒）
    pub time: f64,
}

/// 用 ffmpeg 在时间段内按固定间隔抽取裁切后的帧
///
/// # 参数
/// - `video_path`: 视频绝对路径
/// - `start`/`end`: 时间段（秒）
/// - `x1,y1,x2,y2`: 归一化选区（0~1）
/// - `video_w`/`video_h`: 视频宽高（来自 ffprobe 元数据）
/// - `interval_secs`: 帧间隔（秒）
/// - `out_dir`: 输出目录（帧写入 frame_%05d.jpg）
///
/// 返回按时间排序的帧列表。
pub fn extract_frames(
    video_path: &str,
    start: f64,
    end: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    video_w: u32,
    video_h: u32,
    interval_secs: f64,
    out_dir: &Path,
) -> Result<Vec<ExtractedFrame>, String> {
    if interval_secs <= 0.0 {
        return Err("帧间隔必须为正数".into());
    }
    if end <= start {
        return Err("时间段无效（end 需大于 start）".into());
    }
    let ffmpeg = resolve_ffmpeg().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;
    let (x, y, w, h) = crop_geometry(x1, y1, x2, y2, video_w, video_h)?;
    fs::create_dir_all(out_dir).map_err(|e| format!("无法创建帧目录: {}", e))?;

    let fps = 1.0 / interval_secs;
    let output_pattern = out_dir.join("frame_%05d.jpg");
    let output = Command::new(&ffmpeg)
        .arg("-y")
        .arg("-ss")
        .arg(start.to_string())
        .arg("-i")
        .arg(video_path)
        .arg("-t")
        .arg((end - start).to_string())
        .arg("-vf")
        .arg(format!("fps={},crop={}:{}:{}:{}", fps, w, h, x, y))
        .arg("-q:v")
        .arg("2")
        .arg(&output_pattern)
        .output()
        .map_err(|e| format!("无法执行 ffmpeg: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg 抽帧失败: {}", stderr));
    }

    // 枚举输出帧，按序号排序，逐帧推导时间（frame_00001 → start）
    let mut indexed: Vec<(usize, PathBuf)> = fs::read_dir(out_dir)
        .map_err(|e| format!("无法读取帧目录: {}", e))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let index = name.strip_prefix("frame_")?.strip_suffix(".jpg")?;
            let idx: usize = index.parse().ok()?;
            Some((idx, entry.path()))
        })
        .collect();
    indexed.sort_by_key(|(idx, _)| *idx);

    let frames = indexed
        .into_iter()
        .map(|(idx, path)| ExtractedFrame {
            path,
            time: start + (idx as f64 - 1.0) * interval_secs,
        })
        .collect();
    Ok(frames)
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crop_geometry_normal() {
        let (x, y, w, h) = crop_geometry(0.2, 0.7, 0.8, 0.9, 1920, 1080).unwrap();
        assert_eq!(x, 384);
        assert_eq!(y, 756);
        assert_eq!(w, 1152);
        assert_eq!(h, 216);
    }

    #[test]
    fn test_crop_geometry_clamps_out_of_bounds() {
        let (x, y, w, h) = crop_geometry(-0.1, 0.0, 1.2, 1.0, 1920, 1080).unwrap();
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn test_crop_geometry_min_size() {
        let (_, _, w, h) = crop_geometry(0.0, 0.0, 0.001, 0.001, 1920, 1080).unwrap();
        assert!(w >= 1);
        assert!(h >= 1);
    }

    #[test]
    fn test_crop_geometry_degenerate() {
        // x2 == x1 → 宽度为 0
        assert!(crop_geometry(0.5, 0.5, 0.5, 0.7, 1920, 1080).is_err());
    }

    #[test]
    fn test_crop_geometry_zero_video() {
        assert!(crop_geometry(0.0, 0.0, 1.0, 1.0, 0, 0).is_err());
    }

    #[test]
    fn test_extract_frames_rejects_bad_params() {
        assert!(extract_frames("v.mp4", 0.0, 10.0, 0.1, 0.7, 0.9, 0.9, 1920, 1080, 0.0, Path::new("tmp"))
            .is_err());
        assert!(extract_frames("v.mp4", 10.0, 5.0, 0.1, 0.7, 0.9, 0.9, 1920, 1080, 1.0, Path::new("tmp"))
            .is_err());
    }
}
