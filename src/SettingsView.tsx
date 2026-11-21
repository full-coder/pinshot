import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { formatShortcut, shortcutFromEvent } from "./shortcut";
import { setLocale, systemLocale, t, useLocale, type Locale } from "./i18n";

type Config = {
  general: {
    hotkey_snip: string;
    hotkey_paste: string;
    launch_at_login: boolean;
    language: "auto" | "zh" | "en";
    keep_warm: boolean;
  };
  capture: {
    dim: number;
    magnifier_zoom: number;
  };
  export: { save_dir: string };
  history: { keep_days: number; keep_items: number };
};

export function SettingsView() {
  useLocale();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Only one shortcut field may record at a time. Two armed fields would both receive
  // the same keydown (capture-phase listeners on `window` don't stop each other via
  // stopPropagation) and write one combination into both settings.
  const [recordingHotkey, setRecordingHotkey] = useState<"snip" | "paste" | null>(null);
  const recordingRef = useRef(false);
  recordingRef.current = recordingHotkey !== null;
  // Stable callbacks: an inline arrow here would change `stopRecording` in ShortcutInput
  // on every render, re-binding its capture-phase keydown listener mid-recording.
  const setSnipRecording = useCallback((on: boolean) => setRecordingHotkey(on ? "snip" : null), []);
  const setPasteRecording = useCallback(
    (on: boolean) => setRecordingHotkey(on ? "paste" : null),
    [],
  );

  // Every change is written after a short pause: a slider drag fires dozens of events
  // and each save rewrites config.toml.
  const configRef = useRef<Config | null>(null);
  const timerRef = useRef<number | null>(null);
  const dirtyRef = useRef(false);
  // Saves are serialised so two of them can never land out of order.
  const queueRef = useRef<Promise<unknown>>(Promise.resolve());

  const flush = useCallback(async () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const next = configRef.current;
    if (!next || !dirtyRef.current) return;
    dirtyRef.current = false;
    queueRef.current = queueRef.current.then(async () => {
      try {
        await invoke("settings_save", { config: next });
        setError(null);
        setStatus(t("settings.saved"));
      } catch (cause) {
        // Keep it dirty: the next change (or the close handler) tries again.
        dirtyRef.current = true;
        setError(String(cause));
        setStatus(null);
      }
    });
    await queueRef.current;
  }, []);

  const apply = useCallback(
    (next: Config) => {
      configRef.current = next;
      setConfig(next);
      dirtyRef.current = true;
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => {
        timerRef.current = null;
        void flush();
      }, 400);
    },
    [flush],
  );

  useEffect(() => {
    void (async () => {
      const [loaded, loadWarning] = await invoke<[Config, string | null]>("settings_load");
      configRef.current = loaded;
      setConfig(loaded);
      setWarning(loadWarning);
      applyLanguage(loaded.general.language);
    })().catch((cause) => setError(String(cause)));
  }, []);

  useEffect(() => {
    // WKWebView restores the previous scroll offset when it re-uses a page: without this
    // the window opened halfway down and the language row was never seen.
    const reset = () => rootRef.current?.scrollTo({ top: 0 });
    reset();
    const frame = requestAnimationFrame(reset);
    return () => cancelAnimationFrame(frame);
  }, []);

  // Closing the window right after dragging a slider must not drop that last change.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onCloseRequested(async (event) => {
        // Recording suspends the global shortcuts and resumes them in an effect cleanup.
        // Destroying the window tears down the JS context first, so that cleanup may never
        // run and the shortcuts stay suspended until restart. Resume here as a backstop.
        if (recordingRef.current) void invoke("hotkeys_resume").catch(() => undefined);
        if (!dirtyRef.current) return;
        event.preventDefault();
        await flush();
        await getCurrentWindow().destroy();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      void flush();
    };
  }, [flush]);

  // "Saved" is a receipt, not a state: fade it out so the header stays quiet.
  useEffect(() => {
    if (!status) return;
    const timer = window.setTimeout(() => setStatus(null), 1800);
    return () => window.clearTimeout(timer);
  }, [status]);

  function applyLanguage(language: Config["general"]["language"]) {
    setLocale(language === "auto" ? systemLocale() : (language as Locale));
  }

  function changeLanguage(language: Config["general"]["language"]) {
    if (!config) return;
    const next = { ...config, general: { ...config.general, language } };
    apply(next);
    applyLanguage(language);
    // The language itself has to apply now, not in 400 ms.
    void flush();
  }

  if (error && !config) {
    return <div className="window empty">{error}</div>;
  }
  if (!config) {
    return <div className="window empty">{t("settings.loading")}</div>;
  }

  const patch = <K extends keyof Config>(section: K, value: Partial<Config[K]>) => {
    const base = configRef.current ?? config;
    apply({ ...base, [section]: { ...base[section], ...value } });
  };

  return (
    <div className="window" ref={rootRef}>
      <header className="window-header">
        <h1>{t("settings.title")}</h1>
        <div className="window-actions">
          <span className="muted" role="status">
            {status ?? ""}
          </span>
        </div>
      </header>

      {warning && (
        <div className="banner error">
          {t("settings.warning", { message: warning })}
        </div>
      )}
      {error && <div className="banner error">{error}</div>}

      <section className="group">
        <h2>{t("settings.language")}</h2>
        <Row label={t("settings.language")} hint={t("settings.language.hint")}>
          <select
            className="text short"
            value={config.general.language}
            onChange={(event) =>
              void changeLanguage(event.target.value as Config["general"]["language"])
            }
          >
            <option value="auto">{t("settings.language.auto")}</option>
            <option value="zh">{t("settings.language.zh")}</option>
            <option value="en">{t("settings.language.en")}</option>
          </select>
        </Row>
      </section>

      <section className="group">
        <h2>{t("settings.capture")}</h2>
        <Row label={t("settings.dim")} hint={t("settings.dim.hint")}>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={config.capture.dim}
            onChange={(event) => patch("capture", { dim: Number(event.target.value) })}
          />
          <span className="value">{Math.round(config.capture.dim * 100)}%</span>
        </Row>
        <Row label={t("settings.magnifier")} hint="2–16">
          <input
            type="number"
            min={2}
            max={16}
            value={config.capture.magnifier_zoom}
            onChange={(event) =>
              patch("capture", { magnifier_zoom: clamp(Number(event.target.value), 2, 16) })
            }
          />
        </Row>
        <Row label={t("settings.scroll")} hint={t("settings.scroll.hint")}>
          <span className="muted">{t("settings.scroll.ready")}</span>
        </Row>
        <Row label={t("settings.keep_warm")} hint={t("settings.keep_warm.hint")}>
          <input
            type="checkbox"
            checked={config.general.keep_warm}
            onChange={(event) => patch("general", { keep_warm: event.target.checked })}
          />
        </Row>
      </section>

      <section className="group">
        <h2>{t("settings.export")}</h2>
        <Row label={t("settings.save_dir")}>
          <input
            className="text"
            value={config.export.save_dir}
            onChange={(event) => patch("export", { save_dir: event.target.value })}
          />
        </Row>
      </section>

      <section className="group">
        <h2>{t("settings.history")}</h2>
        <Row label={t("settings.keep_days")} hint={t("settings.keep_days.hint")}>
          <input
            type="number"
            min={1}
            max={365}
            value={config.history.keep_days}
            onChange={(event) =>
              patch("history", { keep_days: clamp(Number(event.target.value), 1, 365) })
            }
          />
        </Row>
        <Row label={t("settings.keep_items")}>
          <input
            type="number"
            min={10}
            max={100000}
            value={config.history.keep_items}
            onChange={(event) =>
              patch("history", { keep_items: clamp(Number(event.target.value), 10, 100000) })
            }
          />
        </Row>
      </section>

      <section className="group">
        <h2>{t("settings.shortcuts")}</h2>
        <Row label={t("settings.hotkey_snip")} hint={t("settings.hotkey_snip.hint")}>
          <ShortcutInput
            value={config.general.hotkey_snip}
            recording={recordingHotkey === "snip"}
            onRecordingChange={setSnipRecording}
            onChange={(next) => patch("general", { hotkey_snip: next })}
          />
        </Row>
        <Row label={t("settings.hotkey_paste")} hint={t("settings.hotkey_paste.hint")}>
          <ShortcutInput
            value={config.general.hotkey_paste}
            recording={recordingHotkey === "paste"}
            onRecordingChange={setPasteRecording}
            onChange={(next) => patch("general", { hotkey_paste: next })}
          />
        </Row>
        <Row label={t("settings.launch_at_login")}>
          <input
            type="checkbox"
            checked={config.general.launch_at_login}
            onChange={(event) => patch("general", { launch_at_login: event.target.checked })}
          />
        </Row>
      </section>
    </div>
  );
}

/**
 * Records a shortcut by pressing it.
 *
 * The app's own global shortcuts are suspended while the field is armed: a registered
 * F1 would otherwise swallow the very keystroke being recorded and start a capture.
 */
function ShortcutInput({
  value,
  recording,
  onRecordingChange,
  onChange,
}: {
  value: string;
  recording: boolean;
  onRecordingChange: (on: boolean) => void;
  onChange: (next: string) => void;
}) {
  const [note, setNote] = useState<string | null>(null);
  const changeRef = useRef(onChange);
  changeRef.current = onChange;
  const stopRecording = useCallback(() => onRecordingChange(false), [onRecordingChange]);

  useEffect(() => {
    if (!recording) return;
    void invoke("hotkeys_suspend").catch(() => undefined);
    window.addEventListener("blur", stopRecording);
    return () => {
      window.removeEventListener("blur", stopRecording);
      void invoke("hotkeys_resume").catch(() => undefined);
    };
  }, [recording]);

  useEffect(() => {
    if (!recording) return;
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat) return;
      if (event.code === "Escape") {
        stopRecording();
        return;
      }
      if (event.code === "Backspace" || event.code === "Delete") {
        changeRef.current("");
        stopRecording();
        return;
      }
      const next = shortcutFromEvent(event);
      if (!next) {
        setNote(t("settings.shortcut.need_modifier"));
        return;
      }
      changeRef.current(next);
      stopRecording();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, stopRecording]);

  return (
    <>
      <button
        type="button"
        className={recording ? "shortcut recording" : "shortcut"}
        onClick={() => {
          setNote(null);
          onRecordingChange(true);
        }}
        title={recording ? t("settings.shortcut.tip_recording") : t("settings.shortcut.tip")}
      >
        {recording
          ? t("settings.shortcut.recording")
          : formatShortcut(value) || t("settings.shortcut.empty")}
      </button>
      {recording && (
        <em className="shortcut-note">
          {t("settings.shortcut.help")}
          {note ? ` · ${note}` : ""}
        </em>
      )}
    </>
  );
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="row">
      <span className="row-label">
        {label}
        {hint && <em>{hint}</em>}
      </span>
      <span className="row-control">{children}</span>
    </label>
  );
}

function clamp(value: number, min: number, max: number) {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}
