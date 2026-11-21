//! Accessibility-based scrolling — no wheel events, no pointer movement.
//!
//! This is what makes "drag the selection to the bottom and the page keeps going"
//! possible without touching the user's input: every scrollable view exposes its
//! scrollbar through the Accessibility API, so the value can be moved directly.
//!
//! Requires the Accessibility grant (`events::ax_trusted`).

use pinshot_core::{Error, Point, Result};

/// How far one step scrolls, as a fraction of the scrollable height.
pub const DEFAULT_FRACTION: f32 = 0.6;

/// Scrolls the view under `at` by `fraction` (0.0–1.0) of its scrollable height.
///
/// Returns the scrollbar position after the move, which lets the caller detect
/// "nothing moved" (already at the end) instead of guessing.
#[cfg(target_os = "macos")]
pub fn scroll_at(at: Point, fraction: f32) -> Result<f32> {
    imp::scroll_at(at, fraction)
}

/// Scrolls the main scrollable view of an application, found by walking its
/// accessibility tree. Independent of where the pointer is — which is what the capture
/// loop needs, since the overlay hides the pointer's context.
#[cfg(target_os = "macos")]
pub fn scroll_app(pid: i32, fraction: f32) -> Result<f32> {
    imp::scroll_app(pid, fraction)
}

/// Current scrollbar position of an application (0 = top, 1 = bottom).
///
/// Reading it before and after a scroll gives the *expected* pixel displacement, which
/// is what keeps the stitcher from accepting a false match on repetitive pages.
#[cfg(target_os = "macos")]
pub fn scroll_position(pid: i32) -> Result<f64> {
    imp::scroll_position(pid)
}

#[cfg(not(target_os = "macos"))]
pub fn scroll_position(_pid: i32) -> Result<f64> {
    Err(Error::unsupported("AX scrolling is macOS-only"))
}

#[cfg(not(target_os = "macos"))]
pub fn scroll_app(_pid: i32, _fraction: f32) -> Result<f32> {
    Err(Error::unsupported("AX scrolling is macOS-only"))
}

#[cfg(not(target_os = "macos"))]
pub fn scroll_at(_at: Point, _fraction: f32) -> Result<f32> {
    Err(Error::unsupported("AX scrolling is macOS-only"))
}

#[cfg(target_os = "macos")]
#[allow(non_camel_case_types)]
mod imp {
    use super::*;
    use objc2_core_foundation::CFString;
    use std::ffi::c_void;

    /// `AXUIElementRef`, `CFTypeRef` and friends are opaque here: the calls below are
    /// declared directly rather than through the generated bindings, whose generic
    /// `CFRetained<T>`/`Retained<T>` split does not match these C signatures.
    type AXRef = *mut c_void;

    const AX_ERROR_SUCCESS: i32 = 0;
    const MAX_DEPTH: usize = 32;

    extern "C" {
        fn AXUIElementCreateSystemWide() -> AXRef;
        fn AXUIElementCopyElementAtPosition(
            application: AXRef,
            x: f32,
            y: f32,
            element: *mut AXRef,
        ) -> i32;
        fn AXUIElementCopyAttributeValue(
            element: AXRef,
            attribute: *const c_void,
            value: *mut AXRef,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: AXRef,
            attribute: *const c_void,
            value: *const c_void,
        ) -> i32;
        fn CFRelease(cf: *const c_void);
        fn CFRetain(cf: *const c_void) -> *const c_void;
    }

    fn cf_string(name: &str) -> *const c_void {
        let string = CFString::from_str(name);
        let raw = objc2_core_foundation::CFRetained::into_raw(string);
        raw.as_ptr() as *const c_void
    }

    fn attribute(element: AXRef, name: &str) -> Option<AXRef> {
        let key = cf_string(name);
        let mut value: AXRef = std::ptr::null_mut();
        let status = unsafe { AXUIElementCopyAttributeValue(element, key, &mut value) };
        unsafe { CFRelease(key) };
        if status != AX_ERROR_SUCCESS || value.is_null() {
            None
        } else {
            Some(value)
        }
    }

    pub fn scroll_at(at: Point, fraction: f32) -> Result<f32> {
        if !crate::events::ax_trusted(false) {
            return Err(Error::permission(
                "Accessibility access is required to scroll without wheel events",
            ));
        }

        let system = unsafe { AXUIElementCreateSystemWide() };
        if system.is_null() {
            return Err(Error::permission(
                "this process may not query the accessibility tree",
            ));
        }
        let mut element: AXRef = std::ptr::null_mut();
        let status = unsafe {
            AXUIElementCopyElementAtPosition(system, at.x as f32, at.y as f32, &mut element)
        };
        unsafe { CFRelease(system) };
        if status != AX_ERROR_SUCCESS || element.is_null() {
            return Err(Error::not_found(format!(
                "no accessibility element at {at:?} (AXError {status})"
            )));
        }

        if std::env::var_os("PINSHOT_TRACE_AX").is_some() {
            let role = attribute(element, "AXRole")
                .map(|value| unsafe {
                    let string = &*(value as *const objc2_foundation::NSString);
                    let utf8: *const i8 = objc2::msg_send![string, UTF8String];
                    if utf8.is_null() {
                        "?".to_owned()
                    } else {
                        std::ffi::CStr::from_ptr(utf8)
                            .to_string_lossy()
                            .into_owned()
                    }
                })
                .unwrap_or_default();
            eprintln!("ax: element at {at:?} role={role:?}");
        }
        // Walk up until something owns a vertical scrollbar.
        let mut current = element;
        for _ in 0..MAX_DEPTH {
            if let Some(bar) = attribute(current, "AXVerticalScrollBar") {
                let moved = set_scrollbar(bar, fraction);
                unsafe { CFRelease(bar) };
                unsafe { CFRelease(current) };
                return moved;
            }
            match attribute(current, "AXParent") {
                Some(parent) => {
                    unsafe { CFRelease(current) };
                    current = parent;
                }
                None => {
                    unsafe { CFRelease(current) };
                    break;
                }
            }
        }
        Err(Error::not_found("no scrollable view under the pointer"))
    }

    fn set_scrollbar(bar: AXRef, fraction: f32) -> Result<f32> {
        let current = read_f64(bar).unwrap_or(0.0);
        let target = (current + fraction as f64).clamp(0.0, 1.0) as f32;

        // Scrollbars take a CFNumber; NSNumber is toll-free bridged.
        let number = objc2_foundation::NSNumber::new_f64(target as f64);
        let value = &*number as *const objc2_foundation::NSNumber as *const c_void;
        let key = cf_string("AXValue");
        let status = unsafe { AXUIElementSetAttributeValue(bar, key, value) };
        unsafe { CFRelease(key) };
        if status != AX_ERROR_SUCCESS {
            return Err(Error::internal(format!(
                "cannot move the scrollbar (AXError {status})"
            )));
        }
        Ok(target)
    }

    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXRef;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    }

    /// Breadth-first search of the app's window tree for a vertical scrollbar.
    pub fn scroll_app(pid: i32, fraction: f32) -> Result<f32> {
        let (app, bar) = find_scrollbar(pid)?;
        let moved = set_scrollbar(bar, fraction);
        unsafe { CFRelease(bar) };
        unsafe { CFRelease(app) };
        moved
    }

    /// Current scrollbar position (0 = top), without scrolling anything.
    pub fn scroll_position(pid: i32) -> Result<f64> {
        let (app, bar) = find_scrollbar(pid)?;
        let position = read_f64(bar);
        unsafe { CFRelease(bar) };
        unsafe { CFRelease(app) };
        position.ok_or_else(|| Error::not_found("the scrollbar has no position"))
    }

    /// The app element and its first vertical scrollbar, both owned by the caller.
    fn find_scrollbar(pid: i32) -> Result<(AXRef, AXRef)> {
        if !crate::events::ax_trusted(false) {
            return Err(Error::permission(
                "Accessibility access is required to scroll without wheel events",
            ));
        }
        if pid <= 0 {
            return Err(Error::not_found("no target application"));
        }
        let app = unsafe { AXUIElementCreateApplication(pid) };
        if app.is_null() {
            return Err(Error::permission("cannot inspect that application"));
        }

        let mut queue: Vec<AXRef> = Vec::new();
        if let Some(window) = attribute(app, "AXFocusedWindow") {
            queue.push(window);
        }
        if let Some(windows) = attribute(app, "AXWindows") {
            let count = unsafe { CFArrayGetCount(windows) };
            for index in 0..count.min(8) {
                let child = unsafe { CFArrayGetValueAtIndex(windows, index) } as AXRef;
                if !child.is_null() {
                    // The array owns its elements; retain the ones we queue.
                    queue.push(unsafe { CFRetain(child) } as AXRef);
                }
            }
            unsafe { CFRelease(windows) };
        }

        let mut visited = 0usize;
        while let Some(node) = queue.pop() {
            visited += 1;
            if visited > 600 {
                unsafe { CFRelease(node) };
                // Everything still queued owns a CFRetain from above; releasing only the
                // current node leaked the whole queue on every early exit.
                for pending in queue.drain(..) {
                    unsafe { CFRelease(pending) };
                }
                break;
            }
            if let Some(bar) = attribute(node, "AXVerticalScrollBar") {
                unsafe { CFRelease(node) };
                for pending in queue.drain(..) {
                    unsafe { CFRelease(pending) };
                }
                return Ok((app, bar));
            }
            if let Some(children) = attribute(node, "AXChildren") {
                let count = unsafe { CFArrayGetCount(children) };
                for index in 0..count.min(64) {
                    let child = unsafe { CFArrayGetValueAtIndex(children, index) } as AXRef;
                    if !child.is_null() {
                        queue.push(unsafe { CFRetain(child) } as AXRef);
                    }
                }
                unsafe { CFRelease(children) };
            }
            unsafe { CFRelease(node) };
        }
        unsafe { CFRelease(app) };
        Err(Error::not_found(
            "this application exposes no scrollable view",
        ))
    }

    fn read_f64(element: AXRef) -> Option<f64> {
        let value = attribute(element, "AXValue")?;
        let number = unsafe { &*(value as *const objc2_foundation::NSNumber) };
        let result: f64 = unsafe { objc2::msg_send![number, doubleValue] };
        unsafe { CFRelease(value) };
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_a_sane_fraction() {
        // Kept as a runtime check on purpose: it must stay a fraction, not a pixel count.
        assert!((0.1..=0.9).contains(&DEFAULT_FRACTION));
    }

    #[test]
    fn scrolling_is_safe_without_the_grant() {
        // Whatever the machine state, the call must return an error, never panic.
        if let Err(error) = scroll_at(Point::new(10, 10), DEFAULT_FRACTION) {
            assert!(!error.message.is_empty());
        }
    }
}
