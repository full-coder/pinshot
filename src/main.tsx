import React from "react";
import ReactDOM from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { HistoryView } from "./HistoryView";
import { Overlay } from "./Overlay";
import { PinView } from "./PinView";
import { SettingsView } from "./SettingsView";
import { setLocale } from "./i18n";
import "./styles.css";

// One bundle, four surfaces: the window label decides which one renders.
const label = getCurrentWindow().label;

function surface() {
  if (label.startsWith("pin-")) return <PinView />;
  if (label === "history") return <HistoryView />;
  if (label === "settings") return <SettingsView />;
  return <Overlay />;
}

// Every open surface follows a language switch in Settings, including the pre-warmed
// overlay that is hidden at the time.
void listen<string>("language:changed", (event) => {
  setLocale(event.payload === "en" ? "en" : "zh");
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{surface()}</React.StrictMode>,
);
