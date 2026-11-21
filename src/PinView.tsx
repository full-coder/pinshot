import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { t, useLocale } from "./i18n";

/** Zoom limits for a pinned image. */
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
/**
 * A pinned image window.
 *
 * Driven entirely by the mouse: drag to move, wheel to zoom, double-click (or the corner
 * button) to close. Keyboard is deliberately not required — a pin must never steal focus
 * from whatever you are typing into (`spec/06-pin-window.md` §2).
 *
 * Zooming resizes the *window* instead of scaling the image inside it: the pin then
 * behaves like a movable loupe, and the image is re-rasterised at the new size instead
 * of turning into a blurred bitmap. The point under the cursor stays under the cursor.
 */
export function PinView() {
  useLocale();
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [zoomLabel, setZoomLabel] = useState<number | null>(null);
  // Right-click menu. WebKit's default context menu is in the OS language, ignoring the
  // app's language setting, so we replace it with a small translated one.
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const dragRef = useRef<{ x: number; y: number; originX: number; originY: number } | null>(null);
  // The mousedown handler awaits two IPC calls before it can fill dragRef. A quick click
  // can release the button in that window; without this flag the late result would write
  // a non-null drag back after mouseup had cleared it, and the window would follow the
  // cursor with no button held.
  const draggingRef = useRef(false);
  const zoomRef = useRef(1);
  const baseRef = useRef<{ width: number; height: number } | null>(null);
  const cursorRef = useRef({ x: 0, y: 0 });
  const wheelRef = useRef(0);
  // A wheel tick can arrive while a previous `apply` is awaiting IPC. Reading the window
  // position before the earlier resize committed and writing back stale coordinates made
  // the image drift away from the cursor. Serialise the whole read→compute→write chain.
  const zoomQueueRef = useRef<Promise<void>>(Promise.resolve());
  const frameRef = useRef<number | null>(null);
  const badgeRef = useRef<number | null>(null);
  const label = getCurrentWindow().label;

  async function closeSelf() {
    try {
      await invoke("pin_close", { id: label });
    } catch (cause) {
      await invoke("overlay_error", { index: 9, message: `close ${label}: ${cause}` }).catch(
        () => undefined,
      );
    }
  }

  useEffect(() => {
    let revoked: string | null = null;
    void (async () => {
      try {
        const buffer = await invoke<ArrayBuffer>("pin_image", { id: label });
        const blob = new Blob([buffer], { type: "image/png" });
        revoked = URL.createObjectURL(blob);
        setUrl(revoked);
      } catch (cause) {
        setError(String(cause));
      }
    })();
    return () => {
      if (revoked) URL.revokeObjectURL(revoked);
    };
  }, [label]);

  // The size at 100% is what the backend picked for this pin; everything else is a
  // multiple of it.
  useEffect(() => {
    void (async () => {
      const win = getCurrentWindow();
      const [size, scale] = await Promise.all([win.innerSize(), win.scaleFactor()]);
      const logical = size.toLogical(scale);
      baseRef.current = { width: logical.width, height: logical.height };
    })();
  }, []);

  // `startDragging()` hands the gesture to AppKit, which swallows the click and
  // breaks double-click-to-close. Moving the window from mousemove keeps both.
  useEffect(() => {
    const move = (event: MouseEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      // Belt and braces: if the button is up, stop moving even if a late mousedown result
      // managed to repopulate dragRef.
      if (event.buttons === 0) {
        dragRef.current = null;
        draggingRef.current = false;
        return;
      }
      event.preventDefault();
      void getCurrentWindow().setPosition(
        new LogicalPosition(
          Math.round(drag.originX + (event.screenX - drag.x)),
          Math.round(drag.originY + (event.screenY - drag.y)),
        ),
      );
    };
    const up = () => {
      dragRef.current = null;
      draggingRef.current = false;
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, []);

  // Plain wheel zooms. `passive: false` is required to keep WebKit from scrolling the
  // page instead; one IPC resize per animation frame keeps a fast trackpad from queuing
  // a backlog of setSize calls.
  useEffect(() => {
    const win = getCurrentWindow();

    const apply = async () => {
      try {
      const delta = wheelRef.current;
      wheelRef.current = 0;
      const base = baseRef.current;
      if (!delta || !base) return;
      const factor = Math.exp(-delta * 0.0015);
      const wanted = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoomRef.current * factor));
      const ratio = wanted / zoomRef.current;
      if (Math.abs(ratio - 1) < 0.001) return;
      zoomRef.current = wanted;

      // Keep the image under the cursor: the same content point must stay at the same
      // screen position while the window grows around it.
      const [scale, position] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
      const cursor = cursorRef.current;
      const origin = position.toLogical(scale);
      const x = Math.round(origin.x + cursor.x * (1 - ratio));
      const y = Math.round(origin.y + cursor.y * (1 - ratio));
      await win.setSize(
        new LogicalSize(
          Math.round(base.width * wanted),
          Math.round(base.height * wanted),
        ),
      );
      await win.setPosition(new LogicalPosition(x, y));

      setZoomLabel(wanted);
      if (badgeRef.current !== null) window.clearTimeout(badgeRef.current);
      badgeRef.current = window.setTimeout(() => setZoomLabel(null), 800);
      } catch (cause) {
        void invoke("overlay_error", {
          index: 9,
          message: `pin zoom failed: ${String(cause)}`,
        }).catch(() => undefined);
      }
    };

    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      cursorRef.current = { x: event.clientX, y: event.clientY };
      wheelRef.current += event.deltaY;
      if (frameRef.current !== null) return;
      frameRef.current = requestAnimationFrame(() => {
        frameRef.current = null;
        zoomQueueRef.current = zoomQueueRef.current.then(apply, apply);
      });
    };

    window.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      window.removeEventListener("wheel", onWheel);
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      if (badgeRef.current !== null) window.clearTimeout(badgeRef.current);
    };
  }, []);

  // Replace WebKit's OS-language context menu with a translated one.
  useEffect(() => {
    const onContext = (event: MouseEvent) => {
      event.preventDefault();
      setMenu({ x: event.clientX, y: event.clientY });
    };
    const dismiss = (event: MouseEvent) => {
      if (event.button !== 0) return;
      // mousedown fires before click. If we closed the menu here, clicking a menu item
      // would unmount the button before its onClick ran — so "Close" did nothing. Ignore
      // presses that land inside the menu; the item's own handler closes it.
      const target = event.target as HTMLElement | null;
      if (target?.closest?.(".pin-menu")) return;
      setMenu(null);
    };
    const onBlur = () => setMenu(null);
    window.addEventListener("contextmenu", onContext);
    window.addEventListener("mousedown", dismiss);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("contextmenu", onContext);
      window.removeEventListener("mousedown", dismiss);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  async function copySelf() {
    setMenu(null);
    try {
      await invoke("pin_copy", { id: label });
    } catch (cause) {
      await invoke("overlay_error", { index: 9, message: `copy ${label}: ${cause}` }).catch(
        () => undefined,
      );
    }
  }

  return (
    <div className="pin">
      {url && (
        <img
          src={url}
          alt=""
          draggable={false}
          onMouseDown={(event) => {
            if (event.button !== 0) return;
            draggingRef.current = true;
            const win = getCurrentWindow();
            void (async () => {
              const [position, scale] = await Promise.all([
                win.outerPosition(),
                win.scaleFactor(),
              ]);
              // mouseup may have landed while the IPC calls were in flight.
              if (!draggingRef.current) return;
              dragRef.current = {
                x: event.screenX,
                y: event.screenY,
                originX: position.x / scale,
                originY: position.y / scale,
              };
            })();
          }}
          onDoubleClick={() => void closeSelf()}
        />
      )}
      {error && <div className="pin-error">{error}</div>}
      {zoomLabel !== null && (
        <div className="pin-zoom">{Math.round(zoomLabel * 100)}%</div>
      )}
      {menu && (
        <div className="pin-menu" style={{ left: menu.x, top: menu.y }}>
          <button type="button" onClick={() => void copySelf()}>
            {t("pin.menu.copy")}
          </button>
          <button
            type="button"
            onClick={() => {
              setMenu(null);
              void closeSelf();
            }}
          >
            {t("pin.menu.close")}
          </button>
        </div>
      )}
      <button
        className="pin-close"
        title={t("pin.close.title")}
        aria-label={t("app.close")}
        onClick={() => void closeSelf()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M2 2 L8 8 M8 2 L2 8"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.3"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}
