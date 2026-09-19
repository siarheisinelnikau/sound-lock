//! Retained, top-down BGRA bitmap. Popup alpha is presented by UpdateLayeredWindow.
use anyhow::{Result, ensure};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{mem::size_of, ptr};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub struct Bitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut [u8; 4],
    pub width: u32,
    pub height: u32,
}
impl Bitmap {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        ensure!(
            width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 16_000_000,
            "Invalid UI bitmap size"
        );
        unsafe {
            let dc = CreateCompatibleDC(ptr::null_mut());
            ensure!(!dc.is_null(), "Could not create the GDI bitmap context");
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width as i32;
            info.bmiHeader.biHeight = -(height as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            let mut bits = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                anyhow::bail!("Could not allocate the UI bitmap");
            }
            let previous = SelectObject(dc, bitmap.cast());
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }
    pub fn pixels(&mut self) -> &mut [[u8; 4]] {
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, self.width as usize * self.height as usize)
        }
    }
    pub fn present(&self, window: &winit::window::Window, alpha: bool) -> Result<()> {
        let hwnd = hwnd(window)?;
        unsafe {
            if alpha {
                transparent(window)?;
                let mut rect: RECT = std::mem::zeroed();
                ensure!(
                    GetWindowRect(hwnd, &mut rect) != 0,
                    "Could not locate the popup"
                );
                let position = POINT {
                    x: rect.left,
                    y: rect.top,
                };
                let origin = POINT { x: 0, y: 0 };
                let size = SIZE {
                    cx: self.width as i32,
                    cy: self.height as i32,
                };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                ensure!(
                    UpdateLayeredWindow(
                        hwnd,
                        ptr::null_mut(),
                        &position,
                        &size,
                        self.dc,
                        &origin,
                        0,
                        &blend,
                        ULW_ALPHA
                    ) != 0,
                    "Could not present the transparent popup: {}",
                    std::io::Error::last_os_error()
                );
            } else {
                let dc = GetDC(hwnd);
                ensure!(
                    !dc.is_null(),
                    "Could not access the Settings drawing surface"
                );
                let ok = BitBlt(
                    dc,
                    0,
                    0,
                    self.width as i32,
                    self.height as i32,
                    self.dc,
                    0,
                    0,
                    SRCCOPY,
                );
                ReleaseDC(hwnd, dc);
                ensure!(ok != 0, "Could not present Settings");
            }
        }
        Ok(())
    }
}
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap.cast());
            DeleteDC(self.dc);
        }
    }
}
pub fn hwnd(window: &winit::window::Window) -> Result<HWND> {
    match window.window_handle()?.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as HWND),
        _ => anyhow::bail!("Windows GDI requires a Win32 window"),
    }
}
pub fn transparent(window: &winit::window::Window) -> Result<()> {
    let hwnd = hwnd(window)?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
    }
    Ok(())
}
