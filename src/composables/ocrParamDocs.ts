/// OCR 运行参数的悬浮解释（InfoDot 的文案来源）。
///
/// 文案依据（改文案时请一并核对）：
/// - `src-tauri/src/ocr/params.rs`：字段注释、默认值常量（`DEFAULT_MIN_SUBTITLE_SEC` /
///   `DEFAULT_MIN_TEXT_LENGTH`）与「素材相关门」的标定说明
/// - `src-tauri/src/ocr/pipeline.rs`：参数如何进入抽帧网格、扫描步长与批处理
/// - `src-tauri/src/ocr/merge.rs`：相似度合并、短碎片吸收、文本长度门的实现
/// - `src-tauri/src/ocr/refine.rs` / `src-tauri/src/ai_runtime/dhash.rs`：段尾边界判定
/// - `docs/development.md`：管线图与默认值表
///
/// 每条统一按「作用 → 调大调小的代价 → 默认值与关闭值」三段写，用 \n 分隔
/// （`InfoDot` 的 tooltip 设了 `white-space: pre-line`）。
export const OCR_PARAM_DOCS: Record<string, string> = {
  /// pipeline.rs:334,343 抽帧网格；:306 扫描步长；:241-245 ≤0 退化
  frame_interval:
    "抽帧网格的间隔（秒）：候选帧时刻 = 选区起点 + k × 间隔。\n" +
    "调小：抓得住一闪而过的嵌字，但送 OCR 的帧数成正比上升、更慢；调大：更快，但可能漏掉短命字幕。实际扫描步长取 min(1/源帧率, 帧间隔)。\n" +
    "默认 0.5；≤0 会退化成 1 秒。",
  /// development.md:183 管线图；pipeline.rs:246,350,450 传入变化检测；refine.rs:229-232 段尾边界
  dhash_threshold:
    "64-bit dHash 的汉明距离门：与参考帧的差异 ≤ 此阈值即视为「画面没变」，跳过 OCR。也用于找字幕段的收尾边界。\n" +
    "调大：跳过的帧更多、更快，但可能漏掉细微变化（淡入淡出、半透明文字）；调小：更敏感，也更慢。\n" +
    "默认 3。",
  /// params.rs:137-138 字段注释；scripts/ocr_worker.py:136-158 批对齐；stores/project.ts:710 实测说明
  batch_size:
    "送往 OCR 的单批帧数（一次推理处理的画面数量）。\n" +
    "调大：吞吐更高，但单批延迟与显存占用上升；调小：更省内存、进度更细，但整体更慢。\n" +
    "默认 16，下限 1。（实测 8 已能平衡批吞吐与单批耗时）",
  /// merge.rs:129-151 similar_text = edit_distance_ratio ≤ 阈值；:295,364 实际用法
  merge_similarity:
    "相邻帧文本的编辑距离比例门：比例 ≤ 此阈值即当作「同一句的过渡帧」合并。\n" +
    "调大：更容易合并（相邻两句被并成一句的风险上升）；调小：更保守，但同一句的多次识别可能留下重复段。\n" +
    "默认 0.3。",
  /// params.rs:141-149 字段注释；merge.rs:767-835 短碎片吸收；pipeline.rs:249-250 ≤0 关闭
  min_subtitle_sec:
    "字幕预估最短长度（秒）：短于此的产出段若与后一条弱关联，就并入后一条（保留碎片起点 + 后条的终点与文本）。\n" +
    "调大：碎片更少，但可能误吞真实短句；调小：更贴近原样，碎片更多。该门与素材相关——实况嵌字的字幕寿命天然长于剧情对白。\n" +
    "默认 1.5；≤0 = 关闭这个合并步骤。",
  /// params.rs:167-170 字段注释；merge.rs:906-916 判定实现；norm_chars(:392-399) 只留字母数字
  min_text_length:
    "单段最少实质字符数：低于此值的产出段直接丢弃（纯符号、单字符噪声）。\n" +
    "调大：更干净，但可能丢掉「！」「……」这类有意义的短段（实质字符只算字母数字，标点与空格不计）。\n" +
    "默认 1：只丢纯符号段；0 = 关闭这道门。",
};

/// 术语表（同一套 hover 机制）
/// 依据：params.rs:153-159（模糊匹配、编辑距离比例 ≤0.34）、correct.rs:232（GLOSSARY_MATCH_TOL）
export const GLOSSARY_DOC =
  "用户提供的正确词条（如游戏专有名词）：产出文本在标点归一化之后与词条做模糊匹配，命中即替换为词条。\n" +
  "用于纠正形近字误读 —— 例：编玛瑙 → 缟玛瑙。编辑距离比例 ≤ 0.34 视为命中。\n" +
  "留空 = 关闭。";
