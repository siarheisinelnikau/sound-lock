use egui::{self, Pos2, Vec2};

pub const TITLE: &str = "Sound Lock — Sound controls";
pub const SETTINGS_TITLE: &str = "Sound Lock — Settings";
pub const SIZE: Vec2 = Vec2::new(368.0, 304.0);
pub const SETTINGS_SIZE: Vec2 = Vec2::new(560.0, 620.0);
pub fn id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("sound-controls-popup")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
#[cfg(windows)]
impl From<tray_icon::Rect> for Anchor {
    fn from(rect: tray_icon::Rect) -> Self {
        Self {
            left: rect.position.x as i32,
            top: rect.position.y as i32,
            right: (rect.position.x + rect.size.width as f64) as i32,
            bottom: (rect.position.y + rect.size.height as f64) as i32,
        }
    }
}
fn place(work: [i32; 4], anchor: Anchor, width: i32, height: i32) -> (i32, i32) {
    let left = work[0] + 8;
    let top = work[1] + 8;
    let x = ((anchor.left + anchor.right - width) / 2).clamp(left, (work[2] - width - 8).max(left));
    let above = anchor.top - height - 8;
    let y = if above >= top {
        above
    } else {
        anchor.bottom + 8
    };
    (x, y.clamp(top, (work[3] - height - 8).max(top)))
}
pub fn dismiss(
    had_focus: bool,
    focused: Option<bool>,
    over_tray: bool,
    escape: bool,
    close: bool,
) -> bool {
    escape || close || (had_focus && focused == Some(false) && !over_tray)
}
#[cfg(windows)]
pub fn cursor_in(anchor: Anchor) -> bool {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).is_ok()
            && point.x >= anchor.left
            && point.x <= anchor.right
            && point.y >= anchor.top
            && point.y <= anchor.bottom
    }
}
#[cfg(not(windows))]
pub fn cursor_in(_: Anchor) -> bool {
    false
}
#[cfg(windows)]
pub fn position(anchor: Option<Anchor>, size: Vec2, ctx: &egui::Context) -> Pos2 {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::GetCursorPos,
        },
    };
    let mut point = POINT::default();
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut x_dpi, mut y_dpi) = (96, 96);
    unsafe {
        let _ = GetCursorPos(&mut point);
        if let Some(a) = anchor {
            point = POINT {
                x: (a.left + a.right) / 2,
                y: (a.top + a.bottom) / 2,
            };
        }
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Pos2::new(32.0, 32.0);
        }
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x_dpi, &mut y_dpi);
    }
    let work = info.rcWork;
    let scale = x_dpi.max(96) as f32 / 96.0 * ctx.zoom_factor();
    let anchor = anchor.unwrap_or(Anchor {
        left: work.right - 32,
        top: work.bottom + 8,
        right: work.right - 8,
        bottom: work.bottom + 32,
    });
    let (x, y) = place(
        [work.left, work.top, work.right, work.bottom],
        anchor,
        (size.x * scale).ceil() as i32,
        (size.y * scale).ceil() as i32,
    );
    let current_scale = ctx.pixels_per_point();
    Pos2::new(x as f32 / current_scale, y as f32 / current_scale)
}
#[cfg(not(windows))]
pub fn position(_: Option<Anchor>, _: Vec2, _: &egui::Context) -> Pos2 {
    Pos2::new(32.0, 32.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_stays_inside_work_area_for_each_taskbar_edge_and_mixed_dpi() {
        for work in [[0, 0, 1920, 1040], [-2560, 0, 0, 1400], [0, -1440, 2560, 0]] {
            let [left, top, right, bottom] = work;
            for anchor in [
                Anchor {
                    left: right - 40,
                    top: bottom,
                    right,
                    bottom: bottom + 40,
                },
                Anchor {
                    left,
                    top: top - 40,
                    right: left + 40,
                    bottom: top,
                },
                Anchor {
                    left: left - 40,
                    top: top + 100,
                    right: left,
                    bottom: top + 140,
                },
                Anchor {
                    left: right,
                    top: top + 100,
                    right: right + 40,
                    bottom: top + 140,
                },
            ] {
                for scale in [1.0_f32, 1.25, 1.5, 2.0] {
                    let (width, height) = ((SIZE.x * scale) as i32, (SIZE.y * scale) as i32);
                    let (x, y) = place(work, anchor, width, height);
                    assert!(
                        x >= left + 8
                            && y >= top + 8
                            && x + width <= right - 8
                            && y + height <= bottom - 8
                    );
                }
            }
        }
    }
    #[test]
    fn focus_loss_dismisses_without_breaking_the_tray_toggle() {
        assert!(!dismiss(false, Some(false), false, false, false));
        assert!(!dismiss(true, Some(false), true, false, false));
        assert!(dismiss(true, Some(false), false, false, false));
        assert!(dismiss(false, Some(true), false, true, false));
        assert!(dismiss(false, None, false, false, true));
    }
}
