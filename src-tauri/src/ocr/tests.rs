use std::sync::Arc;

use super::*;
use crate::ai_runtime::dhash::FrameChange;
use crate::ai_runtime::{OcrProvider, OcrResult, OcrError};

/// 按调用顺序消费预置结果的 mock provider。
/// 结果耗尽后返回"短向量"（比请求少），用于触发数量不匹配的错误路径。
struct MockProvider {
    results: Vec<OcrResult>,
    cursor: std::sync::Mutex<usize>,
}

impl OcrProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn recognize_batch(&self, paths: &[String]) -> Result<Vec<OcrResult>, OcrError> {
        let mut cursor = self.cursor.lock().unwrap();
        let remaining = self.results.get(*cursor..).unwrap_or(&[]);
        let count = remaining.len().min(paths.len());
        let out = remaining[..count].to_vec();
        *cursor += count;
        Ok(out)
    }
}

fn mock(results: Vec<OcrResult>) -> MockProvider {
    MockProvider {
        results,
        cursor: std::sync::Mutex::new(0),
    }
}

fn frame(time: f64, changed: bool) -> FrameChange {
    FrameChange {
        time,
        sample_time: time,
        is_changed: changed,
        base_hash: None,
    }
}

fn ft(time: f64, text: &str, confidence: f64) -> FrameText {
    FrameText {
        time,
        sample_time: time,
        text: Arc::from(text),
        confidence,
    }
}

// ── ocr_pass ──

#[test]
fn test_ocr_pass_carries_forward() {
    let provider = mock(vec![
        OcrResult { text: "A".into(), confidence: 0.9 },
        OcrResult { text: "B".into(), confidence: 0.8 },
    ]);
    let changes = vec![frame(0.0, true), frame(1.0, false), frame(2.0, true), frame(3.0, false)];
    let jpegs = vec![b"J1".to_vec(), b"J2".to_vec()];
    let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {}).unwrap();
    assert_eq!(texts.len(), 4);
    assert_eq!(texts[0].text.as_ref(), "A");
    assert_eq!(texts[1].text.as_ref(), "A"); // 顺延
    assert_eq!(texts[2].text.as_ref(), "B");
    assert_eq!(texts[3].text.as_ref(), "B"); // 顺延
    assert!((texts[0].confidence - 0.9).abs() < 1e-9);
}

#[test]
fn test_ocr_pass_trims_text() {
    let provider = mock(vec![OcrResult { text: "  A  ".into(), confidence: 0.9 }]);
    let changes = vec![frame(0.0, true)];
    let jpegs = vec![b"J".to_vec()];
    let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {}).unwrap();
    assert_eq!(texts[0].text.as_ref(), "A"); // 已 trim
}

#[test]
fn test_ocr_pass_empty_change_resets() {
    let provider = mock(vec![
        OcrResult { text: "A".into(), confidence: 0.9 },
        OcrResult { text: String::new(), confidence: 0.0 },
    ]);
    let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, false)];
    let jpegs = vec![b"J1".to_vec(), b"J2".to_vec()];
    let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 1, |_, _| {}).unwrap();
    assert_eq!(texts[0].text.as_ref(), "A");
    assert_eq!(texts[1].text.as_ref(), ""); // 变化帧识别为空 → 清空
    assert_eq!(texts[2].text.as_ref(), ""); // 顺延空
}

#[test]
fn test_ocr_pass_batch_mixed_changed() {
    let provider = mock(vec![OcrResult { text: "X".into(), confidence: 0.7 }]);
    let changes = vec![frame(0.0, false), frame(1.0, true)];
    let jpegs = vec![b"J".to_vec()];
    let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 2, |_, _| {}).unwrap();
    assert_eq!(texts[0].text.as_ref(), ""); // 首帧未变化且无 previous → 空
    assert_eq!(texts[1].text.as_ref(), "X");
}

#[test]
fn test_ocr_pass_result_mismatch_errors() {
    // 预置结果比变化帧少 → 应返回错误而非静默空文本；
    // 图像数据与变化帧数量不符 → 在调用 recognize 前即报错
    let provider = mock(vec![OcrResult { text: "A".into(), confidence: 0.9 }]);
    let changes = vec![frame(0.0, true), frame(1.0, true), frame(2.0, true)];
    let jpegs = vec![b"J".to_vec()];
    assert!(ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 1, |_, _| {}).is_err());
}

// ── assemble_grid_frames（幻影网格帧回收）──

/// 8 个网格标志：k=0/3/7 为变化帧。模拟 ceil 估算 8 帧、mjpeg 流实际 7 帧。
fn phantom_flags() -> Vec<(bool, Option<u64>)> {
    let mut flags = vec![(false, None); 8];
    flags[0] = (true, None);
    flags[3] = (true, Some(0xA));
    flags[7] = (true, Some(0xB));
    flags
}

#[test]
fn test_assemble_grid_phantom_changed_recovered() {
    let flags = phantom_flags();
    // 实际 7 帧，kept 只含 <7 的变化帧字节（k=0、k=3）
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorded = calls.clone();
    let mut recover = move |t: f64| {
        recorded.borrow_mut().push(t);
        Some(b"J7".to_vec())
    };
    let (changes, jpegs) =
        assemble_grid_frames(&flags, vec![b"J0".to_vec(), b"J3".to_vec()], 7, 10.0, 0.7, &mut recover)
            .unwrap();
    // 7 个实际帧 + 1 个回收的幻影帧
    assert_eq!(changes.len(), 8);
    assert_eq!(jpegs.len(), 3);
    // changes/jpegs 与变化帧按序对齐：k=0、k=3（实际）、k=7（幻影）
    let changed: Vec<usize> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_changed)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(changed, vec![0, 3, 7]);
    assert_eq!(changes[7].time, 10.0 + 7.0 * 0.7);
    assert_eq!(changes[7].base_hash, Some(0xB));
    assert_eq!(jpegs[2], b"J7".to_vec());
    // 幻影帧按网格时间补抽，且只补抽一次
    assert_eq!(calls.borrow().clone(), vec![10.0 + 7.0 * 0.7]);
}

#[test]
fn test_assemble_grid_phantom_unchanged_skipped() {
    let mut flags = phantom_flags();
    flags[7] = (false, None); // 幻影帧未变化 → 无需补抽
    let (changes, jpegs) = assemble_grid_frames(
        &flags,
        vec![b"J0".to_vec(), b"J3".to_vec()],
        7,
        0.0,
        0.7,
        |_| panic!("未变化帧不应触发补抽"),
    )
    .unwrap();
    assert_eq!(changes.len(), 7);
    assert_eq!(jpegs.len(), 2);
}

#[test]
fn test_assemble_grid_recovery_failure_keeps_alignment() {
    let flags = phantom_flags();
    // 补抽失败 → 幻影帧的 change 与 jpeg 同步跳过，对齐不被破坏
    let (changes, jpegs) = assemble_grid_frames(
        &flags,
        vec![b"J0".to_vec(), b"J3".to_vec()],
        7,
        0.0,
        0.7,
        |_| None,
    )
    .unwrap();
    assert_eq!(changes.len(), 7);
    let changed: Vec<usize> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_changed)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(changed, vec![0, 3]);
    assert_eq!(jpegs.len(), 2);
}

#[test]
fn test_assemble_grid_total_matches_no_recovery() {
    let flags = phantom_flags();
    // 估算与实际一致（total=8）→ 无幻影帧，不触发补抽
    let (changes, jpegs) = assemble_grid_frames(
        &flags,
        vec![b"J0".to_vec(), b"J3".to_vec(), b"J7".to_vec()],
        8,
        0.0,
        0.7,
        |_| panic!("total 覆盖全部网格帧时不应触发补抽"),
    )
    .unwrap();
    assert_eq!(changes.len(), 8);
    assert_eq!(jpegs.len(), 3);
}

#[test]
fn test_assemble_grid_mismatch_errors() {
    let flags = phantom_flags();
    // kept 数量与变化帧不符 → 显式报错而非静默错位
    assert!(assemble_grid_frames(&flags, vec![b"J0".to_vec()], 7, 0.0, 0.7, |_| None).is_err());
}

#[test]
fn test_assemble_grid_output_feeds_ocr_pass() {
    // 装配结果（含回收的幻影帧）满足 ocr_pass 契约：数量对齐、文本按序回填
    let flags = phantom_flags();
    let (changes, jpegs) = assemble_grid_frames(
        &flags,
        vec![b"J0".to_vec(), b"J3".to_vec()],
        7,
        0.0,
        0.7,
        |_| Some(b"J7".to_vec()),
    )
    .unwrap();
    let provider = mock(vec![
        OcrResult { text: "A".into(), confidence: 0.9 },
        OcrResult { text: "B".into(), confidence: 0.8 },
        OcrResult { text: "C".into(), confidence: 0.7 },
    ]);
    let texts = ocr_pass(&changes, &jpegs, |images| provider.recognize_batch(images), 16, |_, _| {})
        .unwrap();
    assert_eq!(texts.len(), 8);
    assert_eq!(texts[0].text.as_ref(), "A");
    assert_eq!(texts[3].text.as_ref(), "B");
    assert_eq!(texts[7].text.as_ref(), "C"); // 幻影帧文本正常回填
}

// ── merge_frames ──

#[test]
fn test_merge_consecutive_same() {
    let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "A", 0.9), ft(3.0, "A", 0.9)];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert!((segs[0].start - 1.0).abs() < 1e-9);
    assert!((segs[0].end - 3.5).abs() < 1e-9); // 3.0 + 0.5×1.0（覆盖中点）
    assert_eq!(segs[0].text, "A");
}

#[test]
fn test_merge_empty_breaks_and_skips() {
    let frames = vec![
        ft(0.0, "", 0.0),
        ft(1.0, "A", 0.9),
        ft(2.0, "A", 0.9),
        ft(3.0, "", 0.0),
    ];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert!((segs[0].start - 1.0).abs() < 1e-9);
    assert!((segs[0].end - 2.5).abs() < 1e-9); // 2.0 + 0.5
}

#[test]
fn test_merge_text_change_split() {
    let frames = vec![ft(1.0, "A", 0.9), ft(2.0, "B", 0.8), ft(3.0, "B", 0.8)];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 2);
    assert!((segs[0].start - 1.0).abs() < 1e-9);
    assert!((segs[0].end - 1.5).abs() < 1e-9); // 1.0 + 0.5
    assert!((segs[1].start - 2.0).abs() < 1e-9);
    assert!((segs[1].end - 3.5).abs() < 1e-9); // 3.0 + 0.5
}

#[test]
fn test_merge_clamp_to_clip_end() {
    let frames = vec![ft(4.0, "B", 0.8), ft(5.0, "B", 0.8)];
    let segs = merge_frames(frames, 1.0, 5.5, 0.3);
    assert_eq!(segs.len(), 1);
    assert!((segs[0].start - 4.0).abs() < 1e-9);
    assert!((segs[0].end - 5.5).abs() < 1e-9); // min(5+0.5, 5.5)
}

#[test]
fn test_merge_skips_run_beyond_clip() {
    // 段起始已越过 clip_end → 丢弃
    let frames = vec![ft(6.0, "B", 0.8), ft(7.0, "B", 0.8)];
    let segs = merge_frames(frames, 1.0, 5.5, 0.3);
    assert!(segs.is_empty());
}

// ── 相似度合并 ──

#[test]
fn test_edit_distance_ratio() {
    assert!((edit_distance_ratio("", "") - 0.0).abs() < 1e-9);
    assert!((edit_distance_ratio("abc", "abc") - 0.0).abs() < 1e-9);
    assert!((edit_distance_ratio("abc", "abd") - 1.0 / 3.0).abs() < 1e-9);
    assert!((edit_distance_ratio("abc", "xyz") - 1.0).abs() < 1e-9);
}

#[test]
fn test_merge_similar_prefix_extends() {
    // 打字机/渐进文本：后续帧是前缀的超集 → 合并为一条，保留最长文本
    let frames = vec![
        ft(1.0, "旅行者，你来了", 0.9),
        ft(2.0, "旅行者，你来了。前方似乎有东西在等待。", 0.9),
        ft(3.0, "旅行者，你来了。前方似乎有东西在等待。", 0.9),
    ];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "旅行者，你来了。前方似乎有东西在等待。");
    assert!((segs[0].end - 3.5).abs() < 1e-9); // 3.0 + 0.5
}

#[test]
fn test_merge_similar_rejects_namebox_line_boundary() {
    // D11 回归：姓名框/头衔态**稳定 ≥2.0s** 后，高置信度帧给出下一条对话行 → 两条独立字幕。
    // 实测形态（glupov 嵌字 #18）：`Anton / Former Acting Captain,"Ninth Company"`
    // ≈40 字符 vs 含对话行的 ≈90 字符，占比 0.44 > 1/3 会顶破 3× 前缀规则
    let namebox = "Anton\nFormer Acting Captain,\"Ninth Company";
    let full = "Anton\nFormer Acting Captain,\"Ninth Company\nNo news. But perhaps... no news is the best news.";
    let frames = vec![
        ft(1.0, namebox, 0.97),
        ft(2.0, namebox, 0.97),
        ft(2.8, namebox, 0.97),
        ft(3.0, full, 0.96),
        ft(4.0, full, 0.96),
    ];
    let segs = merge_frames(frames, 0.5, 30.0, 0.3);
    assert_eq!(segs.len(), 2, "稳定姓名框态与对话行应是两条独立字幕");
    assert_eq!(segs[0].text, namebox);
    assert_eq!(segs[1].text, full);
}

#[test]
fn test_run_text_growing_net_growth_only() {
    // 判据边界（真实素材实测的归一化长度序列）：
    // - pierro 打字机链 23→38→55→65→65（对话行仍在逐字补全）→ 净增长
    // - glupov #17 姓名框态 36×8（逐帧同形，OCR 省掉 conf=0.000 的「……」行）→ 未增长
    let run = |lens: &[usize]| -> Vec<(Arc<str>, f64)> {
        lens.iter()
            .map(|&n| (Arc::<str>::from("x".repeat(n)), 0.96))
            .collect()
    };
    assert!(run_text_growing(&run(&[23, 38, 55, 65, 65])));
    assert!(!run_text_growing(&run(&[36, 36, 36, 36, 36, 36, 36, 36])));
    // 抖动方向保守：首帧更长（乱码多认了字）与单帧 run 都不算增长，门槛照旧生效
    assert!(!run_text_growing(&run(&[40, 36, 36, 36])));
    assert!(!run_text_growing(&run(&[36])));
}

#[test]
fn test_merge_frames_merges_growing_typewriter_line_boundary() {
    // D12 收口回归：**打字机中途**采到的短态不是稳定姓名框态。run 内文本仍在逐帧增长
    // （实测 pierro 嵌字归一化长度 23→38→55→65→65）时，整段出现不得切断 run——
    // 否则同一条字幕产出两段（实测 `[1167.77, 1170.67]` + `[1170.67, 1187.28]`，
    // conf 0.982/0.954，下游跨语言对齐因此整体错位一位）。
    // 本例短态已稳定 3.4s（> LINE_BOUNDARY_STABLE_SEC 2.0s）且 conf 0.95（≥ 0.8），
    // 时长门+置信度门+行边界切断三者原本全部成立。
    let partial = "Paimon\nof the Gnoses, Paimon's been dying to ask this question since forever. Why";
    let full = "Paimon\nof the Gnoses, Paimon's been dying to ask this question since forever. Why\ns the Tsaritsa wanna collect them? Is it something to do with the fight against\nthe Heavenly Principles?";
    let frames = vec![
        ft(0.0, "Paimon\nof the Gnoses, Paimon's", 0.97),
        ft(1.0, "Paimon\nof the Gnoses, Paimon's been dying to ask this question since forever.", 0.98),
        ft(2.5, partial, 0.98),
        ft(3.4, full, 0.95),
        ft(4.4, full, 0.95),
    ];
    let segs = merge_frames(frames, 0.5, 30.0, 0.3);
    assert_eq!(segs.len(), 1, "打字机链内的短态不得被切分成独立新字幕");
    assert_eq!(segs[0].text, full);
}

#[test]
fn test_merge_frames_keeps_stable_namebox_run() {
    // 反向守卫（D12 正当用例，glupov #17 实测形态）：姓名框/头衔态**逐帧同形**稳定 4.0s
    // 后出现对话行 —— run 内无净增长，门槛必须照旧判独立（参考亦记为两条）。
    // 本用例锁死"不得为了让打字机对合并而整体移除/无条件压掉这道门槛"——
    // D12 的 A2 全量实测：门槛若被关掉，glupov 72.2 → 54.8（1:1 22→19、被吞并 0→3）。
    let namebox = "Anton\nFormer Acting Captain,\"Ninth Company";
    let full = "Anton\nFormer Acting Captain,\"Ninth Company\nNo news. But perhaps... no news is the best news.";
    let mut frames: Vec<FrameText> = (0..8).map(|k| ft(k as f64 * 0.5, namebox, 0.96)).collect();
    frames.push(ft(4.0, full, 0.93));
    frames.push(ft(5.0, full, 0.94));
    let segs = merge_frames(frames, 0.5, 30.0, 0.3);
    assert_eq!(segs.len(), 2, "稳定姓名框态（run 内无增长）仍应是独立字幕");
    assert_eq!(segs[0].text, namebox);
    assert_eq!(segs[1].text, full);
}

#[test]
fn test_merge_frames_keeps_ellipsis_short_state() {
    // 反向守卫（用户 2026-10 明确的第二对，pierro 嵌字 「丑角」）：短态 `The Jester / …`
    // 只活 1.50s，且语料里 17/18 两条是**独立条目**（真值单调，不该合并）。
    // 两道保险各自独立成立：① 时长 1.50s < 2.0s；② 「……」行归一化为空串，其与长态
    // 对应行距离比 1.0 > LINE_PREFIX_EDIT_TOLERANCE，`is_line_boundary_cut` 直接否决。
    // 本用例锁死"不得为了修打字机对而把这类省略号短态并掉"。
    let short = "The Jester\n…";
    let long = "The Jester\nour judgement. We shall put the project on hold... until the Ruler of Death has\nbeen eliminated.";
    let frames = vec![
        ft(0.0, short, 0.95),
        ft(1.0, short, 0.95),
        ft(1.5, long, 0.94),
        ft(2.5, long, 0.94),
    ];
    let segs = merge_frames(frames, 0.5, 30.0, 0.3);
    assert_eq!(segs.len(), 2, "省略号短态是独立条目，不得并入后继对话行");
    assert_eq!(segs[0].text, short);
    assert_eq!(segs[1].text, long);
}

#[test]
fn test_merge_similar_keeps_wrapped_tail_line() {
    // 反向守卫：多出的行是**换行余尾**（远短于短态末行）→ 不判行边界切断，保持同一条目。
    // 实测形态（moon 嵌字 #16）：`runaway princess back to the` → 补出 `moon.`（4 字符）
    let base = "Aria\nlike the villain in a human novel who tries to bring the runaway princess back to the";
    let wrapped = "Aria\nlike the villain in a human novel who tries to bring the runaway princess back to the\nmoon.";
    let frames = vec![
        ft(1.0, base, 0.95),
        ft(3.0, base, 0.95),
        ft(5.0, base, 0.95),
        ft(5.5, wrapped, 0.95),
        ft(6.5, wrapped, 0.95),
    ];
    let segs = merge_frames(frames, 0.5, 30.0, 0.3);
    assert_eq!(segs.len(), 1, "换行余尾应保持同一条目");
    assert_eq!(segs[0].text, wrapped);
}

#[test]
fn test_merge_similar_ignores_low_conf_extra_line() {
    // 置信度门：低置信度帧多出的一行是识别抖动，不触发行边界切断
    // （实测 moon：一帧 conf=0.64 的乱码把已稳定 12.7s 的段落顶开）
    let base = "Sonnet\nthe big deal! We can just leave the work to those Moon Envoys";
    let garbled = "Sonnet\nthe big deal! We can just leave the work to those Moon Envoys\nr";
    let frames = vec![
        ft(1.0, base, 0.97),
        ft(2.0, base, 0.97),
        ft(4.0, base, 0.97),
        ft(4.5, garbled, 0.64),
    ];
    let segs = merge_frames(frames, 0.5, 30.0, 0.3);
    assert_eq!(segs.len(), 1, "低置信度多行不应拆段");
}

#[test]
fn test_merge_similar_keeps_transient_line_boundary() {
    // 反向守卫：同形态但短态**只持续 0.47s**（打字机换行续写）→ 仍属同一条字幕。
    // 实测形态（glupov 语料）：`…第九连队的编制保住`（3 行）→ 补出换行第二行（4 行）
    let partial = "安东\n原「第九连队」临时连长\n层岩巨渊的经历让他们吃了不少苦头，不过万幸，第九连队的编制保住";
    let wrapped = "安东\n原「第九连队」临时连长\n层岩巨渊的经历让他们吃了不少苦头，不过万幸，第九连队的编制保住\n了。我们没有让祖先与陛下蒙羞。";
    let frames = vec![
        ft(1.00, partial, 0.92),
        ft(1.25, partial, 0.93),
        ft(1.50, wrapped, 0.91),
        ft(1.75, wrapped, 0.92),
    ];
    let segs = merge_frames(frames, 0.25, 30.0, 0.3);
    assert_eq!(segs.len(), 1, "打字机换行续写应保持同一条");
    assert_eq!(segs[0].text, wrapped);
}

#[test]
fn test_merge_similar_keeps_partial_last_line() {
    // 末行只写了一半（严格前缀）→ 仍是同一句的渐进补全，应合并为一条
    let frames = vec![
        ft(1.0, "派蒙\n前方似乎有", 0.9),
        ft(2.0, "派蒙\n前方似乎有什么东西在等待。", 0.9),
    ];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "派蒙\n前方似乎有什么东西在等待。");
}

#[test]
fn test_merge_similar_noise_keeps_longest() {
    // OCR 抖动：同一句少一字 → 相似合并，保留更长
    let frames = vec![
        ft(1.0, "前方似乎有什么东西在等待", 0.9),
        ft(2.0, "前方似乎有什么东西在等待。", 0.9),
    ];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "前方似乎有什么东西在等待。");
}

#[test]
fn test_merge_dissimilar_splits() {
    // 完全不同 → 分两段
    let frames = vec![ft(1.0, "甲", 0.9), ft(2.0, "乙", 0.8)];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 2);
}

// ── 多数投票 ──

#[test]
fn test_vote_ignores_noise_long_text() {
    // 噪声帧更长但与多数帧差异大 → 投票选多数一致文本，不被更长噪声污染
    let frames = vec![
        ft(1.0, "前方似乎有什么东西在等待。", 0.9),
        ft(2.0, "前方似乎有什么东西在等待。", 0.9),
        ft(3.0, "前方似乎有什么东西在等待。前方似乎有什么东X乱码Z", 0.6),
    ];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "前方似乎有什么东西在等待。");
}

#[test]
fn test_vote_prefers_progressive_completion() {
    // D10 回归：复刻 glupov 语料实测段（事件 [8.41 → 10.87]）——run 内每个网格帧都
    // 入表、未变化帧顺延上一文本，故三态权重为 2/4/4；medoid 在此形状下落在**中间态**
    // P2（残缺），而完整态 P3 就在同一 run 内、被 4 帧承载且置信度更高（实测 0.86 vs 0.80）
    let p1 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安";
    let p2 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安排我主管一支连队的所有事务，任务";
    let p3 = "斯捷潘尼扬\n「编玛瑙」\n嗯，最近上头安排我主管一支连队的所有事务，任务瞬间复杂了起来。";
    let mut frames = Vec::new();
    let mut t = 1.0;
    for (text, n, cf) in [(p1, 2, 0.83), (p2, 4, 0.80), (p3, 4, 0.86)] {
        for _ in 0..n {
            frames.push(ft(t, text, cf));
            t += 0.25;
        }
    }
    let segs = merge_frames(frames, 0.25, 30.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, p3);
}

#[test]
fn test_vote_progressive_completion_needs_support() {
    // 完整态只被 1 帧承载**且不是 run 末态**（其后还有残缺态帧）→ 不启用渐进偏好，
    // 输出仍是 medoid 胜者（中间态）。守门理由：单帧的"更长"不可信
    let p1 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安";
    let p2 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安排我主管一支连队的所有事务，任务";
    let p3 = "斯捷潘尼扬\n「编玛瑙」\n嗯，最近上头安排我主管一支连队的所有事务，任务瞬间复杂了起来。";
    let mut frames = Vec::new();
    let mut t = 1.0;
    for (text, n, cf) in [(p1, 2, 0.83), (p2, 4, 0.80), (p3, 1, 0.86), (p2, 2, 0.80)] {
        for _ in 0..n {
            frames.push(ft(t, text, cf));
            t += 0.25;
        }
    }
    let segs = merge_frames(frames, 0.25, 30.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, p2);
}

#[test]
fn test_vote_accepts_final_state_completion() {
    // 完整态只被 1 帧承载但**就是 run 末态**（实测 glupov 语料 `…能力显得更关键。`：
    // 尾部亚阈值变化漏检，仅 1.5s 保险丝在段尾补采到一帧，conf 0.86 > 残缺态 0.85）
    let p2 = "斯捷潘尼扬\n「编玛瑙]\n战斗技巧退居到了次要位置，全局意识和任务排期的能";
    let p3 = "斯捷潘尼扬\n「编玛瑙】\n战斗技巧退居到了次要位置，全局意识和任务排期的能力显得更关键。";
    let frames = vec![
        ft(1.00, "斯捷潘尼扬\n「编玛瑙]\n战斗技巧退居到", 0.81),
        ft(1.25, "斯捷潘尼扬\n「编玛瑙]\n战斗技巧退居到了次要位置，全局意", 0.82),
        ft(1.50, p2, 0.85),
        ft(1.75, p2, 0.85),
        ft(4.00, p3, 0.86),
    ];
    let segs = merge_frames(frames, 0.25, 30.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, p3);
}

#[test]
fn test_vote_rejects_low_confidence_completion() {
    // 低置信度"更长候选"被拒（moon 语料实测：`卡侬 / oo / 桑娜妲。…` 承载帧
    // conf 0.65/0.70，而干净态 0.85）——支持度 ≥2 也要过置信度门。
    //
    // ⚠ A5 复核（2026-09-29）：候选的**归一化增量必须 ≥3**（`MIN_GAIN`），否则会先被
    // 上一道门挡掉，本用例就测不到置信度门——原写法 `oo` 增量恰为 2，属**恒真用例**。
    // 现改为 `oOo`（增量 3）：过关卡到置信度门后才被拒（把 MARGIN 临时放大到 1.0
    // 会让本用例变红，即它确实在测这道门）。
    let clean = "卡侬\n桑娜妲。仔细想想，最近这些年，你丢下工作，偷偷跑出去找人类玩的次数，好像越来越多了。";
    let tailed = "卡侬\noOo\n桑娜妲。仔细想想，最近这些年，你丢下工作，偷偷跑出去找人类玩的次数，好像越来越多了。";
    let frames = vec![
        ft(1.0, clean, 0.98),
        ft(1.25, clean, 0.85),
        ft(1.5, tailed, 0.65),
        ft(1.75, tailed, 0.70),
        ft(2.0, clean, 0.85),
    ];
    let segs = merge_frames(frames, 0.25, 30.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, clean);
}

#[test]
fn test_vote_progressive_completion_at_0_5s_weights() {
    // A5 语境：网格回退 **0.5s**（2026-09-19 `1f05e7c`）后，run 内权重形状从 0.25s 的
    // 2/4/4 缩短为 **1/2/2**。本用例把该形状下的**两臂**都钉住：
    //   关偏好 → medoid 落在中间态 P2（残缺）；开偏好 → 取到 run 末态 P3（完整）。
    // 即"该偏好在当前默认网格下确实会改变文本"有单测级证据；基准层面的受控 A/B 见 A5-P2。
    let p1 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安";
    let p2 = "斯捷潘尼扬\n「编玛瑙]\n嗯，最近上头安排我主管一支连队的所有事务，任务";
    let p3 = "斯捷潘尼扬\n「编玛瑙」\n嗯，最近上头安排我主管一支连队的所有事务，任务瞬间复杂了起来。";
    let texts: Vec<(Arc<str>, f64)> = vec![
        (Arc::from(p1), 0.83),
        (Arc::from(p2), 0.80),
        (Arc::from(p2), 0.80),
        (Arc::from(p3), 0.86),
        (Arc::from(p3), 0.86),
    ];
    assert_eq!(
        vote_text_with(&texts, false).0.as_ref(),
        p2,
        "关偏好时 medoid 落在中间态（1/2/2 权重）"
    );
    assert_eq!(vote_text_with(&texts, true).0.as_ref(), p3, "开偏好时取完整态");
}

#[test]
fn test_vote_progressive_min_gain_guards_noise_variant() {
    // 真实素材（glupov 语料 ev13：`原「第九连队」临时连长` 被 OCR 读成
    // `原「第九连队J临时连长`）：噪声变体只多 **1** 个归一化字符，落在 `MIN_GAIN=3` 之下
    // → 即使它是 run 末态、且置信度更高（0.95 vs 0.85）也不得翻案。
    // 依据：`temp/probe/attr_05_corpus.log` 实测——去掉 `MIN_GAIN` 后语料 97.1→96.9，
    // 差异恰为这一条。
    //
    // 两条断言一起说明"是哪道门在起作用"：增量 1 被挡；把增量加到 3 后就**真会被选中**
    // （说明支持度/末态/置信度三门都放行，唯独 MIN_GAIN 守住）——把 `MIN_GAIN` 临时
    // 改为 0 会让第一条断言变红。
    let clean = "斯捷潘尼扬\n「缟玛瑙」\n原「第九连队临时连长";
    let noise1 = "斯捷潘尼扬\n「缟玛瑙」\n原「第九连队J临时连长";
    let noise3 = "斯捷潘尼扬\n「缟玛瑙」\n原「第九连队JKL临时连长";
    let mk = |noisy: &str| -> Vec<(Arc<str>, f64)> {
        vec![
            (Arc::from(clean), 0.85),
            (Arc::from(clean), 0.85),
            (Arc::from(clean), 0.85),
            (Arc::from(noisy), 0.95),
        ]
    };
    assert_eq!(
        vote_text_with(&mk(noise1), true).0.as_ref(),
        clean,
        "增量 1 的噪声变体应被 MIN_GAIN 挡住"
    );
    assert_eq!(
        vote_text_with(&mk(noise3), true).0.as_ref(),
        noise3,
        "增量 ≥3 时 MIN_GAIN 不再挡 → 证明上一条确实由它守住"
    );
}

#[test]
fn test_progressive_complete_switch_parsing() {
    // A5 的受控 A/B 开关语义：只有显式的 0/false/off/no（大小写、首尾空白无关）才关闭；
    // 其余（含未设置、空串、无法识别的值）一律保持开启——避免"拼错变量名就静默关掉偏好"。
    for off in ["0", "false", "FALSE", "off", "Off", " no "] {
        assert!(!progressive_complete_enabled_from(Some(off)), "应关闭：{off}");
    }
    for on in [None, Some(""), Some("1"), Some("true"), Some("yes"), Some("whatever")] {
        assert!(progressive_complete_enabled_from(on), "应开启：{on:?}");
    }
}

// ── 短碎片激进合并 ──

#[test]
fn test_short_fragment_merges_across_gap() {
    // 实测形态（pierro 4→5 / 21→22）：碎片 0.4s、与其后一条间隔 1.2s（> 既有 0.5s 邻接门），
    // 末行与后条对应行弱关联（少一个空格）→ 并入后条，保留碎片起点
    let segs = vec![
        OcrSegment {
            start: 10.0,
            end: 10.4,
            text: "Stepanyan\nOnyx Agate\nHm? Oh, it's yo".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 11.6,
            end: 16.0,
            text: "Stepanyan\nOnyx Agate\nHm? Oh, it's you... Glad to meet you again.".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 1);
    assert!((out[0].start - 10.0).abs() < 1e-9, "保留碎片起点");
    assert!((out[0].end - 16.0).abs() < 1e-9, "保留后条终点");
    assert!(out[0].text.contains("Glad to meet you"));
}

#[test]
fn test_short_fragment_keeps_unrelated() {
    // 碎片与其后一条**无关联**（不同句子）→ 不并（保护真实短条不被吞掉）
    let segs = vec![
        OcrSegment { start: 10.0, end: 10.4, text: "派蒙\n嗯".into(), confidence: 0.9 },
        OcrSegment {
            start: 10.6,
            end: 14.0,
            text: "旅行者\n前方似乎有什么东西在等待。".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 2);
}

#[test]
fn test_short_fragment_keeps_ellipsis_entry() {
    // D9 护栏：末行归一化后为空（纯省略号条）→ 绝不并入下一条，
    // 否则语料出现"缺失"（glupov 语料 [……] 条即此类）
    let segs = vec![
        OcrSegment {
            start: 531.0,
            end: 531.4,
            text: "安东\n原「第九连队」临时连长\n……".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 534.0,
            end: 545.0,
            text: "安东\n原「第九连队」临时连长\n没有消息。但也许……没有消息就是最好的消息。".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 2, "纯省略号条必须保持独立");
}

#[test]
fn test_short_fragment_merges_trailing_garbage_line() {
    // P1（2026-09-28）：碎片末行是 OCR 把界面数字读成的单字符行（`0`）时，比较行应回退到
    // **最后一个长度 ≥2 的行**。实测 pierro SRT #107 `MurderofBirds`/`0`（0.484s）此前
    // 因此无法被吸收；导入 Premiere 时该 `0` 还被解析器当成字幕序号，导致其后全部被截断。
    let segs = vec![
        OcrSegment {
            start: 2570.585,
            end: 2571.069,
            text: "MurderofBirds\n0".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 2571.069,
            end: 2590.188,
            text: "MurderofBirds\nfor telling me so much. I'm a bit surprised, honestly.".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 1, "尾部单字符垃圾行不应阻止吸收前缀碎片");
    assert!((out[0].start - 2570.585).abs() < 1e-9, "保留碎片起点");
    assert!(out[0].text.contains("for telling me so much"));
}

#[test]
fn test_short_fragment_merges_trailing_garbage_after_partial_line() {
    // 同型第二例（pierro SRT #125）：`Paimon`/`Huh?`/`0`（0.383s）→ 比较行回退到 `Huh?`，
    // 它与后条对应行弱关联（子序列）→ 并入后条
    let segs = vec![
        OcrSegment {
            start: 2894.342,
            end: 2894.725,
            text: "Paimon\nHuh?\n0".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 2894.725,
            end: 2945.993,
            text: "Paimon\nHuh? Oh, hey there, Odette!".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 1, "比较行回退后与后条弱关联即应吸收");
}

#[test]
fn test_short_fragment_keeps_all_garbage_lines() {
    // 碎片全为单字符行时无从比较 → 保持独立（glupov SRT #24 单行 `A` 即此类）；
    // 放宽比较行的选择**不等于**放开护栏，仍需存在长度 ≥2 的行
    let segs = vec![
        OcrSegment { start: 584.243, end: 584.627, text: "A".into(), confidence: 0.9 },
        OcrSegment {
            start: 586.0,
            end: 590.0,
            text: "派蒙\n这里是哪里？".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 2);
}

#[test]
fn test_short_fragment_long_entry_with_short_tail_keeps() {
    // 真实短句尾（moon 语料 #3：末行 `了。` 归一化长度 1）仍不受影响——P1 只放宽"比较行"
    // 的选择，**不放开时长门**：9.57s 的条目远超 min_subtitle_sec，保持独立
    let segs = vec![
        OcrSegment {
            start: 100.0,
            end: 109.567,
            text: "卡侬\n桑娜妲。仔细想想，最近这些年，你丢下工作，偷偷跑出去找人类玩的次数\n了。"
                .into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 109.567,
            end: 120.0,
            text: "卡侬\n那你可得好好补偿我。".into(),
            confidence: 0.95,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 2, "长条目不受时长门触碰");
}

#[test]
fn test_similar_text_prefix_ignores_whitespace() {
    // 判据 bug 回归：前缀判据走归一化后，`Onyx Agate` 与 `OnyxAgate`（少一个空格）
    // 仍判为同句延续（实测 glupov 4→5、8→9 与 pierro 4→5、17→18、21→22 因原始
    // starts_with 而漏并）
    let short = "Stepanyan\nOnyx Agate\n嗯？是你啊…居然有幸";
    let long = "Stepanyan\nOnyxAgate\n嗯？是你啊…居然有幸再见面了。";
    assert!(similar_text(short, long, 0.3), "空白差异不应阻断前缀判定");
}

#[test]
fn test_short_fragment_merges_ocr_misread_head() {
    // 实测形态（glupov #4，1.25s）：首帧 OCR 把 `ll the` 误读成 `u the` → 并入后条
    // （2026-09-18 用户校正：此类属识别精度问题，按合并处理）
    let segs = vec![
        OcrSegment {
            start: 94.04,
            end: 95.29,
            text: "Stepanyan\nOnyx Agate\nu the".into(),
            confidence: 0.82,
        },
        OcrSegment {
            start: 95.29,
            end: 106.77,
            text: "Stepanyan\nOnyxAgate\nll the tedious, drawn-out paperwork and keeping a unit running smoothly".into(),
            confidence: 0.96,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 1, "1.25s 误读首帧应被并入");
    assert!((out[0].start - 94.04).abs() < 1e-9);
}

#[test]
fn test_short_fragment_merges_namebox_state() {
    // 实测形态（moon #5，1.50s）：长后段遮挡导致姓名框先变、打字机晚约 0.5s 进入
    // → 属同一显示，应并入后条（2026-09-18 用户校正）
    let segs = vec![
        OcrSegment { start: 56.23, end: 57.73, text: "Sonnet\"".into(), confidence: 1.0 },
        OcrSegment {
            start: 57.98,
            end: 68.55,
            text: "Sonnet\n've been having a great time playing around too, Canon!".into(),
            confidence: 0.96,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 1, "1.50s 姓名框态应被并入");
    assert!((out[0].start - 56.23).abs() < 1e-9);
}

#[test]
fn test_short_fragment_keeps_stable_namebox() {
    // 3.52s 姓名框态（glupov #18）：远超时长门 → 保持独立（其参考条目正文为「……」）
    let segs = vec![
        OcrSegment {
            start: 531.08,
            end: 534.60,
            text: "Anton\nFormer Acting Captain,\"Ninth Company".into(),
            confidence: 0.97,
        },
        OcrSegment {
            start: 535.03,
            end: 545.33,
            text: "Anton\nFormer Acting Captain,\"Ninth Company\nNo news. But perhaps... no news is the best news.".into(),
            confidence: 0.96,
        },
    ];
    let out = merge_short_fragments_into_next(segs, DEFAULT_MIN_SUBTITLE_SEC);
    assert_eq!(out.len(), 2, "3.52s 姓名框态应保持独立");
}

// ── 亚帧过渡帧吸收（D14 盲点，2026-10-06）──

#[test]
fn test_short_fragment_subframe_transition_absorbed() {
    // D14 盲点（2026-10-06；用户侧证据 = 用户在 GUI 里手动并掉了这一条，本 pass 欠账）。
    // pierro 嵌字产出**改前基线**实测（`temp/bench_output/pierro_questions_hardsub.srt`）：
    //   `[4] 119.052 → 119.136`（**0.084s**，只跨一帧）`MurderofE / Mitya`
    //   `[5] 119.203 → 130.314`（11.111s）`Mitya / ies for leaving…`
    // 碎片是**姓名框从 MurderofBirds 切到 Mitya 的那一帧**：头部 `MurderofE` 是旧名残留、
    // 尾部 `Mitya` 正是后条首行（新姓名框）。前置行一致性护栏按同序号比较头部
    // （`murderofe` vs `mitya`）⇒ 判成"换了说话人"⇒ 拒绝合并（1.5/2.0/3.5 三个门限都保留）。
    // 吸收方向 = 并入**前条**（碎片时间归前条；理由见 `merge_short_fragments_into_next`）。
    let segs = vec![
        OcrSegment {
            start: 109.693,
            end: 118.669,
            text: "MurderofBirds\nSo, Mitya helped too?".into(),
            confidence: 0.98,
        },
        OcrSegment {
            start: 119.052,
            end: 119.136,
            text: "MurderofE\nMitya".into(),
            confidence: 0.94,
        },
        OcrSegment {
            start: 119.203,
            end: 130.314,
            text: "Mitya\nies for leaving in such a hurry at the theater earlier. I was in a rush to verify\nsome theories.".into(),
            confidence: 0.94,
        },
    ];
    let out = merge_short_fragments_into_next(segs, 3.5);
    assert_eq!(out.len(), 2, "0.084s 过渡帧必须被吸收（用户人工兜底的那一条）");
    assert!((out[0].start - 109.693).abs() < 1e-9, "前条起点不变");
    assert!(
        (out[0].end - 119.136).abs() < 1e-9,
        "碎片时间归前条：终点延伸到碎片终点"
    );
    assert!(
        out[0].text.contains("So, Mitya helped"),
        "前条文本保留（碎片文本丢弃）"
    );
    assert!(
        (out[1].start - 119.203).abs() < 1e-9,
        "后条保留自己检测到的起点（碎片起点是过渡瞬间，不夺来当显示起点）"
    );
    assert!((out[1].end - 130.314).abs() < 1e-9, "后条终点不变");
}

#[test]
fn test_short_fragment_subframe_exemption_needs_subframe() {
    // 亚帧豁免**只对亚帧生效**：同一形态（`MurderofE/Mitya` → `Mitya/ies for leaving…`）
    // 若碎片时长 ≥ 一个网格间隔（此处 2.0s），前置行一致性护栏照旧拒绝。
    // 该判定确由护栏把关：末行 `Mitya` 与后条比较行的字符重叠 = 0.8 ≥ SHORT_FRAGMENT_OVERLAP
    // （下面先断言这一点）——去掉护栏、或去掉"亚帧"这个时长条件，本用例立刻变红。
    let fl: Vec<Vec<char>> = "MurderofE\nMitya".lines().map(norm_chars).collect();
    let nl: Vec<Vec<char>> = "Mitya\nies for leaving in such a hurry at the theater earlier."
        .lines()
        .map(norm_chars)
        .collect();
    assert!(
        line_weakly_related(&fl[1], &nl[1]),
        "单看弱关联为真 ⇒ 该判定确由前置行护栏把关（否则用例是假绿）"
    );
    let segs = vec![
        OcrSegment {
            start: 108.0,
            end: 117.0,
            text: "MurderofBirds\nSo, Mitya helped too?".into(),
            confidence: 0.98,
        },
        OcrSegment {
            start: 117.0,
            end: 119.0,
            text: "MurderofE\nMitya".into(),
            confidence: 0.94,
        },
        OcrSegment {
            start: 119.203,
            end: 130.314,
            text: "Mitya\nies for leaving in such a hurry at the theater earlier.".into(),
            confidence: 0.94,
        },
    ];
    let out = merge_short_fragments_into_next(segs, 3.5);
    assert_eq!(out.len(), 3, "2.0s > 一个网格间隔：亚帧豁免不得外溢，护栏照旧拒绝");
}

#[test]
fn test_short_fragment_subframe_ellipsis_kept() {
    // D9 优先级：纯标点（省略号）末行的碎片**无论多短**都不得被吸收——参考侧该条必须独立
    // 留存（glupov 语料 `[……]` 条，六项基准的长期硬门）。亚帧过渡帧（方向 1）与弱关联
    // （方向 2）**共用同一道门** `comparable_line`，两个子形态各钉一条路径，时长都压到亚帧：
    // ① 真实 glupov 语料形态（姓名/头衔/……）→ 钉方向 2（弱关联）不得碰它；
    // ② **构造**形态：真实过渡帧 `MurderofE/Mitya` 末尾追加一行纯标点——该形态的尾部行是
    //    空归一化（在过渡帧判据里按通配），空间关系上"像"后条的起始，故**只有 D9 先否决**
    //    才留得住；不追加这一行时 D9 与方向 1 在真实素材里并不冲突，用例无法在回归时变红。
    let d9_real = vec![
        OcrSegment {
            start: 516.0,
            end: 531.0,
            text: "安东\n原「第九连队」临时连长\n他们还在格鲁波夫休养。".into(),
            confidence: 0.93,
        },
        OcrSegment {
            start: 531.08,
            end: 531.16,
            text: "安东\n原「第九连队」临时连长\n……".into(),
            confidence: 0.88,
        },
        OcrSegment {
            start: 531.90,
            end: 545.0,
            text: "安东\n原「第九连队」临时连长\n没有消息。但也许……没有消息就是最好的消息。".into(),
            confidence: 0.95,
        },
    ];
    assert_eq!(
        merge_short_fragments_into_next(d9_real, 3.5).len(),
        3,
        "① 亚帧豁免不得越过 D9（真实语料形态：纯省略号条独立留存）"
    );

    let d9_transition = vec![
        OcrSegment {
            start: 109.693,
            end: 118.669,
            text: "MurderofBirds\nSo, Mitya helped too?".into(),
            confidence: 0.98,
        },
        OcrSegment {
            start: 119.052,
            end: 119.136,
            text: "MurderofE\nMitya\n……".into(),
            confidence: 0.94,
        },
        OcrSegment {
            start: 119.203,
            end: 130.314,
            text: "Mitya\nies for leaving in such a hurry at the theater earlier.".into(),
            confidence: 0.94,
        },
    ];
    assert_eq!(
        merge_short_fragments_into_next(d9_transition, 3.5).len(),
        3,
        "② 亚帧豁免不得越过 D9（过渡帧形态：末行纯标点 → 先否决再谈形态）"
    );
}

#[test]
fn test_short_fragment_keeps_speaker_switch_opponents() {
    // D17 记录的三个"换了说话人"对手：修复后必须**仍然不被合并**。
    // 三者的时长都 ≥ 一个网格间隔（2.83 / 2.80 / 3.40s），故完全不进亚帧豁免，
    // 仍由前置行一致性护栏与弱关联把关。
    //
    // ① pierro 嵌字 `「丑角」/可以。`（实测 2.83s）→ `派蒙/欸！真的可以吗！…`（末行重叠 0.50）。
    //    先断言"单看弱关联为真"：`可以` 是后条比较行的子序列 ⇒ 该对手**确由护栏把关**。
    let fl: Vec<Vec<char>> = "「丑角」\n可以。".lines().map(norm_chars).collect();
    let nl: Vec<Vec<char>> = "派蒙\n欸！真的可以吗！要不、要不还是算了，万一若娜瓦抓住机会，忽然冒出来…"
        .lines()
        .map(norm_chars)
        .collect();
    assert!(
        line_weakly_related(&fl[1], &nl[1]),
        "单看弱关联为真 ⇒ ①确由前置行护栏把关"
    );
    let jester = vec![
        OcrSegment {
            start: 100.0,
            end: 102.83,
            text: "「丑角」\n可以。".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 102.83,
            end: 110.0,
            text: "派蒙\n欸！真的可以吗！要不、要不还是算了，万一若娜瓦抓住机会，忽然冒出来…"
                .into(),
            confidence: 0.9,
        },
    ];
    assert_eq!(
        merge_short_fragments_into_next(jester, 3.5).len(),
        2,
        "① 换了说话人的短条不得被误判为续写"
    );

    // ② moon 语料产出**改前基线**实测 `[9] 90.334 → 93.134`（2.80s）`卡侬/…艾莉亚。`
    //    → `[10] 93.301 → 94.367`（1.066s）`艾莉亚/卡侬妹妹。`（末行重叠 1.00）。
    //    语料基准门限 1.5s 下它因**时长门**根本不进本 pass；这里用嵌字门限 3.5s 把它拉进来，
    //    专测护栏（= D17"安全阈值上限"的扫描口径）。
    let canon = vec![
        OcrSegment {
            start: 90.334,
            end: 93.134,
            text: "卡侬\n…艾莉亚。".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 93.301,
            end: 94.367,
            text: "艾莉亚\n卡侬妹妹。".into(),
            confidence: 0.9,
        },
    ];
    assert_eq!(
        merge_short_fragments_into_next(canon, 3.5).len(),
        2,
        "② 姓名框错位（卡侬 vs 艾莉亚）不得被误判为续写"
    );

    // ③ glupov `斯捷潘尼扬/「缟玛瑙」/嗯？是你啊…`（D17 记为与"另一句"重叠 0.55）
    //    → 下一条同姓名框的另一句（文本取 `examples/benchmark_examples/glupov.gsa` 里的
    //    真实相邻条目）。注：D17 探针的原始配对未随仓库保留，故按 D17 记述的形态复现
    //    （碎片 = 正文中途截断、后条 = 下一条目），实测重叠以本仓库产出为准。
    let stepanyan = vec![
        OcrSegment {
            start: 60.0,
            end: 63.40,
            text: "斯捷潘尼扬\n「缟玛瑙」\n嗯？是你啊…".into(),
            confidence: 0.9,
        },
        OcrSegment {
            start: 63.40,
            end: 72.0,
            text: "斯捷潘尼扬\n「缟玛瑙」\n嗯，最近上头安排我主管一支连队的所有事务，任务瞬间复杂了起来。"
                .into(),
            confidence: 0.95,
        },
    ];
    assert_eq!(
        merge_short_fragments_into_next(stepanyan, 3.5).len(),
        2,
        "③ 姓名框相同但正文换了句：不得被误判为续写"
    );
}

// ── 条带起点判据（检测层思路②）──

#[test]
fn test_band_onset_prefers_text_band() {
    // 文字带（行 3..6）渐显：整区判据要到累计 4 bit 才越阈，条带判据在 3 bit 时即命中
    let base = 0u64;
    // 行 3..6 的位：bit 24..48 → 依次置位模拟渐变
    let h1 = 0u64; // 无变化
    let h2 = 1u64 << 24; // 文字带 1 bit
    let h3 = (1u64 << 24) | (1u64 << 25) | (1u64 << 26); // 文字带 3 bit
    let h4 = h3 | (1u64 << 27); // 4 bit
    let window = vec![h1, h2, h3, h4];
    let pos = band_onset(&window, base, 3);
    assert_eq!(pos, Some(2), "条带阈值 max(2, 3×24/64=1)=2 → 第 3 帧即越阈");
}

#[test]
fn test_band_onset_none_when_global_motion() {
    // 三条带一起变（运镜）→ 无主导带 → 返回 None，调用方回退整区判据
    let base = 0u64;
    let last = (1u64 << 2) | (1u64 << 26) | (1u64 << 58); // 每带各 1 bit，分布均匀
    let window = vec![base, last];
    assert_eq!(band_onset(&window, base, 3), None);
}

// ── 标点/省略号归一化 ──

/// 默认配置（测试用）
fn pn() -> PunctuationNorm {
    PunctuationNorm::default()
}

#[test]
fn test_normalize_punctuation_brackets() {
    // 实测退化（glupov/pierro 共 20+ 处）：`「」` → `[]`/`【】` → 一律归一为「」
    assert_eq!(normalize_punctuation("「编玛瑙]", &pn()), "「编玛瑙」");
    assert_eq!(normalize_punctuation("【丑角】", &pn()), "「丑角」");
    assert_eq!(normalize_punctuation("（派蒙）", &pn()), "「派蒙」");
    assert_eq!(normalize_punctuation("{米提亚}", &pn()), "「米提亚」");
    assert_eq!(normalize_punctuation("「诺艾尔」", &pn()), "「诺艾尔」");
    // 半角 () 是英文正常标点 → 不参与归一化
    assert_eq!(normalize_punctuation("(Paimon)", &pn()), "(Paimon)");
}

#[test]
fn test_normalize_punctuation_ellipsis() {
    // 各类省略号 → 单个 `…`
    assert_eq!(normalize_punctuation("等等...", &pn()), "等等…");
    assert_eq!(normalize_punctuation("等等……", &pn()), "等等…");
    assert_eq!(normalize_punctuation("等等。。。", &pn()), "等等。。。".replace("。。。", "…"));
    assert_eq!(normalize_punctuation("等等···", &pn()), "等等…");
    assert_eq!(normalize_punctuation("等等…", &pn()), "等等…");
    // 单个句号/点号**不**变（避免把陈述句尾的 。 变成省略号）
    assert_eq!(normalize_punctuation("好的。", &pn()), "好的。");
    assert_eq!(normalize_punctuation("Mr. Smith", &pn()), "Mr. Smith");
    // 混排：括号 + 省略号
    assert_eq!(normalize_punctuation("【丑角】……你来了...", &pn()), "「丑角」…你来了…");
}

#[test]
fn test_normalize_punctuation_custom_targets() {
    // S1：目标字符可配置（用户个性化归一化目标）
    let cfg = PunctuationNorm {
        open_bracket: '[',
        close_bracket: ']',
        ellipsis: "……".to_string(),
        fix_misread_letters: true,
    };
    assert_eq!(normalize_punctuation("「丑角」……你来了...", &cfg), "[丑角]……你来了……");
    assert_eq!(normalize_punctuation("【派蒙】", &cfg), "[派蒙]");
}

#[test]
fn test_normalize_fixes_misread_punct_letters() {
    // S1 补充：英文嵌字引号被读成行尾 `j`（实测 9 处）→ 归一为右括号
    assert_eq!(normalize_punctuation("The Jesterj", &pn()), "The Jester」");
    assert_eq!(normalize_punctuation("Sonnetj", &pn()), "Sonnet」");
    // 正常单词里的 j 不受影响（不在行尾）
    assert_eq!(normalize_punctuation("just a moment", &pn()), "just a moment");
    assert_eq!(normalize_punctuation("enjoy", &pn()), "enjoy");
    // 行内已有括号 → 标点识别正常，不做字母替换
    assert_eq!(normalize_punctuation("「joker」 tail j", &pn()), "「joker」 tail j");
    // 关闭该修复时保持原样
    let off = PunctuationNorm {
        fix_misread_letters: false,
        ..PunctuationNorm::default()
    };
    assert_eq!(normalize_punctuation("The Jesterj", &off), "The Jesterj");
}

// ── 术语表标记（S2，只收集 Diff、不改文本）──

fn gl(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// 便捷断言：文本经术语表标记后应得到 (old 集合, new)
fn hits(text: &str, glossary: &[String]) -> Option<(Vec<String>, String)> {
    collect_glossary_hits(text, glossary).map(|(olds, new)| {
        let mut v: Vec<String> = olds.into_iter().collect();
        v.sort();
        (v, new)
    })
}

#[test]
fn test_glossary_collects_misread_near_form() {
    // 实测形态：缟玛瑙 被读成 编玛瑙（glupov 全部条目，共 11 处，首字即误读）
    let g = gl(&["缟玛瑙"]);
    assert_eq!(hits("「编玛瑙」", &g), Some((vec!["编玛瑙".to_string()], "缟玛瑙".to_string())));
    // **文本本身不被改写**（纠正由前端审批后执行）
    assert_eq!(hits("编玛瑙队长", &g), Some((vec!["编玛瑙".to_string()], "缟玛瑙".to_string())));
    // 完全一致 → 无命中
    assert_eq!(hits("缟玛瑙", &g), None);
}

#[test]
fn test_glossary_merges_multiple_misread_forms() {
    // 同一词条的多种误读形态应归入同一条目（old 为集合）。
    // 注意：形态必须各自达标（等长 + 比例 ≤0.34 + 首字同/差 1 字）
    let g = gl(&["斯捷潘尼扬"]);
    let mut diffs: Vec<Diff> = Vec::new();
    for t in ["斯捷潘尼杨", "斯捷潘尼羊"] {
        if let Some((olds, new)) = collect_glossary_hits(t, &g) {
            merge_diff(&mut diffs, olds, new);
        }
    }
    assert_eq!(diffs.len(), 1, "同一词条只应产生一条待审批条目");
    assert_eq!(diffs[0].new, "斯捷潘尼扬");
    assert_eq!(diffs[0].old, vec!["斯捷潘尼杨".to_string(), "斯捷潘尼羊".to_string()]);
}

#[test]
fn test_glossary_match_rules() {
    // 规则：等长 + 编辑距离比例 ≤0.34 + （首字符相同 或 仅 1 字符不同）
    assert!(hits("编玛瑙", &gl(&["缟玛瑙"])).is_some()); // 首字误读（3 字差 1）
    assert!(hits("NO.0219", &gl(&["NO.0217"])).is_some());
    assert!(hits("米提哑队长", &gl(&["米提亚"])).is_some()); // 同词内后续字误读
    assert!(hits("天空岛的造特", &gl(&["天空岛的造物"])).is_some());
    // 命中是**滑窗**语义：长片段里的局部误读也能标出（`编玛瑙队长` → 前 3 字命中）
    assert_eq!(
        hits("编玛瑙队长", &gl(&["缟玛瑙"])),
        Some((vec!["编玛瑙".to_string()], "缟玛瑙".to_string()))
    );
    // 完全不相关 → 不命中
    assert_eq!(hits("完全不相关的文本", &gl(&["缟玛瑙"])), None);
    // 长度不足（片段比词条短）→ 不命中
    assert_eq!(hits("玛瑙", &gl(&["缟玛瑙"])), None);
}

#[test]
fn test_glossary_keeps_unrelated_text() {
    let g = gl(&["缟玛瑙"]);
    assert_eq!(hits("这是一段普通的对话文本", &g), None);
    assert_eq!(hits("编队集合", &g), None);
    // 空词表 = 关闭
    assert_eq!(hits("编玛瑙", &[]), None);
}

#[test]
fn test_glossary_does_not_swallow_adjacent_punctuation() {
    // 回归用例（2026-09-24 实测）：词条 NO.0217 曾把 `，NO.0217` 当作窗口
    // （跨标点），导致替换后逗号丢失、长度错位。现在只收集命中、不改文本，
    // 但仍须保证**窗口不跨标点**（否则 old 会含逗号，前端替换会出问题）
    let g = gl(&["NO.0217"]);
    assert_eq!(hits("抱歉，NO.0217，前面在剧院我走得有些匆忙。", &g), None);
    assert_eq!(hits("见，NO.0217。", &g), None);
    // 真正的误读仍能标出（同片段内、不跨标点）
    assert_eq!(hits("NO.0219", &g), Some((vec!["NO.0219".to_string()], "NO.0217".to_string())));
}

#[test]
fn test_glossary_after_punctuation_normalization() {
    // 顺序：先归一化标点 → 再术语表（否则 `「斯捷潘尼杨」` 的括号会干扰匹配）
    let g = gl(&["斯捷潘尼扬"]);
    let normalized = normalize_punctuation("「斯捷潘尼杨」", &pn());
    assert_eq!(normalized, "「斯捷潘尼杨」");
    assert_eq!(
        hits(&normalized, &g),
        Some((vec!["斯捷潘尼杨".to_string()], "斯捷潘尼扬".to_string()))
    );
}

#[test]
fn test_glossary_ignores_single_char_terms() {
    // 单字符词条误伤面过大 → 忽略
    assert_eq!(hits("编玛瑙", &gl(&["缟"])), None);
}

// ── 一致性纠错（S3）──

fn seg_texts(texts: &[&str]) -> Vec<OcrSegment> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| OcrSegment {
            start: i as f64,
            end: i as f64 + 1.0,
            text: t.to_string(),
            confidence: 0.9,
        })
        .collect()
}

#[test]
fn test_consistency_hints_finds_variant_pair() {
    // 实测形态：缟玛瑙 被系统性读成 编玛瑙（glupov 11 处）——但若**两种写法并存**，
    // 才构成"变体对"可报；此处构造 3:1 的共存情形
    let segs = seg_texts(&["「缟玛瑙」", "「缟玛瑙」", "「缟玛瑙」", "「编玛瑙」"]);
    let hints = find_consistency_hints(&segs);
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].from, "编玛瑙");
    assert_eq!(hints[0].to, "缟玛瑙");
    assert_eq!(hints[0].from_count, 1);
    assert_eq!(hints[0].to_count, 3);
}

#[test]
fn test_consistency_hints_needs_two_variants() {
    // 只有一种写法（无变体）→ 没有可比较对象，不报建议
    let segs = seg_texts(&["「编玛瑙」", "「编玛瑙」", "「编玛瑙」"]);
    assert!(find_consistency_hints(&segs).is_empty());
}

#[test]
fn test_consistency_hints_reports_majority_as_target() {
    // 两种变体并存：多数派作为 to、少数派作为 from（**只建议，不改写文本**）
    let segs = seg_texts(&[
        "斯捷潘尼扬",
        "斯捷潘尼扬",
        "斯捷潘尼扬",
        "斯捷潘尼杨", // 少数派（1 次）
    ]);
    let hints = find_consistency_hints(&segs);
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].from, "斯捷潘尼杨");
    assert_eq!(hints[0].to, "斯捷潘尼扬");
    assert_eq!(hints[0].from_count, 1);
    assert_eq!(hints[0].to_count, 3);
}

#[test]
fn test_consistency_hints_skips_unbalanced_and_short() {
    // 占比不足 0.7（2:2）→ 不报
    let segs = seg_texts(&["斯捷潘尼扬", "斯捷潘尼扬", "斯捷潘尼杨", "斯捷潘尼杨"]);
    assert!(find_consistency_hints(&segs).is_empty());
    // 合计出现次数 < 3（1:1）→ 不报
    let segs2 = seg_texts(&["斯捷潘尼扬", "斯捷潘尼杨"]);
    assert!(find_consistency_hints(&segs2).is_empty());
    // 单字片段不参与（无法判断）
    let segs3 = seg_texts(&["扬", "扬", "杨"]);
    assert!(find_consistency_hints(&segs3).is_empty());
}

#[test]
fn test_consistency_hints_ignores_non_near_form() {
    // 等长但差 2 字符以上 → 视为不同词，不报
    let segs = seg_texts(&["斯捷潘尼扬", "斯捷潘尼扬", "斯捷潘尼扬", "完全不同词"]);
    assert!(find_consistency_hints(&segs).is_empty());
}

// ── 段尾精化（B-ii）──

#[test]
fn test_refine_segment_ends_uses_dense_switch() {
    // 段尾估计 = 末采样(10.0) + interval×0.5 = 10.25；密帧实测切换在 10.10 → 取 10.10
    let segs = vec![OcrSegment { start: 8.0, end: 10.25, text: "X".into(), confidence: 0.9 }];
    let a = 0x0u64;
    let b = 0xFFFF_FFFF_FFFF_FFFFu64; // 汉明距离 64 > 阈值
    let dense = vec![(9.0, a), (10.0, a), (10.1, b), (10.2, b)];
    let out = refine_segment_ends(segs, &dense, 3, 0.5, 30.0);
    assert!((out[0].end - 10.1).abs() < 1e-9, "实得 {}", out[0].end);
}

#[test]
fn test_refine_segment_ends_band_switch() {
    // 段尾条带判据：文字带（行 3..6）在窗口第 2 帧越阈（2 bit > band_th=2? 用 3 bit）
    // → 段尾取该帧时刻；整区判据（阈值 3）要到累计 4 bit 才命中
    let segs = vec![OcrSegment { start: 8.0, end: 10.25, text: "X".into(), confidence: 0.9 }];
    let a = 0u64;
    let text3 = (1u64 << 24) | (1u64 << 25) | (1u64 << 26); // 文字带 3 bit
    let text4 = text3 | (1u64 << 27); // 4 bit
    let dense = vec![(10.0, a), (10.1, text3), (10.2, text4)];
    let out = refine_segment_ends(segs, &dense, 3, 0.5, 30.0);
    assert!((out[0].end - 10.1).abs() < 1e-9, "条带应比整区更早命中（实得 {}）", out[0].end);
}

#[test]
fn test_refine_segment_ends_keeps_estimate_without_switch() {
    // 窗口内无越阈（本段延续到 clip 末尾）→ 保留原估计
    let segs = vec![OcrSegment { start: 8.0, end: 10.25, text: "X".into(), confidence: 0.9 }];
    let a = 0x0u64;
    let dense = vec![(9.0, a), (10.0, a), (11.0, a)];
    let out = refine_segment_ends(segs, &dense, 3, 0.5, 30.0);
    assert!((out[0].end - 10.25).abs() < 1e-9, "实得 {}", out[0].end);
}

// ── 空帧容错 ──

#[test]
fn test_merge_empty_tolerance_keeps_run() {
    // 单帧空（interval 0.5 → 容错窗口 0.8s）视为 OCR 抖动，不打断 run
    let frames = vec![
        ft(1.0, "A", 0.9),
        ft(2.0, "A", 0.9),
        ft(2.5, "", 0.0),
        ft(3.0, "A", 0.9),
    ];
    let segs = merge_frames(frames, 0.5, 10.0, 0.3);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "A");
    assert!((segs[0].end - 3.25).abs() < 1e-9); // last=3.0 + 0.5×interval 0.25
}

#[test]
fn test_merge_empty_beyond_tolerance_breaks() {
    // 连续空帧超过容错窗口（1.0s 间隔 → 窗口 1.5s）→ 打断
    let frames = vec![
        ft(1.0, "A", 0.9),
        ft(2.0, "A", 0.9),
        ft(3.0, "", 0.0),
        ft(4.0, "", 0.0),
        ft(5.0, "", 0.0),
        ft(6.0, "B", 0.9),
    ];
    let segs = merge_frames(frames, 1.0, 10.0, 0.3);
    assert_eq!(segs.len(), 2);
}

// ── 相邻段包含关系拼接（碎片化修复）──

#[test]
fn test_merge_contained_absorb_progressive() {
    // 渐进中间态短段在前、完整句在后 → 并入（诊断案例：卡侬·那是我本职工）
    let segs = vec![
        OcrSegment { start: 1.0, end: 2.0, text: "卡侬\n·那是我本职工".into(), confidence: 0.8 },
        OcrSegment { start: 2.0, end: 9.0, text: "卡侬\n…那是我本职工作的一部分。他们的主祭呼唤我的名字。".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1);
    assert!(out[0].text.starts_with("卡侬\n…那是我本职工作的一部分"));
    assert!((out[0].start - 1.0).abs() < 1e-9);
    assert!((out[0].end - 9.0).abs() < 1e-9);
}

#[test]
fn test_merge_contained_absorb_residual() {
    // 完整句在前、尾部残缺短段在后 → 并入（文本不变，时间取并集）
    let segs = vec![
        OcrSegment { start: 1.0, end: 8.0, text: "卡侬\n…那是我本职工作的一部分。他们的主祭呼唤我的名字。".into(), confidence: 0.9 },
        OcrSegment { start: 8.0, end: 9.0, text: "呼唤我的名字".into(), confidence: 0.8 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1);
    assert!((out[0].end - 9.0).abs() < 1e-9);
}

#[test]
fn test_merge_contained_absorb_long_residual_tail_match() {
    // D1 第 2 步定稿正例：moon clip 2/5 的 **pre-merge 真实形态**（段序 dump）——
    // 残尾 [01:52.067→01:53.734] 时长 1.667s（> 2.5×interval，最终事件显示的
    // 1.467s 是 clamp 假象），字尾窗比率 0.094 → 走长残尾档并入
    let segs = vec![
        OcrSegment { start: 97.001, end: 112.234, text: "Aria\nng to be an ordinary human girl and dancing in front of your own statue also\none of your duties? That's news to me.".into(), confidence: 0.9 },
        OcrSegment { start: 112.067, end: 113.734, text: "Aria\none of\nyour duties? That's news to me.".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1, "1.667s 句尾重读残尾（≤2.0s 门 + 字尾匹配）应并入");
    assert!(out[0].text.contains("dancing in front of your own"), "文本不变");
    assert!((out[0].end - 113.734).abs() < 1e-9, "时间取并集");
}

#[test]
fn test_merge_contained_nested_residual_does_not_shrink() {
    // D11 回归：pierro 实测段序——残片 #81 经方向 1 吸收完整长段 #82（并集
    // [484.55, 549.32]）后，又来了一个**嵌套在长段内**的短残片 #83
    // （[486.73, 486.98]）；方向 2 若用赋值 `last.end = seg.end` 会把
    // 64.8s 跨度截断成 2.4s（产出 62.5s 空窗、参考 65.7s 条目满扣）
    let segs = vec![
        OcrSegment { start: 484.55, end: 486.32, text: "Mitya\nre of the curse seems to be something like... when m".into(), confidence: 0.9 },
        OcrSegment { start: 486.45, end: 549.32, text: "Mitya\nire of the curse seems to be something like... when moments of technological progress occur, all life in the vicinity is taken away.".into(), confidence: 0.95 },
        OcrSegment { start: 486.73, end: 486.98, text: "Mitya\nire of the curse seems to be something like... when".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1, "三段应并成一段");
    assert!(
        (out[0].end - 549.32).abs() < 1e-9,
        "嵌套残片不得截断已累积跨度（实得 {}）",
        out[0].end
    );
    assert!((out[0].start - 484.55).abs() < 1e-9, "保留最早起点");
}

#[test]
fn test_merge_contained_rejects_namebox_state_as_residual() {
    // 独立姓名框态（对话行清空后只剩姓名框）不得被方向 2 吞并：其文本是任何带
    // 姓名框长段的子序列。glupov 语料 [18]（……省略号条的匹配项）即靠该独立态
    // 配对——放宽时长门曾把它吞成缺失 1（2026-09-16 实证，见 D1 节第 2 步）
    let segs = vec![
        OcrSegment { start: 1.0, end: 9.0, text: "Sonnet\nI've taken a shine to our new master, sisters, and they're willing to send subordinates to share our work".into(), confidence: 0.9 },
        OcrSegment { start: 9.1, end: 12.2, text: "Sonnet".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2, "独立姓名框态保持独立（3.1s 超 2.0s 上界）");
}

#[test]
fn test_merge_contained_rejects_namebox_short_state() {
    // 关键守护：glupov 语料 P17/P18 实测对（1.483s ≤ 2.0s 上界，**时长门放行**）
    // ——必须由"字尾匹配"拒绝：姓名框是前段的**字头**重复，尾窗比率 ≈1。这条
    // 测试即第 2 步首版（纯时长门放宽）吞掉 [18] 造成缺失 1 的回归守卫
    let segs = vec![
        OcrSegment { start: 60.7, end: 64.5, text: "安东\n原「第九连队」临时连长\n等大家恢复了精神，我们会随时准备迎接新的指令。直到陛下的宏愿实现，我们也许会死去，但不会被击垮。".into(), confidence: 0.9 },
        OcrSegment { start: 64.465, end: 65.948, text: "安东\n原「第九连队」临时连长".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2, "姓名框字头态须由字尾匹配拒绝（[18] 依赖它配对）");
}

#[test]
fn test_merge_contained_case_insensitive_progressive() {
    // 实况日志真实碎片对（pierro 44:01.556）：英文大小写/标点差异下仍应拼接
    let segs = vec![
        OcrSegment { start: 1.0, end: 1.484, text: "Paimon\n\"AL\"".into(), confidence: 0.9 },
        OcrSegment { start: 1.484, end: 3.435, text: "Paimon\n\"Allies\"?".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1);
    assert!(out[0].text.contains("Allies"));
    assert!((out[0].start - 1.0).abs() < 1e-9);
    assert!((out[0].end - 3.435).abs() < 1e-9);
}

#[test]
fn test_merge_contained_accepts_span_within_2_5x() {
    // 1.057s（2.11×interval）的真实碎片在 2.5×interval 护栏内 → 应拼接
    let segs = vec![
        OcrSegment { start: 1.0, end: 2.057, text: "Paimon\nal!? Uh..A—Actually, maybe it's best no".into(), confidence: 0.9 },
        OcrSegment { start: 2.057, end: 15.0, text: "Paimon\nal!? Uh... A—Actually, maybe it's best not to...? What if Ronova suddenly pop up again? She might take this chance to".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1, "1.057s ≤ 1.25s 护栏应拼接");
}

#[test]
fn test_merge_contained_rejects_cross_gap() {
    // 跨 2.86s 空档（远超 interval）即使文本包含也不拼接（决策：不覆盖此类碎片）
    let segs = vec![
        OcrSegment { start: 1.0, end: 2.029, text: "The Jester\nnthe circumstances, the Fat".into(), confidence: 0.9 },
        OcrSegment { start: 4.886, end: 30.0, text: "The Jester\nn the circumstances, the Fatui need not hide anything from our allies.".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2, "空档 2.857s > 0.5s 应拒绝拼接");
}

#[test]
fn test_merge_contained_keeps_complete_brief_entry() {
    // 短暂但完整的独立条目：包含关系成立但时长 3s 超护栏 → 保留（护栏取代 1/3 长度比守卫）
    let segs = vec![
        OcrSegment { start: 1.0, end: 4.0, text: "派蒙".into(), confidence: 0.9 },
        OcrSegment { start: 4.0, end: 9.0, text: "派蒙：旅行者你来了".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2, "3s > 1.25s 护栏应保留为独立条目");
}

#[test]
fn test_merge_contained_keeps_shared_prefix_distinct() {
    // 共享前缀但非包含的相邻短条 → 不并（由包含判据本身拒绝）
    let segs = vec![
        OcrSegment { start: 1.0, end: 1.4, text: "派蒙\n嗯？".into(), confidence: 0.9 },
        OcrSegment { start: 1.4, end: 1.8, text: "派蒙\n最近上头安排我主管一支连队".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2);
}

#[test]
fn test_merge_contained_takes_full_text() {
    // 无置信度守卫：方向 1 一律取更长（更完整）文本——语料 A/B 证明守卫会在更长
    // 文本置信度仅略低时保留较短残片（0.93 vs 0.94 属 OCR 噪声），使完整句从语料消失
    let segs = vec![
        OcrSegment { start: 1.0, end: 1.5, text: "Paimon\nto challe".into(), confidence: 0.95 },
        OcrSegment { start: 1.5, end: 3.0, text: "Paimon\nto challenge the Ruler of Death".into(), confidence: 0.60 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].text, "Paimon\nto challenge the Ruler of Death", "方向 1 取完整文本");
    assert!((out[0].end - 3.0).abs() < 1e-9, "时间取并集");
}

#[test]
fn test_merge_contained_keeps_distinct_short() {
    // 真短句（非包含）不并入
    let segs = vec![
        OcrSegment { start: 1.0, end: 2.0, text: "嗯？".into(), confidence: 0.9 },
        OcrSegment { start: 2.0, end: 9.0, text: "我们出发吧。".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2);
}

#[test]
fn test_is_line_prefix_cases() {
    // 逐行前缀 + 末行严格前缀（大小写/空白/标点容忍）
    assert!(is_line_prefix("A\nb", "a\nbc"));
    assert!(is_line_prefix("A\nbC", "a\nbcd e"));
    // D1 第 1 步：共享区 1 字符错位（编辑距离比 0.067~0.15）不再阻断
    // 已证案例（glupov/pierro 嵌字）：末行错位（替换）
    assert!(is_line_prefix(
        "Anton\ncompany\nnt they deserted t",
        "Anton\ncompany\nat they deserted the Fatui"
    ));
    // 已证案例：先前行的错位不再阻断整段判定
    assert!(is_line_prefix(
        "Anton\ncompany\nMitya\nDne has recovered their strength",
        "Anton\ncompany\nMitya\none has recovered their strength, we will be ready"
    ));
    // 末行整行相等 = 行边界切断（对话行清空状态）→ 否
    assert!(!is_line_prefix("a\nbc", "a\nbc\nde"));
    // 中段行差异过大（1/1 = 1.0 > 0.2）→ 否
    assert!(!is_line_prefix("a\nx\nbc", "a\ny\nbcd"));
    // 中段行差异过大（不同句，比率 ≈1.0 > 0.2）→ 否
    assert!(!is_line_prefix(
        "a\n今天天气很好",
        "a\n明天应该会下雨吧再说\n continued text here"
    ));
    // 前段行数多于后段 → 否
    assert!(!is_line_prefix("a\nb\nc", "a\nbc"));
    // 单行（派蒙 型独立短条）→ 否
    assert!(!is_line_prefix("派蒙", "派蒙：旅行者你来了"));
    // 极短末行 1 字符错位（比率 0.5 > 0.2）→ 保守拒绝
    assert!(!is_line_prefix("a\nbc", "a\nxcdef"));
    // 空行/空文本 → 否
    assert!(!is_line_prefix("a\n", "a\nb"));
}

#[test]
fn test_merge_contained_long_fragment_line_prefix() {
    // D9：glupov 嵌字型——残片被漏检拖长到 3.2s（超 2.5×interval 护栏），
    // 但末行是完整段末行的严格前缀 → 仍判为同一句补全，拼接
    let segs = vec![
        OcrSegment { start: 0.0, end: 3.2, text: "Anton\nFormer Acting Captain,\"Ninth Company\nIf they are still alive, I h".into(), confidence: 0.9 },
        OcrSegment { start: 3.41, end: 4.2, text: "Anton\nFormer Acting Captain,\"Ninth Company'\nIf they are still alive, I hope they never come back here.".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1, "行级前缀应拼掉 3.2s 残片");
    assert!((out[0].start - 0.0).abs() < 1e-9, "保留前段起点");
    assert!((out[0].end - 4.2).abs() < 1e-9, "时间取并集");
    assert!(out[0].text.contains("I hope they never come back here."), "取完整文本");
}

#[test]
fn test_merge_contained_wrap_extra_line() {
    // 完整段因换行多出一行（3 段 vs 4 段）：末行仍为严格前缀 → 拼接
    let segs = vec![
        OcrSegment { start: 0.0, end: 1.0, text: "Anton\nFormer Acting Captain,\"Ninth Company\nlurderofBirds. Inever thought I'".into(), confidence: 0.9 },
        OcrSegment { start: 1.2, end: 2.0, text: "Anton\nFormer Acting Captain,\"Ninth Company\nlurderofBirds. I never thought I'd see you again here in Snezhnograd. What a\npleasant surprise.".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 1);
    assert!(out[0].text.contains("pleasant surprise."));
}

#[test]
fn test_merge_contained_line_boundary_keeps_distinct() {
    // 对话行清空、只剩姓名框的独立状态：末行整行相等 → 行边界切断，不并入
    let segs = vec![
        OcrSegment { start: 0.0, end: 3.2, text: "安东\n原「第九连队」临时连长".into(), confidence: 0.9 },
        OcrSegment { start: 3.41, end: 4.2, text: "安东\n原「第九连队」临时连长\n没有消息。但也许……没有消息就是最好的消息。".into(), confidence: 0.9 },
    ];
    let out = merge_contained_adjacent(segs, 0.5);
    assert_eq!(out.len(), 2, "独立姓名框状态应保留");
}

// ── 相邻段时间 clamp（精化后防重叠）──

#[test]
fn test_recall_eligible_gates() {
    // 60fps 密集采样：dt = 1/60 ≈ 0.0167s，min_span=0.2s（12 帧），min_side=0.1s（6 帧）
    let dt = 1.0 / 60.0;
    // 窗口过窄：s 或 m 贴边
    assert!(!recall_eligible(1, 2, 30, dt, 0.2, 0.1)); // s == m+1
    assert!(!recall_eligible(1, 3, 30, dt, 0.2, 0.1)); // span = 0.033 < 0.2
    assert!(!recall_eligible(3, 15, 30, dt, 0.2, 0.1)); // a_side = 0.05 < 0.1
    assert!(!recall_eligible(8, 28, 30, dt, 0.2, 0.1)); // c_side = 0.033 < 0.1
    assert!(!recall_eligible(7, 28, 28, dt, 0.2, 0.1)); // s == wlen 贴尾
    // 三边都够：a=0.117, span=0.2, c=0.15 → 可召回
    assert!(recall_eligible(7, 19, 28, dt, 0.2, 0.1));
    // 30fps：span 0.2s = 6 帧
    let dt30 = 1.0 / 30.0;
    assert!(!recall_eligible(7, 12, 28, dt30, 0.2, 0.1)); // span = 0.167 < 0.2
    assert!(recall_eligible(7, 14, 28, dt30, 0.2, 0.1)); // span = 0.233 √
}

#[test]
fn test_clamp_segment_times_no_overlap() {
    // 前段 end 与后段 start 交叉 → clamp 到后段 start
    let segs = vec![
        OcrSegment { start: 1.0, end: 5.0, text: "A".into(), confidence: 0.9 },
        OcrSegment { start: 3.0, end: 8.0, text: "B".into(), confidence: 0.9 },
    ];
    let out = clamp_segment_times(segs);
    assert_eq!(out.len(), 2);
    assert!((out[0].end - 3.0).abs() < 1e-9);
    assert!((out[1].start - 3.0).abs() < 1e-9);
}

#[test]
fn test_clamp_segment_times_keeps_gap() {
    // 本就无重叠 → 时间不变
    let segs = vec![
        OcrSegment { start: 1.0, end: 3.0, text: "A".into(), confidence: 0.9 },
        OcrSegment { start: 5.0, end: 9.0, text: "B".into(), confidence: 0.9 },
    ];
    let out = clamp_segment_times(segs);
    assert!((out[0].end - 3.0).abs() < 1e-9);
    assert!((out[1].end - 9.0).abs() < 1e-9);
}

// ── 相邻相似合并（阶段 2 去伪短字幕）──

#[test]
fn test_merge_similar_adjacent_merges_same_text() {
    // 完全相同文本的相邻段 → 合并为一段（时间取并集）
    let segs = vec![
        OcrSegment { start: 1.0, end: 3.0, text: "卡侬\n…那是我本职工作的一部分。".into(), confidence: 0.9 },
        OcrSegment { start: 3.0, end: 3.2, text: "卡侬\n…那是我本职工作的一部分。".into(), confidence: 0.9 },
    ];
    let out = merge_similar_adjacent(segs, 0.3);
    assert_eq!(out.len(), 1);
    assert!((out[0].end - 3.2).abs() < 1e-9);
}

#[test]
fn test_merge_similar_adjacent_keeps_distinct() {
    // 文本差异大的相邻段 → 不合并
    let segs = vec![
        OcrSegment { start: 1.0, end: 3.0, text: "卡侬".into(), confidence: 0.9 },
        OcrSegment { start: 3.0, end: 5.0, text: "获得".into(), confidence: 0.9 },
    ];
    let out = merge_similar_adjacent(segs, 0.3);
    assert_eq!(out.len(), 2);
}

// ── lines_from_results ──

#[test]
fn test_lines_from_results_split_and_dedup() {
    // 拆行、trim、丢空行；批内及跨图去重（精确匹配）
    let results = vec![
        OcrResult {
            text: "旅行者，你来了\n\n  派蒙：  \n旅行者，你来了".into(),
            confidence: 0.9,
        },
        OcrResult {
            text: "  第二张的行  \n旅行者，你来了".into(),
            confidence: 0.8,
        },
    ];
    let lines = lines_from_results(&results);
    assert_eq!(lines, vec!["旅行者，你来了", "派蒙：", "第二张的行"]);
}

#[test]
fn test_lines_from_results_empty() {
    assert!(lines_from_results(&[]).is_empty());
    let blank = vec![OcrResult { text: "  \n\n".into(), confidence: 0.5 }];
    assert!(lines_from_results(&blank).is_empty());
}

// ── 采纳替换规则（replace_diff_forms / approve_corpus_diff）──

#[test]
fn test_replace_diff_forms_replaces_all_occurrences_of_all_forms() {
    let olds = vec!["编玛瑙".to_string(), "编玛脑".to_string()];
    // 每个形态替换其全部出现（与前端 split/join 等价，R3 语义）
    assert_eq!(
        replace_diff_forms("编玛瑙和编玛瑙", &olds, "缟玛瑙"),
        "缟玛瑙和缟玛瑙"
    );
    assert_eq!(
        replace_diff_forms("编玛瑙…编玛脑…编玛瑙", &olds, "缟玛瑙"),
        "缟玛瑙…缟玛瑙…缟玛瑙"
    );
    // 无命中 → 原样返回
    assert_eq!(replace_diff_forms("无关文本", &olds, "缟玛瑙"), "无关文本");
}

#[test]
fn test_replace_diff_forms_skips_short_old_forms() {
    // 防御：空串 replace 会在每字符间插入 new（灾难性）；单字符同理过宽
    assert_eq!(
        replace_diff_forms("abc", &["".to_string(), "b".to_string()], "X"),
        "abc"
    );
    // ≥2 字符正常生效
    assert_eq!(replace_diff_forms("abc", &["bc".to_string()], "X"), "aX");
}

#[test]
fn test_approve_corpus_diff_replaces_and_dedups() {
    use crate::project::CorpusItem;
    let item = |id: &str, text: &str| CorpusItem {
        id: id.to_string(),
        text: text.to_string(),
        source: "ocr_track".into(),
        created_at: "1".into(),
    };
    // c1 替换后与 c2（本就正确的写法）撞成同文 → 全等去重保留首现；
    // c3 独立替换。三进二出，保序。
    let corpus = vec![
        item("c1", "本就正确的编玛瑙"),
        item("c2", "本就正确的缟玛瑙"),
        item("c3", "编玛瑙"),
    ];
    let diff = Diff {
        old: vec!["编玛瑙".into()],
        new: "缟玛瑙".into(),
    };
    let out = approve_corpus_diff(corpus, diff);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "c1");
    assert_eq!(out[0].text, "本就正确的缟玛瑙");
    assert_eq!(out[1].id, "c3");
    assert_eq!(out[1].text, "缟玛瑙");
}
