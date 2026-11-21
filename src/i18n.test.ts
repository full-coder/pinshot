import { beforeEach, describe, expect, it } from "vitest";
import { getLocale, setLocale, t } from "./i18n";

describe("i18n", () => {
  beforeEach(() => {
    setLocale("zh");
  });

  it("renders Chinese by default and switches to English", () => {
    expect(t("overlay.scroll")).toBe("滚动截屏");
    setLocale("en");
    expect(getLocale()).toBe("en");
    expect(t("overlay.scroll")).toBe("Scrolling capture");
  });

  it("fills placeholders in both languages", () => {
    expect(t("history.count", { count: 3 })).toBe("3 条");
    setLocale("en");
    expect(t("history.count", { count: 3 })).toBe("3 captures");
  });

  it("keeps unknown placeholders instead of printing undefined", () => {
    expect(t("overlay.scrolling", {})).toContain("{pixels}");
  });
});
