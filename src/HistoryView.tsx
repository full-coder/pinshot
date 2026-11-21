import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getLocale, t, useLocale } from "./i18n";

type Item = {
  id: string;
  createdAt: number;
  width: number;
  height: number;
  kind: string;
  favorite: boolean;
  appName: string | null;
};

function formatTime(seconds: number): string {
  const date = new Date(seconds * 1000);
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  return date.toLocaleString(getLocale() === "zh" ? "zh-CN" : "en-US", {
    month: "2-digit",
    day: "2-digit",
    ...(sameDay ? {} : { hour: "2-digit", minute: "2-digit" }),
    hour: sameDay ? "2-digit" : undefined,
    minute: sameDay ? "2-digit" : undefined,
  });
}

export function HistoryView() {
  useLocale();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [items, setItems] = useState<Item[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  /** 「全部清空」 asks once more before it deletes anything. */
  const [confirming, setConfirming] = useState(false);

  const reload = useCallback(async () => {
    try {
      setItems(await invoke<Item[]>("history_list", { limit: 200 }));
      setError(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Same as the settings window: WKWebView can restore an old scroll offset.
    const reset = () => rootRef.current?.scrollTo({ top: 0 });
    reset();
    const frame = requestAnimationFrame(reset);
    void reload();
    return () => cancelAnimationFrame(frame);
  }, [reload]);

  // A confirmation left hanging is forgotten, so a later stray click cannot wipe the
  // history by accident.
  useEffect(() => {
    if (!confirming) return;
    const timer = setTimeout(() => setConfirming(false), 4000);
    return () => clearTimeout(timer);
  }, [confirming]);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 1800);
    return () => clearTimeout(timer);
  }, [toast]);

  async function copy(item: Item) {
    try {
      await invoke("history_copy", { id: item.id });
      setToast(t("history.copied"));
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function extractText(item: Item) {
    try {
      const text = await invoke<string>("ocr_capture", { id: item.id });
      setToast(
        text ? t("history.ocr.copied", { count: text.length }) : t("history.ocr.empty"),
      );
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function toggleFavorite(item: Item) {
    try {
      await invoke("history_favorite", { id: item.id, favorite: !item.favorite });
      setItems((rows) =>
        rows.map((row) => (row.id === item.id ? { ...row, favorite: !row.favorite } : row)),
      );
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function remove(item: Item) {
    try {
      await invoke("history_delete", { id: item.id });
      setItems((rows) => rows.filter((row) => row.id !== item.id));
      setToast(t("history.deleted"));
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function clearAll() {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    try {
      const removed = await invoke<number>("history_clear");
      setItems([]);
      setToast(
        removed > 0 ? t("history.cleared", { count: removed }) : t("history.already_empty"),
      );
    } catch (cause) {
      setError(String(cause));
    }
  }

  async function prune() {
    try {
      const removed = await invoke<number>("history_prune");
      setToast(t("history.pruned", { count: removed }));
      await reload();
    } catch (cause) {
      setError(String(cause));
    }
  }

  return (
    <div className="window" ref={rootRef}>
      <header className="window-header">
        <h1>{t("history.title")}</h1>
        <div className="window-actions">
          <span className="muted">{t("history.count", { count: items.length })}</span>
          <button onClick={() => void prune()} title={t("history.prune.title")}>
            {t("history.prune")}
          </button>
          <button
            className={confirming ? "danger-solid" : undefined}
            disabled={items.length === 0}
            onClick={() => void clearAll()}
            title={t("history.clear.title")}
          >
            {confirming ? t("history.clear.confirm") : t("history.clear")}
          </button>
          <button onClick={() => void reload()}>{t("history.refresh")}</button>
        </div>
      </header>

      {error && <div className="banner error">{error}</div>}
      {toast && <div className="banner ok">{toast}</div>}

      {loading && <div className="empty">{t("history.loading")}</div>}
      {!loading && items.length === 0 && (
        <div className="empty">
          <strong>{t("history.empty.title")}</strong>
          <span>{t("history.empty.hint")}</span>
        </div>
      )}

      <div className="grid">
        {items.map((item) => (
          <HistoryCard
            key={item.id}
            item={item}
            onCopy={() => void copy(item)}
            onText={() => void extractText(item)}
            onFavorite={() => void toggleFavorite(item)}
            onDelete={() => void remove(item)}
          />
        ))}
      </div>
    </div>
  );
}

function HistoryCard({
  item,
  onCopy,
  onText,
  onFavorite,
  onDelete,
}: {
  item: Item;
  onCopy: () => void;
  onText: () => void;
  onFavorite: () => void;
  onDelete: () => void;
}) {
  const [thumb, setThumb] = useState<string | null>(null);

  useEffect(() => {
    let url: string | null = null;
    void (async () => {
      try {
        const buffer = await invoke<ArrayBuffer>("history_thumb", { id: item.id });
        url = URL.createObjectURL(new Blob([buffer], { type: "image/png" }));
        setThumb(url);
      } catch {
        // A missing thumbnail is not worth an error banner: the card still works.
      }
    })();
    return () => {
      if (url) URL.revokeObjectURL(url);
    };
  }, [item.id]);

  return (
    <figure className={`card${item.favorite ? " favorite" : ""}`}>
      <button className="card-shot" onClick={onCopy} title={t("history.copy")}>
        {thumb ? <img src={thumb} alt="" /> : <span className="card-placeholder" />}
      </button>
      <figcaption>
        <span className="card-time">{formatTime(item.createdAt)}</span>
        <span className="muted">
          {item.width}×{item.height}
        </span>
        <span className="card-buttons">
          <button className="text" onClick={onText} title={t("history.ocr")}>
            {getLocale() === "zh" ? "文" : "T"}
          </button>
          <button
            className={item.favorite ? "star active" : "star"}
            onClick={onFavorite}
            title={item.favorite ? t("history.unfavorite") : t("history.favorite")}
          >
            ★
          </button>
          <button className="danger" onClick={onDelete} title={t("history.delete")}>
            ×
          </button>
        </span>
      </figcaption>
    </figure>
  );
}
