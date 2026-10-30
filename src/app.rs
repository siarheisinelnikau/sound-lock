use crate::{
    audio::{self, Devices, Engine, Shared},
    config::{Config, Mode, Source},
    driver, routing,
    system::Guard,
    tray::{Action, Tray},
    update,
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SetupPage {
    #[default]
    Audio,
    Driver,
    Updates,
}

pub struct App {
    config: Config,
    devices: Devices,
    shared: Arc<Shared>,
    routing: routing::Service,
    route_status: String,
    active: Option<Active>,
    tray: Option<Tray>,
    error: Option<String>,
    setup: bool,
    setup_page: SetupPage,
    driver: driver::Setup,
    updater: update::Updater,
    startup: update::Startup,
    dirty: bool,
    last_edit: Instant,
    quitting: bool,
    input_level: f32,
    reduction: f32,
    hidden: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: update::Startup) -> Self {
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
            routing: routing::Service::new(),
            route_status: String::new(),
            active: None,
            tray,
            error,
            setup: false,
            setup_page: SetupPage::Audio,
            driver: driver::Setup::default(),
            updater: update::Updater::default(),
            startup,
            dirty: false,
            last_edit: Instant::now(),
            quitting: false,
            input_level: 0.0,
            reduction: 0.0,
            hidden: false,
        };
        app.refresh_devices();
        app.sync();
        if let Some(error) = app.startup.failure.take() {
            app.updater.error = Some(error);
            app.updater.message =
                "The previous version was restored. Review the error or retry.".into();
            app.setup_page = SetupPage::Updates;
            app.setup = true;
        } else if app.config.check_updates_on_start {
            app.updater.check(ctx);
        }
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
        if let Err(error) = self.stop_audio() {
            self.error = Some(format!("{error:#}"));
        }
    }
    fn stop_audio(&mut self) -> anyhow::Result<()> {
        // Restore the original destination before tearing down the PCM bridge.
        let result = self.routing.stop();
        self.route_status.clear();
        self.active = None;
        self.input_level = 0.0;
        self.reduction = 0.0;
        result
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
                if self.config.source == Source::Bridge
                    && self.config.auto_route_browser
                    && let Err(error) = self.routing.start(
                        self.config.browser,
                        self.config.input_id.as_deref().unwrap_or_default(),
                        output,
                    )
                {
                    self.error = Some(format!("{error:#}"));
                    return; // Engine drops only after routing rollback.
                }
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
                let available = self.updater.release.is_some();
                if ui
                    .button(
                        RichText::new(if available {
                            "Setup · Update"
                        } else {
                            "Setup"
                        })
                        .color(if available { MINT } else { TEXT }),
                    )
                    .clicked()
                {
                    if available {
                        self.setup_page = SetupPage::Updates;
                        self.setup = true;
                    } else {
                        self.setup = !self.setup;
                    }
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
                    if system {
                        if ui.small_button(RichText::new("DRIVER-FREE").size(10.0).color(MINT))
                            .on_hover_text("This mode needs no driver. Click for optional VB-CABLE setup and full 400% processing.").clicked() {
                            self.setup_page = SetupPage::Driver;
                            self.setup = true;
                        }
                    } else {
                        ui.label(RichText::new("UP TO 4×").size(10.0).color(MINT));
                    }
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
    fn audio_setup_ui(&mut self, ui: &mut egui::Ui) {
        let running = self.active.is_some();
        ui.add_enabled_ui(!running, |ui| {
            if self.config.source == Source::Bridge {
                ui.label("Source output · spare device or VB-CABLE");
                let name = device_label(
                    &self.devices,
                    self.config.input_id.as_ref(),
                    "Choose a source output",
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
            if self.config.source == Source::Bridge {
                let mut changed = false;
                ui.horizontal(|ui| {
                    changed |= ui
                        .checkbox(&mut self.config.auto_route_browser, "Auto-route browser")
                        .changed();
                    egui::ComboBox::from_id_salt("browser")
                        .width(100.0)
                        .selected_text(self.config.browser.label())
                        .show_ui(ui, |ui| {
                            for browser in routing::Browser::ALL {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.config.browser,
                                        browser,
                                        browser.label(),
                                    )
                                    .changed();
                            }
                        });
                });
                if changed {
                    self.changed();
                }
            }
        });
        if running {
            ui.label(RichText::new("Stop processing before changing devices.").color(AMBER));
        }
        ui.separator();
        if self.config.source == Source::Windows {
            ui.label("Reduces peaks through Windows volume controls. No extra driver required.");
            ui.label(RichText::new("For full leveling and 400% gain, use Boost + leveling with optional VB-CABLE or a spare output.").color(MUTED).size(12.0));
            ui.label(RichText::new("Stop/Quit restores your starting volume. This mode cannot boost above it or anticipate transients.").color(MUTED).size(12.0));
        } else {
            ui.label(if self.config.auto_route_browser {
                "Start sends browser audio to Source. Stop or Quit restores its previous output. Start playback if waiting for browser audio."
            } else {
                "Route the browser to Source in Windows Volume Mixer. Restore its normal output manually after stopping."
            });
            ui.label(RichText::new("Keep Source unmuted and inaudible. Listening device must be your headphones or speakers.").color(MUTED).size(12.0));
            if ui.button("Open Windows volume mixer").clicked()
                && let Err(e) = open_mixer()
            {
                self.error = Some(e.to_string());
            }
        }
        if let Some(Active::Bridge(engine)) = &self.active {
            ui.add(
                egui::Label::new(RichText::new(&engine.description).size(10.0).color(MUTED))
                    .truncate(),
            )
            .on_hover_text(&engine.description);
        }
        if let Some(error) = &self.error {
            ui.separator();
            ui.add(egui::Label::new(RichText::new(error).color(AMBER)).truncate())
                .on_hover_text(error);
        }
    }
    fn driver_setup_ui(&mut self, ui: &mut egui::Ui) {
        let endpoint = audio::vb_cable_source(&self.devices).cloned();
        let detected = endpoint.is_some();
        ui.horizontal(|ui| {
            ui.label(RichText::new("VB-CABLE").strong());
            ui.label(
                RichText::new(if detected { "Detected" } else { "Not detected" })
                    .color(if detected { MINT } else { MUTED }),
            );
        });
        ui.label("Optional virtual output for full leveling and 400% gain without a spare physical output.");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.driver.busy(),
                    egui::Button::new(if self.driver.downloading() {
                        "Downloading…"
                    } else {
                        "Download driver"
                    }),
                )
                .clicked()
            {
                self.driver.download(ui.ctx());
            }
            if ui
                .add_enabled(
                    self.driver.downloaded()
                        && !self.driver.busy()
                        && !detected
                        && self.active.is_none(),
                    egui::Button::new(if self.driver.installing() {
                        "Installer open…"
                    } else {
                        "Install driver…"
                    }),
                )
                .clicked()
            {
                // Preserve the physical listening device before the vendor's
                // installer/reboot can change Windows defaults.
                match self.config.save() {
                    Ok(()) => self.driver.install(ui.ctx()),
                    Err(error) => {
                        self.driver.error = Some(format!(
                            "Could not save audio settings before setup: {error}"
                        ))
                    }
                }
            }
        });
        if self.driver.busy() {
            ui.spinner();
        }
        ui.label(RichText::new(self.driver.message).size(11.0).color(MUTED));
        if let Some(error) = &self.driver.error {
            ui.add(egui::Label::new(RichText::new(error).size(11.0).color(AMBER)).truncate())
                .on_hover_text(error);
        }
        ui.hyperlink_to("Official website & license", driver::WEBSITE);
        ui.separator();
        ui.label(RichText::new("Installation opens the vendor's setup with Windows UAC. Administrator access and a Windows restart are required.").size(12.0).color(MUTED));
        if self.active.is_some() {
            ui.label(
                RichText::new("Stop processing before installing or changing the route.")
                    .size(11.0)
                    .color(AMBER),
            );
        }
        ui.horizontal(|ui| {
            if ui.button("Refresh devices").clicked() {
                self.refresh_devices();
            }
            if ui
                .add_enabled(
                    detected && self.active.is_none() && !self.driver.busy(),
                    egui::Button::new("Use VB-CABLE"),
                )
                .clicked()
                && let Some(endpoint) = endpoint
            {
                self.config.source = Source::Bridge;
                self.config.input_id = Some(endpoint.id);
                self.changed();
                self.setup_page = SetupPage::Audio;
            }
        });
        ui.label(RichText::new("After restart: Refresh devices, Use VB-CABLE, select your browser in Audio, then Start processing.").size(11.0).color(MUTED));
    }
    fn updates_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Sound Lock").strong());
            ui.label(RichText::new(format!("v{}", update::VERSION)).color(MINT));
        });
        if ui
            .checkbox(
                &mut self.config.check_updates_on_start,
                "Check for updates on launch",
            )
            .changed()
        {
            self.changed();
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.updater.busy() && !self.driver.installing(),
                    egui::Button::new("Check GitHub"),
                )
                .clicked()
            {
                self.updater.check(ui.ctx());
            }
            ui.hyperlink_to("All releases", update::RELEASES);
        });
        ui.separator();
        ui.label(&self.updater.message);
        if self.updater.busy() {
            ui.spinner();
        }
        if let Some(progress) = self.updater.progress() {
            ui.add(egui::ProgressBar::new(progress).show_percentage());
        }
        if let Some(release) = self.updater.release.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Available: v{}", release.version)).strong());
                ui.hyperlink_to("Release notes", release.notes_url());
            });
            let ready = self.updater.ready();
            if ui
                .add_enabled(
                    !self.updater.busy() && !self.driver.busy(),
                    egui::Button::new(if ready {
                        "Restart & update"
                    } else {
                        "Download update"
                    }),
                )
                .clicked()
            {
                if ready {
                    let result = (|| -> anyhow::Result<()> {
                        let prepared = self.updater.prepare_restart()?;
                        self.config.save()?;
                        self.stop_audio().map_err(|e| {
                            anyhow::anyhow!("Could not restore audio before updating: {e:#}")
                        })?;
                        prepared.handoff();
                        self.quitting = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        Ok(())
                    })();
                    if let Err(error) = result {
                        self.updater.error = Some(format!("{error:#}"));
                    }
                } else {
                    self.updater.download(ui.ctx());
                }
            }
        }
        ui.label(RichText::new("Downloads are verified before installation. Restart stops processing and saves your settings; start processing again when ready.").size(12.0).color(MUTED));
        if let Some(error) = &self.updater.error {
            ui.separator();
            ui.add(egui::Label::new(RichText::new(error).color(AMBER)).truncate())
                .on_hover_text(error);
        }
    }
    fn show_setup_modal(&mut self, ctx: &egui::Context) -> Option<egui::Rect> {
        if !self.setup {
            return None;
        }
        let response = egui::Modal::new(egui::Id::new("audio_setup_modal"))
            .backdrop_color(Color32::from_black_alpha(170))
            .frame(card(16.0))
            .show(ctx, |ui| {
                ui.set_width(350.0);
                let mut close = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings").size(18.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.small_button("Close").clicked();
                    });
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.setup_page, SetupPage::Audio, "Audio");
                    ui.selectable_value(
                        &mut self.setup_page,
                        SetupPage::Driver,
                        "Driver (optional)",
                    );
                    ui.selectable_value(&mut self.setup_page, SetupPage::Updates, "Updates");
                });
                ui.separator();
                match self.setup_page {
                    SetupPage::Audio => self.audio_setup_ui(ui),
                    SetupPage::Driver => self.driver_setup_ui(ui),
                    SetupPage::Updates => self.updates_ui(ui),
                }
                close
            });
        // Backdrop clicks are swallowed, not used to dismiss the dialog or
        // activate the controls underneath. Escape first closes any open combo.
        let escape = response.is_top_modal
            && !response.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if response.inner || escape {
            self.setup = false;
        }
        Some(response.response.rect)
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.updater.poll();
        if self.driver.poll() {
            self.refresh_devices();
        }
        for action in self.tray.as_ref().map(|t| t.poll()).unwrap_or_default() {
            self.action(action, ctx);
        }
        for event in self.routing.poll() {
            match event {
                routing::Event::Status(status) => self.route_status = status,
                routing::Event::Error(error) => {
                    self.stop();
                    self.error = Some(error);
                }
            }
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
                } else if !self.route_status.is_empty() {
                    &self.route_status
                } else if self.active.is_some() {
                    "Processing on this device"
                } else if self.config.source == Source::Windows {
                    "Ready · no driver required"
                } else {
                    "Audio bridge · VB-CABLE or spare output"
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
        let _ = self.show_setup_modal(ctx);
        if let Err(error) = self.startup.acknowledge() {
            self.updater.error = Some(format!("Could not confirm update startup: {error:#}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, Rect, Shape};

    fn app() -> App {
        App {
            config: Config::default(),
            devices: Devices::default(),
            shared: Arc::new(Shared::default()),
            routing: routing::Service::disabled(),
            route_status: String::new(),
            active: None,
            tray: None,
            error: None,
            setup: false,
            setup_page: SetupPage::Audio,
            driver: driver::Setup::default(),
            updater: update::Updater::default(),
            startup: update::Startup::default(),
            dirty: false,
            last_edit: Instant::now(),
            quitting: false,
            input_level: 0.0,
            reduction: 0.0,
            hidden: false,
        }
    }
    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<Event>,
        time: &mut f64,
    ) -> (egui::FullOutput, Option<Rect>) {
        *time += 0.1;
        let mut modal = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 446.0))),
                time: Some(*time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.profiles(ui));
                modal = app.show_setup_modal(ctx);
            },
        );
        (output, modal)
    }
    fn text_position(shape: &Shape, value: &str) -> Option<Pos2> {
        match shape {
            Shape::Text(text) if text.galley.text() == value => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, value)),
            _ => None,
        }
    }
    fn click(ctx: &egui::Context, app: &mut App, pos: Pos2, time: &mut f64) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                time,
            );
        }
    }
    #[test]
    fn modal_dims_and_blocks_real_profile_controls_until_closed() {
        let ctx = egui::Context::default();
        let mut app = app();
        let mut time = 0.0;
        let (output, _) = frame(&ctx, &mut app, vec![], &mut time);
        let speakers = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Speakers"))
            .unwrap();
        app.setup = true;
        frame(&ctx, &mut app, vec![], &mut time);
        let (output, _) = frame(&ctx, &mut app, vec![], &mut time);
        assert!(output.shapes.iter().any(
            |s| matches!(&s.shape, Shape::Rect(rect) if rect.fill == Color32::from_black_alpha(170))
        ));
        click(&ctx, &mut app, speakers, &mut time);
        assert_eq!(
            app.config.mode,
            Mode::Night,
            "Background profile must not receive the click"
        );
        assert!(app.setup, "A backdrop click must not dismiss the modal");
        frame(
            &ctx,
            &mut app,
            vec![Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut time,
        );
        assert!(!app.setup);
        for _ in 0..2 {
            frame(&ctx, &mut app, vec![], &mut time);
        }
        click(&ctx, &mut app, speakers, &mut time);
        assert_eq!(
            app.config.mode,
            Mode::Speakers,
            "Controls must work after the modal closes"
        );
    }
    #[test]
    fn both_setup_pages_fit_the_fixed_viewport_without_scrolling() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 446.0));
        for (page, source) in [
            (SetupPage::Audio, Source::Windows),
            (SetupPage::Audio, Source::Bridge),
            (SetupPage::Driver, Source::Windows),
            (SetupPage::Updates, Source::Windows),
        ] {
            let ctx = egui::Context::default();
            let mut app = app();
            app.setup = true;
            app.setup_page = page;
            app.config.source = source;
            let mut time = 0.0;
            frame(&ctx, &mut app, vec![], &mut time);
            let (_, modal) = frame(&ctx, &mut app, vec![], &mut time);
            assert!(
                screen.contains_rect(modal.unwrap()),
                "Dialog must fit: {:?}",
                modal
            );
        }
        for (ready, downloading) in [(false, false), (true, false), (false, true)] {
            let ctx = egui::Context::default();
            let mut app = app();
            app.setup = true;
            app.setup_page = SetupPage::Updates;
            app.updater = update::fixture_updater(ready, downloading);
            let mut time = 0.0;
            frame(&ctx, &mut app, vec![], &mut time);
            let (_, modal) = frame(&ctx, &mut app, vec![], &mut time);
            assert!(
                screen.contains_rect(modal.unwrap()),
                "Update states must fit: {modal:?}"
            );
        }
    }
}
