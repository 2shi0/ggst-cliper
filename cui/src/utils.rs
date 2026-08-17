use std::io::{self, Write};
use std::path::Path;

pub fn resolve_template_path(tmpl_path: &str) -> String {
    let p = Path::new(tmpl_path);
    if p.is_absolute() && p.exists() {
        return tmpl_path.to_string();
    }
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent() {
            let candidate = exe_dir.join(tmpl_path);
            if candidate.exists() {
                return candidate.to_str().unwrap().to_string();
            }

            let mut curr = exe_dir.parent();
            for _ in 0..6 {
                if let Some(dir) = curr {
                    let c = dir.join(tmpl_path);
                    if c.exists() {
                        return c.to_str().unwrap().to_string();
                    }
                    curr = dir.parent();
                } else {
                    break;
                }
            }
        }
    if let Some(base_dirs) = directories::BaseDirs::new() {
        let app_data_tmpl = base_dirs.config_dir().join("ggst-clipper").join(tmpl_path);
        if app_data_tmpl.exists() {
            return app_data_tmpl.to_str().unwrap().to_string();
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

pub fn sanitize_filename_component(name: &str) -> String {
    name.chars()
        .filter(|&c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\x1F'))
        .collect::<String>()
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename_component() {
        assert_eq!(sanitize_filename_component("BEDMAN?"), "BEDMAN");
        assert_eq!(sanitize_filename_component("POTEMKIN"), "POTEMKIN");
        assert_eq!(sanitize_filename_component("ASUKA R#"), "ASUKA R#");
        assert_eq!(sanitize_filename_component("A.B.A"), "A.B.A");
        assert_eq!(sanitize_filename_component("A.B.A."), "A.B.A");
        assert_eq!(sanitize_filename_component("?*:<>|/\\"), "");
        assert_eq!(sanitize_filename_component("  BEDMAN?  "), "BEDMAN");
    }
}

