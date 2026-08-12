use clap::Parser;
use rayon::prelude::*;
use image::{DynamicImage, GenericImageView, Rgba};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Template matching based video segment extraction tool (Rust version)."
)]
struct Args {
    /// Input video file path (can be passed via drag and drop)
    #[arg(index = 1, allow_hyphen_values = true)]
    input_positional: Option<String>,

    #[arg(short, long, allow_hyphen_values = true)]
    input: Option<String>,

    #[arg(long, default_value = "start.png")]
    start_template: String,

    #[arg(long, default_value = "end.png")]
    end_template: String,

    #[arg(short, long)]
    output: Option<String>,

    #[arg(long, default_value_t = 0.9)]
    threshold: f64,

    #[arg(long, default_value_t = 60)]
    step_frames: usize,
}

fn get_roi_bbox(img: &DynamicImage) -> Option<(u32, u32, u32, u32)> {
    let mut x_min = u32::MAX;
    let mut x_max = 0;
    let mut y_min = u32::MAX;
    let mut y_max = 0;
    let mut found = false;

    let has_alpha = img.color().has_alpha();
    for (x, y, pixel) in img.pixels() {
        let Rgba([r, g, b, a]) = pixel;
        let is_non_white = if has_alpha {
            a > 0 && !(r >= 250 && g >= 250 && b >= 250)
        } else {
            !(r >= 250 && g >= 250 && b >= 250)
        };

        if is_non_white {
            x_min = x_min.min(x);
            x_max = x_max.max(x);
            y_min = y_min.min(y);
            y_max = y_max.max(y);
            found = true;
        }
    }

    if found {
        Some((x_min, y_min, x_max - x_min + 1, y_max - y_min + 1))
    } else {
        None
    }
}

fn extract_roi(buffer: &[u8], buf_w: u32, buf_h: u32, bbox: (u32, u32, u32, u32)) -> Vec<u8> {
    let (x, y, w, h) = bbox;
    let mut roi = Vec::with_capacity((w * h * 3) as usize);

    for j in 0..h {
        let row_y = y + j;
        if row_y >= buf_h {
            roi.extend(std::iter::repeat_n(0, (w * 3) as usize));
            continue;
        }
        for i in 0..w {
            let col_x = x + i;
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



fn compute_similarity_zero_copy(
    buffer: &[u8],
    buf_w: u32,
    buf_h: u32,
    bbox: (u32, u32, u32, u32),
    template: &[u8],
) -> f64 {
    let (x, y, w, h) = bbox;
    let n = (w * h * 3) as f64;

    let (sum_a, sum_b, sum_a_sq, sum_b_sq, sum_ab) = (0..h).into_par_iter().map(|j| {
        let mut local_sum_a = 0.0;
        let mut local_sum_b = 0.0;
        let mut local_sum_a_sq = 0.0;
        let mut local_sum_b_sq = 0.0;
        let mut local_sum_ab = 0.0;

        let row_y = y + j;
        let row_valid = row_y < buf_h;
        
        let mut t_idx = (j * w * 3) as usize;

        for i in 0..w {
            let col_x = x + i;
            let col_valid = col_x < buf_w;
            
            for c in 0..3 {
                let a = if row_valid && col_valid {
                    let idx = ((row_y * buf_w + col_x) * 3 + c) as usize;
                    buffer[idx] as f64
                } else {
                    0.0
                };
                let b = template[t_idx] as f64;
                t_idx += 1;

                local_sum_a += a;
                local_sum_b += b;
                local_sum_a_sq += a * a;
                local_sum_b_sq += b * b;
                local_sum_ab += a * b;
            }
        }
        (local_sum_a, local_sum_b, local_sum_a_sq, local_sum_b_sq, local_sum_ab)
    }).reduce(
        || (0.0, 0.0, 0.0, 0.0, 0.0),
        |acc, local| {
            (
                acc.0 + local.0,
                acc.1 + local.1,
                acc.2 + local.2,
                acc.3 + local.3,
                acc.4 + local.4,
            )
        },
    );

    let num = n * sum_ab - sum_a * sum_b;
    let den_a = n * sum_a_sq - sum_a * sum_a;
    let den_b = n * sum_b_sq - sum_b * sum_b;

    if den_a <= 0.0 || den_b <= 0.0 {
        return 0.0;
    }

    num / (den_a.sqrt() * den_b.sqrt())
}

fn get_video_info(path: &str) -> Option<(u32, u32, f64)> {
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
        let w: u32 = parts[0].parse().ok()?;
        let h: u32 = parts[1].parse().ok()?;
        let fps_parts: Vec<&str> = parts[2].split('/').collect();
        let fps = if fps_parts.len() == 2 {
            let num: f64 = fps_parts[0].parse().ok()?;
            let den: f64 = fps_parts[1].parse().ok()?;
            num / den
        } else {
            parts[2].parse().ok()?
        };
        Some((w, h, fps))
    } else {
        None
    }
}

fn get_video_duration(path: &str) -> Option<f64> {
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

fn find_exact_boundary(
    input_path: &str,
    start_time: f64,
    num_frames: usize,
    fps: f64,
    frame_w: u32,
    frame_h: u32,
    bbox: (u32, u32, u32, u32),
    template: &[u8],
    threshold: f64,
) -> f64 {
    let mut child = Command::new("ffmpeg")
        .args(&[
            "-ss",
            &format!("{:.6}", start_time),
            "-i",
            input_path,
            "-frames:v",
            &num_frames.to_string(),
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
    let frame_size = (frame_w * frame_h * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    
    let mut frame_offset = 0;
    let mut best_match_time = start_time + (num_frames as f64 / fps);

    while stdout.read_exact(&mut buffer).is_ok() {
        let sim = compute_similarity_zero_copy(&buffer, frame_w, frame_h, bbox, template);
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

fn resolve_template_path(tmpl_path: &str) -> String {
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

fn export_segments_ffmpeg(input: &str, segments: &[(f64, f64)], out_dir: &Path) {
    if segments.is_empty() {
        println!("No segments found to export.");
        return;
    }
    fs::create_dir_all(out_dir).unwrap();
    println!("\nExporting {} segment(s)...", segments.len());

    let input_path = Path::new(input);
    let file_stem = input_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");

    for (i, &(start, end)) in segments.iter().enumerate() {
        let out_name = format!("{}-part{:03}.mp4", file_stem, i + 1);
        let out_path = out_dir.join(out_name);
        let mut cmd = Command::new("ffmpeg");
        let start_str = format!("{:.3}", start);
        let end_str = format!("{:.3}", end);
        
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
                        start,
                        end
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

fn pause() {
    println!("\nPress Enter to exit...");
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

fn check_and_install_ffmpeg() {
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

fn main() {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            println!("{}", e);
            pause();
            return;
        }
    };
    check_and_install_ffmpeg();
    let input_path_opt = args.input_positional.or(args.input);

    let input_path = match input_path_opt {
        Some(p) => p,
        None => {
            println!("Please select a video file (file dialog will open)...");
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Video files", &["mp4", "avi", "mkv", "mov", "flv", "wmv", "webm"])
                .set_title("Select a video file")
                .pick_file()
            {
                path.to_str().unwrap().to_string()
            } else {
                println!("No video file was selected.");
                pause();
                return;
            }
        }
    };

    let input_path = fs::canonicalize(&input_path)
        .map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
        .unwrap_or(input_path);

    if !Path::new(&input_path).exists() {
        println!("Error: Video file does not exist: {}", input_path);
        pause();
        return;
    }

    let video_dir = Path::new(&input_path).parent().unwrap_or(Path::new(""));

    let output_dir = match args.output {
        Some(o) => PathBuf::from(o),
        None => video_dir.join("output"),
    };

    let start_tmpl_path = resolve_template_path(&args.start_template);
    let end_tmpl_path = resolve_template_path(&args.end_template);

    println!("[Input Video]: {}", input_path);
    println!("[Output Directory]: {}", output_dir.display());

    // 1. ROI Calculation
    println!("\nCalculating ROI for start template: {}", start_tmpl_path);
    let start_img = match image::open(&start_tmpl_path) {
        Ok(img) => img,
        Err(e) => {
            println!("Error: Failed to open start template '{}': {}", start_tmpl_path, e);
            pause();
            return;
        }
    };
    let start_bbox = match get_roi_bbox(&start_img) {
        Some(bbox) => bbox,
        None => {
            println!("Error: No non-white region found in start template");
            pause();
            return;
        }
    };
    println!("Start ROI (x,y,w,h): {:?}", start_bbox);

    println!("Calculating ROI for end template: {}", end_tmpl_path);
    let end_img = match image::open(&end_tmpl_path) {
        Ok(img) => img,
        Err(e) => {
            println!("Error: Failed to open end template '{}': {}", end_tmpl_path, e);
            pause();
            return;
        }
    };
    let end_bbox = match get_roi_bbox(&end_img) {
        Some(bbox) => bbox,
        None => {
            println!("Error: No non-white region found in end template");
            pause();
            return;
        }
    };
    println!("End ROI (x,y,w,h): {:?}", end_bbox);

    // 2. Crop Templates
    println!("\nCropping templates...");
    let start_tmpl = extract_roi(
        start_img.to_rgb8().as_raw(),
        start_img.width(),
        start_img.height(),
        start_bbox,
    );
    let end_tmpl = extract_roi(
        end_img.to_rgb8().as_raw(),
        end_img.width(),
        end_img.height(),
        end_bbox,
    );

    // 3. Scan video
    let calc_start = Instant::now();
    let (frame_w, frame_h, fps) = match get_video_info(&input_path) {
        Some(info) => info,
        None => {
            println!("Error: Failed to get video info. Is FFmpeg installed and working?");
            pause();
            return;
        }
    };
    let duration = get_video_duration(&input_path).unwrap_or(0.0);
    let total_frames = (duration * fps) as usize;

    println!(
        "\nScanning video ({}x{}, {:.2} FPS, ~{} frames) with step={}...",
        frame_w, frame_h, fps, total_frames, args.step_frames
    );

    let mut segments = Vec::new();
    let mut state = "SEARCH_START";
    let mut start_time: Option<f64> = None;
    let mut start_frame: Option<usize> = None;

    let pb = ProgressBar::new(total_frames as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
            )
            .unwrap()
            .progress_chars("#>-"),
    );

    let select_filter = format!("select='not(mod(n\\,{}))'", args.step_frames);
    let child = Command::new("ffmpeg")
        .args(&[
            "-hwaccel",
            "auto",
            "-threads",
            "0",
            "-i",
            &input_path,
            "-vf",
            &select_filter,
            "-fps_mode",
            "vfr",
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
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            println!("Error: Failed to spawn FFmpeg process: {}", e);
            pause();
            return;
        }
    };

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let frame_size = (frame_w * frame_h * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    let mut f_idx = 0;

    while stdout.read_exact(&mut buffer).is_ok() {
        if state == "SEARCH_START" {
            let sim = compute_similarity_zero_copy(&buffer, frame_w, frame_h, start_bbox, &start_tmpl);

            if sim >= args.threshold {
                let left_f = if f_idx >= args.step_frames { f_idx - args.step_frames } else { 0 };
                let exact_time = find_exact_boundary(
                    &input_path, left_f as f64 / fps, args.step_frames + 1, fps, frame_w, frame_h, start_bbox, &start_tmpl, args.threshold
                );

                start_frame = Some((exact_time * fps).round() as usize);
                start_time = Some(exact_time);
                state = "SEARCH_END";
                pb.suspend(|| {
                    println!(
                        "\n[+] Start detected at frame {} ({:.3}s, sim={:.4})",
                        start_frame.unwrap(),
                        start_time.unwrap(),
                        sim
                    );
                });
            }
        } else if state == "SEARCH_END" {
            let sim = compute_similarity_zero_copy(&buffer, frame_w, frame_h, end_bbox, &end_tmpl);
            if sim >= args.threshold {
                let left_f = if f_idx >= args.step_frames { f_idx - args.step_frames } else { 0 };
                let exact_time = find_exact_boundary(
                    &input_path, left_f as f64 / fps, args.step_frames + 1, fps, frame_w, frame_h, end_bbox, &end_tmpl, args.threshold
                );
                
                let adjusted_time = (exact_time - (120.0 / fps)).max(start_time.unwrap());
                
                pb.suspend(|| {
                    println!(
                        "[+] End detected at frame {} (adjusted to {:.3}s, sim={:.4})",
                        (exact_time * fps).round() as usize, adjusted_time, sim
                    );
                });
                segments.push((start_time.unwrap(), adjusted_time));
                state = "SEARCH_START";
                start_time = None;
                start_frame = None;
            }
        }

        f_idx += args.step_frames;
        if f_idx <= total_frames {
            pb.set_position(f_idx as u64);
        }
    }
    pb.finish_with_message("Done");
    let _ = child.wait();

    if state == "SEARCH_END" {
        println!(
            "\n[!] Warning: Start detected at frame {} ({:.2}s) without a corresponding End before video finish.",
            start_frame.unwrap(),
            start_time.unwrap()
        );
    }

    let calc_duration = calc_start.elapsed();

    // 4. Trimming with FFmpeg
    let export_start = Instant::now();
    export_segments_ffmpeg(&input_path, &segments, &output_dir);
    let export_duration = export_start.elapsed();

    let calc_secs = calc_duration.as_secs();
    let export_secs = export_duration.as_secs();
    let total_secs = calc_secs + export_secs;

    let format_secs = |secs: u64| {
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    };

    println!("\n=== Processing Time ===");
    println!("Frame Calculation: {}", format_secs(calc_secs));
    println!("Video Export:      {}", format_secs(export_secs));
    println!("Total Time:        {}", format_secs(total_secs));
    println!("=======================\n");

    println!("\nProcessing completed!");
    pause();
}

