use clap::Parser;
use image::{DynamicImage, GenericImageView, Rgba};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Template matching based video segment extraction tool (Rust version)."
)]
struct Args {
    /// Input video file path (can be passed via drag and drop)
    #[arg(index = 1)]
    input_positional: Option<String>,

    #[arg(short, long)]
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

    for (x, y, pixel) in img.pixels() {
        let Rgba([r, g, b, a]) = pixel;
        let is_non_white = if img.color().has_alpha() {
            a > 0 && !(r >= 250 && g >= 250 && b >= 250)
        } else {
            !(r >= 250 && g >= 250 && b >= 250)
        };

        if is_non_white {
            if x < x_min {
                x_min = x;
            }
            if x > x_max {
                x_max = x;
            }
            if y < y_min {
                y_min = y;
            }
            if y > y_max {
                y_max = y;
            }
            found = true;
        }
    }

    if found {
        Some((x_min, y_min, x_max - x_min + 1, y_max - y_min + 1))
    } else {
        None
    }
}

fn crop_rgb_image(img: &DynamicImage, bbox: (u32, u32, u32, u32)) -> Vec<u8> {
    let rgb = img.to_rgb8();
    let (x, y, w, h) = bbox;
    let mut cropped = Vec::with_capacity((w * h * 3) as usize);
    for j in 0..h {
        for i in 0..w {
            let p = rgb.get_pixel(x + i, y + j);
            cropped.push(p[0]);
            cropped.push(p[1]);
            cropped.push(p[2]);
        }
    }
    cropped
}

fn extract_roi(frame: &[u8], frame_w: u32, frame_h: u32, bbox: (u32, u32, u32, u32)) -> Vec<u8> {
    let (x, y, w, h) = bbox;
    let mut roi = Vec::with_capacity((w * h * 3) as usize);

    for j in 0..h {
        let row_y = y + j;
        if row_y >= frame_h {
            roi.extend(vec![0; (w * 3) as usize]);
            continue;
        }
        for i in 0..w {
            let col_x = x + i;
            if col_x >= frame_w {
                roi.push(0);
                roi.push(0);
                roi.push(0);
            } else {
                let idx = ((row_y * frame_w + col_x) * 3) as usize;
                roi.push(frame[idx]);
                roi.push(frame[idx + 1]);
                roi.push(frame[idx + 2]);
            }
        }
    }
    roi
}

fn compute_similarity(roi: &[u8], template: &[u8]) -> f64 {
    if roi.len() != template.len() || roi.is_empty() {
        return 0.0;
    }
    let n_pixels = (roi.len() / 3) as f64;
    
    let mut sum_a = [0.0, 0.0, 0.0];
    let mut sum_b = [0.0, 0.0, 0.0];
    
    for i in (0..roi.len()).step_by(3) {
        sum_a[0] += roi[i] as f64;
        sum_a[1] += roi[i+1] as f64;
        sum_a[2] += roi[i+2] as f64;
        sum_b[0] += template[i] as f64;
        sum_b[1] += template[i+1] as f64;
        sum_b[2] += template[i+2] as f64;
    }
    
    let mean_a = [sum_a[0] / n_pixels, sum_a[1] / n_pixels, sum_a[2] / n_pixels];
    let mean_b = [sum_b[0] / n_pixels, sum_b[1] / n_pixels, sum_b[2] / n_pixels];

    let mut num = 0.0;
    let mut den_a = 0.0;
    let mut den_b = 0.0;

    for i in (0..roi.len()).step_by(3) {
        for c in 0..3 {
            let a = roi[i+c] as f64 - mean_a[c];
            let b = template[i+c] as f64 - mean_b[c];
            num += a * b;
            den_a += a * a;
            den_b += b * b;
        }
    }

    if den_a == 0.0 || den_b == 0.0 {
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
        println!("エクスポートするセグメントが見つかりませんでした。");
        return;
    }
    fs::create_dir_all(out_dir).unwrap();
    println!("\n{} 個のセグメントをエクスポートしています...", segments.len());

    for (i, &(start, end)) in segments.iter().enumerate() {
        let out_name = format!("output_{:02}.mp4", i + 1);
        let out_path = out_dir.join(out_name);
        let mut cmd = Command::new("ffmpeg");
        cmd.args(&[
            "-y",
            "-ss",
            &format!("{:.3}", start),
            "-to",
            &format!("{:.3}", end),
            "-i",
            input,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-c:a",
            "copy",
            out_path.to_str().unwrap(),
        ]);

        match cmd.output() {
            Ok(output) => {
                if output.status.success() {
                    println!(
                        "  保存完了: {} ({:.2}s -> {:.2}s)",
                        out_path.display(),
                        start,
                        end
                    );
                } else {
                    println!(
                        "  エラー (セグメント {}): {}",
                        i + 1,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
            Err(e) => {
                println!("  コマンド実行エラー (セグメント {}): {}", i + 1, e);
            }
        }
    }
}

fn pause() {
    println!("\nEnterキーを押して終了します...");
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

fn main() {
    let args = Args::parse();
    let input_path_opt = args.input_positional.or(args.input);

    let input_path = match input_path_opt {
        Some(p) => p,
        None => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("動画ファイル", &["mp4", "avi", "mkv", "mov", "flv", "wmv", "webm"])
                .set_title("動画ファイルを選択してください")
                .pick_file()
            {
                path.to_str().unwrap().to_string()
            } else {
                println!("動画ファイルが選択されませんでした。");
                pause();
                return;
            }
        }
    };

    let input_path = fs::canonicalize(&input_path)
        .unwrap_or_else(|_| PathBuf::from(&input_path))
        .to_str()
        .unwrap()
        .to_string();
        
    // Fix Windows UNC path prefix from canonicalize
    let input_path = if input_path.starts_with(r"\\?\") {
        input_path[4..].to_string()
    } else {
        input_path
    };

    if !Path::new(&input_path).exists() {
        println!("エラー: 動画ファイルが存在しません: {}", input_path);
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

    println!("[入力動画]: {}", input_path);
    println!("[出力ディレクトリ]: {}", output_dir.display());

    // 1. ROI Calculation
    println!("\nCalculating ROI for start template: {}", start_tmpl_path);
    let start_img = image::open(&start_tmpl_path).expect("Failed to open start template");
    let start_bbox = get_roi_bbox(&start_img).expect("No non-white region found in start template");
    println!("Start ROI (x,y,w,h): {:?}", start_bbox);

    println!("Calculating ROI for end template: {}", end_tmpl_path);
    let end_img = image::open(&end_tmpl_path).expect("Failed to open end template");
    let end_bbox = get_roi_bbox(&end_img).expect("No non-white region found in end template");
    println!("End ROI (x,y,w,h): {:?}", end_bbox);

    // 2. Crop Templates
    println!("\nCropping templates...");
    let start_tmpl = crop_rgb_image(&start_img, start_bbox);
    let end_tmpl = crop_rgb_image(&end_img, end_bbox);

    // 3. Scan video
    let (frame_w, frame_h, fps) =
        get_video_info(&input_path).expect("Failed to get video info. Is FFmpeg installed?");
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
    let mut child = Command::new("ffmpeg")
        .args(&[
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
        .spawn()
        .expect("Failed to spawn FFmpeg process");

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let frame_size = (frame_w * frame_h * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    let mut f_idx = 0;

    while stdout.read_exact(&mut buffer).is_ok() {
        if state == "SEARCH_START" {
            let roi = extract_roi(&buffer, frame_w, frame_h, start_bbox);
            let sim = compute_similarity(&roi, &start_tmpl);
            
            if sim >= args.threshold {
                start_frame = Some(f_idx);
                start_time = Some(f_idx as f64 / fps);
                state = "SEARCH_END";
                pb.suspend(|| {
                    println!(
                        "\n[+] Start detected at frame {} ({:.2}s, sim={:.4})",
                        f_idx,
                        start_time.unwrap(),
                        sim
                    );
                });
            }
        } else if state == "SEARCH_END" {
            let roi = extract_roi(&buffer, frame_w, frame_h, end_bbox);
            let sim = compute_similarity(&roi, &end_tmpl);
            if sim >= args.threshold {
                let end_t = f_idx as f64 / fps;
                pb.suspend(|| {
                    println!(
                        "[+] End detected at frame {} ({:.2}s, sim={:.4})",
                        f_idx, end_t, sim
                    );
                });
                segments.push((start_time.unwrap(), end_t));
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

    // 4. Trimming with FFmpeg
    export_segments_ffmpeg(&input_path, &segments, &output_dir);

    println!("\n処理が完了しました！");
    pause();
}
