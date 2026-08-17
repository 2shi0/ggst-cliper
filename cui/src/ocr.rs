use image::RgbImage;
use ort::session::Session;
use ort::value::Tensor;
use strsim::normalized_levenshtein;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::BBox;

// GGST Playable Characters Master List
pub const GGST_CHARACTERS: &[&str] = &[
    "SOL", "KY", "MAY", "AXL", "CHIPP", "POTEMKIN", "FAUST", "MILLIA", "ZATO-1",
    "RAMLETHAL", "LEO", "NAGORIYUKI", "GIOVANNA", "ANJI", "I-NO", "GOLDLEWIS",
    "JACK-O", "HAPPY CHAOS", "BAIKEN", "TESTAMENT", "BRIDGET", "SIN", "BEDMAN?",
    "ASUKA R#", "JOHNNY", "ELPHELT", "A.B.A", "SLAYER", "QUEEN DIZZY", "VENOM", "UNIKA", "LUCY",
];

// Helper: match OCR recognized text against GGST characters
pub fn match_character(raw_text: &str) -> Option<(&'static str, f64)> {
    let clean = raw_text
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '?' || *c == '#' || *c == '.')
        .collect::<String>();

    if clean.is_empty() {
        return None;
    }

    let mut best_match = None;
    let mut best_score = 0.0;

    for &char_name in GGST_CHARACTERS {
        let score = normalized_levenshtein(&clean, char_name);
        let contains_bonus = if clean.contains(char_name) || char_name.contains(&clean) {
            0.15
        } else {
            0.0
        };
        let final_score = (score + contains_bonus).min(1.0);

        if final_score > best_score {
            best_score = final_score;
            best_match = Some(char_name);
        }
    }

    if best_score >= 0.4 {
        best_match.map(|m| (m, best_score))
    } else {
        None
    }
}

// Simple contrast enhancement for GGST health bar text
pub fn enhance_contrast(img: &RgbImage) -> RgbImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }

    let mut gray = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y);
            gray[(y * w + x) as usize] = ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8;
        }
    }

    let mut min_val = 255u8;
    let mut max_val = 0u8;
    for &g in &gray {
        if g < min_val { min_val = g; }
        if g > max_val { max_val = g; }
    }

    let range = (max_val - min_val).max(1) as f32;
    let mut out = img.clone();
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let g = gray[idx];
            let stretched = (((g as f32 - min_val as f32) / range) * 255.0).clamp(0.0, 255.0) as u8;
            out.put_pixel(x, y, image::Rgb([stretched, stretched, stretched]));
        }
    }
    out
}

pub fn resolve_model_path(file_name: &str) -> Option<PathBuf> {
    // 1. Next to current executable / models/ or directly next to exe
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidate1 = exe_dir.join("models").join(file_name);
            if candidate1.exists() {
                return Some(candidate1);
            }
            let candidate2 = exe_dir.join("assets").join("models").join(file_name);
            if candidate2.exists() {
                return Some(candidate2);
            }
            let candidate3 = exe_dir.join(file_name);
            if candidate3.exists() {
                return Some(candidate3);
            }

            // Search ancestor directories (e.g. running from target/release or target/debug)
            let mut curr = exe_dir.parent();
            for _ in 0..6 {
                if let Some(dir) = curr {
                    let c1 = dir.join("assets").join("models").join(file_name);
                    if c1.exists() {
                        return Some(c1);
                    }
                    let c2 = dir.join("models").join(file_name);
                    if c2.exists() {
                        return Some(c2);
                    }
                    curr = dir.parent();
                } else {
                    break;
                }
            }
        }
    }

    // 2. assets/models/ or models/ in current working dir and its ancestors
    let asset_path = Path::new("assets").join("models").join(file_name);
    if asset_path.exists() {
        return Some(asset_path);
    }

    let local_models = Path::new("models").join(file_name);
    if local_models.exists() {
        return Some(local_models);
    }

    if let Ok(cwd) = std::env::current_dir() {
        let mut curr = cwd.parent();
        for _ in 0..6 {
            if let Some(dir) = curr {
                let c1 = dir.join("assets").join("models").join(file_name);
                if c1.exists() {
                    return Some(c1);
                }
                let c2 = dir.join("models").join(file_name);
                if c2.exists() {
                    return Some(c2);
                }
                curr = dir.parent();
            } else {
                break;
            }
        }
    }

    // 3. AppData config dir
    if let Some(base_dirs) = directories::BaseDirs::new() {
        let app_data_model = base_dirs.config_dir().join("ggst-clipper").join("models").join(file_name);
        if app_data_model.exists() {
            return Some(app_data_model);
        }
    }

    // 4. Compile-time CARGO_MANIFEST_DIR fallback
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("assets").join("models").join(file_name));
    if let Some(p) = manifest_path {
        if p.exists() {
            return Some(p);
        }
    }

    None
}

pub struct GGSTDetector {
    det_session: Session,
    rec_session: Session,
    characters: Vec<String>,
}

impl GGSTDetector {
    pub fn try_new() -> Result<Self, Box<dyn std::error::Error>> {
        let det_path = resolve_model_path("ch_PP-OCRv3_det_infer.onnx")
            .ok_or("ch_PP-OCRv3_det_infer.onnx not found")?;
        let rec_path = resolve_model_path("ch_PP-OCRv3_rec_infer.onnx")
            .ok_or("ch_PP-OCRv3_rec_infer.onnx not found")?;
        let keys_path = resolve_model_path("rec_keys.txt")
            .ok_or("rec_keys.txt not found")?;

        let det_session = Session::builder()?.commit_from_file(&det_path)?;
        let rec_session = Session::builder()?.commit_from_file(&rec_path)?;
        let keys_content = fs::read_to_string(&keys_path)?;
        let mut characters = vec!["blank".to_string()];
        for line in keys_content.lines() {
            characters.push(line.to_string());
        }
        characters.push(" ".to_string());

        Ok(Self {
            det_session,
            rec_session,
            characters,
        })
    }

    pub fn detect_and_recognize(&mut self, crop: &RgbImage) -> Result<Option<(&'static str, f32)>, Box<dyn std::error::Error>> {
        let (orig_w, orig_h) = crop.dimensions();
        if orig_w < 5 || orig_h < 5 {
            return Ok(None);
        }

        let enhanced = enhance_contrast(crop);

        let min_side = (orig_w.min(orig_h) as f32).max(1.0f32);
        let scale = (736.0f32 / min_side).max(1.0f32);
        let target_w = ((orig_w as f32 * scale / 32.0).round() as usize * 32).max(32);
        let target_h = ((orig_h as f32 * scale / 32.0).round() as usize * 32).max(32);

        let resized_for_det = image::imageops::resize(&enhanced, target_w as u32, target_h as u32, image::imageops::FilterType::CatmullRom);

        let mean = [0.485f32, 0.456f32, 0.406f32];
        let std = [0.229f32, 0.224f32, 0.225f32];

        let mut det_data = vec![0f32; 1 * 3 * target_h * target_w];
        for y in 0..target_h {
            for x in 0..target_w {
                let p = resized_for_det.get_pixel(x as u32, y as u32);
                for c in 0..3 {
                    let val = (p[c] as f32 / 255.0 - mean[c]) / std[c];
                    let idx = c * (target_h * target_w) + y * target_w + x;
                    det_data[idx] = val;
                }
            }
        }

        let (out_h, out_w, prob_vec) = {
            let input_tensor = Tensor::from_array((vec![1, 3, target_h, target_w], det_data))?;
            let outputs = self.det_session.run(ort::inputs!["x" => input_tensor])?;
            let (shape, prob_slice) = outputs[0].try_extract_tensor::<f32>()?;
            (shape[2] as usize, shape[3] as usize, prob_slice.to_vec())
        };

        let scale_x = orig_w as f32 / out_w as f32;
        let scale_y = orig_h as f32 / out_h as f32;

        let mut visited = vec![false; out_h * out_w];
        let mut text_boxes = Vec::new();

        for y in 0..out_h {
            for x in 0..out_w {
                let idx = y * out_w + x;
                if prob_vec[idx] > 0.25 && !visited[idx] {
                    let mut q = vec![(x, y)];
                    visited[idx] = true;
                    let mut min_x = x;
                    let mut max_x = x;
                    let mut min_y = y;
                    let mut max_y = y;
                    let mut sum_prob = 0.0f32;
                    let mut count = 0;

                    while let Some((cx, cy)) = q.pop() {
                        let c_idx = cy * out_w + cx;
                        sum_prob += prob_vec[c_idx];
                        count += 1;

                        if cx < min_x { min_x = cx; }
                        if cx > max_x { max_x = cx; }
                        if cy < min_y { min_y = cy; }
                        if cy > max_y { max_y = cy; }

                        let neighbors = [
                            (cx.wrapping_sub(1), cy),
                            (cx + 1, cy),
                            (cx, cy.wrapping_sub(1)),
                            (cx, cy + 1),
                        ];

                        for (nx, ny) in neighbors {
                            if nx < out_w && ny < out_h {
                                let nidx = ny * out_w + nx;
                                if !visited[nidx] && prob_vec[nidx] > 0.25 {
                                    visited[nidx] = true;
                                    q.push((nx, ny));
                                }
                            }
                        }
                    }

                    let avg_prob = sum_prob / count as f32;
                    let bw = (max_x - min_x + 1) as f32;
                    let bh = (max_y - min_y + 1) as f32;

                    if count >= 8 && avg_prob >= 0.35 && bw >= 8.0 && bh >= 6.0 {
                        let pad_x = bw * 0.15 + 4.0;
                        let pad_y = bh * 0.15 + 2.0;

                        let rx = (((min_x as f32 - pad_x).max(0.0)) * scale_x).round() as u32;
                        let ry = (((min_y as f32 - pad_y).max(0.0)) * scale_y).round() as u32;
                        let rx2 = (((max_x as f32 + pad_x).min(out_w as f32 - 1.0)) * scale_x).round() as u32;
                        let ry2 = (((max_y as f32 + pad_y).min(out_h as f32 - 1.0)) * scale_y).round() as u32;

                        let rw = (rx2 - rx).min(orig_w - rx);
                        let rh = (ry2 - ry).min(orig_h - ry);

                        if rw >= 10 && rh >= 8 {
                            text_boxes.push((rx, ry, rw, rh));
                        }
                    }
                }
            }
        }

        let mut best_char_match = None;
        let mut highest_score = 0.0;

        for (rx, ry, rw, rh) in text_boxes {
            let text_crop = image::imageops::crop_imm(&enhanced, rx, ry, rw, rh).to_image();
            if let Ok((text, conf)) = self.recognize_single(&text_crop) {
                if let Some((char_name, score)) = match_character(&text) {
                    if score > highest_score {
                        highest_score = score;
                        best_char_match = Some((char_name, conf));
                    }
                }
            }
        }

        // Fallback: If DBNet didn't find any box or low confidence, try whole crop
        if best_char_match.is_none() {
            if let Ok((text, conf)) = self.recognize_single(&enhanced) {
                if let Some((char_name, score)) = match_character(&text) {
                    if score >= 0.45 {
                        best_char_match = Some((char_name, conf));
                    }
                }
            }
        }

        Ok(best_char_match)
    }

    fn recognize_single(&mut self, crop: &RgbImage) -> Result<(String, f32), Box<dyn std::error::Error>> {
        let (orig_w, orig_h) = crop.dimensions();
        if orig_w == 0 || orig_h == 0 {
            return Ok((String::new(), 0.0));
        }

        let target_h = 48usize;
        let ratio = orig_w as f32 / orig_h as f32;
        let target_w = ((target_h as f32 * ratio).ceil() as usize).max(1);

        let resized = image::imageops::resize(crop, target_w as u32, target_h as u32, image::imageops::FilterType::CatmullRom);

        let mut data = vec![0f32; 1 * 3 * target_h * target_w];
        for y in 0..target_h {
            for x in 0..target_w {
                let p = resized.get_pixel(x as u32, y as u32);
                for c in 0..3 {
                    let val = (p[c] as f32 / 255.0 - 0.5) / 0.5;
                    let idx = c * (target_h * target_w) + y * target_w + x;
                    data[idx] = val;
                }
            }
        }

        let input_tensor = Tensor::from_array((vec![1, 3, target_h, target_w], data))?;
        let outputs = self.rec_session.run(ort::inputs!["x" => input_tensor])?;
        
        let (shape, slice) = outputs[0].try_extract_tensor::<f32>()?;
        let seq_len = shape[1] as usize;
        let num_classes = shape[2] as usize;

        let mut recognized_chars = Vec::new();
        let mut confidences = Vec::new();
        let mut prev_idx = 0;

        for t in 0..seq_len {
            let offset = t * num_classes;
            let mut max_idx = 0;
            let mut max_val = f32::NEG_INFINITY;
            for c in 0..num_classes {
                let v = slice[offset + c];
                if v > max_val {
                    max_val = v;
                    max_idx = c;
                }
            }

            if max_idx != 0 && max_idx != prev_idx {
                if max_idx < self.characters.len() {
                    recognized_chars.push(self.characters[max_idx].clone());
                    confidences.push(max_val);
                }
            }
            prev_idx = max_idx;
        }

        let text = recognized_chars.join("");
        let avg_conf = if !confidences.is_empty() {
            confidences.iter().sum::<f32>() / confidences.len() as f32
        } else {
            0.0
        };

        Ok((text, avg_conf))
    }
}

// Calculate default 1P and 2P ROIs based on video dimensions
pub fn get_default_character_rois(video_width: u32, video_height: u32) -> (BBox, BBox) {
    let vw = video_width as f32;
    let vh = video_height as f32;

    // 1P: ~6.54% x, 9.20% y, 22.07% w, 5.90% h
    let p1_x = (vw * 0.0654).round() as u32;
    let p1_y = (vh * 0.0920).round() as u32;
    let p1_w = (vw * 0.2207).round() as u32;
    let p1_h = (vh * 0.0590).round() as u32;

    // 2P: ~66.21% x, 8.33% y, 27.34% w, 6.77% h
    let p2_x = (vw * 0.6621).round() as u32;
    let p2_y = (vh * 0.0833).round() as u32;
    let p2_w = (vw * 0.2734).round() as u32;
    let p2_h = (vh * 0.0677).round() as u32;

    (
        BBox::new(p1_x, p1_y, p1_w, p1_h),
        BBox::new(p2_x, p2_y, p2_w, p2_h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_model_paths() {
        assert!(resolve_model_path("ch_PP-OCRv3_det_infer.onnx").is_some());
        assert!(resolve_model_path("ch_PP-OCRv3_rec_infer.onnx").is_some());
        assert!(resolve_model_path("rec_keys.txt").is_some());
    }

    #[test]
    fn test_detector_initialization() {
        let detector = GGSTDetector::try_new();
        assert!(detector.is_ok(), "Detector failed to initialize: {:?}", detector.err());
    }
}

