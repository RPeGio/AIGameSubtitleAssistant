// ─── MOSS 转写 CLI Provider ───────────────────────────────
// 一次性子进程：`moss-transcribe transcribe <model.gguf> <audio.wav> --format json`。
// stdout 输出 JSON 段数组，stderr 是日志（量小，可整体收集）。
// 模型每次转写加载一次（~1.4s），相对长音频推理（数分钟）可忽略，故不做常驻进程。
//
// 进程生命周期：转写子进程登记在 active_child，支持：
//   - cancel()：取消转写（kill 子进程），供前端"取消 ASR"与超时使用
//   - 应用退出钩子调用 cancel()，避免关应用后孤儿 moss 进程残留占满 CPU
//
// 防残留叠加：每次转写前用 tasklist/taskkill 清理系统上历史残留的
// moss-transcribe 孤儿进程（异常崩溃等可能绕过退出钩子，导致多实例抢 CPU）。

use crate::ai_runtime::{
    resolve_path, AsrError, AsrProvider, AsrSegment, AsrTranscribeOptions, RuntimeConfig,
};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// MOSS CLI 输出的单段（JSON 数组元素）。多余字段（id）由 serde 默认忽略。
#[derive(Deserialize)]
struct RawSegment {
    start: f64,
    end: f64,
    speaker: String,
    text: String,
}

/// GGUF 内嵌的默认生成 token 上限（mtd.default_max_new_tokens=5120）。
/// moss-transcribe CLI 未传 --max-new 时用它，长音频会在该处被硬截断
/// （实测约 5120 token 仅覆盖 ~11.6 分钟音频的转录量）。
const DEFAULT_MAX_NEW_TOKENS: u32 = 5120;

/// 每秒钟音频的生成 token 消耗率（实测 5120 token / 693s ≈ 7.4 token/s）。
/// 取 15 作为安全系数（约 2 倍余量），覆盖对话更密集的游戏剧情场景。
const MAX_NEW_TOKENS_PER_SECOND: f64 = 15.0;

/// 超过该时长（秒）的音频触发分段转写。
/// 依据：moss-transcribe.cpp 单次 prefill 全量 seq，注意力激活随 seq² 增长，
/// 8GB 显存安全段长 ≈ 8 分钟（seq≈6000，prefill 激活 ~2.8GB）。
/// 分段逐段转写可同时规避 prefill OOM 与 max_new=5120 截断。
const SEGMENT_SECONDS: u32 = 480;

/// WAV 解析结果：时长（秒）与 PCM 布局（采样率/声道/位深/data 偏移）。
struct WavInfo {
    seconds: f64,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    /// data 子块在文件中的偏移
    data_offset: u32,
    /// data 子块字节数
    data_len: u32,
}

/// 解析 PCM WAV 头（RIFF/fmt/data），返回时长与布局。
/// ffmpeg 生成的 wav 在 fmt 后可能插 LIST（INFO）等块，故按块链遍历找 data。
/// 解析失败返回 None（调用方回退默认，不阻断转写）。
fn wav_info(path: &Path) -> Option<WavInfo> {
    let mut f = std::fs::File::open(path).ok()?;
    // 头部读大些以容纳 fmt + 可能的 LIST/扩展块（常见 ffmpeg 输出 < 256B）
    let mut header = [0u8; 1024];
    // read() 不保证读满，需循环读满（或读到 EOF）
    let mut n = 0usize;
    while n < header.len() {
        match f.read(&mut header[n..]) {
            Ok(0) => break,
            Ok(m) => n += m,
            Err(_) => return None,
        }
    }
    let header = &header[..n];
    if header.len() < 12 || &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }
    // 遍历块链：@12 起，每块 8 字节头（id+size），块体后按 2 字节对齐推进
    let mut audio_format = 0u16;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut data_offset = 0u32;
    let mut data_len = 0u32;
    let mut pos = 12usize;
    while pos + 8 <= header.len() {
        let id = &header[pos..pos + 4];
        let size = u32::from_le_bytes([header[pos + 4], header[pos + 5], header[pos + 6], header[pos + 7]]) as usize;
        let body = pos + 8;
        match id {
            b"fmt " if size >= 16 => {
                // fmt 块体小（通常 16-40B），需完整读到
                if body + 16 > header.len() {
                    break;
                }
                audio_format = u16::from_le_bytes([header[body], header[body + 1]]);
                channels = u16::from_le_bytes([header[body + 2], header[body + 3]]);
                sample_rate = u32::from_le_bytes([
                    header[body + 4], header[body + 5], header[body + 6], header[body + 7],
                ]);
                bits_per_sample = u16::from_le_bytes([header[body + 14], header[body + 15]]);
            }
            b"data" => {
                // data 块体通常巨大且超出头部缓冲：只取 id+size 即可
                data_offset = body as u32;
                data_len = size as u32;
                break;
            }
            _ => {
                // 其他块（LIST 等）：块体超出头部则无法跳过，终止遍历
                if body + size > header.len() {
                    break;
                }
            }
        }
        // 块体按 2 字节对齐（WAV 规范：奇数大小补 1 字节）
        pos = body + size + (size & 1);
    }
    if audio_format != 1 || channels == 0 || sample_rate == 0 || bits_per_sample == 0 || data_len == 0 {
        return None;
    }
    let bytes_per_sample = (channels as u32) * (bits_per_sample as u32 / 8);
    if bytes_per_sample == 0 {
        return None;
    }
    let seconds = data_len as f64 / (sample_rate as f64 * bytes_per_sample as f64);
    Some(WavInfo {
        seconds,
        sample_rate,
        channels,
        bits_per_sample,
        data_offset,
        data_len,
    })
}

/// 依据 WAV 时长计算 `--max-new`：保证长音频不被默认 5120 上限截断。
/// 解析失败时回退默认值。调用方应保证 audio_path 是完整（未分段）文件，
/// 分段场景逐段计算（每段时长更短，预算安全）。
fn compute_max_new(seconds: f64) -> u32 {
    if seconds <= 0.0 {
        return DEFAULT_MAX_NEW_TOKENS;
    }
    let estimated = (seconds * MAX_NEW_TOKENS_PER_SECOND).ceil() as u32;
    estimated.max(DEFAULT_MAX_NEW_TOKENS)
}

/// 从 PCM WAV 切出 [start_sec, end_sec) 子段，写成标准 44 字节头 + data 子块。
/// 帧对齐（按 bytes_per_sample 对齐），end_sec 超过尾部时截到尾部。
fn slice_wav(
    src: &Path,
    info: &WavInfo,
    start_sec: u32,
    end_sec: u32,
    out: &Path,
) -> Result<(), AsrError> {
    let bytes_per_sample = info.channels as u32 * (info.bits_per_sample as u32 / 8);
    let start_byte = start_sec as u64 * info.sample_rate as u64 * bytes_per_sample as u64;
    let end_byte = end_sec as u64 * info.sample_rate as u64 * bytes_per_sample as u64;
    let start_byte = start_byte.min(info.data_len as u64);
    let end_byte = end_byte.min(info.data_len as u64).max(start_byte);

    let f = std::fs::File::open(src)
        .map_err(|e| AsrError::Worker(format!("MOSS 分段读取失败: {}", e)))?;
    use std::io::{Seek, SeekFrom};
    let mut f = std::io::BufReader::new(f);
    f.seek(SeekFrom::Start(info.data_offset as u64 + start_byte))
        .map_err(|e| AsrError::Worker(format!("MOSS 分段定位失败: {}", e)))?;
    let mut data = vec![0u8; (end_byte - start_byte) as usize];
    f.read_exact(&mut data)
        .map_err(|e| AsrError::Worker(format!("MOSS 分段读取失败: {}", e)))?;

    // 标准 44 字节 WAV 头
    let mut out_buf = Vec::with_capacity(44 + data.len());
    let riff_size = 36u32 + data.len() as u32;
    out_buf.extend_from_slice(b"RIFF");
    out_buf.extend_from_slice(&riff_size.to_le_bytes());
    out_buf.extend_from_slice(b"WAVE");
    out_buf.extend_from_slice(b"fmt ");
    out_buf.extend_from_slice(&16u32.to_le_bytes());
    out_buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out_buf.extend_from_slice(&info.channels.to_le_bytes());
    out_buf.extend_from_slice(&info.sample_rate.to_le_bytes());
    let byte_rate = info.sample_rate * info.channels as u32 * (info.bits_per_sample as u32 / 8);
    out_buf.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = info.channels * (info.bits_per_sample / 8);
    out_buf.extend_from_slice(&block_align.to_le_bytes());
    out_buf.extend_from_slice(&info.bits_per_sample.to_le_bytes());
    out_buf.extend_from_slice(b"data");
    out_buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out_buf.extend_from_slice(&data);

    std::fs::write(out, &out_buf)
        .map_err(|e| AsrError::Worker(format!("MOSS 分段写入失败: {}", e)))
}

/// 解析 `--format json` 的 stdout：JSON 段数组 → AsrSegment 列表。
/// MOSS 不输出置信度，confidence 置 None。
fn parse_segments(json: &str) -> Result<Vec<AsrSegment>, AsrError> {
    let raw: Vec<RawSegment> = serde_json::from_str(json)
        .map_err(|e| AsrError::Worker(format!("MOSS 输出解析失败: {}", e)))?;
    Ok(raw
        .into_iter()
        .map(|s| AsrSegment {
            start: s.start,
            end: s.end,
            text: s.text,
            speaker: Some(s.speaker),
            confidence: None,
        })
        .collect())
}

/// MOSS 转写 provider —— 每次 transcribe 启动一次性子进程
pub struct MossProvider {
    binary: PathBuf,
    model: PathBuf,
    /// 推理线程数：0 = 不设置 MTD_THREADS（CLI 默认全核）
    threads: u32,
    /// 推理后端设备：空 = 不设置 MTD_DEVICE（ggml 自动选最优）| "cpu" | "cuda" | "vulkan"
    device: String,
    /// 转写超时（分钟）：0 = 不限；超时终止子进程并报错
    timeout_minutes: u32,
    ready: AtomicBool,
    /// 取消标志：cancel() 置位，transcribe 轮询检测后返回"已取消"
    cancelled: AtomicBool,
    /// 最近一次错误（供 describe() 向状态探测展示）
    last_error: Mutex<Option<String>>,
    /// 当前活动的转写子进程（供取消/退出钩子终止）
    active_child: Mutex<Option<Child>>,
    /// 开发调试：打印运行时细节日志
    dev_debug: bool,
}

impl MossProvider {
    /// 构造 provider 并用 `info <model>` 探测 GGUF 可加载。探测失败不视为致命，仅 ready=false。
    pub fn spawn(config: &RuntimeConfig, runtime_dir: &Path) -> Result<Self, AsrError> {
        let binary = resolve_path(&config.moss_binary, runtime_dir);
        let model = resolve_path(&config.moss_model, runtime_dir);

        if !binary.is_file() {
            return Err(AsrError::Worker(format!(
                "MOSS 可执行文件不存在: {}",
                binary.display()
            )));
        }
        if !model.is_file() {
            return Err(AsrError::Worker(format!(
                "MOSS 模型不存在: {}",
                model.display()
            )));
        }

        let provider = MossProvider {
            binary,
            model,
            threads: config.moss_threads,
            device: config.moss_device.clone(),
            timeout_minutes: config.moss_timeout_minutes,
            ready: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            last_error: Mutex::new(None),
            active_child: Mutex::new(None),
            dev_debug: config.dev_debug,
        };
        if let Err(e) = provider.probe() {
            provider.set_error(&e.to_string());
        } else {
            provider.ready.store(true, Ordering::SeqCst);
        }
        Ok(provider)
    }

    /// 探测：`info <model>` 成功说明 GGUF 元数据可加载
    fn probe(&self) -> Result<(), AsrError> {
        let output = Command::new(&self.binary)
            .arg("info")
            .arg(&self.model)
            .output()
            .map_err(|e| AsrError::Worker(format!("无法启动 MOSS: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AsrError::Worker(format!(
                "MOSS info 探测失败（{}）：{}",
                output.status,
                stderr.trim()
            )));
        }
        Ok(())
    }

    fn set_error(&self, msg: &str) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(msg.to_string());
        }
    }

    /// 解析 `tasklist /FO CSV /NH` 输出，提取 moss-transcribe 的 PID 列表。
    /// 无进程时 tasklist 输出本地化提示行（非 CSV 格式），不匹配前缀天然跳过。
    /// CSV 行形如：`"moss-transcribe.exe","12345","Console","1","1,234 K"`
    fn parse_tasklist_pids(output: &str) -> Vec<u32> {
        output
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("\"moss-transcribe.exe\""))
            .filter_map(|l| {
                let mut fields = l.split(',');
                fields.next()?; // 映像名
                let pid = fields.next()?.trim_matches('"').parse::<u32>().ok()?;
                Some(pid)
            })
            .collect()
    }

    /// 终止系统上历史残留的 moss-transcribe 孤儿进程（本次子进程尚未
    /// spawn，不会误杀自己）。异常崩溃/被杀软强杀等路径可能绕过退出钩子，
    /// 残留进程会与下次转写叠加抢占 CPU，故每次转写前清理。
    #[cfg(windows)]
    fn cleanup_stale_moss_processes(&self) {
        let output = match Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq moss-transcribe.exe"])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                if self.dev_debug {
                    eprintln!("[moss] tasklist 调用失败，跳过残留清理: {}", e);
                }
                return;
            }
        };
        for pid in Self::parse_tasklist_pids(&String::from_utf8_lossy(&output.stdout)) {
            if self.dev_debug {
                eprintln!("[moss] 清理残留转写进程 PID {}", pid);
            }
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .output();
        }
    }

    /// 非 Windows 平台：无 tasklist/taskkill，跳过清理（当前项目仅 Windows 部署）
    #[cfg(not(windows))]
    fn cleanup_stale_moss_processes(&self) {}

    /// 执行一次转写（阻塞，长音频可达数分钟；由编排层放后台线程）。
    /// 子进程登记在 active_child：轮询等待（可被 cancel 打断）+ 超时终止。
    /// `max_new` 为本次转写的生成 token 上限（调用方按段时长计算）。
    fn run_transcribe(
        &self,
        audio_path: &Path,
        max_new: u32,
    ) -> Result<Vec<AsrSegment>, AsrError> {
        // 防残留叠加：清理历史孤儿进程（本次子进程尚未 spawn，不误杀自己）
        self.cleanup_stale_moss_processes();
        let mut cmd = Command::new(&self.binary);
        cmd.arg("transcribe")
            .arg(&self.model)
            .arg(audio_path)
            // 默认输出是原始流格式，--format json 才会输出结构化段
            .arg("--format")
            .arg("json")
            // 显式放大生成上限：默认 5120 token 会在 ~10 分钟处硬截断
            // （greedy_generate 达 max_new 即停），长音频后半段丢失
            .arg("--max-new")
            .arg(max_new.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.threads > 0 {
            cmd.env("MTD_THREADS", self.threads.to_string());
        }
        // 显式指定推理后端（空 = 不设置，ggml 自动选最优：有 CUDA/Vulkan DLL 即 GPU）
        if !self.device.is_empty() {
            cmd.env("MTD_DEVICE", &self.device);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| AsrError::Worker(format!("无法启动 MOSS 转写: {}", e)))?;

        // 分离管道交给读线程：防止管道缓冲写满阻塞 moss 进程
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = stdout.map(|mut s| {
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                buf
            })
        });
        let err_reader = stderr.map(|mut s| {
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                buf
            })
        });

        {
            let mut slot = self
                .active_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(child);
        }

        // 轮询等待：可被 cancel 打断；timeout_minutes > 0 时超时终止
        let deadline = (self.timeout_minutes > 0)
            .then(|| Instant::now() + Duration::from_secs(self.timeout_minutes as u64 * 60));
        let status = loop {
            if self.cancelled.load(Ordering::SeqCst) {
                // cancel 可能落在子进程登记前的窗口（flag 已置但没 kill），
                // 此处幂等再调一次确保子进程被终止
                self.cancel();
                return Err(AsrError::Worker("MOSS 转写已取消".into()));
            }
            let mut slot = self
                .active_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match slot.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(Some(status)) => {
                        *slot = None;
                        break status;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        *slot = None;
                        return Err(AsrError::Worker(format!("MOSS 进程异常: {}", e)));
                    }
                },
                None => return Err(AsrError::Worker("MOSS 转写已取消".into())),
            }
            drop(slot);
            if let Some(d) = deadline {
                if Instant::now() >= d {
                    self.cancel();
                    return Err(AsrError::Worker(format!(
                        "MOSS 转写超时（{} 分钟），已终止子进程",
                        self.timeout_minutes
                    )));
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        let stdout = out_reader
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = err_reader
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        if !status.success() {
            return Err(AsrError::Worker(format!(
                "MOSS 转写失败（{}）：{}",
                status,
                stderr.trim()
            )));
        }
        parse_segments(&stdout)
    }

    /// 终止当前转写子进程（无活动进程时仅置取消标志，无副作用）。
    /// 供前端"取消 ASR"、超时与应用退出钩子调用。
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = self.active_child.lock() {
            if let Some(child) = slot.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
                *slot = None;
            }
        }
    }
}

impl AsrProvider for MossProvider {
    fn name(&self) -> &str {
        "moss"
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    fn describe(&self) -> String {
        self.last_error.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
    }

    fn transcribe(
        &self,
        audio_path: &Path,
        // MOSS 内置说话人分离与多语言识别，无语言/说话人上限参数，忽略 options
        _options: &AsrTranscribeOptions,
    ) -> Result<Vec<AsrSegment>, AsrError> {
        if !self.is_ready() {
            return Err(AsrError::NotReady);
        }
        // 每次转写重置取消标志：上一次取消不污染本次
        self.cancelled.store(false, Ordering::SeqCst);
        let result = self.transcribe_audio(audio_path);
        match &result {
            Err(e) => self.set_error(&e.to_string()),
            // 成功时清除历史错误：避免取消/失败的残留一直显示在状态探测里
            Ok(_) => {
                if let Ok(mut slot) = self.last_error.lock() {
                    *slot = None;
                }
            }
        }
        result
    }

    fn cancel(&self) {
        MossProvider::cancel(self);
    }
}

impl MossProvider {
    /// 转写入口：检测音频时长，超过阈值时切段逐段转写（规避 prefill OOM
    /// 与 max_new 截断），否则单段直转。分段结果按段偏移合并。
    fn transcribe_audio(&self, audio_path: &Path) -> Result<Vec<AsrSegment>, AsrError> {
        let info = wav_info(audio_path);
        let total_seconds = info.as_ref().map(|i| i.seconds).unwrap_or(0.0);

        // 短音频（或 WAV 头无法解析）：单段直转，max_new 按总时长计算
        if total_seconds <= SEGMENT_SECONDS as f64 {
            let max_new = compute_max_new(total_seconds);
            return self.run_transcribe(audio_path, max_new);
        }

        // 长音频：切段转写 + 偏移合并。每段时长 = SEGMENT_SECONDS，
        // 最后一段为余量。段内时间戳是相对段首的，合并时加段偏移。
        let info = info.ok_or_else(|| {
            AsrError::Worker("MOSS 长音频分段需要可解析的 PCM WAV 头".into())
        })?;
        if self.dev_debug {
            eprintln!("[moss] 音频 {:.0}s 超过分段阈值 {}s，分段转写", total_seconds, SEGMENT_SECONDS);
        }
        let seg_dir = audio_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut all: Vec<AsrSegment> = Vec::new();
        let n_segs = (total_seconds as u32).div_ceil(SEGMENT_SECONDS);
        for seg in 0..n_segs {
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(AsrError::Worker("MOSS 转写已取消".into()));
            }
            let start = seg * SEGMENT_SECONDS;
            let end = ((seg + 1) * SEGMENT_SECONDS).min(total_seconds as u32);
            let seg_wav = seg_dir.join(format!(
                "seg_{:03}_{}_{}.wav",
                seg,
                start,
                std::process::id()
            ));
            slice_wav(audio_path, &info, start, end, &seg_wav)?;
            if self.dev_debug {
                eprintln!("[moss] 分段 {}/{}: [{}-{}s]", seg + 1, n_segs, start, end);
            }
            // 每段 max_new 按段时长计算（短段预算安全）
            let seg_max_new = compute_max_new((end - start) as f64);
            let segs = self.run_transcribe(&seg_wav, seg_max_new)?;
            let _ = std::fs::remove_file(&seg_wav);
            for mut s in segs {
                s.start += start as f64;
                s.end += start as f64;
                all.push(s);
            }
        }
        Ok(all)
    }
}

// ─── 单元测试（纯协议逻辑，不依赖真实 exe/模型）──────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_segments_ok() {
        let json = r#"[
            {"id":"seg_0001","start":0.03,"end":0.45,"speaker":"S01","text":"The road."},
            {"id":"seg_0002","start":0.81,"end":1.35,"speaker":"S02","text":"Gee."}
        ]"#;
        let segs = parse_segments(json).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start, 0.03);
        assert_eq!(segs[0].end, 0.45);
        assert_eq!(segs[0].text, "The road.");
        assert_eq!(segs[0].speaker.as_deref(), Some("S01"));
        assert_eq!(segs[0].confidence, None);
        assert_eq!(segs[1].speaker.as_deref(), Some("S02"));
    }

    #[test]
    fn test_parse_segments_empty_array() {
        let segs = parse_segments("[]").unwrap();
        assert!(segs.is_empty());
    }

    #[test]
    fn test_parse_segments_invalid_json() {
        let result = parse_segments("not json");
        assert!(matches!(result, Err(AsrError::Worker(_))));
    }

    #[test]
    fn test_parse_segments_missing_field_errors() {
        let json = r#"[{"start":0.1,"end":0.5,"speaker":"S01"}]"#;
        assert!(parse_segments(json).is_err());
    }

    #[test]
    fn test_resolve_path() {
        let runtime = Path::new("runtime");
        assert_eq!(
            resolve_path("bin/moss-transcribe.exe", runtime),
            runtime.join("bin/moss-transcribe.exe")
        );
        let abs = if cfg!(windows) {
            "C:\\x\\moss-transcribe.exe"
        } else {
            "/x/moss-transcribe.exe"
        };
        assert_eq!(resolve_path(abs, runtime), PathBuf::from(abs));
    }

    #[test]
    fn test_parse_tasklist_pids_extracts_csv_rows() {
        // 多个残留实例：都应被识别（含内存列带千分位逗号的情况）
        let out = concat!(
            "\"moss-transcribe.exe\",\"12345\",\"Console\",\"1\",\"1,234 K\"\r\n",
            "\"moss-transcribe.exe\",\"67890\",\"Console\",\"1\",\"5,678 K\"\r\n"
        );
        assert_eq!(MossProvider::parse_tasklist_pids(out), vec![12345, 67890]);
    }

    #[test]
    fn test_parse_tasklist_pids_skips_empty_and_foreign() {
        // 无进程时 tasklist 输出本地化提示行（非 CSV），不应误解析出 PID
        let localized = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert!(MossProvider::parse_tasklist_pids(localized).is_empty());
        assert!(MossProvider::parse_tasklist_pids("").is_empty());
        // 其他映像的行不属于 moss，忽略
        let foreign = "\"chrome.exe\",\"999\",\"Console\",\"1\",\"9,999 K\"\r\n";
        assert!(MossProvider::parse_tasklist_pids(foreign).is_empty());
        // 坏 PID 行（理论不会出现）应跳过而非崩溃
        let malformed = "\"moss-transcribe.exe\",\"abc\",\"Console\",\"1\",\"1,234 K\"\r\n";
        assert!(MossProvider::parse_tasklist_pids(malformed).is_empty());
    }

    /// 生成标准 44 字节 PCM WAV：16kHz 单声道 16bit，data 填 pattern。
    /// `insert_list` 为 true 时在 fmt 与 data 之间插入 LIST(INFO) 块
    /// （模拟 ffmpeg 输出，验证块链遍历解析）。
    fn make_test_wav(path: &Path, sample_rate: u32, seconds: u32, insert_list: bool) {
        let channels: u16 = 1;
        let bits: u16 = 16;
        let data_len = (sample_rate * seconds * channels as u32 * 2) as u32;
        let mut buf = Vec::with_capacity(64 + data_len as usize);
        buf.extend_from_slice(b"RIFF");
        // RIFF size = 4(WAVE) + 块链总长
        let list_len = if insert_list { 26u32 } else { 0u32 };
        let list_padded = list_len + (list_len & 1);
        buf.extend_from_slice(&(36 + list_padded + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&bits.to_le_bytes());
        if insert_list {
            // LIST(INFO) 块：26 字节体（ISFT=... 之类），模拟 ffmpeg
            buf.extend_from_slice(b"LIST");
            buf.extend_from_slice(&list_len.to_le_bytes());
            buf.extend_from_slice(b"INFO");
            buf.extend_from_slice(b"ISFT");
            buf.extend_from_slice(&18u32.to_le_bytes());
            buf.extend_from_slice(b"Lavf61.7.100\x00\x00");
        }
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        buf.resize(44 + list_padded as usize + data_len as usize, 0xAB);
        std::fs::write(path, &buf).unwrap();
    }

    #[test]
    fn test_wav_info_parses_duration_and_layout() {
        let dir = std::env::temp_dir().join(format!("gsa_moss_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("t.wav");
        make_test_wav(&wav, 16000, 10, false); // 10s，标准头
        let info = wav_info(&wav).expect("wav_info 应解析成功");
        assert_eq!(info.sample_rate, 16000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.data_offset, 44);
        assert!((info.seconds - 10.0).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wav_info_parses_with_list_chunk() {
        // ffmpeg 输出在 fmt 后插 LIST 块：块链遍历必须跳过它找到 data
        let dir = std::env::temp_dir().join(format!("gsa_moss_wav_list_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("t.wav");
        make_test_wav(&wav, 16000, 20, true); // 20s，含 LIST
        let info = wav_info(&wav).expect("wav_info 应跳过 LIST 解析成功");
        assert_eq!(info.sample_rate, 16000);
        assert!(info.data_offset > 44, "data 应在 LIST 之后，实际 {}", info.data_offset);
        assert!((info.seconds - 20.0).abs() < 0.01, "时长 {}", info.seconds);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_wav_info_rejects_non_wav() {
        let dir = std::env::temp_dir().join(format!("gsa_moss_wav_bad_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("not.wav");
        std::fs::write(&f, b"NOT A WAVE FILE").unwrap();
        assert!(wav_info(&f).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compute_max_new_scales_with_duration() {
        // 短音频不低于默认 5120；长音频按 15 token/s 放大
        assert_eq!(compute_max_new(0.0), DEFAULT_MAX_NEW_TOKENS);
        assert_eq!(compute_max_new(60.0), DEFAULT_MAX_NEW_TOKENS); // 60*15=900 < 5120
        assert_eq!(compute_max_new(480.0), 7200); // 480*15=7200
        assert_eq!(compute_max_new(1200.0), 18000); // 20 分钟
    }

    #[test]
    fn test_slice_wav_produces_valid_subsegment() {
        let dir = std::env::temp_dir().join(format!("gsa_moss_slice_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.wav");
        make_test_wav(&src, 16000, 20, false); // 20s
        let info = wav_info(&src).unwrap();

        // 切 [5, 10)：应得 5s 数据，且能再次解析
        let seg = dir.join("seg.wav");
        slice_wav(&src, &info, 5, 10, &seg).unwrap();
        let info2 = wav_info(&seg).expect("切片应可解析");
        assert!((info2.seconds - 5.0).abs() < 0.01);
        assert_eq!(info2.sample_rate, 16000);
        assert_eq!(info2.channels, 1);
        // 内容验证：data 区应全为填充字节 0xAB（非零，确保不是空切片）
        let bytes = std::fs::read(&seg).unwrap();
        assert_eq!(bytes.len(), 44 + 16000 * 5 * 2);
        assert!(bytes[44..].iter().all(|&b| b == 0xAB));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_slice_wav_clamps_beyond_end() {
        let dir = std::env::temp_dir().join(format!("gsa_moss_slice_end_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.wav");
        make_test_wav(&src, 16000, 10, false);
        let info = wav_info(&src).unwrap();
        // 请求 [8, 999)：应截到 10s
        let seg = dir.join("seg.wav");
        slice_wav(&src, &info, 8, 999, &seg).unwrap();
        let info2 = wav_info(&seg).unwrap();
        assert!((info2.seconds - 2.0).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
