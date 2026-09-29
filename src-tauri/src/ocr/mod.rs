// ─── OCR 字幕生成流水线 ────────────────────────────────────
// 编排：run_ocr（命令）→ pipeline::run_ocr_pipeline 串联：
//   扫描 → 抽帧 → 窗口精化 → OCR → 六遍合并治理 → 精度策略（归一/标记）
//
// 子模块（PR34 后按领域拆分；对外 API 路径不变，经本文件 re-export）：
//   params    —— 数据类型与运行参数（OcrSegment/OcrRunParams/标点归一化配置…）
//   pipeline  —— ocr_pass / 帧装配 / 编排 run_ocr_pipeline / 临时目录守卫
//   merge     —— 合并层六遍：投票(渐进补全)/包含拼接/相似合并/短碎片吸收
//   refine    —— 窗口精化（阶段 1/2）、条带判据、段尾精化与 clamp
//   correct   —— 精度策略：标点归一化 + 术语表(S2) + 一致性纠错(S3)，只标记不改写
//   commands  —— Tauri 命令（run_ocr / run_ocr_images / approve_corpus_diff）
//
// 依赖：
//   video::scan_frame_hashes     → Vec<(time, dhash)>（零落盘变化检测，网格+精化共用）
//   video::extract_frames_bytes  → mjpeg 管道内存帧（仅变化帧保留字节，零落盘）
//   dhash::change_flags_rescued  → 补漏变化标志（帧级差分注入 + 静态超时保险丝）
//   ai_runtime::OcrProvider      → 批量识别（images 元素 = base64 JPEG，IPC 不经磁盘）

// ── 子模块（按领域拆分，行为不变；本文件只做声明与 re-export）──
pub(crate) mod commands;
mod correct;
mod merge;
mod params;
mod pipeline;
mod refine;

// 对外 API（与拆分前完全一致的路径）
pub use commands::{approve_corpus_diff, lines_from_results, run_ocr, run_ocr_images};
pub use correct::{Diff, find_consistency_hints, replace_diff_forms};
pub use merge::{merge_contained_adjacent, merge_frames};
pub use params::{DEFAULT_MIN_SUBTITLE_SEC, OcrProgress, OcrRegionInput, OcrRunParams, OcrSegment, PunctuationNorm};
pub use pipeline::{fmt_time, ocr_pass, run_ocr_pipeline};
pub use refine::refine_window_changes;

// 内部符号引入 ocr 作用域：ocr::tests 经 `use super::*` 访问
#[allow(unused_imports)]
pub(crate) use correct::*;
#[allow(unused_imports)]
pub(crate) use merge::*;
#[allow(unused_imports)]
pub(crate) use params::*;
#[allow(unused_imports)]
pub(crate) use pipeline::*;
#[allow(unused_imports)]
pub(crate) use refine::*;

#[cfg(test)]
mod tests;

