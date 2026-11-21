//! Screen capture backends.
//!
//! Everything above this crate talks to one trait, so the app, the CLI and the tests
//! never care whether the pixels came from CoreGraphics or a synthetic frame.
//!
//! Current status: macOS uses CoreGraphics, Windows uses GDI; Linux still returns
//! `Unsupported`. That is deliberate — a half-working backend is worse than an
//! honest error.

pub mod ax;
pub mod events;
pub mod windows;

#[cfg(target_os = "windows")]
pub mod win;

use pinshot_core::{Error, Image, Monitor, Point, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
    /// The platform has no such concept (Windows, X11).
    NotRequired,
}

pub trait CaptureBackend: Send + Sync {
    /// Which displays exist right now. Cheap enough to call on every capture.
    fn monitors(&self) -> Result<Vec<Monitor>>;

    /// Grabs one display's current contents as RGBA in device pixels.
    fn grab(&self, monitor: &Monitor) -> Result<Image>;

    /// Grabs every display. Backends override this to run the displays in parallel:
    /// a 5K + 1080p pair costs 200 ms serially and ~110 ms concurrently, and the
    /// overlay cannot appear before the last frame arrives.
    fn grab_all(&self, monitors: &[Monitor]) -> Result<Vec<Image>> {
        monitors.iter().map(|monitor| self.grab(monitor)).collect()
    }

    /// Current pointer position in global points, when the platform exposes it.
    fn cursor(&self) -> Option<Point>;

    /// Whether the OS will actually give us pixels.
    fn permission(&self) -> Permission;
}

/// Whether this process may read the screen right now.
pub fn screen_access() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_core_graphics::CGPreflightScreenCaptureAccess()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Asks macOS for the screen-recording permission.
///
/// `CGPreflightScreenCaptureAccess` only reports; the app is added to
/// 系统设置 → 屏幕录制 only after `CGRequestScreenCaptureAccess` has been called.
/// Without this the user sees an empty list and no way to grant it.
pub fn request_screen_access() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_core_graphics::CGRequestScreenCaptureAccess()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// The platform backend for the host OS.
pub fn system() -> Box<dyn CaptureBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacBackend)
    }
    #[cfg(target_os = "windows")]
    {
        win::ensure_dpi_awareness();
        Box::new(win::WindowsBackend)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Box::new(UnsupportedBackend)
    }
}

/// Marker for "no implementation yet", so `doctor` can say exactly what is missing.
pub struct UnsupportedBackend;

impl CaptureBackend for UnsupportedBackend {
    fn monitors(&self) -> Result<Vec<Monitor>> {
        Err(Error::unsupported(format!(
            "screen capture is not implemented on {}",
            std::env::consts::OS
        ))
        .with_hint("macOS and Windows are implemented; Linux is still missing"))
    }

    fn grab(&self, _monitor: &Monitor) -> Result<Image> {
        Err(Error::unsupported(
            "screen capture is not implemented on this platform",
        ))
    }

    fn cursor(&self) -> Option<Point> {
        None
    }

    fn permission(&self) -> Permission {
        Permission::NotRequired
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // CGDisplayCreateImage: synchronous grab, replaced by SCK before 1.0
pub mod macos {
    use super::{CaptureBackend, Permission};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{
        kCGColorSpaceSRGB, CGBitmapContextCreate, CGColorSpace, CGContext, CGDirectDisplayID,
        CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayCreateImage, CGDisplayIsBuiltin,
        CGDisplayIsMain, CGDisplayMode, CGDisplayPixelsHigh, CGDisplayPixelsWide, CGError, CGEvent,
        CGGetActiveDisplayList, CGImage, CGPreflightScreenCaptureAccess,
    };
    use pinshot_core::{Error, Image, Monitor, Point, Rect, Result};
    use std::ffi::c_void;

    /// `kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big`: memory order is
    /// R, G, B, A. Screen captures are opaque, so premultiplied equals straight.
    const RGBA8: u32 = 1 | (4 << 12);
    /// More displays than this on one machine is not a real case; the API needs a
    /// fixed-size buffer.
    const MAX_DISPLAYS: u32 = 16;

    #[derive(Default)]
    pub struct MacBackend;

    impl MacBackend {
        fn permission_granted(&self) -> bool {
            // Reports the stored TCC decision; it never prompts.
            CGPreflightScreenCaptureAccess()
        }

        fn display_ids(&self) -> Result<Vec<CGDirectDisplayID>> {
            let mut ids = [0 as CGDirectDisplayID; MAX_DISPLAYS as usize];
            let mut count: u32 = 0;
            let error =
                unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, ids.as_mut_ptr(), &mut count) };
            if error != CGError::Success {
                return Err(Error::internal(format!(
                    "cannot enumerate displays: {error:?}"
                )));
            }
            Ok(ids[..count as usize].to_vec())
        }

        fn describe(&self, id: CGDirectDisplayID, index: usize) -> Option<Monitor> {
            let bounds = CGDisplayBounds(id);
            let (width, height) = (bounds.size.width as u32, bounds.size.height as u32);
            if width == 0 || height == 0 {
                // A mirrored or sleeping display reports an empty rect.
                return None;
            }
            // CGDisplayPixelsWide reports *points* on Retina displays, so the real
            // pixel size has to come from the display mode. Getting this wrong is
            // exactly the HiDPI bug the whole `Monitor` type exists to prevent.
            let mode = CGDisplayCopyDisplayMode(id);
            let mode_pixels = mode
                .as_ref()
                .map(|mode| {
                    (
                        CGDisplayMode::pixel_width(Some(mode)) as u32,
                        CGDisplayMode::pixel_height(Some(mode)) as u32,
                    )
                })
                .filter(|(w, h)| *w > 0 && *h > 0)
                .unwrap_or((
                    CGDisplayPixelsWide(id) as u32,
                    CGDisplayPixelsHigh(id) as u32,
                ));
            let (pixel_width, pixel_height) = mode_pixels;
            let mode_scale = pixel_width as f32 / width as f32;
            let builtin = CGDisplayIsBuiltin(id);
            Some(Monitor {
                id,
                name: if builtin {
                    "Built-in Display".to_owned()
                } else {
                    format!("Display {}", index + 1)
                },
                bounds: Rect::new(
                    bounds.origin.x as i32,
                    bounds.origin.y as i32,
                    width,
                    height,
                ),
                scale: if mode_scale > 0.0 { mode_scale } else { 1.0 },
                pixel_width,
                pixel_height,
                primary: CGDisplayIsMain(id),
            })
        }

        fn copy_pixels(&self, id: CGDirectDisplayID) -> Result<Image> {
            // Deprecated in favour of ScreenCaptureKit, but still the only synchronous
            // one-shot display grab on macOS 14 (see `spec/03-capture.md`).
            let image = CGDisplayCreateImage(id).ok_or_else(|| {
                Error::permission("macOS returned no image for this display")
                    .with_hint("确认「系统设置 → 隐私与安全性 → 屏幕录制」里已勾选 PinShot")
            })?;

            let width = CGImage::width(Some(&image));
            let height = CGImage::height(Some(&image));
            if width == 0 || height == 0 {
                return Err(Error::internal("the display reported an empty frame"));
            }

            let mut pixels = vec![0u8; width * height * 4];
            let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
                .ok_or_else(|| Error::internal("cannot create the sRGB colour space"))?;
            // Drawing into a known RGBA8 sRGB context both fixes the byte order and
            // converts from the display profile; a raw CGImage buffer would be
            // BGRA in Display P3 on a Retina panel.
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

            let rect = CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: width as f64,
                    height: height as f64,
                },
            };
            CGContext::draw_image(Some(&context), rect, Some(&image));
            drop(context);

            Image::from_rgba(width as u32, height as u32, pixels)
                .ok_or_else(|| Error::internal("capture produced an unexpected buffer size"))
        }
    }

    impl CaptureBackend for MacBackend {
        fn monitors(&self) -> Result<Vec<Monitor>> {
            let mut monitors: Vec<Monitor> = self
                .display_ids()?
                .into_iter()
                .enumerate()
                .filter_map(|(index, id)| self.describe(id, index))
                .collect();
            if monitors.is_empty() {
                return Err(Error::not_found("no active displays"));
            }
            // Primary first: the overlay opens there and most selections start there.
            monitors.sort_by_key(|monitor| !monitor.primary);
            Ok(monitors)
        }

        fn grab(&self, monitor: &Monitor) -> Result<Image> {
            if !self.permission_granted() {
                return Err(Error::permission(
                    "macOS is not allowing PinShot to read the screen",
                ));
            }
            self.copy_pixels(monitor.id)
        }

        fn grab_all(&self, monitors: &[Monitor]) -> Result<Vec<Image>> {
            // Each display is an independent CoreGraphics call; grabbing them
            // concurrently matches the serial cost of the largest one.
            let mut frames: Vec<Option<Result<Image>>> = vec![None; monitors.len()];
            std::thread::scope(|scope| {
                let handles: Vec<_> = monitors
                    .iter()
                    .map(|monitor| scope.spawn(move || self.copy_pixels(monitor.id)))
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

        fn cursor(&self) -> Option<Point> {
            let event = CGEvent::new(None)?;
            let location = CGEvent::location(Some(&event));
            Some(Point::new(location.x as i32, location.y as i32))
        }

        fn permission(&self) -> Permission {
            if self.permission_granted() {
                Permission::Granted
            } else {
                Permission::Denied
            }
        }
    }
}

/// A backend with canned pixels, used by unit tests and `pinshot snap --fake`.
pub mod fake {
    use super::{CaptureBackend, Permission};
    use pinshot_core::{Image, Monitor, Point, Rect, Result};

    pub struct FakeBackend {
        pub monitors: Vec<Monitor>,
        cursor: Point,
    }

    impl Default for FakeBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FakeBackend {
        pub fn new() -> Self {
            Self {
                monitors: vec![Monitor {
                    id: 1,
                    name: "Fake Display".into(),
                    bounds: Rect::new(0, 0, 1440, 900),
                    scale: 2.0,
                    pixel_width: 2880,
                    pixel_height: 1800,
                    primary: true,
                }],
                cursor: Point::new(720, 450),
            }
        }

        /// A frame where every pixel encodes its own coordinate, so a crop can be
        /// verified by reading pixels back instead of comparing screenshots.
        pub fn frame(&self, monitor: &Monitor) -> Image {
            let (width, height) = monitor.pixel_size();
            let mut image = Image::new(width, height);
            for y in 0..height {
                for x in 0..width {
                    let index = (y as usize * width as usize + x as usize) * 4;
                    image.rgba[index] = (x % 256) as u8;
                    image.rgba[index + 1] = (y % 256) as u8;
                    image.rgba[index + 2] = ((x / 256) as u8) << 4 | (y / 256) as u8;
                    image.rgba[index + 3] = 255;
                }
            }
            image
        }
    }

    impl CaptureBackend for FakeBackend {
        fn monitors(&self) -> Result<Vec<Monitor>> {
            Ok(self.monitors.clone())
        }

        fn grab(&self, monitor: &Monitor) -> Result<Image> {
            Ok(self.frame(monitor))
        }

        fn cursor(&self) -> Option<Point> {
            Some(self.cursor)
        }

        fn permission(&self) -> Permission {
            Permission::Granted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_backend_encodes_coordinates() {
        let backend = fake::FakeBackend::new();
        let monitors = backend.monitors().unwrap();
        let frame = backend.grab(&monitors[0]).unwrap();
        assert_eq!((frame.width, frame.height), (2880, 1800));
        assert_eq!(frame.pixel(5, 7), [5, 7, 0, 255]);
        assert_eq!(frame.pixel(300, 400), [44, 144, 17, 255]);
    }

    #[test]
    fn both_backends_satisfy_the_trait() {
        fn accepts(_: Box<dyn CaptureBackend>) {}
        accepts(Box::new(fake::FakeBackend::new()));
        accepts(system());
    }
}
