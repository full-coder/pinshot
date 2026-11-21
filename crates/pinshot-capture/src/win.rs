//! Windows screen-capture backend.
//!
//! GDI is the one grab path that exists on every Windows version without extra
//! runtimes: `BitBlt` from the desktop DC returns the composited screen as the user
//! sees it. Windows.Graphics.Capture would give HDR and per-window frames, but it is
//! asynchronous, needs a D3D device and behaves differently on Windows 10, so the
//! synchronous GDI path is what this build ships.
//!
//! DPI: the process is per-monitor DPI aware (Tauri sets it; `ensure_dpi_awareness`
//! covers the headless CLI). Every coordinate below is therefore a *physical* pixel
//! until it is divided by the monitor's scale factor, which is exactly the
//! points/pixels split `pinshot_core::Monitor` expects.

#![cfg(windows)]

use pinshot_core::{Error, Image, Monitor, Point, Rect, Result};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use windows_sys::Win32::Foundation::{LPARAM, POINT, RECT};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HMONITOR, MONITORINFOEXW,
    SRCCOPY,
};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetWindowLongW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, MessageBoxW, GWL_EXSTYLE,
    MB_ICONERROR, MB_OK, MB_TOPMOST, MONITORINFOF_PRIMARY, WS_EX_TOOLWINDOW,
};

use crate::CaptureBackend;

/// A monitor as the OS reports it: physical pixels plus its effective DPI.
#[derive(Clone, Copy)]
pub(crate) struct RawMonitor {
    pub(crate) handle: HMONITOR,
    pub(crate) rect: RECT,
    pub(crate) dpi: u32,
    pub(crate) primary: bool,
    device: [u16; 32],
}

impl RawMonitor {
    pub(crate) fn scale(&self) -> f32 {
        (self.dpi as f32 / 96.0).max(0.5)
    }

    pub(crate) fn pixel_size(&self) -> (u32, u32) {
        (
            (self.rect.right - self.rect.left).max(1) as u32,
            (self.rect.bottom - self.rect.top).max(1) as u32,
        )
    }
}

/// Makes the process per-monitor DPI aware. Harmless (and a no-op) once Tauri or a
/// manifest has already done it: Windows refuses the second call, which is ignored.
pub fn ensure_dpi_awareness() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

unsafe extern "system" fn collect_monitor(
    handle: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> i32 {
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(handle, &mut info.monitorInfo) == 0 {
        return 1;
    }
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) != 0 || dpi_x == 0 {
        dpi_x = 96;
    }
    let list = unsafe { &mut *(data as *mut Vec<RawMonitor>) };
    list.push(RawMonitor {
        handle,
        rect: info.monitorInfo.rcMonitor,
        dpi: dpi_x,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        device: info.szDevice,
    });
    1
}

/// Every active display, in the OS's enumeration order.
pub(crate) fn raw_monitors() -> Vec<RawMonitor> {
    ensure_dpi_awareness();
    let mut list: Vec<RawMonitor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut list as *mut _ as LPARAM,
        );
    }
    list
}

fn device_name(device: &[u16; 32]) -> String {
    let end = device
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(device.len());
    String::from_utf16_lossy(&device[..end])
        .trim_start_matches("\\\\.\\")
        .to_owned()
}

pub(crate) fn points_rect(rect: RECT, scale: f32) -> Rect {
    let left = (rect.left as f32 / scale).round() as i32;
    let top = (rect.top as f32 / scale).round() as i32;
    let right = (rect.right as f32 / scale).round() as i32;
    let bottom = (rect.bottom as f32 / scale).round() as i32;
    Rect::new(
        left,
        top,
        (right - left).max(0) as u32,
        (bottom - top).max(0) as u32,
    )
}

/// Converts a global point (logical points) to physical screen pixels.
pub(crate) fn point_to_physical(point: Point) -> (i32, i32) {
    for raw in raw_monitors() {
        let scale = raw.scale();
        let bounds = points_rect(raw.rect, scale);
        if point.x >= bounds.x
            && point.x < bounds.x + bounds.w as i32
            && point.y >= bounds.y
            && point.y < bounds.y + bounds.h as i32
        {
            let x = raw.rect.left + ((point.x - bounds.x) as f32 * scale).round() as i32;
            let y = raw.rect.top + ((point.y - bounds.y) as f32 * scale).round() as i32;
            return (x, y);
        }
    }
    (point.x, point.y)
}

/// The monitor whose physical rect contains the point, falling back to the primary one.
pub(crate) fn monitor_at_physical(x: i32, y: i32) -> Option<RawMonitor> {
    let all = raw_monitors();
    all.iter()
        .copied()
        .find(|raw| {
            x >= raw.rect.left && x < raw.rect.right && y >= raw.rect.top && y < raw.rect.bottom
        })
        .or_else(|| all.iter().copied().find(|raw| raw.primary))
        .or_else(|| all.first().copied())
}

/// Converts a physical screen rect to global points using the display it sits on.
pub(crate) fn physical_rect_to_points(rect: RECT) -> Rect {
    let scale = monitor_at_physical(
        rect.left + (rect.right - rect.left) / 2,
        rect.top + (rect.bottom - rect.top) / 2,
    )
    .map(|raw| raw.scale())
    .unwrap_or(1.0);
    points_rect(rect, scale)
}

/// Converts a global rectangle in points to the physical pixels of the display it
/// sits on. Using the rectangle's centre avoids the exclusive right/bottom edge,
/// which does not belong to any monitor and used to fall back to points-as-pixels.
pub(crate) fn physical_rect(bounds: Rect) -> (i32, i32, u32, u32) {
    let centre_x = bounds.x + bounds.w as i32 / 2;
    let centre_y = bounds.y + bounds.h as i32 / 2;
    for raw in raw_monitors() {
        let scale = raw.scale();
        let logical = points_rect(raw.rect, scale);
        if centre_x >= logical.x
            && centre_x < logical.x + logical.w as i32
            && centre_y >= logical.y
            && centre_y < logical.y + logical.h as i32
        {
            let x = raw.rect.left + ((bounds.x - logical.x) as f32 * scale).round() as i32;
            let y = raw.rect.top + ((bounds.y - logical.y) as f32 * scale).round() as i32;
            return (
                x,
                y,
                (bounds.w as f32 * scale).round() as u32,
                (bounds.h as f32 * scale).round() as u32,
            );
        }
    }
    (bounds.x, bounds.y, bounds.w, bounds.h)
}

/// Grabs an arbitrary physical screen rectangle as RGBA.
pub(crate) fn capture_rect(x: i32, y: i32, width: u32, height: u32) -> Result<Image> {
    if width == 0 || height == 0 {
        return Err(Error::internal("cannot capture an empty rectangle"));
    }
    ensure_dpi_awareness();
    unsafe {
        let screen: HDC = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err(Error::internal("cannot open the desktop device context"));
        }
        let memory = CreateCompatibleDC(screen);
        if memory.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
            return Err(Error::internal("cannot create a capture device context"));
        }
        let bitmap: HBITMAP = CreateCompatibleBitmap(screen, width as i32, height as i32);
        if bitmap.is_null() {
            DeleteDC(memory);
            ReleaseDC(std::ptr::null_mut(), screen);
            return Err(Error::internal("cannot allocate the capture bitmap"));
        }
        let previous = SelectObject(memory, bitmap);
        // CAPTUREBLT includes layered windows (DWM-composited surfaces, tooltips).
        let copied = BitBlt(
            memory,
            0,
            0,
            width as i32,
            height as i32,
            screen,
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        );
        // GetDIBits requires the bitmap not to be selected into a DC.
        SelectObject(memory, previous);

        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width as i32;
        // Negative height asks for a top-down DIB, which is the order `Image` uses.
        info.bmiHeader.biHeight = -(height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let lines = GetDIBits(
            screen,
            bitmap,
            0,
            height,
            pixels.as_mut_ptr() as *mut c_void,
            &mut info,
            DIB_RGB_COLORS,
        );

        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);

        if copied == 0 || lines == 0 {
            return Err(Error::internal(
                "BitBlt returned no pixels (the desktop may be locked)",
            ));
        }
        // GDI writes BGRA and leaves alpha at zero; the rest of the app expects RGBA.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Image::from_rgba(width, height, pixels)
            .ok_or_else(|| Error::internal("capture produced an unexpected buffer size"))
    }
}

#[derive(Default)]
pub struct WindowsBackend;

impl WindowsBackend {
    fn find(&self, id: u32) -> Result<RawMonitor> {
        raw_monitors()
            .into_iter()
            .find(|raw| raw.handle as usize as u32 == id)
            .ok_or_else(|| Error::not_found("that display is no longer connected"))
    }
}

impl CaptureBackend for WindowsBackend {
    fn monitors(&self) -> Result<Vec<Monitor>> {
        let mut monitors: Vec<Monitor> = raw_monitors()
            .into_iter()
            .enumerate()
            .map(|(index, raw)| {
                let scale = raw.scale();
                let (pixel_width, pixel_height) = raw.pixel_size();
                let name = device_name(&raw.device);
                Monitor {
                    id: raw.handle as usize as u32,
                    name: if name.is_empty() {
                        format!("Display {}", index + 1)
                    } else {
                        name
                    },
                    bounds: points_rect(raw.rect, scale),
                    scale,
                    pixel_width,
                    pixel_height,
                    primary: raw.primary,
                }
            })
            .collect();
        if monitors.is_empty() {
            return Err(Error::not_found("no active displays"));
        }
        // The overlay opens on the primary display and most selections start there.
        monitors.sort_by_key(|monitor| !monitor.primary);
        Ok(monitors)
    }

    fn grab_all(&self, monitors: &[Monitor]) -> Result<Vec<Image>> {
        // GDI BitBlt for one display is independent of the others; run them concurrently
        // like MacBackend so the overlay waits for the slowest display, not the sum.
        let mut frames: Vec<Option<Result<Image>>> = vec![None; monitors.len()];
        std::thread::scope(|scope| {
            let handles: Vec<_> = monitors
                .iter()
                .map(|monitor| scope.spawn(move || self.grab(monitor)))
                .collect();
            for (slot, handle) in frames.iter_mut().zip(handles) {
                *slot = Some(
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(Error::internal("a capture thread panicked"))),
                );
            }
        });
        frames
            .into_iter()
            .map(|frame| frame.unwrap_or_else(|| Err(Error::internal("missing frame"))))
            .collect()
    }

    fn grab(&self, monitor: &Monitor) -> Result<Image> {
        let raw = self.find(monitor.id)?;
        let (width, height) = raw.pixel_size();
        capture_rect(raw.rect.left, raw.rect.top, width, height)
    }

    fn cursor(&self) -> Option<Point> {
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return None;
        }
        let raw = monitor_at_physical(point.x, point.y)?;
        let scale = raw.scale();
        let bounds = points_rect(raw.rect, scale);
        Some(Point::new(
            bounds.x + ((point.x - raw.rect.left) as f32 / scale).round() as i32,
            bounds.y + ((point.y - raw.rect.top) as f32 / scale).round() as i32,
        ))
    }

    fn permission(&self) -> crate::Permission {
        // Windows has no screen-recording grant: any user process may read the screen.
        crate::Permission::Granted
    }
}

// ── window enumeration, shared by the scroll capture and the CLI ─────────────

unsafe extern "system" fn collect_window(hwnd: *mut c_void, data: LPARAM) -> i32 {
    let windows = unsafe { &mut *(data as *mut Vec<crate::windows::WindowInfo>) };
    if unsafe { IsWindowVisible(hwnd) } == 0 || unsafe { IsIconic(hwnd) } != 0 {
        return 1;
    }
    if unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW != 0 {
        return 1;
    }
    let title_length = unsafe { GetWindowTextLengthW(hwnd) };
    if title_length <= 0 {
        return 1;
    }
    let mut cloaked = 0u32;
    let cloaked_ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut _ as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    if cloaked_ok == 0 && cloaked != 0 {
        // A suspended UWP app keeps its window alive but invisible.
        return 1;
    }

    let mut rect: RECT = zeroed();
    let mut got = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut rect as *mut _ as *mut c_void,
            size_of::<RECT>() as u32,
        )
    } == 0;
    if !got {
        got = unsafe { GetWindowRect(hwnd, &mut rect) } != 0;
    }
    if !got || rect.right - rect.left < 32 || rect.bottom - rect.top < 32 {
        return 1;
    }

    let mut title_buffer = vec![0u16; title_length as usize + 1];
    let copied =
        unsafe { GetWindowTextW(hwnd, title_buffer.as_mut_ptr(), title_buffer.len() as i32) };
    let title = String::from_utf16_lossy(&title_buffer[..copied.max(0) as usize]);
    if title.trim().is_empty() {
        return 1;
    }

    let mut class_buffer = [0u16; 256];
    let class_length =
        unsafe { GetClassNameW(hwnd, class_buffer.as_mut_ptr(), class_buffer.len() as i32) };
    let app = String::from_utf16_lossy(&class_buffer[..class_length.max(0) as usize]);

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };

    windows.push(crate::windows::WindowInfo {
        id: hwnd as usize as u32,
        pid: pid as i32,
        app,
        title,
        bounds: physical_rect_to_points(rect),
        layer: 0,
    });
    1
}

pub(crate) fn list_windows() -> Vec<crate::windows::WindowInfo> {
    ensure_dpi_awareness();
    let mut windows: Vec<crate::windows::WindowInfo> = Vec::new();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
            Some(collect_window),
            &mut windows as *mut _ as LPARAM,
        );
    }
    windows
}

/// Grabs the physical rectangle of a window, in points, through the screen.
///
/// `PrintWindow` was the alternative, but Chromium/Electron apps answer it with a
/// black frame. The scroll capture hides the overlay before it grabs, so reading the
/// screen is both correct and fast there.
pub(crate) fn grab_window_rect(bounds: Rect) -> Result<Image> {
    let (x, y, width, height) = physical_rect(bounds);
    capture_rect(x, y, width, height)
}

/// The user's UI language is Chinese, read the same way Settings → Language does.
pub fn system_is_chinese() -> bool {
    // LANGID low 10 bits are the primary language; 0x04 is Chinese.
    unsafe { GetUserDefaultUILanguage() & 0x3ff == 0x04 }
}

/// A native error dialog, for failures that happen before the tray exists.
pub fn message_box(title: &str, message: &str) {
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let message: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_TOPMOST,
        );
    }
}
