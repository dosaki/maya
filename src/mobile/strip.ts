// The phone's column strip: which of the four columns are on screen, how
// many fit the width, and where the board scrolls to show one.

import { COLUMNS, type Card, type CardState } from "../types";

/** The gap between columns and the gutter either side, in CSS pixels (dp). */
export const GAP = 12;
export const SIDE = 12;
/** A column narrower than this is unreadable. */
export const MIN_COLUMN = 200;

/** How many columns fit: as many of at least 200 dp as the width takes, one to four. */
export function columnsFor(width: number): number {
  const usable = width - 2 * SIDE;
  for (let n = 4; n > 1; n--) {
    if (n * MIN_COLUMN + (n - 1) * GAP <= usable) return n;
  }
  return 1;
}

export function columnWidth(width: number, n: number): number {
  return (width - 2 * SIDE - (n - 1) * GAP) / n;
}

/** The leftmost column on screen, from the board's scroll position. */
export function activeColumn(scrollLeft: number, colWidth: number): number {
  return Math.max(0, Math.min(COLUMNS.length - 1, Math.round(scrollLeft / (colWidth + GAP))));
}

export function scrollLeftFor(index: number, colWidth: number): number {
  return index * (colWidth + GAP);
}

/** The indices on screen when `active` is the leftmost and `n` fit, kept inside the four. */
export function visibleColumns(active: number, n: number): number[] {
  const first = Math.max(0, Math.min(active, COLUMNS.length - n));
  return Array.from({ length: Math.min(n, COLUMNS.length) }, (_, i) => first + i);
}

/** Awaiting Decision when something waits, else Working. */
export function openingColumn(cards: Card[]): number {
  const wanted: CardState = cards.some((c) => c.state === "awaiting") ? "awaiting" : "working";
  return COLUMNS.findIndex((c) => c.state === wanted);
}

export function renderStrip(counts: Record<CardState, number>, visible: number[], onPick: (index: number) => void): HTMLElement {
  const nav = document.createElement("nav");
  nav.className = "strip";
  nav.setAttribute("aria-label", "Columns");
  COLUMNS.forEach((col, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = `strip__tab strip__tab--${col.state}`;
    b.dataset.column = col.state;
    b.setAttribute("aria-selected", String(visible.includes(i)));
    const name = document.createElement("span");
    name.className = "strip__name";
    name.textContent = col.title;
    const count = document.createElement("span");
    count.className = "strip__count";
    count.textContent = String(counts[col.state]);
    b.append(name, count);
    b.addEventListener("click", () => onPick(i));
    nav.append(b);
  });
  return nav;
}
