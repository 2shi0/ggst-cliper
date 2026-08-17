#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;

use config::AppConfig;
use eframe::egui;
use rfd::FileDialog;
use std::process::Command;

struct RoiSelectionState {
    template_type: String,
    image_path: String,
    rect: Option<egui::Rect>,
    drag_start: Option<egui::Pos2>,
}

struct GgstClipApp {
    config: AppConfig,
    show_settings: bool,
    status_message: String,
    roi_selection: Option<RoiSelectionState>,
    saved_notification_time: Option<std::time::Instant>,
}

impl GgstClipApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let config = AppConfig::load();
        Self {
            config,
            show_settings: false,
            status_message: String::new(),
            roi_selection: None,
            saved_notification_time: None,
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
        if self.config.detect_win_loss {
            cmd.arg("--win-template").arg(&self.config.win_template);
            cmd.arg("--lose-template").arg(&self.config.lose_template);
        } else {
            cmd.arg("--win-template").arg("");
            cmd.arg("--lose-template").arg("");
        }
        
        cmd.arg("--threshold").arg(self.config.threshold.to_string());
        cmd.arg("--step-frames").arg(self.config.step_frames.to_string());
        cmd.arg("--start-offset").arg(self.config.start_offset.to_string());
        cmd.arg("--end-offset").arg(self.config.end_offset.to_string());
        cmd.arg("--win-offset").arg(self.config.win_offset.to_string());
        cmd.arg("--detect-characters").arg(self.config.detect_characters.to_string());
        
        let format_roi = |r: [u32; 4]| format!("{},{},{},{}", r[0], r[1], r[2], r[3]);
        cmd.arg("--start-roi").arg(format_roi(self.config.start_roi));
        cmd.arg("--end-roi").arg(format_roi(self.config.end_roi));
        if self.config.detect_win_loss {
            cmd.arg("--win-roi").arg(format_roi(self.config.win_roi));
            cmd.arg("--lose-roi").arg(format_roi(self.config.lose_roi));
        } else {
            cmd.arg("--win-roi").arg("0,0,0,0");
            cmd.arg("--lose-roi").arg("0,0,0,0");
        }

        match cmd.spawn() {
            Ok(_) => {
                self.status_message = String::new();
            }
            Err(e) => {
                self.status_message = format!("Failed to start {}: {}", cli_path.display(), e);
            }
        }
    }

    fn render_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut show = self.show_settings;
        let prev_config = self.config.clone();
        
        let builder = egui::ViewportBuilder::default()
            .with_title("Settings")
            .with_inner_size([350.0, 450.0])
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

                egui::CentralPanel::default()
                    .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(0.0))
                    .show(ctx, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::none().inner_margin(8.0).show(ui, |ui| {
                            let instant_tooltip = |ui: &mut egui::Ui, text: &str| {
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                                let center = rect.center();
                                let is_hovered = response.hovered();
                                let stroke_color = if is_hovered {
                                    egui::Color32::from_rgb(200, 200, 200)
                                } else {
                                    egui::Color32::from_rgb(130, 130, 130)
                                };
                                let text_color = if is_hovered {
                                    egui::Color32::from_rgb(240, 240, 240)
                                } else {
                                    egui::Color32::from_rgb(150, 150, 150)
                                };

                                ui.painter().circle_stroke(center, 6.5, egui::Stroke::new(1.0_f32, stroke_color));
                                ui.painter().circle_filled(
                                    egui::pos2(center.x, center.y - 2.8),
                                    0.85,
                                    text_color,
                                );
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(center.x, center.y - 0.7),
                                        egui::pos2(center.x, center.y + 3.0),
                                    ],
                                    egui::Stroke::new(1.5_f32, text_color),
                                );

                                if is_hovered {
                                    if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
                                        egui::show_tooltip_at(
                                            ui.ctx(),
                                            ui.layer_id(),
                                            response.id,
                                            pos + egui::vec2(16.0, 0.0),
                                            |ui| {
                                                ui.label(text);
                                            }
                                        );
                                    }
                                }
                            };

                            ui.label(egui::RichText::new("Output Directory").strong());
                            let display_text = if self.config.output_dir.is_empty() {
                                "Select Directory...".to_string()
                            } else {
                                self.config.output_dir.clone()
                            };
                            if ui.button(display_text).clicked() {
                                if let Some(path) = FileDialog::new().pick_folder() {
                                    self.config.output_dir = path.to_string_lossy().to_string();
                                }
                            }

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(5.0);

                            ui.label(egui::RichText::new("Clip Range Detection").strong());
                            
                            let mut open_roi_selection = None;
                            let mut pick_image = |ui: &mut egui::Ui, label: &str, template_type: &str, field: &mut String, roi: &mut [u32; 4], default_name: &str| {
                                let file_exists = !field.is_empty() && std::path::Path::new(field).exists();
                                ui.allocate_ui_with_layout(egui::vec2(300.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.label(label);
                                    if file_exists {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button("Remove").clicked() {
                                                let image_path = format!("file://{}", field.replace('\\', "/"));
                                                let _ = std::fs::remove_file(&*field);
                                                ui.ctx().forget_image(&image_path);
                                                field.clear();
                                                *roi = [0, 0, 0, 0];
                                            }
                                            if ui.button("Select Area").clicked() {
                                                open_roi_selection = Some(RoiSelectionState {
                                                    template_type: template_type.to_string(),
                                                    image_path: field.clone(),
                                                    rect: if roi[2] > 0 && roi[3] > 0 {
                                                        Some(egui::Rect::from_min_size(
                                                            egui::pos2(roi[0] as f32, roi[1] as f32),
                                                            egui::vec2(roi[2] as f32, roi[3] as f32),
                                                        ))
                                                    } else {
                                                        None
                                                    },
                                                    drag_start: None,
                                                });
                                            }
                                        });
                                    }
                                });

                                let mut request_pick = false;
                                if !file_exists {
                                    if ui.add_sized([171.0, 96.0], egui::Button::new(egui::RichText::new("Select Image").color(egui::Color32::RED))).clicked() {
                                        request_pick = true;
                                    }
                                } else {
                                    let image_path = format!("file://{}", field.replace('\\', "/"));
                                    let img_response = ui.add(
                                        egui::Image::new(&image_path)
                                            .fit_to_exact_size(egui::vec2(171.0, 96.0))
                                            .sense(egui::Sense::click())
                                    );
                                    if roi[2] > 0 && roi[3] > 0 {
                                        if let Ok((img_w, img_h)) = image::image_dimensions(&*field) {
                                            let scale_x = 171.0 / img_w as f32;
                                            let scale_y = 96.0 / img_h as f32;
                                            let rx = img_response.rect.min.x + roi[0] as f32 * scale_x;
                                            let ry = img_response.rect.min.y + roi[1] as f32 * scale_y;
                                            let rw = roi[2] as f32 * scale_x;
                                            let rh = roi[3] as f32 * scale_y;
                                            
                                             let c = egui::Color32::from_white_alpha(80);
                                            let img_rect = img_response.rect;
                                            let roi_rect = egui::Rect::from_min_size(egui::pos2(rx, ry), egui::vec2(rw, rh));
                                            
                                            ui.painter().rect_filled(egui::Rect::from_min_max(img_rect.min, egui::pos2(img_rect.max.x, roi_rect.min.y)), 0.0, c); // Top
                                            ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(img_rect.min.x, roi_rect.max.y), img_rect.max), 0.0, c); // Bottom
                                            ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(img_rect.min.x, roi_rect.min.y), egui::pos2(roi_rect.min.x, roi_rect.max.y)), 0.0, c); // Left
                                            ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(roi_rect.max.x, roi_rect.min.y), egui::pos2(img_rect.max.x, roi_rect.max.y)), 0.0, c); // Right
                                        }
                                    }
                                    if img_response.clicked() {
                                        request_pick = true;
                                    }
                                }

                                if file_exists {
                                    ui.label(format!("ROI: [{}, {}, {}, {}]", roi[0], roi[1], roi[2], roi[3]));
                                }

                                if request_pick {
                                    if let Some(path) = FileDialog::new()
                                        .add_filter("Images", &["png", "jpg", "jpeg"])
                                        .pick_file()
                                    {
                                        let config_dir = AppConfig::config_dir();
                                        if !config_dir.exists() {
                                            let _ = std::fs::create_dir_all(&config_dir);
                                        }
                                        let dest_path = config_dir.join(default_name);
                                        if let Ok(_) = std::fs::copy(&path, &dest_path) {
                                            *field = dest_path.to_string_lossy().to_string();
                                            ui.ctx().forget_image(&format!("file://{}", field.replace('\\', "/")));
                                            ui.ctx().request_repaint();
                                        } else {
                                            *field = path.to_string_lossy().to_string();
                                        }
                                        *roi = [0, 0, 0, 0]; // Reset ROI on new image
                                    }
                                }
                                ui.add_space(5.0);
                            };

                            pick_image(ui, "Start Template:", "start", &mut self.config.start_template, &mut self.config.start_roi, "start.png");
                            ui.horizontal(|ui| {
                                ui.label("Start Offset:");
                                ui.add(egui::DragValue::new(&mut self.config.start_offset).speed(1));
                                instant_tooltip(ui, "Frame offset for the start of the clip.\nA negative value starts the clip slightly before the match.");
                            });

                            ui.add_space(10.0);

                            pick_image(ui, "End Template:", "end", &mut self.config.end_template, &mut self.config.end_roi, "end.png");
                            ui.horizontal(|ui| {
                                ui.label("End Offset:");
                                ui.add(egui::DragValue::new(&mut self.config.end_offset).speed(1));
                                instant_tooltip(ui, "Frame offset for the end of the clip.\nA positive value ends the clip slightly after the match.");
                            });

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(5.0);

                            ui.checkbox(&mut self.config.detect_win_loss, "Detect Win/Lose");
                            if self.config.detect_win_loss {
                                pick_image(ui, "Win Template:", "win", &mut self.config.win_template, &mut self.config.win_roi, "win.png");
                                pick_image(ui, "Lose Template:", "lose", &mut self.config.lose_template, &mut self.config.lose_roi, "lose.png");
                                ui.horizontal(|ui| {
                                    ui.label("Search Frames:");
                                    ui.add(egui::DragValue::new(&mut self.config.win_offset).speed(1));
                                    instant_tooltip(ui, "Number of frames to search for win/lose template match after the end match.");
                                });
                            }

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.config.detect_characters, "Detect Character Names (GGST only)");
                                instant_tooltip(ui, "Detect 1P/2P character names via OCR 1s after start match\nand include them in exported video filenames.");
                            });

                            if let Some(state) = open_roi_selection {
                                self.roi_selection = Some(state);
                            }

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(5.0);

                            ui.label(egui::RichText::new("Detection Parameters").strong());
                            egui::Grid::new("parameters_grid")
                                .num_columns(3)
                                .spacing([10.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label("Threshold:");
                                    ui.add(egui::DragValue::new(&mut self.config.threshold).speed(0.01).range(0.0..=1.0));
                                    instant_tooltip(ui, "Image matching threshold (0.0 to 1.0).\nLower values require a stricter match.");
                                    ui.end_row();

                                    ui.label("Step Frames:");
                                    ui.add(egui::DragValue::new(&mut self.config.step_frames).speed(1));
                                    instant_tooltip(ui, "Interval of frames to skip for matching.\nHigher values speed up processing but may lower accuracy.");
                                    ui.end_row();
                                });

                            ui.add_space(10.0);
                        });
                    });
                });

                if let Some(saved_time) = self.saved_notification_time {
                    let elapsed = saved_time.elapsed().as_secs_f32();
                    let total_duration = 2.0;
                    let fade_start = 1.2;

                    if elapsed < total_duration {
                        let alpha = if elapsed < fade_start {
                            1.0
                        } else {
                            (1.0 - (elapsed - fade_start) / (total_duration - fade_start)).clamp(0.0, 1.0)
                        };

                        let bg_color = egui::Color32::from_rgba_unmultiplied(35, 80, 45, (230.0 * alpha) as u8);
                        let stroke_color = egui::Color32::from_rgba_unmultiplied(60, 160, 80, (240.0 * alpha) as u8);
                        let text_color = egui::Color32::from_rgba_unmultiplied(220, 255, 220, (255.0 * alpha) as u8);

                        egui::Area::new(egui::Id::new("settings_saved_snackbar"))
                            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
                            .order(egui::Order::Foreground)
                            .interactable(false)
                            .show(ctx, |ui| {
                                egui::Frame::none()
                                    .fill(bg_color)
                                    .stroke(egui::Stroke::new(1.0_f32, stroke_color))
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 5.0;
                                            let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                                            let p = ui.painter();
                                            let p1 = egui::pos2(icon_rect.min.x + 1.5, icon_rect.center().y);
                                            let p2 = egui::pos2(icon_rect.min.x + 4.5, icon_rect.max.y - 2.0);
                                            let p3 = egui::pos2(icon_rect.max.x - 1.5, icon_rect.min.y + 2.0);
                                            p.line_segment([p1, p2], egui::Stroke::new(1.8_f32, text_color));
                                            p.line_segment([p2, p3], egui::Stroke::new(1.8_f32, text_color));
                                            ui.label(egui::RichText::new("Saved").color(text_color).size(13.0).strong());
                                        });
                                    });
                            });

                        ctx.request_repaint_after(std::time::Duration::from_millis(16));
                    }
                }
            }
        );

        if self.config != prev_config {
            self.config.save();
            self.saved_notification_time = Some(std::time::Instant::now());
        }

        self.show_settings = show;
    }

    fn render_roi_window(&mut self, ctx: &egui::Context) {
        let mut close_window = false;
        let mut save_rect = None;

        if let Some(state) = &mut self.roi_selection {
            let (img_w, img_h) = image::image_dimensions(&state.image_path).unwrap_or((800, 600));

            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Select Area for {}", state.template_type))
                .with_inner_size([img_w as f32, img_h as f32 + 35.0])
                .with_resizable(false);

            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("roi_selection_window"),
                builder,
                |ctx, _class| {
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close_window = true;
                    }

                    egui::TopBottomPanel::bottom("roi_bottom_panel").show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Confirm").clicked() {
                                if let Some(rect) = state.rect {
                                    save_rect = Some(rect);
                                }
                                close_window = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_window = true;
                            }
                            if let Some(rect) = state.rect {
                                ui.label(format!("Selected: x: {}, y: {}, w: {}, h: {}", 
                                    rect.min.x.round().max(0.0) as u32, 
                                    rect.min.y.round().max(0.0) as u32, 
                                    rect.width().round().max(0.0) as u32, 
                                    rect.height().round().max(0.0) as u32));
                            } else {
                                ui.label("Drag on the image to select an area.");
                            }
                        });
                    });

                    egui::CentralPanel::default()
                        .frame(egui::Frame::central_panel(&ctx.style()).inner_margin(0.0))
                        .show(ctx, |ui| {
                            let image_path = format!("file://{}", state.image_path.replace('\\', "/"));
                            let image = egui::Image::new(&image_path);
                            
                            let response = ui.add(image.sense(egui::Sense::click_and_drag()));
                            let rect = response.rect;

                            // Handle drag
                            if response.drag_started() {
                                let press_pos = ctx.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
                                state.drag_start = press_pos;
                                state.rect = None;
                            }

                            if response.dragged() {
                                let start_pos = state.drag_start.or_else(|| ctx.input(|i| i.pointer.press_origin()));
                                let current_pos = ctx.input(|i| i.pointer.latest_pos()).or_else(|| response.interact_pointer_pos());

                                if let (Some(start), Some(curr)) = (start_pos, current_pos) {
                                    if rect.width() > 0.0 && rect.height() > 0.0 {
                                        let scale_to_img_x = img_w as f32 / rect.width();
                                        let scale_to_img_y = img_h as f32 / rect.height();

                                        let p1_x = (start.x - rect.min.x) * scale_to_img_x;
                                        let p1_y = (start.y - rect.min.y) * scale_to_img_y;
                                        let p2_x = (curr.x - rect.min.x) * scale_to_img_x;
                                        let p2_y = (curr.y - rect.min.y) * scale_to_img_y;

                                        let min_x = p1_x.min(p2_x).clamp(0.0, img_w as f32);
                                        let min_y = p1_y.min(p2_y).clamp(0.0, img_h as f32);
                                        let max_x = p1_x.max(p2_x).clamp(0.0, img_w as f32);
                                        let max_y = p1_y.max(p2_y).clamp(0.0, img_h as f32);

                                        state.rect = Some(egui::Rect::from_min_max(
                                            egui::pos2(min_x, min_y),
                                            egui::pos2(max_x, max_y),
                                        ));
                                    }
                                }
                            }

                            // Draw rectangle
                            let c = egui::Color32::from_white_alpha(80);
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

                                    ui.painter().rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, abs_rect.min.y)), 0.0, c); // Top
                                    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.min.x, abs_rect.max.y), rect.max), 0.0, c); // Bottom
                                    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(rect.min.x, abs_rect.min.y), egui::pos2(abs_rect.min.x, abs_rect.max.y)), 0.0, c); // Left
                                    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(abs_rect.max.x, abs_rect.min.y), egui::pos2(rect.max.x, abs_rect.max.y)), 0.0, c); // Right

                                    ui.painter().rect_stroke(
                                        abs_rect,
                                        0.0,
                                        egui::Stroke::new(2.0_f32, egui::Color32::RED),
                                    );
                                }
                            } else {
                                ui.painter().rect_filled(rect, 0.0, c);
                            }
                        });
                }
            );
        }

        if let Some(rect) = save_rect {
            if let Some(state) = &self.roi_selection {
                let x = rect.min.x.round().max(0.0) as u32;
                let y = rect.min.y.round().max(0.0) as u32;
                let w = rect.width().round().max(0.0) as u32;
                let h = rect.height().round().max(0.0) as u32;
                let roi = [x, y, w, h];

                match state.template_type.as_str() {
                    "start" => self.config.start_roi = roi,
                    "end" => self.config.end_roi = roi,
                    "win" => self.config.win_roi = roi,
                    "lose" => self.config.lose_roi = roi,
                    _ => {}
                }
                self.config.save();
                self.saved_notification_time = Some(std::time::Instant::now());
            }
        }

        if close_window {
            self.roi_selection = None;
        }
    }
}

impl eframe::App for GgstClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!self.show_settings, |ui| {
                ui.vertical_centered(|ui| {
                ui.add_space(40.0);

                let button_size = [250.0, 50.0];

                if ui.add_sized(button_size, egui::Button::new(egui::RichText::new("Select Video").size(24.0))).clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Video Files", &["mp4", "mkv", "avi", "mov"])
                        .pick_file()
                    {
                        self.run_cli(path.to_string_lossy().as_ref());
                    }
                }

                ui.add_space(20.0);

                if ui.add_sized(button_size, egui::Button::new(egui::RichText::new("Settings").size(24.0))).clicked() {
                    self.show_settings = true;
                }

                ui.add_space(20.0);
                if !self.status_message.is_empty() {
                    ui.label(&self.status_message);
                }
            });

            // "Source code (Github)" at bottom right
            ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.add_space(5.0);
                    ui.hyperlink_to("Source code (Github)", "https://github.com/2shi0/ggst-clipper");
                });
            });
            }); // close add_enabled_ui
        });

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
            .with_inner_size([400.0, 250.0])
            .with_title("ggst-clipper")
            .with_resizable(false)
            .with_icon(load_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "ggst-clipper",
        native_options,
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.override_text_color = Some(egui::Color32::from_gray(245));
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(GgstClipApp::new(cc)))
        }),
    )
}

