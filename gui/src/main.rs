#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod theme;

use config::AppConfig;
use eframe::egui;
use rfd::FileDialog;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{channel, Receiver},
    Arc,
};

fn open_file(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = Command::new("cmd")
            .args(["/c", "start", "", &path.to_string_lossy()])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

fn format_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum MatchResult {
    Win,
    Lose,
    Unknown,
    Skipped,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcMessage {
    Log {
        message: String,
    },
    Progress {
        phase: String,
        current: usize,
        total: usize,
        percentage: f32,
        message: String,
    },
    SegmentDetected {
        index: usize,
        start: f64,
        end: f64,
        result: MatchResult,
        p1: Option<String>,
        p2: Option<String>,
    },
    Done {
        total_frames: usize,
        segments_count: usize,
        calc_time_secs: u64,
        export_time_secs: u64,
        total_time_secs: u64,
        output_dir: String,
    },
    Error {
        message: String,
    },
}

pub enum TaskStatus {
    Idle,
    Running {
        phase: String,
        progress: f32,
        message: String,
        segments_count: usize,
        cancel_flag: Arc<AtomicBool>,
        rx: Receiver<IpcMessage>,
    },
    Finished {
        segments_count: usize,
        calc_time: String,
        export_time: String,
        total_time: String,
        output_dir: String,
    },
    Error {
        message: String,
    },
}

struct RoiSelectionState {
    template_type: String,
    image_path: String,
    rect: Option<egui::Rect>,
    drag_start: Option<egui::Pos2>,
}

struct GgstClipApp {
    config: AppConfig,
    show_settings: bool,
    task_status: TaskStatus,
    roi_selection: Option<RoiSelectionState>,
}

impl GgstClipApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::configure_theme(&cc.egui_ctx);
        let config = AppConfig::load();
        Self {
            config,
            show_settings: false,
            task_status: TaskStatus::Idle,
            roi_selection: None,
        }
    }

    fn run_cli(&mut self, video_path: &str) {
        let cli_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("ggst-clipper-cui.exe")))
            .unwrap_or_else(|| std::path::PathBuf::from("ggst-clipper-cui.exe"));
        let mut cmd = Command::new(&cli_path);
        cmd.arg("-i").arg(video_path);

        if !self.config.output_dir.is_empty() {
            cmd.arg("-o").arg(&self.config.output_dir);
        }

        cmd.arg("--start-template").arg(&self.config.start_template);
        cmd.arg("--end-template").arg(&self.config.end_template);
        cmd.arg("--detect-win-loss")
            .arg(self.config.detect_win_loss.to_string());

        cmd.arg("--threshold")
            .arg(self.config.threshold.to_string());
        cmd.arg("--step-frames")
            .arg(self.config.step_frames.to_string());
        cmd.arg("--start-offset")
            .arg(self.config.start_offset.to_string());
        cmd.arg("--end-offset")
            .arg(self.config.end_offset.to_string());
        cmd.arg("--detect-characters")
            .arg(self.config.detect_characters.to_string());
        if !self.config.my_character.is_empty() && self.config.my_character != "None" {
            cmd.arg("--my-character").arg(&self.config.my_character);
        }

        let format_roi = |r: [u32; 4]| format!("{},{},{},{}", r[0], r[1], r[2], r[3]);
        cmd.arg("--start-roi")
            .arg(format_roi(self.config.start_roi));
        cmd.arg("--end-roi").arg(format_roi(self.config.end_roi));

        // Options for GUI integration
        cmd.arg("--json");
        cmd.arg("--no-pause");

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let (tx, rx) = channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_clone = Arc::clone(&cancel_flag);

        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();

                std::thread::spawn(move || {
                    use std::io::{BufRead, BufReader};

                    let mut got_done_or_error = false;

                    if let Some(stdout) = stdout {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines() {
                            if cancel_flag_clone.load(Ordering::SeqCst) {
                                let _ = child.kill();
                                break;
                            }
                            if let Ok(line_str) = line {
                                if let Ok(ipc_msg) = serde_json::from_str::<IpcMessage>(&line_str) {
                                    match &ipc_msg {
                                        IpcMessage::Done { .. } | IpcMessage::Error { .. } => {
                                            got_done_or_error = true;
                                        }
                                        _ => {}
                                    }
                                    let _ = tx.send(ipc_msg);
                                }
                            }
                        }
                    }

                    if cancel_flag_clone.load(Ordering::SeqCst) {
                        let _ = child.kill();
                        return;
                    }

                    let status = child.wait();
                    if !got_done_or_error {
                        if let Ok(s) = status {
                            if !s.success() {
                                let mut err_msg = format!("Process exited with code {:?}", s.code());
                                if let Some(stderr) = stderr {
                                    let err_reader = BufReader::new(stderr);
                                    let err_lines: Vec<String> =
                                        err_reader.lines().filter_map(|l| l.ok()).collect();
                                    if !err_lines.is_empty() {
                                        err_msg = err_lines.join("\n");
                                    }
                                }
                                let _ = tx.send(IpcMessage::Error { message: err_msg });
                            }
                        }
                    }
                });

                self.task_status = TaskStatus::Running {
                    phase: "Starting...".to_string(),
                    progress: 0.0_f32,
                    message: "Initializing analysis engine...".to_string(),
                    segments_count: 0,
                    cancel_flag,
                    rx,
                };
            }
            Err(e) => {
                self.task_status = TaskStatus::Error {
                    message: format!("Failed to start {}: {}", cli_path.display(), e),
                };
            }
        }
    }

    fn render_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut title_job = egui::text::LayoutJob::default();
            title_job.append(
                "GGST Clipper",
                0.0,
                egui::TextFormat {
                    font_id: egui::TextStyle::Heading.resolve(ui.style()),
                    color: theme::TEXT_WHITE,
                    ..Default::default()
                },
            );
            title_job.append(
                concat!("v", env!("CARGO_PKG_VERSION")),
                ui.spacing().item_spacing.x,
                egui::TextFormat {
                    font_id: egui::TextStyle::Body.resolve(ui.style()),
                    color: theme::TEXT_SUBTLE,
                    ..Default::default()
                },
            );
            ui.label(title_job);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let settings_btn = if self.show_settings {
                    ui.button("⚙ Settings (Open)")
                } else {
                    ui.button("⚙ Settings")
                };
                if settings_btn
                    .on_hover_text("Open Settings Window")
                    .clicked()
                {
                    self.show_settings = !self.show_settings;
                }

                ui.hyperlink_to("GitHub", "https://github.com/2shi0/ggst-clipper");
            });
        });
    }

    fn render_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut show = self.show_settings;
        let prev_config = self.config.clone();

        let builder = egui::ViewportBuilder::default()
            .with_title("GGST Clipper - Settings")
            .with_inner_size([440.0_f32, 540.0_f32])
            .with_resizable(false);

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("settings_window"),
            builder,
            |ctx, class| {
                assert!(
                    class == egui::ViewportClass::Immediate,
                    "This egui backend doesn't support multiple viewports"
                );

                if ctx.input(|i| i.viewport().close_requested()) {
                    show = false;
                }

                theme::configure_theme(ctx);

                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let mut open_roi_selection = None;

                            // --- Section: GENERAL ---
                            ui.heading("General");
                            ui.group(|ui| {
                                ui.label("Output Directory:")
                                    .on_hover_text("Target folder to save clipped match videos.\nIf empty, clips are saved alongside the source video.");

                                ui.horizontal(|ui| {
                                    let display_path = if self.config.output_dir.is_empty() {
                                        "Same as source video folder (Default)".to_string()
                                    } else {
                                        self.config.output_dir.clone()
                                    };

                                    ui.label(theme::subtle(&display_path));

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if !self.config.output_dir.is_empty()
                                                && ui.button("Clear").clicked()
                                            {
                                                self.config.output_dir.clear();
                                            }

                                            if ui.button("Browse...").clicked()
                                                && let Some(path) =
                                                    FileDialog::new().pick_folder()
                                            {
                                                self.config.output_dir =
                                                    path.to_string_lossy().to_string();
                                            }
                                        },
                                    );
                                });
                            });

                            ui.add_space(8.0_f32);

                            // --- Section: TEMPLATES & ROI ---
                            ui.heading("Templates & Detection Range");
                            let mut pick_image =
                                |ui: &mut egui::Ui,
                                 label: &str,
                                 template_type: &str,
                                 field: &mut String,
                                 roi: &mut [u32; 4],
                                 offset: &mut i32,
                                 offset_label: &str,
                                 offset_tooltip: &str,
                                 default_name: &str| {
                                    let file_exists = !field.is_empty()
                                        && std::path::Path::new(field).exists();

                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.strong(label);

                                            if file_exists {
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        if ui.button("Remove").clicked() {
                                                            let image_path = format!(
                                                                "file://{}",
                                                                field.replace('\\', "/")
                                                            );
                                                            let _ = std::fs::remove_file(
                                                                &*field,
                                                            );
                                                            ui.ctx().forget_image(&image_path);
                                                            field.clear();
                                                            *roi = [0, 0, 0, 0];
                                                        }

                                                        if ui.button("Select ROI").clicked() {
                                                            open_roi_selection = Some(
                                                                RoiSelectionState {
                                                                    template_type:
                                                                        template_type
                                                                            .to_string(),
                                                                    image_path:
                                                                        field.clone(),
                                                                    rect: if roi[2] > 0
                                                                        && roi[3] > 0
                                                                    {
                                                                        Some(
                                                                            egui::Rect::from_min_size(
                                                                                egui::pos2(roi[0] as f32, roi[1] as f32),
                                                                                egui::vec2(roi[2] as f32, roi[3] as f32),
                                                                            ),
                                                                        )
                                                                    } else {
                                                                        None
                                                                    },
                                                                    drag_start: None,
                                                                },
                                                            );
                                                        }
                                                    },
                                                );
                                            }
                                        });

                                        ui.add_space(4.0_f32);

                                        let mut request_pick = false;
                                        ui.horizontal(|ui| {
                                            let preview_size = egui::vec2(144.0_f32, 81.0_f32);
                                            if !file_exists {
                                                if ui
                                                    .add_sized(
                                                        preview_size,
                                                        egui::Button::new("+ Select Image"),
                                                    )
                                                    .clicked()
                                                {
                                                    request_pick = true;
                                                }
                                            } else {
                                                let image_path = format!(
                                                    "file://{}",
                                                    field.replace('\\', "/")
                                                );
                                                let img_response = ui.add(
                                                    egui::Image::new(&image_path)
                                                        .fit_to_exact_size(preview_size)
                                                        .sense(egui::Sense::click()),
                                                );

                                                // Overlay ROI on preview
                                                if roi[2] > 0
                                                    && roi[3] > 0
                                                    && let Ok((img_w, img_h)) =
                                                        image::image_dimensions(&*field)
                                                {
                                                    let scale_x =
                                                        preview_size.x / img_w as f32;
                                                    let scale_y =
                                                        preview_size.y / img_h as f32;
                                                    let rx = img_response.rect.min.x
                                                        + roi[0] as f32 * scale_x;
                                                    let ry = img_response.rect.min.y
                                                        + roi[1] as f32 * scale_y;
                                                    let rw = roi[2] as f32 * scale_x;
                                                    let rh = roi[3] as f32 * scale_y;

                                                    let mask_color =
                                                        egui::Color32::from_black_alpha(
                                                            120,
                                                        );
                                                    let img_rect = img_response.rect;
                                                    let roi_rect =
                                                        egui::Rect::from_min_size(
                                                            egui::pos2(rx, ry),
                                                            egui::vec2(rw, rh),
                                                        );

                                                    let p = ui.painter();
                                                    p.rect_filled(
                                                        egui::Rect::from_min_max(
                                                            img_rect.min,
                                                            egui::pos2(
                                                                img_rect.max.x,
                                                                roi_rect.min.y,
                                                            ),
                                                        ),
                                                        0.0_f32,
                                                        mask_color,
                                                    );
                                                    p.rect_filled(
                                                        egui::Rect::from_min_max(
                                                            egui::pos2(
                                                                img_rect.min.x,
                                                                roi_rect.max.y,
                                                            ),
                                                            img_rect.max,
                                                        ),
                                                        0.0_f32,
                                                        mask_color,
                                                    );
                                                    p.rect_filled(
                                                        egui::Rect::from_min_max(
                                                            egui::pos2(
                                                                img_rect.min.x,
                                                                roi_rect.min.y,
                                                            ),
                                                            egui::pos2(
                                                                roi_rect.min.x,
                                                                roi_rect.max.y,
                                                            ),
                                                        ),
                                                        0.0_f32,
                                                        mask_color,
                                                    );
                                                    p.rect_filled(
                                                        egui::Rect::from_min_max(
                                                            egui::pos2(
                                                                roi_rect.max.x,
                                                                roi_rect.min.y,
                                                            ),
                                                            egui::pos2(
                                                                img_rect.max.x,
                                                                roi_rect.max.y,
                                                            ),
                                                        ),
                                                        0.0_f32,
                                                        mask_color,
                                                    );
                                                    p.rect_stroke(
                                                        roi_rect,
                                                        0.0_f32,
                                                        egui::Stroke::new(
                                                            1.5_f32,
                                                            egui::Color32::LIGHT_BLUE,
                                                        ),
                                                    );
                                                }

                                                if img_response.clicked() {
                                                    request_pick = true;
                                                }
                                            }

                                            ui.add_space(8.0_f32);

                                            // Right side details
                                            ui.vertical(|ui| {
                                                if file_exists {
                                                    ui.label(format!(
                                                        "ROI: [{}, {}, {}, {}]",
                                                        roi[0], roi[1], roi[2], roi[3]
                                                    ));
                                                } else {
                                                    ui.label(
                                                        theme::subtle("No template image set"),
                                                    );
                                                }

                                                ui.add_space(4.0_f32);
                                                ui.horizontal(|ui| {
                                                    ui.label(offset_label)
                                                        .on_hover_text(offset_tooltip);
                                                    ui.add(
                                                        egui::DragValue::new(offset)
                                                            .speed(1)
                                                            .suffix(" frames"),
                                                    )
                                                    .on_hover_text(offset_tooltip);
                                                });
                                            });
                                        });

                                        if request_pick
                                            && let Some(path) = FileDialog::new()
                                                .add_filter(
                                                    "Images",
                                                    &["png", "jpg", "jpeg"],
                                                )
                                                .pick_file()
                                        {
                                            let config_dir = AppConfig::config_dir();
                                            let dest_path = config_dir.join(default_name);
                                            if std::fs::copy(&path, &dest_path).is_ok() {
                                                *field =
                                                    dest_path.to_string_lossy().to_string();
                                                ui.ctx().forget_image(&format!(
                                                    "file://{}",
                                                    field.replace('\\', "/")
                                                ));
                                                ui.ctx().request_repaint();
                                            } else {
                                                *field = path.to_string_lossy().to_string();
                                            }
                                            *roi = [0, 0, 0, 0];
                                        }
                                    });
                                };

                            pick_image(
                                ui,
                                "Start Match Template",
                                "start",
                                &mut self.config.start_template,
                                &mut self.config.start_roi,
                                &mut self.config.start_offset,
                                "Start Offset:",
                                "Frame offset for the beginning of the clip.\nA negative value includes seconds before match start.",
                                "start.png",
                            );

                            ui.add_space(4.0_f32);

                            pick_image(
                                ui,
                                "End Match Template",
                                "end",
                                &mut self.config.end_template,
                                &mut self.config.end_roi,
                                &mut self.config.end_offset,
                                "End Offset:",
                                "Frame offset for the end of the clip.\nA positive value includes seconds after match end.",
                                "end.png",
                            );

                            ui.add_space(8.0_f32);

                            // --- Section: OCR & MATCH DETECTION ---
                            ui.heading("Match & Character Detection");
                            ui.group(|ui| {
                                ui.checkbox(
                                    &mut self.config.detect_win_loss,
                                    "Detect Win / Loss (GGST only)",
                                )
                                .on_hover_text(
                                    "Detect WIN / LOSE outcome via OCR from the match result banner.",
                                );

                                ui.checkbox(
                                    &mut self.config.detect_characters,
                                    "Detect Character Names (GGST only)",
                                )
                                .on_hover_text(
                                    "Detect 1P & 2P character names via OCR 1s after match start\nand sort exported clips into character folders.",
                                );

                                if self.config.detect_characters {
                                    ui.add_space(4.0_f32);
                                    ui.horizontal(|ui| {
                                        ui.label("My Character:")
                                            .on_hover_text(
                                                "Select your main character.\nClips will be organized into folders named after your opponent.",
                                            );

                                        let character_names =
                                            AppConfig::get_character_names();
                                        egui::ComboBox::from_id_salt("my_character_combobox")
                                            .selected_text(&self.config.my_character)
                                            .show_ui(ui, |ui| {
                                                for name in character_names {
                                                    ui.selectable_value(
                                                        &mut self.config.my_character,
                                                        name.clone(),
                                                        &name,
                                                    );
                                                }
                                            });

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("Edit List").clicked() {
                                                    AppConfig::ensure_characters_file();
                                                    open_file(&AppConfig::characters_path());
                                                }
                                            },
                                        );
                                    });
                                }
                            });

                            ui.add_space(8.0_f32);

                            // --- Section: ENGINE PARAMETERS ---
                            ui.heading("Engine Parameters");
                            ui.group(|ui| {
                                egui::Grid::new("engine_params_grid")
                                    .num_columns(2)
                                    .spacing([12.0_f32, 6.0_f32])
                                    .show(ui, |ui| {
                                        ui.label("Threshold:")
                                            .on_hover_text("Image matching threshold (0.00 to 1.00).\nLower values require higher precision.");
                                        ui.add(
                                            egui::DragValue::new(&mut self.config.threshold)
                                                .speed(0.01)
                                                .range(0.0..=1.0),
                                        )
                                        .on_hover_text("Image matching threshold (0.00 to 1.00).\nLower values require higher precision.");
                                        ui.end_row();

                                        ui.label("Step Frames:")
                                            .on_hover_text("Frame interval skipped during video scanning.\nHigher values speed up scan time.");
                                        ui.add(
                                            egui::DragValue::new(
                                                &mut self.config.step_frames,
                                            )
                                            .speed(1)
                                            .range(1..=120),
                                        )
                                        .on_hover_text("Frame interval skipped during video scanning.\nHigher values speed up scan time.");
                                        ui.end_row();
                                    });
                            });

                            if let Some(state) = open_roi_selection {
                                self.roi_selection = Some(state);
                            }

                            ui.add_space(8.0_f32);
                            ui.label(theme::subtle("Changes are saved automatically."));
                        });
                });
            },
        );

        if self.config != prev_config {
            self.config.save();
        }

        self.show_settings = show;
    }

    fn render_roi_window(&mut self, ctx: &egui::Context) {
        let mut close_window = false;
        let mut save_rect = None;

        if let Some(state) = &mut self.roi_selection {
            let (img_w, img_h) =
                image::image_dimensions(&state.image_path).unwrap_or((800, 600));

            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Select ROI Area - {}", state.template_type))
                .with_inner_size([img_w as f32, img_h as f32 + 40.0_f32])
                .with_resizable(false);

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("roi_selection_window"),
                builder,
                |ctx, _class| {
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close_window = true;
                    }

                    theme::configure_theme(ctx);

                    egui::TopBottomPanel::bottom("roi_bottom_panel").show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Confirm ROI").clicked() {
                                if let Some(rect) = state.rect {
                                    save_rect = Some(rect);
                                }
                                close_window = true;
                            }

                            if ui.button("Cancel").clicked() {
                                close_window = true;
                            }

                            ui.separator();

                            if let Some(rect) = state.rect {
                                ui.monospace(format!(
                                    "x: {}  y: {}  w: {}  h: {}",
                                    rect.min.x.round().max(0.0_f32) as u32,
                                    rect.min.y.round().max(0.0_f32) as u32,
                                    rect.width().round().max(0.0_f32) as u32,
                                    rect.height().round().max(0.0_f32) as u32
                                ));
                            } else {
                                ui.label(
                                    theme::subtle(
                                        "Click and drag on the image to select the ROI area.",
                                    ),
                                );
                            }
                        });
                    });

                    egui::CentralPanel::default()
                        .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(0.0_f32))
                        .show(ctx, |ui| {
                            let image_path =
                                format!("file://{}", state.image_path.replace('\\', "/"));
                            let image = egui::Image::new(&image_path);

                            let response = ui.add(image.sense(egui::Sense::click_and_drag()));
                            let rect = response.rect;

                            // Handle drag
                            if response.drag_started() {
                                let press_pos = ctx
                                    .input(|i| i.pointer.press_origin())
                                    .or_else(|| response.interact_pointer_pos());
                                state.drag_start = press_pos;
                                state.rect = None;
                            }

                            if response.dragged() {
                                let start_pos = state
                                    .drag_start
                                    .or_else(|| ctx.input(|i| i.pointer.press_origin()));
                                let current_pos = ctx
                                    .input(|i| i.pointer.latest_pos())
                                    .or_else(|| response.interact_pointer_pos());

                                if let (Some(start), Some(curr)) = (start_pos, current_pos)
                                    && rect.width() > 0.0_f32
                                    && rect.height() > 0.0_f32
                                {
                                    let scale_to_img_x = img_w as f32 / rect.width();
                                    let scale_to_img_y = img_h as f32 / rect.height();

                                    let p1_x = (start.x - rect.min.x) * scale_to_img_x;
                                    let p1_y = (start.y - rect.min.y) * scale_to_img_y;
                                    let p2_x = (curr.x - rect.min.x) * scale_to_img_x;
                                    let p2_y = (curr.y - rect.min.y) * scale_to_img_y;

                                    let min_x = p1_x.min(p2_x).clamp(0.0_f32, img_w as f32);
                                    let min_y = p1_y.min(p2_y).clamp(0.0_f32, img_h as f32);
                                    let max_x = p1_x.max(p2_x).clamp(0.0_f32, img_w as f32);
                                    let max_y = p1_y.max(p2_y).clamp(0.0_f32, img_h as f32);

                                    state.rect = Some(egui::Rect::from_min_max(
                                        egui::pos2(min_x, min_y),
                                        egui::pos2(max_x, max_y),
                                    ));
                                }
                            }

                            // Draw rectangle mask and selection frame
                            let mask_color = egui::Color32::from_black_alpha(140);
                            if let Some(roi_rect_img) = state.rect {
                                if img_w > 0 && img_h > 0 {
                                    let scale_to_ui_x = rect.width() / img_w as f32;
                                    let scale_to_ui_y = rect.height() / img_h as f32;

                                    let abs_min = egui::pos2(
                                        rect.min.x + roi_rect_img.min.x * scale_to_ui_x,
                                        rect.min.y + roi_rect_img.min.y * scale_to_ui_y,
                                    );
                                    let abs_max = egui::pos2(
                                        rect.min.x + roi_rect_img.max.x * scale_to_ui_x,
                                        rect.min.y + roi_rect_img.max.y * scale_to_ui_y,
                                    );
                                    let abs_rect = egui::Rect::from_min_max(abs_min, abs_max);

                                    let p = ui.painter();
                                    p.rect_filled(
                                        egui::Rect::from_min_max(
                                            rect.min,
                                            egui::pos2(rect.max.x, abs_rect.min.y),
                                        ),
                                        0.0_f32,
                                        mask_color,
                                    );
                                    p.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x, abs_rect.max.y),
                                            rect.max,
                                        ),
                                        0.0_f32,
                                        mask_color,
                                    );
                                    p.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x, abs_rect.min.y),
                                            egui::pos2(abs_rect.min.x, abs_rect.max.y),
                                        ),
                                        0.0_f32,
                                        mask_color,
                                    );
                                    p.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(abs_rect.max.x, abs_rect.min.y),
                                            egui::pos2(rect.max.x, abs_rect.max.y),
                                        ),
                                        0.0_f32,
                                        mask_color,
                                    );

                                    p.rect_stroke(
                                        abs_rect,
                                        0.0_f32,
                                        egui::Stroke::new(2.0_f32, egui::Color32::LIGHT_BLUE),
                                    );
                                }
                            } else {
                                ui.painter().rect_filled(rect, 0.0_f32, mask_color);
                            }
                        });
                },
            );
        }

        if let Some(rect) = save_rect
            && let Some(state) = &self.roi_selection
        {
            let x = rect.min.x.round().max(0.0_f32) as u32;
            let y = rect.min.y.round().max(0.0_f32) as u32;
            let w = rect.width().round().max(0.0_f32) as u32;
            let h = rect.height().round().max(0.0_f32) as u32;
            let roi = [x, y, w, h];

            match state.template_type.as_str() {
                "start" => self.config.start_roi = roi,
                "end" => self.config.end_roi = roi,
                "win" => self.config.win_roi = roi,
                "lose" => self.config.lose_roi = roi,
                _ => {}
            }
            self.config.save();
        }

        if close_window {
            self.roi_selection = None;
        }
    }
}

impl eframe::App for GgstClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle background task messages
        if let TaskStatus::Running {
            ref mut phase,
            ref mut progress,
            ref mut message,
            ref mut segments_count,
            ref rx,
            ..
        } = self.task_status
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));

            let mut next_status = None;
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    IpcMessage::Log { message: m } => {
                        *message = m;
                    }
                    IpcMessage::Progress {
                        phase: p,
                        percentage,
                        message: m,
                        ..
                    } => {
                        let (phase_title, overall_progress) = match p.as_str() {
                            "scan" => (
                                "Scanning Video...".to_string(),
                                (percentage * 0.5_f32).clamp(0.0_f32, 0.5_f32),
                            ),
                            "export" => (
                                "Exporting Clips...".to_string(),
                                (0.5_f32 + percentage * 0.5_f32).clamp(0.5_f32, 1.0_f32),
                            ),
                            _ => (p, percentage),
                        };
                        *phase = phase_title;
                        *progress = overall_progress;
                        *message = m;
                    }
                    IpcMessage::SegmentDetected { index, .. } => {
                        *segments_count = index;
                    }
                    IpcMessage::Done {
                        segments_count: count,
                        calc_time_secs,
                        export_time_secs,
                        total_time_secs,
                        output_dir,
                        ..
                    } => {
                        next_status = Some(TaskStatus::Finished {
                            segments_count: count,
                            calc_time: format_duration(calc_time_secs),
                            export_time: format_duration(export_time_secs),
                            total_time: format_duration(total_time_secs),
                            output_dir,
                        });
                        break;
                    }
                    IpcMessage::Error { message: err } => {
                        next_status = Some(TaskStatus::Error { message: err });
                        break;
                    }
                }
            }
            if let Some(status) = next_status {
                self.task_status = status;
            }
        }

        // Handle Drag & Drop video files
        let is_dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if matches!(self.task_status, TaskStatus::Idle) {
            let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
            if let Some(file) = dropped_files.first() {
                if let Some(path) = &file.path {
                    let path_str = path.to_string_lossy().to_string();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if matches!(
                        ext.as_str(),
                        "mp4" | "mkv" | "avi" | "mov" | "flv" | "wmv" | "webm"
                    ) {
                        self.run_cli(&path_str);
                    }
                }
            }
        }

        let mut transition_to_idle = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!self.show_settings, |ui| {
                // Header Area
                self.render_header(ui);
                ui.separator();
                ui.add_space(4.0_f32);

                // Content Area
                match &mut self.task_status {
                    TaskStatus::Idle => {
                        let available_size = ui.available_size();
                        let (rect, _resp) =
                            ui.allocate_exact_size(available_size, egui::Sense::hover());

                        let stroke = if is_dragging {
                            egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(100, 150, 240))
                        } else {
                            ui.visuals().widgets.noninteractive.bg_stroke
                        };

                        let bg = ui.visuals().widgets.noninteractive.bg_fill;

                        ui.painter().rect(
                            rect,
                            egui::Rounding::same(6.0_f32),
                            bg,
                            stroke,
                        );

                        let mut request_pick = false;
                        let content_height = 82.0_f32;
                        let top_space = ((rect.height() - content_height) / 2.0_f32).max(0.0_f32);

                        ui.allocate_new_ui(
                            egui::UiBuilder::new().max_rect(rect),
                            |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(top_space);
                                    ui.heading("Drop match video file here");
                                    ui.add_space(4.0_f32);
                                    ui.label(
                                        theme::subtle(
                                            "Supported formats: MP4, MKV, AVI, MOV, WebM",
                                        ),
                                    );

                                    ui.add_space(12.0_f32);

                                    if ui.button("Select Video...").clicked() {
                                        request_pick = true;
                                    }
                                });
                            },
                        );

                        if request_pick {
                            if let Some(path) = FileDialog::new()
                                .add_filter(
                                    "Video Files",
                                    &["mp4", "mkv", "avi", "mov", "flv", "wmv", "webm"],
                                )
                                .pick_file()
                            {
                                self.run_cli(path.to_string_lossy().as_ref());
                            }
                        }
                    }
                    TaskStatus::Running {
                        phase,
                        progress,
                        message,
                        segments_count,
                        cancel_flag,
                        ..
                    } => {
                        ui.group(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(12.0_f32);

                                ui.horizontal(|ui| {
                                    ui.add_space(
                                        (ui.available_width() - 180.0_f32).max(0.0_f32) / 2.0_f32,
                                    );
                                    ui.spinner();
                                    ui.heading(&*phase);
                                });

                                ui.add_space(10.0_f32);

                                let progress_bar = egui::ProgressBar::new(*progress)
                                    .show_percentage()
                                    .animate(true);
                                ui.add_sized(
                                    [ui.available_width() - 32.0_f32, 16.0_f32],
                                    progress_bar,
                                );

                                ui.add_space(8.0_f32);
                                ui.label(&*message);

                                if *segments_count > 0 {
                                    ui.add_space(4.0_f32);
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Found {} clip{} so far",
                                            segments_count,
                                            if *segments_count > 1 { "s" } else { "" }
                                        ))
                                        .strong(),
                                    );
                                }

                                ui.add_space(14.0_f32);

                                if ui.button("Cancel").clicked() {
                                    cancel_flag.store(true, Ordering::SeqCst);
                                    transition_to_idle = true;
                                }

                                ui.add_space(8.0_f32);
                            });
                        });
                    }
                    TaskStatus::Finished {
                        segments_count,
                        calc_time,
                        export_time,
                        total_time,
                        output_dir,
                    } => {
                        ui.group(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(12.0_f32);
                                ui.heading("Processing Completed");

                                ui.add_space(8.0_f32);
                                ui.label(format!("Clips: {}", segments_count));
                                ui.label(
                                    theme::subtle(format!(
                                        "Total: {} (Scan: {}, Export: {})",
                                        total_time, calc_time, export_time
                                    )),
                                );

                                ui.add_space(14.0_f32);

                                let out_path = PathBuf::from(&*output_dir);
                                ui.horizontal(|ui| {
                                    ui.add_space(
                                        (ui.available_width() - 180.0_f32).max(0.0_f32) / 2.0_f32,
                                    );
                                    if ui.button("Open Folder").clicked() {
                                        open_file(&out_path);
                                    }
                                    if ui.button("Done").clicked() {
                                        transition_to_idle = true;
                                    }
                                });

                                ui.add_space(8.0_f32);
                            });
                        });
                    }
                    TaskStatus::Error { message } => {
                        ui.group(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.add_space(12.0_f32);
                                ui.colored_label(egui::Color32::RED, "Analysis Error");

                                ui.add_space(8.0_f32);
                                ui.label(&*message);

                                ui.add_space(14.0_f32);
                                if ui.button("Back").clicked() {
                                    transition_to_idle = true;
                                }

                                ui.add_space(8.0_f32);
                            });
                        });
                    }
                }
            });
        });

        if transition_to_idle {
            self.task_status = TaskStatus::Idle;
        }

        if self.show_settings {
            self.render_settings_window(ctx);
        }

        if self.roi_selection.is_some() {
            self.render_roi_window(ctx);
        }
    }
}

fn load_icon() -> egui::IconData {
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(include_bytes!("../../assets/icon.png"))
            .expect("Failed to open icon path")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };

    egui::IconData {
        rgba: icon_rgba,
        width: icon_width,
        height: icon_height,
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0_f32, 200.0_f32])
            .with_title("ggst-clipper")
            .with_resizable(false)
            .with_icon(load_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "ggst-clipper",
        native_options,
        Box::new(|cc| Ok(Box::new(GgstClipApp::new(cc)))),
    )
}
