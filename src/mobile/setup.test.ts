import { describe, expect, it, vi } from "vitest";
import type { NetworkStatus } from "./network";
import { isFirstRun, renderSetup, setupView } from "./setup";

const off: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };
const paired = [{ id: "a", name: "x", hostname: "x", platform: "macos", address: "", connected: false, lastSeen: null, note: null }];

describe("the setup overlay", () => {
  it("is a first run until the server has run or an assistant is paired", () => {
    expect(isFirstRun(off)).toBe(true);
    expect(isFirstRun({ ...off, role: "main" })).toBe(false);
    expect(isFirstRun({ ...off, assistants: paired })).toBe(false);
  });

  it("covers the tabs on a first run only; a stopped server shows on the board", () => {
    expect(setupView(off, false)).toBe("overlay");
    expect(setupView({ ...off, assistants: paired }, false)).toBe("board");
    expect(setupView(off, true)).toBe("board");
    expect(setupView({ ...off, role: "main" }, true)).toBe("none");
    expect(setupView({ ...off, role: "main", assistants: paired }, false)).toBe("none");
  });

  it("starts with the name and port typed, and explains itself differently the first time", () => {
    const h = { onStart: vi.fn() };
    const first = renderSetup({ firstRun: true, name: "Pixel 8", port: 4127, error: null, busy: false }, h);
    expect(first.textContent).toContain("pair them with the code");
    expect(first.querySelector<HTMLInputElement>("input[name=name]")!.value).toBe("Pixel 8");
    first.querySelector<HTMLInputElement>("input[name=port]")!.value = "4200";
    first.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("Pixel 8", 4200);
  });

  it("says the server is stopped with a Start button and the error, and no fields", () => {
    const h = { onStart: vi.fn() };
    const stopped = renderSetup({ firstRun: false, name: "Pixel 8", port: 4127, error: "Could not listen on port 4127", busy: false }, h);
    expect(stopped.querySelector("h2")!.textContent).toBe("The server is stopped.");
    expect(stopped.textContent).toContain("Could not listen on port 4127");
    expect(stopped.querySelector("input")).toBeNull();
    stopped.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("Pixel 8", 4127);
    const busy = renderSetup({ firstRun: false, name: "Pixel 8", port: 4127, error: null, busy: true }, h);
    expect(busy.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(true);
  });
});
