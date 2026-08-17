// ─── 导出模块 ───────────────────────────────────────────
// 单轨字幕导出：SRT / ASS / LRC / TXT
// 前端传整轨数据（始终反映当前编辑状态），后端生成内容并写入目标路径

use crate::project::{TimelineEvent, Track};
use std::fs;

/// 中间表示：从事件提取的统一字幕行（毫秒时间戳 + 文本）
struct SubtitleLine {
    start_ms: i64,
    end_ms: i64,
    text: String,
}

/// 导出格式（前端传小写字符串，如 "srt"）
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleFormat {
    Srt,
    Ass,
    Lrc,
    Txt,
}

/// 从轨道提取可导出的字幕行：
/// - 仅文本事件（OcrText/Asr/Fused/Manual），OcrRegion 无文本跳过
/// - 空文本跳过；end < start 时防御性取 start
/// - 按开始时间升序排序
fn extract_lines(track: &Track) -> Vec<SubtitleLine> {
    let mut lines: Vec<SubtitleLine> = track
        .events
        .iter()
        .filter(|ev| !matches!(ev, TimelineEvent::OcrRegion(_)))
        .map(|ev| {
            let (start, end) = event_times(ev);
            let start_ms = ms(start);
            let text = event_text(ev).trim().to_string();
            SubtitleLine {
                start_ms,
                end_ms: ms(end).max(start_ms),
                text,
            }
        })
        .filter(|l| !l.text.is_empty())
        .collect();
    lines.sort_by_key(|l| l.start_ms);
    lines
}

/// 所有事件变体共享 start/end 字段
fn event_times(ev: &TimelineEvent) -> (f64, f64) {
    match ev {
        TimelineEvent::OcrText(e) => (e.start, e.end),
        TimelineEvent::OcrRegion(e) => (e.start, e.end),
        TimelineEvent::Asr(e) => (e.start, e.end),
        TimelineEvent::Fused(e) => (e.start, e.end),
        TimelineEvent::Manual(e) => (e.start, e.end),
    }
}

/// 事件的导出文本（OcrRegion 无文本 → 空串）
fn event_text(ev: &TimelineEvent) -> &str {
    match ev {
        TimelineEvent::OcrText(e) => &e.text,
        TimelineEvent::OcrRegion(_) => "",
        TimelineEvent::Asr(e) => &e.text,
        TimelineEvent::Fused(e) => &e.text,
        TimelineEvent::Manual(e) => &e.text,
    }
}

/// 秒 → 毫秒（四舍五入）
fn ms(s: f64) -> i64 {
    (s * 1000.0).round() as i64
}

/// SRT 时间戳: HH:MM:SS,mmm
fn srt_ts(ms: i64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// ASS 时间戳: H:MM:SS.cc（厘秒 = 1/100 秒）
fn ass_ts(ms: i64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms / 10 % 100
    )
}

/// LRC 时间戳: [mm:ss.xx]（分钟可超过 60，符合 LRC 规范）
fn lrc_ts(ms: i64) -> String {
    format!("[{:02}:{:02}.{:02}]", ms / 60_000, ms / 1000 % 60, ms / 10 % 100)
}

/// SRT：序号 + 时间范围 + 文本（保留换行）
fn format_srt(lines: &[SubtitleLine]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_ts(l.start_ms),
            srt_ts(l.end_ms),
            l.text
        ));
    }
    out
}

/// ASS：固定默认样式（Microsoft YaHei 40，底部居中），换行转 \N
fn format_ass(lines: &[SubtitleLine]) -> String {
    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str("PlayResX: 1920\n");
    out.push_str("PlayResY: 1080\n");
    out.push_str("ScaledBorderAndShadow: yes\n");
    out.push_str("\n[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    out.push_str("Style: Default,Microsoft YaHei,40,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,2,0,2,30,30,30,1\n");
    out.push_str("\n[Events]\n");
    out.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
    for l in lines {
        let text = l.text.replace('\n', "\\N");
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            ass_ts(l.start_ms),
            ass_ts(l.end_ms),
            text
        ));
    }
    out
}

/// LRC：每个开始时间一行 [mm:ss.xx] 文本，文本内换行合并为空格
fn format_lrc(lines: &[SubtitleLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&format!("{}{}\n", lrc_ts(l.start_ms), l.text.replace('\n', " ")));
    }
    out
}

/// TXT：纯文本逐行（无时间戳）
fn format_txt(lines: &[SubtitleLine]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.text);
        out.push('\n');
    }
    out
}

/// Tauri 命令：导出单轨字幕。返回导出的字幕条数
#[tauri::command]
pub fn export_track_subtitle(
    track: Track,
    format: SubtitleFormat,
    dest_path: String,
) -> Result<usize, String> {
    let lines = extract_lines(&track);
    let content = match format {
        SubtitleFormat::Srt => format_srt(&lines),
        SubtitleFormat::Ass => format_ass(&lines),
        SubtitleFormat::Lrc => format_lrc(&lines),
        SubtitleFormat::Txt => format_txt(&lines),
    };
    fs::write(&dest_path, content).map_err(|e| format!("写入文件失败: {e}"))?;
    Ok(lines.len())
}

// ─── 单元测试（纯逻辑，不写文件）────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AsrEvent, FusedEvent, ManualEvent, OcrRegionEvent, OcrTextEvent};

    fn sample_track() -> Track {
        Track {
            id: "t1".into(),
            name: "角色字幕".into(),
            track_type: "fused".into(),
            track_role: "game".into(),
            preview_visible: true,
            events: vec![
                // 应被过滤：无文本
                TimelineEvent::OcrRegion(OcrRegionEvent {
                    id: "r1".into(),
                    start: 0.0,
                    end: 1.0,
                    x1: 0.0,
                    y1: 0.0,
                    x2: 1.0,
                    y2: 1.0,
                }),
                // 应被过滤：空白文本
                TimelineEvent::Manual(ManualEvent {
                    id: "m1".into(),
                    start: 5.0,
                    end: 6.0,
                    text: "  \t".into(),
                    character: None,
                }),
                // 故意乱序：f2 在 f1 前，排序后应恢复
                TimelineEvent::Fused(FusedEvent {
                    id: "f2".into(),
                    start: 1.5,
                    end: 3.25,
                    text: "第二句".into(),
                    character: Some("角色B".into()),
                }),
                TimelineEvent::Fused(FusedEvent {
                    id: "f1".into(),
                    start: 0.5,
                    end: 1.2,
                    text: "第一句\n多行".into(),
                    character: None,
                }),
            ],
        }
    }

    #[test]
    fn test_extract_lines_filters_and_sorts() {
        let lines = extract_lines(&sample_track());
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 500);
        assert_eq!(lines[0].end_ms, 1200);
        assert_eq!(lines[0].text, "第一句\n多行");
        assert_eq!(lines[1].start_ms, 1500);
        assert_eq!(lines[1].end_ms, 3250);
        assert_eq!(lines[1].text, "第二句");
    }

    #[test]
    fn test_srt_format() {
        let lines = extract_lines(&sample_track());
        let srt = format_srt(&lines);
        assert!(srt.contains("1\n00:00:00,500 --> 00:00:01,200\n第一句\n多行\n\n"));
        assert!(srt.contains("2\n00:00:01,500 --> 00:00:03,250\n第二句\n\n"));
    }

    #[test]
    fn test_ass_format() {
        let lines = extract_lines(&sample_track());
        let ass = format_ass(&lines);
        assert!(ass.starts_with("[Script Info]\n"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("[Events]"));
        assert!(ass.contains("Dialogue: 0,0:00:00.50,0:00:01.20,Default,,0,0,0,,第一句\\N多行\n"));
        assert!(ass.contains("Dialogue: 0,0:00:01.50,0:00:03.25,Default,,0,0,0,,第二句\n"));
    }

    #[test]
    fn test_lrc_format() {
        let lines = extract_lines(&sample_track());
        assert_eq!(format_lrc(&lines), "[00:00.50]第一句 多行\n[00:01.50]第二句\n");
    }

    #[test]
    fn test_txt_format() {
        let lines = extract_lines(&sample_track());
        assert_eq!(format_txt(&lines), "第一句\n多行\n第二句\n");
    }

    #[test]
    fn test_end_before_start_clamped() {
        let track = Track {
            events: vec![TimelineEvent::Asr(AsrEvent {
                id: "a1".into(),
                start: 3.0,
                end: 1.0,
                text: "倒挂".into(),
                speaker: "S01".into(),
                character: None,
                confidence: 0.9,
            })],
            ..sample_track()
        };
        let lines = extract_lines(&track);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_ms, 3000);
        assert_eq!(lines[0].end_ms, 3000);
    }

    #[test]
    fn test_empty_track() {
        let track = Track {
            events: vec![],
            ..sample_track()
        };
        let lines = extract_lines(&track);
        assert!(lines.is_empty());
        assert_eq!(format_srt(&lines), "");
        assert!(format_ass(&lines).contains("[Events]\n"));
        assert_eq!(format_lrc(&lines), "");
        assert_eq!(format_txt(&lines), "");
    }

    #[test]
    fn test_ocr_text_event_extracted() {
        let track = Track {
            events: vec![TimelineEvent::OcrText(OcrTextEvent {
                id: "o1".into(),
                start: 10.0,
                end: 10.9,
                text: "  OCR 台词  ".into(),
                confidence: 0.95,
            })],
            ..sample_track()
        };
        let lines = extract_lines(&track);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "OCR 台词");
        assert_eq!(lines[0].start_ms, 10_000);
        assert_eq!(lines[0].end_ms, 10_900);
    }

    #[test]
    fn test_ms_rounding() {
        // 2.345s → 2345ms；2.3456s → 2346ms（四舍五入）
        assert_eq!(ms(2.345), 2345);
        assert_eq!(ms(2.3456), 2346);
        assert_eq!(ms(1.5), 1500);
        // 负值防御：0.0 → 0
        assert_eq!(ms(0.0), 0);
    }
}
