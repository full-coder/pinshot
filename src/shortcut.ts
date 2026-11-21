/**
 * Turning a key press into the string this app stores in `config.toml`.
 *
 * Two rules matter:
 *  - the text has to parse with `global-hotkey` (`CommandOrControl+Shift+Digit1`), because
 *    that is what Rust registers;
 *  - a bare letter is refused. A global shortcut that is just `A` would swallow that key
 *    in every app; only function keys are allowed on their own.
 */

export type KeyEventLike = {
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
};

const NAMED: Record<string, string> = {
  Space: "Space",
  Escape: "Escape",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  PrintScreen: "PrintScreen",
  ScrollLock: "ScrollLock",
  Pause: "Pause",
  Minus: "Minus",
  Equal: "Equal",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Backslash: "Backslash",
  Backquote: "Backquote",
  NumpadAdd: "NumpadAdd",
  NumpadSubtract: "NumpadSubtract",
  NumpadMultiply: "NumpadMultiply",
  NumpadDivide: "NumpadDivide",
  NumpadDecimal: "NumpadDecimal",
  NumpadEnter: "NumpadEnter",
  NumpadEqual: "NumpadEqual",
};

/** The `KeyboardEvent.code` as a token `global-hotkey` understands, or null. */
export function keyToken(code: string): string | null {
  if (NAMED[code]) return NAMED[code];
  if (/^Key[A-Z]$/.test(code)) return code;
  if (/^Digit[0-9]$/.test(code)) return code;
  if (/^Numpad[0-9]$/.test(code)) return code;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  return null;
}

/** True for the modifier keys themselves, which never end a recording. */
export function isModifierCode(code: string): boolean {
  return /^(Shift|Control|Alt|Meta|Fn)(Left|Right)?$/.test(code);
}

export function shortcutFromEvent(event: KeyEventLike): string | null {
  const key = keyToken(event.code);
  if (!key) return null;
  const mods: string[] = [];
  // On macOS the Command key; on Windows/Linux the plugin maps it to Control as well.
  if (event.metaKey) mods.push("CommandOrControl");
  if (event.ctrlKey) mods.push("Control");
  if (event.altKey) mods.push("Alt");
  if (event.shiftKey) mods.push("Shift");
  if (mods.length === 0 && !/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return null;
  return [...mods, key].join("+");
}

const MODIFIER_GLYPHS: Record<string, string> = {
  CommandOrControl: "⌘",
  Command: "⌘",
  Cmd: "⌘",
  Super: "⌘",
  Control: "⌃",
  Ctrl: "⌃",
  Alt: "⌥",
  Option: "⌥",
  Shift: "⇧",
};

const KEY_LABELS: Record<string, string> = {
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Space: "Space",
  Escape: "Esc",
  Backspace: "⌫",
  Delete: "⌦",
  Enter: "↩",
  Tab: "⇥",
  PageUp: "PgUp",
  PageDown: "PgDn",
  Minus: "-",
  Equal: "=",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Backquote: "`",
};

/** `CommandOrControl+Shift+Digit1` → `⇧⌘1`, for the button in the settings window. */
export function formatShortcut(value: string): string {
  if (!value.trim()) return "";
  const tokens = value.split("+").map((token) => token.trim()).filter(Boolean);
  let modifiers = "";
  let key = "";
  for (const token of tokens) {
    const glyph = MODIFIER_GLYPHS[token];
    if (glyph) {
      modifiers += glyph;
      continue;
    }
    key = KEY_LABELS[token] ?? token.replace(/^Key/, "").replace(/^Digit/, "").replace(/^Numpad/, "Num");
  }
  // macOS order: control, option, shift, command.
  const ordered = ["⌃", "⌥", "⇧", "⌘"]
    .map((glyph) => (modifiers.includes(glyph) ? glyph : ""))
    .join("");
  return `${ordered}${key}`;
}
