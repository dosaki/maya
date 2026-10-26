import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import indexHtml from "../index.html?raw";
import { COLOR_THEMES, COLOR_THEME_KEY, applyColorTheme, asColorTheme } from "./theme";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
});

afterEach(() => vi.restoreAllMocks());

describe("colour themes", () => {
  it("offers Dark first, the look Maya always had, then Light", () => {
    expect(COLOR_THEMES).toEqual([["dark", "Dark"], ["light", "Light"]]);
  });

  it("reads a saved theme and falls back to dark for anything else", () => {
    expect(asColorTheme("light")).toBe("light");
    expect(asColorTheme("dark")).toBe("dark");
    for (const odd of [undefined, null, "", "sepia", "LIGHT", 3, {}]) expect(asColorTheme(odd)).toBe("dark");
  });

  it("puts the theme on the page and remembers it for the next start", () => {
    applyColorTheme("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem(COLOR_THEME_KEY)).toBe("light");
    applyColorTheme("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem(COLOR_THEME_KEY)).toBe("dark");
  });

  it("still themes the page when the browser will not store anything", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("storage is off");
    });
    expect(() => applyColorTheme("light")).not.toThrow();
    expect(document.documentElement.dataset.theme).toBe("light");
  });
});

/** The inline script in index.html, which runs before the stylesheet paints. */
function bootScript(): string {
  const match = indexHtml.match(/<script>([\s\S]*?)<\/script>/);
  if (!match) throw new Error("index.html has no inline boot script");
  return match[1];
}

describe("the first paint", () => {
  const boot = () => new Function(bootScript())();

  it("starts light when light was saved last time", () => {
    localStorage.setItem(COLOR_THEME_KEY, "light");
    boot();
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("leaves the page dark when nothing, dark or something unknown was saved", () => {
    boot();
    expect(document.documentElement.dataset.theme).toBeUndefined();
    for (const saved of ["dark", "sepia", ""]) {
      localStorage.setItem(COLOR_THEME_KEY, saved);
      boot();
      expect(document.documentElement.dataset.theme).toBeUndefined();
    }
  });

  it("uses the same storage key as the theme module", () => {
    expect(bootScript()).toContain(`"${COLOR_THEME_KEY}"`);
  });

  it("does not break the page when storage cannot be read", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("storage is off");
    });
    expect(boot).not.toThrow();
  });
});
