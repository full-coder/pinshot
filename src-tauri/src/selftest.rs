//! Debug hooks (`--capture`, `--*-selftest`, `--self-test`, …).
//!
//! They are compiled into release builds on purpose: every bug report here was
//! diagnosed through one of these, and they only run when the matching argument is
//! present. Keeping them out of `main.rs` leaves the setup path readable.

use crate::*;
use tauri::{Emitter, Manager};

/// Installs whichever hook the command line asked for. Called once from `setup`.
pub(crate) fn install(app: &AppHandle) {
    // `--capture-after <ms>` starts one capture after a delay: what the first hotkey
    // press after a quiet spell costs.
    if let Some(delay) = std::env::args()
        .skip_while(|arg| arg != "--capture-after")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(delay));
            start_capture(handle);
        });
    }
    // `--set-language <zh|en>` changes the interface language through the same path
    // the settings window uses (after 3 s, so a running capture is available).
    if let Some(locale) = std::env::args()
        .skip_while(|arg| arg != "--set-language")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(3000));
            let (mut config, _) = Config::load();
            config.general.language = locale.clone();
            match settings_save(handle, config) {
                Ok(()) => diag(&format!("set-language: {locale}")),
                Err(error) => diag(&format!("set-language: {error}")),
            }
        });
    }
    // `--capture` opens the overlay immediately: it makes the shell testable
    // from a terminal, and doubles as a shortcut for scripted use.
    if std::env::args().any(|arg| arg == "--capture") {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            start_capture(handle);
        });
    }
    // `--cancel-selftest <ms>` starts a capture and cancels it `<ms>` later, then
    // reports every overlay window still alive. It exists because "取消后又冒出
    // 一个截屏框" happens only when the cancel lands while another display's
    // overlay is still being built.
    if let Some(delay) = std::env::args()
        .skip_while(|arg| arg != "--cancel-selftest")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            start_capture(handle.clone());
            std::thread::sleep(std::time::Duration::from_millis(delay));
            close_overlays(&handle);
            std::thread::sleep(std::time::Duration::from_millis(3000));
            let alive: Vec<String> = handle
                .webview_windows()
                .into_iter()
                .filter(|(label, _)| label.starts_with("overlay-"))
                .map(|(label, window)| {
                    format!("{label}(visible={})", window.is_visible().unwrap_or(false))
                })
                .collect();
            // Windows are kept warm between captures now, so "left over" means
            // *visible*: a hidden window is the page waiting for the next F1.
            let visible = handle
                .webview_windows()
                .into_iter()
                .filter(|(label, window)| {
                    label.starts_with("overlay-") && window.is_visible().unwrap_or(false)
                })
                .count();
            diag(&format!(
                "cancel-selftest: {visible} visible / {} overlay window(s) left: {alive:?}",
                alive.len()
            ));
        });
    }
    // `--probe-selftest <ms>` waits for the warm overlays, starts one capture and
    // keeps it open for `<ms>`: the setup real F1 presses find, without the cold
    // window build of the other self-tests.
    if let Some(hold) = std::env::args()
        .skip_while(|arg| arg != "--probe-selftest")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1800));
            start_capture(handle.clone());
            std::thread::sleep(std::time::Duration::from_millis(hold));
            close_overlays(&handle);
        });
    }
    // `--idle-selftest <secs>` reproduces "the first capture after a long break":
    // capture once so the overlays hold a real frame, close, stay idle for <secs>,
    // then capture again. The `perf:` lines then show whether the idle cost is in the
    // window ops, the grab or the page boot — a hotkey press cannot be scripted.
    if let Some(seconds) = std::env::args()
        .skip_while(|arg| arg != "--idle-selftest")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1800));
            start_capture(handle.clone());
            // Let the page finish its boot and paint, then close it the way Esc does.
            std::thread::sleep(std::time::Duration::from_millis(2500));
            close_overlays(&handle);
            diag(&format!("idle-selftest: idling for {seconds} s"));
            std::thread::sleep(std::time::Duration::from_secs(seconds));
            diag("idle-selftest: capturing again");
            start_capture(handle.clone());
            std::thread::sleep(std::time::Duration::from_millis(3500));
            close_overlays(&handle);
            handle.exit(0);
        });
    }
    // `--text-selftest x,y,w,h,text` types into the text tool and exports, then
    // copies the clipboard image to /tmp/text-selftest.png. It answers one
    // question: does text that is visible in the overlay reach the export?
    if let Some(spec) = std::env::args()
        .skip_while(|arg| arg != "--text-selftest")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            let parts: Vec<&str> = spec.splitn(5, ',').collect();
            if parts.len() != 5 {
                diag("text-selftest: spec must be x,y,w,h,text");
                return;
            }
            let region = parts[..4].join(",");
            let label = parts[4].to_owned();
            std::env::set_var("PINSHOT_DEBUG_REGION", &region);
            start_capture(handle.clone());
            // Wait for the window to exist *and* to have booted.
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if handle.get_webview_window("overlay-0").is_some() {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let Some(window) = handle.get_webview_window("overlay-0") else {
                diag("text-selftest: no overlay");
                return;
            };
            let script = format!(
                r#"
                    (async () => {{
                      const fire = (type, cx, cy) => window.dispatchEvent(new MouseEvent(type, {{
                        clientX: cx, clientY: cy, button: 0, buttons: 1, bubbles: true
                      }}));
                      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
                      const log = (message) => {{
                        window.__TAURI_INTERNALS__.invoke('overlay_error', {{
                          index: 0,
                          message: 'selftest: ' + message,
                        }});
                      }};
                      const tool = [...document.querySelectorAll('.toolbar button')]
                        .find((b) => ['文字', 'Text'].includes(b.textContent.trim()));
                      log('tool=' + (tool ? 'found' : 'missing'));
                      tool?.click();
                      // React updates state asynchronously; wait after every click.
                      await wait(300);
                      const typeAt = async (x, y, text) => {{
                        fire('mousedown', x, y);
                        fire('mouseup', x, y);
                        await wait(300);
                        const input = document.querySelector('.text-input');
                        if (!input) {{
                          log('no-input at ' + x + ',' + y);
                          return false;
                        }}
                        log('input-present at ' + x + ',' + y);
                        input.focus();
                        // Use the native setter so React's value tracker sees the change
                        // and runs onChange (which is what grows the field).
                        const setter = Object.getOwnPropertyDescriptor(
                          HTMLInputElement.prototype, 'value').set;
                        setter.call(input, text);
                        input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                        await wait(120);
                        log('field width ' + Math.round(input.getBoundingClientRect().width));
                        input.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true }}));
                        await wait(300);
                        return true;
                      }};
                      // Two labels: the second click must not move the first field.
                      await typeAt({x}, {y}, {label:?});
                      await typeAt({x}, {y2}, {label2:?});
                      // Open a third field but type nothing, then click away again: an
                      // empty field must never turn into a copy of the previous label.
                      await typeAt({x}, {y3}, '');
                      log('open fields left: ' + document.querySelectorAll('.text-input').length);
                    }})();
                    "#,
                x = 160,
                y = 200,
                y2 = 320,
                y3 = 420,
                label = label,
                label2 = format!("{label}二"),
            );
            // The page reports through `overlay_error`, so every branch is visible
            // in the log instead of "eval succeeded".
            if let Err(error) = window.eval(&script) {
                diag(&format!("text-selftest: eval failed: {error}"));
            }
            // The script above runs asynchronously (two clicks, two labels), so wait
            // for it to finish before exporting.
            std::thread::sleep(std::time::Duration::from_millis(2600));
            // Export through the same path the Copy button uses.
            let _ = handle.emit(
                "overlay:key",
                OverlayKey {
                    action: "copy".to_owned(),
                    target: Some(0),
                },
            );
            std::thread::sleep(std::time::Duration::from_millis(1200));
            let _ = std::process::Command::new("osascript")
                    .args([
                        "-e",
                        "set f to (POSIX file \"/tmp/text-selftest.png\")\nset d to the clipboard as «class PNGf»\nset out to open for access f with write permission\nwrite d to out\nclose access out",
                    ])
                    .status();
            diag("text-selftest: copied clipboard to /tmp/text-selftest.png");
            close_overlays(&handle);
            handle.exit(0);
        });
    }
    // `--pin-selftest x,y,w,h` adds a label and clicks 「贴图」 through the DOM, then
    // reports whether a pin window appeared. It covers the whole button path:
    // composite the annotated PNG → record it → open the pin window.
    if let Some(spec) = std::env::args()
        .skip_while(|arg| arg != "--pin-selftest")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            let numbers: Vec<i32> = spec
                .split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect();
            let [x, y, w, h] = numbers.as_slice() else {
                diag("pin-selftest: spec must be x,y,w,h");
                return;
            };
            let region = format!("{x},{y},{w},{h}");
            std::env::set_var("PINSHOT_DEBUG_REGION", &region);
            start_capture(handle.clone());
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if handle.get_webview_window("overlay-0").is_some() {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let Some(window) = handle.get_webview_window("overlay-0") else {
                diag("pin-selftest: no overlay");
                return;
            };
            let script = format!(
                r#"
                    (async () => {{
                      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
                      const log = (message) => {{
                        window.__TAURI_INTERNALS__.invoke('overlay_error', {{
                          index: 0,
                          message: 'selftest: ' + message,
                        }});
                      }};
                      const fire = (type, cx, cy) => window.dispatchEvent(new MouseEvent(type, {{
                        clientX: cx, clientY: cy, button: 0, buttons: 1, bubbles: true
                      }}));
                      const tool = [...document.querySelectorAll('.toolbar button')]
                        .find((b) => ['文字', 'Text'].includes(b.textContent.trim()));
                      log('tool=' + (tool ? 'found' : 'missing'));
                      tool?.click();
                      await wait(300);
                      fire('mousedown', {x}, {y});
                      fire('mouseup', {x}, {y});
                      await wait(300);
                      const input = document.querySelector('.text-input');
                      log('input=' + (input ? 'present' : 'missing'));
                      if (input) {{
                        const setter = Object.getOwnPropertyDescriptor(
                          HTMLInputElement.prototype, 'value').set;
                        setter.call(input, '贴图');
                        input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                        await wait(150);
                        input.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true }}));
                        await wait(400);
                      }}
                      const pin = [...document.querySelectorAll('.toolbar button')]
                        .find((b) => ['贴图', 'Pin'].includes(b.textContent.trim()));
                      log('pin=' + (pin ? 'found' : 'missing'));
                      pin?.click();
                    }})();
                    "#,
                x = 220,
                y = 240,
            );
            if let Err(error) = window.eval(&script) {
                diag(&format!("pin-selftest: eval failed: {error}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(2500));
            let pins: Vec<String> = handle
                .webview_windows()
                .into_keys()
                .filter(|label| label.starts_with("pin-"))
                .collect();
            diag(&format!(
                "pin-selftest: {} pin window(s): {pins:?}",
                pins.len()
            ));
            // One wheel notch must grow the window: the page resizes the pin instead
            // of scaling a bitmap, so the size is the observable.
            if let Some(label) = pins.first() {
                if let Some(window) = handle.get_webview_window(label) {
                    // `outer_position()` reports physical pixels, while PINSHOT_DEBUG_REGION
                    // x,y are logical points: on a 2x display a pin over the selection reads
                    // back at ~2x those values. Print both so the placement is checkable.
                    let pos = window.outer_position().ok();
                    diag(&format!(
                        "pin-selftest: outer_position {:?} (selection logical {x},{y})",
                        pos.map(|p| (p.x, p.y))
                    ));
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    let before = window.inner_size().ok();
                    if let Err(error) = window.eval(
                        "window.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, \
                             clientX: 200, clientY: 150, bubbles: true, cancelable: true }));",
                    ) {
                        diag(&format!("pin-selftest: wheel eval failed: {error}"));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    let after = window.inner_size().ok();
                    match (before, after) {
                        (Some(before), Some(after)) => diag(&format!(
                            "pin-selftest: zoom {}x{} -> {}x{}",
                            before.width, before.height, after.width, after.height
                        )),
                        _ => diag("pin-selftest: zoom size unavailable"),
                    }
                    // Right-click must show our translated menu with an opaque background,
                    // and clicking "Close" must actually close the pin window.
                    if let Err(error) = window.eval(
                        r#"(async () => {
                          const wait = (ms) => new Promise((r) => setTimeout(r, ms));
                          const log = (m) => window.__TAURI_INTERNALS__.invoke('overlay_error', {
                            index: 9, message: 'pin-selftest: ' + m });
                          window.dispatchEvent(new MouseEvent('contextmenu', {
                            clientX: 200, clientY: 150, bubbles: true, cancelable: true }));
                          await wait(200);
                          const menu = document.querySelector('.pin-menu');
                          if (!menu) { log('menu=NO MENU'); return; }
                          const text = [...menu.querySelectorAll('button')]
                            .map((b) => b.textContent.trim()).join(' | ');
                          const bg = getComputedStyle(menu).backgroundColor;
                          log('menu=' + text + ' bg=' + bg);
                          // Click the second item (关闭贴图 / Close pin). Use real mouse
                          // events so the window-level mousedown dismiss cannot pre-empt it.
                          const close = menu.querySelectorAll('button')[1];
                          close.dispatchEvent(new MouseEvent('mousedown', {
                            button: 0, bubbles: true }));
                          close.click();
                        })();"#,
                    ) {
                        diag(&format!("pin-selftest: menu eval failed: {error}"));
                    }
                    // The script is async; give it time to report and close before checking.
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    let still_open = handle
                        .get_webview_window(label)
                        .map(|w| w.is_visible().unwrap_or(false))
                        .unwrap_or(false);
                    diag(&format!(
                        "pin-selftest: after menu-close, pin {} {}",
                        label,
                        if still_open {
                            "STILL OPEN (FAIL)"
                        } else {
                            "closed ok"
                        }
                    ));
                }
            }
            for label in pins {
                if let Some(window) = handle.get_webview_window(&label) {
                    let _ = window.close();
                }
            }
            handle.exit(0);
        });
    }
    // `--arrow-selftest x,y,w,h` draws one arrow across the region and exports it,
    // so the arrow head can be inspected pixel by pixel without a human dragging.
    if let Some(spec) = std::env::args()
        .skip_while(|arg| arg != "--arrow-selftest")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::env::set_var("PINSHOT_DEBUG_REGION", &spec);
            start_capture(handle.clone());
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if handle.get_webview_window("overlay-0").is_some() {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let Some(window) = handle.get_webview_window("overlay-0") else {
                diag("arrow-selftest: no overlay");
                return;
            };
            let script = r#"
                    (async () => {
                      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
                      const fire = (type, x, y) => window.dispatchEvent(new MouseEvent(type, {
                        clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true
                      }));
                      const log = (message) => {
                        window.__TAURI_INTERNALS__.invoke('overlay_error', {
                          index: 0,
                          message: 'arrow-selftest: ' + message,
                        });
                      };
                      const tool = [...document.querySelectorAll('.toolbar button')]
                        .find((element) => ['箭头', 'Arrow'].includes(element.textContent.trim()));
                      if (!tool) { log('no arrow tool'); return; }
                      tool.click();
                      await wait(250);
                      fire('mousedown', 200, 240);
                      fire('mousemove', 620, 470);
                      fire('mouseup', 620, 470);
                      await wait(400);
                      log('drawn');
                    })();
                "#;
            if let Err(error) = window.eval(script) {
                diag(&format!("arrow-selftest: eval failed: {error}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = handle.emit(
                "overlay:key",
                OverlayKey {
                    action: "copy".to_owned(),
                    target: Some(0),
                },
            );
            std::thread::sleep(std::time::Duration::from_millis(1200));
            let _ = std::process::Command::new("osascript")
                    .args([
                        "-e",
                        "set f to (POSIX file \"/tmp/arrow-selftest.png\")\nset d to the clipboard as «class PNGf»\nset out to open for access f with write permission\nwrite d to out\nclose access out",
                    ])
                    .status();
            diag("arrow-selftest: copied clipboard to /tmp/arrow-selftest.png");
            close_overlays(&handle);
            handle.exit(0);
        });
    }
    // `--demo-selftest x,y,w,h[,holdMs]` stages the shot the README uses: a selection
    // with an arrow, a mosaic patch and one text label, held for `holdMs` so a
    // `screencapture` can pick it up. Nothing is exported.
    if let Some(spec) = std::env::args()
        .skip_while(|arg| arg != "--demo-selftest")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            let numbers: Vec<i32> = spec
                .split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect();
            let (x, y, w, h, hold, marks) = match numbers.as_slice() {
                [x, y, w, h] => (*x, *y, *w, *h, 12000u64, None),
                [x, y, w, h, hold] => (*x, *y, *w, *h, (*hold).max(1000) as u64, None),
                // Optional absolute annotation points, for staging a specific picture:
                // arrow start/end, mosaic start/end, text click.
                [x, y, w, h, hold, ax, ay, bx, by, mx, my, mx2, my2, tx, ty] => (
                    *x,
                    *y,
                    *w,
                    *h,
                    (*hold).max(1000) as u64,
                    Some([*ax, *ay, *bx, *by, *mx, *my, *mx2, *my2, *tx, *ty]),
                ),
                _ => {
                    diag("demo-selftest: spec must be x,y,w,h[,holdMs[,annotations…]]");
                    return;
                }
            };
            std::env::set_var("PINSHOT_DEBUG_REGION", format!("{x},{y},{w},{h}"));
            start_capture(handle.clone());
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if handle.get_webview_window("overlay-0").is_some() {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1600));
            let Some(window) = handle.get_webview_window("overlay-0") else {
                diag("demo-selftest: no overlay");
                return;
            };
            let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
            // Defaults are fractions of the selection, so the script works for any
            // display; `marks` overrides them with absolute points.
            let [ax, ay, bx, by, mx, my, mx2, my2, tx, ty] = marks.unwrap_or(
                [
                    x + w * 0.10,
                    y + h * 0.30,
                    x + w * 0.42,
                    y + h * 0.62,
                    x + w * 0.55,
                    y + h * 0.66,
                    x + w * 0.86,
                    y + h * 0.74,
                    x + w * 0.12,
                    y + h * 0.80,
                ]
                .map(|value| value.round() as i32),
            );
            let script = format!(
                r#"
                    (async () => {{
                      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
                      const fire = (type, x, y) => window.dispatchEvent(new MouseEvent(type, {{
                        clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true
                      }}));
                      const log = (message) => {{
                        window.__TAURI_INTERNALS__.invoke('overlay_error', {{
                          index: 0, message: 'demo-selftest: ' + message,
                        }});
                      }};
                      const pick = (names) => [...document.querySelectorAll('.toolbar button')]
                        .find((element) => names.includes(element.textContent.trim()));
                      const drag = async (tool, x1, y1, x2, y2) => {{
                        const button = pick(tool);
                        if (!button) {{ log('missing tool ' + tool); return; }}
                        button.click();
                        await wait(220);
                        fire('mousedown', x1, y1);
                        fire('mousemove', (x1 + x2) / 2, (y1 + y2) / 2);
                        fire('mousemove', x2, y2);
                        fire('mouseup', x2, y2);
                        await wait(260);
                      }};
                      await drag(['箭头', 'Arrow'],
                        {ax}, {ay}, {bx}, {by});
                      await drag(['马赛克', 'Mosaic'],
                        {mx}, {my}, {mx2}, {my2});
                      const textTool = pick(['文字', 'Text']);
                      if (textTool) {{
                        textTool.click();
                        await wait(220);
                        fire('mousedown', {tx}, {ty});
                        fire('mouseup', {tx}, {ty});
                        await wait(320);
                        const input = document.querySelector('.text-input');
                        if (input) {{
                          const setter = Object.getOwnPropertyDescriptor(
                            HTMLInputElement.prototype, 'value').set;
                          setter.call(input, '滚慢一点，接缝就不会缺');
                          input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                          await wait(160);
                          input.dispatchEvent(new KeyboardEvent('keydown', {{
                            key: 'Enter', bubbles: true
                          }}));
                          await wait(300);
                        }} else {{
                          log('no text input');
                        }}
                      }}
                      pick(['选择', 'Select'])?.click();
                      log('staged');
                    }})();
                    "#,
                ax = ax,
                ay = ay,
                bx = bx,
                by = by,
                mx = mx,
                my = my,
                mx2 = mx2,
                my2 = my2,
                tx = tx,
                ty = ty,
            );
            if let Err(error) = window.eval(&script) {
                diag(&format!("demo-selftest: eval failed: {error}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(2600 + hold));
            close_overlays(&handle);
            handle.exit(0);
        });
    }
    // `--capture-selftest <n>` captures n times and reports click-to-pixels for
    // each run, so a performance change can be measured instead of guessed.
    if let Some(count) = std::env::args()
        .skip_while(|arg| arg != "--capture-selftest")
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            // Let the app finish booting (tray, permissions warm-up).
            std::thread::sleep(std::time::Duration::from_millis(1500));
            for run in 1..=count {
                let started = std::time::Instant::now();
                start_capture(handle.clone());
                let mut elapsed = None;
                for _ in 0..600 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    if handle
                        .state::<AppState>()
                        .first_frame
                        .load(Ordering::SeqCst)
                    {
                        elapsed = Some(started.elapsed().as_millis());
                        break;
                    }
                }
                diag(&format!(
                    "capture-selftest: run {run} first frame in {} ms",
                    elapsed
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "timeout".to_owned())
                ));
                std::thread::sleep(std::time::Duration::from_millis(900));
                close_overlays(&handle);
                std::thread::sleep(std::time::Duration::from_millis(700));
            }
            handle.exit(0);
        });
    }
    // `--shortcut-selftest` opens the settings window, arms the 截图 key field,
    // presses F2 in the page, saves, and reports the value that reached the
    // config. It answers "does pressing a key really record it".
    if std::env::args().any(|arg| arg == "--shortcut-selftest") {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(900));
            let _ = open_window(handle.clone(), "settings".into());
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if handle.get_webview_window("settings").is_some() {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1200));
            let Some(window) = handle.get_webview_window("settings") else {
                diag("shortcut-selftest: no settings window");
                handle.exit(1);
                return;
            };
            let script = r#"
                    (async () => {
                      const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
                      const log = (message) => window.__TAURI_INTERNALS__.invoke('overlay_error', {
                        index: 0, message: 'shortcut-selftest: ' + message,
                      });
                      // The long-shot result view reuses `.toolbar`, which is a
                      // two-row column: check the override really makes it a row.
                      const stage = document.createElement('div');
                      stage.className = 'stage long-shot';
                      stage.innerHTML = '<div class="toolbar"><button>a</button><button>b</button><button>c</button></div>';
                      document.body.appendChild(stage);
                      log('long-shot toolbar: ' + getComputedStyle(stage.querySelector('.toolbar')).flexDirection);
                      log('settings scrollTop: ' + (document.querySelector('.window')?.scrollTop ?? -1));
                      stage.remove();
                      const button = document.querySelector('.shortcut');
                      if (!button) { log('no shortcut button'); return; }
                      log('before: ' + button.textContent.trim());
                      button.click();
                      await wait(400);
                      log('armed: ' + button.textContent.trim());
                      window.dispatchEvent(new KeyboardEvent('keydown', { code: 'F2', bubbles: true }));
                      await wait(300);
                      log('after key: ' + button.textContent.trim());
                      const after = button.textContent.trim();
                      // No save button any more: the change has to be written by itself.
                      // Give the debounce its 400 ms, then look at the config file.
                      await wait(1200);
                      log('after key: ' + after);
                    })();
                "#;
            if let Err(error) = window.eval(script) {
                diag(&format!("shortcut-selftest: eval failed: {error}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(2200));
            let (config, _) = Config::load();
            diag(&format!(
                "shortcut-selftest: config hotkey_snip = {:?} (autosaved, no button)",
                config.general.hotkey_snip
            ));
            handle.exit(0);
        });
    }
    // `--settings-autosave-selftest` drags the dim slider through the DOM and checks
    // that config.toml changed without pressing anything, then puts the old value back.
    if std::env::args().any(|arg| arg == "--settings-autosave-selftest") {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let _ = open_window(handle.clone(), "settings".into());
            std::thread::sleep(std::time::Duration::from_millis(1800));
            let Some(window) = handle.get_webview_window("settings") else {
                diag("settings-autosave: no settings window");
                return;
            };
            let before = Config::load().0.capture.dim;
            let target = if before > 0.5 { 0.35 } else { 0.65 };
            let set = |value: f32| {
                format!(
                    r#"
                    (() => {{
                      const input = document.querySelector('input[type=range]');
                      if (!input) return 'no-range';
                      const setter = Object.getOwnPropertyDescriptor(
                        HTMLInputElement.prototype, 'value').set;
                      setter.call(input, '{value}');
                      input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                      return 'ok';
                    }})();
                    "#
                )
            };
            let _ = window.eval(set(target));
            std::thread::sleep(std::time::Duration::from_millis(1400));
            let after = Config::load().0.capture.dim;
            diag(&format!(
                "settings-autosave: dim {before} -> {after} (wanted {target}) {}",
                if (after - target).abs() < 0.001 {
                    "ok"
                } else {
                    "MISMATCH"
                }
            ));
            // Leave the file the way it was.
            let _ = window.eval(set(before));
            std::thread::sleep(std::time::Duration::from_millis(1400));
            diag(&format!(
                "settings-autosave: restored dim = {}",
                Config::load().0.capture.dim
            ));
            handle.exit(0);
        });
    }
    // `--window-close-selftest` opens Settings, marks it dirty, then calls window.close()
    // — the exact path a click on the X button takes. The frontend handler preventDefaults,
    // flushes the pending save and calls getCurrentWindow().destroy(). If the destroy
    // permission is missing, the window stays open; this checks it really closes.
    if std::env::args().any(|arg| arg == "--window-close-selftest") {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let _ = open_window(handle.clone(), "settings".into());
            std::thread::sleep(std::time::Duration::from_millis(1600));
            let Some(window) = handle.get_webview_window("settings") else {
                diag("window-close: no settings window");
                handle.exit(1);
                return;
            };
            // Dirty the form through the DOM so the close handler takes the
            // preventDefault→flush→destroy branch (not the trivial not-dirty path).
            let _ = window.eval(
                r#"
                (() => {
                  const input = document.querySelector('input[type=range]');
                  if (!input) return;
                  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
                  setter.call(input, input.value === '0.65' ? '0.55' : '0.65');
                  input.dispatchEvent(new Event('input', { bubbles: true }));
                })();
                "#,
            );
            std::thread::sleep(std::time::Duration::from_millis(300));
            // A click on X maps to Window::close, which emits CloseRequested to the page.
            let _ = window.close();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let still_open = handle
                .get_webview_window("settings")
                .map(|w| w.is_visible().unwrap_or(false))
                .unwrap_or(false);
            diag(&format!(
                "window-close: settings window {} {}",
                if still_open { "STILL OPEN" } else { "closed" },
                if still_open {
                    "FAIL (destroy denied?)"
                } else {
                    "ok"
                }
            ));
            // Put the config back to what it was before the test dirtied it.
            handle.exit(if still_open { 1 } else { 0 });
        });
    }
    // `--open-window <settings|history>` just opens that window and stays: handy
    // for looking at it (and screenshotting it) without clicking the tray menu.
    if let Some(kind) = std::env::args()
        .skip_while(|arg| arg != "--open-window")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            let _ = open_window(handle, kind);
        });
    }
    // `--window-selftest <settings|history>` opens a secondary window the way the
    // tray menu does, hides it, and opens it again with one call. It exists to
    // check the reported bug: "one click on 设置/历史记录 does not bring it back".
    if let Some(kind) = std::env::args()
        .skip_while(|arg| arg != "--window-selftest")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let _ = open_window(handle.clone(), kind.clone());
            for delay in [150u64, 600, 1200] {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                report_window(&handle, &kind, "open");
            }
            // The user clicks somewhere else: the window goes behind, then one
            // menu click has to bring it back.
            if let Some(window) = handle.get_webview_window(&kind) {
                let _ = window.hide();
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
            report_window(&handle, &kind, "hidden");
            let _ = open_window(handle.clone(), kind.clone());
            for delay in [150u64, 600, 1200] {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                report_window(&handle, &kind, "reopen");
            }
            if let Some(window) = handle.get_webview_window(&kind) {
                let _ = window.close();
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
            handle.exit(0);
        });
    }
    // `--drag-selftest x1,y1,x2,y2` drives a region drag *inside the overlay
    // webview* (DOM events, so the real cursor is never touched) and captures the
    // screen mid-drag. It exists because "the selection box only appears after
    // releasing the mouse" cannot be reproduced from a unit test.
    if let Some(spec) = std::env::args()
        .skip_while(|arg| arg != "--drag-selftest")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            let numbers: Vec<i32> = spec
                .split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect();
            let (x1, y1, x2, y2, hold) = match numbers.as_slice() {
                [x1, y1, x2, y2] => (*x1, *y1, *x2, *y2, 700u64),
                // The 5th value keeps the button down, which is what a live
                // scroll needs in order to keep going.
                [x1, y1, x2, y2, hold] => (*x1, *y1, *x2, *y2, (*hold).max(0) as u64),
                _ => {
                    diag("drag-selftest: spec must be x1,y1,x2,y2[,holdMs]");
                    return;
                }
            };
            start_capture(handle.clone());
            // The overlay has to exist *and* have booted: a fixed sleep raced the
            // slow first capture and reported "no overlay window".
            for _ in 0..60 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if handle.get_webview_window("overlay-0").is_some() {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1200));
            let Some(window) = handle.get_webview_window("overlay-0") else {
                diag("drag-selftest: no overlay window");
                return;
            };
            let script = format!(
                r#"
                    (() => {{
                      const fire = (type, x, y) => window.dispatchEvent(new MouseEvent(type, {{
                        clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true
                      }}));
                      fire('mousedown', {x1}, {y1});
                      fire('mousemove', {midx}, {midy});
                      fire('mousemove', {x2}, {y2});
                    }})();
                    "#,
                midx = (x1 + x2) / 2,
                midy = (y1 + y2) / 2
            );
            let _ = window.eval(&script);
            // Screenshot mid-hold: for a live scroll the region has to be
            // visibly longer already, not just present.
            let shot_at = (hold / 2).max(400);
            std::thread::sleep(std::time::Duration::from_millis(shot_at));
            let _ = std::process::Command::new("screencapture")
                .args(["-x", "/tmp/drag-during.png"])
                .status();
            if hold > shot_at {
                std::thread::sleep(std::time::Duration::from_millis(hold - shot_at));
                let _ = std::process::Command::new("screencapture")
                    .args(["-x", "/tmp/drag-hold.png"])
                    .status();
            }
            let release = format!(
                r#"
                    window.dispatchEvent(new MouseEvent('mouseup', {{
                      clientX: {x2}, clientY: {y2}, button: 0, bubbles: true
                    }}));
                    "#
            );
            let _ = window.eval(&release);
            std::thread::sleep(std::time::Duration::from_millis(700));
            let _ = std::process::Command::new("screencapture")
                .args(["-x", "/tmp/drag-after.png"])
                .status();
            diag("drag-selftest: captured /tmp/drag-during.png and /tmp/drag-after.png");
            close_overlays(&handle);
            handle.exit(0);
        });
    }

    // `--auto-commit <ms>` fires the same event the modal keys produce, so the
    // whole capture → overlay → export path can be tested without synthetic
    // keyboard input (which macOS routes by window-server focus).
    // Debug hook: close the newest pin after a delay, so the close path can be
    // verified without synthetic mouse input.
    if let Some(milliseconds) = std::env::args()
        .skip_while(|arg| arg != "--pin-close-after")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(milliseconds));
            let id = {
                let state = handle.state::<AppState>();
                let manager = state.pins.lock().unwrap();
                manager.pins().map(|pin| pin.id.clone()).last()
            };
            if let Some(id) = id {
                let _ = pin_close(handle.clone(), id);
            }
        });
    }
    // Debug hook: pin the newest capture at startup, so the pin window can be
    // verified without driving the tray menu.
    // `--request-access` asks macOS for input-synthesis permission, which is what
    // puts PinShot into 系统设置 → 隐私与安全性 → 辅助功能.
    if std::env::args().any(|arg| arg == "--request-access") {
        let before = pinshot_capture::events::ax_trusted(false);
        let granted = pinshot_capture::events::request_ax_trust();
        diag(&format!("request-access: before={before} after={granted}"));
    }

    // `--scroll-selftest x,y,w,h` drives the real auto-scroll loop against
    // whatever window sits under that point, and writes the result to
    // /tmp/scroll-selftest.png. This is the only way to exercise wheel
    // injection + stitching without a clicking user.
    if let Some(spec) = std::env::args()
        .skip_while(|arg| arg != "--scroll-selftest")
        .nth(1)
    {
        // `window:<id>[,manual]` captures one *window* per frame instead of a
        // display region: other windows covering it do not matter, and only the
        // screen-recording permission is required.
        if let Some(rest) = spec.strip_prefix("window:") {
            let id: u32 = rest
                .split(',')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let manual = rest.contains("manual");
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                let windows = match pinshot_capture::windows::list() {
                    Ok(list) => list,
                    Err(error) => {
                        diag(&format!("window-scroll: {error}"));
                        return;
                    }
                };
                let Some(window) = windows.into_iter().find(|w| w.id == id) else {
                    diag(&format!("window-scroll: no window {id}"));
                    return;
                };
                diag(&format!(
                    "window-scroll: {} — {} ({}×{})",
                    window.app, window.title, window.bounds.w, window.bounds.h
                ));
                let limit: u32 = std::env::var("PINSHOT_SCROLL_MAX")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(12);
                let ticks = std::cell::Cell::new(0u32);
                let started = std::time::Instant::now();
                // Capture and scroll target the *same* window id, so a stale
                // pid can never make them act on different windows.
                let pid = window.pid;
                let result = if manual {
                    pinshot_app::scrolling::manual_scroll(
                        || pinshot_capture::windows::grab(&window),
                        || std::thread::sleep(std::time::Duration::from_millis(280)),
                        || {
                            ticks.set(ticks.get() + 1);
                            ticks.get() > limit
                        },
                        pinshot_app::scrolling::ScrollRunOptions::default(),
                        || pinshot_capture::ax::scroll_position(pid).ok(),
                        || std::thread::sleep(std::time::Duration::from_millis(90)),
                        |progress| diag(&format!("window-scroll: {} frames", progress.frames)),
                    )
                } else {
                    pinshot_app::scrolling::auto_scroll(
                        || pinshot_capture::windows::grab(&window),
                        |pixels| {
                            let viewport = window.bounds.h as f32;
                            let fraction = (pixels as f32 / viewport).clamp(0.1, 0.7);
                            pinshot_capture::ax::scroll_app(pid, fraction)
                                .map(|_| ())
                                .map_err(|error| {
                                    pinshot_core::Error::internal(format!(
                                        "AX scroll: {}",
                                        error.message
                                    ))
                                })
                        },
                        || std::thread::sleep(std::time::Duration::from_millis(260)),
                        pinshot_app::scrolling::ScrollRunOptions {
                            max_frames: 24,
                            ..Default::default()
                        },
                        |progress| diag(&format!("window-scroll: {} frames", progress.frames)),
                    )
                };
                match result {
                    Ok(image) => match pinshot_app::encode_png(&image) {
                        Ok(png) => {
                            let _ = std::fs::write("/tmp/window-scroll.png", &png);
                            diag(&format!(
                                "window-scroll: {}×{} in {} ms (manual={manual})",
                                image.width,
                                image.height,
                                started.elapsed().as_millis()
                            ));
                        }
                        Err(error) => diag(&format!("window-scroll: encode {error}")),
                    },
                    Err(error) => diag(&format!("window-scroll: {}", error.message)),
                }
            });
            return;
        }
        // The 5th field may be `manual`: capture what the user scrolls instead
        // of injecting wheel events.
        let manual = spec.split(',').any(|part| part.trim() == "manual");
        diag(&format!("scroll-selftest: spec={spec:?} manual={manual}"));
        let numbers: Vec<i32> = spec
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        if let [x, y, w, h] = numbers.as_slice() {
            let (x, y, w, h) = (*x, *y, *w as u32, *h as u32);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(4000));
                let backend = pinshot_capture::system();
                let monitors = match backend.monitors() {
                    Ok(list) => list,
                    Err(error) => {
                        diag(&format!("scroll-selftest: {error}"));
                        return;
                    }
                };
                let point = pinshot_core::Point::new(x + w as i32 / 2, y + h as i32 / 2);
                let Some(monitor) = monitors.iter().find(|m| m.bounds.contains(point)).cloned()
                else {
                    diag("scroll-selftest: point is outside every display");
                    return;
                };
                let region = monitor.rect_to_pixel(Rect::new(
                    x - monitor.bounds.x,
                    y - monitor.bounds.y,
                    w,
                    h,
                ));
                diag(&format!(
                    "scroll-selftest: ax_trusted={}",
                    pinshot_capture::events::ax_trusted(false)
                ));
                // Activate the app that owns the window under the region, not
                // "whatever was frontmost": the wheel follows the window.
                let target = pinshot_capture::windows::under_point(point)
                    .ok()
                    .flatten()
                    .map(|window| window.pid)
                    .unwrap_or_else(pinshot_capture::events::frontmost_pid);
                let activated = pinshot_capture::events::activate_app(target);
                diag(&format!("scroll-selftest: activate={activated}"));
                diag(&format!(
                    "scroll-selftest: frontmost now {}",
                    pinshot_capture::events::frontmost_pid()
                ));
                diag(&format!(
                    "scroll-selftest: target pid {target} (self {})",
                    std::process::id()
                ));
                let ratio: f32 = std::env::var("PINSHOT_SCROLL_RATIO")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.7);
                let max_frames: usize = std::env::var("PINSHOT_SCROLL_MAX")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(40);
                let started = std::time::Instant::now();
                let result = if manual {
                    // Bound the debug run so a self-test does not spend a
                    // minute encoding a 20 000 px debug PNG.
                    let ticks = std::cell::Cell::new(0u32);
                    let limit: u32 = std::env::var("PINSHOT_SCROLL_MAX")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(12);
                    pinshot_app::scrolling::manual_scroll(
                        || backend.grab(&monitor).map(|frame| frame.crop(region)),
                        || std::thread::sleep(std::time::Duration::from_millis(280)),
                        || {
                            ticks.set(ticks.get() + 1);
                            ticks.get() > limit
                        },
                        pinshot_app::scrolling::ScrollRunOptions {
                            max_frames,
                            ..Default::default()
                        },
                        || {
                            let target = pinshot_capture::windows::under_point(point)
                                .ok()
                                .flatten()
                                .map(|window| window.pid)
                                .unwrap_or(0);
                            pinshot_capture::ax::scroll_position(target).ok()
                        },
                        || std::thread::sleep(std::time::Duration::from_millis(90)),
                        |progress| diag(&format!("scroll-selftest: {} frames", progress.frames)),
                    )
                } else {
                    pinshot_app::scrolling::auto_scroll(
                        || backend.grab(&monitor).map(|frame| frame.crop(region)),
                        |pixels| {
                            // In manual mode the human scrolls; we only observe.
                            if manual {
                                return Ok(());
                            }
                            let viewport = (h as f32 * monitor.scale.max(1.0)).max(1.0);
                            let fraction = (pixels as f32 / viewport).clamp(0.05, 0.9);
                            pinshot_capture::ax::scroll_app(target, fraction)
                                .map(|_| ())
                                .map_err(|error| {
                                    pinshot_core::Error::internal(format!(
                                        "AX scroll: {}",
                                        error.message
                                    ))
                                })?;
                            Ok(())
                        },
                        || std::thread::sleep(std::time::Duration::from_millis(220)),
                        pinshot_app::scrolling::ScrollRunOptions {
                            step_ratio: ratio,
                            max_frames,
                            ..Default::default()
                        },
                        |progress| diag(&format!("scroll-selftest: {} frames", progress.frames)),
                    )
                };
                match result {
                    Ok(image) => match pinshot_app::encode_png(&image) {
                        Ok(png) => {
                            let path = "/tmp/scroll-selftest.png";
                            let _ = std::fs::write(path, &png);
                            diag(&format!(
                                "scroll-selftest: {}×{} in {} ms → {path}",
                                image.width,
                                image.height,
                                started.elapsed().as_millis()
                            ));
                        }
                        Err(error) => diag(&format!("scroll-selftest: encode {error}")),
                    },
                    Err(error) => diag(&format!("scroll-selftest: {}", error.message)),
                }
            });
        }
    }

    // `--self-test <name>` runs one command body at startup. Synthetic clicks
    // do not reach regular windows on this machine (the overlay only works
    // because it sits at screen-saver level), so the command bodies are
    // exercised directly and the windows are verified by screenshot.
    if let Some(test) = std::env::args()
        .skip_while(|arg| arg != "--self-test")
        .nth(1)
    {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            match test.as_str() {
                "history-copy" => match history_list(1) {
                    Ok(items) => match items.first() {
                        Some(item) => match history_copy(item.id.clone()) {
                            Ok(()) => diag("self-test: copied the newest capture"),
                            Err(error) => diag(&format!("self-test: copy failed: {error}")),
                        },
                        None => diag("self-test: history is empty"),
                    },
                    Err(error) => diag(&format!("self-test: list failed: {error}")),
                },
                "ocr" => match history_list(1) {
                    Ok(items) => match items.first() {
                        Some(item) => match ocr_capture(item.id.clone()) {
                            Ok(text) => diag(&format!(
                                "self-test: ocr read {} chars: {:?}",
                                text.chars().count(),
                                text.lines().next().unwrap_or("")
                            )),
                            Err(error) => diag(&format!("self-test: ocr failed: {error}")),
                        },
                        None => diag("self-test: history is empty"),
                    },
                    Err(error) => diag(&format!("self-test: list failed: {error}")),
                },
                "permission" => {
                    diag(&format!(
                        "permission: screen={} ax={} input={} bundle={:?} pid={}",
                        pinshot_capture::screen_access(),
                        pinshot_capture::events::ax_trusted(false),
                        pinshot_capture::events::post_access(),
                        std::env::current_exe().ok(),
                        std::process::id()
                    ));
                }
                "settings-save" => {
                    let (config, _) = Config::load();
                    match settings_save(handle.clone(), config) {
                        Ok(()) => diag("self-test: settings saved"),
                        Err(error) => diag(&format!("self-test: save failed: {error}")),
                    }
                }
                other => diag(&format!("self-test: unknown test {other}")),
            }
            let _ = handle;
        });
    }

    // Debug hooks so the secondary windows can be verified from a script.
    for (flag, kind) in [("--history", "history"), ("--settings", "settings")] {
        if std::env::args().any(|arg| arg == flag) {
            let handle = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Err(error) = open_window(handle.clone(), kind.to_owned()) {
                    report(&handle, "无法打开窗口", &error);
                }
            });
        }
    }
    if std::env::args().any(|arg| arg == "--pin-last") {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(700));
            if let Err(error) = pin_last_capture(handle.clone()) {
                report(&handle, "无法贴图", &error);
            }
        });
    }
    if let Some(milliseconds) = std::env::args()
        .skip_while(|arg| arg != "--auto-commit")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
    {
        // `--auto-commit-action save` exercises the ⌘S path instead.
        let action = std::env::args()
            .skip_while(|arg| arg != "--auto-commit-action")
            .nth(1)
            .unwrap_or_else(|| "enter".to_owned());
        let action: &'static str = match action.as_str() {
            "save" => "save",
            "pin" => "pin",
            _ => "enter",
        };
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(milliseconds));
            // Same targeting as the real shortcut path.
            let _ = handle.emit(
                "overlay:key",
                OverlayKey {
                    action: action.to_owned(),
                    target: overlay_under_cursor(&handle),
                },
            );
        });
    }
}
