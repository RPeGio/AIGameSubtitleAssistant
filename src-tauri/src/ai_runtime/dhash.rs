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
    /// 帧时间（秒）。帧级差分注入的时刻覆写只改此字段（有效边界），
    /// 采样覆盖以 `sample_time` 计。
    pub time: f64,
    /// 该帧的网格采样时刻（OCR 图像的实际采样点）；常规帧 = time
    pub sample_time: f64,
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

/// 变化标志序列：(is_changed, base_hash)，与输入哈希序列等长
pub type ChangeFlags = Vec<(bool, Option<u64>)>;

/// 网格变化检测（含补漏，D8）。
///
/// 缺陷背景：`change_flags` 的代表链语义（每帧与"最近已 OCR 帧"比较）在
/// **画面静止 + 同位置整行文字替换**时会整句漏检——两行文字在 9×8 dHash 下
/// 距离仅 2~4 位（≤阈值 3），且字幕切换常落在两个网格样本之间。实测 pierro
/// 语料 [75]"请期待吧……"显示 6 秒却从未触发 OCR，语料永久缺失。
///
/// 补漏两层：
/// - **(1a) 帧级差分注入**：密集扫描相邻帧 `d > 阈值` 即帧级切换（代表链看不到、
///   相邻帧看得见），其后首个网格样本强制 changed，并把该样本的有效时刻覆写为
///   切换帧时刻——嵌字段首由此获得帧级精确起点，而非网格时刻；
/// - **(1b) 静态超时保险丝**：距最近已 OCR 网格样本超过 `stale_timeout` 秒 →
///   该样本强制 changed。渐进显示步进 ≤ 阈值时文本停在中途，超时帧读到完整
///   文本，由合并层吸收文本（时间轴不动）。`stale_timeout <= 0` 关闭该层。
///
/// 返回 `(flags, time_overrides)`：flags 与 `grid_hashes` 等长（语义同
/// `change_flags`）；time_overrides 与网格样本对齐，`Some(t)` 表示该样本
/// 因帧级差分强制变化、有效时刻应取切换帧时刻 t。
pub fn change_flags_rescued(
    grid_hashes: &[u64],
    grid_times: &[f64],
    dense: &[(f64, u64)],
    threshold: u32,
    stale_timeout: f64,
) -> (ChangeFlags, Vec<Option<f64>>) {
    let n = grid_hashes.len();

    // (1a) 帧级差分：切换点（dense 相邻 d > 阈值）映射到其后首个网格样本。
    // 多个切换点落进同一网格区间时取最早者（首个状态最接近切换时刻）。
    let mut force = vec![false; n];
    let mut override_t: Vec<Option<f64>> = vec![None; n];
    if dense.len() >= 2 {
        let mut gi = 0usize;
        for j in 1..dense.len() {
            if hamming_distance(dense[j - 1].1, dense[j].1) > threshold {
                let t = dense[j].0;
                while gi < n && grid_times[gi] < t {
                    gi += 1;
                }
                if gi < n && !force[gi] {
                    force[gi] = true;
                    override_t[gi] = Some(t);
                }
            }
        }
    }

    // 代表链 walk：常规代表比较 + 强制 + 静态超时
    let mut result = Vec::with_capacity(n);
    let mut rep: Option<u64> = None;
    let mut last_ocr_t = f64::NEG_INFINITY;
    for k in 0..n {
        let (is_changed, base_hash) = match rep {
            None => (true, None),
            Some(r) => {
                let stale = stale_timeout > 0.0 && grid_times[k] - last_ocr_t > stale_timeout;
                let changed = force[k] || stale || hamming_distance(r, grid_hashes[k]) > threshold;
                (changed, if changed { Some(r) } else { None })
            }
        };
        if is_changed {
            rep = Some(grid_hashes[k]);
            last_ocr_t = override_t[k].unwrap_or(grid_times[k]);
        }
        result.push((is_changed, base_hash));
    }
    (result, override_t)
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

    // ── change_flags_rescued（补漏，D8）──

    #[test]
    fn test_change_flags_rescued_dense_gap_injection() {
        // 场景复刻 pierro 语料 [75]：字幕切换落在两个网格样本之间，
        // 且新旧行的 dHash 距离 ≤ 阈值 → 常规代表链整句漏检。
        // A=[74]文本，T=切换后首帧，B=[75]稳定文本；
        // d(A,T)=4>3（帧级切换可检出）、d(A,B)=1、d(T,B)=3（均 ≤3，代表链看不见）。
        let a = 0x0u64;
        let t = 0xFu64;
        let b = 0x1u64;
        // dense（30fps）：0.0..0.2 为 A，0.3 起切换为 T，0.5 起稳定为 B
        let dense = [
            (0.0, a),
            (0.1, a),
            (0.2, a),
            (0.3, t),
            (0.4, t),
            (0.5, b),
            (0.6, b),
        ];
        // 网格（0.5s 相位，固定 x.0/x.5）：0.0 与 0.5 两样本都"看不到"切换
        let grid_hashes = [a, b, b];
        let grid_times = [0.0, 0.5, 1.0];
        let (flags, overrides) =
            change_flags_rescued(&grid_hashes, &grid_times, &dense, 3, 0.0);
        // 常规链会全判"未变化"（首帧除外）；补漏后 0.5 样本被强制 changed
        assert_eq!(
            flags,
            vec![(true, None), (true, Some(a)), (false, None)]
        );
        // 有效时刻覆写为切换帧时刻 0.3（帧级精确段首）
        assert_eq!(overrides, vec![None, Some(0.3), None]);
    }

    #[test]
    fn test_change_flags_rescued_stale_timeout() {
        // 静态段：样本与代表距离恒 0，超过 stale_timeout → 强制采样
        let h = 0x1234u64;
        let dense = [(0.0, h), (20.0, h)];
        let grid_hashes = [h, h, h, h, h];
        let grid_times = [0.0, 3.0, 6.0, 9.0, 12.0];
        let (flags, _) = change_flags_rescued(&grid_hashes, &grid_times, &dense, 3, 5.0);
        // k1 距上次 3s ≤5 不强制；k2 距 6s >5 强制；k3 距 3s 不强制；k4 距 6s 强制
        assert_eq!(
            flags,
            vec![
                (true, None),
                (false, None),
                (true, Some(h)),
                (false, None),
                (true, Some(h))
            ]
        );
    }

    #[test]
    fn test_change_flags_rescued_stale_disabled() {
        // stale_timeout = 0 → (1b) 关闭，静态段不产生任何强制采样
        let h = 0x1234u64;
        let dense = [(0.0, h), (20.0, h)];
        let grid_hashes = [h, h, h, h, h];
        let grid_times = [0.0, 3.0, 6.0, 9.0, 12.0];
        let (flags, _) = change_flags_rescued(&grid_hashes, &grid_times, &dense, 5, 0.0);
        assert_eq!(
            flags,
            vec![(true, None), (false, None), (false, None), (false, None), (false, None)]
        );
    }

    #[test]
    fn test_change_flags_rescued_flags_match_plain_with_holes() {
        // 无静态缺口时，补漏版的 flags 与常规版一致；
        // 但被网格正常检出的切换点同样给出时刻覆写（切换帧时刻更精确）
        // dense（0.5s）：0.0=0 → 0.5/1.0=0x3F → 1.5/2.0 起=0；
        // 网格（1.0s）哈希取各网格时刻的密集哈希：[0, 0x3F, 0, 0, 0]
        let hashes = vec![0u64, 0x3F, 0x3F, 0x0, 0x0];
        let dense: Vec<(f64, u64)> =
            (0..hashes.len()).map(|k| (k as f64 * 0.5, hashes[k])).collect();
        let grid_hashes = vec![0u64, 0x3F, 0x0, 0x0, 0x0];
        let grid_times: Vec<f64> = (0..grid_hashes.len()).map(|k| k as f64 * 1.0).collect();
        let (rescued, overrides) =
            change_flags_rescued(&grid_hashes, &grid_times, &dense, 5, 0.0);
        assert_eq!(rescued, change_flags(&grid_hashes, 5));
        // 网格 t=1.0 的变化帧其切换发生在 dense t=0.5；t=2.0 的变化帧切换在 t=1.5
        assert_eq!(overrides, vec![None, Some(0.5), Some(1.5), None, None]);
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
