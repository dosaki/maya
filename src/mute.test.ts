import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { initMute, renderMuteButton } from "./mute";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("mute button", () => {
  it("shows whether Maya is muted", () => {
    const on = renderMuteButton(false);
    expect(on.getAttribute("aria-pressed")).toBe("false");
    expect(on.getAttribute("aria-label")).toBe("Mute Maya");
    expect(on.className).not.toContain("mute--on");
    const off = renderMuteButton(true);
    expect(off.getAttribute("aria-pressed")).toBe("true");
    expect(off.getAttribute("aria-label")).toBe("Unmute Maya");
    expect(off.className).toContain("mute--on");
    expect(off.title).toContain("still notifies");
  });
});

describe("mute flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<span id="mute-host"></span>';
    invoke.mockReset();
  });

  it("starts from the saved setting and toggles it on each click", async () => {
    let muted = true;
    invoke.mockImplementation((cmd: string, args?: { muted: boolean }) => {
      if (cmd === "get_config") return Promise.resolve({ muted });
      if (cmd === "set_muted") { muted = args!.muted; return Promise.resolve({ muted }); }
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initMute();
    const button = () => document.querySelector<HTMLButtonElement>("#mute-host button")!;
    expect(button().getAttribute("aria-pressed")).toBe("true");
    button().click();
    await flush();
    expect(invoke).toHaveBeenCalledWith("set_muted", { muted: false });
    expect(button().getAttribute("aria-pressed")).toBe("false");
    button().click();
    await flush();
    expect(invoke).toHaveBeenLastCalledWith("set_muted", { muted: true });
    expect(button().getAttribute("aria-pressed")).toBe("true");
  });

  it("keeps the old state when saving fails", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "get_config" ? Promise.resolve({ muted: false }) : Promise.reject(new Error("disk full"))));
    await initMute();
    document.querySelector<HTMLButtonElement>("#mute-host button")!.click();
    await flush();
    expect(document.querySelector("#mute-host button")!.getAttribute("aria-pressed")).toBe("false");
  });
});
