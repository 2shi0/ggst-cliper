use crate::image_proc::{compute_similarity_zero_copy, TemplateStats};
use crate::types::{BBox, MatchResult, Segment, VideoInfo};
use crate::utils::pause;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn check_and_install_ffmpeg() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        println!("FFmpeg is not installed. It is required for this program to run.");
        println!("Would you like to install it automatically using winget? [y/N]");
        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
        if input.trim().eq_ignore_ascii_case("y") {
            println!("Installing FFmpeg...");
            let status = Command::new("winget")
                .args(&["install", "ffmpeg"])
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("FFmpeg installed successfully. Please restart the application (you may need to reopen your terminal for PATH changes to take effect).");
                    pause();
                    std::process::exit(0);
                }
                _ => {
                    println!("Failed to install FFmpeg. Please install it manually.");
                    pause();
                    std::process::exit(1);
                }
            }
        } else {
            println!("FFmpeg installation aborted. The program will now exit.");
            pause();
            std::process::exit(1);
        }
    }
}

pub fn get_video_info(path: &str) -> Option<VideoInfo> {
    let output = Command::new("ffprobe")
        .args(&[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0",
            path,
        ])
        .output()
        .ok()?;

    let s = String::from_utf8_lossy(&output.stdout);
    let line = s.lines().next()?;
    let parts: Vec<&str> = line.trim().split(',').collect();
    if parts.len() >= 3 {
        let width: u32 = parts[0].parse().ok()?;
        let height: u32 = parts[1].parse().ok()?;
        let fps_parts: Vec<&str> = parts[2].split('/').collect();
        let fps = if fps_parts.len() == 2 {
            let num: f64 = fps_parts[0].parse().ok()?;
            let den: f64 = fps_parts[1].parse().ok()?;
            num / den
        } else {
            parts[2].parse().ok()?
        };
        Some(VideoInfo { width, height, fps })
    } else {
        None
    }
}

pub fn get_video_duration(path: &str) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args(&[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path,
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse().ok()
}

pub fn find_exact_boundary(
    input_path: &str,
    start_time: f64,
    num_frames: usize,
    fps: f64,
    bbox: BBox,
    template: &[u8],
    threshold: f64,
    stats: &TemplateStats,
) -> f64 {
    let crop_filter = format!("format=rgb24,crop={}:{}:{}:{}", bbox.width, bbox.height, bbox.x, bbox.y);
    let mut child = Command::new("ffmpeg")
        .args(&[
            "-ss",
            &format!("{:.6}", start_time),
            "-i",
            input_path,
            "-frames:v",
            &num_frames.to_string(),
            "-vf",
            &crop_filter,
            "-f",
            "image2pipe",
            "-pix_fmt",
            "rgb24",
            "-vcodec",
            "rawvideo",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn ffmpeg for boundary search");

    let mut stdout = child.stdout.take().unwrap();
    let frame_size = (bbox.width * bbox.height * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    let local_bbox = BBox::new(0, 0, bbox.width, bbox.height);

    let mut frame_offset = 0;
    let mut best_match_time = start_time + (num_frames as f64 / fps);

    while stdout.read_exact(&mut buffer).is_ok() {
        let sim = compute_similarity_zero_copy(&buffer, bbox.width, bbox.height, local_bbox, template, stats);
        if sim >= threshold {
            best_match_time = start_time + (frame_offset as f64 / fps);
            break;
        }
        frame_offset += 1;
    }

    let _ = child.kill();
    let _ = child.wait();

    best_match_time
}

pub fn find_match_result(
    input_path: &str,
    start_time: f64,
    win_bbox: Option<BBox>,
    win_tmpl: Option<&[u8]>,
    win_stats: Option<&TemplateStats>,
    lose_bbox: Option<BBox>,
    lose_tmpl: Option<&[u8]>,
    lose_stats: Option<&TemplateStats>,
    threshold: f64,
    win_offset: usize,
) -> MatchResult {
    if win_offset == 0 || (win_bbox.is_none() && lose_bbox.is_none()) {
        return MatchResult::Skipped;
    }

    let num_frames = win_offset;

    let w_box = win_bbox.unwrap_or(BBox::new(0,0,0,0));
    let l_box = lose_bbox.unwrap_or(BBox::new(0,0,0,0));

    let union_bbox = if win_bbox.is_some() && lose_bbox.is_some() {
        w_box.union(&l_box)
    } else if win_bbox.is_some() {
        w_box
    } else {
        l_box
    };

    let rel_win_bbox = if win_bbox.is_some() {
        BBox::new(w_box.x - union_bbox.x, w_box.y - union_bbox.y, w_box.width, w_box.height)
    } else {
        BBox::new(0,0,0,0)
    };

    let rel_lose_bbox = if lose_bbox.is_some() {
        BBox::new(l_box.x - union_bbox.x, l_box.y - union_bbox.y, l_box.width, l_box.height)
    } else {
        BBox::new(0,0,0,0)
    };

    let crop_filter = format!("format=rgb24,crop={}:{}:{}:{}", union_bbox.width, union_bbox.height, union_bbox.x, union_bbox.y);
    let mut child = Command::new("ffmpeg")
        .args(&[
            "-ss",
            &format!("{:.6}", start_time),
            "-i",
            input_path,
            "-frames:v",
            &num_frames.to_string(),
            "-vf",
            &crop_filter,
            "-f",
            "image2pipe",
            "-pix_fmt",
            "rgb24",
            "-vcodec",
            "rawvideo",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn ffmpeg for match result search");

    let mut stdout = child.stdout.take().unwrap();
    let frame_size = (union_bbox.width * union_bbox.height * 3) as usize;
    let mut buffer = vec![0u8; frame_size];

    let mut result = MatchResult::Unknown;

    while stdout.read_exact(&mut buffer).is_ok() {
        if let (Some(w_tmpl), Some(w_stats)) = (win_tmpl, win_stats) {
            let sim = compute_similarity_zero_copy(&buffer, union_bbox.width, union_bbox.height, rel_win_bbox, w_tmpl, w_stats);
            if sim >= threshold {
                result = MatchResult::Win;
                break;
            }
        }
        if let (Some(l_tmpl), Some(l_stats)) = (lose_tmpl, lose_stats) {
            let sim = compute_similarity_zero_copy(&buffer, union_bbox.width, union_bbox.height, rel_lose_bbox, l_tmpl, l_stats);
            if sim >= threshold {
                result = MatchResult::Lose;
                break;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    result
}

pub fn extract_frame_roi(input_path: &str, time_sec: f64, bbox: BBox) -> Option<image::RgbImage> {
    let crop_filter = format!("format=rgb24,crop={}:{}:{}:{}", bbox.width, bbox.height, bbox.x, bbox.y);
    let mut child = Command::new("ffmpeg")
        .args(&[
            "-ss",
            &format!("{:.6}", time_sec),
            "-i",
            input_path,
            "-frames:v",
            "1",
            "-vf",
            &crop_filter,
            "-f",
            "image2pipe",
            "-pix_fmt",
            "rgb24",
            "-vcodec",
            "rawvideo",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let frame_size = (bbox.width * bbox.height * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    stdout.read_exact(&mut buffer).ok()?;
    let _ = child.wait();

    image::RgbImage::from_raw(bbox.width, bbox.height, buffer)
}

pub fn export_segments_ffmpeg(input: &str, segments: &[Segment], out_dir: &Path) {
    if segments.is_empty() {
        println!("No segments found to export.");
        return;
    }
    fs::create_dir_all(out_dir).unwrap();
    println!("\nExporting {} segment(s)...", segments.len());

    let input_path = Path::new(input);
    let file_stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    for (i, seg) in segments.iter().enumerate() {
        let prefix = match seg.result {
            MatchResult::Win => "win-",
            MatchResult::Lose => "lose-",
            MatchResult::Unknown => "unknown-",
            MatchResult::Skipped => "",
        };
        let char_suffix = match (&seg.p1_name, &seg.p2_name) {
            (Some(p1), Some(p2)) => format!("-{}-vs-{}", p1, p2),
            (Some(p1), None) => format!("-{}-vs-Unknown", p1),
            (None, Some(p2)) => format!("-Unknown-vs-{}", p2),
            (None, None) => "".to_string(),
        };
        let out_name = format!("{}{}{}-part{:03}.mp4", prefix, file_stem, char_suffix, i + 1);
        let out_path = out_dir.join(out_name);
        let mut cmd = Command::new("ffmpeg");
        let start_str = format!("{:.3}", seg.start);
        let end_str = format!("{:.3}", seg.end);

        cmd.args(&[
            "-y",
            "-threads",
            "0",
            "-ss",
            &start_str,
            "-to",
            &end_str,
            "-i",
            input,
            "-map",
            "0:v:0",
            "-map",
            "0:a?",
            "-map",
            "0:v:0",
            "-c:v:0",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-c:a",
            "aac",
            "-c:v:1",
            "mjpeg",
            "-filter:v:1",
            "select=eq(n\\,155)",
            "-disposition:v:1",
            "attached_pic",
            out_path.to_str().unwrap(),
        ]);

        match cmd.output() {
            Ok(output) => {
                if output.status.success() {
                    println!(
                        "  Saved: {} ({:.2}s -> {:.2}s)",
                        out_path.display(),
                        seg.start,
                        seg.end
                    );
                } else {
                    println!(
                        "  Error (segment {}): {}",
                        i + 1,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
            Err(e) => {
                println!("  Command execution error (segment {}): {}", i + 1, e);
            }
        }
    }
}
