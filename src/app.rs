use crate::theme::{ACCENT, AMBER, BG, BORDER, CARD, MUTED, SELECTED, TEXT, card};
use crate::{
    audio::{self, Devices, Engine, Shared},
    config::{Config, Mode, Source},
    driver, popup, routing,
    system::Guard,
    theme,
    tray::{Action, Tray},
    update,
};
use egui::{self, Color32, RichText, Stroke, Vec2};
use soundlock_dsp::gain_to_db;
#[cfg(feature = "tray-smoke-test")]
#[path = "tray_smoke.rs"]
mod tray_smoke;

use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

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
    popup_open: bool,
    popup_had_focus: bool,
    popup_focus_pending: bool,
    popup_anchor: Option<popup::Anchor>,
    notice: Option<String>,
    preview: bool,
    preview_running: bool,
    install_requested: bool,
    #[cfg(feature = "tray-smoke-test")]
    smoke: Option<tray_smoke::SmokeTest>,
}

impl App {
    pub fn new(cc: &crate::desktop::CreationContext<'_>, startup: update::Startup) -> Self {
        let ctx = &cc.egui_ctx;
        theme::apply(ctx);
        let preview = std::env::args().any(|a| a == "--preview")
            || (cfg!(any(feature = "ui-capture", feature = "tray-smoke-test"))
                && std::env::var("SOUND_LOCK_PREVIEW").as_deref() == Ok("1"));
        let quiet = std::env::args().any(|a| a == "--startup");
        let preferences = std::env::args().any(|a| a == "--settings");
        let (config, mut error) = if preview {
            (
                Config {
                    source: Source::Bridge,
                    volume_percent: 200.0,
                    input_id: Some("preview:cable".into()),
                    output_id: Some("preview:headphones".into()),
                    ..Config::default()
                },
                None,
            )
        } else {
            Config::load()
        };
        let tray = match Tray::new(ctx) {
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
            routing: if preview {
                routing::Service::disabled()
            } else {
                let wake = ctx.clone();
                routing::Service::new(move || wake.request_repaint_of(egui::ViewportId::ROOT))
            },
            route_status: String::new(),
            active: None,
            setup: preferences || tray.is_none(),
            tray,
            error,
            setup_page: SetupPage::Audio,
            driver: driver::Setup::default(),
            updater: update::Updater::default(),
            startup,
            dirty: false,
            last_edit: Instant::now(),
            quitting: false,
            input_level: 0.0,
            reduction: 0.0,
            popup_open: !quiet && !preferences,
            popup_had_focus: false,
            popup_focus_pending: true,
            popup_anchor: None,
            notice: None,
            preview,
            preview_running: false,
            install_requested: false,
            #[cfg(feature = "tray-smoke-test")]
            smoke: tray_smoke::SmokeTest::new(cc, preview),
        };
        if preview {
            app.devices = Devices {
                outputs: vec![
                    audio::Endpoint {
                        id: "preview:cable".into(),
                        name: "CABLE Input (VB-Audio Virtual Cable)".into(),
                        cable: true,
                    },
                    audio::Endpoint {
                        id: "preview:headphones".into(),
                        name: "Headphones · Wireless audio".into(),
                        cable: false,
                    },
                ],
                default_output: Some("preview:headphones".into()),
            };
        } else {
            app.refresh_devices();
        }
        app.sync();
        if let Some(error) = app.startup.failure.take() {
            app.updater.error = Some(error);
            app.updater.message =
                "The previous version was restored. Review the error or retry.".into();
            app.open_settings(SetupPage::Updates, ctx);
        } else {
            app.check_updates_on_start(ctx);
        }
        #[cfg(feature = "ui-capture")]
        if preview {
            if let Ok(scale) = std::env::var("SOUND_LOCK_UI_SCALE")
                && let Ok(scale) = scale.parse::<f32>()
                && (1.0..=2.0).contains(&scale)
            {
                ctx.set_zoom_factor(scale);
            }
            match std::env::var("SOUND_LOCK_CAPTURE_VIEW").as_deref() {
                Ok("audio") => app.open_settings(SetupPage::Audio, ctx),
                Ok("driver") => app.open_settings(SetupPage::Driver, ctx),
                Ok("updates") => {
                    app.updater = update::fixture_updater_for_preview();
                    app.open_settings(SetupPage::Updates, ctx);
                }
                Ok("dialog") => {
                    app.open_settings(SetupPage::Audio, ctx);
                    app.notice = Some("The selected audio device is unavailable. Reconnect your headphones, then refresh the device list in Audio settings.".into());
                }
                Ok("popup") => {
                    app.preview_running = true;
                    app.input_level = 0.16;
                    app.reduction = 4.2;
                }
                _ => {}
            }
        }
        if app.setup {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        app
    }
    fn check_updates_on_start(&mut self, ctx: &egui::Context) {
        if self.config.check_updates_on_start && !self.preview {
            self.updater.check(ctx);
        }
    }
    fn is_running(&self) -> bool {
        self.active.is_some() || self.preview_running
    }
    fn show_popup(&mut self, anchor: Option<popup::Anchor>, ctx: &egui::Context) {
        self.popup_anchor = anchor.or_else(|| self.tray.as_ref().and_then(Tray::anchor));
        self.popup_open = true;
        self.popup_had_focus = false;
        self.popup_focus_pending = true;
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::RequestPaintWhileHidden,
        );
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    fn hide_popup(&mut self, ctx: &egui::Context) {
        self.popup_open = false;
        ctx.send_viewport_cmd_to(popup::id(), egui::ViewportCommand::Visible(false));
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::RequestPaintWhileHidden,
        );
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    fn open_settings(&mut self, page: SetupPage, ctx: &egui::Context) {
        self.hide_popup(ctx);
        self.setup = true;
        self.setup_page = page;
        for command in [
            egui::ViewportCommand::Visible(true),
            egui::ViewportCommand::Minimized(false),
            egui::ViewportCommand::Focus,
            egui::ViewportCommand::RequestPaintWhileHidden,
        ] {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, command);
        }
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    fn close_settings(&mut self, ctx: &egui::Context) {
        self.setup = false;
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::Visible(false),
        );
    }
    fn sync(&self) {
        self.shared.update(self.config.settings());
        self.shared.peak_limit.store(
            (self.config.peak_limit / 100.0).to_bits(),
            Ordering::Relaxed,
        );
    }
    fn changed(&mut self, ctx: &egui::Context) {
        self.dirty = true;
        self.last_edit = Instant::now();
        self.sync();
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        if self.popup_open {
            ctx.request_repaint_of(popup::id());
        }
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
        self.preview_running = false;
        // Restore the original destination before tearing down the PCM bridge.
        let result = self.routing.stop();
        self.route_status.clear();
        self.active = None;
        self.input_level = 0.0;
        self.reduction = 0.0;
        result
    }
    fn toggle(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        if self.popup_open {
            ctx.request_repaint_of(popup::id());
        }
        if self.preview {
            self.preview_running = !self.preview_running;
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            return;
        }
        if self.active.is_some() {
            self.stop();
            return;
        }
        if !self.ready() {
            self.open_settings(SetupPage::Audio, ctx);
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
            Action::Show => self.show_popup(None, ctx),
            Action::TogglePopup(anchor) => {
                if self.popup_open {
                    self.hide_popup(ctx);
                } else {
                    self.show_popup(Some(anchor), ctx);
                }
            }
            Action::Settings => self.open_settings(SetupPage::Audio, ctx),
            Action::CheckUpdates => {
                self.open_settings(SetupPage::Updates, ctx);
                if !self.preview {
                    self.updater.check(ctx);
                }
            }
            Action::ToggleAudio => self.toggle(ctx),
            Action::ToggleProcessing => {
                self.config.enabled = !self.config.enabled;
                self.changed(ctx);
            }
            Action::Mode(mode) => {
                self.config.mode = mode;
                self.changed(ctx);
            }
            Action::Quit => {
                self.quitting = true;
                self.install_requested = false;
                self.popup_open = false;
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::RequestPaintWhileHidden,
                );
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::logo(ui, 28.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("Sound Lock").size(17.0).strong());
                ui.add(
                    egui::Label::new(
                        RichText::new(device_label(
                            &self.devices,
                            self.config.output_id.as_ref(),
                            "Choose an output in Settings",
                        ))
                        .size(10.0)
                        .color(MUTED),
                    )
                    .truncate(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Dismiss · Esc")
                    .clicked()
                {
                    self.hide_popup(ui.ctx());
                }
                let available = self.updater.release.is_some();
                if ui
                    .small_button(
                        RichText::new(if available { "Update" } else { "Settings" })
                            .color(if available { ACCENT } else { TEXT }),
                    )
                    .clicked()
                {
                    self.open_settings(
                        if available {
                            SetupPage::Updates
                        } else {
                            SetupPage::Audio
                        },
                        ui.ctx(),
                    );
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
                    self.changed(ui.ctx());
                }
            }
        });
    }
    fn volume(&mut self, ui: &mut egui::Ui) {
        let system = self.config.source == Source::Windows;
        card(10.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        RichText::new(if system { "Peak limit" } else { "Volume boost" })
                            .size(13.0)
                            .strong(),
                    );
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if system {
                                    "System limiter".into()
                                } else {
                                    format!(
                                        "Boost + leveling · +{:.1} dB",
                                        gain_to_db(self.config.volume_percent / 100.0)
                                    )
                                })
                                .size(10.0)
                                .color(MUTED),
                            )
                            .frame(false)
                            .min_size(Vec2::ZERO),
                        )
                        .clicked()
                    {
                        self.open_settings(SetupPage::Audio, ui.ctx());
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let value = if system {
                        self.config.peak_limit
                    } else {
                        self.config.volume_percent
                    };
                    ui.label(RichText::new(format!("{value:.0}%")).size(32.0).color(TEXT));
                });
            });
            ui.spacing_mut().slider_width = ui.available_width();
            let (value, range) = if system {
                (&mut self.config.peak_limit, 10.0..=100.0)
            } else {
                (&mut self.config.volume_percent, 100.0..=400.0)
            };
            if ui
                .add(
                    egui::Slider::new(value, range)
                        .show_value(false)
                        .step_by(1.0)
                        .trailing_fill(true),
                )
                .changed()
            {
                self.changed(ui.ctx());
            }
        });
    }
    fn profiles(&mut self, ui: &mut egui::Ui) {
        let width = (ui.available_width() - 16.0) / 3.0;
        ui.horizontal(|ui| {
            for mode in Mode::ALL {
                let selected = self.config.mode == mode;
                let button =
                    egui::Button::new(RichText::new(mode.label()).size(12.0).color(if selected {
                        TEXT
                    } else {
                        MUTED
                    }))
                    .fill(if selected { SELECTED } else { CARD })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if selected {
                            ACCENT.gamma_multiply(0.6)
                        } else {
                            BORDER
                        },
                    ))
                    .corner_radius(6);
                if ui
                    .add_sized([width, 28.0], button)
                    .on_hover_text(mode.description())
                    .clicked()
                {
                    self.config.mode = mode;
                    self.changed(ui.ctx());
                }
            }
        });
    }
    fn meters(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .checkbox(
                    &mut self.config.enabled,
                    if self.config.source == Source::Windows {
                        "Peak reduction"
                    } else {
                        "Leveling"
                    },
                )
                .changed()
            {
                self.changed(ui.ctx());
            }
            let level = ((gain_to_db(self.input_level) + 60.0) / 60.0).clamp(0.0, 1.0);
            ui.add(
                egui::ProgressBar::new(level)
                    .desired_width((ui.available_width() - 66.0).max(30.0))
                    .desired_height(4.0)
                    .fill(ACCENT.gamma_multiply(0.8)),
            )
            .on_hover_text(format!("Input: {:.0} dBFS", gain_to_db(self.input_level)));
            ui.label(
                RichText::new(format!("−{:.1} dB", self.reduction))
                    .size(11.0)
                    .color(MUTED),
            )
            .on_hover_text("Current gain reduction");
        });
    }
    fn audio_setup_ui(&mut self, ui: &mut egui::Ui) {
        let running = self.is_running();
        let bridge = self.config.source == Source::Bridge;
        ui.add_enabled_ui(!running,|ui| {
            let mut changed=false;
            if bridge { ui.horizontal(|ui| {
                ui.add_sized([66.0,24.0],egui::Label::new("Source"));
                egui::ComboBox::from_id_salt("source").width(ui.available_width()-10.0)
                    .selected_text(device_label(&self.devices,self.config.input_id.as_ref(),"Choose a source output"))
                    .show_ui(ui,|ui| { for device in &self.devices.outputs { changed|=ui.selectable_value(&mut self.config.input_id,Some(device.id.clone()),&device.name).changed(); } });
            }); }
            ui.horizontal(|ui| {
                ui.add_sized([66.0,24.0],egui::Label::new("Listening"));
                egui::ComboBox::from_id_salt("output").width(ui.available_width()-10.0)
                    .selected_text(device_label(&self.devices,self.config.output_id.as_ref(),"Choose headphones or speakers"))
                    .show_ui(ui,|ui| { for device in self.devices.outputs.iter().filter(|d|!d.cable) { changed|=ui.selectable_value(&mut self.config.output_id,Some(device.id.clone()),&device.name).changed(); } });
            });
            ui.horizontal(|ui| {
                if ui.small_button("Refresh devices").clicked() { if !self.preview { self.refresh_devices(); } changed=true; }
                if bridge {
                    changed|=ui.checkbox(&mut self.config.auto_route_browser,"Auto-route").on_hover_text("Switch this browser to Source on Start; restore its previous output on Stop/Quit.").changed();
                    egui::ComboBox::from_id_salt("browser").width(90.0).selected_text(self.config.browser.label()).show_ui(ui,|ui| {
                        for browser in routing::Browser::ALL { changed|=ui.selectable_value(&mut self.config.browser,browser,browser.label()).changed(); }
                    });
                }
            });
            if changed { self.changed(ui.ctx()); }
        });
        if running {
            ui.label(
                RichText::new("Stop processing before changing devices.")
                    .size(11.0)
                    .color(AMBER),
            );
        }
        ui.separator();
        if !bridge {
            ui.label(RichText::new("Driver-free peak reduction using Windows volume. Stop restores your starting level. Choose Boost + leveling for full processing and 400% gain.").size(12.0).color(MUTED));
        } else {
            ui.label(RichText::new(if self.config.auto_route_browser {
                "Keep Source unmuted and inaudible. Start routes the browser here; Stop/Quit restores its previous output. Play audio if waiting for the browser."
            } else {
                "Route the browser to Source manually in Windows Volume Mixer. Keep Source unmuted and inaudible, and restore the browser output after stopping."
            }).size(12.0).color(MUTED));
            if ui.small_button("Windows volume mixer").clicked()
                && let Err(error) = open_mixer()
            {
                self.error = Some(error.to_string());
            }
        }
        if let Some(Active::Bridge(engine)) = &self.active {
            ui.add(
                egui::Label::new(RichText::new(&engine.description).size(10.0).color(MUTED))
                    .truncate(),
            )
            .on_hover_text(&engine.description);
        }
        if let Some(error) = self.error.clone() {
            ui.separator();
            if error_line(ui, &error) {
                self.notice = Some(error);
            }
        }
    }
    fn driver_setup_ui(&mut self, ui: &mut egui::Ui) {
        let endpoint = audio::vb_cable_source(&self.devices).cloned();
        let detected = endpoint.is_some();
        ui.horizontal(|ui| {
            ui.label(RichText::new("VB-CABLE").strong());
            ui.label(
                RichText::new(if detected { "Detected" } else { "Not detected" })
                    .color(if detected { ACCENT } else { MUTED }),
            );
        });
        ui.label("Optional virtual output for full leveling and 400% gain without a spare physical output.");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.driver.busy() && !self.preview,
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
                        && !self.preview
                        && !self.driver.busy()
                        && !detected
                        && !self.is_running(),
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
        if let Some(error) = self.driver.error.clone()
            && error_line(ui, &error)
        {
            self.notice = Some(error);
        }
        ui.hyperlink_to("Official website & license", driver::WEBSITE);
        ui.separator();
        ui.label(RichText::new("Installation opens the vendor's setup with Windows UAC. Administrator access and a Windows restart are required.").size(12.0).color(MUTED));
        if self.is_running() {
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
                    detected && !self.is_running() && !self.driver.busy(),
                    egui::Button::new("Use VB-CABLE"),
                )
                .clicked()
                && let Some(endpoint) = endpoint
            {
                self.config.source = Source::Bridge;
                self.config.input_id = Some(endpoint.id);
                self.changed(ui.ctx());
                self.setup_page = SetupPage::Audio;
            }
        });
        ui.label(RichText::new("After restart: Refresh devices, Use VB-CABLE, select your browser in Audio, then Start processing.").size(11.0).color(MUTED));
    }
    fn updates_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Installed: v{}", update::VERSION))
                    .size(12.0)
                    .color(MUTED),
            );
            if ui
                .add_enabled(
                    !self.updater.busy() && !self.driver.installing() && !self.preview,
                    egui::Button::new("Check for updates"),
                )
                .clicked()
            {
                self.updater.check(ui.ctx());
            }
            ui.hyperlink_to("All releases", update::RELEASES);
        });
        if ui
            .checkbox(
                &mut self.config.check_updates_on_start,
                "Check for updates on startup",
            )
            .changed()
        {
            self.changed(ui.ctx());
        }
        ui.separator();
        ui.horizontal(|ui| {
            if self.updater.busy() && self.updater.progress().is_none() {
                ui.spinner();
            }
            ui.add(egui::Label::new(&self.updater.message).truncate())
                .on_hover_text(&self.updater.message);
        });
        if let Some(progress) = self.updater.progress() {
            ui.add(egui::ProgressBar::new(progress).show_percentage());
        }
        if let Some(release) = self.updater.release.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Available: v{}", release.version)).strong());
                ui.hyperlink_to("Release notes", release.notes_url());
            });
            let ready = self.updater.ready();
            let can_defer = self.install_requested && self.updater.busy();
            let label = if can_defer {
                "Install later"
            } else if ready {
                "Restart & install"
            } else {
                "Download and install"
            };
            if ui
                .add_enabled(
                    (can_defer || !self.updater.busy()) && !self.driver.busy() && !self.preview,
                    egui::Button::new(label),
                )
                .clicked()
            {
                if can_defer {
                    self.install_requested = false;
                } else if ready {
                    self.install_update(ui.ctx());
                } else {
                    self.install_requested = true;
                    self.updater.download(ui.ctx());
                }
            }
        }
        ui.label(
            RichText::new(if self.install_requested {
                "After verification: save settings, stop audio and restart."
            } else {
                "Restart saves settings and stops processing."
            })
            .size(12.0)
            .color(MUTED),
        );
        if let Some(error) = self.updater.error.clone() {
            ui.separator();
            if error_line(ui, &error) {
                self.notice = Some(error);
            }
        }
    }
    fn install_update(&mut self, ctx: &egui::Context) {
        self.install_requested = false;
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(!self.preview, "Updates cannot be installed in preview mode");
            let prepared = self.updater.prepare_restart()?;
            self.config.save()?;
            self.stop_audio().map_err(|error| {
                anyhow::anyhow!("Could not restore audio before updating: {error:#}")
            })?;
            prepared.handoff();
            self.action(Action::Quit, ctx);
            Ok(())
        })();
        if let Err(error) = result {
            self.updater.error = Some(format!("{error:#}"));
            self.open_settings(SetupPage::Updates, ctx);
        }
    }
    fn settings_content(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Settings").size(22.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Sound controls").clicked() {
                    self.close_settings(ui.ctx());
                    self.show_popup(None, ui.ctx());
                }
            });
        });
        ui.label(
            RichText::new("Devices, optional driver and application updates.")
                .size(11.0)
                .color(MUTED),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (page, label) in [
                (SetupPage::Audio, "Audio"),
                (SetupPage::Driver, "Driver (optional)"),
                (SetupPage::Updates, "Updates"),
            ] {
                ui.selectable_value(&mut self.setup_page, page, label);
            }
        });
        ui.add_space(6.0);
        card(14.0).show(ui, |ui| match self.setup_page {
            SetupPage::Audio => {
                self.source_tabs(ui);
                ui.add_space(6.0);
                self.audio_setup_ui(ui);
            }
            SetupPage::Driver => self.driver_setup_ui(ui),
            SetupPage::Updates => self.updates_ui(ui),
        });
        ui.add_space(5.0);
        let footer = ui.horizontal(|ui| {
            ui.label(
                RichText::new(if self.preview {
                    "Preview · no devices or settings are changed".into()
                } else {
                    format!("Sound Lock {}", update::VERSION)
                })
                .size(10.0)
                .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Quit").clicked() {
                    self.action(Action::Quit, ui.ctx());
                }
            });
        });
        footer.response.rect
    }
    fn show_notice(&mut self, ctx: &egui::Context) -> Option<egui::Rect> {
        let text = self.notice.clone()?;
        let response = egui::Modal::new(egui::Id::unique("sound-lock-notice"))
            .backdrop_color(Color32::from_black_alpha(170))
            .frame(theme::dialog())
            .show(ctx, |ui| {
                ui.set_width(330.0);
                ui.label(RichText::new("Needs attention").size(17.0).strong());
                ui.add_space(6.0);
                let short: String = text.chars().take(440).collect();
                ui.label(short);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy details").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(theme::primary("Close")).clicked()
                    })
                    .inner
                })
                .inner
            });
        let escape = response.is_top_modal
            && !response.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if response.inner || escape {
            self.notice = None;
        }
        Some(response.response.rect)
    }
    fn popup_content(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        egui::Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(12)
            .shadow(egui::Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 0,
                color: Color32::from_black_alpha(145),
            })
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_width(popup::SIZE.x - 48.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                self.header(ui);
                self.volume(ui);
                self.profiles(ui);
                self.meters(ui);
                ui.horizontal(|ui| {
                    let status = if self.error.is_some() {
                        "Audio needs attention"
                    } else if self.is_running() {
                        "Processing"
                    } else {
                        "Ready"
                    };
                    let color = if self.error.is_some() { AMBER } else { MUTED };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(status).size(11.0).color(color))
                                .frame(false),
                        )
                        .on_hover_text(self.error.as_deref().unwrap_or(&self.route_status))
                        .clicked()
                        && let Some(error) = self.error.clone()
                    {
                        self.notice = Some(error);
                        self.open_settings(SetupPage::Audio, ui.ctx());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.is_running() {
                            "Stop"
                        } else if self.ready() {
                            "Start"
                        } else {
                            "Set up audio"
                        };
                        if ui.add_sized([100.0, 30.0], theme::primary(label)).clicked() {
                            if self.is_running() || self.ready() {
                                self.toggle(ui.ctx());
                            } else {
                                self.open_settings(SetupPage::Audio, ui.ctx());
                            }
                        }
                    });
                });
            })
            .response
            .rect
    }
    fn popup_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.popup_focus_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(popup::position(
                self.popup_anchor,
                popup::SIZE,
                &ctx,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.popup_focus_pending = false;
        }
        let focused = ctx.input(|i| i.viewport().focused);
        if focused == Some(true) {
            self.popup_had_focus = true;
        }
        let benchmark_hold = cfg!(feature = "ui-capture")
            && self.preview
            && std::env::var("SOUND_LOCK_RENDER_ACTIVITY").as_deref() == Ok("1");
        if popup::dismiss(
            self.popup_had_focus,
            if benchmark_hold { Some(true) } else { focused },
            self.popup_anchor.is_some_and(popup::cursor_in),
            ctx.input(|i| i.key_pressed(egui::Key::Escape)),
            ctx.input(|i| i.viewport().close_requested()),
        ) {
            self.hide_popup(&ctx);
            return;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(10))
            .show(ui, |ui| {
                self.popup_content(ui);
            });
        #[cfg(feature = "ui-capture")]
        if self.preview
            && std::env::var("SOUND_LOCK_CAPTURE_VIEW")
                .as_deref()
                .unwrap_or("popup")
                == "popup"
        {
            self.capture(&ctx);
        }
    }
    fn tick(&mut self, ctx: &egui::Context) -> bool {
        let mut ui_changed = self.updater.poll();
        let driver_message = self.driver.message;
        if self.driver.poll() && !self.preview {
            self.refresh_devices();
        }
        ui_changed |= driver_message != self.driver.message;
        for action in self.tray.as_ref().map(|t| t.poll()).unwrap_or_default() {
            ui_changed = true;
            self.action(action, ctx);
        }
        if self.install_requested && !self.updater.busy() && !self.quitting {
            if self.updater.error.is_some() {
                self.install_requested = false;
            } else if self.updater.ready() {
                if self.driver.busy() {
                    self.install_requested = false;
                    self.updater.message =
                        "Update ready. Close driver setup, then restart to install.".into();
                } else {
                    self.install_update(ctx);
                }
            }
        }
        for event in self.routing.poll() {
            ui_changed = true;
            match event {
                routing::Event::Status(status) => self.route_status = status,
                routing::Event::Error(error) => {
                    self.stop();
                    self.error = Some(error);
                }
            }
        }
        if self.active.is_some() && self.shared.failed.load(Ordering::Relaxed) {
            ui_changed = true;
            self.stop();
            self.error = Some(
                "Audio device unavailable. Reconnect it, open Settings and refresh devices.".into(),
            );
        }
        let previous_meters = (self.input_level, self.reduction);
        if self.preview_running {
            self.input_level = 0.16;
            self.reduction = 4.2;
            #[cfg(feature = "ui-capture")]
            if std::env::var("SOUND_LOCK_RENDER_ACTIVITY").as_deref() == Ok("1") {
                let wave = (self.last_edit.elapsed().as_secs_f32() * 2.0).sin() * 0.5 + 0.5;
                self.input_level = 0.02 + wave * 0.25;
                self.reduction = wave * 6.0;
            }
        } else if self.active.is_some() {
            self.input_level = Shared::meter(&self.shared.input_peak).max(self.input_level * 0.85);
            self.reduction = Shared::meter(&self.shared.reduction).max(self.reduction * 0.9);
        } else {
            self.input_level = 0.0;
            self.reduction = 0.0;
        }
        if self.dirty && self.last_edit.elapsed() > Duration::from_millis(500) {
            if !self.preview
                && let Err(error) = self.config.save()
            {
                self.error = Some(format!("Could not save settings: {error}"));
            }
            self.dirty = false;
        }
        if let Some(tray) = &self.tray {
            tray.sync(
                self.is_running(),
                self.config.enabled,
                self.config.mode,
                self.updater.release.as_ref().map(|r| r.version.as_str()),
            );
        }
        #[cfg(feature = "tray-smoke-test")]
        if let Some(mut smoke) = self.smoke.take() {
            smoke.tick(self, ctx);
            self.smoke = Some(smoke);
        }
        if self.popup_open && (ui_changed || previous_meters != (self.input_level, self.reduction))
        {
            ctx.request_repaint_of(popup::id());
        }
        ui_changed
    }
    fn next_poll(&self) -> Option<Duration> {
        let mut delay = None;
        let mut include = |value| delay = Some(delay.map_or(value, |old: Duration| old.min(value)));
        if self.is_running() {
            include(Duration::from_millis(if self.popup_open {
                50
            } else {
                250
            }));
        }
        if self.updater.busy() || self.driver.busy() {
            include(Duration::from_millis(100));
        }
        if self.dirty {
            include(Duration::from_millis(500).saturating_sub(self.last_edit.elapsed()));
        }
        delay
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.setup {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_settings(&ctx);
        }
        if self.setup {
            let mut used_height = popup::SETTINGS_SIZE.y;
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(BG).inner_margin(18))
                .show(ui, |ui| {
                    used_height = self.settings_content(ui).bottom() + 18.0;
                });
            // Fit the current page rather than keeping an empty area beneath it.
            let desired = used_height.ceil().clamp(320.0, popup::SETTINGS_SIZE.y);
            let current = ctx.input(|i| i.viewport().inner_rect.map(|r| r.height()));
            if current.is_some_and(|height| (height - desired).abs() > 2.0) {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                    popup::SETTINGS_SIZE.x,
                    desired,
                )));
            }
            self.show_notice(&ctx);
            if self.notice.is_none()
                && !egui::Popup::is_any_open(&ctx)
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                self.close_settings(&ctx);
            }
            #[cfg(feature = "ui-capture")]
            if self.preview
                && std::env::var("SOUND_LOCK_CAPTURE_VIEW")
                    .as_deref()
                    .unwrap_or("popup")
                    != "popup"
            {
                self.capture(&ctx);
            }
        }
    }
    fn exit(&mut self) {
        self.stop();
        if !self.preview
            && let Err(error) = self.config.save()
        {
            eprintln!("Cannot save settings: {error}");
        }
        #[cfg(feature = "tray-smoke-test")]
        if let Some(smoke) = &self.smoke {
            smoke.finish();
        }
    }
    #[cfg(feature = "ui-capture")]
    fn capture(&mut self, ctx: &egui::Context) {
        match crate::capture::update(ctx) {
            Ok(true) => self.action(Action::Quit, ctx),
            Ok(false) => {}
            Err(error) => {
                eprintln!("Capture failed: {error}");
                std::process::exit(1);
            }
        }
    }
}

pub struct Application(std::rc::Rc<std::cell::RefCell<App>>);
thread_local! { static VIEWPORT_STATE: std::cell::RefCell<std::rc::Weak<std::cell::RefCell<App>>> = const { std::cell::RefCell::new(std::rc::Weak::new()) }; }
impl Application {
    pub fn new(app: App) -> Self {
        let state = std::rc::Rc::new(std::cell::RefCell::new(app));
        VIEWPORT_STATE.with(|slot| *slot.borrow_mut() = std::rc::Rc::downgrade(&state));
        Self(state)
    }
}
impl Application {
    pub fn logic(&mut self, ctx: &egui::Context) -> bool {
        self.0.borrow_mut().tick(ctx)
    }
    pub fn next_poll(&self) -> Option<Duration> {
        self.0.borrow().next_poll()
    }
    pub fn progress_visible(&self) -> bool {
        let app = self.0.borrow();
        app.setup && (app.updater.busy() || app.driver.busy())
    }
    pub fn is_quitting(&self) -> bool {
        self.0.borrow().quitting
    }
    pub fn did_present(&mut self) {
        let mut app = self.0.borrow_mut();
        if let Err(error) = app.startup.acknowledge() {
            app.updater.error = Some(format!("Could not confirm update startup: {error:#}"));
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let builder = {
            let mut app = self.0.borrow_mut();
            app.settings_ui(ui);
            (app.popup_open && !app.quitting).then(|| {
                egui::ViewportBuilder::default()
                    .with_title(popup::TITLE)
                    .with_decorations(false)
                    .with_resizable(false)
                    .with_taskbar(false)
                    .with_transparent(true)
                    .with_visible(true)
                    .with_active(true)
                    .with_window_level(egui::WindowLevel::AlwaysOnTop)
                    .with_inner_size(popup::SIZE)
                    .with_position(popup::position(app.popup_anchor, popup::SIZE, &ctx))
            })
        };
        if let Some(builder) = builder {
            ctx.show_viewport_deferred(popup::id(), builder, |ui, _| {
                if let Some(state) = VIEWPORT_STATE.with(|slot| slot.borrow().upgrade()) {
                    state.borrow_mut().popup_ui(ui);
                }
            });
        }
    }
    pub fn exit(&mut self) {
        self.0.borrow_mut().exit();
    }
}

fn device_label(devices: &Devices, id: Option<&String>, fallback: &str) -> String {
    devices
        .outputs
        .iter()
        .find(|d| Some(&d.id) == id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| fallback.into())
}
fn error_line(ui: &mut egui::Ui, error: &str) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [ui.available_width() - 62.0, 20.0],
            egui::Label::new(RichText::new(error).size(11.0).color(AMBER)).truncate(),
        )
        .on_hover_text(error);
        ui.small_button("Details").clicked()
    })
    .inner
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
            popup_open: false,
            popup_had_focus: false,
            popup_focus_pending: false,
            popup_anchor: None,
            notice: None,
            preview: true,
            preview_running: false,
            install_requested: false,
            #[cfg(feature = "tray-smoke-test")]
            smoke: None,
        }
    }
    fn frame(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<Event>,
        time: &mut f64,
    ) -> (egui::FullOutput, Rect, Option<Rect>) {
        frame_scaled(ctx, app, events, time, 1.0)
    }
    fn frame_scaled(
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<Event>,
        time: &mut f64,
        scale: f32,
    ) -> (egui::FullOutput, Rect, Option<Rect>) {
        *time += 0.1;
        let mut content = Rect::NOTHING;
        let mut modal = None;
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, popup::SETTINGS_SIZE)),
            time: Some(*time),
            events,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(scale);
        let mut output = ctx.run_ui(input, |root| {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(BG).inner_margin(18))
                .show(root, |ui| {
                    app.settings_content(ui);
                    content = ui.min_rect();
                });
            modal = app.show_notice(root.ctx());
        });
        output.textures_delta.clear();
        (output, content, modal)
    }
    fn text_position(shape: &Shape, value: &str) -> Option<Pos2> {
        match shape {
            Shape::Text(t) if t.galley.text() == value => Some(t.pos + t.galley.size() * 0.5),
            Shape::Vec(s) => s.iter().find_map(|s| text_position(s, value)),
            _ => None,
        }
    }
    fn any_rect(shape: &Shape, predicate: &impl Fn(&egui::epaint::RectShape) -> bool) -> bool {
        match shape {
            Shape::Rect(rect) => predicate(rect),
            Shape::Vec(shapes) => shapes.iter().any(|shape| any_rect(shape, predicate)),
            _ => false,
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
    fn dialog_dims_and_blocks_the_actual_settings_controls() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app();
        let mut time = 0.0;
        let (output, _, _) = frame(&ctx, &mut app, vec![], &mut time);
        let target = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, "Boost + leveling"))
            .unwrap();
        app.notice = Some("Reconnect your headphones, then refresh devices.".into());
        frame(&ctx, &mut app, vec![], &mut time);
        frame(&ctx, &mut app, vec![], &mut time);
        let (output, _, modal) = frame(&ctx, &mut app, vec![], &mut time);
        assert!(
            Rect::from_min_size(Pos2::ZERO, popup::SETTINGS_SIZE).contains_rect(modal.unwrap())
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|s| any_rect(&s.shape, &|r| r.fill == Color32::from_black_alpha(170)))
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|s| any_rect(&s.shape, &|r| r.stroke.color
                    == Color32::from_rgb(91, 98, 115)
                    && r.stroke.width > 0.0))
        );
        click(&ctx, &mut app, target, &mut time);
        assert_eq!(app.config.source, Source::Windows);
        assert!(
            app.notice.is_some(),
            "Backdrop clicks must not dismiss the dialog"
        );
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
        assert!(app.notice.is_none());
        for _ in 0..2 {
            frame(&ctx, &mut app, vec![], &mut time);
        }
        click(&ctx, &mut app, target, &mut time);
        assert_eq!(app.config.source, Source::Bridge);
    }
    #[test]
    fn settings_pages_and_update_states_fit_without_scrolling() {
        let screen = Rect::from_min_size(Pos2::ZERO, popup::SETTINGS_SIZE);
        for (page, source) in [
            (SetupPage::Audio, Source::Windows),
            (SetupPage::Audio, Source::Bridge),
            (SetupPage::Driver, Source::Bridge),
            (SetupPage::Updates, Source::Windows),
        ] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = app();
            app.setup_page = page;
            app.config.source = source;
            let mut time = 0.0;
            frame(&ctx, &mut app, vec![], &mut time);
            let (_, content, _) = frame(&ctx, &mut app, vec![], &mut time);
            assert!(
                screen.contains_rect(content),
                "Settings content must fit: {content:?}, page {}",
                page as u8
            );
        }
        for (ready, downloading) in [(false, false), (true, false), (false, true)] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = app();
            app.setup_page = SetupPage::Updates;
            app.updater = update::fixture_updater(ready, downloading);
            app.install_requested = downloading;
            let mut time = 0.0;
            frame(&ctx, &mut app, vec![], &mut time);
            let (_, content, _) = frame(&ctx, &mut app, vec![], &mut time);
            assert!(
                screen.contains_rect(content),
                "Update state must fit: {content:?}"
            );
        }
    }
    #[test]
    fn quick_controls_fit_the_popup_for_both_processing_paths() {
        for source in [Source::Windows, Source::Bridge] {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut app = app();
            app.config.source = source;
            app.devices.outputs.push(audio::Endpoint {
                id: "preview:headphones".into(),
                name: "Headphones with a very long USB device name that must be truncated".into(),
                cable: false,
            });
            app.config.output_id = Some("preview:headphones".into());
            app.error = Some("This example audio error must not expand the panel".into());
            let mut rect = Rect::NOTHING;
            for _ in 0..2 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, popup::SIZE)),
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE.inner_margin(10))
                            .show(root, |ui| rect = app.popup_content(ui));
                    },
                );
                output.textures_delta.clear();
            }
            assert!(
                Rect::from_min_size(Pos2::ZERO, popup::SIZE).contains_rect(rect),
                "Popup must fit: {rect:?}"
            );
        }
    }
    #[test]
    fn tray_actions_keep_preferences_separate_from_the_quick_panel() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.action(Action::Show, &ctx);
        assert!(app.popup_open && !app.setup);
        let anchor = popup::Anchor {
            left: 1800,
            top: 1040,
            right: 1830,
            bottom: 1080,
        };
        app.action(Action::TogglePopup(anchor), &ctx);
        assert!(!app.popup_open && !app.setup);
        app.action(Action::TogglePopup(anchor), &ctx);
        assert!(app.popup_open && !app.setup);
        app.action(Action::Settings, &ctx);
        assert!(app.setup && !app.popup_open);
        app.close_settings(&ctx);
        app.action(Action::Mode(Mode::Headphones), &ctx);
        assert!(!app.setup && !app.popup_open && app.config.mode == Mode::Headphones);
        app.action(Action::CheckUpdates, &ctx);
        assert!(app.setup && !app.popup_open && app.setup_page == SetupPage::Updates);
    }
    #[test]
    fn preview_cannot_install_updates_or_exit_for_them() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.install_requested = true;
        app.install_update(&ctx);
        assert!(!app.install_requested && !app.quitting);
        assert!(app.updater.error.as_ref().unwrap().contains("preview"));
        assert!(app.setup && app.setup_page == SetupPage::Updates);
    }
    #[test]
    fn disabled_startup_checks_and_preview_make_no_update_request() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.preview = false;
        app.config.check_updates_on_start = false;
        app.check_updates_on_start(&ctx);
        assert!(!app.updater.busy());
        app.preview = true;
        app.config.check_updates_on_start = true;
        app.check_updates_on_start(&ctx);
        assert!(!app.updater.busy());
        assert!(!app.setup && !app.popup_open && app.notice.is_none());
    }
    #[test]
    fn idle_ui_sleeps_but_pending_work_and_debounced_edits_keep_a_deadline() {
        let ctx = egui::Context::default();
        let mut app = app();
        app.setup = true;
        assert!(app.next_poll().is_none());
        app.popup_open = true;
        assert!(app.next_poll().is_none());
        app.config.volume_percent = 175.0;
        app.changed(&ctx);
        assert!(app.dirty && app.next_poll().is_some());
        app.last_edit = Instant::now() - Duration::from_millis(501);
        app.tick(&ctx);
        assert!(!app.dirty && app.next_poll().is_none());
        assert_eq!(app.config.volume_percent, 175.0);
        app.toggle(&ctx);
        assert!(app.is_running() && app.next_poll().is_some());
        app.toggle(&ctx);
        assert!(!app.is_running() && app.next_poll().is_none());
        app.updater = update::fixture_updater(false, true);
        assert!(app.next_poll().is_some());
    }
    #[test]
    fn settings_layout_and_font_atlas_follow_native_pixel_density() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for page in [SetupPage::Audio, SetupPage::Driver, SetupPage::Updates] {
                let ctx = egui::Context::default();
                theme::apply(&ctx);
                let mut app = app();
                app.config.source = Source::Bridge;
                app.setup_page = page;
                let mut time = 0.0;
                frame_scaled(&ctx, &mut app, vec![], &mut time, scale);
                let (output, content, _) = frame_scaled(&ctx, &mut app, vec![], &mut time, scale);
                assert_eq!(output.pixels_per_point, scale);
                assert!(
                    Rect::from_min_size(Pos2::ZERO, popup::SETTINGS_SIZE).contains_rect(content),
                    "Layout clipped at {scale}: {content:?}"
                );
                assert!(
                    !ctx.tessellate(output.shapes, output.pixels_per_point)
                        .is_empty()
                );
            }
        }
    }
}
