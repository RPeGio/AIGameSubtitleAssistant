/// 句界切分：分割工具的「按断句标点」「按空格」两种策略共用。
///
/// 设计依据（用户 2026-10-19 的图解）：把每个断点的字符比例折算成时间，各断点以
/// 相同速率向左右对称扩散、相遇即停 ⇒ 最终分区等价于「时间上离 T 最近的断点」
/// （等速率 ⇒ 相邻相遇点在中点；外侧断点延伸到 clip 两端）。
/// 这里直接实现该等价判定，不模拟扩散过程。
///
/// 现状（"raw"）是两段都保留原文本，用户随后手工改；本模块只服务另外两档。

export type SplitStrategy = "punct" | "space" | "raw";
export type MergeStrategy = "latter" | "former" | "stack";

/// 断句标点（中英混排常见句读）
const SENTENCE_PUNCT = /[。．.！!？?；;，,、：:]/;
const SPACE = /\s/;

function isBreak(ch: string, mode: SplitStrategy): boolean {
  if (mode === "punct") return SENTENCE_PUNCT.test(ch);
  if (mode === "space") return SPACE.test(ch);
  return false;
}

/// 可切位置的字符下标集合：每个元素是「左段结束」的下标（断点之后）。
/// 句首、句尾的断点丢弃（按图解：句头句尾标点不参与断句）。
export function cutIndices(text: string, mode: SplitStrategy): number[] {
  if (!text || mode === "raw") return [];
  const cuts: number[] = [];
  let i = 0;
  while (i < text.length) {
    if (!isBreak(text[i], mode)) {
      i++;
      continue;
    }
    // 连续断点（连续空白/连续标点）合并成一个断点，切在最后一个之后
    let j = i;
    while (j + 1 < text.length && isBreak(text[j + 1], mode)) j++;
    const cut = j + 1;
    const headEmpty = text.slice(0, i).trim() === "";
    const tailEmpty = text.slice(cut).trim() === "";
    if (!headEmpty && !tailEmpty) cuts.push(cut);
    i = cut;
  }
  return cuts;
}

/// 比例（0..1）最近的断点下标；并列取靠左的。没有断点返回 null。
export function nearestCut(cuts: number[], length: number, ratio: number): number | null {
  if (cuts.length === 0 || length <= 0) return null;
  const r = Math.min(1, Math.max(0, ratio));
  let best = cuts[0];
  let bestDist = Math.abs(best / length - r);
  for (let k = 1; k < cuts.length; k++) {
    const d = Math.abs(cuts[k] / length - r);
    if (d < bestDist) {
      best = cuts[k];
      bestDist = d;
    }
  }
  return best;
}

/// 在 cut 处把文本切成两段：左段保留断点（标点/空白）并去掉尾部空白，
/// 右段去掉首部空白。
export function splitTextAtCut(text: string, cut: number): { left: string; right: string } {
  return { left: text.slice(0, cut).trimEnd(), right: text.slice(cut).trimStart() };
}

/// 只剥掉句末连续的标点与空白（句中标点不动）——第 17 项「去除句末标点」用
export function stripTrailingPunctuation(text: string): string {
  return text.replace(/[。．.！!？?；;，,、：:…—\s]+$/u, "").trimEnd();
}

/// 策略选项（面板与持久化共用）
export const SPLIT_OPTIONS: { value: SplitStrategy; label: string }[] = [
  { value: "punct", label: "按断句标点" },
  { value: "space", label: "按空格" },
  { value: "raw", label: "保留原文" },
];

export const MERGE_OPTIONS: { value: MergeStrategy; label: string }[] = [
  { value: "latter", label: "保留后者文本" },
  { value: "former", label: "保留前者文本" },
  { value: "stack", label: "前后文本堆叠" },
];

/// 把文本按「标点结尾」切成若干段，每段**原样保留**它自己的标点。
/// 例：`123，4567，234，` → ["123，", "4567，", "234，"]。
/// 句首标点产生的空片并入下一片（不丢字符），整段无标点时返回单段。
/// 供「被分割态」下键盘 ⬅️➡️ 整段搬家使用（搬家逐字进行，不 trim）。
export function textSegments(text: string): string[] {
  if (!text) return [];
  const cuts: number[] = [];
  let i = 0;
  while (i < text.length) {
    if (!SENTENCE_PUNCT.test(text[i])) {
      i++;
      continue;
    }
    let j = i;
    while (j + 1 < text.length && SENTENCE_PUNCT.test(text[j + 1])) j++;
    cuts.push(j + 1);
    i = j + 1;
  }
  if (cuts.length === 0) return [text];

  const pieces: string[] = [];
  let from = 0;
  for (const cut of cuts) {
    pieces.push(text.slice(from, cut));
    from = cut;
  }
  if (from < text.length) pieces.push(text.slice(from));

  // 空片（句首标点造成）并入相邻片：头部并入下一片、尾部并入上一片
  const merged: string[] = [];
  for (const piece of pieces) {
    if (piece === "") continue;
    if (merged.length === 0) merged.push(piece);
    else if (piece.trim() === "" && merged.length > 0) merged[merged.length - 1] += piece;
    else merged.push(piece);
  }
  return merged.length > 0 ? merged : [text];
}
