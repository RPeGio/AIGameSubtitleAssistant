// ─── 音频波形峰值提取 ─────────────────────────────────────
//
// get_audio_peaks：为前端主时间轴的音频波形层提供数据。
// 用低采样率裸 PCM 流（不落盘）按固定时间窗聚合峰值，
// 返回每窗最大绝对振幅（线性 0.0~1.0，已按时长内最大值归一化）。
//
// 前端拿到的是均匀时间轴上的相对高度数组，下标 i 对应 [i/50, (i+1)/50) 秒。

use std::io::{BufReader, Read};
use std::process::Command;
use tauri::command;

// ─── 常量 ─────────────────────────────────────────────────

/// 内部解码采样率：波形显示足够，同时把管道流量控制在 16KB/s
const INTERNAL_SAMPLE_RATE: u32 = 8000;
/// 峰值窗密度：每秒 50 窗（20ms/窗）。
/// 前端缩放上限 800px/s 下每窗仍有 16px 宽，足够；更粗的显示由前端降采样。
/// 数组下标 i 对应时间 [i/50, (i+1)/50) 秒。
/// 与 INTERNAL_SAMPLE_RATE 一起算出每窗样本数：8000/50 = 160。
const SAMPLES_PER_WINDOW: usize = 160;
/// 每窗字节数：160 样本 × 2 字节（s16le）
const BYTES_PER_WINDOW: usize = SAMPLES_PER_WINDOW * 2;

// ─── 命令 ─────────────────────────────────────────────────

/// 提取整条音轨的峰值包络，供时间轴绘制波形。
///
/// 无音轨时报"视频不包含音轨"（录屏文件常见），
/// 前端也可先用 VideoMetadata.audio_codec 预判跳过调用。
#[command]
pub async fn get_audio_peaks(video_path: String) -> Result<Vec<f32>, String> {
    let ffmpeg = super::resolve_ffmpeg().ok_or_else(|| "FFMPEG_NOT_FOUND".to_string())?;

    let mut child = Command::new(&ffmpeg)
        .arg("-i")
        .arg(&video_path)
        .arg("-vn") // 只要音频
        .arg("-ac")
        .arg("1") // 混成单声道
        .arg("-ar")
        .arg(INTERNAL_SAMPLE_RATE.to_string()) // 降采样重采样
        .arg("-f")
        .arg("s16le") // 无容器无头的 16bit 小端裸 PCM
        .arg("pipe:1") // 写 stdout，不落盘
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
    let mut windows: Vec<f32> = Vec::new();
    let mut buf = [0u8; BYTES_PER_WINDOW];
    let mut last_partial: Vec<u8> = Vec::new();
    loop {
        match stdout.read_exact(&mut buf) {
            Ok(()) => windows.push(window_peak(&buf)),
            // PCM 是无边界结构的裸流：末尾不足一窗属正常收尾，
            // 把剩余字节凑成最后一窗（非 2 字节对齐的尾巴会被解析时丢弃）
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                let _ = stdout.read_to_end(&mut last_partial);
                break;
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("ffmpeg 波形输出读取失败: {}", e));
            }
        }
    }
    if !last_partial.is_empty() {
        windows.push(window_peak(&last_partial));
    }

    let status = child.wait().map_err(|e| format!("ffmpeg 进程等待失败: {}", e))?;
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr_buf);
        // 无音轨时 ffmpeg 以非零码退出，stderr 含该固定句式 → 映射为现有友好错误
        if msg.contains("does not contain any stream") {
            return Err("视频不包含音轨".into());
        }
        return Err(format!("ffmpeg 波形提取失败: {}", msg));
    }

    // 全局峰值归一化：让安静录制也能铺满可视高度；全静音保持全零
    let max = windows.iter().fold(0.0f32, |m, &v| if v > m { v } else { m });
    if max > 1e-9 {
        let inv = 1.0 / max;
        for v in windows.iter_mut() {
            *v *= inv;
        }
    }
    Ok(windows)
}

// ─── 峰值聚合 ─────────────────────────────────────────────

/// 一窗 PCM 内取最大绝对振幅（线性 0.0~1.0）。
///
/// PCM 是无容器裸流，read_exact 按固定字节窗切分：
/// 样本严格 2 字节小端 int16，奇数尾巴（极端收尾）会被 chunks_exact 丢弃。
/// -32768 的 abs 在 i16 内会溢出，所以先提升为 i32 再取绝对值。
fn window_peak(bytes: &[u8]) -> f32 {
    let mut peak: i32 = 0;
    for sample in bytes.chunks_exact(2) {
        let s = i16::from_le_bytes([sample[0], sample[1]]) as i32;
        if s.abs() > peak {
            peak = s.abs();
        }
    }
    peak as f32 / 32768.0
}
