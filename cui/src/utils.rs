use std::io::{self, Write};
use std::path::Path;

pub fn resolve_template_path(tmpl_path: &str) -> String {
    let p = Path::new(tmpl_path);
    if p.is_absolute() && p.exists() {
        return tmpl_path.to_string();
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidate = exe_dir.join(tmpl_path);
            if candidate.exists() {
                return candidate.to_str().unwrap().to_string();
            }
        }
    }
    if p.exists() {
        return tmpl_path.to_string();
    }
    tmpl_path.to_string()
}

pub fn pause() {
    println!("\nPress Enter to exit...");
    let _ = io::stdout().flush();
    let mut s = String::new();
    let _ = io::stdin().read_line(&mut s);
}

pub fn format_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
