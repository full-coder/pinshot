//! The macOS pieces that have no Tauri API: activation policy, first-responder focus,
//! window level and the "accept the first click" override.
//!
//! Everything here is a thin `msg_send` shim; the rest of the app calls these names and
//! never touches AppKit directly. On other platforms they are no-ops, which is what the
//! window/scroll code needs in order to stay platform-neutral.

#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
use tauri::{AppHandle, WebviewWindow};

/// Back to a background app so the next capture does not fight the page for focus.
///
/// While 「设置」/「历史记录」 is open the app has to stay a regular app: switching to
/// accessory would take its menu bar and keyboard away mid-edit.
#[cfg(target_os = "macos")]
pub(crate) fn rest_activation_policy(app: &AppHandle) {
    if crate::aux_window_visible(app) {
        return;
    }
    use objc2::msg_send;
    unsafe {
        let ns_app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
        // 1 = accessory
        let _: bool = msg_send![ns_app, setActivationPolicy: 1isize];
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn rest_activation_policy(_app: &AppHandle) {}

/// A borderless window sits below the menu bar; the saver level puts the frozen
/// frame above it, so a selection can start at y = 0.
#[cfg(target_os = "macos")]
pub(crate) fn raise_above_menu_bar(window: &WebviewWindow) {
    use objc2::msg_send;
    if let Ok(pointer) = window.ns_window() {
        let ns_window = pointer as *mut AnyObject;
        unsafe {
            let _: () = msg_send![ns_window, setLevel: 1000isize];
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn raise_above_menu_bar(_window: &WebviewWindow) {}

/// Becomes the active app, as early as possible.
///
/// macOS only makes the overlay's window key once PinShot is active, and activation is
/// asynchronous (it took ~150 ms when it happened together with `show()`). Starting it
/// when the shortcut fires hides that latency behind the frame grab, so the window is
/// already key when it appears and the first drag is not eaten by "click to activate".
#[cfg(target_os = "macos")]
pub(crate) fn activate_app() {
    use objc2::msg_send;
    unsafe {
        let ns_app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn activate_app() {}

/// Lets the overlay take the very first click after it appears.
///
/// The app is a background (accessory) app, so showing the overlay does not activate it
/// on its own. macOS then treats the next mouse-down as "activate this app" and holds it
/// back for as long as activation takes — ~200 ms in practice, which is exactly the
/// moment the user starts dragging. Returning `true` from `acceptsFirstMouse:` hands the
/// click to the page immediately; the click still activates the app.
#[cfg(target_os = "macos")]
pub(crate) fn accept_first_mouse(window: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::Sel;
    use std::ffi::{c_char, c_void};

    unsafe extern "C-unwind" fn yes(
        _this: *mut AnyObject,
        _cmd: Sel,
        _event: *mut AnyObject,
    ) -> bool {
        true
    }

    extern "C" {
        fn class_replaceMethod(
            cls: *const AnyObject,
            name: Sel,
            imp: *const c_void,
            types: *const c_char,
        ) -> *const c_void;
    }

    let Ok(pointer) = window.ns_window() else {
        return;
    };
    let ns_window = pointer as *mut AnyObject;
    unsafe {
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        if content.is_null() {
            return;
        }
        let selector = Sel::register(c"acceptsFirstMouse:");
        let types = c"B@:@".as_ptr();
        // WebKit puts its own content view below the WKWebView, and that is the view
        // that hit-tests a click. Patch the whole tree rather than guess which class
        // ends up under the pointer.
        let mut stack = vec![content];
        while let Some(view) = stack.pop() {
            let class: *const AnyObject = msg_send![view, class];
            if !class.is_null() {
                class_replaceMethod(class, selector, yes as *const c_void, types);
            }
            let subviews: *mut AnyObject = msg_send![view, subviews];
            if subviews.is_null() {
                continue;
            }
            let count: usize = msg_send![subviews, count];
            for index in 0..count {
                let child: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
                if !child.is_null() {
                    stack.push(child);
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn accept_first_mouse(_window: &WebviewWindow) {}

/// Borderless windows do not hand keyboard focus to their webview on their own, so
/// the overlay would draw but ignore every keystroke. Making the *WKWebView* (not the
/// window's content view) the first responder is what actually routes key events into
/// the page: a container view swallows them and they never reach its children.
#[cfg(target_os = "macos")]
pub(crate) fn focus_webview(window: &WebviewWindow) {
    focus_webview_inner(window, true);
}

/// Re-asserts keyboard focus without activating the app or re-ordering the window.
///
/// The single delayed retry after `overlay_show` uses this: the window is already visible
/// and key, and calling `makeKeyAndOrderFront` again there made both full-screen
/// overlays visibly blink on every capture.
#[cfg(target_os = "macos")]
pub(crate) fn focus_webview_quiet(window: &WebviewWindow) {
    focus_webview_inner(window, false);
}

#[cfg(target_os = "macos")]
fn focus_webview_inner(window: &WebviewWindow, reorder: bool) {
    use objc2::msg_send;
    use std::ptr::null_mut;
    if let Ok(pointer) = window.ns_window() {
        let ns_window = pointer as *mut AnyObject;
        unsafe {
            let ns_app: *mut AnyObject = msg_send![objc2::class!(NSApplication), sharedApplication];
            if reorder {
                let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];
                let _: () = msg_send![ns_window, makeKeyAndOrderFront: null_mut::<AnyObject>()];
            }
            let content: *mut AnyObject = msg_send![ns_window, contentView];
            let webview = find_webview(content);
            if let Some(view) = webview {
                let _: bool = msg_send![ns_window, makeFirstResponder: view];
            } else if !content.is_null() {
                let _: bool = msg_send![ns_window, makeFirstResponder: content];
            }
            let key: bool = msg_send![ns_window, isKeyWindow];
            let active: bool = msg_send![ns_app, isActive];
            let policy: isize = msg_send![ns_app, activationPolicy];
            crate::diag(&format!(
                "focus {}: key={key} active={active} policy={policy} webview={} reorder={reorder}",
                window.label(),
                webview.is_some()
            ));
        }
    }
}

/// Depth-first search for the `WKWebView` inside the window's content view.
#[cfg(target_os = "macos")]
unsafe fn find_webview(view: *mut AnyObject) -> Option<*mut AnyObject> {
    use objc2::msg_send;
    if view.is_null() {
        return None;
    }
    let class = unsafe { (*view).class() };
    if class.name().to_string_lossy().contains("WebView") {
        return Some(view);
    }
    let subviews: *mut AnyObject = unsafe { msg_send![view, subviews] };
    if subviews.is_null() {
        return None;
    }
    let count: usize = unsafe { msg_send![subviews, count] };
    for index in 0..count {
        let child: *mut AnyObject = unsafe { msg_send![subviews, objectAtIndex: index] };
        if let Some(found) = unsafe { find_webview(child) } {
            return Some(found);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn focus_webview(_window: &WebviewWindow) {}

#[cfg(not(target_os = "macos"))]
pub(crate) fn focus_webview_quiet(_window: &WebviewWindow) {}
