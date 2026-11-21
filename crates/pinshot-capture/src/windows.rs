//! Window enumeration and window-level capture (macOS and Windows).
//!
//! `spec/03-capture.md` §3.1: capturing a *window* rather than a display is what makes
//! scroll capture work when other windows overlap the page, and it is also how "capture
//! the window under the cursor" finds its target.
//!
//! Both calls are read-only: they never move the pointer or synthesise input.

use pinshot_core::Rect;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use pinshot_core::{Error, Image, Result};

/// One on-screen window, as reported by the window server.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub id: u32,
    pub pid: i32,
    pub app: String,
    pub title: String,
    /// Frame in points, top-left origin.
    pub bounds: Rect,
    /// 0 is the normal window layer; menu bars and floating panels sit above.
    pub layer: i32,
}

impl WindowInfo {
    /// A window worth capturing: has a size, an identity, and is not a menu or overlay.
    pub fn is_capturable(&self) -> bool {
        self.bounds.w >= 32
            && self.bounds.h >= 32
            && self.layer == 0
            && (!self.app.is_empty() || !self.title.is_empty())
    }
}

#[cfg(target_os = "macos")]
pub use imp::{grab, grab_below, list};

#[cfg(target_os = "windows")]
pub use win_imp::{grab, grab_below, list};

/// Captures what is on screen **below** one window, in global points.
///
/// This is what makes a live scroll capture possible: the overlay stays visible (and
/// grows) while the page underneath keeps scrolling, and the capture never contains
/// the overlay itself. `area` uses the virtual-desktop point space, like `Monitor`.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn grab_below(_window_id: u32, _area: Rect) -> Result<Image> {
    Err(Error::unsupported("window-relative capture is macOS-only"))
}

/// The frontmost capturable window whose frame contains `point`.
///
/// This is the window the user is pointing at, which is what a wheel event will reach
/// once its owner is activated — not "whatever app happened to be frontmost".
pub fn under_point(point: pinshot_core::Point) -> pinshot_core::Result<Option<WindowInfo>> {
    let windows = list()?;
    Ok(windows
        .into_iter()
        .filter(WindowInfo::is_capturable)
        .find(|window| window.bounds.contains(point)))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn list() -> Result<Vec<WindowInfo>> {
    Err(Error::unsupported(
        "window enumeration is macOS-only for now",
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn grab(_window: &WindowInfo) -> Result<Image> {
    Err(Error::unsupported("window capture is macOS-only for now"))
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // CGWindowListCreateImage: the synchronous window grab on macOS 14
mod imp {
    use super::WindowInfo;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{
        kCGColorSpaceSRGB, CGBitmapContextCreate, CGColorSpace, CGImage, CGWindowID,
        CGWindowImageOption, CGWindowListCopyWindowInfo, CGWindowListCreateImage,
        CGWindowListOption,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use pinshot_core::{Error, Image, Rect, Result};
    use std::ffi::c_void;

    const RGBA8: u32 = 1 | (4 << 12);

    /// Every on-screen window, frontmost first.
    pub fn list() -> Result<Vec<WindowInfo>> {
        let options =
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
        let list: CFRetained<CFArray> = CGWindowListCopyWindowInfo(options, 0)
            .ok_or_else(|| Error::permission("the window server did not return a window list"))?;

        // CFArray/NSDictionary are toll-free bridged; the Foundation accessors are far
        // less error-prone than hand-rolled CFDictionary lookups.
        let array: &NSArray<NSDictionary<NSString, AnyObject>> =
            unsafe { &*(&*list as *const CFArray as *const _) };

        let mut windows = Vec::new();
        for entry in array.iter() {
            let Some(number) = entry.objectForKey(&NSString::from_str("kCGWindowNumber")) else {
                continue;
            };
            let id: u32 = unsafe { objc2::msg_send![&*number, unsignedIntValue] };
            let pid: i32 = entry
                .objectForKey(&NSString::from_str("kCGWindowOwnerPID"))
                .map(|value| unsafe { objc2::msg_send![&*value, intValue] })
                .unwrap_or(0);
            let layer: i32 = entry
                .objectForKey(&NSString::from_str("kCGWindowLayer"))
                .map(|value| unsafe { objc2::msg_send![&*value, intValue] })
                .unwrap_or(0);
            let app = string_value(&entry, "kCGWindowOwnerName");
            let title = string_value(&entry, "kCGWindowName");
            let bounds = entry
                .objectForKey(&NSString::from_str("kCGWindowBounds"))
                .and_then(|value| bounds_of(&value))
                .unwrap_or(Rect::new(0, 0, 0, 0));
            if bounds.w == 0 || bounds.h == 0 {
                continue;
            }
            windows.push(WindowInfo {
                id,
                pid,
                app,
                title,
                bounds,
                layer,
            });
        }
        Ok(windows)
    }

    /// Captures what the screen shows below `window_id`, in global points.
    pub fn grab_below(window_id: u32, area: Rect) -> Result<Image> {
        // `OnScreenBelowWindow` composites every window below the reference one, which
        // is exactly "the desktop without our overlay".
        let image = CGWindowListCreateImage(
            CGRect {
                origin: CGPoint {
                    x: area.x as f64,
                    y: area.y as f64,
                },
                size: CGSize {
                    width: area.w as f64,
                    height: area.h as f64,
                },
            },
            CGWindowListOption::OptionOnScreenBelowWindow,
            window_id as CGWindowID,
            CGWindowImageOption::Default,
        )
        .ok_or_else(|| Error::permission("cannot capture the screen below the overlay"))?;

        let width = CGImage::width(Some(&image));
        let height = CGImage::height(Some(&image));
        if width == 0 || height == 0 {
            return Err(Error::internal("the screen below the overlay is empty"));
        }
        let mut pixels = vec![0u8; width * height * 4];
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .ok_or_else(|| Error::internal("cannot create the sRGB colour space"))?;
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr() as *mut c_void,
                width,
                height,
                8,
                width * 4,
                Some(&space),
                RGBA8,
            )
        }
        .ok_or_else(|| Error::internal("cannot create a bitmap context"))?;
        objc2_core_graphics::CGContext::draw_image(
            Some(&context),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: width as f64,
                    height: height as f64,
                },
            },
            Some(&image),
        );
        Image::from_rgba(width as u32, height as u32, pixels)
            .ok_or_else(|| Error::internal("below-window capture produced an unexpected buffer"))
    }

    /// Captures one window, including the parts other windows cover.
    pub fn grab(window: &WindowInfo) -> Result<Image> {
        // An empty screen rect + `OptionIncludingWindow` is the documented way to grab
        // exactly one window, regardless of what is on top of it.
        let image = CGWindowListCreateImage(
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: 0.0,
                    height: 0.0,
                },
            },
            CGWindowListOption::OptionIncludingWindow,
            window.id as CGWindowID,
            CGWindowImageOption::BoundsIgnoreFraming,
        )
        .ok_or_else(|| {
            Error::permission(format!(
                "cannot capture window {} ({})",
                window.id, window.title
            ))
        })?;

        let width = CGImage::width(Some(&image));
        let height = CGImage::height(Some(&image));
        if width == 0 || height == 0 {
            return Err(Error::internal("the window reported an empty frame"));
        }
        let mut pixels = vec![0u8; width * height * 4];
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
            .ok_or_else(|| Error::internal("cannot create the sRGB colour space"))?;
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr() as *mut c_void,
                width,
                height,
                8,
                width * 4,
                Some(&space),
                RGBA8,
            )
        }
        .ok_or_else(|| Error::internal("cannot create a bitmap context"))?;
        objc2_core_graphics::CGContext::draw_image(
            Some(&context),
            CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: width as f64,
                    height: height as f64,
                },
            },
            Some(&image),
        );
        Image::from_rgba(width as u32, height as u32, pixels)
            .ok_or_else(|| Error::internal("window capture produced an unexpected buffer"))
    }

    fn string_value(entry: &NSDictionary<NSString, AnyObject>, key: &str) -> String {
        let Some(value) = entry.objectForKey(&NSString::from_str(key)) else {
            return String::new();
        };
        let pointer: *const i8 = unsafe { objc2::msg_send![&*value, UTF8String] };
        if pointer.is_null() {
            return String::new();
        }
        unsafe { std::ffi::CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    }

    /// `kCGWindowBounds` is a CFDictionary with X/Y/Width/Height; CoreGraphics ships the
    /// canonical reader. Hand-rolling `objectForKey:` on the bridged dictionary read
    /// freed memory and produced plausible-looking garbage.
    fn bounds_of(value: &AnyObject) -> Option<Rect> {
        let dictionary = unsafe { &*(value as *const AnyObject as *const CFDictionary) };
        let mut rect = CGRect::default();
        let ok = unsafe {
            objc2_core_graphics::CGRectMakeWithDictionaryRepresentation(Some(dictionary), &mut rect)
        };
        if !ok {
            return None;
        }
        Some(Rect::new(
            rect.origin.x as i32,
            rect.origin.y as i32,
            rect.size.width.max(0.0) as u32,
            rect.size.height.max(0.0) as u32,
        ))
    }

    /// `Retained` is used by the typed accessors above.
    #[allow(dead_code)]
    fn _retained_marker(_: Retained<AnyObject>) {}
}

#[cfg(target_os = "windows")]
mod win_imp {
    use super::WindowInfo;
    use pinshot_core::{Image, Rect, Result};

    /// Every visible top-level window, in z-order (frontmost first).
    pub fn list() -> Result<Vec<WindowInfo>> {
        Ok(crate::win::list_windows())
    }

    /// Grabs a window without any of our own overlay in the frame.
    ///
    /// The caller hides the overlay before a scroll capture starts, so reading the
    /// screen region is enough and works for GPU-composited apps that answer
    /// `PrintWindow` with a black frame.
    pub fn grab(window: &WindowInfo) -> Result<Image> {
        crate::win::grab_window_rect(window.bounds)
    }

    /// Captures what is on screen below a window. On Windows the overlay is hidden for
    /// the whole scroll capture, so this is a plain screen grab of `area`.
    pub fn grab_below(_window_id: u32, area: Rect) -> Result<Image> {
        crate::win::grab_window_rect(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_windows_are_capturable() {
        let window = |layer: i32, w: u32, app: &str| WindowInfo {
            id: 1,
            pid: 1,
            app: app.into(),
            title: String::new(),
            bounds: Rect::new(0, 0, w, 400),
            layer,
        };
        assert!(window(0, 800, "Safari").is_capturable());
        assert!(
            !window(25, 800, "Window Server").is_capturable(),
            "menu bar layer"
        );
        assert!(!window(0, 20, "tiny").is_capturable(), "too small");
        assert!(!window(0, 800, "").is_capturable(), "no identity");
    }
}
