#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop shell. Thin by design: it opens windows, forwards commands to
//! `pinshot-app` and does nothing with pixels itself.

use pinshot_app::{ExportOptions, Session};
use pinshot_capture::{system, Permission};
use pinshot_core::{Config, Monitor, Rect};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Emitter;
use tauri::{AppHandle, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder};

mod platform;
mod selftest;

#[cfg(target_os = "macos")]
use platform::{accept_first_mouse, focus_webview, focus_webview_quiet};
use platform::{activate_app, raise_above_menu_bar, rest_activation_policy};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// `CGRequestScreenCaptureAccess` only ever prompts once per process, and macOS caches
/// the TCC answer until the app restarts. Asking again on every hotkey press just stacks
/// system dialogs (and eventually earns a "stopped auto-granting permission" notice), so
/// the first denial prompts and every later one stays quiet until a restart.
static SCREEN_ACCESS_PROMPTED: AtomicBool = AtomicBool::new(false);

/// True only for the first call in this process — see `SCREEN_ACCESS_PROMPTED`.
fn should_prompt_screen_access() -> bool {
    !SCREEN_ACCESS_PROMPTED.swap(true, Ordering::SeqCst)
}

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Arc<Session>>>,
    /// Result of the last scroll capture, as PNG bytes.
    scroll_result: Mutex<Option<Vec<u8>>>,
    scroll_cancel: Arc<AtomicBool>,
    /// True while a scroll capture is running: the overlay must stay hidden even if its
    /// webview finishes booting and asks to be shown.
    capturing: Arc<AtomicBool>,
    monitors: Mutex<Vec<Monitor>>,
    pins: Mutex<pinshot_app::pins::PinManager>,
    /// Guards against a second hotkey press while the overlay is already up.
    overlay_open: AtomicBool,
    /// Mirrors `general.keep_warm` so the idle timer and the capture path agree without
    /// reading the config file from a thread.
    keep_warm: AtomicBool,
    /// Bumped by every capture, so an idle-release timer can tell it is stale.
    capture_serial: std::sync::atomic::AtomicU64,
    /// When the current capture started, in microseconds since the epoch. Used for the
    /// click-to-pixels timing that `--capture-selftest` reports.
    capture_started: std::sync::atomic::AtomicU64,
    /// Set by the first `overlay_show` of a capture: the frozen frame is on screen.
    first_frame: AtomicBool,
    /// Global shortcuts that are not overlay keys, and what they do. The settings window
    /// lets the user change them, so the handler cannot hard-code F1.
    hotkeys: std::sync::Mutex<Vec<(Code, Modifiers, HotkeyAction)>>,
    /// Display scale of the overlay that is exporting, in `f32::to_bits` form.
    export_scale: AtomicU32,
    /// Global point where the next pin should appear (the selection's screen position).
    /// Set by the overlay just before `pin_png`; consumed (and cleared) by `pin_window`.
    pin_origin: Mutex<Option<(i32, i32)>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaDto {
    index: usize,
    points_width: u32,
    points_height: u32,
    pixel_width: u32,
    pixel_height: u32,
    scale: f32,
    /// 0.0 – 1.0, how much to darken the area outside the selection.
    dim: f32,
    /// Loupe magnification, 2–16×.
    magnifier_zoom: u32,
    /// Resolved interface language (`zh` / `en`) for this capture.
    locale: &'static str,
    /// Which capture these pixels belong to. An overlay that booted for an older one
    /// must never reveal itself over a newer capture.
    capture_serial: u64,
    /// Debug/scripting hook: start with this region selected instead of an empty overlay.
    initial_region: Option<RegionDto>,
    /// This monitor's top-left corner in the global virtual-desktop (points), so the
    /// overlay can place a pin at the selection's real screen position.
    origin_x: i32,
    origin_y: i32,
}

#[derive(Serialize, Clone, Copy)]
struct RegionDto {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[tauri::command]
fn set_export_scale(state: State<'_, AppState>, scale: f32) {
    state.export_scale.store(scale.to_bits(), Ordering::Relaxed);
}

/// Records where the overlay's current selection sits on the real screen, so the pin
/// window can appear over that spot instead of the top-left corner. Consumed by the
/// next `pin_window` call.
#[tauri::command]
fn set_pin_origin(state: State<'_, AppState>, x: i32, y: i32) {
    if let Ok(mut slot) = state.pin_origin.lock() {
        *slot = Some((x, y));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryItemDto {
    id: String,
    created_at: i64,
    width: u32,
    height: u32,
    kind: String,
    favorite: bool,
    app_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PinDto {
    id: String,
    capture_id: String,
    width: u32,
    height: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportDto {
    width: u32,
    height: u32,
    path: Option<String>,
    clipboard: bool,
    png_bytes: usize,
}

// ── commands ────────────────────────────────────────────────────────────────

#[tauri::command]
fn overlay_meta(app: AppHandle, index: usize) -> Result<MetaDto, String> {
    // The overlay's window is built while the screen grab is still running; wait for it
    // instead of failing, because that overlap is what keeps the tool feeling instant.
    wait_for_session(&app, std::time::Duration::from_millis(6000))?;
    let state = app.state::<AppState>();
    let monitors = state.monitors.lock().map_err(|error| error.to_string())?;
    let monitor = monitors
        .get(index)
        .ok_or_else(|| format!("monitor {index} is not part of this capture"))?;
    // Trust the frame we actually captured over what the display claimed: the two can
    // disagree (CGDisplayPixelsWide reports points on Retina) and a mismatch tears
    // every drawing operation.
    let session = state.session.lock().map_err(|error| error.to_string())?;
    let (pixel_width, pixel_height) = session
        .as_ref()
        .and_then(|session| session.frame(index))
        .map(|frame| (frame.width, frame.height))
        .unwrap_or((monitor.pixel_width, monitor.pixel_height));
    let scale = if monitor.bounds.w > 0 {
        pixel_width as f32 / monitor.bounds.w as f32
    } else {
        monitor.scale
    };
    // One-shot: a scripted run sets this before starting the capture, and it must not
    // leak into the next one (otherwise every later capture opens with a preset region
    // and the user sees a box without dragging).
    let initial_region = std::env::var("PINSHOT_DEBUG_REGION").ok().and_then(|spec| {
        if index != 0 {
            // Leave it for display 0 to consume.
            return None;
        }
        std::env::remove_var("PINSHOT_DEBUG_REGION");
        let values: Vec<f32> = spec
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        match values.as_slice() {
            [x, y, w, h] if index == 0 => Some(RegionDto {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
            }),
            _ => None,
        }
    });
    let (dim, magnifier_zoom) = session
        .as_ref()
        .map(|session| {
            (
                session.config.capture.dim,
                session.config.capture.magnifier_zoom,
            )
        })
        .unwrap_or((0.5, 8));
    Ok(MetaDto {
        index,
        points_width: monitor.bounds.w,
        points_height: monitor.bounds.h,
        pixel_width,
        pixel_height,
        scale,
        capture_serial: state.capture_serial.load(Ordering::SeqCst),
        dim,
        magnifier_zoom,
        locale: if LANGUAGE.load(Ordering::Relaxed) == 1 {
            "en"
        } else {
            "zh"
        },
        initial_region,
        origin_x: monitor.bounds.x,
        origin_y: monitor.bounds.y,
    })
}

/// True while no capture is running: a pre-created overlay parks until `overlay:begin`
/// instead of booting straight away.
#[tauri::command]
fn overlay_idle(app: AppHandle) -> Option<u64> {
    if overlay_is_open(&app) {
        Some(
            app.state::<AppState>()
                .capture_serial
                .load(Ordering::SeqCst),
        )
    } else {
        None
    }
}

#[tauri::command]
fn overlay_frame(app: AppHandle, index: usize) -> Result<tauri::ipc::Response, String> {
    let session = wait_for_session(&app, std::time::Duration::from_millis(6000))?;
    let frame = session
        .frame(index)
        .ok_or_else(|| format!("monitor {index} has no frame"))?;
    let started = std::time::Instant::now();
    let bytes = frame.rgba.clone();
    diag(&format!(
        "perf: overlay_frame {} MB copied in {} ms",
        bytes.len() / (1024 * 1024),
        started.elapsed().as_millis()
    ));
    Ok(tauri::ipc::Response::new(bytes))
}

/// The frontend calls this once the frame is on screen, so the window never flashes
/// white before the frozen desktop is drawn.
#[tauri::command]
fn overlay_show(app: AppHandle, index: usize, serial: Option<u64>) -> Result<(), String> {
    // AppKit calls must happen on the main thread; a command may arrive on the IPC
    // thread, where `makeKeyAndOrderFront` silently does nothing.
    let handle = app.clone();
    app.run_on_main_thread(move || {
        // A slow boot (the first one after the Mac woke up) must not draw itself over a
        // capture the user has already restarted.
        if let Some(serial) = serial {
            if handle
                .state::<AppState>()
                .capture_serial
                .load(Ordering::SeqCst)
                != serial
            {
                if let Some(window) = handle.get_webview_window(&label(index)) {
                    let _ = window.hide();
                }
                return;
            }
        }
        // A cancel may have landed while this window was still booting: never reveal a
        // window from a session the user already dismissed.
        if handle.state::<AppState>().capturing.load(Ordering::SeqCst) {
            // A scroll capture is running; its whole point is that the real page is
            // visible. Showing the overlay here is what made "the box is back" happen.
            if let Some(window) = handle.get_webview_window(&label(index)) {
                let _ = window.hide();
            }
            return;
        }
        if !overlay_is_open(&handle) {
            if let Some(window) = handle.get_webview_window(&label(index)) {
                // Hide, never close: the page is reused for the next capture.
                let _ = window.hide();
            }
            return;
        }
        if let Some(window) = handle.get_webview_window(&label(index)) {
            let state = handle.state::<AppState>();
            let first = !state.first_frame.swap(true, Ordering::SeqCst);
            // Do it here, not when the window is built: WebKit creates its content view
            // while the page loads, and that view is the one a click hit-tests.
            #[cfg(target_os = "macos")]
            {
                // The activation started with the hotkey can be dropped while the
                // display wakes; without this the first drag only activates the app.
                activate_app();
                accept_first_mouse(&window);
            }
            let shown_at = std::time::Instant::now();
            let _ = window.show();
            if first {
                let started = state.capture_started.load(Ordering::SeqCst);
                let elapsed = now_micros().saturating_sub(started) as f64 / 1000.0;
                diag(&format!(
                    "perf: first frame on screen after {elapsed:.0} ms (overlay {index}, show {:.0} ms)",
                    shown_at.elapsed().as_secs_f64() * 1000.0
                ));
            }
            diag(&format!("overlay {index} shown"));
            // Handing the keyboard over is not on the pixel path: the window is already
            // on screen and no click can land inside the few milliseconds it takes.
            let _ = window.set_focus();
            #[cfg(target_os = "macos")]
            focus_webview(&window);
        }
        // Activation is refused while a tray menu is still closing, or while a display is
        // still waking from sleep. Two late retries (400/800 ms) cover both without the
        // old 80/220/450 ladder that re-ordered both full-screen windows three times per
        // capture — visible as the overlay blinking several times on each hotkey press.
        // Every retry is quiet: no activate/makeKeyAndOrderFront, so no blink.
        let retry = handle.clone();
        std::thread::spawn(move || {
            for delay in [400u64, 800] {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                let nested = retry.clone();
                let _ = retry.run_on_main_thread(move || {
                    if let Some(window) = nested.get_webview_window(&label(index)) {
                        if window.is_visible().unwrap_or(false) {
                            // `set_focus` re-orders the window even when it is already key.
                            // Only do it if activation really was refused (tray menu still
                            // closing, display just woke); otherwise re-asserting the first
                            // responder alone is enough and does not blink the overlay.
                            if !window.is_focused().unwrap_or(false) {
                                let _ = window.set_focus();
                            }
                            #[cfg(target_os = "macos")]
                            focus_webview_quiet(&window);
                        }
                    }
                });
            }
        });
    })
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn overlay_error(index: usize, message: String) {
    // Also goes to the diag file: the webview has no terminal attached when the app runs
    // from Finder, and that is exactly when a report is needed.
    diag(&format!("webview[{index}]: {message}"));
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn capture_commit(
    app: AppHandle,
    state: State<'_, AppState>,
    index: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    action: String,
) -> Result<ExportDto, String> {
    let session = state
        .session
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "no active capture".to_owned())?;

    let save = action == "file";
    let pin = action == "pin";
    let (scale, monitor_origin) = state
        .monitors
        .lock()
        .ok()
        .and_then(|monitors| {
            monitors
                .get(index)
                .map(|monitor| (monitor.scale, (monitor.bounds.x, monitor.bounds.y)))
        })
        .unwrap_or((1.0, (0, 0)));
    // Selection coordinates are local to this monitor; the pin appears over the real
    // screen spot by offsetting with the monitor's global origin.
    let pin_at = Some((monitor_origin.0 + x, monitor_origin.1 + y));
    let (exported, capture_id) = tauri::async_runtime::spawn_blocking(move || {
        let mut options = ExportOptions::from_config(&session.config);
        // Pinning still writes the PNG to the history store: the pin window reads its
        // pixels from there, exactly like a pin restored from the tray.
        options.clipboard = !save && !pin;
        options.save_to_file = save;
        let exported = pinshot_app::export(
            &session,
            index,
            Some(Rect::new(x, y, width, height)),
            &options,
        )?;
        let id = record(&exported, scale, "snip");
        Ok::<_, pinshot_core::Error>((exported, id))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;

    // Build the pin before hiding the overlay: on Windows, creating a WebView2 window
    // while the overlay is being torn down leaves the new window on its blank first
    // surface until the user moves it.
    if pin {
        let capture_id = capture_id
            .ok_or_else(|| tr("无法写入历史记录", "Could not store the capture").to_owned())?;
        if let Err(error) = pin_window(
            &app,
            capture_id,
            exported.width,
            exported.height,
            scale,
            pin_at,
        ) {
            diag(&format!("pin: {error}"));
            return Err(error);
        }
    }
    close_overlays(&app);
    Ok(ExportDto {
        width: exported.width,
        height: exported.height,
        path: exported.path.map(|path| path.display().to_string()),
        clipboard: exported.copied_to_clipboard,
        png_bytes: exported.png.len(),
    })
}

fn raw_png(request: &tauri::ipc::Request<'_>) -> Result<Vec<u8>, String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => Ok(bytes.clone()),
        _ => Err("expected raw PNG bytes".to_owned()),
    }
}

/// Annotated export: the overlay renders the final pixels (single renderer, so the
/// result is exactly what the user saw) and hands us a PNG.
#[tauri::command]
fn copy_png(
    app: AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ExportDto, String> {
    let bytes = raw_png(&request)?;
    let image = pinshot_app::decode_png(&bytes).map_err(|error| error.to_string())?;
    pinshot_app::copy_to_clipboard(&image).map_err(|error| error.to_string())?;
    let exported = pinshot_app::Export {
        path: None,
        width: image.width,
        height: image.height,
        png: Arc::from(bytes.as_slice()),
        copied_to_clipboard: true,
    };
    record(&exported, export_scale(&state), "snip");
    close_overlays(&app);
    Ok(ExportDto {
        width: image.width,
        height: image.height,
        path: None,
        clipboard: true,
        png_bytes: bytes.len(),
    })
}

#[tauri::command]
fn save_png(
    app: AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ExportDto, String> {
    let bytes = raw_png(&request)?;
    let image = pinshot_app::decode_png(&bytes).map_err(|error| error.to_string())?;
    let (config, _) = Config::load();
    let path = pinshot_app::write_file(&config.export.save_dir, None, &bytes)
        .map_err(|error| error.to_string())?;
    let exported = pinshot_app::Export {
        path: Some(path.clone()),
        width: image.width,
        height: image.height,
        png: Arc::from(bytes.as_slice()),
        copied_to_clipboard: false,
    };
    record(&exported, export_scale(&state), "snip");
    close_overlays(&app);
    Ok(ExportDto {
        width: image.width,
        height: image.height,
        path: Some(path.display().to_string()),
        clipboard: false,
        png_bytes: bytes.len(),
    })
}

/// Annotated export for the 「贴图」 button: store the pixels, then pin them.
#[tauri::command]
fn pin_png(
    app: AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<PinDto, String> {
    let bytes = raw_png(&request)?;
    let image = pinshot_app::decode_png(&bytes).map_err(|error| error.to_string())?;
    let exported = pinshot_app::Export {
        path: None,
        width: image.width,
        height: image.height,
        png: Arc::from(bytes.as_slice()),
        copied_to_clipboard: false,
    };
    let scale = export_scale(&state);
    let pin_at = state
        .pin_origin
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    let capture_id = record(&exported, scale, "snip")
        .ok_or_else(|| tr("无法写入历史记录", "Could not store the capture").to_owned())?;
    let pinned = pin_window(&app, capture_id, image.width, image.height, scale, pin_at).map_err(
        |error| {
            diag(&format!("pin: {error}"));
            error
        },
    )?;
    close_overlays(&app);
    Ok(pinned)
}

/// Pins the newest capture. Mouse-driven on purpose: it is reachable from the tray
/// menu and therefore works even when the window does not own the keyboard.
fn pin_capture(app: &AppHandle, capture: pinshot_store::CaptureRow) -> Result<PinDto, String> {
    pin_window(
        app,
        capture.id,
        capture.width,
        capture.height,
        capture.scale,
        None,
    )
}

/// Opens a always-on-top window for an image already stored under `capture_id`.
fn pin_window(
    app: &AppHandle,
    capture_id: String,
    pixel_width: u32,
    pixel_height: u32,
    scale: f32,
    at: Option<(i32, i32)>,
) -> Result<PinDto, String> {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let width = (pixel_width as f32 / scale).round().max(1.0) as u32;
    let height = (pixel_height as f32 / scale).round().max(1.0) as u32;
    let count = app
        .state::<AppState>()
        .pins
        .lock()
        .map_err(|e| e.to_string())?
        .len() as i32;
    let monitors = pinshot_capture::system().monitors().unwrap_or_default();
    // The virtual desktop is the union of all displays: clamp there so a pin placed on
    // the secondary monitor is not dragged back onto the primary, and its close button
    // is always reachable.
    let desktop = monitors
        .iter()
        .map(|monitor| monitor.bounds)
        .reduce(|a, b| a.union(&b));
    // Keep the window inside a display: an oversized pin puts its close button off
    // screen, and a pin you cannot close is worse than no pin. The image scales down
    // (`object-fit: contain`), so the content stays fully visible.
    let (width, height) = match &desktop {
        Some(desktop) => (
            width.min(desktop.w.saturating_sub(40)).max(80),
            height.min(desktop.h.saturating_sub(80)).max(60),
        ),
        None => (width, height),
    };
    // Preferred: appear exactly over the selection the user just made. Otherwise (pins
    // from history / tray, which have no selection) fall back to the staggered corner.
    let (mut x, mut y) = match at {
        Some((x, y)) => (x, y),
        None => {
            let base = 80 + count * 24;
            (base, base)
        }
    };
    if let Some(desktop) = &desktop {
        x = x
            .max(desktop.x + 20)
            .min(desktop.x + desktop.w as i32 - width as i32 - 20);
        y = y
            .max(desktop.y + 20)
            .min(desktop.y + desktop.h as i32 - height as i32 - 20);
    }
    let bounds = Rect::new(x, y, width, height);

    let id = {
        let state = app.state::<AppState>();
        let mut manager = state.pins.lock().map_err(|error| error.to_string())?;
        manager.create(bounds, Some(capture_id.clone()))
    };

    let window = WebviewWindowBuilder::new(app, id.clone(), WebviewUrl::App("index.html".into()))
        .title("PinShot")
        .position(bounds.x as f64, bounds.y as f64)
        .inner_size(bounds.w as f64, bounds.h as f64)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .shadow(true)
        .skip_taskbar(true)
        .build()
        .map_err(|error| error.to_string())?;
    let _ = window.set_focus();

    Ok(PinDto {
        id,
        capture_id,
        width: pixel_width,
        height: pixel_height,
    })
}

#[tauri::command]
fn pin_last_capture(app: AppHandle) -> Result<PinDto, String> {
    let directory = pinshot_core::config::data_dir().join("history");
    let store = pinshot_store::Store::open(&directory).map_err(|error| error.to_string())?;
    let capture = store
        .recent(1)
        .map_err(|error| error.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| tr("还没有截图可以贴", "There is no capture to pin yet").to_owned())?;
    pin_capture(&app, capture)
}

/// The pinned image, as PNG bytes.
#[tauri::command]
fn pin_image(app: AppHandle, id: String) -> Result<tauri::ipc::Response, String> {
    diag(&format!("pin_image: {id} requested"));
    let capture_id = {
        let state = app.state::<AppState>();
        let manager = state.pins.lock().map_err(|error| error.to_string())?;
        manager
            .get(&id)
            .and_then(|pin| pin.capture_id.clone())
            .ok_or_else(|| format!("no pin {id}"))?
    };
    let directory = pinshot_core::config::data_dir().join("history");
    let store = pinshot_store::Store::open(&directory).map_err(|error| error.to_string())?;
    let row = store
        .capture(&capture_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the capture for this pin is gone".to_owned())?;
    let hash = row
        .blob_hash
        .ok_or_else(|| "the capture has no image".to_owned())?;
    let bytes = store.read_blob(&hash).map_err(|error| error.to_string())?;
    diag(&format!("pin_image: {id} -> {} bytes", bytes.len()));
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
fn pin_close(app: AppHandle, id: String) -> Result<(), String> {
    let closed = {
        let state = app.state::<AppState>();
        let mut manager = state.pins.lock().map_err(|error| error.to_string())?;
        manager.close(&id)
    };
    if closed {
        if let Some(window) = app.get_webview_window(&id) {
            let _ = window.close();
        }
    }
    Ok(())
}

#[tauri::command]
fn pin_restore(app: AppHandle) -> Result<Option<String>, String> {
    let restored = {
        let state = app.state::<AppState>();
        let mut manager = state.pins.lock().map_err(|error| error.to_string())?;
        manager.restore()
    };
    let Some(id) = restored else { return Ok(None) };
    // A restored pin needs its window back; the row survives the close.
    if app.get_webview_window(&id).is_none() {
        let bounds = {
            let state = app.state::<AppState>();
            let manager = state.pins.lock().map_err(|error| error.to_string())?;
            manager.get(&id).map(|pin| pin.bounds)
        };
        if let Some(bounds) = bounds {
            let _ =
                WebviewWindowBuilder::new(&app, id.clone(), WebviewUrl::App("index.html".into()))
                    .title("PinShot")
                    .position(bounds.x as f64, bounds.y as f64)
                    .inner_size(bounds.w as f64, bounds.h as f64)
                    .decorations(false)
                    .always_on_top(true)
                    .resizable(false)
                    .skip_taskbar(true)
                    .build();
        }
    }
    Ok(Some(id))
}

fn history_store() -> Result<pinshot_store::Store, String> {
    let directory = pinshot_core::config::data_dir().join("history");
    pinshot_store::Store::open(&directory).map_err(|error| error.to_string())
}

#[tauri::command]
fn history_list(limit: u32) -> Result<Vec<HistoryItemDto>, String> {
    let store = history_store()?;
    let rows = store.recent(limit).map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| HistoryItemDto {
            id: row.id,
            created_at: row.created_at,
            width: row.width,
            height: row.height,
            kind: row.kind,
            favorite: row.favorite,
            app_name: row.app_name,
        })
        .collect())
}

/// A cached 320 px thumbnail, generated on first request.
#[tauri::command]
fn history_thumb(id: String) -> Result<tauri::ipc::Response, String> {
    let directory = pinshot_core::config::data_dir().join("history");
    let cache = directory.join("thumbs").join(format!("{id}.png"));
    if let Ok(bytes) = std::fs::read(&cache) {
        return Ok(tauri::ipc::Response::new(bytes));
    }
    let store = pinshot_store::Store::open(&directory).map_err(|error| error.to_string())?;
    let row = store
        .capture(&id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("no capture {id}"))?;
    let hash = row
        .blob_hash
        .ok_or_else(|| "the capture has no image".to_owned())?;
    let png = store.read_blob(&hash).map_err(|error| error.to_string())?;
    let thumb = pinshot_app::thumbnail(&png, 320).map_err(|error| error.to_string())?;
    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&cache, &thumb);
    Ok(tauri::ipc::Response::new(thumb))
}

/// Puts a capture back on the clipboard.
#[tauri::command]
fn history_copy(id: String) -> Result<(), String> {
    let store = history_store()?;
    let row = store
        .capture(&id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("no capture {id}"))?;
    let hash = row
        .blob_hash
        .ok_or_else(|| "the capture has no image".to_owned())?;
    let png = store.read_blob(&hash).map_err(|error| error.to_string())?;
    let image = pinshot_app::decode_png(&png).map_err(|error| error.to_string())?;
    pinshot_app::copy_to_clipboard(&image).map_err(|error| error.to_string())
}

/// Copies a pinned image to the clipboard (right-click menu in the pin window).
#[tauri::command]
fn pin_copy(app: AppHandle, id: String) -> Result<(), String> {
    let capture_id = {
        let state = app.state::<AppState>();
        let manager = state.pins.lock().map_err(|error| error.to_string())?;
        manager
            .get(&id)
            .and_then(|pin| pin.capture_id.clone())
            .ok_or_else(|| format!("no pin {id}"))?
    };
    history_copy(capture_id)
}

#[tauri::command]
fn history_delete(id: String) -> Result<(), String> {
    let mut store = history_store()?;
    store
        .delete_capture(&id)
        .map_err(|error| error.to_string())?;
    let cache = pinshot_core::config::data_dir()
        .join("history")
        .join("thumbs")
        .join(format!("{id}.png"));
    let _ = std::fs::remove_file(cache);
    Ok(())
}

/// Empties the history. Favourites go too — the button says 「全部清空」.
#[tauri::command]
fn history_clear() -> Result<u32, String> {
    let mut store = history_store()?;
    let removed = store.clear_all().map_err(|error| error.to_string())?;
    // The thumbnail cache is derived data: drop it so the grid cannot show files whose
    // rows are gone.
    let cache = pinshot_core::config::data_dir()
        .join("history")
        .join("thumbs");
    if let Ok(entries) = std::fs::read_dir(cache) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(removed)
}

#[tauri::command]
fn history_favorite(id: String, favorite: bool) -> Result<(), String> {
    let mut store = history_store()?;
    store
        .set_favorite(&id, favorite)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn history_prune() -> Result<u32, String> {
    let (config, _) = Config::load();
    let mut store = history_store()?;
    store
        .prune(
            pinshot_store::RetainPolicy {
                keep_items: config.history.keep_items,
                keep_days: config.history.keep_days,
            },
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_load() -> (Config, Option<String>) {
    Config::load()
}

#[tauri::command]
fn settings_save(app: AppHandle, config: Config) -> Result<(), String> {
    // The settings window saves on every change now, so this runs on each keystroke
    // and on every tick of a slider. Only the side effects whose value actually moved
    // are applied: re-registering four global hotkeys and rebuilding the tray for a
    // one-pixel dim change was pure waste, and the language event would repaint the
    // pre-warmed overlay for nothing.
    let (previous, _) = Config::load();
    config.save().map_err(|error| error.to_string())?;

    if previous.general.hotkey_snip != config.general.hotkey_snip
        || previous.general.hotkey_paste != config.general.hotkey_paste
    {
        // A new shortcut takes effect now instead of after a restart.
        register_hotkey(&app);
    }
    if previous.general.language != config.general.language {
        set_language(&config);
        // The tray menu is native and the overlay is a long-lived page, so neither
        // would notice a language switch on its own.
        refresh_tray(&app);
        let locale = if LANGUAGE.load(Ordering::Relaxed) == 1 {
            "en"
        } else {
            "zh"
        };
        let _ = app.emit("language:changed", locale);
    }
    app.state::<AppState>()
        .keep_warm
        .store(config.general.keep_warm, Ordering::SeqCst);
    if previous.general.keep_warm && !config.general.keep_warm {
        schedule_idle_release(&app);
    }
    if previous.general.launch_at_login != config.general.launch_at_login {
        // 「开机启动」 used to be a checkbox that wrote a config value nobody read.
        spawn_launch_at_login(config.general.launch_at_login);
    }
    Ok(())
}

/// The LaunchAgent that starts PinShot at login.
#[cfg(target_os = "macos")]
fn autostart_plist_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join("Library/LaunchAgents/ai.pinshot.desktop.plist"))
}

#[cfg(target_os = "macos")]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Applies the 「开机启动」 setting.
///
/// A LaunchAgent is what macOS actually reads at login. `launchctl` is slow enough to be
/// worth a thread, and a failure must never take the settings window down with it.
fn spawn_launch_at_login(enabled: bool) {
    std::thread::spawn(move || apply_launch_at_login(enabled));
}

#[cfg(target_os = "macos")]
fn apply_launch_at_login(enabled: bool) {
    let Some(path) = autostart_plist_path() else {
        return;
    };
    if !enabled {
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg("-w")
            .arg(&path)
            .status();
        let _ = std::fs::remove_file(&path);
        diag("autostart: off");
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // Inside a bundle, go through LaunchServices (`open -g`, no focus steal) rather than
    // executing the binary directly: that is what keeps the login launch identical to
    // double-clicking the app.
    let arguments = match exe
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
    {
        Some(bundle) => format!(
            "<string>/usr/bin/open</string><string>-g</string><string>-a</string><string>{}</string>",
            xml_escape(&bundle.display().to_string())
        ),
        None => format!("<string>{}</string>", xml_escape(&exe.display().to_string())),
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.pinshot.desktop</string>
  <key>ProgramArguments</key><array>{arguments}</array>
  <key>RunAtLoad</key><true/>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, plist) {
        Ok(()) => {
            let _ = std::process::Command::new("launchctl")
                .arg("load")
                .arg("-w")
                .arg(&path)
                .status();
            diag(&format!("autostart: on ({})", path.display()));
        }
        Err(error) => diag(&format!(
            "autostart: cannot write {}: {error}",
            path.display()
        )),
    }
}

/// Windows keeps login items in the per-user `Run` key; no service or shortcut needed.
#[cfg(target_os = "windows")]
fn apply_launch_at_login(enabled: bool) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut command = std::process::Command::new("reg");
    command.creation_flags(CREATE_NO_WINDOW);
    if enabled {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // The quotes keep the path intact when it contains a space; the marker lets
        // the next launch tell "Windows started me" from "the user clicked me", which
        // is the difference between a broken autostart and an icon hidden in the tray.
        let value = format!("\"{}\" --autostart", exe.display());
        let status = command
            .args([
                "add",
                RUN_KEY,
                "/v",
                "PinShot",
                "/t",
                "REG_SZ",
                "/d",
                value.as_str(),
                "/f",
            ])
            .status();
        diag(&format!(
            "autostart: on ({})",
            status.map(|status| status.success()).unwrap_or(false)
        ));
    } else {
        let _ = command
            .args(["delete", RUN_KEY, "/v", "PinShot", "/f"])
            .status();
        diag("autostart: off");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn apply_launch_at_login(_enabled: bool) {}

/// Brings a normal window to the front and gives it the keyboard.
///
/// PinShot is a menu-bar (accessory) process, and macOS will not activate an accessory
/// app or make its windows key on its own. It also refuses activation while the tray
/// menu is still closing, so the first click on 「设置」/「历史记录」 used to show the
/// window behind everything and only a second click brought it forward. Hence: switch to
/// a regular app, activate, then repeat shortly after the menu has gone.
fn reveal_window(app: &AppHandle, label: &str) {
    let name = label.to_owned();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(&name) {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            #[cfg(target_os = "macos")]
            focus_webview(&window);
        }
    });

    let handle = app.clone();
    let name = label.to_owned();
    std::thread::spawn(move || {
        for delay in [80u64, 220, 450] {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            let nested = handle.clone();
            let label = name.clone();
            let _ = handle.run_on_main_thread(move || {
                if let Some(window) = nested.get_webview_window(&label) {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.set_focus();
                        #[cfg(target_os = "macos")]
                        focus_webview(&window);
                    }
                }
            });
        }
    });
}

/// Opens (or focuses) a secondary window: history or settings.
#[tauri::command]
fn open_window(app: AppHandle, kind: String) -> Result<(), String> {
    // Settings/history are ordinary windows and should bring the menu bar and Dock
    // icon back. The capture overlay deliberately stays accessory: switching policy
    // costs ~150 ms and that delay is exactly when the first drag used to be swallowed.
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    let (label, title, width, height) = match kind.as_str() {
        "history" => ("history", tr("历史记录", "History"), 940.0, 660.0),
        // Tall enough that the shortcut section is visible without scrolling, but still
        // resizable and scrollable on smaller displays.
        "settings" => ("settings", tr("设置", "Settings"), 640.0, 780.0),
        other => return Err(format!("unknown window: {other}")),
    };
    if app.get_webview_window(label).is_some() {
        reveal_window(&app, label);
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(width, height)
        .min_inner_size(420.0, 320.0)
        .build()
        .map_err(|error| error.to_string())?;
    // A freshly built window is not frontmost either: same activation dance.
    reveal_window(&app, label);

    // Back to a menu-bar-only process once the user closes the last regular window, so
    // the Dock icon does not linger after the settings are dismissed.
    let handle = app.clone();
    window.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::Destroyed) {
            return;
        }
        rest_activation_policy(&handle);
        // A settings window can be closed while a shortcut field is armed: the page's
        // cleanup never runs then, so hand the global shortcuts back here.
        if !aux_window_visible(&handle) {
            register_hotkey(&handle);
        }
    });
    Ok(())
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScrollProgressDto {
    frames: usize,
    height: u32,
    state: &'static str,
    /// Why a run failed, in words the overlay can show as-is.
    message: Option<String>,
    /// Frames dropped because the page jumped more than a viewport between captures.
    skipped: usize,
}

/// Captures a scrolling page: hides the overlay, injects wheel events, stitches.
///
/// The overlay is hidden first because it shows a *frozen* frame; scrolling the app
/// behind it would be invisible. Everything else is `pinshot_app::auto_scroll`, which
/// is unit tested against synthetic pages.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn scroll_start(
    app: AppHandle,
    state: State<'_, AppState>,
    index: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    mode: String,
    direction: Option<String>,
) -> Result<(), String> {
    let manual = mode == "manual";
    // Only the automatic mode has to drive the page, which needs the Accessibility
    // permission. Manual capture just watches the screen while the user scrolls, so
    // asking for 辅助功能 there would be a permission request the feature does not use.
    if !manual && !pinshot_capture::events::ax_trusted(false) {
        // Shows the system dialog and registers PinShot under 辅助功能.
        pinshot_capture::events::request_ax_trust();
        return Err(tr(
            "滚动截屏需要「辅助功能」权限：请在系统设置里允许 PinShot 控制你的电脑，然后重试",
            "Scrolling capture needs the Accessibility permission: allow PinShot to control your computer in System Settings, then try again",
        )
        .to_owned());
    }
    let upward = direction.as_deref() == Some("up");
    // Scrolling goes through the Accessibility API: no wheel events are synthesised,
    // so the user's own scrolling is untouched. Only the (opt-in) wheel fallback needs
    // `capture.allow_synthetic_scroll`.
    let session = state
        .session
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "no active capture".to_owned())?;
    let monitor = session
        .monitors()
        .get(index)
        .cloned()
        .ok_or_else(|| format!("monitor {index} is not part of this capture"))?;

    // Region in the monitor's pixel space, and the global point the wheel is posted at.
    let region = monitor.rect_to_pixel(Rect::new(x, y, width, height));
    let centre = pinshot_core::Point::new(
        monitor.bounds.x + x + width as i32 / 2,
        monitor.bounds.y + y + height as i32 / 2,
    );

    // Claim the scroll session BEFORE the hide/wait loop (~1.4 s). A hotkey pressed in
    // that window used to see `capturing == false`, so close_overlays did not set
    // `scroll_cancel`; the scroll survived the cancel, then re-showed a ghost overlay
    // over a session that no longer existed. Claiming early makes the cancel reach it.
    state.capturing.store(true, Ordering::SeqCst);
    state
        .scroll_cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let scroll_serial = state.capture_serial.load(Ordering::SeqCst);

    // Hiding a window is an AppKit call: off the main thread it is silently ignored and
    // the captures then contain the frozen overlay (identical frames forever).
    {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let mut hidden = 0;
            for (label, window) in handle.webview_windows() {
                if label.starts_with("overlay-") && window.hide().is_ok() {
                    hidden += 1;
                }
            }
            diag(&format!("scroll: hide called on {hidden} overlay(s)"));
        });
    }
    // Do not start grabbing until the overlays are actually off screen: `hide()` is
    // asynchronous, and a grab taken too early contains the frozen overlay (every frame
    // then looks identical and the capture "sees no movement").
    for _ in 0..25 {
        let visible = app.webview_windows().into_iter().any(|(label, window)| {
            label.starts_with("overlay-") && window.is_visible().unwrap_or(false)
        });
        if !visible {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    if manual {
        // The overlay is gone, so a notification is the only way to tell the user what to
        // do next. It has to come after the hide, otherwise it competes with it.
        notify(
            &app,
            tr("滚动采集中", "Capturing as you scroll"),
            tr(
                "慢慢滚（一屏一屏地滚最稳），上下都行；按 Enter 完成",
                "Scroll slowly (one screen at a time works best), either direction; press Enter to finish",
            ),
        );
    }
    // The window server keeps compositing the overlay for a frame or two after it is
    // "hidden"; grabbing immediately would put the frozen overlay into frame 1 and the
    // stitcher then sees two unrelated pictures.
    std::thread::sleep(std::time::Duration::from_millis(250));
    // If something re-showed a window while we were waiting (the webview's own
    // `overlay_show` used to do exactly that), hide again before grabbing.
    for _ in 0..3 {
        let visible: Vec<tauri::WebviewWindow> = app
            .webview_windows()
            .into_iter()
            .filter(|(label, window)| {
                label.starts_with("overlay-") && window.is_visible().unwrap_or(false)
            })
            .map(|(_, window)| window)
            .collect();
        if visible.is_empty() {
            break;
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            for (label, window) in handle.webview_windows() {
                if label.starts_with("overlay-") {
                    let _ = window.hide();
                }
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    diag(&format!(
        "scroll: overlays visible after hide: {}",
        app.webview_windows()
            .into_iter()
            .filter(|(label, window)| label.starts_with("overlay-")
                && window.is_visible().unwrap_or(false))
            .count()
    ));

    let cancel = state.scroll_cancel.clone();
    let handle = app.clone();
    let target_pid = target_pid_for_region(&monitor, x, y, width, height);

    std::thread::spawn(move || {
        let backend = pinshot_capture::system();
        let options = pinshot_app::scrolling::ScrollRunOptions {
            direction: if upward {
                pinshot_app::scrolling::Direction::Up
            } else {
                pinshot_app::scrolling::Direction::Down
            },
            // Manual capture ticks every 120 ms, so allow enough frames for a long page
            // without holding a huge number of full-screen buffers.
            max_frames: 60,
            ..Default::default()
        };
        let cancel_for_stop = cancel.clone();
        let capture_frames = || {
            let frame = backend.grab(&monitor)?;
            Ok(frame.crop(region))
        };
        let warned = std::sync::Arc::new(AtomicBool::new(false));
        let warned_for_emit = warned.clone();
        let handle_for_emit = handle.clone();
        let progress_emit = |progress: pinshot_app::scrolling::ScrollProgress| {
            let _ = handle.emit(
                "scroll:progress",
                ScrollProgressDto {
                    frames: progress.frames,
                    height: progress.height,
                    state: "capturing",
                    message: None,
                    skipped: progress.skipped,
                },
            );
            // One gentle nudge, not a stream of them.
            if progress.skipped > 0 && !warned_for_emit.swap(true, Ordering::SeqCst) {
                notify(
                    &handle_for_emit,
                    tr("滚得有点快", "Scrolled too fast"),
                    tr(
                        "两次取样之间没有重叠了，慢一点滚，长图才不会有缺口",
                        "Two samples no longer overlap — scroll a little slower so the long image has no gaps",
                    ),
                );
            }
        };

        let result = if manual {
            // Manual mode: the user scrolls, we only observe. Stop on Enter/Esc, on
            // the page ending, or after the frame cap.
            pinshot_app::scrolling::manual_scroll(
                capture_frames,
                || std::thread::sleep(std::time::Duration::from_millis(260)),
                || cancel_for_stop.load(std::sync::atomic::Ordering::SeqCst),
                options,
                || pinshot_capture::ax::scroll_position(target_pid).ok(),
                || std::thread::sleep(std::time::Duration::from_millis(90)),
                progress_emit,
            )
        } else {
            pinshot_app::scrolling::auto_scroll(
                capture_frames,
                |pixels| {
                    // Scrolling through the Accessibility API: no wheel events, no pointer
                    // movement, no focus stealing. `pixels` is converted to a fraction of
                    // the captured region, which is what AXValue expects.
                    let viewport = (height as f32 * monitor.scale.max(1.0)).max(1.0);
                    let fraction = (pixels as f32 / viewport).clamp(0.05, 0.9);
                    let signed = if upward { -fraction } else { fraction };
                    pinshot_capture::ax::scroll_app(target_pid, signed)
                        // Fall back to the element under the region when the app-level walk
                        // finds nothing (embedded web views, Qt apps).
                        .or_else(|_| pinshot_capture::ax::scroll_at(centre, signed))
                        .map(|_| ())
                        .map_err(|error| {
                            pinshot_core::Error::internal(format!("AX scroll: {}", error.message))
                        })?;
                    Ok(())
                },
                || std::thread::sleep(std::time::Duration::from_millis(220)),
                options,
                progress_emit,
            )
        };

        let finished = match result {
            Ok(image) => {
                diag(&format!(
                    "scroll done: {}×{} (manual={manual})",
                    image.width, image.height
                ));
                match pinshot_app::encode_png(&image) {
                    Ok(png) => {
                        // A poisoned lock used to drop the PNG silently yet still report
                        // "done", leaving the user with no long image and no explanation.
                        let mut stored = false;
                        if let Ok(mut slot) = handle.state::<AppState>().scroll_result.lock() {
                            *slot = Some(png);
                            stored = true;
                        }
                        ScrollProgressDto {
                            frames: 0,
                            height: image.height,
                            state: if stored { "done" } else { "failed" },
                            message: if stored {
                                None
                            } else {
                                Some("滚动截图结果保存失败（内部状态异常）".into())
                            },
                            skipped: 0,
                        }
                    }
                    Err(error) => {
                        eprintln!("scroll: cannot encode the result: {error}");
                        ScrollProgressDto {
                            frames: 0,
                            height: 0,
                            state: "failed",
                            message: Some(format!("{error}")),
                            skipped: 0,
                        }
                    }
                }
            }
            Err(error) => {
                diag(&format!("scroll failed: {}", error.message));
                // In manual mode the用户 is already scrolling for us, so "改用��动滚动"
                // (the stitching error's suggestion) is the wrong advice.
                let message = if manual {
                    tr(
                "没有捕获到新的内容：请在窗口里滚动页面后再按 Enter",
                "Nothing new was captured: scroll the page in that window, then press Enter",
            )
            .to_owned()
                } else {
                    error.message.clone()
                };
                eprintln!("scroll: {message}");
                ScrollProgressDto {
                    frames: 0,
                    height: 0,
                    state: if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                        "cancelled"
                    } else {
                        "failed"
                    },
                    message: Some(message),
                    skipped: 0,
                }
            }
        };
        handle
            .state::<AppState>()
            .capturing
            .store(false, Ordering::SeqCst);
        let _ = handle.emit("scroll:progress", finished);
        // Reveal the overlays unless a NEW capture took over mid-scroll. Enter/Esc set
        // `scroll_cancel` as the normal "stop and stitch" signal, so checking that flag
        // here would hide the result view on an ordinary finish ("Enter did nothing").
        let reveal = should_reveal_overlays(
            handle
                .state::<AppState>()
                .capture_serial
                .load(Ordering::SeqCst),
            scroll_serial,
        );
        if reveal {
            for (label, window) in handle.webview_windows() {
                if label.starts_with("overlay-") {
                    let _ = window.show();
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn scroll_cancel(state: State<'_, AppState>) {
    state
        .scroll_cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

/// The stitched long image, as PNG bytes.
#[tauri::command]
fn scroll_result(state: State<'_, AppState>) -> Result<tauri::ipc::Response, String> {
    let png = state
        .scroll_result
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "no scroll result yet".to_owned())?;
    Ok(tauri::ipc::Response::new(png))
}

/// Recognises the text of a history capture and copies it to the clipboard.
///
/// On-device Vision; nothing leaves the machine (`spec/01-requirements.md` OCR-1/2).
#[tauri::command]
fn ocr_capture(id: String) -> Result<String, String> {
    let store = history_store()?;
    let row = store
        .capture(&id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("no capture {id}"))?;
    let hash = row
        .blob_hash
        .ok_or_else(|| "the capture has no image".to_owned())?;
    let png = store.read_blob(&hash).map_err(|error| error.to_string())?;
    let image = pinshot_app::decode_png(&png).map_err(|error| error.to_string())?;
    let lines = pinshot_ocr::recognize(&image).map_err(|error| error.to_string())?;
    let text = pinshot_ocr::plain_text(&lines);
    if !text.is_empty() {
        pinshot_app::copy_text(&text).map_err(|error| error.to_string())?;
    }
    Ok(text)
}

#[tauri::command]
fn overlay_cancel(app: AppHandle) {
    close_overlays(&app);
}

// ── overlay lifecycle ───────────────────────────────────────────────────────

fn overlay_is_open(app: &AppHandle) -> bool {
    app.state::<AppState>().overlay_open.load(Ordering::SeqCst)
}

fn export_scale(state: &State<'_, AppState>) -> f32 {
    f32::from_bits(state.export_scale.load(Ordering::Relaxed))
}

/// Mirrors every export into the local history. Failures are logged, never fatal:
/// losing a history row must not cost the user their screenshot.
fn record(export: &pinshot_app::Export, scale: f32, kind: &str) -> Option<String> {
    if export.png.is_empty() {
        return None;
    }
    let directory = pinshot_core::config::data_dir().join("history");
    match pinshot_store::Store::open(&directory) {
        Ok(mut store) => match pinshot_app::record(&mut store, export, kind, scale) {
            Ok(id) => {
                // Build the grid thumbnail now, off the capture path. Doing it here means
                // opening 历史记录 reads small cached files instead of decoding a 5 MP PNG
                // per card (which is what made the panel crawl on a full history).
                let png = export.png.clone();
                let thumb_id = id.clone();
                std::thread::spawn(move || write_thumbnail(&thumb_id, &png));
                Some(id)
            }
            Err(error) => {
                eprintln!("history: {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("history unavailable: {error}");
            None
        }
    }
}

/// Writes the cached thumbnail for a capture, unless it is already there.
fn write_thumbnail(id: &str, png: &[u8]) {
    let path = pinshot_core::config::data_dir()
        .join("history")
        .join("thumbs")
        .join(format!("{id}.png"));
    if path.exists() {
        return;
    }
    let Ok(thumb) = pinshot_app::thumbnail(png, 320) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&path, thumb) {
        diag(&format!("history: cannot cache a thumbnail: {error}"));
    }
}

/// The active capture session, waiting up to `timeout` for the grab to finish.
///
/// `start_capture` builds the overlay windows first and grabs afterwards, so the page
/// starts loading while the pixels are still being read. Both commands below are called
/// by that page, so they have to wait rather than report "no active capture".
fn store_monitors(app: &AppHandle, monitors: &[Monitor]) {
    let state = app.state::<AppState>();
    let slot = state.monitors.lock();
    if let Ok(mut slot) = slot {
        *slot = monitors.to_vec();
    }
}

fn store_session(app: &AppHandle, session: Arc<Session>) {
    let state = app.state::<AppState>();
    let slot = state.session.lock();
    if let Ok(mut slot) = slot {
        *slot = Some(session);
    }
}

fn wait_for_session(app: &AppHandle, timeout: std::time::Duration) -> Result<Arc<Session>, String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        {
            let state = app.state::<AppState>();
            if let Ok(slot) = state.session.lock() {
                if let Some(session) = slot.as_ref() {
                    return Ok(session.clone());
                }
            }
            // A cancel clears the session; stop waiting instead of blocking the page.
            if !state.overlay_open.load(Ordering::SeqCst) {
                return Err("no active capture".to_owned());
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err("no active capture".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// Interface language for the native parts (tray menu, dialogs, notifications).
///
/// The webviews keep their own copy in `localStorage`; this one exists so the tray menu
/// and the system notifications follow the same choice.
static LANGUAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn tr(zh: &'static str, en: &'static str) -> &'static str {
    if LANGUAGE.load(Ordering::Relaxed) == 1 {
        en
    } else {
        zh
    }
}

/// A native notification, in the interface language. macOS keeps using `osascript`
/// (already permission-free and working); Windows/Linux go through the notification
/// plugin so the scroll-capture hints are not silently dropped there.
fn notify(app: &AppHandle, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    let _ = app; // only the non-macOS branch needs the handle
    #[cfg(target_os = "macos")]
    {
        // AppleScript string literals only need `"` and `\` escaped; our messages are
        // fixed catalog strings, but escape defensively in case that changes.
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!(
                    "display notification \"{}\" with title \"{}\"",
                    esc(body),
                    esc(title)
                ),
            ])
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_notification::NotificationExt;
        if let Err(error) = app.notification().builder().title(title).body(body).show() {
            eprintln!("notification: {error}");
        }
    }
}

/// Picks the language a new process should start with.
fn resolve_language(config: &Config) -> u8 {
    match config.general.language.as_str() {
        "zh" => 0,
        "en" => 1,
        _ if system_is_chinese() => 0,
        _ => 1,
    }
}

fn set_language(config: &Config) {
    LANGUAGE.store(resolve_language(config), Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
fn system_is_chinese() -> bool {
    // macOS keeps the user's ordered language list in the global preferences. `defaults`
    // is a 10 ms subprocess and this runs once per launch.
    if let Ok(output) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout).to_lowercase();
        if let Some(first) = text.split('"').nth(1) {
            return first.starts_with("zh");
        }
    }
    std::env::var("LANG")
        .map(|value| value.to_lowercase().starts_with("zh"))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn system_is_chinese() -> bool {
    pinshot_capture::win::system_is_chinese()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_is_chinese() -> bool {
    std::env::var("LANG")
        .map(|value| value.to_lowercase().starts_with("zh"))
        .unwrap_or(false)
}

fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_micros() as u64)
        .unwrap_or(0)
}

/// The app whose window sits under the middle of a capture region.
///
/// That is the app a scroll has to reach, and resolving it from the region (instead of
/// from wherever the pointer was when the capture started) is what keeps capture and
/// scrolling on the same window.
fn target_pid_for_region(monitor: &Monitor, x: i32, y: i32, width: u32, height: u32) -> i32 {
    let centre = pinshot_core::Point::new(
        monitor.bounds.x + x + width as i32 / 2,
        monitor.bounds.y + y + height as i32 / 2,
    );
    pinshot_capture::windows::under_point(centre)
        .ok()
        .flatten()
        .map(|window| window.pid)
        .unwrap_or_else(pinshot_capture::events::frontmost_pid)
}

fn label(index: usize) -> String {
    format!("overlay-{index}")
}

fn start_capture(app: AppHandle) {
    let started = std::time::Instant::now();
    let state = app.state::<AppState>();
    state.capture_started.store(now_micros(), Ordering::SeqCst);
    state.first_frame.store(false, Ordering::SeqCst);
    state.capture_serial.fetch_add(1, Ordering::SeqCst);
    // No window enumeration here: it costs 5–15 ms of every capture and only the scroll
    // capture needs a target app, which it resolves from its own region when it starts.
    if state.overlay_open.swap(true, Ordering::SeqCst) {
        // Already selecting: treat the hotkey as "cancel", like Esc.
        close_overlays(&app);
        return;
    }

    // Activation is asynchronous and slower than the capture; start it now instead of
    // when the frame is ready, so the window is key from its first visible millisecond.
    let _ = app.run_on_main_thread(activate_app);

    // Capture blocks for ~200 ms; never on the UI thread.
    std::thread::spawn(move || {
        let (config, warning) = Config::load();
        if let Some(warning) = warning {
            eprintln!("config: {warning}");
        }
        let backend = system();
        if backend.permission() == Permission::Denied {
            reset(&app);
            // The grant is cached per process: even after the user ticks the box in
            // System Settings, this running copy still reads Denied. Prompt and alert
            // once, then stay quiet so repeated hotkey presses don't stack dialogs or
            // flash the screen. The message tells them to restart, which is what
            // actually makes the grant visible to us.
            if should_prompt_screen_access() {
                // Triggers the system prompt, which is also what registers PinShot in
                // the Screen Recording pane.
                pinshot_capture::request_screen_access();
                report(
                    &app,
                    tr("PinShot 需要「屏幕录制」权限", "PinShot needs the Screen Recording permission"),
                    tr(
                        "已为你打开系统提示：允许后请彻底退出并重新打开 PinShot（授权不会立即对正在运行的进程生效）。",
                        "The system prompt is open: allow PinShot, then quit and reopen it — a running process does not see the grant immediately.",
                    ),
                );
            } else {
                diag("capture: Screen Recording still denied; already prompted, waiting for a restart");
            }
            return;
        }
        // Listing displays costs about a millisecond, grabbing costs a fifth of a second
        // and the overlay's page takes ~150 ms to load. Build the windows from the (cheap)
        // list first, then grab: the two run concurrently and `overlay_meta` /
        // `overlay_frame` wait for the session, so the click-to-pixels time is roughly
        // max(page load, grab) instead of their sum.
        let monitors = match backend.monitors() {
            Ok(monitors) => monitors,
            Err(error) => {
                reset(&app);
                report(&app, tr("无法截屏", "Capture failed"), &error.to_string());
                return;
            }
        };
        // Compare against the layout the warm windows were BUILT for before overwriting
        // it. The old order stored the current layout first, so overlays_are_warm always
        // saw an identical pair and never detected a display add/remove or resolution
        // change: stale windows were reused and the metadata used the new geometry.
        let warm = overlays_are_warm(&app, &monitors);
        store_monitors(&app, &monitors);
        {
            let handle = app.clone();
            let windows_for = monitors.clone();
            let _ = app.run_on_main_thread(move || {
                let built = std::time::Instant::now();
                if warm {
                    begin_warm_overlays(&handle, &windows_for);
                } else {
                    destroy_overlays(&handle);
                    open_overlays(&handle, &windows_for, false);
                }
                diag(&format!(
                    "capture: overlays {} in {} ms",
                    if warm { "woken" } else { "built" },
                    built.elapsed().as_millis()
                ));
            });
        }

        match Session::start_with_monitors(&*backend, config, monitors) {
            Ok(session) => {
                store_session(&app, Arc::new(session));
                diag(&format!(
                    "capture: session ready in {} ms",
                    started.elapsed().as_millis()
                ));
            }
            Err(error) => {
                // The windows are already up and waiting for the session: close them
                // instead of leaving a frozen-looking overlay with nothing behind it.
                reset(&app);
                close_overlays(&app);
                report(&app, tr("无法截屏", "Capture failed"), &error.to_string());
            }
        }
    });
}

fn open_overlays(app: &AppHandle, monitors: &[Monitor], warming: bool) {
    // Safety net: if a webview fails before calling `overlay_show`, show it anyway so
    // the failure is visible instead of leaving an invisible full-screen window. A
    // pre-warmed window has no capture behind it, so it must stay hidden.
    if !warming {
        let handle = app.clone();
        let count = monitors.len();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            if !overlay_is_open(&handle) {
                return;
            }
            for index in 0..count {
                if !overlay_is_open(&handle) {
                    return;
                }
                if handle.state::<AppState>().capturing.load(Ordering::SeqCst) {
                    continue;
                }
                if let Some(window) = handle.get_webview_window(&label(index)) {
                    if !window.is_visible().unwrap_or(true) {
                        eprintln!("overlay {index} never became ready; showing it anyway");
                        let _ = window.show();
                    }
                }
            }
        });
    }

    for (index, monitor) in monitors.iter().enumerate() {
        // Cancelling while the overlays are still being created is a real papercut: the
        // second monitor's window would appear after the user dismissed the first. Stop
        // building as soon as the session is gone. A pre-warm has no session at all.
        if !warming && !overlay_is_open(app) {
            return;
        }
        let builder =
            WebviewWindowBuilder::new(app, label(index), WebviewUrl::App("index.html".into()))
                .title("PinShot")
                .position(monitor.bounds.x as f64, monitor.bounds.y as f64)
                .inner_size(monitor.bounds.w as f64, monitor.bounds.h as f64)
                .decorations(false)
                .always_on_top(true)
                .resizable(false)
                .shadow(false)
                .skip_taskbar(true)
                .visible(false)
                .focused(true);

        match builder.build() {
            Ok(window) => {
                if !warming && !overlay_is_open(app) {
                    // The cancel landed between the check above and the build.
                    let _ = window.close();
                    return;
                }
                raise_above_menu_bar(&window);
                if index == 0 && !warming {
                    // One key registration is enough for all overlays: the event is
                    // broadcast and each window ignores it unless it has a selection.
                    // Never while warming: OVERLAY_KEYS includes bare Enter/Escape and
                    // registering those with no capture up would swallow them system-wide.
                    register_overlay_keys(app);
                }
            }
            Err(error) => {
                eprintln!("cannot open the overlay for display {index}: {error}");
                reset(app);
                return;
            }
        }
    }
}

/// True when every display already has a hidden overlay whose page is loaded and whose
/// geometry matches the current layout. Only then can a capture skip the window build
/// (~50 ms) and the page load (~90 ms).
fn overlays_are_warm(app: &AppHandle, monitors: &[Monitor]) -> bool {
    let state = app.state::<AppState>();
    let stored = match state.monitors.lock() {
        Ok(stored) => stored.clone(),
        Err(_) => return false,
    };
    if stored.len() != monitors.len() || monitors.is_empty() {
        return false;
    }
    let same_layout = stored.iter().zip(monitors).all(|(was, now)| {
        was.bounds == now.bounds
            && was.pixel_width == now.pixel_width
            && was.pixel_height == now.pixel_height
            && (was.scale - now.scale).abs() < 0.001
    });
    // A removed display leaves an extra warm window behind; require an exact count so a
    // 2→1 monitor change rebuilds instead of reusing the stale second window (whose
    // overlay_meta would then be out of range).
    let window_count = app
        .webview_windows()
        .keys()
        .filter(|name| name.starts_with("overlay-"))
        .count();
    same_layout
        && window_count == monitors.len()
        && (0..monitors.len()).all(|index| app.get_webview_window(&label(index)).is_some())
}

/// Tells the pre-created overlays that a capture has started: the pages are already
/// loaded, so this only refreshes the geometry and asks them to boot a fresh capture.
fn begin_warm_overlays(app: &AppHandle, monitors: &[Monitor]) {
    let started = std::time::Instant::now();
    let serial = app
        .state::<AppState>()
        .capture_serial
        .load(Ordering::SeqCst);
    for (index, _monitor) in monitors.iter().enumerate() {
        let Some(window) = app.get_webview_window(&label(index)) else {
            continue;
        };
        // `overlays_are_warm` has already compared the geometry with this capture's
        // display list, so re-applying frame and level here is pure overhead. It is not
        // free overhead either: after a long idle every AppKit call has to wake the
        // window server and the cold web content process, and the full set of calls
        // cost ~270 ms before the page even heard about the capture.
        // Never let the previous capture stay clickable while this one boots: after the
        // Mac sleeps the surface can come back without a fresh paint.
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        }
        // Tell the page first: it starts decoding the new frame while the grab below is
        // still running.
        let _ = window.emit("overlay:begin", serial);
    }
    diag(&format!(
        "capture: overlays told in {} ms",
        started.elapsed().as_millis()
    ));
    // After the page, never before: registering four system-wide hotkeys is a main
    // thread round trip to the window server and used to delay every capture by ~250 ms
    // before the overlay even knew it had a frame to draw.
    if !monitors.is_empty() {
        register_overlay_keys(app);
    }
    diag("capture: warm overlays woken");
}

/// Builds the overlay windows ahead of time so their pages are loaded before the first
/// capture. They stay hidden until a capture asks for one.
fn prewarm_overlays(app: &AppHandle) {
    let backend = system();
    let monitors = match backend.monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            diag(&format!("prewarm: no displays: {}", error.message));
            return;
        }
    };
    if overlay_is_open(app) {
        return;
    }
    store_monitors(app, &monitors);
    let handle = app.clone();
    let for_windows = monitors.clone();
    let _ = app.run_on_main_thread(move || {
        // The user may have pressed F1 while the warm-up was still starting: that
        // capture owns the windows now.
        if overlay_is_open(&handle) {
            return;
        }
        if handle
            .webview_windows()
            .keys()
            .any(|label| label.starts_with("overlay-"))
        {
            return;
        }
        let built = std::time::Instant::now();
        open_overlays(&handle, &for_windows, true);
        diag(&format!(
            "prewarm: overlays built in {} ms",
            built.elapsed().as_millis()
        ));
    });
}

/// Gives the warm overlays back after a minute of no captures, unless
/// `general.keep_warm` is on.
///
/// One timer per close is harmless: the serial check makes every older timer a no-op as
/// soon as a new capture starts, and the flag check covers the user turning the setting
/// back on while a timer is already asleep.
fn schedule_idle_release(app: &AppHandle) {
    let serial = app
        .state::<AppState>()
        .capture_serial
        .load(Ordering::SeqCst);
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        {
            let state = handle.state::<AppState>();
            if state.keep_warm.load(Ordering::SeqCst)
                || state.capture_serial.load(Ordering::SeqCst) != serial
                || overlay_is_open(&handle)
            {
                return;
            }
        }
        let inner = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if inner.state::<AppState>().keep_warm.load(Ordering::SeqCst) {
                return;
            }
            destroy_overlays(&inner);
            diag("prewarm: idle, overlays released");
        });
    });
}

/// Keeps the window server's cached display surface alive across long idle spells.
///
/// Measured on macOS: after ~150 s without a grab, the first capture's screen grab
/// grows from ~120 ms to ~230 ms and `RegisterEventHotKey` from ~40 ms to ~150 ms,
/// so the first screenshot after a break lands in ~310 ms instead of ~130 ms. The
/// one-off grab at launch cannot hold that for a whole session. A throwaway grab
/// every 8 s keeps the surface ready; the bytes are discarded immediately —
/// nothing is stored, saved or shown. Active captures already keep the backend
/// warm, so the pulse skips them. `keep_warm` off means the user traded the
/// first-capture latency for battery, so honour that here too.
fn keep_warm_pulse(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        // A one-off jitter keeps the pulse from landing exactly on a hotkey press.
        // Two concurrent grabs were measured ~70 ms slower than either alone, so a
        // pulse that always fired on a round multiple of the interval could fight
        // a scripted (or habitual) capture at the same instant.
        let jitter = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_millis() as u64 % 4000)
            .unwrap_or(0);
        std::thread::sleep(std::time::Duration::from_millis(jitter));
        loop {
            std::thread::sleep(std::time::Duration::from_secs(8));
            {
                let state = handle.state::<AppState>();
                if !state.keep_warm.load(Ordering::SeqCst) || overlay_is_open(&handle) {
                    continue;
                }
            }
            let started = std::time::Instant::now();
            let backend = system();
            if let Ok(monitors) = backend.monitors() {
                if let Some(monitor) = monitors.first() {
                    let _ = backend.grab(monitor);
                }
            }
            let elapsed = started.elapsed().as_millis();
            if elapsed > 250 {
                diag(&format!("keep-warm: grab took {elapsed} ms"));
            }
        }
    });
}

/// Whether a settings/history window is up, which needs a normal (Dock) app.
fn aux_window_visible(app: &AppHandle) -> bool {
    ["settings", "history"].iter().any(|name| {
        app.get_webview_window(name)
            .map(|window| window.is_visible().unwrap_or(false))
            .unwrap_or(false)
    })
}

/// Whether a finished scroll should re-show the overlays (and with them the long-image
/// result view). Only a NEW capture — which bumps `capture_serial` — must keep them hidden;
/// the `scroll_cancel` flag is set by Enter too, so it cannot distinguish the two.
fn should_reveal_overlays(current_serial: u64, scroll_serial: u64) -> bool {
    current_serial == scroll_serial
}

fn close_overlays(app: &AppHandle) {
    // A capture can be cancelled while a scroll capture is still stitching in the
    // background. Tell that thread to stop and clear the flag immediately; otherwise it
    // finishes against a session that was already reset, re-shows the hidden overlays as a
    // ghost, and suppresses the next capture's overlay (which waits on `capturing`).
    if app
        .state::<AppState>()
        .capturing
        .swap(false, Ordering::SeqCst)
    {
        app.state::<AppState>()
            .scroll_cancel
            .store(true, Ordering::SeqCst);
        diag("overlay: scroll capture cancelled by a new capture");
    }
    for (label, window) in app.webview_windows() {
        if label.starts_with("overlay-") {
            // The page clears its scene on this event, so the next capture starts from a
            // blank overlay instead of the last screenshot plus its annotations.
            let _ = window.hide();
            // The serial lets the page ignore a late end event from a capture the user
            // already restarted (and would otherwise wipe the new one).
            let serial = app
                .state::<AppState>()
                .capture_serial
                .load(Ordering::SeqCst);
            let _ = window.emit("overlay:end", serial);
        }
    }
    unregister_overlay_keys(app);
    rest_activation_policy(app);
    reset(app);
    schedule_idle_release(app);

    // A window can be mid-build when the cancel arrives (the builder call is between the
    // `overlay_is_open` check and the actual creation), so sweep once more after the
    // dust settles. `reset` already cleared the flag, so nothing here can race a fresh
    // capture: a new session sets the flag again and the sweep stops touching it.
    let handle = app.clone();
    std::thread::spawn(move || {
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(120));
            if overlay_is_open(&handle) {
                return;
            }
            for (label, window) in handle.webview_windows() {
                if label.starts_with("overlay-") {
                    // Hide, never close: the page stays loaded for the next capture.
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    }
                }
            }
        }
    });
}

/// Destroys the overlays outright: used when the display layout changed and the
/// pre-warmed windows no longer match.
fn destroy_overlays(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("overlay-") {
            let _ = window.close();
        }
    }
}

/// Keys the overlay needs while it is modal.
///
/// They are registered globally rather than relying on the webview having keyboard
/// focus: a borderless window can be key and still not route key events into the
/// page, and a screenshot tool that ignores `Enter` is useless. Registering them
/// only while the overlay is open keeps the rest of the system untouched.
const OVERLAY_KEYS: [&str; 4] = [
    "Enter",
    "Escape",
    "CommandOrControl+S",
    "CommandOrControl+C",
];

fn register_overlay_keys(app: &AppHandle) {
    let started = std::time::Instant::now();
    diag("keys: overlay keys registered");
    for key in OVERLAY_KEYS {
        // One `RegisterEventHotKey` round trip is the expensive part after a long
        // idle (~250 ms); the other three are instant.
        if let Err(error) = app.global_shortcut().register(key) {
            eprintln!("cannot register the overlay key {key}: {error}");
        }
    }
    diag(&format!(
        "keys: overlay keys ready in {} ms",
        started.elapsed().as_millis()
    ));
}

fn unregister_overlay_keys(app: &AppHandle) {
    diag("keys: overlay keys released");
    for key in OVERLAY_KEYS {
        let _ = app.global_shortcut().unregister(key);
    }
}

/// Which overlay the pointer is over, if any.
fn overlay_under_cursor(app: &AppHandle) -> Option<usize> {
    let monitors = app.state::<AppState>().monitors.lock().ok()?.clone();
    let cursor = pinshot_capture::system().cursor();
    // With a gap between displays the cursor can be in no monitor. Returning None made
    // the frontend skip its index filter, so every overlay committed and the exports
    // raced for one filename. Fall back to the primary display instead.
    if let Some(cursor) = cursor {
        if let Some(index) = monitors.iter().position(|m| m.bounds.contains(cursor)) {
            return Some(index);
        }
    }
    monitors
        .iter()
        .position(|m| m.primary)
        .or_else(|| (!monitors.is_empty()).then_some(0))
}

/// Event payload for `overlay:key`.
///
/// The target index travels in the payload instead of relying on window-scoped
/// emission: Tauri delivers a `window.emit` to every webview of the app in practice,
/// so the receiver filters by index. Broadcasting one event and discarding it in the
/// other windows is also the only way to keep a single source of truth for "which
/// display is the user looking at".
#[derive(Clone, serde::Serialize)]
struct OverlayKey {
    action: String,
    target: Option<usize>,
}

/// What a registered global shortcut does while the overlay is closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HotkeyAction {
    Capture,
    /// Pin (or re-show) the most recent capture.
    Pin,
}

impl AppState {
    /// Which action a fired shortcut belongs to, if it is one we registered.
    ///
    /// Matching on key + modifiers (not the whole `Shortcut`, which carries a numeric id)
    /// means a rebind in the settings window works without restarting for the lookup.
    fn hotkey_action(&self, shortcut: &Shortcut) -> Option<HotkeyAction> {
        let hotkeys = self.hotkeys.lock().ok()?;
        hotkeys
            .iter()
            .find(|(key, mods, _)| *key == shortcut.key && *mods == shortcut.mods)
            .map(|(_, _, action)| *action)
    }

    fn remember_hotkey(&self, shortcut: &Shortcut, action: HotkeyAction) {
        if let Ok(mut hotkeys) = self.hotkeys.lock() {
            hotkeys.retain(|(key, mods, _)| *key != shortcut.key || *mods != shortcut.mods);
            hotkeys.push((shortcut.key, shortcut.mods, action));
        }
    }
}

/// Maps a fired shortcut to the action the overlay understands.
fn overlay_action(shortcut: &Shortcut) -> Option<&'static str> {
    let unmodified = shortcut.mods.is_empty();
    match (shortcut.key, unmodified) {
        (Code::Enter, true) => Some("enter"),
        (Code::Escape, true) => Some("escape"),
        (Code::KeyS, false) if shortcut.mods.contains(Modifiers::SUPER) => Some("save"),
        (Code::KeyC, false) if shortcut.mods.contains(Modifiers::SUPER) => Some("copy"),
        _ => None,
    }
}

fn reset(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.overlay_open.store(false, Ordering::SeqCst);
    let session_slot = state.session.lock();
    if let Ok(mut slot) = session_slot {
        *slot = None;
    }
    // The monitor list is deliberately kept: it describes the layout the (still warm)
    // overlay windows were built for, and `overlays_are_warm` compares against it.
}

fn report(_app: &AppHandle, title: &str, message: &str) {
    eprintln!("{title}: {message}");
    #[cfg(target_os = "macos")]
    {
        // A native alert without pulling in a dialog plugin for one call.
        let script = format!(
            "display alert \"{}\" message \"{}\" as warning",
            escape(title),
            escape(message)
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        pinshot_capture::win::message_box(title, message);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (title, message);
    }
}

/// Diagnostics go to a file as well as stderr: a bundle launched through
/// LaunchServices has no terminal attached.
fn diag(message: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let since = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    eprintln!("[{since:>6} ms] {message}");
    use std::io::Write;
    let path = diag_path();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{since:>6} ms] {message}");
    }
}

/// `/tmp` on macOS (where the notes and scripts look for it), the per-user temp
/// directory on Windows.
fn diag_path() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        std::path::PathBuf::from("/tmp/pinshot-diag.log")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir().join("pinshot-diag.log")
    }
}

/// Logs what a secondary window is doing, for `--window-selftest`.
fn report_window(app: &AppHandle, label: &str, phase: &str) {
    match app.get_webview_window(label) {
        Some(window) => {
            let visible = window.is_visible().unwrap_or(false);
            let focused = window.is_focused().unwrap_or(false);
            diag(&format!(
                "window-selftest: {phase} {label} visible={visible} focused={focused}"
            ));
        }
        None => diag(&format!("window-selftest: {phase} {label} missing")),
    }
}

#[cfg(target_os = "macos")]
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

// ── setup ───────────────────────────────────────────────────────────────────

/// Keeps the process out of App Nap.
///
/// A screenshot tool has to answer the global shortcut immediately; App Nap throttles a
/// process that has been quiet for a while, which is exactly the first capture after a
/// break. `UserInitiatedAllowingIdleSystemSleep` prevents the throttling but still lets
/// the Mac sleep. The token is kept for the lifetime of the process.
#[cfg(target_os = "macos")]
fn prevent_app_nap() {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use std::ffi::c_char;
    unsafe {
        let reason: *mut AnyObject = msg_send![
            objc2::class!(NSString),
            stringWithUTF8String: c"PinShot stays ready to capture".as_ptr() as *const c_char
        ];
        let process: *mut AnyObject = msg_send![objc2::class!(NSProcessInfo), processInfo];
        // 0x00EF_FFFF = NSActivityUserInitiatedAllowingIdleSystemSleep: no App Nap,
        // but the Mac (and the display) may still sleep.
        let token: *mut AnyObject = msg_send![
            process,
            beginActivityWithOptions: 0x00EF_FFFFusize,
            reason: reason
        ];
        if !token.is_null() {
            let _: *mut AnyObject = msg_send![token, retain];
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn prevent_app_nap() {}

/// Creates the tray icon, retrying if the shell is not ready yet.
///
/// At login the app can start before Explorer has created the notification area, and
/// a failing tray used to abort `setup` — the process then looked like "it never
/// started". Keeping the app alive and retrying is better than exiting silently.
fn setup_tray_or_retry(app: &AppHandle) {
    match setup_tray(app) {
        Ok(()) => return,
        Err(error) => diag(&format!("tray: cannot create the icon: {error}")),
    }
    let handle = app.clone();
    std::thread::spawn(move || {
        for delay in [800u64, 1600, 3200] {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            let nested = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                if nested.tray_by_id("main").is_some() {
                    return;
                }
                match setup_tray(&nested) {
                    Ok(()) => diag("tray: created on retry"),
                    Err(error) => diag(&format!("tray: retry failed: {error}")),
                }
            });
        }
    });
}

/// The tray menu, rebuilt whenever the interface language changes.
fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    // The shortcut goes through the accelerator, never through the label: a `\t` in the
    // title makes AppKit measure the item one character wide and the Chinese text wraps
    // into a vertical column.
    let capture = MenuItem::with_id(app, "capture", tr("截图", "Capture"), true, Some("F1"))?;
    let pin = MenuItem::with_id(
        app,
        "pin",
        tr("贴出最近一次截图", "Pin the latest capture"),
        true,
        None::<&str>,
    )?;
    let restore = MenuItem::with_id(
        app,
        "restore",
        tr("恢复上一张贴图", "Restore the previous pin"),
        true,
        None::<&str>,
    )?;
    let permission = MenuItem::with_id(
        app,
        "permission",
        tr(
            "检查「屏幕录制 / 辅助功能」权限",
            "Check Screen Recording / Accessibility",
        ),
        true,
        None::<&str>,
    )?;
    let history = MenuItem::with_id(
        app,
        "history",
        tr("历史记录", "History"),
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", tr("设置", "Settings"), true, None::<&str>)?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        tr("退出 PinShot", "Quit PinShot"),
        true,
        None::<&str>,
    )?;
    Menu::with_items(
        app,
        &[
            &capture,
            &pin,
            &restore,
            &permission,
            &history,
            &settings,
            &quit,
        ],
    )
}

/// Swaps the tray's menu and tooltip for the current language, without a restart.
fn refresh_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    match tray_menu(app) {
        Ok(menu) => {
            if let Err(error) = tray.set_menu(Some(menu)) {
                diag(&format!("tray: cannot replace the menu: {error}"));
            }
        }
        Err(error) => diag(&format!("tray: cannot rebuild the menu: {error}")),
    }
    let _ = tray.set_tooltip(Some(tr(
        "PinShot · 按 F1 截图",
        "PinShot · press F1 to capture",
    )));
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = tray_menu(app)?;
    // A macOS template icon is black+alpha, which disappears on the Windows tray's
    // dark background; Windows gets the coloured app icon instead.
    #[cfg(target_os = "macos")]
    let icon_bytes: &[u8] = include_bytes!("../icons/tray.png");
    #[cfg(not(target_os = "macos"))]
    let icon_bytes: &[u8] = include_bytes!("../icons/icon.png");
    let icon = tauri::image::Image::from_bytes(icon_bytes)?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(tr("PinShot · 按 F1 截图", "PinShot · press F1 to capture"))
        .menu(&menu)
        // Left click opens the menu. Jumping straight into a capture on a single click
        // was too easy to trigger by accident and hid every other action.
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "capture" => start_capture(app.clone()),
            "pin" => {
                if let Err(error) = pin_last_capture(app.clone()) {
                    report(app, tr("无法贴图", "Could not pin"), &error);
                }
            }
            "restore" => {
                if let Err(error) = pin_restore(app.clone()) {
                    report(app, tr("无法恢复贴图", "Could not restore the pin"), &error);
                }
            }
            "permission" => {
                // Both grants matter: screen recording for the pixels, accessibility for
                // the menu-bar window walk. Ask for the missing one instead of guessing.
                let screen = pinshot_capture::system().permission() == Permission::Granted;
                let ax = pinshot_capture::events::ax_trusted(false);
                diag(&format!("permission check: screen={screen} ax={ax}"));
                match (screen, ax) {
                    (true, true) => report(
                        app,
                        tr("权限正常", "Permissions are fine"),
                        tr(
                            "「屏幕录制」和「辅助功能」都已授权，截图和滚动截屏可以正常使用。",
                            "Screen Recording and Accessibility are both granted; capture and scrolling capture work.",
                        ),
                    ),
                    (false, _) => {
                        pinshot_capture::request_screen_access();
                        report(
                            app,
                            tr("缺少「屏幕录制」权限", "Screen Recording is missing"),
                            tr(
                                "已在系统设置里打开对应位置：请勾选 PinShot，然后重启 PinShot。",
                                "The right pane is open in System Settings: tick PinShot, then restart it.",
                            ),
                        );
                    }
                    (true, false) => {
                        pinshot_capture::events::request_ax_trust();
                        report(
                            app,
                            tr("缺少「辅助功能」权限", "Accessibility is missing"),
                            tr(
                                "滚动截屏需要它在「系统设置 → 隐私与安全性 → 辅助功能」里被勾选。",
                                "Scrolling capture needs it ticked in System Settings → Privacy & Security → Accessibility.",
                            ),
                        );
                    }
                }
            }
            "history" => {
                if let Err(error) = open_window(app.clone(), "history".into()) {
                    report(app, tr("无法打开历史", "Could not open History"), &error);
                }
            }
            "settings" => {
                if let Err(error) = open_window(app.clone(), "settings".into()) {
                    report(app, tr("无法打开设置", "Could not open Settings"), &error);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

/// Drops every capture/paste shortcut. Used while the settings window is recording a new
/// combination: a live global shortcut would swallow the very keystroke being recorded.
fn unregister_hotkeys(app: &AppHandle) {
    let state = app.state::<AppState>();
    let entries = state
        .hotkeys
        .lock()
        .map(|mut hotkeys| std::mem::take(&mut *hotkeys))
        .unwrap_or_default();
    for (key, mods, _) in entries {
        let _ = app
            .global_shortcut()
            .unregister(Shortcut::new(Some(mods), key));
    }
    diag("keys: capture/paste shortcuts released");
}

#[tauri::command]
fn hotkeys_suspend(app: AppHandle) {
    unregister_hotkeys(&app);
}

#[tauri::command]
fn hotkeys_resume(app: AppHandle) {
    register_hotkey(&app);
}

fn register_hotkey(app: &AppHandle) {
    use std::str::FromStr;
    // Re-registering after a change must not fail with "already registered".
    unregister_hotkeys(app);
    let (config, _) = Config::load();
    let preferred = config.general.hotkey_snip.clone();
    // The configured shortcut first, then the historical defaults: F1 is what muscle
    // memory has, but macOS sometimes reserves it for brightness.
    let candidates = [preferred.as_str(), "F1", "CommandOrControl+Shift+1"];
    for (index, text) in candidates.iter().enumerate() {
        let Ok(shortcut) = Shortcut::from_str(text) else {
            eprintln!("hotkey: cannot parse {text:?}");
            continue;
        };
        match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                app.state::<AppState>()
                    .remember_hotkey(&shortcut, HotkeyAction::Capture);
                let note = if index == 0 {
                    ""
                } else {
                    tr(
                        "（配置里的快捷键被占用）",
                        " (the configured shortcut is taken)",
                    )
                };
                diag(&format!("hotkey: capture = {text} {note}"));
                break;
            }
            Err(error) => eprintln!("cannot register {text}: {error}"),
        }
    }

    // The paste hotkey was only ever a settings field; register it for real.
    match Shortcut::from_str(&config.general.hotkey_paste) {
        Ok(shortcut) => match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                app.state::<AppState>()
                    .remember_hotkey(&shortcut, HotkeyAction::Pin);
                diag(&format!("hotkey: pin = {}", config.general.hotkey_paste));
            }
            Err(error) => {
                eprintln!("cannot register pin hotkey: {error}");
                report(
                    app,
                    tr("「贴图」快捷键被占用", "The pin shortcut is taken"),
                    &format!(
                        "{}{}",
                        config.general.hotkey_paste,
                        tr(
                            " 已被别的软件占用，可在设置里换一个。",
                            " is used by another app — pick a different one in Settings.",
                        ),
                    ),
                );
            }
        },
        Err(_) => eprintln!("hotkey: cannot parse {:?}", config.general.hotkey_paste),
    }
}

fn main() {
    tauri::Builder::default()
        // First plugin on purpose: a second launch must exit before it registers
        // shortcuts, builds a tray icon or opens overlay windows.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            diag("single-instance: another launch ignored");
            // Give the launch a visible result: this app has no main window, and a
            // silently disappearing double-click looks like a broken install.
            let _ = open_window(app.clone(), "settings".into());
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let overlay_open = app
                        .state::<AppState>()
                        .overlay_open
                        .load(std::sync::atomic::Ordering::SeqCst);
                    // A capture/paste shortcut keeps its job even while the overlay is up:
                    // F1 there means "never mind", which is what starting a capture does
                    // when one is already running.
                    match app.state::<AppState>().hotkey_action(shortcut) {
                        Some(HotkeyAction::Capture) => {
                            // The log timestamps are enough to tell a late shortcut
                            // delivery (throttled app) from a slow capture.
                            diag("hotkey: capture pressed");
                            start_capture(app.clone());
                            return;
                        }
                        Some(HotkeyAction::Pin) => {
                            // While snipping, the pin key means "pin this capture" (what
                            // Snipaste does), not "pin the previous one and ignore the
                            // current selection". During a scroll capture the overlay is
                            // hidden and has nothing to save, so that case falls through.
                            let scrolling = app
                                .state::<AppState>()
                                .capturing
                                .load(std::sync::atomic::Ordering::SeqCst);
                            if overlay_open && !scrolling {
                                let _ = app.emit(
                                    "overlay:key",
                                    OverlayKey {
                                        action: "pin".to_owned(),
                                        target: overlay_under_cursor(app),
                                    },
                                );
                            } else if let Err(error) = pin_last_capture(app.clone()) {
                                report(app, tr("无法贴图", "Could not pin"), &error);
                            }
                            return;
                        }
                        None => {}
                    }
                    if overlay_open {
                        if let Some(action) = overlay_action(shortcut) {
                            // Only the display under the pointer commits: broadcasting
                            // would capture every screen at once and the two exports
                            // race for the same filename.
                            let _ = app.emit(
                                "overlay:key",
                                OverlayKey {
                                    action: action.to_owned(),
                                    target: overlay_under_cursor(app),
                                },
                            );
                        }
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            overlay_meta,
            overlay_frame,
            overlay_idle,
            overlay_show,
            overlay_error,
            pin_last_capture,
            pin_image,
            pin_close,
            pin_restore,
            history_list,
            history_thumb,
            history_copy,
            history_delete,
            history_clear,
            history_favorite,
            history_prune,
            ocr_capture,
            settings_load,
            settings_save,
            hotkeys_suspend,
            hotkeys_resume,
            open_window,
            scroll_start,
            scroll_cancel,
            scroll_result,
            capture_commit,
            set_export_scale,
            set_pin_origin,
            copy_png,
            save_png,
            pin_png,
            pin_copy,
            overlay_cancel
        ])
        .setup(|app| {
            // Accessory at rest, and for the capture overlay too: an accessory app can
            // still become active and own the keyboard, and staying accessory avoids the
            // menu-bar swap (and its ~150 ms activation delay) on every capture. Only the
            // settings/history windows switch to Regular.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if std::env::args().any(|arg| arg == "--autostart") {
                diag("autostart: launched by the system at login");
            }
            let keep_warm = {
                let (config, _) = Config::load();
                set_language(&config);
                app.state::<AppState>()
                    .keep_warm
                    .store(config.general.keep_warm, Ordering::SeqCst);
                config.general.keep_warm
            };
            prevent_app_nap();
            setup_tray_or_retry(app.handle());
            register_hotkey(app.handle());
            {
                let (config, _) = Config::load();
                spawn_launch_at_login(config.general.launch_at_login);
            }
            // First grab of the session pays for CoreGraphics/display setup. Pay it at
            // launch instead of when the user clicks the tray icon.
            let warm_handle = app.handle().clone();
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                let backend = system();
                if let Ok(monitors) = backend.monitors() {
                    if let Some(monitor) = monitors.first() {
                        let _ = backend.grab(monitor);
                    }
                }
                diag(&format!(
                    "warm-up: capture backend ready in {} ms",
                    started.elapsed().as_millis()
                ));
                // The overlay page takes ~90 ms to load; load it now, hidden, so the
                // first click on F1 does not pay for it.
                std::thread::sleep(std::time::Duration::from_millis(400));
                prewarm_overlays(&warm_handle);
                // Hold the capture backend warm for the whole session, not just the
                // launch: the first screenshot after a long break otherwise pays a
                // ~180 ms window-server cold-start penalty. The pulse checks
                // `keep_warm` itself, so a runtime toggle is honoured too.
                keep_warm_pulse(&warm_handle);
                if !keep_warm {
                    schedule_idle_release(&warm_handle);
                }
            });
            selftest::install(app.handle());

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build PinShot")
        .run(|_app, event| {
            // Closing the overlay must not quit: PinShot lives in the tray.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_reveals_overlays_on_normal_finish_but_not_on_takeover() {
        // Enter sets scroll_cancel=true but leaves the serial alone: the result view must
        // reappear. A hotkey starting a fresh capture bumps the serial: the old overlays
        // must stay hidden so they don't cover the new capture.
        assert!(should_reveal_overlays(7, 7));
        assert!(!should_reveal_overlays(8, 7));
    }

    #[test]
    fn screen_access_is_prompted_once_per_process() {
        SCREEN_ACCESS_PROMPTED.store(false, Ordering::SeqCst);
        assert!(should_prompt_screen_access());
        assert!(!should_prompt_screen_access());
        assert!(!should_prompt_screen_access());
        // Leave the flag clean for any other test in this binary.
        SCREEN_ACCESS_PROMPTED.store(false, Ordering::SeqCst);
    }
}
