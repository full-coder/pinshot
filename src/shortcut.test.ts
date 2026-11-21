import { describe, expect, it } from "vitest";
import { formatShortcut, isModifierCode, keyToken, shortcutFromEvent } from "./shortcut";

const press = (
  code: string,
  mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {},
) => ({
  code,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("shortcutFromEvent", () => {
  it("records a single function key", () => {
    expect(shortcutFromEvent(press("F1"))).toBe("F1");
    expect(shortcutFromEvent(press("F12"))).toBe("F12");
  });

  it("records modifier combinations in the order the parser expects", () => {
    expect(shortcutFromEvent(press("Digit1", { metaKey: true, shiftKey: true }))).toBe(
      "CommandOrControl+Shift+Digit1",
    );
    expect(shortcutFromEvent(press("KeyA", { ctrlKey: true, altKey: true }))).toBe(
      "Control+Alt+KeyA",
    );
  });

  it("refuses a bare letter, which would swallow that key everywhere", () => {
    expect(shortcutFromEvent(press("KeyA"))).toBeNull();
    expect(shortcutFromEvent(press("Digit1"))).toBeNull();
  });

  it("ignores modifier keys and keys the parser does not know", () => {
    expect(shortcutFromEvent(press("ShiftLeft", { shiftKey: true }))).toBeNull();
    expect(shortcutFromEvent(press("MetaLeft", { metaKey: true }))).toBeNull();
    expect(shortcutFromEvent(press("IntlBackslash", { metaKey: true }))).toBeNull();
  });

  it("keeps arrows and named keys", () => {
    expect(shortcutFromEvent(press("ArrowUp", { metaKey: true }))).toBe("CommandOrControl+ArrowUp");
    expect(shortcutFromEvent(press("Space", { ctrlKey: true }))).toBe("Control+Space");
  });
});

describe("keyToken", () => {
  it("accepts exactly what global-hotkey can parse", () => {
    expect(keyToken("KeyQ")).toBe("KeyQ");
    expect(keyToken("Digit7")).toBe("Digit7");
    expect(keyToken("NumpadAdd")).toBe("NumpadAdd");
    expect(keyToken("Comma")).toBe("Comma");
    expect(keyToken("F24")).toBe("F24");
    expect(keyToken("MediaPlayPause")).toBeNull();
    expect(isModifierCode("AltRight")).toBe(true);
    expect(isModifierCode("KeyA")).toBe(false);
  });
});

describe("formatShortcut", () => {
  it("draws the combination the way macOS does", () => {
    expect(formatShortcut("CommandOrControl+Shift+Digit1")).toBe("⇧⌘1");
    expect(formatShortcut("Control+Alt+KeyA")).toBe("⌃⌥A");
    expect(formatShortcut("F1")).toBe("F1");
    expect(formatShortcut("CommandOrControl+ArrowUp")).toBe("⌘↑");
  });

  it("tolerates an empty value", () => {
    expect(formatShortcut("")).toBe("");
  });
});
