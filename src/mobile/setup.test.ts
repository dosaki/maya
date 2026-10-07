import { describe, expect, it, vi } from "vitest";
import type { NetworkStatus } from "./network";
import { isFirstRun, renderSetup } from "./setup";

const off: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };

describe("the setup overlay", () => {
  it("is a first run until the server has run or an assistant is paired", () => {
    expect(isFirstRun(off)).toBe(true);
    expect(isFirstRun({ ...off, role: "main" })).toBe(false);
    expect(isFirstRun({ ...off, assistants: [{ id: "a", name: "x", hostname: "x", platform: "macos", address: "", connected: false, lastSeen: null, note: null }] })).toBe(false);
  });

  it("starts with the name and port typed, and explains itself differently the first time", () => {
    const h = { onStart: vi.fn() };
    const first = renderSetup({ firstRun: true, name: "Pixel 8", port: 4127, error: null, busy: false }, h);
    expect(first.textContent).toContain("pair them with the code");
    expect(first.querySelector<HTMLInputElement>("input[name=name]")!.value).toBe("Pixel 8");
    first.querySelector<HTMLInputElement>("input[name=port]")!.value = "4200";
    first.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("Pixel 8", 4200);
    const later = renderSetup({ firstRun: false, name: "Pixel 8", port: 4127, error: "Could not listen on port 4127", busy: true }, h);
    expect(later.textContent).toContain("The server is stopped.");
    expect(later.textContent).toContain("Could not listen on port 4127");
    expect(later.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(true);
  });
});
