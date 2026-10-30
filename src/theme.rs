use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};

pub const BG: Color32 = Color32::from_rgb(23, 24, 28);
pub const CARD: Color32 = Color32::from_rgb(31, 33, 39);
pub const BORDER: Color32 = Color32::from_rgb(62, 66, 77);
pub const ACCENT: Color32 = Color32::from_rgb(151, 175, 220);
pub const SELECTED: Color32 = Color32::from_rgb(47, 58, 80);
pub const TEXT: Color32 = Color32::from_rgb(235, 237, 242);
pub const MUTED: Color32 = Color32::from_rgb(156, 162, 177);
pub const AMBER: Color32 = Color32::from_rgb(224, 185, 139);

pub fn apply(ctx: &egui::Context) {
    // Prefer Windows' own UI font; no proprietary font is bundled.
    if let Some(windows) = std::env::var_os("WINDIR")
        && let Ok(bytes) =
            std::fs::read(std::path::PathBuf::from(windows).join("Fonts/segoeui.ttf"))
    {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "windows-ui".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "windows-ui".into());
        ctx.set_fonts(fonts);
    }
    ctx.set_theme(egui::Theme::Dark);
    ctx.send_viewport_cmd_to(
        egui::ViewportId::ROOT,
        egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark),
    );
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = CARD;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.selection.bg_fill = SELECTED;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.window_corner_radius = 12.into();
    style.visuals.window_shadow = dialog_shadow();
    style.visuals.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 1,
        color: Color32::from_black_alpha(145),
    };
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(39, 42, 50);
    style.visuals.widgets.inactive.weak_bg_fill = CARD;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(49, 53, 64);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(45, 49, 59);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(91, 103, 128));
    style.visuals.widgets.active.bg_fill = SELECTED;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for widgets in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widgets.corner_radius = 6.into();
    }
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.interact_size.y = 24.0;
    for text in [egui::TextStyle::Body, egui::TextStyle::Button] {
        style.text_styles.insert(text, FontId::proportional(13.0));
    }
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}
pub fn card(margin: f32) -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(9)
        .inner_margin(margin)
}
pub fn dialog_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 2,
        color: Color32::from_black_alpha(180),
    }
}
pub fn dialog() -> egui::Frame {
    card(18.0)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(91, 98, 115)))
        .shadow(dialog_shadow())
        .corner_radius(12)
}
pub fn logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8, SELECTED);
    for (x, h) in [(-5.0_f32, 4.0_f32), (0.0, 8.0), (5.0, 6.0)] {
        let c = rect.center() + Vec2::new(x, 0.0);
        ui.painter().line_segment(
            [c - Vec2::new(0.0, h), c + Vec2::new(0.0, h)],
            Stroke::new(2.0_f32, ACCENT),
        );
    }
}
pub fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(TEXT).strong())
        .fill(SELECTED)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(78, 97, 132)))
        .corner_radius(7)
}
