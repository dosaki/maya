import type { CardSize, ColorTheme } from "./types";

/** The themes the picker offers. Dark comes first: it is the default, and the look Maya always had. */
export const COLOR_THEMES = [
  ["dark", "Dark"],
  ["light", "Light"],
] as const;

/** Where the page remembers its theme, so the next start can paint it before the saved settings arrive. */
export const COLOR_THEME_KEY = "maya.colorTheme";

/** A saved theme name as a theme: Dark for anything that is not one. */
export function asColorTheme(value: unknown): ColorTheme {
  return COLOR_THEMES.find(([key]) => key === value)?.[0] ?? "dark";
}

/** Puts the theme on the page, and remembers it for the next start's first paint. */
export function applyColorTheme(theme: ColorTheme): void {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem(COLOR_THEME_KEY, theme);
  } catch {
    // Without storage the next start paints dark for a moment; the saved settings still choose the theme once they load.
  }
}

/** The card sizes the picker offers, tightest first. */
export const CARD_SIZES = [
  ["compact", "Compact"],
  ["roomy", "Roomy"],
  ["spacious", "Spacious"],
] as const;

/** Where the page remembers its card size, so the next start paints it before the saved settings arrive.
 *  0.57.0 used `maya.cardSize`, and kept Spacious there for everyone who had not picked a size. */
export const CARD_SIZE_KEY = "maya.cards";

/** A saved card size as a size: Compact, the default, for anything that is not one. */
export function asCardSize(value: unknown): CardSize {
  return CARD_SIZES.find(([key]) => key === value)?.[0] ?? "compact";
}

/** Puts the card size on the page, and remembers it for the next start's first paint. */
export function applyCardSize(size: CardSize): void {
  document.documentElement.dataset.cards = size;
  try {
    localStorage.setItem(CARD_SIZE_KEY, size);
  } catch {
    // Without storage the next start paints Compact for a moment; the saved settings still choose the size once they load.
  }
}
