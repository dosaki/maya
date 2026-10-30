import { describe, expect, it, vi } from "vitest";
import { makeGroupHistory } from "./settings-history";

function fakeHistory() {
  const h = {
    state: null as unknown,
    pushState: vi.fn((s: unknown) => { h.state = s; }),
    replaceState: vi.fn((s: unknown) => { h.state = s; }),
    back: vi.fn(() => { h.state = null; }),
  };
  return h;
}

describe("makeGroupHistory", () => {
  it("pushes an entry when a group opens, and replaces it when another opens over it", () => {
    const h = fakeHistory();
    const g = makeGroupHistory(h as unknown as History);
    g.opened("voice");
    expect(h.pushState).toHaveBeenCalledWith({ settings: "voice" }, "");
    expect(g.holding()).toBe(true);
    g.opened("network");
    expect(h.pushState).toHaveBeenCalledTimes(1);
    expect(h.replaceState).toHaveBeenCalledWith({ settings: "network" }, "");
  });

  it("goes back once on close while it holds an entry, and not at all otherwise", () => {
    const h = fakeHistory();
    const g = makeGroupHistory(h as unknown as History);
    g.closed();
    expect(h.back).not.toHaveBeenCalled();
    g.opened("voice");
    g.closed();
    expect(h.back).toHaveBeenCalledTimes(1);
    expect(g.holding()).toBe(false);
  });
});
