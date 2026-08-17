use ini::Ini;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub start_template: String,
    pub end_template: String,
    pub win_template: String,
    pub lose_template: String,
    pub detect_win_loss: bool,
    pub detect_characters: bool,
    pub my_character: String,
    pub threshold: f32,
    pub step_frames: u32,
    pub start_offset: i32,
    pub end_offset: i32,
    pub win_offset: u32,
    pub output_dir: String,
    pub start_roi: [u32; 4],
    pub end_roi: [u32; 4],
    pub win_roi: [u32; 4],
    pub lose_roi: [u32; 4],
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            start_template: "start.png".to_string(),
            end_template: "end.png".to_string(),
            win_template: "win.png".to_string(),
            lose_template: "lose.png".to_string(),
            detect_win_loss: true,
            detect_characters: true,
            my_character: "None".to_string(),
            threshold: 0.9,
            step_frames: 60,
            start_offset: 0,
            end_offset: -120,
            win_offset: 180,
            output_dir: "".to_string(),
            start_roi: [0, 0, 0, 0],
            end_roi: [0, 0, 0, 0],
            win_roi: [0, 0, 0, 0],
            lose_roi: [0, 0, 0, 0],
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        if let Some(base_dirs) = directories::BaseDirs::new() {
            base_dirs.config_dir().join("ggst-clipper")
        } else {
            PathBuf::from("ggst-clipper")
        }
    }

    pub fn config_path() -> PathBuf {
        let dir = Self::config_dir();
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }
        dir.join("config.ini")
    }

    pub fn characters_path() -> PathBuf {
        Self::config_dir().join("characters.txt")
    }

    pub fn ensure_characters_file() {
        let path = Self::characters_path();
        if !path.exists() {
            let dir = Self::config_dir();
            if !dir.exists() {
                let _ = std::fs::create_dir_all(&dir);
            }
            let default_text = include_str!("../../assets/models/characters.txt");
            let _ = std::fs::write(&path, default_text);
        }
    }

    pub fn get_character_names() -> Vec<String> {
        Self::ensure_characters_file();
        let path = Self::characters_path();
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| include_str!("../../assets/models/characters.txt").to_string());

        let mut names = vec!["None".to_string()];
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }
            let canonical = if let Some(idx) = line.find(':') {
                &line[..idx]
            } else if let Some(idx) = line.find('=') {
                &line[..idx]
            } else {
                line
            };
            let canonical = canonical.trim();
            if !canonical.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(canonical)) {
                names.push(canonical.to_string());
            }
        }
        names
    }

    pub fn load() -> Self {
        Self::ensure_characters_file();
        let path = Self::config_path();
        let mut config = Self::default();

        if let Ok(conf) = Ini::load_from_file(&path) {
            if let Some(section) = conf.section(Some("Settings")) {
                if let Some(val) = section.get("start_template") { config.start_template = val.to_string(); }
                if let Some(val) = section.get("end_template") { config.end_template = val.to_string(); }
                if let Some(val) = section.get("win_template") { config.win_template = val.to_string(); }
                if let Some(val) = section.get("lose_template") { config.lose_template = val.to_string(); }
                if let Some(val) = section.get("detect_win_loss") {
                    if let Ok(v) = val.parse::<bool>() { config.detect_win_loss = v; }
                }
                if let Some(val) = section.get("detect_characters") {
                    if let Ok(v) = val.parse::<bool>() { config.detect_characters = v; }
                }
                if let Some(val) = section.get("my_character") { config.my_character = val.to_string(); }
                
                if let Some(val) = section.get("threshold") { 
                    if let Ok(v) = val.parse::<f32>() { config.threshold = v; } 
                }
                if let Some(val) = section.get("step_frames") { 
                    if let Ok(v) = val.parse::<u32>() { config.step_frames = v; } 
                }
                if let Some(val) = section.get("start_offset") { 
                    if let Ok(v) = val.parse::<i32>() { config.start_offset = v; } 
                }
                if let Some(val) = section.get("end_offset") { 
                    if let Ok(v) = val.parse::<i32>() { config.end_offset = v; } 
                }
                if let Some(val) = section.get("win_offset") { 
                    if let Ok(v) = val.parse::<u32>() { config.win_offset = v; } 
                }
                if let Some(val) = section.get("output_dir") { config.output_dir = val.to_string(); }

                let parse_roi = |s: &str| -> Option<[u32; 4]> {
                    let parts: Vec<&str> = s.split(',').collect();
                    if parts.len() == 4 {
                        let mut arr = [0; 4];
                        for i in 0..4 {
                            arr[i] = parts[i].trim().parse::<u32>().ok()?;
                        }
                        return Some(arr);
                    }
                    None
                };

                if let Some(val) = section.get("start_roi") { if let Some(arr) = parse_roi(val) { config.start_roi = arr; } }
                if let Some(val) = section.get("end_roi") { if let Some(arr) = parse_roi(val) { config.end_roi = arr; } }
                if let Some(val) = section.get("win_roi") { if let Some(arr) = parse_roi(val) { config.win_roi = arr; } }
                if let Some(val) = section.get("lose_roi") { if let Some(arr) = parse_roi(val) { config.lose_roi = arr; } }
            }
        }
        config
    }

    pub fn save(&self) {
        let path = Self::config_path();
        let mut conf = Ini::new();
        conf.with_section(Some("Settings"))
            .set("start_template", &self.start_template)
            .set("end_template", &self.end_template)
            .set("win_template", &self.win_template)
            .set("lose_template", &self.lose_template)
            .set("detect_win_loss", self.detect_win_loss.to_string())
            .set("detect_characters", self.detect_characters.to_string())
            .set("my_character", &self.my_character)
            .set("threshold", self.threshold.to_string())
            .set("step_frames", self.step_frames.to_string())
            .set("start_offset", self.start_offset.to_string())
            .set("end_offset", self.end_offset.to_string())
            .set("win_offset", self.win_offset.to_string())
            .set("output_dir", &self.output_dir)
            .set("start_roi", format!("{},{},{},{}", self.start_roi[0], self.start_roi[1], self.start_roi[2], self.start_roi[3]))
            .set("end_roi", format!("{},{},{},{}", self.end_roi[0], self.end_roi[1], self.end_roi[2], self.end_roi[3]))
            .set("win_roi", format!("{},{},{},{}", self.win_roi[0], self.win_roi[1], self.win_roi[2], self.win_roi[3]))
            .set("lose_roi", format!("{},{},{},{}", self.lose_roi[0], self.lose_roi[1], self.lose_roi[2], self.lose_roi[3]));
        
        let _ = conf.write_to_file(&path);
    }
}

