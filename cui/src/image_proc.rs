use image::DynamicImage;
use crate::types::BBox;

pub fn extract_roi(buffer: &[u8], buf_w: u32, buf_h: u32, bbox: BBox) -> Vec<u8> {
    let mut roi = Vec::with_capacity((bbox.width * bbox.height * 3) as usize);

    for j in 0..bbox.height {
        let row_y = bbox.y + j;
        if row_y >= buf_h {
            roi.extend(std::iter::repeat_n(0, (bbox.width * 3) as usize));
            continue;
        }
        for i in 0..bbox.width {
            let col_x = bbox.x + i;
            if col_x >= buf_w {
                roi.extend_from_slice(&[0, 0, 0]);
            } else {
                let idx = ((row_y * buf_w + col_x) * 3) as usize;
                roi.extend_from_slice(&buffer[idx..idx + 3]);
            }
        }
    }
    roi
}

pub struct TemplateStats {
    pub sum_b: f64,
    pub den_b: f64,
}

pub fn compute_template_stats(template: &[u8]) -> TemplateStats {
    let mut sum_b = 0.0;
    let mut sum_b_sq = 0.0;
    for &val in template {
        let b = val as f64;
        sum_b += b;
        sum_b_sq += b * b;
    }
    let n = template.len() as f64;
    TemplateStats {
        sum_b,
        den_b: n * sum_b_sq - sum_b * sum_b,
    }
}

pub fn compute_similarity_zero_copy(
    buffer: &[u8],
    buf_w: u32,
    buf_h: u32,
    bbox: BBox,
    template: &[u8],
    stats: &TemplateStats,
) -> f64 {
    let roi = extract_roi(buffer, buf_w, buf_h, bbox);
    let n = template.len() as f64;

    let mut sum_a = 0.0;
    let mut sum_a_sq = 0.0;
    let mut sum_ab = 0.0;

    for i in 0..template.len() {
        let a = roi[i] as f64;
        let b = template[i] as f64;
        sum_a += a;
        sum_a_sq += a * a;
        sum_ab += a * b;
    }

    let num = n * sum_ab - sum_a * stats.sum_b;
    let den_a = n * sum_a_sq - sum_a * sum_a;

    if den_a <= 0.0 || stats.den_b <= 0.0 {
        return 0.0;
    }

    num / (den_a.sqrt() * stats.den_b.sqrt())
}

/// Scales the template ROI and resizes the cropped template image to match the target video resolution.
/// Returns `(scaled_bbox, template_rgb_bytes, template_stats)`.
pub fn scale_and_crop_template(
    img: &DynamicImage,
    roi: BBox,
    target_width: u32,
    target_height: u32,
) -> (BBox, Vec<u8>, TemplateStats) {
    let img_w = img.width();
    let img_h = img.height();

    // 1. Clamp ROI to original image bounds
    let clamp_x = roi.x.min(img_w.saturating_sub(1));
    let clamp_y = roi.y.min(img_h.saturating_sub(1));
    let clamp_w = roi.width.min(img_w.saturating_sub(clamp_x)).max(1);
    let clamp_h = roi.height.min(img_h.saturating_sub(clamp_y)).max(1);

    if (img_w == target_width && img_h == target_height) || target_width == 0 || target_height == 0 {
        let rgb = img.to_rgb8();
        let raw = extract_roi(rgb.as_raw(), img_w, img_h, BBox::new(clamp_x, clamp_y, clamp_w, clamp_h));
        let stats = compute_template_stats(&raw);
        (BBox::new(clamp_x, clamp_y, clamp_w, clamp_h), raw, stats)
    } else {
        let scale_x = target_width as f64 / img_w as f64;
        let scale_y = target_height as f64 / img_h as f64;

        let scaled_x = ((clamp_x as f64) * scale_x).round() as u32;
        let scaled_y = ((clamp_y as f64) * scale_y).round() as u32;
        let scaled_w = ((clamp_w as f64) * scale_x).round().max(1.0) as u32;
        let scaled_h = ((clamp_h as f64) * scale_y).round().max(1.0) as u32;

        let final_x = scaled_x.min(target_width.saturating_sub(1));
        let final_y = scaled_y.min(target_height.saturating_sub(1));
        let final_w = scaled_w.min(target_width.saturating_sub(final_x)).max(1);
        let final_h = scaled_h.min(target_height.saturating_sub(final_y)).max(1);
        let final_bbox = BBox::new(final_x, final_y, final_w, final_h);

        let rgb = img.to_rgb8();
        let cropped = image::imageops::crop_imm(&rgb, clamp_x, clamp_y, clamp_w, clamp_h).to_image();
        let resized = image::imageops::resize(&cropped, final_w, final_h, image::imageops::FilterType::CatmullRom);
        let raw = resized.into_raw();
        let stats = compute_template_stats(&raw);

        (final_bbox, raw, stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[test]
    fn test_scale_and_crop_template_same_res() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(100, 100));
        let roi = BBox::new(10, 20, 30, 40);
        let (scaled_bbox, raw, _stats) = scale_and_crop_template(&img, roi, 100, 100);

        assert_eq!(scaled_bbox, BBox::new(10, 20, 30, 40));
        assert_eq!(raw.len(), (30 * 40 * 3) as usize);
    }

    #[test]
    fn test_scale_and_crop_template_upscale() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(1920, 1080));
        let roi = BBox::new(100, 200, 300, 400);
        // 4K Target (2x)
        let (scaled_bbox, raw, _stats) = scale_and_crop_template(&img, roi, 3840, 2160);

        assert_eq!(scaled_bbox, BBox::new(200, 400, 600, 800));
        assert_eq!(raw.len(), (600 * 800 * 3) as usize);
    }

    #[test]
    fn test_scale_and_crop_template_downscale() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(1920, 1080));
        let roi = BBox::new(192, 108, 384, 216);
        // 720p Target (2/3x)
        let (scaled_bbox, raw, _stats) = scale_and_crop_template(&img, roi, 1280, 720);

        assert_eq!(scaled_bbox, BBox::new(128, 72, 256, 144));
        assert_eq!(raw.len(), (256 * 144 * 3) as usize);
    }
}

