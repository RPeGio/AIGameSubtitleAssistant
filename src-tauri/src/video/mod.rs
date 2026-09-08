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
use std::io::{BufReader, Read};
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

/// 内存中的抽取帧（mjpeg 管道产出，不落盘）
#[derive(Debug, Clone)]
pub struct FrameBytes {
    /// 网格序号（0 基，fps 滤镜输出顺序）
    pub index: usize,
    /// 该帧在视频中的时间点（秒）= start + index * interval
    pub time: f64,
    /// JPEG 编码字节（与旧落盘版同 `-q:v 2` 质量；直接 base64 后作为 OCR IPC 载荷）
    pub jpeg: Vec<u8>,
}

/// 网格抽帧结果：kept = 命中 keep 的帧（含字节），total = 流中实际帧总数
pub struct GridFrames {
    pub kept: Vec<FrameBytes>,
    pub total: usize,
}

/// 从 mjpeg 字节流中取出一个完整 JPEG（FFD8 开头、FFD9 结尾）。
///
/// JPEG 熵编码段中 0xFF 一律后跟填充（0x00）或真实标记，
/// 因此字节流中的 FFD9 只会是帧结束标记，按标记切分是安全的。
/// buf 中已完整的帧被取出后仅剩下一个未完成帧（≤ 数十 KB），重复扫描开销可忽略。
fn try_take_jpeg(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let soi = buf.windows(2).position(|w| w == [0xFF, 0xD8])?;
    if soi > 0 {
        buf.drain(..soi);
    }
    let eoi = buf.windows(2).position(|w| w == [0xFF, 0xD9])?;
    Some(buf.drain(..eoi + 2).collect())
}

/// 用 ffmpeg 在时间段内按固定间隔抽取裁切后的帧，经 mjpeg 管道返回**内存字节**。
///
/// 与旧文件版 `extract_frames` 同参数语义（fps 滤镜 + crop、`-q:v 2`），
/// 但帧不落盘：`keep` 列出要保留的网格序号（0 基，升序），其余帧读到即丢——
/// 变化帧集合已由零落盘扫描得知，这里只为 OCR 载荷保留字节。
///
/// 返回 kept 帧（升序）与流中实际帧总数。
#[allow(clippy::too_many_arguments)]
pub fn extract_frames_bytes(
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
    keep: &[usize],
) -> Result<GridFrames, String> {
    if interval_secs <= 0.0 {
        return Err("帧间隔必须为正数".into());
    }
    if end <= start {
        return Err("时间段无效（end 需大于 start）".into());
    }
    let ffmpeg = resolve_ffmpeg().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;
    let (x, y, w, h) = crop_geometry(x1, y1, x2, y2, video_w, video_h)?;

    let mut child = Command::new(&ffmpeg)
        .arg("-ss")
        .arg(start.to_string())
        .arg("-i")
        .arg(video_path)
        .arg("-t")
        .arg((end - start).to_string())
        .arg("-vf")
        .arg(format!(
            "fps={},crop={}:{}:{}:{}",
            1.0 / interval_secs,
            w,
            h,
            x,
            y
        ))
        .arg("-q:v")
        .arg("2")
        .arg("-f")
        .arg("mjpeg")
        .arg("pipe:1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 ffmpeg: {}", e))?;

    // stderr 由独立线程排空，避免管道缓冲区写满导致 ffmpeg 阻塞
    let mut stderr = child.stderr.take().expect("stderr 已 piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let mut stdout = BufReader::new(child.stdout.take().expect("stdout 已 piped"));
    let mut buf: Vec<u8> = Vec::with_capacity(256 * 1024);
    let mut chunk = [0u8; 64 * 1024];
    let mut kept: Vec<FrameBytes> = Vec::new();
    let mut total = 0usize;
    let mut keep_ptr = 0usize; // keep 升序，随帧序号单调推进
    let mut done = false;
    while !done {
        // 先尝试消化缓冲区里的完整帧
        while let Some(jpeg) = try_take_jpeg(&mut buf) {
            let k = total;
            total += 1;
            while keep_ptr < keep.len() && keep[keep_ptr] < k {
                keep_ptr += 1;
            }
            if keep_ptr < keep.len() && keep[keep_ptr] == k {
                kept.push(FrameBytes {
                    index: k,
                    time: start + (k as f64) * interval_secs,
                    jpeg,
                });
            }
        }
        // 再读入更多字节
        match stdout.read(&mut chunk) {
            Ok(0) => done = true,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => {
                let _ = child.kill();
                return Err(format!("ffmpeg 抽帧输出读取失败: {}", e));
            }
        }
    }

    let status = child.wait().map_err(|e| format!("ffmpeg 进程等待失败: {}", e))?;
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "ffmpeg 抽帧失败: {}",
            String::from_utf8_lossy(&stderr_buf)
        ));
    }
    Ok(GridFrames { kept, total })
}

/// 抽取指定时间点裁切区域的单帧 JPEG 字节（窗口精化的短字幕召回用，不落盘）。
#[allow(clippy::too_many_arguments)]
pub fn extract_single_frame_bytes(
    video_path: &str,
    time: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    video_w: u32,
    video_h: u32,
) -> Result<Vec<u8>, String> {
    let ffmpeg = resolve_ffmpeg().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;
    let (x, y, w, h) = crop_geometry(x1, y1, x2, y2, video_w, video_h)?;

    let mut child = Command::new(&ffmpeg)
        .arg("-y")
        .arg("-ss")
        .arg(time.to_string())
        .arg("-i")
        .arg(video_path)
        .arg("-frames:v")
        .arg("1")
        .arg("-vf")
        .arg(format!("crop={}:{}:{}:{}", w, h, x, y))
        .arg("-q:v")
        .arg("2")
        .arg("-f")
        .arg("mjpeg")
        .arg("pipe:1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 ffmpeg: {}", e))?;

    let mut stderr = child.stderr.take().expect("stderr 已 piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let mut stdout = child.stdout.take().expect("stdout 已 piped");
    let mut jpeg = Vec::new();
    stdout
        .read_to_end(&mut jpeg)
        .map_err(|e| format!("ffmpeg 单帧输出读取失败: {}", e))?;

    let status = child.wait().map_err(|e| format!("ffmpeg 进程等待失败: {}", e))?;
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "ffmpeg 单帧抽取失败: {}",
            String::from_utf8_lossy(&stderr_buf)
        ));
    }
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return Err("ffmpeg 单帧抽取输出不是 JPEG".into());
    }
    Ok(jpeg)
}

// ─── 零落盘扫描（OCR 变化检测用）──────────────────────────

/// 以固定间隔扫描时间段内裁切区域的 dHash，全程不落盘。
///
/// ffmpeg 把裁切区域缩放为 9×8 灰度后经 rawvideo 管道输出（每帧恰 72 字节），
/// Rust 逐帧读取并计算 dHash。替代"密帧 JPEG 落盘 → 逐张读回算哈希"的旧路径：
/// 哈希用途的帧不再产生任何磁盘写入。
///
/// 帧时间推导与 extract_frames 一致：第 k 帧（0 基）时间 = start + k * interval_secs。
/// 返回按时间升序的 (时间, dHash) 序列。
#[allow(clippy::too_many_arguments)]
pub fn scan_frame_hashes(
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
) -> Result<Vec<(f64, u64)>, String> {
    if interval_secs <= 0.0 {
        return Err("扫描帧间隔必须为正数".into());
    }
    if end <= start {
        return Err("时间段无效（end 需大于 start）".into());
    }
    let ffmpeg = resolve_ffmpeg().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;
    let (x, y, w, h) = crop_geometry(x1, y1, x2, y2, video_w, video_h)?;

    let mut child = Command::new(&ffmpeg)
        .arg("-ss")
        .arg(start.to_string())
        .arg("-i")
        .arg(video_path)
        .arg("-t")
        .arg((end - start).to_string())
        .arg("-vf")
        // flags=area：面积平均缩放，大幅降采样时比默认 bicubic 抗锯齿，哈希更稳定
        .arg(format!(
            "fps={},crop={}:{}:{}:{},format=gray,scale=9:8:flags=area",
            1.0 / interval_secs,
            w,
            h,
            x,
            y
        ))
        .arg("-f")
        .arg("rawvideo")
        .arg("pipe:1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 ffmpeg: {}", e))?;

    // stderr 由独立线程排空：管道缓冲区写满会让 ffmpeg 阻塞，拖死 stdout 读取
    let mut stderr = child.stderr.take().expect("stderr 已 piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let mut stdout = BufReader::new(child.stdout.take().expect("stdout 已 piped"));
    const FRAME_BYTES: usize = 9 * 8;
    let mut buf = [0u8; FRAME_BYTES];
    let mut out: Vec<(f64, u64)> = Vec::new();
    loop {
        match stdout.read_exact(&mut buf) {
            Ok(()) => {
                let time = start + (out.len() as f64) * interval_secs;
                out.push((time, crate::ai_runtime::dhash::dhash_gray9x8(&buf)));
            }
            // rawvideo 帧是原子写入：正常结束恰好整帧对齐，UnexpectedEof 即流结束；
            // 若 ffmpeg 中途被杀，下方退出码校验兜底报错
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                let _ = child.kill();
                return Err(format!("ffmpeg 扫描输出读取失败: {}", e));
            }
        }
    }

    let status = child.wait().map_err(|e| format!("ffmpeg 进程等待失败: {}", e))?;
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "ffmpeg 扫描失败: {}",
            String::from_utf8_lossy(&stderr_buf)
        ));
    }
    Ok(out)
}

// ─── 音频提取（ASR 用）────────────────────────────────────

/// 提取视频音轨为 16kHz 单声道 PCM WAV（MOSS 的输入格式）。
///
/// # 参数
/// - `video_path`: 视频绝对路径
/// - `out_dir`: 输出目录（写入 audio.wav）
///
/// 返回生成的 audio.wav 路径。
/// 无音轨时返回友好错误（录屏文件常见）。
pub fn extract_audio(video_path: &str, out_dir: &Path) -> Result<PathBuf, String> {
    let ffmpeg = resolve_ffmpeg().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;
    fs::create_dir_all(out_dir).map_err(|e| format!("无法创建音频目录: {}", e))?;

    let output_path = out_dir.join("audio.wav");
    let output = Command::new(&ffmpeg)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&output_path)
        .output()
        .map_err(|e| format!("无法执行 ffmpeg: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // 录屏无音轨是常见场景，映射为友好错误
        if stderr.contains("does not contain any stream") {
            return Err("视频不包含音轨".into());
        }
        return Err(format!("ffmpeg 音频提取失败: {}", stderr));
    }
    if !output_path.is_file() {
        return Err("ffmpeg 退出码为 0 但音频文件未生成".into());
    }
    Ok(output_path)
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
    fn test_extract_frames_bytes_rejects_bad_params() {
        assert!(extract_frames_bytes("v.mp4", 0.0, 10.0, 0.1, 0.7, 0.9, 0.9, 1920, 1080, 0.0, &[])
            .is_err());
        assert!(extract_frames_bytes("v.mp4", 10.0, 5.0, 0.1, 0.7, 0.9, 0.9, 1920, 1080, 1.0, &[])
            .is_err());
    }

    #[test]
    fn test_try_take_jpeg_splits_marker_stream() {
        // 两个帧拼接的字节流：按 SOI/EOI 标记正确切分
        let a = vec![0xFF, 0xD8, 0x01, 0x02, 0xFF, 0xD9];
        let b = vec![0xFF, 0xD8, 0x03, 0xFF, 0xD9];
        let mut buf = Vec::new();
        buf.extend_from_slice(&a);
        buf.extend_from_slice(&b);
        let first = try_take_jpeg(&mut buf).unwrap();
        assert_eq!(first, a);
        let second = try_take_jpeg(&mut buf).unwrap();
        assert_eq!(second, b);
        assert!(try_take_jpeg(&mut buf).is_none());
    }

    #[test]
    fn test_try_take_jpeg_skips_leading_noise() {
        // SOI 前有杂散字节 → 跳过；不完整帧 → None 等待更多数据
        let mut buf = vec![0x00, 0x01, 0xFF, 0xD8, 0x09];
        assert!(try_take_jpeg(&mut buf).is_none());
        buf.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(try_take_jpeg(&mut buf).unwrap(), vec![0xFF, 0xD8, 0x09, 0xFF, 0xD9]);
    }
}
