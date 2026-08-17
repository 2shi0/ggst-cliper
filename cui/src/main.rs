mod cli;
mod ffmpeg;
mod image_proc;
mod ocr;
mod types;
mod utils;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::BufReader;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use cli::Args;
use ffmpeg::{
    check_and_install_ffmpeg, export_segments_ffmpeg, extract_frame_roi, find_exact_boundary, find_match_result, get_video_duration,
    get_video_info,
};
use image_proc::{compute_similarity_zero_copy, extract_roi, compute_template_stats};
use ocr::{get_default_character_rois, GGSTDetector};
use types::{BBox, SearchState, Segment};
use utils::{format_duration, pause, resolve_template_path};

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
    let win_tmpl_path = resolve_template_path(&args.win_template);
    let lose_tmpl_path = resolve_template_path(&args.lose_template);

    println!("[Input Video]: {}", input_path);
    println!("[Output Directory]: {}", output_dir.display());

    let parse_roi = |s: Option<&String>| -> Option<BBox> {
        if let Some(s) = s {
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() == 4 {
                if let (Ok(x), Ok(y), Ok(w), Ok(h)) = (
                    parts[0].parse::<u32>(),
                    parts[1].parse::<u32>(),
                    parts[2].parse::<u32>(),
                    parts[3].parse::<u32>(),
                ) {
                    if w > 0 && h > 0 {
                        return Some(BBox::new(x, y, w, h));
                    }
                }
            }
        }
        None
    };

    // 1. ROI Calculation
    println!("\nLoading start template: {}", start_tmpl_path);
    let start_img = match image::open(&start_tmpl_path) {
        Ok(img) => img,
        Err(e) => {
            println!("Error: Failed to open start template '{}': {}", start_tmpl_path, e);
            pause();
            return;
        }
    };
    let start_bbox = parse_roi(args.start_roi.as_ref()).unwrap_or_else(|| BBox::new(0, 0, start_img.width(), start_img.height()));
    println!("Start ROI (x,y,w,h): {:?}", start_bbox);

    println!("Loading end template: {}", end_tmpl_path);
    let end_img = match image::open(&end_tmpl_path) {
        Ok(img) => img,
        Err(e) => {
            println!("Error: Failed to open end template '{}': {}", end_tmpl_path, e);
            pause();
            return;
        }
    };
    let end_bbox = parse_roi(args.end_roi.as_ref()).unwrap_or_else(|| BBox::new(0, 0, end_img.width(), end_img.height()));
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

    let start_stats = compute_template_stats(&start_tmpl);
    let end_stats = compute_template_stats(&end_tmpl);

    let (mut win_bbox_opt, mut win_tmpl_opt, mut win_stats_opt) = (None, None, None);
    if Path::new(&win_tmpl_path).exists() {
        if let Ok(img) = image::open(&win_tmpl_path) {
            if let Some(bbox) = parse_roi(args.win_roi.as_ref()) {
                let tmpl = extract_roi(img.to_rgb8().as_raw(), img.width(), img.height(), bbox);
                let stats = compute_template_stats(&tmpl);
                win_bbox_opt = Some(bbox);
                win_tmpl_opt = Some(tmpl);
                win_stats_opt = Some(stats);
                println!("Win ROI (x,y,w,h): {:?}", bbox);
            } else {
                println!("Win ROI not set or invalid (0,0,0,0). Win detection skipped.");
            }
        }
    } else {
        println!("Win template '{}' not found. Win detection skipped.", win_tmpl_path);
    }

    let (mut lose_bbox_opt, mut lose_tmpl_opt, mut lose_stats_opt) = (None, None, None);
    if Path::new(&lose_tmpl_path).exists() {
        if let Ok(img) = image::open(&lose_tmpl_path) {
            if let Some(bbox) = parse_roi(args.lose_roi.as_ref()) {
                let tmpl = extract_roi(img.to_rgb8().as_raw(), img.width(), img.height(), bbox);
                let stats = compute_template_stats(&tmpl);
                lose_bbox_opt = Some(bbox);
                lose_tmpl_opt = Some(tmpl);
                lose_stats_opt = Some(stats);
                println!("Lose ROI (x,y,w,h): {:?}", bbox);
            } else {
                println!("Lose ROI not set or invalid (0,0,0,0). Lose detection skipped.");
            }
        }
    } else {
        println!("Lose template '{}' not found. Lose detection skipped.", lose_tmpl_path);
    }

    let union_bbox = start_bbox.union(&end_bbox);
    let rel_start_bbox = BBox::new(
        start_bbox.x - union_bbox.x,
        start_bbox.y - union_bbox.y,
        start_bbox.width,
        start_bbox.height,
    );
    let rel_end_bbox = BBox::new(
        end_bbox.x - union_bbox.x,
        end_bbox.y - union_bbox.y,
        end_bbox.width,
        end_bbox.height,
    );

    // 3. Scan video
    let calc_start = Instant::now();
    let video_info = match get_video_info(&input_path) {
        Some(info) => info,
        None => {
            println!("Error: Failed to get video info. Is FFmpeg installed and working?");
            pause();
            return;
        }
    };
    let duration = get_video_duration(&input_path).unwrap_or(0.0);
    let total_frames = (duration * video_info.fps) as usize;

    let (def_p1_roi, def_p2_roi) = get_default_character_rois(video_info.width, video_info.height);
    let p1_roi = parse_roi(args.p1_roi.as_ref()).unwrap_or(def_p1_roi);
    let p2_roi = parse_roi(args.p2_roi.as_ref()).unwrap_or(def_p2_roi);

    let mut detector_opt = if args.detect_characters {
        match GGSTDetector::try_new() {
            Ok(d) => {
                println!("[OCR]: GGST Character Detector loaded successfully.");
                Some(d)
            }
            Err(e) => {
                println!("[OCR Warning]: Failed to load GGST Character Detector: {}. Character detection will be disabled.", e);
                None
            }
        }
    } else {
        None
    };

    println!(
        "\nScanning video ({}x{}, {:.2} FPS, ~{} frames) with step={}...",
        video_info.width, video_info.height, video_info.fps, total_frames, args.step_frames
    );
    println!(
        "ROI streaming size: {}x{} (Cropped from original {}x{})",
        union_bbox.width, union_bbox.height, video_info.width, video_info.height
    );

    let mut segments = Vec::new();
    let mut state = SearchState::SearchStart;
    let mut start_time: Option<f64> = None;
    let mut start_frame: Option<usize> = None;
    let mut current_p1_name: Option<String> = None;
    let mut current_p2_name: Option<String> = None;

    let pb = ProgressBar::new(total_frames as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
            )
            .unwrap()
            .progress_chars("#>-"),
    );

    let crop_filter = format!(
        "format=rgb24,crop={}:{}:{}:{},select='not(mod(n\\,{}))'",
        union_bbox.width, union_bbox.height, union_bbox.x, union_bbox.y, args.step_frames
    );
    let child = Command::new("ffmpeg")
        .args(&[
            "-hwaccel",
            "auto",
            "-threads",
            "0",
            "-i",
            &input_path,
            "-vf",
            &crop_filter,
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
    let frame_size = (union_bbox.width * union_bbox.height * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    let mut f_idx = 0;

    let mut frames_processed = 0;
    while stdout.read_exact(&mut buffer).is_ok() {
        frames_processed += 1;
        match state {
            SearchState::SearchStart => {
                let sim = compute_similarity_zero_copy(
                    &buffer,
                    union_bbox.width,
                    union_bbox.height,
                    rel_start_bbox,
                    &start_tmpl,
                    &start_stats,
                );

                if sim >= args.threshold {
                    let left_f = if f_idx >= args.step_frames {
                        f_idx - args.step_frames
                    } else {
                        0
                    };
                    let exact_time = find_exact_boundary(
                        &input_path,
                        left_f as f64 / video_info.fps,
                        args.step_frames + 1,
                        video_info.fps,
                        start_bbox,
                        &start_tmpl,
                        args.threshold,
                        &start_stats,
                    );

                    let raw_frame = (exact_time * video_info.fps).round() as usize;
                    let adjusted_time =
                        (exact_time + (args.start_offset as f64 / video_info.fps)).max(0.0);
                    let adjusted_frame = (adjusted_time * video_info.fps).round() as usize;

                    start_frame = Some(adjusted_frame);
                    start_time = Some(adjusted_time);
                    state = SearchState::SearchEnd;

                    // Character Name Detection 1.0s after exact start
                    let mut p1_detected = None;
                    let mut p2_detected = None;

                    if let Some(ref mut detector) = detector_opt {
                        let probe_time = exact_time + 1.0;
                        if let Some(p1_img) = extract_frame_roi(&input_path, probe_time, p1_roi) {
                            if let Ok(Some((name, conf))) = detector.detect_and_recognize(&p1_img) {
                                p1_detected = Some((name.to_string(), conf));
                            }
                        }
                        if let Some(p2_img) = extract_frame_roi(&input_path, probe_time, p2_roi) {
                            if let Ok(Some((name, conf))) = detector.detect_and_recognize(&p2_img) {
                                p2_detected = Some((name.to_string(), conf));
                            }
                        }

                        // Retry 60 frames later if either character was not detected
                        if p1_detected.is_none() || p2_detected.is_none() {
                            let retry_time = probe_time + (60.0 / video_info.fps);
                            if p1_detected.is_none() {
                                if let Some(p1_img) = extract_frame_roi(&input_path, retry_time, p1_roi) {
                                    if let Ok(Some((name, conf))) = detector.detect_and_recognize(&p1_img) {
                                        p1_detected = Some((name.to_string(), conf));
                                    }
                                }
                            }
                            if p2_detected.is_none() {
                                if let Some(p2_img) = extract_frame_roi(&input_path, retry_time, p2_roi) {
                                    if let Ok(Some((name, conf))) = detector.detect_and_recognize(&p2_img) {
                                        p2_detected = Some((name.to_string(), conf));
                                    }
                                }
                            }
                        }
                    }

                    current_p1_name = p1_detected.map(|(n, _)| n);
                    current_p2_name = p2_detected.map(|(n, _)| n);

                    let p1_display = current_p1_name.clone();
                    let p2_display = current_p2_name.clone();

                    pb.suspend(|| {
                        if args.start_offset != 0 {
                            println!(
                                "\n[+] Start detected at frame {} ({:.3}s, sim={:.4}) -> adjusted to frame {} ({:.3}s, offset={}f)",
                                raw_frame,
                                exact_time,
                                sim,
                                adjusted_frame,
                                adjusted_time,
                                args.start_offset
                            );
                        } else {
                            println!(
                                "\n[+] Start detected at frame {} ({:.3}s, sim={:.4})",
                                raw_frame, exact_time, sim
                            );
                        }
                        if p1_display.is_some() || p2_display.is_some() {
                            println!(
                                "    [Characters Detected]: 1P: {} vs 2P: {}",
                                p1_display.as_deref().unwrap_or("Unknown"),
                                p2_display.as_deref().unwrap_or("Unknown")
                            );
                        }
                    });
                }
            }
            SearchState::SearchEnd => {
                let sim = compute_similarity_zero_copy(
                    &buffer,
                    union_bbox.width,
                    union_bbox.height,
                    rel_end_bbox,
                    &end_tmpl,
                    &end_stats,
                );
                if sim >= args.threshold {
                    let left_f = if f_idx >= args.step_frames {
                        f_idx - args.step_frames
                    } else {
                        0
                    };
                    let exact_time = find_exact_boundary(
                        &input_path,
                        left_f as f64 / video_info.fps,
                        args.step_frames + 1,
                        video_info.fps,
                        end_bbox,
                        &end_tmpl,
                        args.threshold,
                        &end_stats,
                    );

                    let raw_frame = (exact_time * video_info.fps).round() as usize;
                    let adjusted_time = (exact_time + (args.end_offset as f64 / video_info.fps))
                        .max(start_time.unwrap());
                    let adjusted_frame = (adjusted_time * video_info.fps).round() as usize;

                    let match_result = find_match_result(
                        &input_path,
                        exact_time,
                        win_bbox_opt,
                        win_tmpl_opt.as_deref(),
                        win_stats_opt.as_ref(),
                        lose_bbox_opt,
                        lose_tmpl_opt.as_deref(),
                        lose_stats_opt.as_ref(),
                        args.threshold,
                        args.win_offset,
                    );

                    pb.suspend(|| {
                        if args.end_offset != 0 {
                            println!(
                                "[+] End detected at frame {} ({:.3}s, sim={:.4}) -> adjusted to frame {} ({:.3}s, offset={}f) [Result: {:?}]",
                                raw_frame, exact_time, sim, adjusted_frame, adjusted_time, args.end_offset, match_result
                            );
                        } else {
                            println!(
                                "[+] End detected at frame {} ({:.3}s, sim={:.4}) [Result: {:?}]",
                                raw_frame, exact_time, sim, match_result
                            );
                        }
                    });
                    segments.push(Segment {
                        start: start_time.unwrap(),
                        end: adjusted_time,
                        result: match_result,
                        p1_name: current_p1_name.take(),
                        p2_name: current_p2_name.take(),
                    });
                    state = SearchState::SearchStart;
                    start_time = None;
                    start_frame = None;
                }
            }
        }

        f_idx += args.step_frames;
        if f_idx <= total_frames {
            pb.set_position(f_idx as u64);
        }
    }
    pb.finish_with_message("Done");
    let status = child.wait();
    if frames_processed == 0 {
        println!("\n[!] Error: No frames were read from FFmpeg. FFmpeg process may have exited prematurely.");
    }
    if let Ok(s) = status {
        if !s.success() {
            println!("\n[!] FFmpeg main process exited with status: {}", s);
        }
    }

    if state == SearchState::SearchEnd {
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

    println!("\n=== Processing Time ===");
    println!("Frame Calculation: {}", format_duration(calc_secs));
    println!("Video Export:      {}", format_duration(export_secs));
    println!("Total Time:        {}", format_duration(total_secs));
    println!("=======================\n");

    println!("\nProcessing completed!");
    pause();
}
