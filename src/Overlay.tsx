import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  SceneModel,
  boundsOf,
  rectFromCorners,
  shiftShape,
  rectIsEmpty,
  type Point,
  type Rect,
  type Shape,
  type ToolId,
} from "./scene";
import { drawScene } from "./render";
import { setLocale, t, useLocale } from "./i18n";
import { drawMagnifier } from "./magnifier";
import { Toolbar } from "./Toolbar";
import { ACCENT } from "./theme";

type Meta = {
  index: number;
  pointsWidth: number;
  pointsHeight: number;
  pixelWidth: number;
  pixelHeight: number;
  scale: number;
  /** 0–1, how much to darken what is outside the selection. */
  dim: number;
  /** Loupe magnification, 2–16×. */
  magnifierZoom: number;
  /** Resolved interface language for this capture. */
  locale: "zh" | "en";
  /** Which capture these pixels belong to. */
  captureSerial: number;
  /** This monitor's top-left in the global virtual desktop (points). */
  originX: number;
  originY: number;
  /** Initial folder for the native save picker. */
  saveDir: string;
  initialRegion?: { x: number; y: number; w: number; h: number } | null;
};

async function chooseSaveDirectory(defaultPath: string): Promise<boolean> {
  const selected = await open({
    directory: true,
    multiple: false,
    defaultPath: defaultPath || undefined,
    title: t("overlay.save.pick_folder"),
  });
  const directory = Array.isArray(selected) ? selected[0] : selected;
  if (!directory) return false;
  await invoke("set_save_dir", { directory });
  return true;
}

const MIN_DRAG = 3;
/** How far (in points) from a shape still counts as clicking it. */
const HIT_TOLERANCE = 10;


/** The shapes a tool produces; the size/colour controls act on the last of these. */
function kindsOfTool(tool: ToolId): Shape["kind"][] {
  switch (tool) {
    case "rect":
      return ["rect"];
    case "ellipse":
      return ["ellipse"];
    case "arrow":
      return ["line"];
    case "pen":
      return ["pen"];
    case "marker":
      return ["marker"];
    case "mosaic":
      return ["mosaic"];
    case "text":
      return ["text"];
    case "counter":
      return ["counter"];
    default:
      return [];
  }
}

const TOOL_KEYS: Record<string, ToolId> = {
  r: "rect",
  o: "ellipse",
  a: "arrow",
  l: "arrow",
  p: "pen",
  h: "marker",
  m: "mosaic",
  t: "text",
  n: "counter",
  v: "select",
};

type Handle = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

/** Which resize handle (if any) the pointer is on. */
function handleAt(sel: Rect, point: Point, grab = 7): Handle | null {
  const nearX = (value: number) => Math.abs(point.x - value) <= grab;
  const nearY = (value: number) => Math.abs(point.y - value) <= grab;
  const insideX = point.x >= sel.x - grab && point.x <= sel.x + sel.w + grab;
  const insideY = point.y >= sel.y - grab && point.y <= sel.y + sel.h + grab;
  if (!insideX || !insideY) return null;
  const left = nearX(sel.x);
  const right = nearX(sel.x + sel.w);
  const top = nearY(sel.y);
  const bottom = nearY(sel.y + sel.h);
  if (top && left) return "nw";
  if (top && right) return "ne";
  if (bottom && left) return "sw";
  if (bottom && right) return "se";
  if (top) return "n";
  if (bottom) return "s";
  if (left) return "w";
  if (right) return "e";
  return null;
}

/** Applies a handle drag to the original rectangle. */
function resizeRect(rect: Rect, handle: Handle, dx: number, dy: number, min = 2): Rect {
  let x = rect.x;
  let y = rect.y;
  let w = rect.w;
  let h = rect.h;
  if (handle.includes("w")) {
    const next = Math.min(rect.x + rect.w - min, rect.x + dx);
    w = rect.x + rect.w - next;
    x = next;
  }
  if (handle.includes("e")) {
    w = Math.max(min, rect.w + dx);
  }
  if (handle.includes("n")) {
    const next = Math.min(rect.y + rect.h - min, rect.y + dy);
    h = rect.y + rect.h - next;
    y = next;
  }
  if (handle.includes("s")) {
    h = Math.max(min, rect.h + dy);
  }
  return { x, y, w, h };
}

/** True when a mouse event landed on the overlay's toolbar. */
function isToolbarClick(event: MouseEvent): boolean {
  const target = event.target as HTMLElement | null;
  return !!target?.closest?.(".toolbar");
}

export function Overlay() {
  useLocale();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const labelRef = useRef<HTMLDivElement | null>(null);
  const backgroundRef = useRef<HTMLCanvasElement | null>(null);
  const imageRef = useRef<ImageData | null>(null);
  const metaRef = useRef<Meta | null>(null);
  const modelRef = useRef(new SceneModel());
  const draftRef = useRef<Shape | null>(null);
  const dragRef = useRef<{ start: Point; current: Point } | null>(null);
  const textInputRef = useRef<HTMLInputElement | null>(null);
  /** Off-screen 2D context used to measure text before it is drawn. */
  const measureRef = useRef<CanvasRenderingContext2D | null>(null);
  const cursorRef = useRef<Point | null>(null);
  const selectionRef = useRef<Rect | null>(null);
  /** Capture this page booted for; a stale boot must not show over a newer capture. */
  const serialRef = useRef<number | null>(null);
  /** Set by the input effect, called by the global-shortcut listener. */
  const actionRef = useRef<(action: string) => void>(() => undefined);
  // The same physical key reaches us twice: once as a DOM keydown, once forwarded by
  // Rust's global shortcut (for when the webview has no keyboard focus). `runAction` is
  // not idempotent, so collapse only that cross-source pair — two genuine presses from
  // the same source must both go through even if they land within 50 ms.
  const lastActionRef = useRef<{ action: string; source: string; at: number }>({
    action: "",
    source: "",
    at: 0,
  });
  /** Set while dragging one of the selection's handles. */
  const resizeRef = useRef<{ handle: Handle; start: Point; rect: Rect } | null>(null);
  /** The annotation the colour/size controls act on, when one is selected. */
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const selectedIdRef = useRef<number | null>(null);
  /** Set while dragging an existing annotation to a new position. */
  const moveRef = useRef<{ before: Shape; start: Point } | null>(null);

  const [meta, setMeta] = useState<Meta | null>(null);
  const [selection, setSelection] = useState<Rect | null>(null);
  const [tool, setTool] = useState<ToolId>("rect");
  const [color, setColor] = useState(ACCENT);
  const [width, setWidth] = useState(3);
  /** Font size for text labels: independent from the stroke width on purpose. */
  const [textSize, setTextSize] = useState(21);
  const [textInput, setTextInput] = useState<Point | null>(null);
  /** Bumped every time the field opens; used as its React key (see openTextField). */
  const [draftSeq, setDraftSeq] = useState(0);
  /** Width of the field, grown to fit what has been typed. */
  const [fieldWidth, setFieldWidth] = useState(0);

  /**
   * Opens (or re-opens) the text field. The sequence number keys the <input>, so React
   * remounts it: a reused node keeps its old DOM value, which turned the next click into
   * a silent duplicate of the previous label instead of an empty field.
   */
  const openTextField = useCallback(
    (at: Point, editing: { id: number; text: string } | null = null) => {
      setEditingText(editing);
      setTextInput(at);
      setDraftSeq((value) => value + 1);
    },
    [],
  );

  /** Grows the field to the typed text, measured with the same font that draws it. */
  const fitField = useCallback(() => {
    const input = textInputRef.current;
    if (!input) return;
    const context = measureRef.current ?? (measureRef.current = document.createElement("canvas").getContext("2d"));
    if (!context) return;
    context.font = `${textSize}px -apple-system, BlinkMacSystemFont, "PingFang SC", sans-serif`;
    const measured = context.measureText(input.value).width;
    setFieldWidth(Math.max(88, Math.ceil(measured) + 18));
  }, [textSize]);

  /** Set when the field is editing an existing label instead of creating one. */
  const [editingText, setEditingText] = useState<{ id: number; text: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [scrollFrames, setScrollFrames] = useState<number | null>(null);
  /** Frames dropped because the page jumped between captures (scrolled too fast). */
  const [skippedFrames, setSkippedFrames] = useState(0);
  const [longShot, setLongShot] = useState<{ url: string; blob: Blob } | null>(null);
  const longShotUrlRef = useRef<string | null>(null);
  /** Swap the long-image result, revoking the old object URL. The Blob owns the PNG
   * bytes; keeping a second Uint8Array pinned another full copy on this warm page. */
  const setLongShotSafe = useCallback((next: { url: string; blob: Blob } | null) => {
    const previous = longShotUrlRef.current;
    if (previous && previous !== next?.url) URL.revokeObjectURL(previous);
    longShotUrlRef.current = next?.url ?? null;
    setLongShot(next);
  }, []);
  const [revision, setRevision] = useState(0);

  // ── drawing ──────────────────────────────────────────────────────────────

  const paint = useCallback(() => {
    const canvas = canvasRef.current;
    const background = backgroundRef.current;
    const info = metaRef.current;
    if (!canvas || !background || !info) return;
    const context = canvas.getContext("2d");
    if (!context) return;

    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.drawImage(background, 0, 0);
    context.setTransform(info.scale, 0, 0, info.scale, 0, 0);

    // While a region is being dragged the selection is deliberately unset (the toolbar
    // must not appear yet), so draw the live rectangle instead of the committed one —
    // otherwise nothing is visible until the mouse comes back up.
    const committed = selectionRef.current;
    const drag = dragRef.current;
    const liveDrag = drag ? rectFromCorners(drag.start, drag.current) : null;
    const sel: Rect | null =
      committed ?? (liveDrag && !rectIsEmpty(liveDrag) ? liveDrag : null);

    if (sel) {
      context.fillStyle = `rgba(0, 0, 0, ${info.dim})`;
      context.fillRect(0, 0, info.pointsWidth, sel.y);
      context.fillRect(0, sel.y + sel.h, info.pointsWidth, info.pointsHeight - sel.y - sel.h);
      context.fillRect(0, sel.y, sel.x, sel.h);
      context.fillRect(sel.x + sel.w, sel.y, info.pointsWidth - sel.x - sel.w, sel.h);

      // Annotations only exist inside the selection, exactly like the export.
      context.save();
      context.beginPath();
      context.rect(sel.x, sel.y, sel.w, sel.h);
      context.clip();
      drawScene(context, modelRef.current.all, { scale: info.scale });
      if (draftRef.current) {
        drawScene(context, [draftRef.current], { scale: info.scale });
      }
      context.restore();

      const selected = selectedIdRef.current;
      if (selected !== null) {
        const shape = modelRef.current.byId(selected);
        if (shape) {
          context.save();
          context.setLineDash([4, 3]);
          context.strokeStyle = "rgba(255, 255, 255, 0.9)";
          context.lineWidth = 1;
          for (const box of boundsOf(shape, 3)) {
            context.strokeRect(box.x + 0.5, box.y + 0.5, box.w, box.h);
          }
          context.restore();
        }
      }

      context.strokeStyle = ACCENT;
      context.lineWidth = 1;
      context.strokeRect(sel.x + 0.5, sel.y + 0.5, Math.max(0, sel.w - 1), Math.max(0, sel.h - 1));

      {
        // Resize handles: corners and edge midpoints.
        const half = 3;
        const points: Point[] = [
          { x: sel.x, y: sel.y },
          { x: sel.x + sel.w / 2, y: sel.y },
          { x: sel.x + sel.w, y: sel.y },
          { x: sel.x + sel.w, y: sel.y + sel.h / 2 },
          { x: sel.x + sel.w, y: sel.y + sel.h },
          { x: sel.x + sel.w / 2, y: sel.y + sel.h },
          { x: sel.x, y: sel.y + sel.h },
          { x: sel.x, y: sel.y + sel.h / 2 },
        ];
        context.fillStyle = "#ffffff";
        context.strokeStyle = ACCENT;
        for (const point of points) {
          context.beginPath();
          context.rect(point.x - half, point.y - half, half * 2, half * 2);
          context.fill();
          context.stroke();
        }
      }

      const label = labelRef.current;
      if (label) {
        label.textContent = `${Math.round(sel.w)} × ${Math.round(sel.h)}`;
        label.style.left = `${sel.x}px`;
        label.style.top = `${Math.max(14, sel.y - 4)}px`;
        label.style.display = "block";
      }
    } else {
      if (labelRef.current) labelRef.current.style.display = "none";
      // Before a region exists there is no "outside"; keep a lighter version of the same
      // dim so picking "no dimming" really means no dimming.
      context.fillStyle = `rgba(0, 0, 0, ${info.dim * 0.36})`;
      context.fillRect(0, 0, info.pointsWidth, info.pointsHeight);
    }

    const cursor = cursorRef.current;
    if (cursor && !committed) {
      const zoom = Math.min(16, Math.max(2, info.magnifierZoom));
      drawMagnifier(context, background, imageRef.current, info, cursor, dragRef.current, zoom);
    }
  }, []);

  // Coalesce the flood of mousemove events into at most one full-canvas repaint per frame.
  // Without this a 2x full-screen surface (~20 MB) is cleared and redrawn on every event,
  // which starves annotation dragging and the magnifier on high-polling-rate mice.
  const paintFrameRef = useRef<number | null>(null);
  const schedulePaint = useCallback(() => {
    if (paintFrameRef.current !== null) return;
    paintFrameRef.current = requestAnimationFrame(() => {
      paintFrameRef.current = null;
      paint();
    });
  }, [paint]);
  useEffect(() => () => {
    if (paintFrameRef.current !== null) cancelAnimationFrame(paintFrameRef.current);
    if (longShotUrlRef.current) URL.revokeObjectURL(longShotUrlRef.current);
  }, []);

  // `revision` only exists to trigger a repaint after the scene changes; `paint`
  // itself stays stable, which keeps the effects below from re-binding (and `boot`
  // from re-running and resetting the selection) on every edit.
  useEffect(() => {
    paint();
  }, [paint, revision]);

  useEffect(() => {
    selectedIdRef.current = selectedId;
  }, [selectedId]);

  // A borderless webview does not always honour `autoFocus`; focus it ourselves once the
  // input exists, otherwise typing goes to whatever had focus before.
  useEffect(() => {
    if (!textInput) return;
    const focus = () => {
      void getCurrentWindow()
        .setFocus()
        .catch(() => undefined);
      const input = textInputRef.current;
      input?.focus({ preventScroll: true });
      input?.select();
    };
    // WebKit only honours `autofocus` for elements present at load, and the window may
    // still be handing focus over; try once now and once after the first paint.
    focus();
    // Measure after the node exists, so the field already fits the text it is opened with.
    const fit = () => {
      focus();
      fitField();
    };
    const frame = requestAnimationFrame(fit);
    return () => cancelAnimationFrame(frame);
  }, [textInput, draftSeq, fitField]);

  // ── long capture ─────────────────────────────────────────────────────────

  /** The one scroll-capture path: hide the overlay, watch the real screen, Enter ends. */
  const startManual = useCallback(
    async (rect: Rect) => {
      const info = metaRef.current;
      if (!info) return;
      setSkippedFrames(0);
      try {
        await invoke("scroll_start", {
          index: info.index,
          x: Math.round(rect.x),
          y: Math.round(rect.y),
          width: Math.round(rect.w),
          height: Math.round(rect.h),
          mode: "manual",
          direction: "down",
        });
      } catch (cause) {
        setError(String(cause));
      }
    },
    [],
  );

  // ── boot ─────────────────────────────────────────────────────────────────

  /**
   * Clears everything a capture left behind.
   *
   * The overlay window is reused across captures (its page is already loaded, which saves
   * ~90 ms per click), so the scene, the frozen frame and any half-open text field must be
   * dropped explicitly instead of dying with the window.
   */
  const resetScene = useCallback(() => {
    modelRef.current = new SceneModel();
    selectionRef.current = null;
    draftRef.current = null;
    dragRef.current = null;
    moveRef.current = null;
    resizeRef.current = null;
    imageRef.current = null;
    const background = backgroundRef.current;
    if (background) {
      // The detached canvas would otherwise keep its full-screen backing store alive
      // until a GC that may never run while this warm page sits idle.
      background.width = 1;
      background.height = 1;
    }
    backgroundRef.current = null;
    metaRef.current = null;
    setMeta(null);
    setSelection(null);
    setSelectedId(null);
    setLongShotSafe(null);
    setError(null);
    setBusy(false);
    setSkippedFrames(0);
    setScrollFrames(null);
    setEditingText(null);
    setTextInput(null);
    setRevision((value) => value + 1);
    const canvas = canvasRef.current;
    if (canvas) {
      // Dropping the backing store matters: 2880×1800 is a 20 MB surface, and the page
      // now stays alive between captures. Remove the compositor layer first, then shrink
      // the CSS box and bitmap so WebKit does not stretch 1×1 pixels across the display.
      canvas.style.display = "none";
      canvas.style.width = "1px";
      canvas.style.height = "1px";
      canvas.width = 1;
      canvas.height = 1;
    }
  }, [setLongShotSafe]);

  const boot = useCallback(async () => {
    // A warm page can have lost the previous capture's `overlay:end` (for example while
    // the Mac slept). Clear the scene *before* waiting for pixels, otherwise the old
    // selection is still live under the cursor and the first drag moves it instead of
    // starting a new region.
    resetScene();
    let step = "label";
    const bootAt = performance.now();
    const marks: string[] = [];
    const mark = (name: string) => marks.push(`${name}=${Math.round(performance.now() - bootAt)}`);
    const label = getCurrentWindow().label;
    const index = Number(label.split("-")[1] ?? "0");
    try {
      step = "meta";
      const info = await invoke<Meta>("overlay_meta", { index });
      // The pre-warmed page can outlive a language switch in Settings; take the
      // language the capture was started with.
      setLocale(info.locale);
      mark("meta");
      step = "frame";
      const buffer = await invoke<ArrayBuffer>("overlay_frame", { index });
      mark("frame");
      step = "image-data";
      const image = new ImageData(
        new Uint8ClampedArray(buffer),
        info.pixelWidth,
        info.pixelHeight,
      );

      mark("image");
      step = "background";
      const background = document.createElement("canvas");
      background.width = info.pixelWidth;
      background.height = info.pixelHeight;
      background.getContext("2d")?.putImageData(image, 0, 0);

      mark("background");
      step = "canvas";
      const canvas = canvasRef.current;
      if (canvas) {
        canvas.width = info.pixelWidth;
        canvas.height = info.pixelHeight;
        canvas.style.width = `${info.pointsWidth}px`;
        canvas.style.height = `${info.pointsHeight}px`;
      }

      step = "state";
      imageRef.current = image;
      metaRef.current = info;
      backgroundRef.current = background;
      // Start with nothing selected: the first drag defines the region, and the
      // toolbar appears once it exists (dragging inside an existing region annotates).
      // Scripted/debug runs can open with a preset region.
      const preset = info.initialRegion ?? null;
      selectionRef.current = preset;
      setMeta(info);
      setSelection(preset);
      paint();
      mark("paint");
      // Size and paint while the element is detached from compositing; revealing the
      // finished layer avoids a full-screen layout/allocation on the visible frame.
      if (canvas) canvas.style.display = "block";
      // One line per capture: the click-to-pixels budget is what makes the tool feel
      // instant, so the numbers have to be visible in the log.
      void invoke("overlay_error", {
        index,
        message: `perf: boot ${Math.round(performance.now() - bootAt)} ms (start=${Math.round(bootAt)} ${marks.join(" ")}) ${info.pixelWidth}×${info.pixelHeight}`,
      }).catch(() => undefined);
      await invoke("overlay_show", { index, serial: info.captureSerial });
      return info.captureSerial;
    } catch (cause) {
      const message = `[${step}] ${String(cause)}`;
      setError(message);
      await invoke("overlay_error", { index, message }).catch(() => undefined);
      await invoke("overlay_show", { index, serial: null }).catch(() => undefined);
      return null;
    }
  }, [paint, resetScene]);


  const bootingRef = useRef(false);
  const pendingBootRef = useRef(false);
  const bootOnce = useCallback(async () => {
    // A slow boot (first capture after sleep) can still be running when the user
    // cancels and tries again. Dropping that second `overlay:begin` left the new
    // capture with an empty, non-interactive overlay, so queue it instead.
    if (bootingRef.current) {
      pendingBootRef.current = true;
      return;
    }
    bootingRef.current = true;
    try {
      do {
        pendingBootRef.current = false;
        const booted = await boot();
        if (!pendingBootRef.current) break;
        // A queued signal for the capture that just rendered is the duplicate the page
        // already gets from its mount check; render it twice and the second boot wipes
        // the first one's state (including a preset selection).
        if (booted !== null && booted === serialRef.current) break;
      } while (true);
    } finally {
      bootingRef.current = false;
    }
  }, [boot]);

  useEffect(() => {
    let cancelled = false;
    const begin = listen<number>("overlay:begin", (event) => {
      // Both the mount check and this event can announce the same capture; only the
      // first one may boot it.
      if (event.payload === serialRef.current) return;
      serialRef.current = event.payload;
      void bootOnce();
    });
    const end = listen<number>("overlay:end", (event) => {
      // A new capture may have started before this event was delivered; clearing then
      // would wipe its selection.
      if (event.payload === serialRef.current) resetScene();
    });
    void (async () => {
      // A window kept alive between captures parks until a capture starts; one built for
      // a capture boots straight away.
      const serial = await invoke<number | null>("overlay_idle").catch(() => null);
      if (!cancelled && serial !== null && serial !== serialRef.current) {
        serialRef.current = serial;
        void bootOnce();
      }
      await Promise.all([begin, end]);
      // The capture may have started while the listeners were being registered.
      const latest = await invoke<number | null>("overlay_idle").catch(() => null);
      if (!cancelled && latest !== null && latest !== serialRef.current) {
        serialRef.current = latest;
        void bootOnce();
      }
    })();
    return () => {
      cancelled = true;
      void begin.then((off) => off());
      void end.then((off) => off());
    };
  }, [bootOnce, resetScene]);

  // Scroll capture: the overlay hides while Rust drives the page, then the stitched
  // image comes back as PNG bytes and replaces the stage.
  useEffect(() => {
    const pending = listen<{ frames: number; height: number; state: string; message?: string | null }>(
      "scroll:progress",
      (event) => {
        const { state, frames, message } = event.payload;
        const skipped = (event.payload as { skipped?: number }).skipped ?? 0;
        if (state === "capturing") {
          if (skipped > 0) setSkippedFrames(skipped);
          setScrollFrames(frames);
          return;
        }
        setScrollFrames(null);
        if (state !== "done") {
          const text =
        state === "cancelled" ? t("overlay.scroll.cancelled") : message || t("overlay.scroll.failed");
          void invoke("overlay_error", { index: 0, message: `scroll: ${text}` }).catch(
            () => undefined,
          );
          setError(text);
          return;
        }
        const resultSerial = serialRef.current;
        void (async () => {
          try {
            const buffer = await invoke<ArrayBuffer>("scroll_result");
            // A cancel or a new capture may have replaced the session while that IPC was
            // in flight. Don't paint a stale long image over the fresh selection.
            if (serialRef.current !== resultSerial) return;
            const bytes = new Uint8Array(buffer);
            const blob = new Blob([bytes], { type: "image/png" });
            const url = URL.createObjectURL(blob);
            setLongShotSafe({ url, blob });
          } catch (cause) {
            setError(String(cause));
          }
        })();
      },
    );
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  // Keys forwarded by Rust when the webview does not hold keyboard focus.
  useEffect(() => {
    const pending = listen<{ action: string; target: number | null }>(
      "overlay:key",
      (event) => {
        const payload = event.payload;
        // Only the display under the pointer acts; the others drop the event.
        if (payload.target != null && metaRef.current && payload.target !== metaRef.current.index) {
          return;
        }
        actionRef.current(payload.action);
      },
    );
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  // ── export ───────────────────────────────────────────────────────────────

  const commit = useCallback(
    async (action: "clipboard" | "file" | "pin") => {
      const info = metaRef.current;
      if (!info || busy) return;
      const area = selectionRef.current ?? { x: 0, y: 0, w: info.pointsWidth, h: info.pointsHeight };
      if (area.w < 1 || area.h < 1) return;
      setBusy(true);
      try {
        if (action === "file" && !(await chooseSaveDirectory(info.saveDir))) {
          setBusy(false);
          return;
        }
        const model = modelRef.current;
        if (model.size === 0) {
          // No annotations: let Rust crop the original frame. It is both faster and
          // keeps the untouched pixels bit for bit.
          await invoke("capture_commit", {
            index: info.index,
            x: Math.round(area.x),
            y: Math.round(area.y),
            width: Math.round(area.w),
            height: Math.round(area.h),
            action,
          });
          return;
        }

        const background = backgroundRef.current;
        const composite = document.createElement("canvas");
        composite.width = Math.round(area.w * info.scale);
        composite.height = Math.round(area.h * info.scale);
        const context = composite.getContext("2d");
        if (!context || !background) throw new Error("cannot render the export");
        context.drawImage(
          background,
          Math.round(area.x * info.scale),
          Math.round(area.y * info.scale),
          composite.width,
          composite.height,
          0,
          0,
          composite.width,
          composite.height,
        );
        context.setTransform(info.scale, 0, 0, info.scale, -area.x * info.scale, -area.y * info.scale);
        drawScene(context, model.all, { scale: info.scale });

        const blob = await new Promise<Blob | null>((resolve) =>
          composite.toBlob((value) => resolve(value), "image/png"),
        );
        if (!blob) throw new Error("PNG encoding failed");
        const bytes = new Uint8Array(await blob.arrayBuffer());
        await invoke("set_export_scale", { scale: info.scale });
        if (action === "pin") {
          // Tell Rust where this selection sits on the real screen so the pin appears
          // in place instead of the top-left corner of the primary display.
          await invoke("set_pin_origin", {
            x: Math.round(info.originX + area.x),
            y: Math.round(info.originY + area.y),
          });
          await invoke("pin_png", bytes);
        }
        else await invoke(action === "file" ? "save_png" : "copy_png", bytes);
      } catch (cause) {
        setBusy(false);
        setError(String(cause));
      }
    },
    [busy],
  );

  /**
   * Colour and size set the default for the next shape, and change the selected one if
   * there is a selection. They never silently follow "whatever was drawn last" — that
   * made text and shapes change colour long after they were finished.
   */
  const changeWidth = useCallback(
    (next: number) => {
      setWidth(next);
      // With the move tool: the selected shape. Otherwise: the most recent shape this
      // tool draws, so adjusting "粗细" fixes the rectangle you just drew but never
      // touches a label's font size.
      const id =
        tool === "select"
          ? selectedId
          : (modelRef.current.lastWithKinds(kindsOfTool(tool))?.id ?? null);
      // Labels have their own font size (changeTextSize); a stroke width must never
      // resize them.
      if (id !== null && modelRef.current.updateStyle(id, { width: next })) {
        setRevision((value) => value + 1);
      }
    },
    [selectedId, tool],
  );

  const changeTextSize = useCallback(
    (next: number) => {
      setTextSize(next);
      const id =
        tool === "select"
          ? selectedId
          : (modelRef.current.lastWithKinds(["text"])?.id ?? null);
      if (id !== null && modelRef.current.updateTextSize(id, next)) {
        setRevision((value) => value + 1);
      }
    },
    [selectedId, tool],
  );

  const changeColor = useCallback(
    (next: string) => {
      setColor(next);
      const id =
        tool === "select"
          ? selectedId
          : (modelRef.current.lastWithKinds(kindsOfTool(tool))?.id ?? null);
      if (id !== null && modelRef.current.updateStyle(id, { color: next })) {
        setRevision((value) => value + 1);
      }
    },
    [selectedId, tool],
  );

  /** Closes the field without adding anything (Esc, or leaving it empty). */
  const closeTextField = useCallback(() => {
    setEditingText(null);
    setTextInput(null);
  }, []);

  /** Commits whatever is in the text field (Enter, or right-click like Snipaste). */
  const commitText = useCallback(() => {
    const input = textInputRef.current;
    const value = input?.value.trim() ?? "";
    const editing = editingText;
    const at = textInput;
    closeTextField();
    if (!at) return;
    if (!value) return;
    const model = modelRef.current;
    if (editing) {
      if (model.updateText(editing.id, value)) setRevision((v) => v + 1);
      return;
    }
    model.add({
      id: model.id(),
      kind: "text",
      at,
      text: value,
      size: textSize,
      style: { color, width },
    });
    setRevision((v) => v + 1);
    // Stay on the text tool; clicking the label moves it (see onDown). Switching to the
    // move tool here made every later size tweak resize the label the user just wrote.
  }, [closeTextField, color, editingText, textInput, textSize, width]);

  const cancel = useCallback(() => {
    void invoke("overlay_cancel");
  }, []);

  // The stitched long image is exported as-is: it is already at native resolution and
  // far taller than the window, so annotating it needs the scrollable result view that
  // lands with M5.
  const copyLongShot = useCallback(async (blob: Blob) => {
    try {
      await invoke("set_export_scale", { scale: 1 });
      await invoke("copy_png", new Uint8Array(await blob.arrayBuffer()));
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  const saveLongShot = useCallback(async (blob: Blob) => {
    try {
      if (!(await chooseSaveDirectory(metaRef.current?.saveDir ?? ""))) return;
      await invoke("set_export_scale", { scale: 1 });
      await invoke("save_png", new Uint8Array(await blob.arrayBuffer()));
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  // ── input ────────────────────────────────────────────────────────────────

  useEffect(() => {
    const info = () => metaRef.current;

    const buildDraft = (start: Point, current: Point): Shape | null => {
      const model = modelRef.current;
      const style = { color, width };
      switch (tool) {
        case "rect":
        case "ellipse": {
          const rect = rectFromCorners(start, current);
          if (rectIsEmpty(rect)) return null;
          return { id: model.id(), kind: tool, rect, style };
        }
        case "mosaic": {
          const rect = rectFromCorners(start, current);
          if (rectIsEmpty(rect)) return null;
          // The options row's size control is the mosaic grain, so it has to matter.
          return { id: model.id(), kind: "mosaic", rect, block: 6 + width * 2 };
        }
        case "arrow":
          return {
            id: model.id(),
            kind: "line",
            points: [start, current],
            style,
            arrow: tool === "arrow",
          };
        case "pen":
        case "marker":
          return { id: model.id(), kind: tool, points: [start, current], style };
        default:
          return null;
      }
    };

    const onMove = (event: MouseEvent) => {
      cursorRef.current = { x: event.clientX, y: event.clientY };
      // A mouseup that happened outside the window never reaches us; without this the
      // element kept following the pointer with no button held.
      if (event.buttons === 0 && (dragRef.current || moveRef.current || resizeRef.current)) {
        // A display waking from sleep can deliver a mousemove with buttons=0 mid-drag.
        // Commit an in-progress *region* drag as the selection instead of dropping it:
        // otherwise `dragRef` is gone but `draftRef` (the rectangle) survives, and the
        // next mouseup mistakes it for a rectangle annotation — the region never becomes
        // a selection, so the toolbar never appears and the user sees only the dim mask.
        const drag = dragRef.current;
        if (drag && !selectionRef.current && !moveRef.current && !resizeRef.current) {
          const rect = rectFromCorners(drag.start, drag.current);
          selectionRef.current = rectIsEmpty(rect, MIN_DRAG)
            ? { x: 0, y: 0, w: info()?.pointsWidth ?? 0, h: info()?.pointsHeight ?? 0 }
            : rect;
          draftRef.current = null;
        }
        dragRef.current = null;
        moveRef.current = null;
        resizeRef.current = null;
        setSelection(selectionRef.current);
        schedulePaint();
        return;
      }
      const moving = moveRef.current;
      if (moving) {
        const dx = event.clientX - moving.start.x;
        const dy = event.clientY - moving.start.y;
        // A click with a stray mousemove must not become an undo step that does nothing.
        if (dx === 0 && dy === 0) return;
        modelRef.current.replaceShape(shiftShape(moving.before, dx, dy));
        // The model is a ref, so a repaint is all that's needed mid-drag; the single
        // setRevision on mouseup commits the undo step. Re-rendering the whole overlay
        // (and Toolbar) on every move was the拖拽 jank.
        schedulePaint();
        return;
      }
      const resizing = resizeRef.current;
      if (resizing) {
        const next = resizeRect(
          resizing.rect,
          resizing.handle,
          event.clientX - resizing.start.x,
          event.clientY - resizing.start.y,
        );
        selectionRef.current = next;
        setSelection(next);
        schedulePaint();
        return;
      }
      // Hover feedback: with the move tool, anything under the pointer can be dragged.
      const canvas = canvasRef.current;
      if (canvas && !dragRef.current && !moveRef.current && !resizeRef.current) {
        const hovered =
          tool === "select" &&
          modelRef.current.hitTest({ x: event.clientX, y: event.clientY }, HIT_TOLERANCE);
        canvas.style.cursor = hovered ? "move" : "crosshair";
      }
      const drag = dragRef.current;
      if (drag) {
        drag.current = { x: event.clientX, y: event.clientY };
        if (tool === "pen" || tool === "marker") {
          const draft = draftRef.current;
          if (draft && (draft.kind === "pen" || draft.kind === "marker")) {
            draft.points.push({ x: event.clientX, y: event.clientY });
          } else {
            draftRef.current = buildDraft(drag.start, drag.current);
          }
        } else {
          draftRef.current = buildDraft(drag.start, drag.current);
        }

      }
      schedulePaint();
    };

    const onDown = (event: MouseEvent) => {
      if (event.button !== 0) return;
      // The toolbar is part of the overlay; clicking a button must not start a region
      // drag or be mistaken for "the user released the mouse".
      if (isToolbarClick(event)) return;
      // Clicking inside the open field is caret placement, not a new click on the canvas.
      if (event.target === textInputRef.current) return;
      const screen = info();
      if (!screen) return;
      // An open label is finished by clicking anywhere else, so the next click starts a
      // fresh one. Without this, the field simply moved and no second label was ever
      // added.
      if (textInput) commitText();
      const point = { x: event.clientX, y: event.clientY };
      const sel = selectionRef.current;

      // Outside the current selection: start a new region instead of annotating.
      const inside =
        sel &&
        point.x >= sel.x &&
        point.y >= sel.y &&
        point.x <= sel.x + sel.w &&
        point.y <= sel.y + sel.h;

      if (!inside) {
        dragRef.current = { start: point, current: point };
        selectionRef.current = null;
        setSelection(null);
        draftRef.current = null;
        paint();
        return;
      }

      if (tool === "select") {
        const hit = modelRef.current.hitTest(point, HIT_TOLERANCE);
        setSelectedId(hit ? hit.id : null);
        setRevision((value) => value + 1);
        // Double-click a label to edit it again, like Snipaste's text tool.
        if (event.detail === 2 && hit && hit.kind === "text") {
          openTextField({ x: hit.at.x, y: hit.at.y }, { id: hit.id, text: hit.text });
          return;
        }
        if (hit) {
          moveRef.current = { before: hit, start: point };
          return;
        }
      }

      // Grab a handle before anything else, so an existing selection can be resized.
      const handle = sel ? handleAt(sel, point) : null;
      if (handle) {
        resizeRef.current = { handle, start: point, rect: sel as Rect };
        return;
      }

      if (tool === "text") {
        // The text tool always starts a new label. Moving an existing one is the move
        // tool's job (V), which is what people expect after the first label exists.
        openTextField(point);
        return;
      }
      if (tool === "counter") {
        const model = modelRef.current;
        model.add({
          id: model.id(),
          kind: "counter",
          at: point,
          number: model.counter(),
          style: { color, width },
        });
        setRevision((value) => value + 1);
        return;
      }
      if (tool === "select") return;

      dragRef.current = { start: point, current: point };
      draftRef.current = null;
      setSelectedId(null);
      paint();
    };

    const onUp = (event: MouseEvent) => {
      if (event.button !== 0) return;
      if (isToolbarClick(event)) return;
      if (moveRef.current) {
        const moving = moveRef.current;
        moveRef.current = null;
        const moved = modelRef.current.byId(moving.before.id);
        if (moved && (moved as Shape) !== moving.before) {
          modelRef.current.recordModify(moving.before, moved);
        }
        setRevision((value) => value + 1);
        return;
      }
      if (resizeRef.current) {
        resizeRef.current = null;
        setSelection(selectionRef.current);
        paint();
        return;
      }
      const drag = dragRef.current;
      dragRef.current = null;

      // Finishing a region selection.
      if (!selectionRef.current && drag) {
        const rect = rectFromCorners(drag.start, drag.current);
        if (rectIsEmpty(rect, MIN_DRAG)) {
          selectionRef.current = { x: 0, y: 0, w: info()?.pointsWidth ?? 0, h: info()?.pointsHeight ?? 0 };
        } else {
          selectionRef.current = rect;
        }
        setSelection(selectionRef.current);
        draftRef.current = null;
        paint();
        return;
      }

      const draft = draftRef.current;
      draftRef.current = null;
      // Without a committed selection this gesture was a region drag, not an annotation.
      // Never turn a leftover rectangle draft into a shape — that is what made a region
      // capture look like the rectangle tool had drawn on the frozen frame.
      if (!selectionRef.current) {
        paint();
        return;
      }
      if (!draft) {
        paint();
        return;
      }
      if (draft.kind === "line" || draft.kind === "pen" || draft.kind === "marker") {
        if (draft.points.length < 2) {
          paint();
          return;
        }
        // A click without a drag is not a line.
        const first = draft.points[0];
        const last = draft.points[draft.points.length - 1];
        if (Math.abs(first.x - last.x) < MIN_DRAG && Math.abs(first.y - last.y) < MIN_DRAG) {
          paint();
          return;
        }
      }
      modelRef.current.add(draft);
      setRevision((value) => value + 1);
    };

    const onContext = (event: MouseEvent) => {
      event.preventDefault();
      if (draftRef.current) {
        draftRef.current = null;
        paint();
        return;
      }
      if (textInput) {
        // Snipaste ends text input with a right-click; match that.
        commitText();
        return;
      }
      const sel = selectionRef.current;
      const full = info();
      if (sel && full && (sel.w < full.pointsWidth || sel.h < full.pointsHeight)) {
        selectionRef.current = { x: 0, y: 0, w: full.pointsWidth, h: full.pointsHeight };
        setSelection(selectionRef.current);
        paint();
        return;
      }
      cancel();
    };

    // The overlay is modal: Escape/Enter/Save/Copy are also registered as global
    // shortcuts in Rust and arrive through `overlay:key` (see `main.rs`). Both paths
    // funnel here, so behaviour cannot drift between "focused" and "not focused".
    const runAction = (action: string, source: "dom" | "global") => {
      const now = performance.now();
      const last = lastActionRef.current;
      // Collapse only the DOM+global duplicate of one physical key. Two genuine presses
      // from the same source must both go through (e.g. Enter twice to copy twice).
      if (last.action === action && last.source !== source && now - last.at < 50) {
        return;
      }
      lastActionRef.current = { action, source, at: now };
      // The text field owns the keyboard while it is open. Without this, the global
      // Enter shortcut exported the capture at the same moment the input committed the
      // text — the text was added to the scene *after* the export ran, so it was missing
      // from the saved image.
      if (textInput) {
        if (action === "escape") {
          closeTextField();
          return;
        }
        // Pinning while a label is open keeps that label: the text is committed first.
        if (action === "pin") {
          commitText();
          void commit("pin");
          return;
        }
        return;
      }
      // While a scroll capture is running, Enter/Esc mean "stop and stitch".
      if (scrollFrames !== null && (action === "enter" || action === "escape")) {
        void invoke("scroll_cancel");
        return;
      }
      if (action === "escape") {
        if (draftRef.current) {
          draftRef.current = null;
          paint();
        } else {
          cancel();
        }
      } else if (action === "enter" || action === "copy") {
        void commit("clipboard");
      } else if (action === "save") {
        void commit("file");
      } else if (action === "pin") {
        void commit("pin");
      }
    };
    // The global-shortcut path always reports source "global"; DOM keydowns report "dom".
    actionRef.current = (action) => runAction(action, "global");

    const onKey = (event: KeyboardEvent) => {
      const withCommand = event.metaKey || event.ctrlKey;
      if (event.key === "Escape") {
        event.preventDefault();
        runAction("escape", "dom");
        return;
      }
      if (event.key === "Enter") {
        event.preventDefault();
        runAction("enter", "dom");
        return;
      }
      if (withCommand && event.key.toLowerCase() === "s") {
        event.preventDefault();
        runAction("save", "dom");
        return;
      }
      if (withCommand && event.key.toLowerCase() === "c") {
        event.preventDefault();
        runAction("copy", "dom");
        return;
      }
      if (withCommand && event.key.toLowerCase() === "z") {
        event.preventDefault();
        const model = modelRef.current;
        if (event.shiftKey) model.redo();
        else model.undo();
        setRevision((value) => value + 1);
        return;
      }
      if (withCommand) return;
      const next = TOOL_KEYS[event.key.toLowerCase()];
      if (!next) return;
      if (textInput) commitText();
      setTool(next);
      if (next !== "select") setSelectedId(null);
    };

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("mouseup", onUp);
    window.addEventListener("contextmenu", onContext);

    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("mouseup", onUp);
      window.removeEventListener("contextmenu", onContext);

      window.removeEventListener("keydown", onKey);
    };
  }, [
    cancel,
    changeColor,
    closeTextField,
    commitText,
    openTextField,
    changeWidth,
    color,
    commit,
    paint,
    schedulePaint,
    scrollFrames,
    textInput,
    tool,
    width,
  ]);

  if (error) {
    return (
      <div className="error" role="alert">
        <strong>{t("overlay.error.title")}</strong>
        <p>{error}</p>
        <button className="primary" onClick={() => void cancel()}>
          {t("app.close")}
        </button>
      </div>
    );
  }

  // The toolbar is wider than a small selection, so it is centred on the screen rather
  // than hung off the selection's corner — anchoring it to the corner pushed the
  // export buttons off the right edge of the display.
  const styles = meta
    ? {
        left: "50%",
        top:
          (selection?.y ?? 0) + (selection?.h ?? 0) + 50 < meta.pointsHeight
            ? (selection?.y ?? 0) + (selection?.h ?? 0) + 10
            : Math.max(8, (selection?.y ?? 0) - 50),
        transform: "translateX(-50%)",
      }
    : undefined;

  if (longShot) {
    return (
      <div className="stage long-shot">
        <img src={longShot.url} alt="" />
        {skippedFrames > 0 && (
          <div className="hint">{t("overlay.longshot.skipped", { frames: skippedFrames })}</div>
        )}
        <div className="toolbar" style={{ left: "50%", top: 12, transform: "translateX(-50%)" }}>
          <button
            className="primary"
            onClick={() => void copyLongShot(longShot.blob)}
            title={t("overlay.longshot.copy.title")}
          >
            {t("overlay.longshot.copy")}
          </button>
          <button onClick={() => void saveLongShot(longShot.blob)}>
            {t("overlay.longshot.save")}
          </button>
          <button onClick={() => void cancel()}>{t("app.close")}</button>
        </div>
      </div>
    );
  }

  return (
    <div className="stage">
      <canvas ref={canvasRef} />
      <div ref={labelRef} className="size-label" style={{ display: "none" }} />
      {!selection && <div className="hint">{t("overlay.select.hint")}</div>}
      {scrollFrames !== null && (
        <div className="hint">{t("overlay.scrolling", { pixels: Math.round(scrollFrames) })}</div>
      )}
      {selection && meta && scrollFrames === null && (
        <Toolbar
          style={styles}
          tool={tool}
          colour={color}
          width={width}
          textSize={textSize}
          busy={busy}
          onTool={(next) => {
            if (textInput) commitText();
            setTool(next);
            if (next !== "select") setSelectedId(null);
          }}
          onColour={changeColor}
          onWidth={changeWidth}
          onTextSize={changeTextSize}
          onScroll={() => {
            const area = selectionRef.current;
            if (area) void startManual(area);
          }}
          onCopy={() => {
            // Text still in the field is part of what the user wants to copy.
            if (textInput) commitText();
            void commit("clipboard");
          }}
          onSave={() => {
            if (textInput) commitText();
            void commit("file");
          }}
          onPin={() => {
            if (textInput) commitText();
            void commit("pin");
          }}
          onCancel={cancel}
        />
      )}
      {textInput && (
        <input
          key={draftSeq}
          ref={textInputRef}
          className="text-input"
          autoFocus
          defaultValue={editingText?.text ?? ""}
          style={{
            left: textInput.x,
            top: textInput.y - textSize - 6,
            width: fieldWidth || undefined,
            color,
            // Match the size the text will be drawn at, so what you see is what you get.
            fontSize: textSize,
          }}
          onChange={fitField}
          onKeyDown={(event) => {
            // Typing must not trigger the tool shortcuts (r/o/a/p…) or the window keys.
            event.stopPropagation();
            if (event.key === "Enter") {
              commitText();
            }
            if (event.key === "Escape") {
              setEditingText(null);
              setTextInput(null);
            }
          }}
        />
      )}
    </div>
  );
}

