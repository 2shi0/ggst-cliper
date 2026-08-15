#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;

use config::AppConfig;
use eframe::egui;
use rfd::FileDialog;
use std::path::Path;
use std::process::Command;

struct GgstClipApp {
    config: AppConfig,
    show_settings: bool,
    status_message: String,
}

impl GgstClipApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        Self {
            config: AppConfig::load(),
            show_settings: false,
            status_message: String::new(),
        }
    }

    fn run_cli(&mut self, video_path: &str) {
        let cli_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("ggst-clip.exe")))
            .unwrap_or_else(|| std::path::PathBuf::from("ggst-clip.exe"));
        let mut cmd = Command::new(&cli_path);
        cmd.arg("-i").arg(video_path);

        if !self.config.output_dir.is_empty() {
            cmd.arg("-o").arg(&self.config.output_dir);
        }

        cmd.arg("--start-template").arg(&self.config.start_template);
        cmd.arg("--end-template").arg(&self.config.end_template);
        cmd.arg("--win-template").arg(&self.config.win_template);
        cmd.arg("--lose-template").arg(&self.config.lose_template);
        
        cmd.arg("--threshold").arg(self.config.threshold.to_string());
        cmd.arg("--step-frames").arg(self.config.step_frames.to_string());
        cmd.arg("--start-offset").arg(self.config.start_offset.to_string());
        cmd.arg("--end-offset").arg(self.config.end_offset.to_string());

        match cmd.spawn() {
            Ok(_) => {
                self.status_message = format!("CLI process started from: {}", cli_path.display());
            }
            Err(e) => {
                self.status_message = format!("Failed to start {}: {}", cli_path.display(), e);
            }
        }
    }

    fn copy_image_if_needed(path: &str, target_dir: &Path, default_name: &str) -> String {
        let p = Path::new(path);
        if !p.exists() || p.parent() == Some(target_dir) {
            return path.to_string(); // Already in target dir or doesn't exist (maybe relative default)
        }
        
        let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or(default_name);
        let dest_path = target_dir.join(file_name);
        
        if let Ok(_) = std::fs::copy(p, &dest_path) {
            dest_path.to_string_lossy().to_string()
        } else {
            path.to_string()
        }
    }

    fn save_settings(&mut self) {
        let config_dir = AppConfig::config_dir();
        if !config_dir.exists() {
            let _ = std::fs::create_dir_all(&config_dir);
        }

        self.config.start_template = Self::copy_image_if_needed(&self.config.start_template, &config_dir, "start.png");
        self.config.end_template = Self::copy_image_if_needed(&self.config.end_template, &config_dir, "end.png");
        self.config.win_template = Self::copy_image_if_needed(&self.config.win_template, &config_dir, "win.png");
        self.config.lose_template = Self::copy_image_if_needed(&self.config.lose_template, &config_dir, "lose.png");

        self.config.save();
        self.show_settings = false;
    }

    fn render_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut show = self.show_settings;
        
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
                            ui.label("Output Directory:");
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
                            ui.label("Templates:");
                            
                            let mut pick_image = |label: &str, field: &mut String, default_name: &str| {
                                ui.allocate_ui_with_layout(egui::vec2(171.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.label(label);
                                    if !field.is_empty() {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.button("Remove").clicked() {
                                                let image_path = format!("file://{}", field.replace('\\', "/"));
                                                let _ = std::fs::remove_file(&*field);
                                                ui.ctx().forget_image(&image_path);
                                                field.clear();
                                            }
                                        });
                                    }
                                });

                                let mut request_pick = false;
                                if field.is_empty() {
                                    if ui.add_sized([171.0, 96.0], egui::Button::new(egui::RichText::new("⚠ Select Image").color(egui::Color32::RED))).clicked() {
                                        request_pick = true;
                                    }
                                } else {
                                    let image_path = format!("file://{}", field.replace('\\', "/"));
                                    let img_response = ui.add(
                                        egui::Image::new(&image_path)
                                            .fit_to_exact_size(egui::vec2(171.0, 96.0))
                                            .sense(egui::Sense::click())
                                    );
                                    if img_response.clicked() {
                                        request_pick = true;
                                    }
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
                                    }
                                }
                                ui.add_space(5.0);
                            };

                            pick_image("Start Template:", &mut self.config.start_template, "start.png");
                            pick_image("End Template:", &mut self.config.end_template, "end.png");
                            pick_image("Win Template:", &mut self.config.win_template, "win.png");
                            pick_image("Lose Template:", &mut self.config.lose_template, "lose.png");

                            ui.add_space(10.0);
                            ui.label("Parameters:");
                            egui::Grid::new("parameters_grid")
                                .num_columns(3)
                                .spacing([10.0, 4.0])
                                .show(ui, |ui| {
                                    let instant_tooltip = |ui: &mut egui::Ui, text: &str| {
                                        let r = ui.label("ℹ");
                                        if r.hovered() {
                                            if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
                                                egui::show_tooltip_at(
                                                    ui.ctx(),
                                                    ui.layer_id(),
                                                    r.id,
                                                    pos + egui::vec2(16.0, 0.0),
                                                    |ui| {
                                                        ui.label(text);
                                                    }
                                                );
                                            }
                                        }
                                    };

                                    ui.label("Threshold:");
                                    ui.add(egui::DragValue::new(&mut self.config.threshold).speed(0.01).range(0.0..=1.0));
                                    instant_tooltip(ui, "Image matching threshold (0.0 to 1.0).\nLower values require a stricter match.");
                                    ui.end_row();

                                    ui.label("Step Frames:");
                                    ui.add(egui::DragValue::new(&mut self.config.step_frames).speed(1));
                                    instant_tooltip(ui, "Interval of frames to skip for matching.\nHigher values speed up processing but may lower accuracy.");
                                    ui.end_row();

                                    ui.label("Start Offset:");
                                    ui.add(egui::DragValue::new(&mut self.config.start_offset).speed(1));
                                    instant_tooltip(ui, "Frame offset for the start of the clip.\nA negative value starts the clip slightly before the match.");
                                    ui.end_row();

                                    ui.label("End Offset:");
                                    ui.add(egui::DragValue::new(&mut self.config.end_offset).speed(1));
                                    instant_tooltip(ui, "Frame offset for the end of the clip.\nA positive value ends the clip slightly after the match.");
                                    ui.end_row();
                                });

                            ui.add_space(20.0);
                            ui.horizontal(|ui| {
                                if ui.button("Save").clicked() {
                                    self.save_settings();
                                    show = false;
                                }
                                if ui.button("Cancel").clicked() {
                                    self.config = AppConfig::load(); // Revert changes
                                    show = false;
                                }
                            });
                        });
                    });
                });
            }
        );

        self.show_settings = show;
    }
}

impl eframe::App for GgstClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!self.show_settings, |ui| {
                ui.vertical_centered(|ui| {
                ui.add_space(40.0);

                let button_size = [250.0, 50.0];

                if ui.add_sized(button_size, egui::Button::new("Select Video")).clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Video Files", &["mp4", "mkv", "avi", "mov"])
                        .pick_file()
                    {
                        self.run_cli(path.to_string_lossy().as_ref());
                    }
                }

                ui.add_space(20.0);

                if ui.add_sized(button_size, egui::Button::new("Settings")).clicked() {
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
                    ui.hyperlink_to("Source code (Github)", "https://github.com/");
                });
            });
            }); // close add_enabled_ui
        });

        if self.show_settings {
            self.render_settings_window(ctx);
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
            .with_title("ggst-clip")
            .with_resizable(false)
            .with_icon(load_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "ggst-clip",
        native_options,
        Box::new(|cc| Ok(Box::new(GgstClipApp::new(cc)))),
    )
}

