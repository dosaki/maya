import { describe, expect, it } from "vitest";
import { makeClickGuard } from "./clickguard";

describe("makeClickGuard", () => {
  it("blocks clicks shortly after a board repaint", () => {
    const g = makeClickGuard(300);
    expect(g.allowClick(1000)).toBe(true);
    g.markPaint(1000);
    expect(g.allowClick(1100)).toBe(false);
    expect(g.allowClick(1301)).toBe(true);
  });

  it("defers repaints while a pointer button is held down", () => {
    const g = makeClickGuard(300);
    expect(g.canPaint()).toBe(true);
    g.setPointerDown(true);
    expect(g.canPaint()).toBe(false);
    g.setPointerDown(false);
    expect(g.canPaint()).toBe(true);
  });
});
