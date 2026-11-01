use crate::{
    config::Mode,
    i18n::{self, tr},
    popup::Anchor,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    TogglePopup(Anchor),
    Settings,
    CheckUpdates,
    ToggleAudio,
    ToggleProcessing,
    Mode(Mode),
    Quit,
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::sync::{
        Arc,
        mpsc::{self, Receiver},
    };
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    pub struct Tray {
        icon: TrayIcon,
        events: Receiver<Action>,
        show: MenuItem,
        settings: MenuItem,
        quit: MenuItem,
        audio: MenuItem,
        updates: MenuItem,
        processing: CheckMenuItem,
        modes: Vec<CheckMenuItem>,
        last_state: std::cell::RefCell<Option<State>>,
        #[cfg(feature = "tray-smoke-test")]
        menu_handler: Arc<dyn Fn(MenuEvent) + Send + Sync>,
        #[cfg(feature = "tray-smoke-test")]
        click_handler: Arc<dyn Fn(TrayIconEvent) + Send + Sync>,
        #[cfg(feature = "tray-smoke-test")]
        menu_actions: Vec<(tray_icon::menu::MenuId, Action)>,
    }
    struct State {
        language: i18n::Language,
        running: bool,
        enabled: bool,
        mode: Mode,
        update: Option<String>,
    }
    fn wake(ctx: &egui::Context) {
        // Logic runs while hidden; only viewport changes need a root UI pass.
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::RequestPaintWhileHidden,
        );
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    impl Tray {
        pub fn new(ctx: &egui::Context) -> anyhow::Result<Self> {
            let menu = Menu::new();
            let show = MenuItem::new(tr("Sound controls"), true, None);
            let settings = MenuItem::new(tr("Settings"), true, None);
            let updates = MenuItem::new(tr("Check for updates"), true, None);
            let audio = MenuItem::new(tr("Start processing"), true, None);
            let processing = CheckMenuItem::new(tr("Leveling enabled"), true, true, None);
            let modes: Vec<_> = Mode::ALL
                .iter()
                .map(|mode| CheckMenuItem::new(tr(mode.label()), true, *mode == Mode::Night, None))
                .collect();
            let quit = MenuItem::new(tr("Quit Sound Lock"), true, None);
            menu.append_items(&[
                &show,
                &settings,
                &updates,
                &PredefinedMenuItem::separator(),
                &audio,
                &processing,
            ])?;
            for mode in &modes {
                menu.append(mode)?;
            }
            menu.append_items(&[&PredefinedMenuItem::separator(), &quit])?;
            let mut actions = vec![
                (show.id().clone(), Action::Show),
                (settings.id().clone(), Action::Settings),
                (updates.id().clone(), Action::CheckUpdates),
                (audio.id().clone(), Action::ToggleAudio),
                (processing.id().clone(), Action::ToggleProcessing),
                (quit.id().clone(), Action::Quit),
            ];
            for (index, item) in modes.iter().enumerate() {
                actions.push((item.id().clone(), Action::Mode(Mode::ALL[index])));
            }
            let (sender, events) = mpsc::channel();
            let event_sender = sender.clone();
            let event_ctx = ctx.clone();
            let menu_actions = actions.clone();
            let menu_handler: Arc<dyn Fn(MenuEvent) + Send + Sync> = Arc::new(move |event| {
                if let Some((_, action)) = actions.iter().find(|(id, _)| *id == event.id) {
                    let _ = event_sender.send(*action);
                    wake(&event_ctx);
                }
            });
            let handler = menu_handler.clone();
            MenuEvent::set_event_handler(Some(move |event| handler(event)));
            let event_ctx = ctx.clone();
            let click_handler: Arc<dyn Fn(TrayIconEvent) + Send + Sync> = Arc::new(move |event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } = event
                {
                    let _ = sender.send(Action::TogglePopup(rect.into()));
                    wake(&event_ctx);
                }
            });
            let handler = click_handler.clone();
            TrayIconEvent::set_event_handler(Some(move |event| handler(event)));
            let icon = TrayIconBuilder::new()
                .with_tooltip(tr("Sound Lock · steady sound"))
                .with_icon({
                    // Rasterize at the shell's native small-icon metric, rather
                    // than relying on a fixed 32px bitmap to survive shrinking.
                    let size = unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                            windows_sys::Win32::UI::WindowsAndMessaging::SM_CXSMICON,
                        )
                    }
                    .clamp(16, 64) as u32;
                    Icon::from_rgba(icon_rgba(size as usize), size, size)?
                })
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()?;
            #[cfg(not(feature = "tray-smoke-test"))]
            let _ = menu_actions;
            Ok(Self {
                icon,
                events,
                show,
                settings,
                quit,
                audio,
                updates,
                processing,
                modes,
                last_state: std::cell::RefCell::new(None),
                #[cfg(feature = "tray-smoke-test")]
                menu_handler,
                #[cfg(feature = "tray-smoke-test")]
                click_handler,
                #[cfg(feature = "tray-smoke-test")]
                menu_actions,
            })
        }
        pub fn anchor(&self) -> Option<Anchor> {
            self.icon.rect().map(Into::into)
        }
        pub fn poll(&self) -> Vec<Action> {
            self.events.try_iter().collect()
        }
        pub fn sync(&self, running: bool, enabled: bool, mode: Mode, update: Option<&str>) {
            if self.last_state.borrow().as_ref().is_some_and(|last| {
                last.language == i18n::language()
                    && last.running == running
                    && last.enabled == enabled
                    && last.mode == mode
                    && last.update.as_deref() == update
            }) {
                return;
            }
            self.show.set_text(tr("Sound controls"));
            self.settings.set_text(tr("Settings"));
            self.quit.set_text(tr("Quit Sound Lock"));
            self.processing.set_text(tr("Leveling enabled"));
            self.audio.set_text(if running {
                tr("Stop processing")
            } else {
                tr("Start processing")
            });
            self.processing.set_checked(enabled);
            for (i, item) in self.modes.iter().enumerate() {
                item.set_checked(Mode::ALL[i] == mode);
                item.set_text(tr(Mode::ALL[i].label()));
            }
            self.updates.set_text(update.map_or_else(
                || tr("Check for updates").into(),
                |version| i18n::message("Update available: v{version}", "{version}", version),
            ));
            let tooltip = update.map_or_else(
                || tr("Sound Lock · steady sound").into(),
                |version| {
                    i18n::message(
                        "Sound Lock · update v{version} available",
                        "{version}",
                        version,
                    )
                },
            );
            let _ = self.icon.set_tooltip(Some(tooltip));
            *self.last_state.borrow_mut() = Some(State {
                language: i18n::language(),
                running,
                enabled,
                mode,
                update: update.map(str::to_owned),
            });
        }
        #[cfg(feature = "tray-smoke-test")]
        pub fn dispatch_click(&self) {
            let rect = self.icon.rect().expect("Tray anchor");
            (self.click_handler)(TrayIconEvent::Click {
                id: self.icon.id().clone(),
                position: rect.position,
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
            });
        }
        #[cfg(feature = "tray-smoke-test")]
        pub fn dispatch_menu(&self, action: Action) {
            let (id, _) = self
                .menu_actions
                .iter()
                .find(|(_, a)| *a == action)
                .expect("Tray action");
            (self.menu_handler)(MenuEvent { id: id.clone() });
        }
    }
}
pub fn icon_rgba(size: usize) -> Vec<u8> {
    let mut pixels = vec![0; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 / size as f32 - 0.5;
            let dy = y as f32 / size as f32 - 0.5;
            if dx * dx + dy * dy < 0.235 {
                let index = (y * size + x) * 4;
                pixels[index..index + 4].copy_from_slice(&[54, 62, 79, 255]);
                let height = match x * 12 / size {
                    3 => 0.14,
                    5 | 6 => 0.29,
                    8 => 0.21,
                    _ => 0.0,
                };
                if dy.abs() < height {
                    pixels[index..index + 4].copy_from_slice(&[218, 228, 245, 255]);
                }
            }
        }
    }
    pixels
}
#[cfg(windows)]
pub use windows::Tray;
#[cfg(not(windows))]
pub struct Tray;
#[cfg(not(windows))]
impl Tray {
    pub fn new(_: &egui::Context) -> anyhow::Result<Self> {
        anyhow::bail!("The system tray is supported on Windows")
    }
    pub fn anchor(&self) -> Option<Anchor> {
        None
    }
    pub fn poll(&self) -> Vec<Action> {
        Vec::new()
    }
    pub fn sync(&self, _: bool, _: bool, _: Mode, _: Option<&str>) {}
}
