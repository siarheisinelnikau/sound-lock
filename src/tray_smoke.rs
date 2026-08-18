//! Opt-in native-window lifecycle checks. Preview mode never opens audio devices
//! or writes the user's settings; callbacks are the actual installed tray ones.
use super::*;

#[derive(Debug, PartialEq)]
enum Stage {
    Ready,
    Opened,
    ToggledOff,
    ToggledOn,
    Settings,
    OverSettings,
    FocusedSettings,
    Minimized,
    Restored,
    Hidden,
    ModeChanged,
    Reopened,
    HiddenForQuit,
    Quit,
}
pub struct SmokeTest {
    observe_root: Box<dyn Fn() -> (bool, bool)>,
    report: std::path::PathBuf,
    started: Instant,
    stage: Stage,
    passed: Vec<&'static str>,
    quiet: bool,
}
impl SmokeTest {
    pub fn new(cc: &eframe::CreationContext<'_>, preview: bool) -> Option<Self> {
        let report = std::env::var_os("SOUND_LOCK_TRAY_SMOKE_REPORT")?;
        assert!(preview, "Native tray checks require --preview");
        let window = cc.winit_window().expect("Native root window").clone();
        Some(Self {
            observe_root: Box::new(move || {
                (
                    window.is_visible().unwrap_or(false),
                    window.is_minimized().unwrap_or(false),
                )
            }),
            report: report.into(),
            started: Instant::now(),
            stage: Stage::Ready,
            passed: Vec::new(),
            quiet: std::env::args().any(|a| a == "--startup"),
        })
    }
    fn pass(&mut self, name: &'static str) {
        eprintln!("PASS {name}");
        self.passed.push(name);
    }
    pub fn tick(&mut self, app: &mut App, ctx: &egui::Context) {
        assert!(
            self.started.elapsed() < Duration::from_secs(25),
            "Native tray test timed out at {:?}; passed {:?}",
            self.stage,
            self.passed
        );
        let (root, minimized) = (self.observe_root)();
        let visible = popup_visible();
        let menu = |action| app.tray.as_ref().expect("Tray").dispatch_menu(action);
        match self.stage {
            Stage::Ready if self.started.elapsed() > Duration::from_millis(650) => {
                assert!(!root, "Settings must not appear on startup");
                if self.quiet {
                    assert!(!visible);
                    self.pass("startup_stays_in_tray");
                    menu(Action::Show);
                }
                self.stage = Stage::Opened;
            }
            Stage::Opened if visible && !root => {
                self.pass("tray_opens_only_quick_controls");
                app.tray.as_ref().unwrap().dispatch_click();
                self.stage = Stage::ToggledOff;
            }
            Stage::ToggledOff if !visible && !root => {
                self.pass("second_click_dismisses_popup");
                app.tray.as_ref().unwrap().dispatch_click();
                self.stage = Stage::ToggledOn;
            }
            Stage::ToggledOn if visible && !root => {
                self.pass("click_reopens_popup");
                menu(Action::Settings);
                self.stage = Stage::Settings;
            }
            Stage::Settings if root && !visible => {
                self.pass("settings_is_a_separate_window");
                app.show_popup(None, ctx);
                self.stage = Stage::OverSettings;
            }
            Stage::OverSettings if root && visible && app.popup_had_focus => {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                self.stage = Stage::FocusedSettings;
            }
            Stage::FocusedSettings if root && !visible => {
                self.pass("focus_loss_dismisses_popup");
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Minimized(true),
                );
                self.stage = Stage::Minimized;
            }
            Stage::Minimized if minimized => {
                self.pass("settings_can_minimize");
                menu(Action::Settings);
                self.stage = Stage::Restored;
            }
            Stage::Restored if root && !minimized => {
                self.pass("settings_restores_from_tray");
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                self.stage = Stage::Hidden;
            }
            Stage::Hidden if !root && !visible => {
                self.pass("closing_settings_keeps_app_in_tray");
                menu(Action::Mode(Mode::Headphones));
                self.stage = Stage::ModeChanged;
            }
            Stage::ModeChanged if app.config.mode == Mode::Headphones => {
                assert!(!root && !visible);
                self.pass("hidden_tray_commands_do_not_flash_windows");
                menu(Action::Show);
                self.stage = Stage::Reopened;
            }
            Stage::Reopened if visible && !root => {
                self.pass("popup_reopens_after_both_windows_were_hidden");
                app.hide_popup(ctx);
                self.stage = Stage::HiddenForQuit;
            }
            Stage::HiddenForQuit if !root && !visible => {
                menu(Action::Quit);
                self.stage = Stage::Quit;
            }
            _ => {}
        }
        ctx.request_repaint_after(Duration::from_millis(40));
    }
    pub fn finish(&self) {
        assert_eq!(
            self.stage,
            Stage::Quit,
            "Unexpected exit during tray regression"
        );
        let mut passed = self.passed.clone();
        passed.push("quit_while_both_windows_hidden");
        let report = serde_json::json!({"status":"passed","steps":passed,"audio":"preview only; no audio devices or user settings touched"});
        std::fs::write(&self.report, serde_json::to_vec_pretty(&report).unwrap())
            .expect("Write tray report");
    }
}
fn popup_visible() -> bool {
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{
                EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
            },
        },
        core::BOOL,
    };
    unsafe extern "system" fn find(window: HWND, data: LPARAM) -> BOOL {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, Some(&mut pid));
        }
        if pid != std::process::id() {
            return true.into();
        }
        let mut text = [0_u16; 128];
        let len = unsafe { GetWindowTextW(window, &mut text) };
        if String::from_utf16_lossy(&text[..len.max(0) as usize]) == popup::TITLE {
            unsafe {
                *(data.0 as *mut bool) = IsWindowVisible(window).as_bool();
            }
        }
        true.into()
    }
    let mut visible = false;
    let _ = unsafe { EnumWindows(Some(find), LPARAM(&mut visible as *mut _ as isize)) };
    visible
}
