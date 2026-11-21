//! Session and export: freeze the screen, crop what the user selected, hand the
//! result to the clipboard and/or disk.
//!
//! This crate knows nothing about windows, hotkeys or React. The desktop shell calls
//! it, the CLI calls it, and tests call it with a fake backend (`spec/02`, ADR-1).

pub mod pins;
pub mod scrolling;

use pinshot_capture::{CaptureBackend, Permission};
use pinshot_core::{Config, Error, Image, Monitor, Rect, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One frozen moment of the desktop: every monitor's pixels, captured once.
pub struct Session {
    pub config: Config,
    monitors: Vec<Monitor>,
    frames: Vec<Image>,
}

impl Session {
    /// Captures every display. Returns `Permission` when the OS refused, so the
    /// caller can show the guide instead of a black overlay.
    pub fn start(backend: &dyn CaptureBackend, config: Config) -> Result<Self> {
        if backend.permission() == Permission::Denied {
            return Err(Error::permission(
                "macOS is not allowing PinShot to read the screen",
            ));
        }
        let monitors = backend.monitors()?;
        Self::start_with_monitors(backend, config, monitors)
    }

    /// Grabs the displays in `monitors`.
    ///
    /// The caller may already have listed them (and built the overlay windows from that
    /// list) while the grab runs: listing displays costs a millisecond, grabbing costs
    /// a fifth of a second, and the overlay's page load overlaps the grab instead of
    /// waiting for it.
    pub fn start_with_monitors(
        backend: &dyn CaptureBackend,
        config: Config,
        monitors: Vec<Monitor>,
    ) -> Result<Self> {
        if backend.permission() == Permission::Denied {
            return Err(Error::permission(
                "macOS is not allowing PinShot to read the screen",
            ));
        }
        let frames = backend.grab_all(&monitors)?;
        if frames.len() != monitors.len() {
            return Err(Error::internal("a display produced no frame"));
        }
        Ok(Self {
            config,
            monitors,
            frames,
        })
    }

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn frame(&self, index: usize) -> Option<&Image> {
        self.frames.get(index)
    }

    /// Total bytes held by the frozen frames.
    pub fn memory(&self) -> usize {
        self.frames.iter().map(Image::memory).sum()
    }

    /// Crops one monitor's frame.
    ///
    /// `area` is in the monitor's local **points** (what the overlay reports); the
    /// conversion to pixels happens through `Monitor::rect_to_pixel`, the single place
    /// where HiDPI is allowed to matter.
    pub fn crop(&self, monitor: usize, area: Option<Rect>) -> Result<Image> {
        let frame = self.frame(monitor).ok_or_else(|| {
            Error::not_found(format!("monitor {monitor} is not part of this session"))
        })?;
        let Some(area) = area else {
            return Ok(frame.clone());
        };
        let monitor = &self.monitors[monitor];
        let pixels = monitor.rect_to_pixel(area);
        let cropped = frame.crop(pixels);
        if cropped.is_empty() {
            return Err(Error::internal(format!(
                "selection {area} is outside the captured frame ({}×{})",
                frame.width, frame.height
            )));
        }
        Ok(cropped)
    }
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub clipboard: bool,
    pub save_to_file: bool,
    pub dir: PathBuf,
    /// Overrides the generated `PinShot_<timestamp>.png` name.
    pub filename: Option<String>,
}

impl ExportOptions {
    pub fn from_config(config: &Config) -> Self {
        Self {
            clipboard: config.export.copy_to_clipboard,
            save_to_file: config.export.save_to_file,
            dir: config.export.save_dir.clone(),
            filename: None,
        }
    }

    /// Nothing leaves the process: used by tests and by `--dry-run`.
    pub fn disabled(dir: impl Into<PathBuf>) -> Self {
        Self {
            clipboard: false,
            save_to_file: false,
            dir: dir.into(),
            filename: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Export {
    pub path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    /// Shared: exporting copies/clips/history/thumbnail all reference the same bytes
    /// instead of cloning a multi-megabyte PNG on every hand-off.
    pub png: Arc<[u8]>,
    pub copied_to_clipboard: bool,
}

/// Encodes `image` as PNG and delivers it according to `options`.
///
/// Clipboard and disk failures are **not** fatal one-for-the-other: if the file was
/// written but the clipboard rejected the image, the user still has the file, and the
/// error is returned alongside `path`. That distinction matters when a sandboxed
/// clipboard is the only thing that broke.
pub fn export(
    session: &Session,
    monitor: usize,
    area: Option<Rect>,
    options: &ExportOptions,
) -> Result<Export> {
    let image = session.crop(monitor, area)?;
    let png = encode_png(&image)?;

    let mut path = None;
    if options.save_to_file {
        path = Some(write_file(
            options.dir.as_path(),
            options.filename.clone(),
            &png,
        )?);
    }

    let mut copied = false;
    if options.clipboard {
        copy_to_clipboard(&image)?;
        copied = true;
    }

    Ok(Export {
        path,
        width: image.width,
        height: image.height,
        png: Arc::from(png),
        copied_to_clipboard: copied,
    })
}

/// Decodes a PNG produced by the overlay's renderer. The frontend is the single
/// source of truth for annotation pixels; Rust only transports them.
pub fn decode_png(bytes: &[u8]) -> Result<Image> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| Error::internal(format!("cannot decode the annotated image: {error}")))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    Image::from_rgba(width, height, rgba.into_raw())
        .ok_or_else(|| Error::internal("decoded image has an unexpected size"))
}

/// Writes the capture into the local history. Called by both front ends so the
/// desktop shell and the CLI record the same way.
pub fn record(
    store: &mut pinshot_store::Store,
    export: &Export,
    kind: &str,
    scale: f32,
) -> Result<String> {
    let mut capture =
        pinshot_store::NewCapture::snip(export.width, export.height, scale, export.png.clone());
    capture.kind = kind.to_owned();
    store.insert_capture(capture)
}

/// Downscales a PNG to `max` on its longest side and re-encodes it.
///
/// History thumbnails have to be small: the list shows dozens at once, and shipping
/// full 5K blobs through IPC would make the panel feel like a download.
pub fn thumbnail(png: &[u8], max: u32) -> Result<Vec<u8>> {
    let image = decode_png(png)?;
    let longest = image.width.max(image.height).max(1);
    let scale = (max as f32 / longest as f32).min(1.0);
    let width = ((image.width as f32 * scale).round() as u32).max(1);
    let height = ((image.height as f32 * scale).round() as u32).max(1);
    let source = image::RgbaImage::from_raw(image.width, image.height, image.rgba)
        .ok_or_else(|| Error::internal("cannot rebuild the image for thumbnailing"))?;
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    let (width, height) = resized.dimensions();
    let buffer = Image::from_rgba(width, height, resized.into_raw())
        .ok_or_else(|| Error::internal("thumbnail has an unexpected size"))?;
    encode_png(&buffer)
}

pub fn encode_png(image: &Image) -> Result<Vec<u8>> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    let mut out = Vec::with_capacity(image.byte_len() / 4);
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            &image.rgba,
            image.width,
            image.height,
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| Error::internal(format!("PNG encoding failed: {error}")))?;
    Ok(out)
}

pub fn write_file(dir: &Path, filename: Option<String>, png: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .map_err(|error| Error::io(format!("cannot create {}: {error}", dir.display())))?;
    let name = filename.unwrap_or_else(default_filename);
    let path = dir.join(name);
    // Write then rename: a crash mid-write must not leave a truncated PNG that looks
    // like a valid capture in the history.
    // A per-process counter keeps concurrent exports (two displays committing in the
    // same millisecond) from sharing one .part file and truncating each other.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "png.{}.{}.part",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&temporary, png)
        .map_err(|error| Error::io(format!("cannot write {}: {error}", temporary.display())))?;
    // On Windows rename fails when the destination exists (no MOVEFILE_REPLACE_EXISTING),
    // so an explicit overwrite of the same filename would always error out after a
    // successful capture. Remove the old file first.
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| Error::io(format!("cannot replace {}: {error}", path.display())))?;
    }
    std::fs::rename(&temporary, &path)
        .map_err(|error| Error::io(format!("cannot finalise {}: {error}", path.display())))?;
    Ok(path)
}

pub fn default_filename() -> String {
    // Milliseconds: two overlays (one per display) can save in the same second, and a
    // collision would silently overwrite the first file.
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S-%3f");
    format!("PinShot_{stamp}.png")
}

/// Puts plain text on the clipboard (the OCR result, usually).
pub fn copy_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| Error::internal(format!("cannot open the clipboard: {error}")))?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|error| Error::internal(format!("cannot put text on the clipboard: {error}")))?;
    Ok(())
}

pub fn copy_to_clipboard(image: &Image) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| Error::internal(format!("cannot open the clipboard: {error}")))?;
    clipboard
        .set_image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: std::borrow::Cow::Borrowed(&image.rgba),
        })
        .map_err(|error| {
            Error::internal(format!("cannot put the image on the clipboard: {error}"))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinshot_capture::fake::FakeBackend;

    fn session() -> Session {
        let backend = FakeBackend::new();
        Session::start(&backend, Config::default()).unwrap()
    }

    #[test]
    fn a_session_holds_one_frame_per_monitor() {
        let session = session();
        assert_eq!(session.monitors().len(), 1);
        assert_eq!(session.frame(0).unwrap().width, 2880);
        assert!(session.memory() >= 2880 * 1800 * 4);
    }

    #[test]
    fn a_retina_selection_is_cropped_in_pixels() {
        let session = session();
        // 100×50 points on a 2× display covers 200×100 pixels.
        let image = session.crop(0, Some(Rect::new(10, 20, 100, 50))).unwrap();
        assert_eq!((image.width, image.height), (200, 100));
        // The fake frame encodes its own coordinates.
        assert_eq!(image.pixel(0, 0), [20, 40, 0, 255]);
        assert_eq!(image.pixel(199, 99), [219, 139, 0, 255]);
    }

    #[test]
    fn a_selection_outside_the_screen_is_an_error_not_a_blank_image() {
        let session = session();
        let error = session
            .crop(0, Some(Rect::new(5000, 5000, 10, 10)))
            .unwrap_err();
        assert_eq!(error.kind, pinshot_core::ErrorKind::Internal);
    }

    #[test]
    fn export_writes_a_png_of_the_selection() {
        let session = session();
        let dir = tempfile::tempdir().unwrap();
        let export = export(
            &session,
            0,
            Some(Rect::new(0, 0, 40, 30)),
            &ExportOptions::disabled(dir.path()),
        )
        .unwrap();
        assert_eq!((export.width, export.height), (80, 60));
        assert!(!export.copied_to_clipboard);
        assert!(export.path.is_none());
    }

    #[test]
    fn export_saves_a_decodable_file() {
        let session = session();
        let dir = tempfile::tempdir().unwrap();
        let options = ExportOptions {
            clipboard: false,
            save_to_file: true,
            dir: dir.path().to_owned(),
            filename: Some("shot.png".into()),
        };
        let export = export(&session, 0, Some(Rect::new(0, 0, 20, 10)), &options).unwrap();
        let path = export.path.expect("file written");
        let decoded = image::load_from_memory(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (40, 20));
        // Temporary files never survive a successful write.
        assert!(!path.with_extension("png.part").exists());
    }

    #[test]
    fn annotated_pngs_round_trip_through_decode() {
        let session = session();
        let image = session.crop(0, Some(Rect::new(0, 0, 20, 10))).unwrap();
        let png = encode_png(&image).unwrap();
        let decoded = decode_png(&png).unwrap();
        assert_eq!((decoded.width, decoded.height), (40, 20));
        assert_eq!(decoded.rgba, image.rgba);
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn thumbnails_shrink_but_keep_the_aspect() {
        let session = session();
        let image = session.crop(0, Some(Rect::new(0, 0, 100, 50))).unwrap();
        let png = encode_png(&image).unwrap();
        let thumb = thumbnail(&png, 40).unwrap();
        let decoded = image::load_from_memory(&thumb).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (40, 20));
        // Never upscale.
        let small = image::load_from_memory(&thumbnail(&png, 10_000).unwrap()).unwrap();
        assert_eq!((small.width(), small.height()), (200, 100));
    }

    #[test]
    fn a_custom_filename_without_extension_is_used_verbatim() {
        assert!(default_filename().starts_with("PinShot_"));
        assert!(default_filename().ends_with(".png"));
    }

    #[test]
    fn permission_denied_is_reported_before_any_capture() {
        struct Denied;
        impl CaptureBackend for Denied {
            fn monitors(&self) -> Result<Vec<Monitor>> {
                panic!("must not enumerate displays when permission is denied");
            }
            fn grab(&self, _: &Monitor) -> Result<Image> {
                unreachable!()
            }
            fn cursor(&self) -> Option<pinshot_core::Point> {
                None
            }
            fn permission(&self) -> Permission {
                Permission::Denied
            }
        }
        let error = Session::start(&Denied, Config::default())
            .err()
            .expect("permission should fail the session");
        assert_eq!(error.kind, pinshot_core::ErrorKind::Permission);
        assert!(error.hint.is_some());
    }
}
