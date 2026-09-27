import { describe, expect, it } from "vitest";
import { makeClickGuard } from "./clickguard";

describe("makeClickGuard", () => {
  it("blocks clicks only in a very short window after a board repaint", () => {
    const g = makeClickGuard({ clickWindowMs: 120, settleMs: 300 });
    expect(g.allowClick(1000)).toBe(true);
    g.markPaint(1000);
    expect(g.allowClick(1100)).toBe(false);
    expect(g.allowClick(1121)).toBe(true);
  });

  it("defers repaints while a pointer button is held down", () => {
    const g = makeClickGuard({ clickWindowMs: 120, settleMs: 300 });
    expect(g.canPaint(5000)).toBe(true);
    g.setPointerDown(true);
    expect(g.canPaint(5000)).toBe(false);
    g.setPointerDown(false);
    expect(g.canPaint(5000)).toBe(true);
  });

  it("defers repaints until the pointer has settled over the board", () => {
    const g = makeClickGuard({ clickWindowMs: 120, settleMs: 300 });
    g.markPointerMove(1000);
    expect(g.canPaint(1200)).toBe(false);
    expect(g.canPaint(1301)).toBe(true);
    g.markPointerLeave();
    g.markPointerMove(2000);
    g.markPointerLeave();
    expect(g.canPaint(2001)).toBe(true);
  });
});
