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
