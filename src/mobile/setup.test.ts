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
    const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };
    const first = renderSetup({ firstRun: true, mode: "main", refused: null, name: "Pixel 8", port: 4127, error: null, busy: false }, h);
    expect(first.textContent).toContain("pair them with the code");
    expect(first.querySelector<HTMLInputElement>("input[name=name]")!.value).toBe("Pixel 8");
    first.querySelector<HTMLInputElement>("input[name=port]")!.value = "4200";
    first.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("Pixel 8", 4200);
  });

  it("says the server is stopped with a Start button and the error, and no fields", () => {
    const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };
    const stopped = renderSetup({ firstRun: false, mode: "main", refused: null, name: "Pixel 8", port: 4127, error: "Could not listen on port 4127", busy: false }, h);
    expect(stopped.querySelector("h2")!.textContent).toBe("The server is stopped.");
    expect(stopped.textContent).toContain("Could not listen on port 4127");
    expect(stopped.querySelector("input")).toBeNull();
    stopped.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("Pixel 8", 4127);
    const busy = renderSetup({ firstRun: false, mode: "main", refused: null, name: "Pixel 8", port: 4127, error: null, busy: true }, h);
    expect(busy.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(true);
  });

  it("shows a joined phone's board, or why the main shares nothing", () => {
    const joined: NetworkStatus = { ...off, role: "assistant", assistant: { connected: true, mainName: "desk", error: null, mirror: "allowed" } };
    expect(isFirstRun(joined)).toBe(false);
    expect(setupView(joined, false)).toBe("none");
    expect(setupView({ ...joined, assistant: { ...joined.assistant, mirror: "refused" } }, false)).toBe("board");
    expect(setupView({ ...joined, assistant: { ...joined.assistant, mirror: "unsupported" } }, false)).toBe("board");
    const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };
    const refused = renderSetup({ firstRun: false, mode: "main", name: "Pixel 8", port: 4127, error: null, busy: false, refused: { main: "desk", older: false } }, h);
    expect(refused.textContent).toContain(`desk has not allowed this phone to see its sessions. Turn on "Sees and controls all sessions" for Pixel 8 in desk's Settings → Network.`);
    expect(refused.querySelector("button")).toBeNull();
    const older = renderSetup({ firstRun: false, mode: "main", name: "Pixel 8", port: 4127, error: null, busy: false, refused: { main: "desk", older: true } }, h);
    expect(older.textContent).toContain("desk runs an older Maya that cannot share its sessions. Update it.");
  });

  it("offers Be the main or Join a main on the first run, and joins with what was typed", () => {
    const h = { onStart: vi.fn(), onJoin: vi.fn(), onMode: vi.fn() };
    const first = renderSetup({ firstRun: true, mode: "main", name: "Pixel 8", port: 4127, error: null, busy: false, refused: null }, h);
    first.querySelector<HTMLButtonElement>("button[data-action=mode-join]")!.click();
    expect(h.onMode).toHaveBeenCalledWith("join");
    const join = renderSetup({ firstRun: true, mode: "join", name: "Pixel 8", port: 4127, error: "Wrong or expired pairing code.", busy: false, refused: null }, h);
    expect(join.querySelector("button[data-action=start]")).toBeNull();
    expect(join.textContent).toContain("Wrong or expired pairing code.");
    join.querySelector<HTMLInputElement>("input[name=host]")!.value = "192.168.1.10";
    join.querySelector<HTMLInputElement>("input[name=code]")!.value = "483921";
    join.querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("Pixel 8", "192.168.1.10", 4127, "483921");
  });
});
