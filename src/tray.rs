use crate::config::Mode;
use eframe::egui;

#[derive(Clone, Copy)]
pub enum Action {
    Show,
    ToggleAudio,
    ToggleProcessing,
    Mode(Mode),
    Quit,
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::sync::mpsc::{self, Receiver};
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    pub struct Tray {
        _icon: TrayIcon,
        events: Receiver<Action>,
        audio: MenuItem,
        processing: CheckMenuItem,
        modes: Vec<CheckMenuItem>,
    }

    impl Tray {
        pub fn new(ctx: &egui::Context, window: isize) -> anyhow::Result<Self> {
            let menu = Menu::new();
            let show = MenuItem::new("Open Sound Lock", true, None);
            let audio = MenuItem::new("Start processing", true, None);
            let processing = CheckMenuItem::new("Leveling enabled", true, true, None);
            let modes: Vec<_> = Mode::ALL
                .iter()
                .map(|mode| CheckMenuItem::new(mode.label(), true, *mode == Mode::Night, None))
                .collect();
            let quit = MenuItem::new("Quit", true, None);
            menu.append_items(&[&show, &audio, &processing, &PredefinedMenuItem::separator()])?;
            for mode in &modes {
                menu.append(mode)?;
            }
            menu.append_items(&[&PredefinedMenuItem::separator(), &quit])?;
            let (sender, events) = mpsc::channel();
            let mut actions = vec![
                (show.id().clone(), Action::Show),
                (audio.id().clone(), Action::ToggleAudio),
                (processing.id().clone(), Action::ToggleProcessing),
                (quit.id().clone(), Action::Quit),
            ];
            for (index, item) in modes.iter().enumerate() {
                actions.push((item.id().clone(), Action::Mode(Mode::ALL[index])));
            }
            let event_sender = sender.clone();
            let event_ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some((_, action)) = actions.iter().find(|(id, _)| *id == event.id) {
                    let _ = event_sender.send(*action);
                    wake_window(window);
                    event_ctx.request_repaint();
                }
            }));
            let event_ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    let _ = sender.send(Action::Show);
                    wake_window(window);
                    event_ctx.request_repaint();
                }
            }));
            let rgba = icon_rgba(32);
            let icon = TrayIconBuilder::new()
                .with_tooltip("Sound Lock · steady sound")
                .with_icon(Icon::from_rgba(rgba, 32, 32)?)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()?;
            Ok(Self {
                _icon: icon,
                events,
                audio,
                processing,
                modes,
            })
        }
        pub fn poll(&self) -> Vec<Action> {
            self.events.try_iter().collect()
        }
        pub fn sync(&self, running: bool, enabled: bool, mode: Mode) {
            self.audio.set_text(if running {
                "Stop processing"
            } else {
                "Start processing"
            });
            self.processing.set_checked(enabled);
            for (i, item) in self.modes.iter().enumerate() {
                item.set_checked(Mode::ALL[i] == mode);
            }
        }
    }

    // A repaint request alone cannot restore an invisible winit window. Wake the
    // actual HWND from the tray callback, before waiting for egui to drain events.
    fn wake_window(window: isize) {
        use ::windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{IsWindow, SW_RESTORE, SetForegroundWindow, ShowWindow},
        };
        let hwnd = HWND(window as *mut std::ffi::c_void);
        // SAFETY: the handle belongs to the root window; callbacks live inside
        // its event loop. IsWindow also handles shutdown/stale handle cases.
        unsafe {
            if IsWindow(Some(hwnd)).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

pub fn icon_rgba(size: usize) -> Vec<u8> {
    let mut pixels = vec![0; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 / size as f32 - 0.5;
            let dy = y as f32 / size as f32 - 0.5;
            if dx * dx + dy * dy < 0.245 {
                let index = (y * size + x) * 4;
                pixels[index..index + 4].copy_from_slice(&[109, 230, 190, 255]);
                let column = (x * 10 / size) as i32;
                let height = match column {
                    2 => 0.16,
                    4 => 0.30,
                    6 => 0.23,
                    _ => 0.0,
                };
                if dy.abs() < height {
                    pixels[index..index + 4].copy_from_slice(&[16, 25, 32, 255]);
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
    pub fn new(_: &egui::Context, _: isize) -> anyhow::Result<Self> {
        anyhow::bail!("The system tray is supported on Windows")
    }
    pub fn poll(&self) -> Vec<Action> {
        Vec::new()
    }
    pub fn sync(&self, _: bool, _: bool, _: Mode) {}
}
