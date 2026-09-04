// ─── dHash 感知哈希 ───────────────────────────────────────
// 用于 OCR 前的变化检测：对裁切后的帧算 64 bit dHash，
// 相邻（与最近一次已 OCR 的帧）汉明距离 ≤ 阈值 → 判定"未变化"，跳过 OCR。
//
// dHash：灰度 → 缩放到 9x8 → 每像素与右侧像素比较（左>右记1）→ 64 bit。

use crate::ai_runtime::OcrError;
use image::DynamicImage;
use std::path::Path;

/// 计算一张已解码图像的 dHash（64 bit）
pub fn dhash_image(img: &DynamicImage) -> u64 {
    // 灰度 + 缩放到 9x8（标准 dHash 尺寸）
    let small = img.grayscale().resize_exact(9, 8, image::imageops::FilterType::Triangle);
    let gray = small.to_luma8();
    let mut hash: u64 = 0;
    let mut bit = 0u8;
    for y in 0..8 {
        for x in 0..8 {
            let left = gray.get_pixel(x, y).0[0];
            let right = gray.get_pixel(x + 1, y).0[0];
            if left > right {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }
    hash
}

/// 读取图像文件并计算 dHash
pub fn dhash_file(path: &Path) -> Result<u64, OcrError> {
    let img = image::open(path).map_err(|e| OcrError::Io(std::io::Error::other(e.to_string())))?;
    Ok(dhash_image(&img))
}

/// 两个 64 bit 哈希的汉明距离（相异位数）
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 变化检测结果：一帧是否需要 OCR
#[derive(Debug, Clone)]
pub struct FrameChange {
    pub frame: crate::video::ExtractedFrame,
    /// true → 相对最近一次已 OCR 的帧有明显变化，需要 OCR
    pub is_changed: bool,
    /// changed 帧"与之不同的上一段代表哈希"（窗口精化的基准 A；非 changed 帧为 None）
    pub base_hash: Option<u64>,
}

/// 对帧序列做变化检测。
///
/// 第一帧恒为 changed；后续帧与"最近一次 changed 的帧"比较，
/// 汉明距离 ≤ 阈值视为未变化（可跳过 OCR）。
pub fn detect_changes(
    frames: &[crate::video::ExtractedFrame],
    threshold: u32,
) -> Result<Vec<FrameChange>, OcrError> {
    let mut result = Vec::with_capacity(frames.len());
    let mut last_ocr_hash: Option<u64> = None;

    for frame in frames {
        let hash = dhash_file(&frame.path)?;
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
        result.push(FrameChange {
            frame: frame.clone(),
            is_changed,
            base_hash,
        });
    }
    Ok(result)
}

/// 窗口精化的结果
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRefine {
    /// 第一个与基准 A 距离 > 阈值（内容已明显变化）的窗口帧下标
    pub main_boundary: Option<usize>,
    /// main_boundary 之后内容又相对当前代表发生变化的帧下标（多突变/短字幕，阶段 2 用）
    pub sub_changes: Vec<usize>,
}

/// 在窗口密帧哈希序列上定位帧级变化边界（阶段 1：帧级打轴精度）。
///
/// 判定**相对基准 A**（上一段代表哈希）而非相邻帧 —— 渐变累计超过阈值即命中，
/// 不会出现"相邻帧差异过小而永远检测不到"的情况。
/// `sub_changes` 记录主边界后又发生明显变化的下标（窗口内多突变/短字幕，供阶段 2 召回）。
pub fn refine_window_hashes(hashes: &[u64], base_hash: u64, threshold: u32) -> WindowRefine {
    let mut main_boundary = None;
    let mut sub_changes = Vec::new();
    // 当前段代表：窗口起点应为 A（与 base 相似），主边界后切换为新内容
    let mut current = base_hash;

    for (i, &h) in hashes.iter().enumerate() {
        if hamming_distance(current, h) > threshold {
            if main_boundary.is_none() {
                main_boundary = Some(i);
            } else {
                sub_changes.push(i);
            }
            current = h;
        }
    }
    WindowRefine {
        main_boundary,
        sub_changes,
    }
}

// ─── 单元测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("gsa_{}_{}", name, std::process::id()))
    }

    fn solid_rgb(color: [u8; 3], w: u32, h: u32) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(color))
    }

    /// 左/右各半不同亮度的双色图（产生 dHash 差异）
    fn two_tone(left: [u8; 3], right: [u8; 3], w: u32, h: u32) -> RgbImage {
        let mut img = RgbImage::new(w, h);
        for (x, _, px) in img.enumerate_pixels_mut() {
            let c = if x < w / 2 { left } else { right };
            *px = Rgb(c);
        }
        img
    }

    #[test]
    fn test_dhash_identical_images_equal() {
        let img = DynamicImage::ImageRgb8(solid_rgb([100, 100, 100], 32, 32));
        assert_eq!(dhash_image(&img), dhash_image(&img));
    }

    #[test]
    fn test_dhash_different_images_differ() {
        // 左亮右暗 vs 左暗右亮 → 亮度对比方向相反 → dHash 不同
        let a = DynamicImage::ImageRgb8(two_tone([200, 200, 200], [20, 20, 20], 32, 32));
        let b = DynamicImage::ImageRgb8(two_tone([20, 20, 20], [200, 200, 200], 32, 32));
        assert_ne!(dhash_image(&a), dhash_image(&b));
    }

    #[test]
    fn test_hamming_distance() {
        assert_eq!(hamming_distance(0b0000, 0b0000), 0);
        assert_eq!(hamming_distance(0b1010, 0b0000), 2);
        assert_eq!(hamming_distance(0b1111, 0b0000), 4);
        assert_eq!(hamming_distance(u64::MAX, 0), 64);
    }

    #[test]
    fn test_detect_changes_first_always_changed() {
        let dir = temp_dir("dc_first");
        fs::create_dir_all(&dir).unwrap();
        let img = solid_rgb([80, 80, 80], 64, 20);
        let p = dir.join("f.jpg");
        img.save(&p).unwrap();

        let frames = vec![crate::video::ExtractedFrame {
            path: p.clone(),
            time: 0.0,
        }];
        let changes = detect_changes(&frames, 5).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].is_changed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_changes_identical_second_unchanged() {
        let dir = temp_dir("dc_same");
        fs::create_dir_all(&dir).unwrap();
        let img = solid_rgb([80, 80, 80], 64, 20);
        let p1 = dir.join("f1.jpg");
        let p2 = dir.join("f2.jpg");
        img.save(&p1).unwrap();
        img.save(&p2).unwrap();

        let frames = vec![
            crate::video::ExtractedFrame { path: p1, time: 0.0 },
            crate::video::ExtractedFrame { path: p2, time: 1.0 },
        ];
        let changes = detect_changes(&frames, 5).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(changes[0].is_changed);
        assert!(!changes[1].is_changed); // 与第一帧相同 → 未变化
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_changes_different_second_changed() {
        let dir = temp_dir("dc_diff");
        fs::create_dir_all(&dir).unwrap();
        let dark_l = two_tone([20, 20, 20], [200, 200, 200], 64, 20);
        let light_l = two_tone([200, 200, 200], [20, 20, 20], 64, 20);
        let p1 = dir.join("f1.jpg");
        let p2 = dir.join("f2.jpg");
        dark_l.save(&p1).unwrap();
        light_l.save(&p2).unwrap();

        let frames = vec![
            crate::video::ExtractedFrame { path: p1, time: 0.0 },
            crate::video::ExtractedFrame { path: p2, time: 1.0 },
        ];
        let changes = detect_changes(&frames, 5).unwrap();
        assert!(changes[0].is_changed);
        assert!(changes[1].is_changed);
        let _ = fs::remove_dir_all(&dir);
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
}
