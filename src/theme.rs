use egui::{self, Color32, FontId, RichText, Stroke, Vec2};

const BG: Color32 = Color32::from_rgb(23, 24, 28);
const CARD: Color32 = Color32::from_rgb(31, 33, 39);
const BORDER: Color32 = Color32::from_rgb(62, 66, 77);
const ACCENT: Color32 = Color32::from_rgb(151, 175, 220);
const SELECTED: Color32 = Color32::from_rgb(47, 58, 80);
const TEXT: Color32 = Color32::from_rgb(235, 237, 242);
const MUTED: Color32 = Color32::from_rgb(156, 162, 177);
const AMBER: Color32 = Color32::from_rgb(224, 185, 139);

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
    apply_theme(ctx, Theme::Dark);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Theme {
    Light,
    #[default]
    Dark,
}
impl Theme {
    pub const ALL: [Self; 2] = [Self::Light, Self::Dark];
    pub fn label(self) -> &'static str {
        crate::i18n::tr(match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        })
    }
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub selected: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub amber: Color32,
}
thread_local! { static LIGHT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub fn palette() -> Palette {
    if LIGHT.get() {
        Palette {
            bg: Color32::from_rgb(245, 246, 249),
            card: Color32::WHITE,
            border: Color32::from_rgb(202, 207, 217),
            accent: Color32::from_rgb(46, 77, 133),
            selected: Color32::from_rgb(220, 231, 249),
            text: Color32::from_rgb(29, 35, 46),
            muted: Color32::from_rgb(86, 96, 115),
            amber: Color32::from_rgb(133, 80, 18),
        }
    } else {
        Palette {
            bg: BG,
            card: CARD,
            border: BORDER,
            accent: ACCENT,
            selected: SELECTED,
            text: TEXT,
            muted: MUTED,
            amber: AMBER,
        }
    }
}

pub fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let light = theme == Theme::Light;
    LIGHT.set(light);
    let colors = palette();
    let egui_theme = if light {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    };
    ctx.set_theme(egui_theme);
    for viewport in [egui::ViewportId::ROOT, crate::popup::id()] {
        ctx.send_viewport_cmd_to(
            viewport,
            egui::ViewportCommand::SetTheme(if light {
                egui::SystemTheme::Light
            } else {
                egui::SystemTheme::Dark
            }),
        );
    }
    let mut style = (*ctx.style_of(egui_theme)).clone();
    style.visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    style.visuals.panel_fill = colors.bg;
    style.visuals.window_fill = colors.card;
    style.visuals.extreme_bg_color = colors.bg;
    style.visuals.faint_bg_color = colors.card;
    style.visuals.override_text_color = Some(colors.text);
    style.visuals.hyperlink_color = colors.accent;
    style.visuals.selection.bg_fill = colors.selected;
    style.visuals.selection.stroke = Stroke::new(1.0, colors.accent);
    style.visuals.window_stroke = Stroke::new(1.0, colors.border);
    style.visuals.window_corner_radius = 12.into();
    style.visuals.window_shadow = dialog_shadow();
    style.visuals.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 1,
        color: Color32::from_black_alpha(if light { 35 } else { 145 }),
    };
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.border);
    style.visuals.widgets.inactive.bg_fill = if light {
        Color32::from_rgb(234, 237, 243)
    } else {
        Color32::from_rgb(39, 42, 50)
    };
    style.visuals.widgets.inactive.weak_bg_fill = colors.card;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, colors.border);
    style.visuals.widgets.hovered.bg_fill = if light {
        Color32::from_rgb(222, 230, 243)
    } else {
        Color32::from_rgb(49, 53, 64)
    };
    style.visuals.widgets.hovered.weak_bg_fill = style.visuals.widgets.hovered.bg_fill;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, colors.accent);
    style.visuals.widgets.active.bg_fill = colors.selected;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, colors.text);
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
    ctx.set_style_of(egui_theme, style);
}
pub fn card(margin: f32) -> egui::Frame {
    egui::Frame::new()
        .fill(palette().card)
        .stroke(Stroke::new(1.0_f32, palette().border))
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
        .stroke(Stroke::new(1.0_f32, palette().border))
        .shadow(dialog_shadow())
        .corner_radius(12)
}
pub fn logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 8, palette().selected);
    for (x, h) in [(-5.0_f32, 4.0_f32), (0.0, 8.0), (5.0, 6.0)] {
        let c = rect.center() + Vec2::new(x, 0.0);
        ui.painter().line_segment(
            [c - Vec2::new(0.0, h), c + Vec2::new(0.0, h)],
            Stroke::new(2.0_f32, palette().accent),
        );
    }
}
pub fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(palette().text).strong())
        .fill(palette().selected)
        .stroke(Stroke::new(1.0_f32, palette().accent))
        .corner_radius(7)
}
