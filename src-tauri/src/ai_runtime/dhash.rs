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
        let is_changed = match last_ocr_hash {
            None => true,
            Some(prev) => hamming_distance(prev, hash) > threshold,
        };
        if is_changed {
            last_ocr_hash = Some(hash);
        }
        result.push(FrameChange {
            frame: frame.clone(),
            is_changed,
        });
    }
    Ok(result)
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
}
