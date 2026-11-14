import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import indexHtml from "../index.html?raw";
import { CARD_SIZES, CARD_SIZE_KEY, COLOR_THEMES, COLOR_THEME_KEY, applyCardSize, applyColorTheme, asCardSize, asColorTheme } from "./theme";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  delete document.documentElement.dataset.cards;
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

describe("card sizes", () => {
  it("offers Compact, Roomy and Spacious", () => {
    expect(CARD_SIZES).toEqual([["compact", "Compact"], ["roomy", "Roomy"], ["spacious", "Spacious"]]);
  });

  it("reads an unknown or missing card size as Compact", () => {
    expect(asCardSize("roomy")).toBe("roomy");
    expect(asCardSize("spacious")).toBe("spacious");
    for (const odd of [undefined, null, "", "huge", "ROOMY", 3]) expect(asCardSize(odd)).toBe("compact");
  });

  it("puts the card size on the page and remembers it", () => {
    applyCardSize("compact");
    expect(document.documentElement.dataset.cards).toBe("compact");
    expect(localStorage.getItem(CARD_SIZE_KEY)).toBe("compact");
  });

  it("still sizes the cards when the browser will not store anything", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("storage is off");
    });
    expect(() => applyCardSize("roomy")).not.toThrow();
    expect(document.documentElement.dataset.cards).toBe("roomy");
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

  it("starts with the card size saved last time, Roomy or Spacious", () => {
    for (const saved of ["roomy", "spacious"]) {
      localStorage.setItem(CARD_SIZE_KEY, saved);
      boot();
      expect(document.documentElement.dataset.cards).toBe(saved);
    }
  });

  it("leaves the cards Compact when nothing, Compact or something unknown was saved", () => {
    boot();
    expect(document.documentElement.dataset.cards).toBeUndefined();
    for (const saved of ["compact", "huge"]) {
      localStorage.setItem(CARD_SIZE_KEY, saved);
      boot();
      expect(document.documentElement.dataset.cards).toBeUndefined();
    }
  });

  it("ignores the size 0.57.0 kept, which it kept as Spacious for everyone who had not picked one", () => {
    localStorage.setItem("maya.cardSize", "spacious");
    boot();
    expect(document.documentElement.dataset.cards).toBeUndefined();
  });

  it("uses the same card size key as the theme module", () => {
    expect(bootScript()).toContain(`"${CARD_SIZE_KEY}"`);
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
