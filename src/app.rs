use crate::{
    audio::{self, Devices, Engine, Shared},
    config::{Config, Mode, Source},
    system::Guard,
    tray::{Action, Tray},
};
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use raw_window_handle::HasWindowHandle;
use soundlock_dsp::gain_to_db;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(15, 20, 28);
const CARD: Color32 = Color32::from_rgb(23, 31, 41);
const MINT: Color32 = Color32::from_rgb(119, 234, 197);
const TEXT: Color32 = Color32::from_rgb(234, 240, 246);
const MUTED: Color32 = Color32::from_rgb(145, 161, 180);
const AMBER: Color32 = Color32::from_rgb(245, 190, 114);

enum Active {
    System { _guard: Guard },
    Bridge(Engine),
}

pub struct App {
    config: Config,
    devices: Devices,
    shared: Arc<Shared>,
    active: Option<Active>,
    tray: Option<Tray>,
    error: Option<String>,
    setup: bool,
    dirty: bool,
    last_edit: Instant,
    quitting: bool,
    input_level: f32,
    reduction: f32,
    hidden: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*ctx.style()).clone();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = CARD;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = MINT.gamma_multiply(0.35);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, MINT);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(41, 79, 71);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(39, 52, 65);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(31, 41, 54);
        style.spacing.item_spacing = Vec2::new(8.0, 7.0);
        style.spacing.button_padding = Vec2::new(10.0, 6.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.0));
        ctx.set_style(style);
        let handle = cc.window_handle().ok().map_or(0, |h| match h.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(h) => h.hwnd.get(),
            _ => 0,
        });
        let (config, mut error) = Config::load();
        let tray = match Tray::new(ctx, handle) {
            Ok(tray) => Some(tray),
            Err(e) => {
                error = Some(format!("Tray unavailable: {e}"));
                None
            }
        };
        let mut app = Self {
            config,
            devices: Devices::default(),
            shared: Arc::new(Shared::default()),
            active: None,
            tray,
            error,
            setup: false,
            dirty: false,
            last_edit: Instant::now(),
            quitting: false,
            input_level: 0.0,
            reduction: 0.0,
            hidden: false,
        };
        app.refresh_devices();
        app.sync();
        app
    }
    fn sync(&self) {
        self.shared.update(self.config.settings());
        self.shared.peak_limit.store(
            (self.config.peak_limit / 100.0).to_bits(),
            Ordering::Relaxed,
        );
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.last_edit = Instant::now();
        self.sync();
    }
    fn refresh_devices(&mut self) {
        match audio::devices() {
            Ok(devices) => {
                if self.config.output_id.is_none() {
                    self.config.output_id = devices
                        .default_output
                        .as_ref()
                        .filter(|id| devices.outputs.iter().any(|d| &d.id == *id && !d.cable))
                        .cloned()
                        .or_else(|| {
                            devices
                                .outputs
                                .iter()
                                .find(|d| !d.cable)
                                .map(|d| d.id.clone())
                        });
                }
                self.devices = devices;
            }
            Err(e) => self.error = Some(format!("Could not enumerate devices: {e}")),
        }
    }
    fn ready(&self) -> bool {
        let output = self
            .devices
            .outputs
            .iter()
            .any(|d| !d.cable && Some(&d.id) == self.config.output_id.as_ref());
        output
            && (self.config.source == Source::Windows
                || (self.config.input_id != self.config.output_id
                    && self
                        .devices
                        .outputs
                        .iter()
                        .any(|d| Some(&d.id) == self.config.input_id.as_ref())))
    }
    fn stop(&mut self) {
        self.active = None;
        self.input_level = 0.0;
        self.reduction = 0.0;
    }
    fn toggle(&mut self) {
        if self.active.is_some() {
            self.stop();
            return;
        }
        if !self.ready() {
            self.setup = true;
            return;
        }
        self.sync();
        let output = self.config.output_id.as_deref().unwrap_or_default();
        let result = match self.config.source {
            Source::Windows => Guard::start(output, self.shared.clone())
                .map(|guard| Active::System { _guard: guard }),
            Source::Bridge => Engine::start(
                self.config.input_id.as_deref().unwrap_or_default(),
                output,
                self.shared.clone(),
            )
            .map(Active::Bridge),
        };
        match result {
            Ok(active) => {
                self.active = Some(active);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }
    fn action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Show => {
                self.hidden = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Action::ToggleAudio => self.toggle(),
            Action::ToggleProcessing => {
                self.config.enabled = !self.config.enabled;
                self.changed();
            }
            Action::Mode(mode) => {
                self.config.mode = mode;
                self.changed();
            }
            Action::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 15.0, MINT);
            for (x, h) in [(-6.0, 5.0), (0.0, 9.0), (6.0, 7.0)] {
                let c = rect.center() + Vec2::new(x, 0.0);
                ui.painter().line_segment(
                    [c - Vec2::new(0.0, h), c + Vec2::new(0.0, h)],
                    Stroke::new(2.5_f32, BG),
                );
            }
            ui.label(RichText::new("sound lock").size(22.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Setup").clicked() {
                    self.setup = !self.setup;
                }
            });
        });
    }
    fn source_tabs(&mut self, ui: &mut egui::Ui) {
        let width = (ui.available_width() - 8.0) / 2.0;
        ui.horizontal(|ui| {
            for (source, label) in [
                (Source::Windows, "System limiter"),
                (Source::Bridge, "Boost + leveling"),
            ] {
                if ui
                    .add_sized(
                        [width, 29.0],
                        egui::Button::new(label).selected(self.config.source == source),
                    )
                    .clicked()
                {
                    self.stop();
                    self.config.source = source;
                    self.changed();
                }
            }
        });
    }
    fn volume(&mut self, ui: &mut egui::Ui) {
        let system = self.config.source == Source::Windows;
        card(12.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if system {
                        "PEAK TARGET"
                    } else {
                        "VOLUME BOOST"
                    })
                    .size(10.0)
                    .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(if system { "NO DRIVER" } else { "UP TO 4×" })
                            .size(10.0)
                            .color(MINT),
                    );
                });
            });
            let value = if system {
                self.config.peak_limit
            } else {
                self.config.volume_percent
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{value:.0}")).size(46.0).strong());
                ui.label(RichText::new("%").size(22.0).color(MUTED));
                ui.label(
                    RichText::new(if system {
                        "lower = softer peaks".into()
                    } else {
                        format!("+{:.1} dB", gain_to_db(value / 100.0))
                    })
                    .size(11.0)
                    .color(MINT),
                );
            });
            ui.spacing_mut().slider_width = (ui.available_width() - 76.0).max(180.0);
            let (value, range, label) = if system {
                (&mut self.config.peak_limit, 10.0..=100.0, "Peak limit")
            } else {
                (&mut self.config.volume_percent, 100.0..=400.0, "Volume")
            };
            let response = ui.add(
                egui::Slider::new(value, range)
                    .show_value(false)
                    .text(label)
                    .step_by(1.0)
                    .trailing_fill(true)
                    .handle_shape(egui::style::HandleShape::Circle),
            );
            if response.changed() {
                self.changed();
            }
        });
    }
    fn profiles(&mut self, ui: &mut egui::Ui) {
        let width = (ui.available_width() - 16.0) / 3.0;
        ui.horizontal(|ui| {
            for mode in Mode::ALL {
                let selected = self.config.mode == mode;
                let button = egui::Button::new(RichText::new(mode.label()).color(if selected {
                    BG
                } else {
                    TEXT
                }))
                .fill(if selected { MINT } else { CARD })
                .corner_radius(8.0);
                if ui.add_sized([width, 29.0], button).clicked() {
                    self.config.mode = mode;
                    self.changed();
                }
            }
        });
        ui.label(
            RichText::new(self.config.mode.description())
                .size(11.0)
                .color(MUTED),
        );
    }
    fn meters(&mut self, ui: &mut egui::Ui) {
        card(10.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Input").size(11.0).color(MUTED));
                let level = ((gain_to_db(self.input_level) + 60.0) / 60.0).clamp(0.0, 1.0);
                ui.add(
                    egui::ProgressBar::new(level)
                        .desired_width(ui.available_width() - 55.0)
                        .desired_height(5.0)
                        .fill(MINT),
                );
                ui.label(
                    RichText::new(if self.input_level < 1e-5 {
                        "−∞ dB".into()
                    } else {
                        format!("{:.0} dB", gain_to_db(self.input_level))
                    })
                    .size(11.0)
                    .color(MUTED),
                );
            });
            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.config.enabled,
                        if self.config.source == Source::Windows {
                            "Peak reduction"
                        } else {
                            "Leveling enabled"
                        },
                    )
                    .changed()
                {
                    self.changed();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("−{:.1} dB", self.reduction))
                            .size(12.0)
                            .color(MINT),
                    )
                    .on_hover_text("Current gain reduction");
                });
            });
        });
    }
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        let running = self.active.is_some();
        ui.add_enabled_ui(!running, |ui| {
            if self.config.source == Source::Bridge {
                ui.label("Source output (must be inaudible)");
                let name = device_label(
                    &self.devices,
                    self.config.input_id.as_ref(),
                    "Choose a spare output",
                );
                let mut changed = false;
                egui::ComboBox::from_id_salt("source")
                    .width(ui.available_width() - 10.0)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for d in &self.devices.outputs {
                            changed |= ui
                                .selectable_value(
                                    &mut self.config.input_id,
                                    Some(d.id.clone()),
                                    &d.name,
                                )
                                .changed();
                        }
                    });
                if changed {
                    self.changed();
                }
            }
            ui.label("Listening device");
            let name = device_label(
                &self.devices,
                self.config.output_id.as_ref(),
                "Choose headphones or speakers",
            );
            let mut changed = false;
            egui::ComboBox::from_id_salt("output")
                .width(ui.available_width() - 10.0)
                .selected_text(name)
                .show_ui(ui, |ui| {
                    for d in self.devices.outputs.iter().filter(|d| !d.cable) {
                        changed |= ui
                            .selectable_value(
                                &mut self.config.output_id,
                                Some(d.id.clone()),
                                &d.name,
                            )
                            .changed();
                    }
                });
            if changed {
                self.changed();
            }
            if ui.button("Refresh devices").clicked() {
                self.refresh_devices();
                self.changed();
            }
        });
        if running {
            ui.label(RichText::new("Stop processing before changing devices.").color(AMBER));
        }
        ui.separator();
        if self.config.source == Source::Windows {
            ui.label("Works directly with Windows volume controls. No routing, extension or driver installation.");
            ui.label(RichText::new("Reactive peak reduction only: it cannot boost beyond the device's maximum or catch the first transient. Your starting volume is the upper bound and is restored on Stop/Quit.").color(MUTED).size(12.0));
        } else {
            ui.label("Route your browser/player to an unused, inaudible output, such as HDMI without speakers. Select that device as Source and your headphones as Listening device.");
            ui.label(RichText::new("Requires two different outputs. Leave the source unmuted; its original audio must be inaudible. No automatic suppression or routing. With only one output, use System limiter.").color(AMBER).size(12.0));
            if ui.button("Open Windows volume mixer").clicked()
                && let Err(e) = open_mixer()
            {
                self.error = Some(e.to_string());
            }
        }
        if let Some(Active::Bridge(engine)) = &self.active {
            ui.label(RichText::new(&engine.description).size(10.0).color(MUTED));
        }
        if let Some(error) = &self.error {
            ui.separator();
            ui.colored_label(AMBER, error);
        }
        ui.label(RichText::new("Close the main window to keep processing in the tray. Quit restores system volume. Force-killing the app cannot run restoration.").size(11.0).color(MUTED));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        for action in self.tray.as_ref().map(|t| t.poll()).unwrap_or_default() {
            self.action(action, ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hidden = true;
        }
        if self.active.is_some() && self.shared.failed.load(Ordering::Relaxed) {
            self.stop();
            self.error = Some(
                "Audio device unavailable. Reconnect it, open Setup and refresh devices.".into(),
            );
        }
        if self.active.is_some() {
            self.input_level = Shared::meter(&self.shared.input_peak).max(self.input_level * 0.85);
            self.reduction = Shared::meter(&self.shared.reduction).max(self.reduction * 0.9);
        }
        if self.dirty && self.last_edit.elapsed() > Duration::from_millis(500) {
            if let Err(e) = self.config.save() {
                self.error = Some(format!("Could not save settings: {e}"));
            }
            self.dirty = false;
        }
        if let Some(tray) = &self.tray {
            tray.sync(self.active.is_some(), self.config.enabled, self.config.mode);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(16.0))
            .show(ctx, |ui| {
                self.header(ui);
                self.source_tabs(ui);
                self.volume(ui);
                self.profiles(ui);
                self.meters(ui);
                let status = if self.error.is_some() {
                    "Audio needs attention · open Setup"
                } else if self.active.is_some() {
                    "Processing on this device"
                } else if self.config.source == Source::Windows {
                    "Ready · no driver required"
                } else {
                    "Audio bridge · two outputs required"
                };
                ui.label(
                    RichText::new(status)
                        .size(11.0)
                        .color(if self.error.is_some() { AMBER } else { MUTED }),
                )
                .on_hover_text(self.error.as_deref().unwrap_or(""));
                let label = if self.active.is_some() {
                    "Stop processing"
                } else if self.ready() {
                    "Start processing"
                } else {
                    "Set up audio"
                };
                if ui
                    .add_sized(
                        [ui.available_width(), 35.0],
                        egui::Button::new(RichText::new(label).strong().color(BG))
                            .fill(MINT)
                            .corner_radius(8.0),
                    )
                    .clicked()
                {
                    self.toggle();
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(device_label(
                                &self.devices,
                                self.config.output_id.as_ref(),
                                "No output",
                            ))
                            .size(10.0)
                            .color(MUTED),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Quit").clicked() {
                            self.action(Action::Quit, ctx);
                        }
                        if self.tray.is_some() && ui.small_button("Hide").clicked() {
                            self.hidden = true;
                        }
                    });
                });
            });
        if self.setup {
            let mut open = true;
            egui::Window::new("Audio setup")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .default_width(350.0)
                .show(ctx, |ui| self.setup_ui(ui));
            self.setup = open;
        }
        if self.hidden && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.stop();
        if let Err(e) = self.config.save() {
            eprintln!("Cannot save settings: {e}");
        }
    }
}

fn card(margin: f32) -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(11.0)
        .inner_margin(margin)
}
fn device_label(devices: &Devices, id: Option<&String>, fallback: &str) -> String {
    devices
        .outputs
        .iter()
        .find(|d| Some(&d.id) == id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| fallback.into())
}
fn open_mixer() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .arg("ms-settings:apps-volume")
            .creation_flags(0x08000000)
            .spawn()?;
    }
    Ok(())
}
