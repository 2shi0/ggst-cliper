mod cli;
mod ffmpeg;
mod image_proc;
mod ocr;
mod types;
mod utils;

use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use cli::Args;
use ffmpeg::{
    check_and_install_ffmpeg, export_segments_ffmpeg, extract_frame_roi, find_exact_boundary,
    get_video_duration, get_video_info,
};
use image_proc::{compute_similarity_zero_copy, scale_and_crop_template};
use ocr::{get_default_character_rois, get_default_win_lose_roi, GGSTDetector};
use types::{BBox, IpcMessage, MatchResult, SearchState, Segment};
use utils::{format_duration, pause, resolve_template_path};

fn send_ipc(msg: &IpcMessage) {
    if let Ok(json) = serde_json::to_string(msg) {
        println!("{}", json);
        let _ = std::io::stdout().flush();
    }
}

fn exit_error(msg: &str, is_json: bool, no_pause: bool) -> ! {
    if is_json {
        send_ipc(&IpcMessage::Error {
            message: msg.to_string(),
        });
    } else {
        println!("Error: {}", msg);
        if !no_pause {
            pause();
        }
    }
    std::process::exit(1);
}

fn main() {
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let _ = e.print();
            if e.use_stderr() {
                let has_no_pause = std::env::args().any(|arg| arg == "--no-pause" || arg == "--json");
                if !has_no_pause {
                    pause();
                }
            }
            return;
        }
    };

    if !args.json {
        check_and_install_ffmpeg();
    }

    let input_path_opt = args.input_positional.or(args.input);

    let input_path = match input_path_opt {
        Some(p) => p,
        None => {
            if args.json {
                exit_error("No video file was specified.", true, args.no_pause);
            }
            println!("Please select a video file (file dialog will open)...");
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(
                    "Video files",
                    &["mp4", "avi", "mkv", "mov", "flv", "wmv", "webm"],
                )
                .set_title("Select a video file")
                .pick_file()
            {
                path.to_str().unwrap().to_string()
            } else {
                println!("No video file was selected.");
                if !args.no_pause {
                    pause();
                }
                return;
            }
        }
    };

    let input_path = fs::canonicalize(&input_path)
        .map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
        .unwrap_or(input_path);

    if !Path::new(&input_path).exists() {
        exit_error(
            &format!("Video file does not exist: {}", input_path),
            args.json,
            args.no_pause,
        );
    }

    let video_dir = Path::new(&input_path).parent().unwrap_or(Path::new(""));

    let output_dir = match args.output {
        Some(o) => PathBuf::from(o),
        None => video_dir.join("output"),
    };

    let start_tmpl_path = resolve_template_path(&args.start_template);
    let end_tmpl_path = resolve_template_path(&args.end_template);

    if !args.json {
        println!("[Input Video]: {}", input_path);
        println!("[Output Directory]: {}", output_dir.display());
    }

    let video_info = match get_video_info(&input_path) {
        Some(info) => info,
        None => {
            exit_error(
                "Failed to get video info. Is FFmpeg installed and working?",
                args.json,
                args.no_pause,
            );
        }
    };
    let duration = get_video_duration(&input_path).unwrap_or(0.0);
    let total_frames = (duration * video_info.fps) as usize;

    let parse_roi = |s: Option<&String>| -> Option<BBox> {
        if let Some(s) = s {
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() == 4
                && let (Ok(x), Ok(y), Ok(w), Ok(h)) = (
                    parts[0].parse::<u32>(),
                    parts[1].parse::<u32>(),
                    parts[2].parse::<u32>(),
                    parts[3].parse::<u32>(),
                )
                && w > 0
                && h > 0
            {
                return Some(BBox::new(x, y, w, h));
            }
        }
        None
    };

    // 1. Template Loading and Auto-Scaling
    if !args.json {
        println!("\nLoading start template: {}", start_tmpl_path);
    }
    let start_img = match image::open(&start_tmpl_path) {
        Ok(img) => img,
        Err(e) => {
            exit_error(
                &format!(
                    "Failed to open start template '{}': {}",
                    start_tmpl_path, e
                ),
                args.json,
                args.no_pause,
            );
        }
    };
    let raw_start_bbox = parse_roi(args.start_roi.as_ref())
        .unwrap_or_else(|| BBox::new(0, 0, start_img.width(), start_img.height()));
    let (start_bbox, start_tmpl, start_stats) = scale_and_crop_template(
        &start_img,
        raw_start_bbox,
        video_info.width,
        video_info.height,
    );
    if !args.json {
        if start_img.width() != video_info.width || start_img.height() != video_info.height {
            println!(
                "[Scale]: Start template auto-scaled from {}x{} to video {}x{} (ROI: {:?} -> {:?})",
                start_img.width(),
                start_img.height(),
                video_info.width,
                video_info.height,
                raw_start_bbox,
                start_bbox
            );
        } else {
            println!("Start ROI (x,y,w,h): {:?}", start_bbox);
        }
        println!("Loading end template: {}", end_tmpl_path);
    }

    let end_img = match image::open(&end_tmpl_path) {
        Ok(img) => img,
        Err(e) => {
            exit_error(
                &format!("Failed to open end template '{}': {}", end_tmpl_path, e),
                args.json,
                args.no_pause,
            );
        }
    };
    let raw_end_bbox = parse_roi(args.end_roi.as_ref())
        .unwrap_or_else(|| BBox::new(0, 0, end_img.width(), end_img.height()));
    let (end_bbox, end_tmpl, end_stats) = scale_and_crop_template(
        &end_img,
        raw_end_bbox,
        video_info.width,
        video_info.height,
    );
    if !args.json {
        if end_img.width() != video_info.width || end_img.height() != video_info.height {
            println!(
                "[Scale]: End template auto-scaled from {}x{} to video {}x{} (ROI: {:?} -> {:?})",
                end_img.width(),
                end_img.height(),
                video_info.width,
                video_info.height,
                raw_end_bbox,
                end_bbox
            );
        } else {
            println!("End ROI (x,y,w,h): {:?}", end_bbox);
        }
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

    // 2. OCR and Scan Preparation
    let calc_start = Instant::now();

    let (def_p1_roi, def_p2_roi) = get_default_character_rois(video_info.width, video_info.height);
    let p1_roi = parse_roi(args.p1_roi.as_ref()).unwrap_or(def_p1_roi);
    let p2_roi = parse_roi(args.p2_roi.as_ref()).unwrap_or(def_p2_roi);

    let def_win_lose_roi = get_default_win_lose_roi(video_info.width, video_info.height);
    let win_lose_roi = parse_roi(args.win_roi.as_ref())
        .filter(|r| r.width >= 50 && r.height >= 20)
        .unwrap_or(def_win_lose_roi);

    if !args.json && args.detect_win_loss {
        println!("Win/Lose Detection ROI (x,y,w,h): {:?}", win_lose_roi);
    }

    let mut detector_opt = if args.detect_characters || args.detect_win_loss {
        match GGSTDetector::try_new() {
            Ok(d) => {
                if !args.json {
                    println!("[OCR]: GGST Detector loaded successfully.");
                }
                Some(d)
            }
            Err(e) => {
                if !args.json {
                    println!(
                        "[OCR Warning]: Failed to load GGST Detector: {}. OCR detection will be disabled.",
                        e
                    );
                }
                None
            }
        }
    } else {
        None
    };

    if !args.json {
        println!(
            "\nScanning video ({}x{}, {:.2} FPS, ~{} frames) with step={}...",
            video_info.width, video_info.height, video_info.fps, total_frames, args.step_frames
        );
        println!(
            "ROI streaming size: {}x{} (Cropped from original {}x{})",
            union_bbox.width, union_bbox.height, video_info.width, video_info.height
        );
    }

    let mut segments = Vec::new();
    let mut state = SearchState::SearchStart;
    let mut start_time: Option<f64> = None;
    let mut start_frame: Option<usize> = None;
    let mut current_p1_name: Option<String> = None;
    let mut current_p2_name: Option<String> = None;

    let pb = if !args.json {
        let bar = ProgressBar::new(total_frames as u64);
        bar.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(bar)
    } else {
        None
    };

    let crop_filter = format!(
        "format=rgb24,crop={}:{}:{}:{},select='not(mod(n\\,{}))'",
        union_bbox.width, union_bbox.height, union_bbox.x, union_bbox.y, args.step_frames
    );
    let child = Command::new("ffmpeg")
        .args([
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
            exit_error(
                &format!("Failed to spawn FFmpeg process: {}", e),
                args.json,
                args.no_pause,
            );
        }
    };

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let frame_size = (union_bbox.width * union_bbox.height * 3) as usize;
    let mut buffer = vec![0u8; frame_size];
    let mut f_idx: usize = 0;
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
                    let left_f = f_idx.saturating_sub(args.step_frames);
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

                    let (p1, p2) = detect_match_characters(
                        detector_opt.as_mut(),
                        &input_path,
                        exact_time,
                        p1_roi,
                        p2_roi,
                    );
                    current_p1_name = p1;
                    current_p2_name = p2;

                    let p1_display = current_p1_name.clone();
                    let p2_display = current_p2_name.clone();

                    if let Some(ref pb) = pb {
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
                    let left_f = f_idx.saturating_sub(args.step_frames);
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

                    let match_result = if args.detect_win_loss {
                        detect_match_win_lose(
                            detector_opt.as_mut(),
                            &input_path,
                            exact_time,
                            win_lose_roi,
                        )
                    } else {
                        MatchResult::Skipped
                    };

                    if let Some(ref pb) = pb {
                        pb.suspend(|| {
                            if args.end_offset != 0 {
                                println!(
                                    "[+] End detected at frame {} ({:.3}s, sim={:.4}) -> adjusted to frame {} ({:.3}s, offset={}f) [Result: {:?}]",
                                    raw_frame,
                                    exact_time,
                                    sim,
                                    adjusted_frame,
                                    adjusted_time,
                                    args.end_offset,
                                    match_result
                                );
                            } else {
                                println!(
                                    "[+] End detected at frame {} ({:.3}s, sim={:.4}) [Result: {:?}]",
                                    raw_frame, exact_time, sim, match_result
                                );
                            }
                        });
                    }

                    let segment = Segment {
                        start: start_time.unwrap(),
                        end: adjusted_time,
                        result: match_result,
                        p1_name: current_p1_name.take(),
                        p2_name: current_p2_name.take(),
                    };

                    if args.json {
                        send_ipc(&IpcMessage::SegmentDetected {
                            index: segments.len() + 1,
                            start: segment.start,
                            end: segment.end,
                            result: segment.result,
                            p1: segment.p1_name.clone(),
                            p2: segment.p2_name.clone(),
                        });
                    }

                    segments.push(segment);
                    state = SearchState::SearchStart;
                    start_time = None;
                    start_frame = None;
                }
            }
        }

        f_idx += args.step_frames;
        if let Some(ref pb) = pb {
            if f_idx <= total_frames {
                pb.set_position(f_idx as u64);
            }
        } else if args.json {
            let pct = if total_frames > 0 {
                (f_idx as f32 / total_frames as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            send_ipc(&IpcMessage::Progress {
                phase: "scan".to_string(),
                current: f_idx.min(total_frames),
                total: total_frames,
                percentage: pct,
                message: format!(
                    "Scanning frames ({}/{})",
                    f_idx.min(total_frames),
                    total_frames
                ),
            });
        }
    }

    if let Some(ref pb) = pb {
        pb.finish_with_message("Done");
    }

    let status = child.wait();
    if frames_processed == 0 {
        if args.json {
            exit_error(
                "No frames were read from FFmpeg. Process may have exited prematurely.",
                true,
                args.no_pause,
            );
        } else {
            println!("\n[!] Error: No frames were read from FFmpeg. FFmpeg process may have exited prematurely.");
        }
    }
    if let Ok(s) = status {
        if !s.success() && !args.json {
            println!("\n[!] FFmpeg main process exited with status: {}", s);
        }
    } else if let Err(e) = status {
        if !args.json {
            println!("\n[!] FFmpeg main process error: {}", e);
        }
    }

    if state == SearchState::SearchEnd && !args.json {
        println!(
            "\n[!] Warning: Start detected at frame {} ({:.2}s) without a corresponding End before video finish.",
            start_frame.unwrap(),
            start_time.unwrap()
        );
    }

    let calc_duration = calc_start.elapsed();

    // 4. Trimming with FFmpeg
    let export_start = Instant::now();
    export_segments_ffmpeg(
        &input_path,
        &segments,
        &output_dir,
        args.my_character.as_deref(),
        if args.json {
            Some(&|current, total, path| {
                let pct = current as f32 / total as f32;
                let file_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                send_ipc(&IpcMessage::Progress {
                    phase: "export".to_string(),
                    current,
                    total,
                    percentage: pct,
                    message: format!("Exporting clip {}/{} -> {}", current, total, file_name),
                });
            })
        } else {
            None
        },
    );
    let export_duration = export_start.elapsed();

    let calc_secs = calc_duration.as_secs();
    let export_secs = export_duration.as_secs();
    let total_secs = calc_secs + export_secs;

    if args.json {
        send_ipc(&IpcMessage::Done {
            total_frames,
            segments_count: segments.len(),
            calc_time_secs: calc_secs,
            export_time_secs: export_secs,
            total_time_secs: total_secs,
            output_dir: output_dir.to_string_lossy().to_string(),
        });
    } else {
        println!("\n=== Processing Time ===");
        println!("Frame Calculation: {}", format_duration(calc_secs));
        println!("Video Export:      {}", format_duration(export_secs));
        println!("Total Time:        {}", format_duration(total_secs));
        println!("=======================\n");

        println!("\nProcessing completed!");
        if !args.no_pause {
            pause();
        }
    }
}

fn detect_match_win_lose(
    detector_opt: Option<&mut GGSTDetector>,
    input_path: &str,
    exact_time: f64,
    win_lose_roi: BBox,
) -> MatchResult {
    let Some(detector) = detector_opt else {
        return MatchResult::Skipped;
    };

    for offset in [0.5, 1.0, 1.5, 2.0, 2.5, 3.0] {
        let probe_time = exact_time + offset;
        if let Some(crop) = extract_frame_roi(input_path, probe_time, win_lose_roi) {
            if let Ok(res) = detector.detect_win_lose(&crop) {
                if res == MatchResult::Win || res == MatchResult::Lose {
                    return res;
                }
            }
        }
    }

    MatchResult::Unknown
}

fn detect_match_characters(
    detector_opt: Option<&mut GGSTDetector>,
    input_path: &str,
    exact_time: f64,
    p1_roi: BBox,
    p2_roi: BBox,
) -> (Option<String>, Option<String>) {
    let Some(detector) = detector_opt else {
        return (None, None);
    };

    let probe_time = exact_time + 2.0;
    let mut p1_detected = None;
    let mut p2_detected = None;

    let check_frame = |t: f64,
                       p1_det: &mut Option<String>,
                       p2_det: &mut Option<String>,
                       det: &mut GGSTDetector| {
        if p1_det.is_none()
            && let Some(p1_img) = extract_frame_roi(input_path, t, p1_roi)
            && let Ok(Some((name, _conf))) = det.detect_and_recognize(&p1_img)
        {
            *p1_det = Some(name);
        }
        if p2_det.is_none()
            && let Some(p2_img) = extract_frame_roi(input_path, t, p2_roi)
            && let Ok(Some((name, _conf))) = det.detect_and_recognize(&p2_img)
        {
            *p2_det = Some(name);
        }
    };

    check_frame(probe_time, &mut p1_detected, &mut p2_detected, detector);

    for retry_count in 1..=6 {
        if p1_detected.is_some() && p2_detected.is_some() {
            break;
        }
        let retry_time = probe_time + (0.5 * retry_count as f64);
        check_frame(retry_time, &mut p1_detected, &mut p2_detected, detector);
    }

    (p1_detected, p2_detected)
}
