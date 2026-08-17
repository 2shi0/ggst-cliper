use image::RgbImage;
use ort::session::Session;
use ort::value::Tensor;
use strsim::normalized_levenshtein;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::BBox;

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterRule {
    pub canonical_name: String,
    pub aliases: Vec<String>,
}

pub fn parse_character_rules(content: &str) -> Vec<CharacterRule> {
    let mut rules = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }

        // Split by ':' or '='
        let (canonical_part, aliases_part) = if let Some(idx) = line.find(':') {
            (&line[..idx], Some(&line[idx + 1..]))
        } else if let Some(idx) = line.find('=') {
            (&line[..idx], Some(&line[idx + 1..]))
        } else {
            (line, None)
        };

        let canonical_name = canonical_part.trim().to_string();
        if canonical_name.is_empty() {
            continue;
        }

        let mut aliases = Vec::new();
        if let Some(alias_str) = aliases_part {
            for a in alias_str.split(',') {
                let a = a.trim();
                if !a.is_empty() {
                    aliases.push(a.to_string());
                }
            }
        }

        // If no aliases provided, use canonical_name itself as alias
        if aliases.is_empty() {
            aliases.push(canonical_name.clone());
        }

        rules.push(CharacterRule {
            canonical_name,
            aliases,
        });
    }
    rules
}

// Helper: match OCR recognized text against GGST characters using rules
pub fn match_character(raw_text: &str, character_rules: &[CharacterRule]) -> Option<(String, f64)> {
    let clean = raw_text
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '?' || *c == '#' || *c == '.')
        .collect::<String>();

    if clean.is_empty() {
        return None;
    }

    // 1-2文字の短いノイズは、完全一致（例: "KY"）以外は無視
    if clean.len() <= 2 {
        for rule in character_rules {
            for alias in &rule.aliases {
                let clean_alias = alias
                    .to_uppercase()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '?' || *c == '#' || *c == '.')
                    .collect::<String>();
                if clean == clean_alias {
                    return Some((rule.canonical_name.clone(), 1.0));
                }
            }
        }
        return None;
    }

    let mut best_match = None;
    let mut best_score = 0.0;
    let mut best_target_len = 0;

    for rule in character_rules {
        for alias in &rule.aliases {
            let clean_alias = alias
                .to_uppercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '?' || *c == '#' || *c == '.')
                .collect::<String>();

            if clean_alias.is_empty() {
                continue;
            }

            let score = normalized_levenshtein(&clean, &clean_alias);
            let contains_bonus = if clean.len() >= 3 && clean_alias.len() >= 3 {
                if clean.contains(&clean_alias) || clean_alias.contains(&clean) {
                    let min_l = clean.len().min(clean_alias.len());
                    let max_l = clean.len().max(clean_alias.len());
                    let ratio = min_l as f64 / max_l as f64;
                    if ratio >= 0.5 {
                        0.25
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let final_score = (score + contains_bonus).min(1.0);
            if final_score > best_score {
                best_score = final_score;
                best_target_len = clean_alias.len();
                best_match = Some(rule.canonical_name.clone());
            }
        }
    }

    // Strict threshold: short names (<=3 chars) require score >= 0.75 to prevent 1-char noise matches (e.g. "WIN" -> "SIN")
    let required_threshold = if best_target_len <= 3 {
        0.75
    } else {
        0.55
    };

    if best_score >= required_threshold {
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
        let app_data_dir = base_dirs.config_dir().join("ggst-clipper");
        let app_data_direct = app_data_dir.join(file_name);
        if app_data_direct.exists() {
            return Some(app_data_direct);
        }
        let app_data_model = app_data_dir.join("models").join(file_name);
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
    pub character_rules: Vec<CharacterRule>,
}

impl GGSTDetector {
    pub fn try_new() -> Result<Self, Box<dyn std::error::Error>> {
        let det_path = resolve_model_path("ch_PP-OCRv3_det_infer.onnx")
            .ok_or("ch_PP-OCRv3_det_infer.onnx not found")?;
        let rec_path = resolve_model_path("en_PP-OCRv3_rec_infer.onnx")
            .ok_or("en_PP-OCRv3_rec_infer.onnx not found")?;
        let keys_path = resolve_model_path("en_dict.txt")
            .ok_or("en_dict.txt not found")?;

        let det_session = Session::builder()?.commit_from_file(&det_path)?;
        let rec_session = Session::builder()?.commit_from_file(&rec_path)?;
        let keys_content = fs::read_to_string(&keys_path)?;
        let mut characters = vec!["blank".to_string()];
        for line in keys_content.lines() {
            characters.push(line.to_string());
        }
        characters.push(" ".to_string());

        let character_rules = Self::load_character_rules();

        Ok(Self {
            det_session,
            rec_session,
            characters,
            character_rules,
        })
    }

    pub fn load_character_rules() -> Vec<CharacterRule> {
        // 1. AppData/Roaming/ggst-clipper/characters.txt
        if let Some(base_dirs) = directories::BaseDirs::new() {
            let app_data_char = base_dirs.config_dir().join("ggst-clipper").join("characters.txt");
            if app_data_char.exists() {
                if let Ok(content) = fs::read_to_string(&app_data_char) {
                    let rules = parse_character_rules(&content);
                    if !rules.is_empty() {
                        return rules;
                    }
                }
            }
        }

        // 2. resolve_model_path
        if let Some(char_path) = resolve_model_path("characters.txt") {
            if let Ok(content) = fs::read_to_string(&char_path) {
                let rules = parse_character_rules(&content);
                if !rules.is_empty() {
                    return rules;
                }
            }
        }

        // 3. Default embedded list
        Self::default_character_rules()
    }

    pub fn default_character_rules() -> Vec<CharacterRule> {
        parse_character_rules(include_str!("../../assets/models/characters.txt"))
    }

    pub fn detect_and_recognize(&mut self, crop: &RgbImage) -> Result<Option<(String, f32)>, Box<dyn std::error::Error>> {
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
                if visited[idx] || prob_vec[idx] < 0.25 {
                    continue;
                }

                // Flood fill to find connected text component
                let mut min_x = x;
                let mut max_x = x;
                let mut min_y = y;
                let mut max_y = y;
                let mut queue = std::collections::VecDeque::new();

                queue.push_back((x, y));
                visited[idx] = true;

                while let Some((cx, cy)) = queue.pop_front() {
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;

                        if nx >= 0 && nx < out_w as isize && ny >= 0 && ny < out_h as isize {
                            let n_idx = ny as usize * out_w + nx as usize;
                            if !visited[n_idx] && prob_vec[n_idx] >= 0.25 {
                                visited[n_idx] = true;
                                queue.push_back((nx as usize, ny as usize));
                            }
                        }
                    }
                }

                let bw = (max_x - min_x + 1) as f32 * scale_x;
                let bh = (max_y - min_y + 1) as f32 * scale_y;
                let bx = (min_x as f32 * scale_x).floor().max(0.0) as u32;
                let by = (min_y as f32 * scale_y).floor().max(0.0) as u32;

                // Expand box slightly to cover full character strokes
                let pad_x = (bw * 0.15).max(3.0) as u32;
                let pad_y = (bh * 0.15).max(3.0) as u32;

                let exp_x = bx.saturating_sub(pad_x);
                let exp_y = by.saturating_sub(pad_y);
                let exp_w = ((bw as u32 + pad_x * 2).min(orig_w - exp_x)).max(1);
                let exp_h = ((bh as u32 + pad_y * 2).min(orig_h - exp_y)).max(1);

                if exp_w >= 10 && exp_h >= 6 {
                    text_boxes.push((exp_x, exp_y, exp_w, exp_h));
                }
            }
        }

        // Horizontal box merging: combine adjacent character fragments on the same line
        text_boxes.sort_by_key(|b| b.0); // sort left to right
        let mut merged_boxes: Vec<(u32, u32, u32, u32)> = Vec::new();
        for b in text_boxes {
            if let Some(last) = merged_boxes.last_mut() {
                let last_right = last.0 + last.2;
                // Merge if horizontal gap is small (<= 25px) and vertical overlap is large
                if b.0 <= last_right + 25 {
                    let new_right = (b.0 + b.2).max(last_right);
                    let new_top = last.1.min(b.1);
                    let new_bottom = (last.1 + last.3).max(b.1 + b.3);
                    last.0 = last.0.min(b.0);
                    last.1 = new_top;
                    last.2 = new_right - last.0;
                    last.3 = new_bottom - new_top;
                    continue;
                }
            }
            merged_boxes.push(b);
        }

        let mut best_char_match = None;
        let mut highest_score = 0.0;

        for (rx, ry, rw, rh) in &merged_boxes {
            // Try both enhanced and raw crop for the box
            let text_crop_enh = image::imageops::crop_imm(&enhanced, *rx, *ry, *rw, *rh).to_image();
            let text_crop_raw = image::imageops::crop_imm(crop, *rx, *ry, *rw, *rh).to_image();

            for (t_crop, _tag) in [(&text_crop_enh, "enhanced"), (&text_crop_raw, "raw")] {
                if let Ok((text, conf)) = self.recognize_single(t_crop) {
                    if let Some((char_name, score)) = match_character(&text, &self.character_rules) {
                        if score > highest_score {
                            highest_score = score;
                            best_char_match = Some((char_name, conf));
                        }
                    }
                }
            }
        }

        // Fallback: If DBNet didn't find any box or low confidence, try whole crop
        if best_char_match.is_none() {
            for t_crop in [&enhanced, crop] {
                if let Ok((text, conf)) = self.recognize_single(t_crop) {
                    if let Some((char_name, score)) = match_character(&text, &self.character_rules) {
                        if score >= 0.55 && score > highest_score {
                            highest_score = score;
                            best_char_match = Some((char_name, conf));
                        }
                    }
                }
            }
        }

        Ok(best_char_match)
    }

    pub fn recognize_single(&mut self, crop: &RgbImage) -> Result<(String, f32), Box<dyn std::error::Error>> {
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
                    data[c * (target_h * target_w) + y * target_w + x] = (p[c] as f32 / 255.0 - 0.5) / 0.5;
                }
            }
        }

        let input_tensor = Tensor::from_array((vec![1, 3, target_h, target_w], data))?;
        let outputs = self.rec_session.run(ort::inputs!["x" => input_tensor])?;
        let (shape, prob_slice) = outputs[0].try_extract_tensor::<f32>()?;

        let seq_len = shape[1] as usize;
        let num_classes = shape[2] as usize;

        let mut recognized_chars = Vec::new();
        let mut last_idx = 0;
        let mut total_prob = 0.0;
        let mut count = 0;

        for t in 0..seq_len {
            let row_offset = t * num_classes;
            let mut max_idx = 0;
            let mut max_prob = -f32::INFINITY;

            for c in 0..num_classes {
                let val = prob_slice[row_offset + c];
                if val > max_prob {
                    max_prob = val;
                    max_idx = c;
                }
            }

            if max_idx != 0 && max_idx != last_idx {
                if max_idx < self.characters.len() {
                    recognized_chars.push(self.characters[max_idx].clone());
                    total_prob += max_prob;
                    count += 1;
                }
            }
            last_idx = max_idx;
        }

        let text = recognized_chars.join("");
        let avg_conf = if count > 0 { total_prob / count as f32 } else { 0.0 };

        Ok((text, avg_conf))
    }
}

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
        assert!(resolve_model_path("en_PP-OCRv3_rec_infer.onnx").is_some());
        assert!(resolve_model_path("en_dict.txt").is_some());
    }

    #[test]
    fn test_detector_initialization() {
        let detector = GGSTDetector::try_new();
        assert!(detector.is_ok(), "Detector failed to initialize: {:?}", detector.err());
    }

    #[test]
    fn test_parse_character_rules() {
        let sample = "
            # Comment line
            // Another comment
            SOL: SOL BADGUY, SOL
            KY = KY KISKE, KYKISKE, KY
            MAY
            ZATO-1: ZATO-1, ZATO=1
        ";
        let rules = parse_character_rules(sample);
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[0].canonical_name, "SOL");
        assert_eq!(rules[0].aliases, vec!["SOL BADGUY", "SOL"]);
        assert_eq!(rules[1].canonical_name, "KY");
        assert_eq!(rules[1].aliases, vec!["KY KISKE", "KYKISKE", "KY"]);
        assert_eq!(rules[2].canonical_name, "MAY");
        assert_eq!(rules[2].aliases, vec!["MAY"]);
        assert_eq!(rules[3].canonical_name, "ZATO-1");
        assert_eq!(rules[3].aliases, vec!["ZATO-1", "ZATO=1"]);
    }

    #[test]
    fn test_load_character_rules() {
        let rules = GGSTDetector::load_character_rules();
        assert!(!rules.is_empty(), "Character rules list should not be empty");
        assert!(rules.iter().any(|r| r.canonical_name == "SOL"));
        assert!(rules.iter().any(|r| r.canonical_name == "KY"));
    }

    #[test]
    fn test_match_character_logic() {
        let rules = GGSTDetector::load_character_rules();

        // Exact match
        assert_eq!(match_character("POTEMKIN", &rules).map(|(n, _)| n), Some("POTEMKIN".to_string()));
        assert_eq!(match_character("BEDMAN?", &rules).map(|(n, _)| n), Some("BEDMAN?".to_string()));

        // In-game full name variants
        assert_eq!(match_character("KY KISKE", &rules).map(|(n, _)| n), Some("KY".to_string()));
        assert_eq!(match_character("KYKISKE", &rules).map(|(n, _)| n), Some("KY".to_string()));
        assert_eq!(match_character("SOL BADGUY", &rules).map(|(n, _)| n), Some("SOL".to_string()));
        assert_eq!(match_character("CHIPP ZANUFF", &rules).map(|(n, _)| n), Some("CHIPP".to_string()));
        assert_eq!(match_character("LPOTEMKIN", &rules).map(|(n, _)| n), Some("POTEMKIN".to_string()));
        assert_eq!(match_character("JAM KURADOBERI", &rules).map(|(n, _)| n), Some("JAM".to_string()));
        assert_eq!(match_character("ROBO-KY", &rules).map(|(n, _)| n), Some("ROBO-KY".to_string()));
        assert_eq!(match_character("ROBOKY", &rules).map(|(n, _)| n), Some("ROBO-KY".to_string()));

        // Short noise / false positives should be rejected
        assert_eq!(match_character("WIN", &rules), None);
        assert_eq!(match_character("MIN", &rules), None);
        assert_eq!(match_character("IN", &rules), None);
        assert_eq!(match_character("A", &rules), None);
        assert_eq!(match_character("Y", &rules), None);
        assert_eq!(match_character("O", &rules), None);
        assert_eq!(match_character("TA", &rules), None);
        assert_eq!(match_character("1", &rules), None);
    }

    #[test]
    fn test_custom_character_rule() {
        let custom_rules = parse_character_rules("NEW_FIGHTER: THE NEW FIGHTER, NEWFIGHTER");
        assert_eq!(match_character("THE NEW FIGHTER", &custom_rules).map(|(n, _)| n), Some("NEW_FIGHTER".to_string()));
        assert_eq!(match_character("NEWFIGHTER", &custom_rules).map(|(n, _)| n), Some("NEW_FIGHTER".to_string()));
    }
}
