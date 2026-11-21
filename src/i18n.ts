/**
 * Two-language text catalog (简体中文 / English).
 *
 * The UI strings live here so a window can render in either language without a build
 * step. The choice is stored in `localStorage` (all webviews share it) and in
 * `config.toml` (`general.language`), so the next launch and the Rust-side tray menu
 * agree with what the user picked.
 */

import { useSyncExternalStore } from "react";

export type Locale = "zh" | "en";
export const LOCALES: Locale[] = ["zh", "en"];

const zh = {
  "app.name": "PinShot",
  "app.cancel": "取消",
  "app.close": "关闭",
  "app.copy": "复制",

  "overlay.tool.select": "选择",
  "overlay.tool.rect": "矩形",
  "overlay.tool.ellipse": "椭圆",
  "overlay.tool.arrow": "箭头",
  "overlay.tool.pen": "画笔",
  "overlay.tool.marker": "记号",
  "overlay.tool.mosaic": "马赛克",
  "overlay.tool.text": "文字",
  "overlay.tool.counter": "序号",

  "overlay.option.size": "字号",
  "overlay.option.grain": "粒度",
  "overlay.option.width": "粗细",
  "overlay.option.wheel": "{label}：滚轮可调整",
  "overlay.smaller": "减小",
  "overlay.bigger": "增大",

  "overlay.scroll": "滚动截屏",
  "overlay.scroll.hint": "滚动截屏：按下后框会暂时隐藏，你正常滚动页面（上下都行），按 Enter 出长图（不模拟滚轮、不动光标）",
  "overlay.save": "保存",
  "overlay.pin": "贴图",
  "overlay.pin.title": "把这张图钉在屏幕上（像 Snipaste 的贴图）",
  "overlay.cancel.title": "取消 (Esc)",
  "overlay.select.hint": "拖动选择区域 · Esc 取消",
  "overlay.scrolling": "正在滚动截屏 · 已捕获 {pixels} px · 慢慢滚动页面 · 按 Enter 完成 · Esc 取消",
  "overlay.scroll.cancelled": "已取消滚动截屏",
  "overlay.scroll.failed": "滚动截屏失败，请改用手动截图",
  "overlay.scroll.empty": "没有捕获到新的内容：请确认选区落在可滚动的内容上（页面可能已经到底，或这个窗口不支持滚动）。",
  "overlay.longshot.copy": "复制长图",
  "overlay.longshot.copy.title": "复制长图到剪贴板",
  "overlay.longshot.save": "保存长图",
  "overlay.longshot.skipped": "有 {frames} 帧因为滚动过快被跳过，长图里可能有缺口 —— 下次慢一点更完整",
  "overlay.error.title": "无法开始截图",

  "settings.title": "设置",
  "settings.warning": "配置文件有问题，已回退默认值：{message}",
  "settings.saved": "已保存",
  "settings.loading": "正在读取设置…",
  "settings.language": "语言",
  "settings.language.hint": "立即生效，托盘菜单同步切换",
  "settings.language.auto": "跟随系统",
  "settings.language.zh": "简体中文",
  "settings.language.en": "English",
  "settings.capture": "截图",
  "settings.dim": "选区外遮罩",
  "settings.dim.hint": "0 表示不遮罩，1 表示全黑",
  "settings.magnifier": "放大镜倍率",
  "settings.scroll": "滚动截屏",
  "settings.scroll.hint": "在选区里点「滚动截屏」后，画面会暂时隐藏，正常滚动页面即可，按 Enter 生成完整长图。全程不模拟滚轮、不移动光标。",
  "settings.scroll.ready": "已就绪",
  "settings.keep_warm": "保留预热窗口",
  "settings.keep_warm.hint": "开启：截图框常驻内存（约 80–100 MB），隔多久第一次截图都快；关闭：每次截图 60 秒后释放，下次重新加载（慢一点）",
  "settings.export": "导出",
  "settings.save_dir": "保存目录",
  "settings.history": "历史",
  "settings.keep_days": "保留天数",
  "settings.keep_days.hint": "超过这个天数的截图会被清理（收藏除外）",
  "settings.keep_items": "最多保留条数",
  "settings.shortcuts": "快捷键",
  "settings.hotkey_snip": "截图",
  "settings.hotkey_snip.hint": "点一下，然后直接按键盘",
  "settings.hotkey_paste": "贴图",
  "settings.hotkey_paste.hint": "贴出最近一次截图",
  "settings.launch_at_login": "开机启动",
  "settings.shortcut.recording": "按下快捷键…",
  "settings.shortcut.empty": "未设置",
  "settings.shortcut.help": "Esc 取消 · ⌫ 清除",
  "settings.shortcut.need_modifier": "需要组合键，或直接用 F1–F24",
  "settings.shortcut.tip": "点一下改键",
  "settings.shortcut.tip_recording": "按下想要的组合键 · Esc 取消 · ⌫ 清除",

  "history.title": "历史记录",
  "history.count": "{count} 条",
  "history.prune": "清理",
  "history.prune.title": "按设置里的保留策略清理",
  "history.clear": "全部清空",
  "history.clear.confirm": "确认清空？",
  "history.clear.title": "删除全部历史截图（包括收藏）",
  "history.refresh": "刷新",
  "history.loading": "正在读取本地历史…",
  "history.empty.title": "还没有截图",
  "history.empty.hint": "按 F1 截一张，它就会出现在这里。",
  "history.copied": "已复制到剪贴板",
  "history.deleted": "已删除",
  "history.pruned": "已清理 {count} 条",
  "history.cleared": "已清空 {count} 条",
  "history.already_empty": "历史已经是空的",
  "history.ocr.copied": "已复制 {count} 个字符",
  "history.ocr.empty": "没有识别到文字",
  "history.copy": "复制到剪贴板",
  "history.ocr": "识别文字并复制",
  "history.favorite": "收藏（不会被自动清理）",
  "history.unfavorite": "取消收藏",
  "history.delete": "删除",

  "pin.load_failed": "无法读取这张贴图",
  "pin.close.title": "关闭（双击图片也可以）",
  "pin.menu.copy": "复制图片到剪贴板",
  "pin.menu.close": "关闭贴图",
} as const;

export type TextKey = keyof typeof zh;

const en: Record<TextKey, string> = {
  "app.name": "PinShot",
  "app.cancel": "Cancel",
  "app.close": "Close",
  "app.copy": "Copy",

  "overlay.tool.select": "Select",
  "overlay.tool.rect": "Rectangle",
  "overlay.tool.ellipse": "Ellipse",
  "overlay.tool.arrow": "Arrow",
  "overlay.tool.pen": "Pen",
  "overlay.tool.marker": "Highlighter",
  "overlay.tool.mosaic": "Mosaic",
  "overlay.tool.text": "Text",
  "overlay.tool.counter": "Counter",

  "overlay.option.size": "Size",
  "overlay.option.grain": "Grain",
  "overlay.option.width": "Width",
  "overlay.option.wheel": "{label}: scroll to adjust",
  "overlay.smaller": "Smaller",
  "overlay.bigger": "Bigger",

  "overlay.scroll": "Scrolling capture",
  "overlay.scroll.hint": "Scrolling capture: the overlay hides, scroll the page yourself (either direction) and press Enter for the long image. No synthetic wheel events, no cursor movement.",
  "overlay.save": "Save",
  "overlay.pin": "Pin",
  "overlay.pin.title": "Pin this image to the screen (like Snipaste)",
  "overlay.cancel.title": "Cancel (Esc)",
  "overlay.select.hint": "Drag to select a region · Esc to cancel",
  "overlay.scrolling": "Capturing as you scroll · {pixels} px so far · scroll slowly · Enter to finish · Esc to cancel",
  "overlay.scroll.cancelled": "Scrolling capture cancelled",
  "overlay.scroll.failed": "Scrolling capture failed — try a normal capture instead",
  "overlay.scroll.empty": "Nothing new was captured: make sure the region covers something that scrolls (the page may already be at its end, or this window does not scroll).",
  "overlay.longshot.copy": "Copy long image",
  "overlay.longshot.copy.title": "Copy the long image to the clipboard",
  "overlay.longshot.save": "Save long image",
  "overlay.longshot.skipped": "{frames} frames were skipped because the page moved too fast — the long image may have gaps; scroll a little slower next time",
  "overlay.error.title": "Capture could not start",

  "settings.title": "Settings",
  "settings.warning": "The config file had a problem; defaults were used: {message}",
  "settings.saved": "Saved",
  "settings.loading": "Loading settings…",
  "settings.language": "Language",
  "settings.language.hint": "Applied immediately, including the tray menu",
  "settings.language.auto": "Match system",
  "settings.language.zh": "简体中文",
  "settings.language.en": "English",
  "settings.capture": "Capture",
  "settings.dim": "Dim outside the selection",
  "settings.dim.hint": "0 = no dimming, 1 = fully black",
  "settings.magnifier": "Magnifier zoom",
  "settings.scroll": "Scrolling capture",
  "settings.scroll.hint": "Press the button in the selection, scroll the page yourself, then press Enter for the whole long image. No synthetic wheel events, no cursor movement.",
  "settings.scroll.ready": "Ready",
  "settings.keep_warm": "Keep the capture window ready",
  "settings.keep_warm.hint": "On: the overlay stays in memory (~80–100 MB), so the first capture is fast even after a long break. Off: released 60 s after each capture; the next one reloads (slower).",
  "settings.export": "Export",
  "settings.save_dir": "Save folder",
  "settings.history": "History",
  "settings.keep_days": "Keep for (days)",
  "settings.keep_days.hint": "Captures older than this are pruned (favourites are kept)",
  "settings.keep_items": "Keep at most",
  "settings.shortcuts": "Shortcuts",
  "settings.hotkey_snip": "Capture",
  "settings.hotkey_snip.hint": "Click, then press the keys",
  "settings.hotkey_paste": "Pin",
  "settings.hotkey_paste.hint": "Pins the most recent capture",
  "settings.launch_at_login": "Launch at login",
  "settings.shortcut.recording": "Press a shortcut…",
  "settings.shortcut.empty": "Not set",
  "settings.shortcut.help": "Esc cancels · ⌫ clears",
  "settings.shortcut.need_modifier": "Use a modifier, or a bare F1–F24",
  "settings.shortcut.tip": "Click to change",
  "settings.shortcut.tip_recording": "Press the combination · Esc cancels · ⌫ clears",

  "history.title": "History",
  "history.count": "{count} captures",
  "history.prune": "Prune",
  "history.prune.title": "Apply the retention policy from the settings",
  "history.clear": "Clear all",
  "history.clear.confirm": "Really clear?",
  "history.clear.title": "Delete every capture, favourites included",
  "history.refresh": "Refresh",
  "history.loading": "Reading the local history…",
  "history.empty.title": "No captures yet",
  "history.empty.hint": "Press F1 and it will show up here.",
  "history.copied": "Copied to the clipboard",
  "history.deleted": "Deleted",
  "history.pruned": "Pruned {count}",
  "history.cleared": "Cleared {count}",
  "history.already_empty": "The history is already empty",
  "history.ocr.copied": "Copied {count} characters",
  "history.ocr.empty": "No text was recognised",
  "history.copy": "Copy to the clipboard",
  "history.ocr": "Recognise text and copy it",
  "history.favorite": "Favourite (never pruned)",
  "history.unfavorite": "Remove from favourites",
  "history.delete": "Delete",

  "pin.load_failed": "This pinned image could not be loaded",
  "pin.close.title": "Close (or double-click the image)",
  "pin.menu.copy": "Copy image to clipboard",
  "pin.menu.close": "Close pin",
};

const CATALOG: Record<Locale, Record<TextKey, string>> = { zh, en };

const STORAGE_KEY = "pinshot.locale";

/** The language the OS asks for. */
export function systemLocale(): Locale {
  const language = typeof navigator === "undefined" ? "" : navigator.language ?? "";
  return language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

function detect(): Locale {
  const stored = typeof localStorage === "undefined" ? null : localStorage.getItem(STORAGE_KEY);
  if (stored === "zh" || stored === "en") return stored;
  return systemLocale();
}

let current: Locale = detect();
const listeners = new Set<() => void>();

export function getLocale(): Locale {
  return current;
}

export function setLocale(locale: Locale): void {
  if (locale === current) return;
  current = locale;
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // A locked-down webview may refuse storage; the choice still applies to this window.
  }
  for (const listener of listeners) listener();
}

/** Renders the text for `key`, substituting `{name}` placeholders. */
export function t(key: TextKey, values?: Record<string, string | number>): string {
  const template = CATALOG[current][key] ?? CATALOG.zh[key] ?? key;
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in values ? String(values[name]) : match,
  );
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Subscribe a component to language changes. */
export function useLocale(): Locale {
  return useSyncExternalStore(subscribe, getLocale, getLocale);
}
