//! `pinshot` — headless entry point.
//!
//! Everything the GUI does with pixels is reachable from here, which makes the
//! capture pipeline testable in a terminal and scriptable from CI.

use chrono::TimeZone;
use clap::{Parser, Subcommand};
use pinshot_app::{ExportOptions, Session};
use pinshot_capture::{system, CaptureBackend, Permission};
use pinshot_core::{Config, Error, ErrorKind, Point, Rect, Result};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "pinshot",
    version,
    about = "PinShot — screenshot, annotate, pin"
)]
struct Cli {
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Override the data directory (history lives here). Useful for tests.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List displays with their point and pixel geometry.
    Monitors,

    /// Capture the screen (or a region) now.
    Snap {
        /// Region in points: X Y WIDTH HEIGHT. Defaults to the whole display.
        #[arg(long, num_args = 4, allow_hyphen_values = true, value_names = ["X", "Y", "W", "H"])]
        area: Option<Vec<i32>>,

        /// Display index from `pinshot monitors`.
        #[arg(long, default_value_t = 0)]
        monitor: usize,

        /// Write the PNG to this exact path.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,

        /// Do not write a file (clipboard only).
        #[arg(long)]
        no_save: bool,

        /// Also put the image on the clipboard.
        #[arg(long)]
        clipboard: bool,

        /// Use a synthetic display instead of the real screen.
        #[arg(long)]
        fake: bool,

        /// Recognise the text and copy *that* to the clipboard instead of the image.
        #[arg(long)]
        ocr: bool,
    },

    /// Report permissions, displays and paths.
    Doctor,

    /// Scroll-capture the live screen and stitch it into one long image.
    ///
    /// Scrolls through the Accessibility API (no wheel events, no pointer movement) and
    /// grows the image one revealed strip at a time, which is the same path the overlay
    /// uses while the user drags to the edge.
    ScrollLive {
        /// Region in points: X Y WIDTH HEIGHT.
        #[arg(long, num_args = 4, allow_hyphen_values = true, value_names = ["X", "Y", "W", "H"])]
        area: Vec<i32>,
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 20)]
        max: u32,
        /// Target app; defaults to the window under the region.
        #[arg(long)]
        pid: Option<i32>,
        /// Scroll upward instead (the capture starts at the bottom of the page).
        #[arg(long)]
        up: bool,
        #[arg(long)]
        json: bool,
    },

    /// Stitch a directory of captured frames into one long image.
    ///
    /// The same engine the overlay uses; this command exists so a failed scroll job
    /// can be reproduced offline (`spec/04-scrolling-capture.md` §11).
    Scroll {
        /// Directory of frame images, sorted by file name.
        #[arg(long, value_name = "DIR")]
        frames: PathBuf,
        /// Output PNG (defaults to `<frames>/../scroll.png`).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        /// Hard cap on the output height.
        #[arg(long)]
        max_height: Option<u32>,
    },

    /// List on-screen windows (for window-level capture).
    Windows {
        #[arg(long)]
        json: bool,
        /// Capture every window and compare the real pixel size with the reported frame.
        #[arg(long)]
        verify: bool,
    },

    /// Scroll the view under a point through the Accessibility API.
    ///
    /// No wheel events and no pointer movement: it moves the scrollbar directly.
    AxScroll {
        x: i32,
        y: i32,
        /// Fraction of the scrollable height (default 0.5); negative scrolls up.
        #[arg(default_value_t = 0.5, allow_hyphen_values = true)]
        fraction: f32,
        /// Scroll the app's own scroll view instead of the element under the point.
        #[arg(long)]
        pid: Option<i32>,
        #[arg(long)]
        json: bool,
    },

    /// Capture a single window by id (from `pinshot windows`).
    WindowShot {
        #[arg(long)]
        id: u32,
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Recognise text in an image (on-device Vision).
    Ocr {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List (or prune) the local capture history.
    History {
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        json: bool,
        /// Apply the retention policy now instead of listing.
        #[arg(long)]
        prune: bool,
    },

    /// Drive the GUI from a script: `drag`, `click`, `key`, `scroll`.
    Simulate {
        #[command(subcommand)]
        action: SimulateAction,
    },
}

#[derive(Subcommand)]
enum SimulateAction {
    /// Drag from one point to another (global points).
    Drag {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        #[arg(long, default_value_t = 8)]
        steps: u32,
    },
    Click {
        x: i32,
        y: i32,
    },
    /// `return` / `escape` / `tab` / `space`
    Key {
        name: String,
    },
    /// Move the real pointer (needed before a wheel event can reach a window).
    Warp {
        x: i32,
        y: i32,
    },
    /// Post wheel lines at a point (positive scrolls content up).
    Scroll {
        x: i32,
        y: i32,
        lines: i32,
    },
}

fn main() -> ExitCode {
    #[cfg(target_os = "windows")]
    pinshot_capture::win::ensure_dpi_awareness();
    let cli = Cli::parse();
    if let Some(directory) = &cli.data_dir {
        std::env::set_var("PINSHOT_DATA_DIR", directory);
    }
    let result = match cli.command {
        Command::Monitors => monitors(cli.json),
        Command::Snap {
            area,
            monitor,
            out,
            no_save,
            clipboard,
            fake,
            ocr,
        } => snap(
            cli.json,
            area,
            monitor,
            out,
            SnapFlags {
                no_save,
                clipboard,
                fake,
                ocr,
            },
        ),
        Command::Doctor => doctor(cli.json),
        Command::Scroll {
            frames,
            out,
            json,
            max_height,
        } => scroll(&frames, out, json, max_height),
        Command::ScrollLive {
            area,
            out,
            max,
            pid,
            up,
            json,
        } => scroll_live(&area, out, max, pid, up, json),
        Command::Windows { json, verify } => windows(json, verify),
        Command::AxScroll {
            x,
            y,
            fraction,
            pid,
            json,
        } => ax_scroll(x, y, fraction, pid, json),
        Command::WindowShot { id, out, json } => window_shot(id, &out, json),
        Command::Ocr { file, json } => ocr(&file, json),
        Command::History { limit, json, prune } => {
            history(cli.data_dir.clone(), limit, json, prune)
        }
        Command::Simulate { action } => simulate(action),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": false,
                        "error": { "kind": error.kind.as_str(), "message": error.message, "hint": error.hint }
                    })
                );
            } else {
                eprintln!("error: {error}");
            }
            if error.kind == ErrorKind::Permission {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn backend(fake: bool) -> Box<dyn CaptureBackend> {
    if fake {
        Box::new(pinshot_capture::fake::FakeBackend::new())
    } else {
        system()
    }
}

fn monitors(json: bool) -> Result<()> {
    let backend = system();
    let displays = backend.monitors()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&displays).unwrap());
        return Ok(());
    }
    println!(
        "{:<3} {:<24} {:<8} {:<20} {:<16} PRIMARY",
        "#", "NAME", "SCALE", "POINTS", "PIXELS"
    );
    for (index, monitor) in displays.iter().enumerate() {
        println!(
            "{:<3} {:<24} {:<8} {:<20} {:<16} {}",
            index,
            truncate(&monitor.name, 24),
            format!("{}x", monitor.scale),
            format!(
                "{}×{} @ {},{}",
                monitor.bounds.w, monitor.bounds.h, monitor.bounds.x, monitor.bounds.y
            ),
            format!("{}×{}", monitor.pixel_width, monitor.pixel_height),
            if monitor.primary { "yes" } else { "" }
        );
    }
    Ok(())
}

/// The switches `snap` takes beyond its area and destination.
struct SnapFlags {
    no_save: bool,
    clipboard: bool,
    fake: bool,
    ocr: bool,
}

fn snap(
    json: bool,
    area: Option<Vec<i32>>,
    monitor: usize,
    out: Option<PathBuf>,
    flags: SnapFlags,
) -> Result<()> {
    let SnapFlags {
        no_save,
        clipboard,
        fake,
        ocr,
    } = flags;
    let config = Config::default();
    let backend = backend(fake);

    if !fake && backend.permission() == Permission::Denied {
        return Err(Error::permission("screen recording permission is missing"));
    }

    let started = std::time::Instant::now();
    let session = Session::start(&*backend, config.clone())?;
    let captured = started.elapsed();

    let region = match area.as_deref() {
        Some([x, y, w, h]) if *w > 0 && *h > 0 => Some(Rect::new(*x, *y, *w as u32, *h as u32)),
        Some(_) => {
            return Err(Error::internal(
                "--area needs four values: X Y WIDTH HEIGHT (width and height > 0)",
            ))
        }
        None => None,
    };

    let mut options = ExportOptions::from_config(&config);
    // With --ocr the clipboard receives text, so the image itself must not take it.
    options.clipboard = clipboard && !ocr;
    options.save_to_file = !no_save || out.is_some();
    if let Some(path) = out {
        options.dir = path
            .parent()
            .map(Path::to_owned)
            .unwrap_or_else(|| config.export.save_dir.clone());
        options.filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
    }

    let exported = pinshot_app::export(&session, monitor, region, &options)?;

    // Record before printing: a capture that is not in the history is a capture the
    // user cannot find again.
    let history = match open_history(&config_data_dir()) {
        Ok(mut store) => {
            let scale = session
                .monitors()
                .get(monitor)
                .map(|monitor| monitor.scale)
                .unwrap_or(1.0);
            match pinshot_app::record(&mut store, &exported, "snip", scale) {
                Ok(id) => Some(id),
                Err(error) => {
                    eprintln!("warning: could not record the capture: {error}");
                    None
                }
            }
        }
        Err(error) => {
            eprintln!("warning: history unavailable: {error}");
            None
        }
    };

    let recognized = if ocr {
        let image = pinshot_app::decode_png(&exported.png)?;
        let lines = pinshot_ocr::recognize(&image)?;
        Some(pinshot_ocr::plain_text(&lines))
    } else {
        None
    };
    if let Some(text) = &recognized {
        pinshot_app::copy_text(text)?;
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "ocr": recognized,
                "width": exported.width,
                "height": exported.height,
                "path": exported.path,
                "pngBytes": exported.png.len(),
                "clipboard": exported.copied_to_clipboard,
                "captureMs": captured.as_millis() as u64,
                "totalMs": started.elapsed().as_millis() as u64,
                "memoryMb": session.memory() / (1024 * 1024),
                "historyId": history,
            })
        );
    } else {
        println!(
            "captured {}×{} in {} ms ({} ms total, {} MB frozen)",
            exported.width,
            exported.height,
            captured.as_millis(),
            started.elapsed().as_millis(),
            session.memory() / (1024 * 1024)
        );
        if let Some(path) = &exported.path {
            println!("saved    {}", path.display());
        }
        if exported.copied_to_clipboard {
            println!("clipboard  image copied");
        }
    }
    Ok(())
}

fn doctor(json: bool) -> Result<()> {
    let backend = system();
    let permission = backend.permission();
    let displays = backend.monitors();
    let cursor = backend.cursor();
    let config_path = pinshot_core::config::data_dir().join("config.toml");

    let permission_label = match permission {
        Permission::Granted => "granted",
        Permission::Denied => "denied",
        Permission::NotRequired => "not required",
    };

    if json {
        println!(
            "{}",
            serde_json::json!({
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "permission": permission_label,
                "monitors": displays.as_ref().map(|list| list.len()).unwrap_or(0),
                "cursor": cursor,
                "inputAccess": pinshot_capture::events::post_access(),
                "dataDir": pinshot_core::config::data_dir(),
                "config": config_path,
                "saveDir": pinshot_core::config::default_save_dir(),
            })
        );
    } else {
        println!(
            "os          {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        println!("permission  {permission_label}");
        match &displays {
            Ok(list) => println!("displays    {}", list.len()),
            Err(error) => println!("displays    error: {}", error.message),
        }
        match cursor {
            Some(Point { x, y }) => println!("cursor      ({x}, {y})"),
            None => println!("cursor      unavailable"),
        }
        println!(
            "accessibility {}",
            if pinshot_capture::events::ax_trusted(false) {
                "granted（自动滚动可用）"
            } else {
                "not granted（自动滚动需要「辅助功能」权限）"
            }
        );
        println!("data        {}", pinshot_core::config::data_dir().display());
        println!("config      {}", config_path.display());
        println!(
            "save dir    {}",
            pinshot_core::config::default_save_dir().display()
        );
        if permission == Permission::Denied {
            println!();
            println!("→ 打开「系统设置 → 隐私与安全性 → 屏幕录制」，勾选运行 pinshot 的终端后重试");
        }
    }

    Ok(())
}

fn config_data_dir() -> PathBuf {
    std::env::var_os("PINSHOT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(pinshot_core::config::data_dir)
}

fn scroll(frames: &Path, out: Option<PathBuf>, json: bool, max_height: Option<u32>) -> Result<()> {
    use pinshot_app::scrolling::{PushOutcome, ScrollCollector};

    let mut paths: Vec<PathBuf> = std::fs::read_dir(frames)
        .map_err(|error| Error::io(format!("cannot read {}: {error}", frames.display())))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase),
                Some(ext) if ext == "png" || ext == "jpg" || ext == "jpeg"
            )
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(Error::not_found(format!(
            "no images in {}",
            frames.display()
        )));
    }

    let mut options = pinshot_stitch::StitchOptions::default();
    if let Some(height) = max_height {
        options.max_height = height as usize;
    }
    let mut collector = ScrollCollector::new(options, 200);
    let started = std::time::Instant::now();
    for path in &paths {
        let bytes = std::fs::read(path)
            .map_err(|error| Error::io(format!("cannot read {}: {error}", path.display())))?;
        let image = pinshot_app::decode_png(&bytes)
            .map_err(|error| Error::io(format!("{}: {}", path.display(), error.message)))?;
        match collector.push(image) {
            PushOutcome::Accepted => {}
            PushOutcome::Duplicate => {}
            PushOutcome::Stopped(reason) => {
                if !json {
                    println!("stopped early: {reason:?}");
                }
                break;
            }
        }
    }

    let stitched = collector.finish()?;
    let png = pinshot_app::encode_png(&stitched)?;
    let target = out.unwrap_or_else(|| frames.join("..").join("scroll.png"));
    std::fs::write(&target, &png)
        .map_err(|error| Error::io(format!("cannot write {}: {error}", target.display())))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "frames": collector.len(),
                "width": stitched.width,
                "height": stitched.height,
                "path": target,
                "bytes": png.len(),
                "ms": started.elapsed().as_millis() as u64,
            })
        );
    } else {
        println!(
            "stitched {} frame(s) from {} source(s) → {}×{} in {} ms",
            collector.len(),
            paths.len(),
            stitched.width,
            stitched.height,
            started.elapsed().as_millis()
        );
        println!("saved    {}", target.display());
    }
    Ok(())
}

fn scroll_live(
    area: &[i32],
    out: Option<PathBuf>,
    max: u32,
    pid: Option<i32>,
    up: bool,
    json: bool,
) -> Result<()> {
    use pinshot_app::scrolling::{Direction, ScrollStep};

    let (x, y, w, h) = match area {
        [x, y, w, h] if *w > 0 && *h > 0 => (*x, *y, *w as u32, *h as u32),
        _ => {
            return Err(Error::internal(
                "--area needs X Y WIDTH HEIGHT, width/height > 0",
            ))
        }
    };
    if !pinshot_capture::events::ax_trusted(false) {
        return Err(Error::permission(
            "live scroll capture needs the Accessibility permission for the terminal",
        ));
    }
    let backend = system();
    let monitors = backend.monitors()?;
    let centre = Point::new(x + w as i32 / 2, y + h as i32 / 2);
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.bounds.contains(centre))
        .cloned()
        .ok_or_else(|| Error::not_found("the region is not on any display"))?;
    let region = monitor.rect_to_pixel(Rect::new(x - monitor.bounds.x, y - monitor.bounds.y, w, h));
    let target = pid.or_else(|| {
        pinshot_capture::windows::under_point(centre)
            .ok()
            .flatten()
            .map(|window| window.pid)
    });

    let direction = if up { Direction::Up } else { Direction::Down };
    let grab = || backend.grab(&monitor).map(|frame| frame.crop(region));

    let started = std::time::Instant::now();
    let steps = std::cell::Cell::new(0u32);
    let image = pinshot_app::scrolling::live_scroll(
        grab,
        |fraction| match target {
            Some(pid) => {
                let before = pinshot_capture::ax::scroll_position(pid).ok();
                match pinshot_capture::ax::scroll_app(pid, fraction) {
                    Ok(after) => Ok(ScrollStep::Moved(
                        before.map(|position| after as f64 - position),
                    )),
                    Err(_) => Ok(ScrollStep::Unsupported),
                }
            }
            None => match pinshot_capture::ax::scroll_at(centre, fraction) {
                Ok(_) => Ok(ScrollStep::Moved(None)),
                Err(_) => Ok(ScrollStep::Unsupported),
            },
        },
        || std::thread::sleep(std::time::Duration::from_millis(240)),
        || false,
        direction,
        max as usize,
        |_strip, height| {
            steps.set(steps.get() + 1);
            if !json {
                eprintln!("captured {} step(s), {} px tall", steps.get(), height);
            }
        },
    )?;
    let frames = steps.get();
    let png = pinshot_app::encode_png(&image)?;
    let target_path = out.unwrap_or_else(|| std::env::temp_dir().join("pinshot-scroll-live.png"));
    std::fs::write(&target_path, &png)
        .map_err(|error| Error::io(format!("cannot write {}: {error}", target_path.display())))?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "width": image.width,
                "height": image.height,
                "frames": frames,
                "path": target_path,
                "ms": started.elapsed().as_millis() as u64,
            })
        );
    } else {
        println!(
            "scrolled to {}×{} in {} frames, {} ms → {}",
            image.width,
            image.height,
            frames,
            started.elapsed().as_millis(),
            target_path.display()
        );
    }
    Ok(())
}

fn windows(json: bool, verify: bool) -> Result<()> {
    let list: Vec<pinshot_capture::windows::WindowInfo> = pinshot_capture::windows::list()?;
    if verify {
        // The window server can report a frame that does not match the pixels it hands
        // back (a stale or ghost entry with the same title). Capturing is the ground
        // truth: a window whose image is a different size is not the window the user
        // sees.
        println!(
            "{:<8} {:<20} {:<16} {:<16} MATCH",
            "ID", "APP", "REPORTED", "CAPTURED"
        );
        for window in list.iter().filter(|w| w.is_capturable()) {
            match pinshot_capture::windows::grab(window) {
                Ok(image) => println!(
                    "{:<8} {:<20} {:<16} {:<16} {}",
                    window.id,
                    truncate(&window.app, 18),
                    format!("{}×{}", window.bounds.w, window.bounds.h),
                    format!("{}×{}", image.width, image.height),
                    if image.width == window.bounds.w || image.width == window.bounds.w * 2 {
                        "yes"
                    } else {
                        "NO"
                    }
                ),
                Err(error) => println!(
                    "{:<8} {:<20} {:<16} {:<16} capture failed: {}",
                    window.id,
                    truncate(&window.app, 18),
                    format!("{}×{}", window.bounds.w, window.bounds.h),
                    "-",
                    error.message
                ),
            }
        }
        return Ok(());
    }
    if json {
        let values: Vec<serde_json::Value> = list
            .iter()
            .map(|window| {
                serde_json::json!({
                    "id": window.id,
                    "pid": window.pid,
                    "app": window.app,
                    "title": window.title,
                    "layer": window.layer,
                    "capturable": window.is_capturable(),
                    "bounds": { "x": window.bounds.x, "y": window.bounds.y,
                                "w": window.bounds.w, "h": window.bounds.h },
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(values));
        return Ok(());
    }
    println!("{:<8} {:<24} {:<10} TITLE", "ID", "APP", "SIZE");
    for window in list.iter().filter(|window| window.is_capturable()) {
        println!(
            "{:<8} {:<24} {:<10} {}",
            window.id,
            truncate(&window.app, 22),
            format!("{}×{}", window.bounds.w, window.bounds.h),
            truncate(&window.title, 60)
        );
    }
    Ok(())
}

fn ax_scroll(x: i32, y: i32, fraction: f32, pid: Option<i32>, json: bool) -> Result<()> {
    let position = match pid {
        Some(pid) => pinshot_capture::ax::scroll_app(pid, fraction)?,
        None => pinshot_capture::ax::scroll_at(Point::new(x, y), fraction)?,
    };
    if json {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "scrollbar": position })
        );
    } else {
        println!("scrollbar now at {:.2}", position);
    }
    Ok(())
}

fn window_shot(id: u32, out: &Path, json: bool) -> Result<()> {
    let windows = pinshot_capture::windows::list()?;
    let window = windows
        .into_iter()
        .find(|window| window.id == id)
        .ok_or_else(|| Error::not_found(format!("no window with id {id}")))?;
    let started = std::time::Instant::now();
    let image = pinshot_capture::windows::grab(&window)?;
    let png = pinshot_app::encode_png(&image)?;
    std::fs::write(out, &png)
        .map_err(|error| Error::io(format!("cannot write {}: {error}", out.display())))?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "app": window.app,
                "title": window.title,
                "width": image.width,
                "height": image.height,
                "path": out,
                "ms": started.elapsed().as_millis() as u64,
            })
        );
    } else {
        println!(
            "{} — {} → {}×{}",
            window.app, window.title, image.width, image.height
        );
        println!("saved {}", out.display());
    }
    Ok(())
}

fn ocr(file: &Path, json: bool) -> Result<()> {
    let bytes = std::fs::read(file)
        .map_err(|error| Error::io(format!("cannot read {}: {error}", file.display())))?;
    let image = pinshot_app::decode_png(&bytes)?;
    let started = std::time::Instant::now();
    let lines = pinshot_ocr::recognize(&image)?;
    let text = pinshot_ocr::plain_text(&lines);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "lines": lines.len(),
                "ms": started.elapsed().as_millis() as u64,
                "text": text,
                "boxes": lines.iter().map(|line| serde_json::json!({
                    "text": line.text,
                    "confidence": line.confidence,
                    "x": line.x, "y": line.y, "w": line.width, "h": line.height,
                })).collect::<Vec<_>>(),
            })
        );
    } else if lines.is_empty() {
        println!("no text found");
    } else {
        println!("{text}");
    }
    Ok(())
}

fn open_history(data_dir: &Path) -> Result<pinshot_store::Store> {
    pinshot_store::Store::open(&data_dir.join("history"))
}

fn history(data_dir: Option<PathBuf>, limit: u32, json: bool, prune: bool) -> Result<()> {
    let root = data_dir.unwrap_or_else(config_data_dir);
    let mut store = open_history(&root)?;
    if prune {
        let (config, _) = Config::load();
        let removed = store.prune(
            pinshot_store::RetainPolicy {
                keep_items: config.history.keep_items,
                keep_days: config.history.keep_days,
            },
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0),
        )?;
        println!("pruned {removed} capture(s)");
        return Ok(());
    }

    let rows = store.recent(limit)?;
    if json {
        let values: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "id": row.id,
                    "createdAt": row.created_at,
                    "kind": row.kind,
                    "width": row.width,
                    "height": row.height,
                    "scale": row.scale,
                    "favorite": row.favorite,
                    "app": row.app_name,
                })
            })
            .collect();
        println!("{}", serde_json::Value::Array(values));
        return Ok(());
    }
    if rows.is_empty() {
        println!("no captures yet");
        return Ok(());
    }
    println!(
        "{:<18} {:<12} {:<12} {:<8} ID",
        "CREATED", "SIZE", "KIND", "SCALE"
    );
    for row in rows {
        let created = chrono::Local
            .timestamp_opt(row.created_at, 0)
            .single()
            .map(|time| time.format("%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "—".into());
        println!(
            "{:<18} {:<12} {:<12} {:<8} {}",
            created,
            format!("{}×{}", row.width, row.height),
            row.kind,
            format!("{}x", row.scale),
            row.id
        );
    }
    Ok(())
}

fn simulate(action: SimulateAction) -> Result<()> {
    use pinshot_capture::events;
    // Hard gate: synthesising input moves the real pointer and steals focus from
    // whatever the owner is doing. It must be opt-in per shell, never a default.
    if std::env::var_os("PINSHOT_ALLOW_SYNTHETIC_INPUT").is_none() {
        return Err(Error::unsupported(
            "synthetic input is disabled (set PINSHOT_ALLOW_SYNTHETIC_INPUT=1 to allow it)",
        ));
    }
    let posted = match action {
        SimulateAction::Drag {
            x1,
            y1,
            x2,
            y2,
            steps,
        } => events::drag(Point::new(x1, y1), Point::new(x2, y2), steps),
        SimulateAction::Click { x, y } => events::click(Point::new(x, y)),
        SimulateAction::Key { name } => {
            let code = match name.to_ascii_lowercase().as_str() {
                "return" | "enter" => events::keys::RETURN,
                "escape" | "esc" => events::keys::ESCAPE,
                "tab" => events::keys::TAB,
                "space" => events::keys::SPACE,
                other => {
                    let mut characters = other.chars();
                    match (characters.next(), characters.next()) {
                        (Some(single), None) => events::keys::from_char(single)
                            .ok_or_else(|| Error::internal(format!("unknown key: {other}")))?,
                        _ => return Err(Error::internal(format!("unknown key: {other}"))),
                    }
                }
            };
            events::key(code)
        }
        SimulateAction::Warp { x, y } => events::warp(Point::new(x, y)),
        SimulateAction::Scroll { x, y, lines } => events::scroll(Point::new(x, y), lines),
    };
    if !posted {
        return Err(Error::unsupported(
            "the event could not be posted (Accessibility permission missing?)",
        ));
    }
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
