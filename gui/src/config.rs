use ini::Ini;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub start_template: String,
    pub end_template: String,
    pub win_template: String,
    pub lose_template: String,
    pub threshold: f32,
    pub step_frames: u32,
    pub start_offset: i32,
    pub end_offset: i32,
    pub output_dir: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            start_template: "start.png".to_string(),
            end_template: "end.png".to_string(),
            win_template: "win.png".to_string(),
            lose_template: "lose.png".to_string(),
            threshold: 0.9,
            step_frames: 60,
            start_offset: 0,
            end_offset: -120,
            output_dir: "".to_string(),
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        if let Some(base_dirs) = directories::BaseDirs::new() {
            base_dirs.config_dir().join("ggst-clip")
        } else {
            PathBuf::from("ggst-clip")
        }
    }

    pub fn config_path() -> PathBuf {
        let dir = Self::config_dir();
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }
        dir.join("config.ini")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        let mut config = Self::default();

        if let Ok(conf) = Ini::load_from_file(&path) {
            if let Some(section) = conf.section(Some("Settings")) {
                if let Some(val) = section.get("start_template") { config.start_template = val.to_string(); }
                if let Some(val) = section.get("end_template") { config.end_template = val.to_string(); }
                if let Some(val) = section.get("win_template") { config.win_template = val.to_string(); }
                if let Some(val) = section.get("lose_template") { config.lose_template = val.to_string(); }
                
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
                if let Some(val) = section.get("output_dir") { config.output_dir = val.to_string(); }
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
            .set("threshold", self.threshold.to_string())
            .set("step_frames", self.step_frames.to_string())
            .set("start_offset", self.start_offset.to_string())
            .set("end_offset", self.end_offset.to_string())
            .set("output_dir", &self.output_dir);
        
        let _ = conf.write_to_file(&path);
    }
}
