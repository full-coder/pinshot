import type { CSSProperties } from "react";
import { t, type TextKey } from "./i18n";
import type { ToolId } from "./scene";

const TOOLS: { id: ToolId; key: TextKey; hint: string }[] = [
  { id: "select", key: "overlay.tool.select", hint: "V" },
  { id: "rect", key: "overlay.tool.rect", hint: "R" },
  { id: "ellipse", key: "overlay.tool.ellipse", hint: "O" },
  { id: "arrow", key: "overlay.tool.arrow", hint: "A" },
  { id: "pen", key: "overlay.tool.pen", hint: "P" },
  { id: "marker", key: "overlay.tool.marker", hint: "H" },
  { id: "mosaic", key: "overlay.tool.mosaic", hint: "M" },
  { id: "text", key: "overlay.tool.text", hint: "T" },
  { id: "counter", key: "overlay.tool.counter", hint: "N" },
];

const PALETTE = [
  "#e8452c",
  "#f5a623",
  "#f8e71c",
  "#7ed321",
  "#4a90d9",
  "#9013fe",
  "#ffffff",
  "#1c1c1e",
];

type Props = {
  style?: CSSProperties;
  tool: ToolId;
  colour: string;
  width: number;
  textSize: number;
  busy: boolean;
  onTool: (tool: ToolId) => void;
  onColour: (colour: string) => void;
  onWidth: (width: number) => void;
  onTextSize: (size: number) => void;
  onScroll: () => void;
  onCopy: () => void;
  onSave: () => void;
  onPin: () => void;
  onCancel: () => void;
};

/**
 * The two-row toolbar inside the selection: tools + actions, then only the controls the
 * chosen tool actually needs. It is presentational — every change goes back through the
 * callbacks so the overlay stays the single owner of the scene and the selection.
 */
export function Toolbar({
  style,
  tool,
  colour,
  width,
  textSize,
  busy,
  onTool,
  onColour,
  onWidth,
  onTextSize,
  onScroll,
  onCopy,
  onSave,
  onPin,
  onCancel,
}: Props) {
  const options: { color: boolean; label: TextKey } | null =
    tool === "select"
      ? null
      : tool === "text"
        ? { color: true, label: "overlay.option.size" }
        : tool === "mosaic"
          ? { color: false, label: "overlay.option.grain" }
          : { color: true, label: "overlay.option.width" };

  return (
    <div className="toolbar" style={style}>
      {/* Row 1: pick the tool and run the actions. */}
      <div className="toolbar-row">
        <div className="tools">
          {TOOLS.map((entry) => (
            <button
              key={entry.id}
              className={tool === entry.id ? "tool active" : "tool"}
              title={`${t(entry.key)} (${entry.hint})`}
              onClick={() => onTool(entry.id)}
            >
              {t(entry.key)}
            </button>
          ))}
        </div>
        <div className="divider" />
        <button disabled={busy} title={t("overlay.scroll.hint")} onClick={onScroll}>
          {t("overlay.scroll")}
        </button>
        <div className="divider" />
        <button className="primary" disabled={busy} onClick={onCopy}>
          {t("app.copy")}
        </button>
        <button disabled={busy} onClick={onSave}>
          {t("overlay.save")}
        </button>
        <button disabled={busy} onClick={onPin} title={t("overlay.pin.title")}>
          {t("overlay.pin")}
        </button>
        <button disabled={busy} onClick={onCancel} title={t("overlay.cancel.title")}>
          {t("app.cancel")}
        </button>
      </div>

      {/* Row 2: only what the chosen tool actually needs. */}
      {options && (
        <div className="toolbar-row options">
          {options.color && (
            <div className="palette">
              {PALETTE.map((swatch) => (
                <button
                  key={swatch}
                  className={colour === swatch ? "swatch active" : "swatch"}
                  style={{ background: swatch }}
                  title={swatch}
                  onClick={() => onColour(swatch)}
                />
              ))}
            </div>
          )}
          <span className="option-label">{t(options.label)}</span>
          <div
            className="width"
            title={t("overlay.option.wheel", { label: t(options.label) })}
            onWheel={(event) => {
              event.preventDefault();
              const up = event.deltaY < 0;
              if (tool === "text") {
                onTextSize(Math.min(96, Math.max(10, textSize + (up ? 2 : -2))));
              } else {
                onWidth(Math.min(20, Math.max(1, width + (up ? 1 : -1))));
              }
            }}
          >
            <button
              onClick={() =>
                tool === "text"
                  ? onTextSize(Math.max(10, textSize - 2))
                  : onWidth(Math.max(1, width - 1))
              }
              title={t("overlay.smaller")}
            >
              −
            </button>
            <span>{tool === "text" ? textSize : width}</span>
            <button
              onClick={() =>
                tool === "text"
                  ? onTextSize(Math.min(96, textSize + 2))
                  : onWidth(Math.min(20, width + 1))
              }
              title={t("overlay.bigger")}
            >
              +
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
