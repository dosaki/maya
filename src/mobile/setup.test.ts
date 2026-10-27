import { describe, expect, it, vi } from "vitest";
import type { NetworkStatus } from "./network";
import { isFirstRun, renderSetup, setupView, type SetupModel } from "./setup";

const off: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };
const paired = [{ id: "a", name: "x", hostname: "x", platform: "macos", address: "", connected: false, lastSeen: null, note: null }];

describe("the setup overlay", () => {
  it("is a first run until the phone has joined, and the overlay covers the tabs only then", () => {
    expect(isFirstRun(off)).toBe(true);
    expect(setupView(off, false)).toBe("overlay");
    expect(setupView(off, true)).toBe("board");
    // A phone saved as a main on 0.34 loads as off, with assistants it no longer has.
    expect(setupView({ ...off, assistants: paired }, false)).toBe("overlay");
  });

  it("shows a joined phone's board, or why the main shares nothing", () => {
    const joined: NetworkStatus = { ...off, role: "assistant", assistant: { connected: true, mainName: "desk", error: null, mirror: "allowed" } };
    expect(isFirstRun(joined)).toBe(false);
    expect(setupView(joined, false)).toBe("none");
    expect(setupView({ ...joined, assistant: { ...joined.assistant, mirror: "refused" } }, false)).toBe("board");
    expect(setupView({ ...joined, assistant: { ...joined.assistant, mirror: "unsupported" } }, false)).toBe("board");
    const h = { onJoin: vi.fn() };
    const refused = renderSetup({ host: "", joinPort: 4127, code: "", firstRun: false, name: "Pixel 8", error: null, busy: false, refused: { main: "desk", older: false } }, h);
    expect(refused.textContent).toContain(`desk has not allowed this phone to see its sessions. Turn on "Sees and controls all sessions" for Pixel 8 in desk's Settings → Network.`);
    expect(refused.querySelector("button")).toBeNull();
    const older = renderSetup({ host: "", joinPort: 4127, code: "", firstRun: false, name: "Pixel 8", error: null, busy: false, refused: { main: "desk", older: true } }, h);
    expect(older.textContent).toContain("desk runs an older Maya that cannot share its sessions. Update it.");
  });

  it("offers only Join on the first run, and joins with what was typed", () => {
    const h = { onJoin: vi.fn() };
    const join = renderSetup({ host: "", joinPort: 4127, code: "", firstRun: true, name: "Pixel 8", error: "Wrong or expired pairing code.", busy: false, refused: null }, h);
    expect(join.querySelector("[data-mode]")).toBeNull();
    expect(join.querySelector("button[data-action^=mode]")).toBeNull();
    expect(join.querySelector("button[data-action=start]")).toBeNull();
    expect(join.textContent).toContain("Wrong or expired pairing code.");
    join.querySelector<HTMLInputElement>("input[name=host]")!.value = "192.168.1.10";
    join.querySelector<HTMLInputElement>("input[name=code]")!.value = "483921";
    join.querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("Pixel 8", "192.168.1.10", 4127, "483921");
  });

  it("shows why the phone could not reconnect on the Join card, after the first run too", () => {
    const h = { onJoin: vi.fn() };
    const card = renderSetup({ host: "", joinPort: 4127, code: "", firstRun: false, name: "Pixel 8", error: "could not read this phone's key", busy: false, refused: null }, h);
    expect(card.textContent).toContain("could not read this phone's key");
    expect(card.querySelector("button[data-action=join]")).not.toBeNull();
  });

  it("renders the join form from the model, and keeps what is typed across a repaint", () => {
    const h = { onJoin: vi.fn() };
    const model: SetupModel = { firstRun: true, name: "Pixel 8", host: "192.168.1.10", joinPort: 4200, code: "483921", error: null, busy: false, refused: null };
    const join = renderSetup(model, h);
    expect(join.querySelector<HTMLInputElement>("input[name=host]")!.value).toBe("192.168.1.10");
    expect(join.querySelector<HTMLInputElement>("input[name=joinPort]")!.value).toBe("4200");
    expect(join.querySelector<HTMLInputElement>("input[name=code]")!.value).toBe("483921");
    // Typing writes to the model, so the card rebuilt after an error shows it again.
    const typed = (card: HTMLElement, name: string, value: string) => {
      const input = card.querySelector<HTMLInputElement>(`input[name=${name}]`)!;
      input.value = value;
      input.dispatchEvent(new Event("input"));
    };
    typed(join, "host", "10.0.0.2");
    typed(join, "name", "Fold");
    model.error = "Wrong or expired pairing code.";
    expect(model.host).toBe("10.0.0.2");
    const repaint = renderSetup(model, h);
    expect(repaint.querySelector<HTMLInputElement>("input[name=host]")!.value).toBe("10.0.0.2");
    expect(repaint.querySelector<HTMLInputElement>("input[name=name]")!.value).toBe("Fold");
  });
});
