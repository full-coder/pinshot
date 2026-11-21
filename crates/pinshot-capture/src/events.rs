//! Synthetic input, used to drive the app from a script and to power auto-scroll.
//!
//! These are the same primitives the scroll-capture injector needs (`spec/04` §5):
//! post a wheel event at a point, in pixels, without moving the pointer. Posting
//! events requires the Accessibility permission; without it macOS silently drops
//! them, so `doctor` reports the permission state separately.

use pinshot_core::Point;

#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use objc2_core_foundation::CGPoint;
    use objc2_core_graphics::{CGEvent, CGEventTapLocation, CGEventType, CGScrollEventUnit};

    fn post(event: Option<objc2_core_foundation::CFRetained<CGEvent>>) -> bool {
        let Some(event) = event else { return false };
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        true
    }

    fn mouse(event_type: CGEventType, at: Point) -> bool {
        let position = CGPoint {
            x: at.x as f64,
            y: at.y as f64,
        };
        post(CGEvent::new_mouse_event(
            None,
            event_type,
            position,
            objc2_core_graphics::CGMouseButton::Left,
        ))
    }

    pub fn click(at: Point) -> bool {
        mouse(CGEventType::MouseMoved, at);
        mouse(CGEventType::LeftMouseDown, at);
        mouse(CGEventType::LeftMouseUp, at)
    }

    /// Press, drag through a few intermediate points, release.
    pub fn drag(from: Point, to: Point, steps: u32) -> bool {
        let steps = steps.max(1);
        mouse(CGEventType::MouseMoved, from);
        if !mouse(CGEventType::LeftMouseDown, from) {
            return false;
        }
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let point = Point::new(
                (from.x as f32 + (to.x - from.x) as f32 * t).round() as i32,
                (from.y as f32 + (to.y - from.y) as f32 * t).round() as i32,
            );
            if !mouse(CGEventType::LeftMouseDragged, point) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        mouse(CGEventType::LeftMouseUp, to)
    }

    /// Whether this process may synthesise input. Auto-scroll is dead without it.
    ///
    /// Note: this returns true for processes that have *never* been granted anything —
    /// it reflects the request state, not the actual TCC decision. `ax_trusted` is the
    /// reliable check on macOS 14.
    pub fn post_access() -> bool {
        objc2_core_graphics::CGPreflightPostEventAccess()
    }

    /// The real Accessibility check. `prompt = true` shows the system dialog and adds
    /// the app to 系统设置 → 辅助功能.
    pub fn ax_trusted(prompt: bool) -> bool {
        use objc2_application_services::kAXTrustedCheckOptionPrompt;
        use objc2_core_foundation::{kCFBooleanTrue, CFDictionary, CFString};

        if !prompt {
            return unsafe { objc2_application_services::AXIsProcessTrustedWithOptions(None) };
        }
        unsafe {
            let key: &CFString = kAXTrustedCheckOptionPrompt;
            let value = kCFBooleanTrue.expect("kCFBooleanTrue");
            let options = CFDictionary::from_slices(&[key], &[value]);
            let erased: &CFDictionary = options.as_ref();
            objc2_application_services::AXIsProcessTrustedWithOptions(Some(erased))
        }
    }

    /// Requests Accessibility access; returns whether it is already granted.
    pub fn request_ax_trust() -> bool {
        ax_trusted(true)
    }

    /// Asks the system for input-synthesis permission.
    ///
    /// macOS only lists an app under 辅助功能 once it has *asked*; a silent
    /// `CGPreflightPostEventAccess()` never adds it, which is why PinShot was missing
    /// from that pane. This shows the real prompt and registers the app.
    pub fn request_post_access() -> bool {
        objc2_core_graphics::CGRequestPostEventAccess()
    }

    pub fn key(virtual_key: u16) -> bool {
        let down = post(CGEvent::new_keyboard_event(None, virtual_key, true));
        let up = post(CGEvent::new_keyboard_event(None, virtual_key, false));
        down && up
    }
    /// Brings an application to the front so global wheel events reach it.
    ///
    /// `CGEventPostToPid` is not enough for scroll: WebKit routes the wheel to the
    /// window under the cursor inside the *active* app. Activating the page's app first
    /// and posting globally is what actually scrolls it.
    pub fn activate_app(pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        unsafe {
            let class = objc2::class!(NSRunningApplication);
            let application: *mut AnyObject = objc2::msg_send![
                class,
                runningApplicationWithProcessIdentifier: pid
            ];
            if application.is_null() {
                return false;
            }
            // 1 = AllWindows, 2 = IgnoringOtherApps. Without the latter macOS defers
            // activation to the next user interaction and our wheel goes nowhere.
            objc2::msg_send![application, activateWithOptions: 3isize]
        }
    }

    /// The pid of the application that currently owns the keyboard focus.
    pub fn frontmost_pid() -> i32 {
        unsafe {
            let workspace: *mut AnyObject =
                objc2::msg_send![objc2::class!(NSWorkspace), sharedWorkspace];
            if workspace.is_null() {
                return 0;
            }
            let application: *mut AnyObject = objc2::msg_send![workspace, frontmostApplication];
            if application.is_null() {
                return 0;
            }
            objc2::msg_send![application, processIdentifier]
        }
    }

    /// Scrolls by roughly `points` (logical units), expressed in wheel lines.
    ///
    /// WebKit ignores synthetic wheel events posted in `Pixel` units (verified against
    /// Safari: the page does not move), while `Line` units work. A line is ~40 points,
    /// and the caller must convert device pixels to points *before* calling this —
    /// mixing the two doubles the step on a 2x display, which leaves consecutive
    /// frames with no overlap at all.
    pub fn scroll_points(at: Point, points: i32) -> bool {
        const POINTS_PER_LINE: i32 = 40;
        let lines = if points >= 0 {
            (points / POINTS_PER_LINE).max(1)
        } else {
            (points / POINTS_PER_LINE).min(-1)
        };
        scroll(at, lines)
    }

    /// Moves the real pointer.
    ///
    /// A synthetic wheel event is delivered to the window under the *actual* cursor;
    /// a posted `MouseMoved` does not reliably move it, so scrolling a specific window
    /// needs a warp first.
    pub fn warp(at: Point) -> bool {
        let point = CGPoint {
            x: at.x as f64,
            y: at.y as f64,
        };
        objc2_core_graphics::CGWarpMouseCursorPosition(point)
            == objc2_core_graphics::CGError::Success
    }

    pub fn scroll(at: Point, lines: i32) -> bool {
        mouse(CGEventType::MouseMoved, at);
        post(CGEvent::new_scroll_wheel_event2(
            None,
            CGScrollEventUnit::Line,
            1,
            lines,
            0,
            0,
        ))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowLongW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SetCursorPos, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_RESTORE,
        WS_EX_TOOLWINDOW,
    };

    fn send(input: INPUT) -> bool {
        unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) == 1 }
    }

    fn mouse(flags: u32, data: u32) -> bool {
        send(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
    }

    fn send_key(code: u16, up: bool) -> bool {
        send(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: code as VIRTUAL_KEY,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
    }

    pub fn click(at: Point) -> bool {
        warp(at) && mouse(MOUSEEVENTF_LEFTDOWN, 0) && mouse(MOUSEEVENTF_LEFTUP, 0)
    }

    pub fn drag(from: Point, to: Point, steps: u32) -> bool {
        let steps = steps.max(1);
        if !warp(from) || !mouse(MOUSEEVENTF_LEFTDOWN, 0) {
            return false;
        }
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let point = Point::new(
                (from.x as f32 + (to.x - from.x) as f32 * t).round() as i32,
                (from.y as f32 + (to.y - from.y) as f32 * t).round() as i32,
            );
            warp(point);
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        warp(to) && mouse(MOUSEEVENTF_LEFTUP, 0)
    }

    pub fn key(code: u16) -> bool {
        send_key(code, false) && send_key(code, true)
    }

    /// Windows has no per-app input-synthesis grant for same-integrity processes.
    pub fn post_access() -> bool {
        true
    }

    pub fn request_post_access() -> bool {
        true
    }

    pub fn ax_trusted(_prompt: bool) -> bool {
        true
    }

    pub fn request_ax_trust() -> bool {
        true
    }

    pub fn scroll_points(at: Point, points: i32) -> bool {
        // A wheel line is ~40 points, matching the macOS constant.
        let lines = if points >= 0 {
            (points / 40).max(1)
        } else {
            (points / 40).min(-1)
        };
        scroll(at, lines)
    }

    pub fn warp(at: Point) -> bool {
        let (x, y) = crate::win::point_to_physical(at);
        unsafe { SetCursorPos(x, y) != 0 }
    }

    pub fn scroll(at: Point, lines: i32) -> bool {
        if !warp(at) {
            return false;
        }
        // WHEEL_DELTA is 120 per notch; positive means away from the user (up).
        mouse(MOUSEEVENTF_WHEEL, (lines.saturating_mul(120)) as u32)
    }

    unsafe extern "system" fn find_process_window(hwnd: *mut c_void, data: LPARAM) -> i32 {
        let (pid, found) = unsafe { &mut *(data as *mut (i32, *mut c_void)) };
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        if unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW != 0 {
            return 1;
        }
        let mut window_pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut window_pid) };
        if window_pid as i32 == *pid {
            *found = hwnd;
            return 0;
        }
        1
    }

    pub fn activate_app(pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        let mut data: (i32, *mut c_void) = (pid, std::ptr::null_mut());
        unsafe {
            EnumWindows(Some(find_process_window), &mut data as *mut _ as LPARAM);
            let found = data.1;
            if found.is_null() {
                return false;
            }
            if IsIconic(found) != 0 {
                ShowWindow(found, SW_RESTORE);
            }
            SetForegroundWindow(found) != 0
        }
    }

    pub fn frontmost_pid() -> i32 {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return 0;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, &mut pid);
            pid as i32
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub fn click(_at: Point) -> bool {
        false
    }
    pub fn drag(_from: Point, _to: Point, _steps: u32) -> bool {
        false
    }
    pub fn key(_virtual_key: u16) -> bool {
        false
    }
    pub fn post_access() -> bool {
        true
    }
    pub fn request_post_access() -> bool {
        true
    }
    pub fn ax_trusted(_prompt: bool) -> bool {
        true
    }
    pub fn request_ax_trust() -> bool {
        true
    }
    pub fn scroll(_at: Point, _lines: i32) -> bool {
        false
    }
    pub fn scroll_points(_at: Point, _points: i32) -> bool {
        false
    }
    pub fn warp(_at: Point) -> bool {
        false
    }
    pub fn frontmost_pid() -> i32 {
        0
    }
    pub fn activate_app(_pid: i32) -> bool {
        false
    }
}

pub use platform::{
    activate_app, ax_trusted, click, drag, frontmost_pid, key, post_access, request_ax_trust,
    request_post_access, scroll, scroll_points, warp,
};

/// Virtual key codes used by the automation scripts.
#[cfg(target_os = "macos")]
pub mod keys {
    pub const RETURN: u16 = 36;
    pub const TAB: u16 = 48;
    pub const ESCAPE: u16 = 53;
    pub const SPACE: u16 = 49;

    /// ANSI virtual key codes for the letter row, used by the automation scripts to
    /// exercise tool shortcuts (`r`, `o`, `m`, …).
    pub fn from_char(character: char) -> Option<u16> {
        let code = match character.to_ascii_lowercase() {
            'a' => 0,
            's' => 1,
            'd' => 2,
            'f' => 3,
            'h' => 4,
            'g' => 5,
            'z' => 6,
            'x' => 7,
            'c' => 8,
            'v' => 9,
            'b' => 11,
            'q' => 12,
            'w' => 13,
            'e' => 14,
            'r' => 15,
            'y' => 16,
            't' => 17,
            'o' => 31,
            'u' => 32,
            'i' => 34,
            'p' => 35,
            'l' => 37,
            'j' => 38,
            'k' => 40,
            'n' => 45,
            'm' => 46,
            '1' => 18,
            '2' => 19,
            '3' => 20,
            '4' => 21,
            _ => return None,
        };
        Some(code)
    }
}

/// Windows virtual-key codes (`VK_*`) for the same scripts.
#[cfg(target_os = "windows")]
pub mod keys {
    pub const RETURN: u16 = 0x0D;
    pub const TAB: u16 = 0x09;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;

    pub fn from_char(character: char) -> Option<u16> {
        match character.to_ascii_uppercase() {
            'A'..='Z' | '0'..='9' => Some(character.to_ascii_uppercase() as u16),
            _ => None,
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod keys {
    pub const RETURN: u16 = 0;
    pub const TAB: u16 = 0;
    pub const ESCAPE: u16 = 0;
    pub const SPACE: u16 = 0;

    pub fn from_char(_character: char) -> Option<u16> {
        None
    }
}
