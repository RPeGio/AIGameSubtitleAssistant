import { ref, watch, type Ref } from "vue";
import type { OcrRunParams, PunctuationNorm } from "../types";

/// 语料/嵌字 OCR 参数的共享默认值（新增参数只改此处）。
export function defaultPunctuation(): PunctuationNorm {
  return {
    open_bracket: "「",
    close_bracket: "」",
    ellipsis: "…",
    fix_misread_letters: true,
  };
}

/// 产品默认 `min_subtitle_sec` 与后端 `DEFAULT_MIN_SUBTITLE_SEC` 同源（1.5s）
export function createDefaultOcrParams(): OcrRunParams {
  return {
    frame_interval: 0.5,
    dhash_threshold: 3,
    batch_size: 16,
    merge_similarity: 0.3,
    min_subtitle_sec: 1.5,
    punctuation: defaultPunctuation(),
    glossary: [],
  };
}

// ────────────────────────── 持久化（2026-10 修复）──────────────────────────
//
// **修复的前端缺陷**：此前 CorpusView / AsrView 各写
// `const ocrParams = ref(createDefaultOcrParams())`——那是**组件局部状态**，
// 切换栏目（组件卸载）就丢，每次都静默回到默认值。后果：用户以为在用自己调好的参数、
// 实际跑的是默认参数，**产出可信度直接塌掉且没有任何提示**；`min_subtitle_sec` 这类
// 关键参数更是事后无从复现（本项目的 vesna 案例就踩到了：那条嵌字轨的阈值再也考据不出来）。
//
// 修法：按**栏目各自持久化**到 localStorage。
// 为什么不分栏共享一份：语料 OCR 与嵌字 OCR 是两次不同操作，参数本就该分开
// （例如嵌字侧常需按素材把 `min_subtitle_sec` 调低，语料侧则沿用产品默认）。

const LS_PREFIX = "gsa.ocrParams.";

function loadOcrParams(key: string): OcrRunParams {
  const base = createDefaultOcrParams();
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return base;
    const saved = JSON.parse(raw) as Partial<OcrRunParams>;
    // 与默认值**逐字段合并**：以后新增参数能自动拿到默认值，旧存档也不会因缺字段而崩
    return {
      ...base,
      ...saved,
      punctuation: { ...base.punctuation, ...(saved.punctuation ?? {}) },
      glossary: Array.isArray(saved.glossary) ? saved.glossary : base.glossary,
    };
  } catch {
    // 存档损坏 ⇒ 退回默认，不阻塞用户
    return base;
  }
}

function saveOcrParams(key: string, v: OcrRunParams): void {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    // 存储不可用（隐私模式 / 配额满）⇒ 静默降级为"不持久化"，不影响本次运行
  }
}

/// 创建**跨栏目切换保留**的 OCR 参数 ref。
///
/// `scope` 决定持久化键，语料与嵌字各自独立。
export function usePersistedOcrParams(scope: "corpus" | "asr"): Ref<OcrRunParams> {
  const key = LS_PREFIX + scope;
  const params = ref<OcrRunParams>(loadOcrParams(key)) as Ref<OcrRunParams>;
  watch(params, (v) => saveOcrParams(key, v), { deep: true });
  return params;
}
