import { describe, expect, it, vi } from "vitest";
import type { Card } from "../types";
import { activeColumn, columnWidth, columnsFor, openingColumn, renderStrip, scrollLeftFor, visibleColumns } from "./strip";

const NOW = 1_790_600_000_000;
function card(state: Card["state"]): Card {
  return { sessionId: state, pid: 1, name: "x", cwd: "/x", state, stateSince: NOW, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null, context: null };
}

describe("columns for a width", () => {
  it("gives one column on a phone upright, three or four sideways, never more than four", () => {
    expect(columnsFor(360)).toBe(1);
    expect(columnsFor(411)).toBe(1);
    expect(columnsFor(640)).toBe(2);
    expect(columnsFor(700)).toBe(3);
    expect(columnsFor(800)).toBe(3);
    expect(columnsFor(915)).toBe(4);
    expect(columnsFor(1280)).toBe(4);
  });

  it("sizes columns to fill the width minus the gutters and gaps", () => {
    expect(columnWidth(360, 1)).toBe(336);
    expect(columnWidth(915, 4)).toBe((915 - 24 - 36) / 4);
  });
});

describe("the active column", () => {
  it("follows the scroll position and survives a change of column count", () => {
    const portrait = columnWidth(360, 1);
    expect(activeColumn(0, portrait)).toBe(0);
    expect(activeColumn(scrollLeftFor(2, portrait), portrait)).toBe(2);
    expect(activeColumn(scrollLeftFor(2, portrait) + 40, portrait)).toBe(2);
    // Rotating to landscape (three columns): the same column stays in view.
    const landscape = columnWidth(800, 3);
    const after = scrollLeftFor(activeColumn(scrollLeftFor(2, portrait), portrait), landscape);
    expect(activeColumn(after, landscape)).toBe(2);
    expect(activeColumn(10_000, portrait)).toBe(3);
  });

  it("lists the columns on screen, clamped to the board", () => {
    expect(visibleColumns(0, 1)).toEqual([0]);
    expect(visibleColumns(2, 1)).toEqual([2]);
    expect(visibleColumns(0, 3)).toEqual([0, 1, 2]);
    expect(visibleColumns(2, 3)).toEqual([1, 2, 3]);
    expect(visibleColumns(3, 4)).toEqual([0, 1, 2, 3]);
  });

  it("opens on Awaiting Decision when it has cards, else Working", () => {
    expect(openingColumn([card("idle"), card("awaiting")])).toBe(2);
    expect(openingColumn([card("idle"), card("completed")])).toBe(1);
    expect(openingColumn([])).toBe(1);
  });
});

describe("renderStrip", () => {
  it("names the four columns with counts, marks the visible ones, and picks on tap", () => {
    const onPick = vi.fn();
    const el = renderStrip({ idle: 11, working: 3, awaiting: 1, completed: 0 }, [1, 2], onPick);
    const tabs = [...el.querySelectorAll<HTMLButtonElement>(".strip__tab")];
    expect(tabs.map((t) => t.dataset.column)).toEqual(["idle", "working", "awaiting", "completed"]);
    expect(tabs.map((t) => t.querySelector(".strip__count")!.textContent)).toEqual(["11", "3", "1", "0"]);
    expect(tabs.map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "true", "true", "false"]);
    tabs[3].click();
    expect(onPick).toHaveBeenCalledWith(3);
  });
});
