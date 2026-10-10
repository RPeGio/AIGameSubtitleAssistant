use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 合并后的一段字幕事件（不含 id，写轨道时由 store 分配）
#[derive(Debug, Clone, Serialize)]
pub struct OcrSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub confidence: f64,
}

/// 一帧的文本（顺延后的完整序列）。
/// text 用 `Arc<str>` 共享，未变化帧只做引用计数递增，不逐帧拷贝字符串。
/// `sample_time` = 该帧的网格采样时刻（图像的实际采样点）；帧级差分注入的
/// 时刻覆写只改 `time`（有效边界），采样覆盖仍以 `sample_time` 计。
#[derive(Debug, Clone)]
pub struct FrameText {
    pub time: f64,
    pub sample_time: f64,
    pub text: Arc<str>,
    pub confidence: f64,
}

/// 字幕预估最短长度的**默认值**（秒）——前端可调项 `OcrRunParams::min_subtitle_sec` 的缺省。
///
/// 依据（2026-09-18 主观评审 + 基准实测）：真实字幕寿命通常 ≥1s（与 D10 保险丝 1.5s、
/// D9 的 2.5×interval 护栏同源），短于此的产出段几乎必为**分段错误**——实测嵌字碎片成因：
/// 打字机首帧、OCR 误读（`You've`→`Du've`、`ll the`→`u the`）、共享区少一个空格
/// （`Onyx Agate`/`OnyxAgate`）、**长后段遮挡导致姓名框先变而打字机晚半秒进入**（moon #5）、
/// 淡出残留、画面图案误识别。
///
/// 取 **1.5s**（2026-09-18 用户校正后放宽）：覆盖实测的 glupov #4（1.25s 误读首帧）与
/// moon #5（1.50s 姓名框态）。语料侧同形态条目（如 OCR 漏掉纯「……」行后退化为"姓名+头衔"
/// 的那条）会因此被后段并入——用户判定其**根因是识别精度而非合并缺陷**，故语料基准增加
/// "被并入"判定（`tests/common::absorbed_indices`）单列报告、不计满分缺失。
pub const DEFAULT_MIN_SUBTITLE_SEC: f64 = 1.5;

/// 标点归一化配置（S1：参数化，供前端"个性化归一化目标"使用）。
///
/// 归一化是**精度策略的统一前置层**（用户 2026-09-24 决策）：术语表/一致性纠错都要求
/// 产出文本与词条**同形**才能匹配，故先统一标点形态，纠错只需关心实词字形。
#[derive(Debug, Clone, Deserialize)]
pub struct PunctuationNorm {
    /// 左括号归一化目标（默认 `「`；用户可改为 `[` 等）
    #[serde(default = "default_open_bracket")]
    pub open_bracket: char,
    /// 右括号归一化目标（默认 `」`）
    #[serde(default = "default_close_bracket")]
    pub close_bracket: char,
    /// 省略号归一化目标（默认 `…`；用户可改为 `……` 等）
    #[serde(default = "default_ellipsis")]
    pub ellipsis: String,
    /// 是否修复"标点被识别成拉丁字母"（默认开，见 `fix_misread_punct_letters`）
    #[serde(default = "default_true")]
    pub fix_misread_letters: bool,
}

pub(crate) fn default_open_bracket() -> char {
    DEFAULT_OPEN_BRACKET
}
pub(crate) fn default_close_bracket() -> char {
    DEFAULT_CLOSE_BRACKET
}
pub(crate) fn default_ellipsis() -> String {
    DEFAULT_ELLIPSIS.to_string()
}
pub(crate) fn default_true() -> bool {
    true
}
/// `OcrRunParams::min_subtitle_sec` 的 serde 缺省（与产品默认同源）
pub(crate) fn default_min_subtitle_sec() -> f64 {
    DEFAULT_MIN_SUBTITLE_SEC
}

impl Default for PunctuationNorm {
    fn default() -> Self {
        Self {
            open_bracket: DEFAULT_OPEN_BRACKET,
            close_bracket: DEFAULT_CLOSE_BRACKET,
            ellipsis: DEFAULT_ELLIPSIS.to_string(),
            fix_misread_letters: true,
        }
    }
}

/// 默认归一化目标（用户 2026-09-18 指定）：
/// - 各类括号（`（）`、`{}`、`【】`、`[]`、`〔〕〖〗〈〉《》『』` 及 `「」` 自身）→ `「」`；
/// - 各类省略号（`...`、`。。`、`···`、`……`、`…`）→ 单个 `…`。
///
/// 说明：**半角 `()` 不参与归一化**——英文嵌字里它是正常标点；而 `[]`/`{}` 保留参与，
/// 是因为实测 OCR 会把 `「」` 退化成 `]`/`【`（glupov/pierro 共 20+ 处），归一化正好修掉该缺陷。
pub(crate) const DEFAULT_OPEN_BRACKET: char = '「';
pub(crate) const DEFAULT_CLOSE_BRACKET: char = '」';
pub(crate) const DEFAULT_ELLIPSIS: &str = "…";

/// 前端传入的 ocr_region 选区（归一化坐标 + 时间段）
#[derive(Deserialize)]
pub struct OcrRegionInput {
    pub start: f64,
    pub end: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// OCR 运行参数
#[derive(Deserialize)]
pub struct OcrRunParams {
    /// 帧间隔（秒），默认 0.5（与前端 `ocrDefaults.ts` 的 createDefaultOcrParams 同源）
    pub frame_interval: f64,
    /// dHash 变化检测阈值，默认 3
    pub dhash_threshold: u32,
    /// OCR 批大小，默认 16
    pub batch_size: usize,
    /// 合并相似度阈值（编辑距离比例，默认 0.3）
    pub merge_similarity: f64,
    /// 字幕预估最短长度（秒，产品默认 1.5，见 `DEFAULT_MIN_SUBTITLE_SEC`）：短于此的产出段
    /// 若与后一条弱关联，则并入后一条（保留碎片起点 + 后条终点/文本）。**前端可调**——
    /// 调大能减少碎片，但会提高"误吞真实短句"的概率（基准语料硬门会暴露）。
    ///
    /// 该门是**素材相关**的：实况嵌字的字幕寿命天然长于剧情语料（基准实测参考条目最短
    /// 时长：嵌字 3.52/3.62/2.25s vs 语料 1.48s），故基准按素材分别标定。
    /// serde default 与前端缺省同源（`DEFAULT_MIN_SUBTITLE_SEC`），防旧参数缺字段反序列化失败
    #[serde(default = "default_min_subtitle_sec")]
    pub min_subtitle_sec: f64,
    /// 标点归一化配置（默认见 `PunctuationNorm::default`）；缺省时用默认值
    #[serde(default)]
    pub punctuation: PunctuationNorm,
    /// 术语表（S2，可选精度策略）：用户提供的正确词条列表（如游戏专有名词）。
    ///
    /// 产出文本在**标点归一化之后**与词条做模糊匹配（编辑距离比例 ≤0.34），
    /// 命中即替换为词条——用于纠正形近字误读（如 `编玛瑙` → `缟玛瑙`）。
    /// 空列表 = 关闭（默认）。**前端可增删条目**（input-text 列表）。
    #[serde(default)]
    pub glossary: Vec<String>,
    /// 一致性纠错（S3，可选）：扫描产出文本中的形近变体并**只给建议**（不改写文本）。
    ///
    /// 用户指出"多数派不一定正确"，故本策略不做自动改写：运行结束后通过进度回调
    /// 输出建议行（`[精度建议] 少数派 → 多数派（x/y 次）`），由用户决定是否写入术语表。
    /// 默认关闭。
    #[serde(default)]
    pub consistency_hints: bool,
}

/// 进度事件载荷
#[derive(Clone, Serialize)]
pub struct OcrProgress {
    pub clip_index: usize,
    pub clip_count: usize,
    /// 整体进度 0.0 ~ 1.0
    pub progress: f64,
    pub message: String,
}

