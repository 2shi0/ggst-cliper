use eframe::egui::{self, Color32, RichText};

pub const TEXT_WHITE: Color32 = Color32::from_rgb(250, 250, 250);
pub const TEXT_SUBTLE: Color32 = Color32::from_rgb(200, 205, 215);

pub fn subtle(text: impl Into<String>) -> RichText {
    RichText::new(text).color(TEXT_SUBTLE)
}

pub fn configure_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    // 基本テキスト色: 白
    visuals.override_text_color = Some(TEXT_WHITE);

    visuals.widgets.noninteractive.fg_stroke.color = TEXT_WHITE;
    visuals.widgets.inactive.fg_stroke.color = TEXT_WHITE;
    visuals.widgets.hovered.fg_stroke.color = TEXT_WHITE;
    visuals.widgets.active.fg_stroke.color = TEXT_WHITE;
    visuals.widgets.open.fg_stroke.color = TEXT_WHITE;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0_f32, 6.0_f32);
    style.spacing.button_padding = egui::vec2(8.0_f32, 4.0_f32);
    ctx.set_style(style);
}
