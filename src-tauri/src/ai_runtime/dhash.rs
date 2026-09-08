// ─── dHash 感知哈希 ───────────────────────────────────────
// 用于 OCR 前的变化检测：对裁切后的帧算 64 bit dHash，
// 相邻（与最近一次已 OCR 的帧）汉明距离 ≤ 阈值 → 判定"未变化"，跳过 OCR。
//
// dHash：灰度 → 缩放到 9x8 → 每像素与右侧像素比较（左>右记1）→ 64 bit。
// 灰度 + 缩放由 ffmpeg 滤镜链（format=gray,scale=9:8）在管道内完成，
// Rust 只消费定长字节——哈希用途的帧全程不落盘（video::scan_frame_hashes）。

/// 从 rawvideo 管道输出的 9×8 灰度帧字节（72 字节，行优先）直接计算 dHash。
pub fn dhash_gray9x8(bytes: &[u8]) -> u64 {
    debug_assert_eq!(bytes.len(), 9 * 8, "rawvideo 灰度帧应为 9×8=72 字节");
    let mut hash: u64 = 0;
    let mut bit = 0u8;
    for y in 0..8 {
        for x in 0..8 {
            let left = bytes[y * 9 + x];
            let right = bytes[y * 9 + x + 1];
            if left > right {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }
    hash
}

/// 两个 64 bit 哈希的汉明距离（相异位数）
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 变化检测结果：一帧是否需要 OCR
#[derive(Debug, Clone)]
pub struct FrameChange {
    /// 帧时间（秒）
    pub time: f64,
    /// true → 相对最近一次已 OCR 的帧有明显变化，需要 OCR
    pub is_changed: bool,
    /// changed 帧"与之不同的上一段代表哈希"（窗口精化的基准 A；非 changed 帧为 None）
    pub base_hash: Option<u64>,
}

/// 对哈希序列做变化检测（纯内存版）。
///
/// 语义：第一帧恒为 changed；后续帧与"最近一次 changed 的帧"比较，
/// 汉明距离 ≤ 阈值视为未变化（可跳过 OCR）。
/// 返回与 `hashes` 等长的 (is_changed, base_hash) 标志序列，
/// 由调用方与帧列表 zip 组装 `FrameChange`。
pub fn change_flags(hashes: &[u64], threshold: u32) -> Vec<(bool, Option<u64>)> {
    let mut result = Vec::with_capacity(hashes.len());
    let mut last_ocr_hash: Option<u64> = None;

    for &hash in hashes {
        let (is_changed, base_hash) = match last_ocr_hash {
            None => (true, None),
            Some(prev) => {
                if hamming_distance(prev, hash) > threshold {
                    (true, Some(prev))
                } else {
                    (false, None)
                }
            }
        };
        if is_changed {
            last_ocr_hash = Some(hash);
        }
        result.push((is_changed, base_hash));
    }
    result
}

/// 窗口精化的结果
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRefine {
    /// 第一个与基准 A 距离 > 阈值（内容已明显变化）的窗口帧下标
    pub main_boundary: Option<usize>,
    /// main_boundary 之后内容又相对当前代表发生变化的帧下标（多突变/短字幕，阶段 2 用）
    pub sub_changes: Vec<usize>,
}

/// 返回窗口内"内容突变帧"下标序列（相对基准 A 累计判定），按时间升序。
///
/// 首个为 main 边界，后续为 sub（窗口内多突变/短字幕）。空 = 窗口内无变化。
/// 判定**相对当前代表**而非相邻帧，渐变累计超过阈值即命中。
pub fn boundary_indices(hashes: &[u64], base_hash: u64, threshold: u32) -> Vec<usize> {
    let mut boundaries = Vec::new();
    let mut current = base_hash;
    for (i, &h) in hashes.iter().enumerate() {
        if hamming_distance(current, h) > threshold {
            boundaries.push(i);
            current = h;
        }
    }
    boundaries
}

/// 在窗口密帧哈希序列上定位帧级变化边界（阶段 1：帧级打轴精度）。
///
/// `sub_changes` 记录主边界后又发生明显变化的下标（多突变/短字幕，阶段 2 召回）。
/// 变化帧全序列可通过 [`boundary_indices`] 获取（阶段 2 用于区分"真正的下一段
/// 边界"与"中间的短字幕"，并规避 A→短字幕→C 时误用 main 的边界 bug）。
pub fn refine_window_hashes(hashes: &[u64], base_hash: u64, threshold: u32) -> WindowRefine {
    let mut boundaries = boundary_indices(hashes, base_hash, threshold).into_iter();
    let main_boundary = boundaries.next();
    let sub_changes = boundaries.collect();
    WindowRefine {
        main_boundary,
        sub_changes,
    }
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dhash_gray9x8_flat_is_zero() {
        // 全部等灰度：left > right 恒假 → 哈希为 0
        assert_eq!(dhash_gray9x8(&[128; 72]), 0);
    }

    #[test]
    fn test_dhash_gray9x8_row_layout() {
        // 仅第 0 行递减（left > right 恒真）→ 只有前 8 位置位
        let mut bytes = [128u8; 72];
        for x in 0..9 {
            bytes[x] = (200 - x * 10) as u8;
        }
        assert_eq!(dhash_gray9x8(&bytes), 0xFF);
        // 全部行递减 → 64 位全 1
        for y in 0..8 {
            for x in 0..9 {
                bytes[y * 9 + x] = (200 - x * 10) as u8;
            }
        }
        assert_eq!(dhash_gray9x8(&bytes), u64::MAX);
    }

    #[test]
    fn test_dhash_gray9x8_direction_flips_hash() {
        // 同一帧内容左右镜像 → 对比方向反转 → 哈希不同
        let mut a = [128u8; 72];
        let mut b = [128u8; 72];
        for y in 0..8 {
            a[y * 9] = 200;
            a[y * 9 + 1] = 50;
            b[y * 9] = 50;
            b[y * 9 + 1] = 200;
        }
        assert_ne!(dhash_gray9x8(&a), dhash_gray9x8(&b));
    }

    #[test]
    fn test_hamming_distance() {
        assert_eq!(hamming_distance(0b0000, 0b0000), 0);
        assert_eq!(hamming_distance(0b1010, 0b0000), 2);
        assert_eq!(hamming_distance(0b1111, 0b0000), 4);
        assert_eq!(hamming_distance(u64::MAX, 0), 64);
    }

    #[test]
    fn test_change_flags_first_always_changed() {
        let flags = change_flags(&[0x1234], 5);
        assert_eq!(flags.len(), 1);
        assert!(flags[0].0);
        assert!(flags[0].1.is_none());
    }

    #[test]
    fn test_change_flags_identical_second_unchanged() {
        // 与第一帧相同 → 未变化
        let flags = change_flags(&[0x1234, 0x1234], 5);
        assert_eq!(flags.len(), 2);
        assert!(flags[0].0);
        assert!(!flags[1].0);
        assert!(flags[1].1.is_none());
    }

    #[test]
    fn test_change_flags_different_second_changed() {
        // hamming(0, 0x3F) = 6 > 阈值 5 → changed，且 base_hash = 第一帧
        let flags = change_flags(&[0x0, 0x3F], 5);
        assert!(flags[0].0);
        assert!(flags[1].0);
        assert_eq!(flags[1].1, Some(0x0));
        // 对照：距离恰等于阈值（5 位）→ 未变化
        let flags_eq = change_flags(&[0x0, 0x1F], 5);
        assert!(!flags_eq[1].0);
    }

    // ── 窗口精化（refine_window_hashes）──

    #[test]
    fn test_refine_window_main_boundary() {
        // 窗口内 A,A,B,B：突变 → 主边界在第一个 B
        let hashes = vec![0u64, 0, 0xFFFF, 0xFFFF];
        let r = refine_window_hashes(&hashes, 0u64, 5);
        assert_eq!(r.main_boundary, Some(2));
        assert!(r.sub_changes.is_empty());
    }

    #[test]
    fn test_refine_window_no_change() {
        // 窗口内全部与基准 A 相似 → 无边界
        let hashes = vec![0u64, 1, 2, 3];
        let r = refine_window_hashes(&hashes, 0u64, 5);
        assert_eq!(r.main_boundary, None);
        assert!(r.sub_changes.is_empty());
    }

    #[test]
    fn test_refine_window_sub_change() {
        // A,B,A：主边界后内容又变回 A（短字幕出现又消失）
        let hashes = vec![0u64, 0xFFFF, 0xFFFF, 0x0001];
        let r = refine_window_hashes(&hashes, 0u64, 5);
        assert_eq!(r.main_boundary, Some(1));
        assert_eq!(r.sub_changes, vec![3]);
    }

    #[test]
    fn test_refine_window_gradual_cumulative() {
        // 渐变累计：逐帧微小偏离 A（1,2,3,4...位），超过阈值（3）后才命中
        let hashes = vec![0u64, 1, 3, 7, 15, 31];
        let r = refine_window_hashes(&hashes, 0u64, 3);
        assert_eq!(r.main_boundary, Some(4)); // hamming(0,15)=4 > 3，首超阈值帧
        assert!(r.sub_changes.is_empty());
    }

    // ── boundary_indices（阶段 2：变化帧全序列）──

    #[test]
    fn test_boundary_indices_simple_switch() {
        // A,A,B,B：单一变化 → 序列只有一个边界
        let hashes = vec![0u64, 0, 0xFFFF, 0xFFFF];
        assert_eq!(boundary_indices(&hashes, 0u64, 5), vec![2]);
    }

    #[test]
    fn test_boundary_indices_short_subtitle() {
        // A,B'(短字幕),C：两次变化 → main=B'首帧、sub=C 首帧
        let hashes = vec![0u64, 0, 0x0F0F, 0x0F0F, 0xFFFF, 0xFFFF];
        // 0 与 0x0F0F 距离大(>5) → main=2；0x0F0F 与 0xFFFF 距离大 → sub=4
        assert_eq!(boundary_indices(&hashes, 0u64, 5), vec![2, 4]);
    }

    #[test]
    fn test_boundary_indices_empty() {
        let hashes = vec![0u64, 1, 2, 3];
        assert!(boundary_indices(&hashes, 0u64, 5).is_empty());
    }
}
