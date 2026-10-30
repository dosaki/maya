import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type Handler = (e: { payload: unknown }) => void;
const handlers: Record<string, Handler> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, h: Handler) => {
    handlers[name] = h;
    return Promise.resolve(() => {});
  },
}));

import { CHECK_EVERY_MS, makeUpdates, renderUpdateBar, type UpdateInfo } from "./update";

const info: UpdateInfo = { version: "0.19.0", download: "https://github.com/dosaki/maya/releases/tag/v0.19.0" };
const flush = () => new Promise((r) => setTimeout(r, 0));
const bar = () => document.querySelector("#update-bar .update-bar");
const text = () => document.querySelector("#update-bar .update-bar__text")?.textContent;
const labels = () => [...document.querySelectorAll<HTMLButtonElement>("#update-bar button")].map((b) => b.textContent);
const press = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("#update-bar button")].find((b) => b.textContent === label)!.click();

function backend(kind: string, check: () => Promise<UpdateInfo | null>, install = () => new Promise<void>(() => {}), working: string[] = []) {
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "update_kind") return Promise.resolve(kind);
    if (cmd === "native_working") return Promise.resolve(working);
    if (cmd === "update_check") return check();
    if (cmd === "update_install") return install();
    if (cmd === "open_url") return Promise.resolve();
    return Promise.reject(new Error("unexpected " + cmd));
  });
}

describe("renderUpdateBar", () => {
  it("shows nothing when idle, off, or dismissed for this version", () => {
    expect(renderUpdateBar({ phase: "idle" }, "self", null)).toBeNull();
    expect(renderUpdateBar({ phase: "available", info }, "off", null)).toBeNull();
    expect(renderUpdateBar({ phase: "available", info }, "self", "0.19.0")).toBeNull();
  });

  it("offers Update where Maya installs itself and Download where it cannot", () => {
    const self = renderUpdateBar({ phase: "available", info }, "self", null)!;
    expect(self.querySelector(".update-bar__text")?.textContent).toBe("Maya 0.19.0 is ready.");
    expect([...self.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Update", "Later"]);
    const notice = renderUpdateBar({ phase: "available", info }, "notice", null)!;
    expect(notice.querySelector(".update-bar__text")?.textContent).toBe("Maya 0.19.0 is out.");
    expect([...notice.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Download", "Later"]);
  });

  it("shows the percentage while downloading, none when the size is unknown, and no buttons", () => {
    const some = renderUpdateBar({ phase: "downloading", info, percent: 42 }, "self", null)!;
    expect(some.textContent).toBe("Downloading Maya 0.19.0… 42%");
    expect(some.querySelectorAll("button").length).toBe(0);
    expect(renderUpdateBar({ phase: "downloading", info, percent: null }, "self", null)!.textContent).toBe("Downloading Maya 0.19.0…");
  });

  it("shows a failed install with Try again", () => {
    const el = renderUpdateBar({ phase: "error", info, message: "the update failed: signature" }, "self", null)!;
    expect(el.querySelector(".update-bar__text")?.textContent).toBe("Maya 0.19.0 could not be installed: the update failed: signature");
    expect([...el.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Try again", "Later"]);
  });
});

describe("makeUpdates", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="update-bar" hidden></div>';
    invoke.mockReset();
    vi.useRealTimers();
  });

  it("never checks when the kind is off", async () => {
    backend("off", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    expect(invoke).not.toHaveBeenCalledWith("update_check");
    expect(bar()).toBeNull();
  });

  it("checks at start and shows the bar for a newer release", async () => {
    backend("self", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    expect(text()).toBe("Maya 0.19.0 is ready.");
    expect(document.getElementById("update-bar")!.hidden).toBe(false);
  });

  it("checks again every six hours", async () => {
    vi.useFakeTimers();
    backend("self", () => Promise.resolve(null));
    await makeUpdates().init();
    await vi.advanceTimersByTimeAsync(0);
    const calls = () => invoke.mock.calls.filter((c) => c[0] === "update_check").length;
    expect(calls()).toBe(1);
    await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS);
    expect(calls()).toBe(2);
  });

  it("a failed background check shows nothing", async () => {
    backend("self", () => Promise.reject("could not check for updates: offline"));
    await makeUpdates().init();
    await flush();
    expect(bar()).toBeNull();
  });

  it("Later hides the bar until a newer version appears", async () => {
    let latest: UpdateInfo = info;
    backend("self", () => Promise.resolve(latest));
    const u = makeUpdates();
    await u.init();
    await flush();
    press("Later");
    expect(bar()).toBeNull();
    await u.check();
    expect(bar()).toBeNull();
    latest = { version: "0.20.0", download: "https://github.com/dosaki/maya/releases/tag/v0.20.0" };
    await u.check();
    expect(text()).toBe("Maya 0.20.0 is ready.");
  });

  it("a check from Settings brings a dismissed bar back", async () => {
    backend("self", () => Promise.resolve(info));
    const u = makeUpdates();
    await u.init();
    await flush();
    press("Later");
    await u.check(true);
    expect(text()).toBe("Maya 0.19.0 is ready.");
  });

  it("Update installs, shows progress, and ignores further presses", async () => {
    backend("self", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(invoke).toHaveBeenCalledWith("update_install");
    expect(text()).toBe("Downloading Maya 0.19.0…");
    expect(labels()).toEqual([]);
    handlers["update-progress"]({ payload: { received: 50, total: 200 } });
    expect(text()).toBe("Downloading Maya 0.19.0… 25%");
    handlers["update-progress"]({ payload: { received: 50, total: 0 } });
    expect(text()).toBe("Downloading Maya 0.19.0…");
  });

  it("a background check during the download leaves the bar downloading", async () => {
    backend("self", () => Promise.resolve(info));
    const u = makeUpdates();
    await u.init();
    await flush();
    press("Update");
    await flush();
    await u.check();
    expect(text()).toBe("Downloading Maya 0.19.0…");
  });

  it("a failed install shows the error, and Try again checks before installing", async () => {
    let fail = true;
    backend("self", () => Promise.resolve(info), () => (fail ? Promise.reject("the update failed: signature") : new Promise<void>(() => {})));
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(text()).toBe("Maya 0.19.0 could not be installed: the update failed: signature");
    fail = false;
    invoke.mockClear();
    press("Try again");
    await flush();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(["native_working", "update_check", "update_install"]);
  });

  it("installs the update without asking when no Native session is working", async () => {
    backend("self", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(document.querySelector("#quit-host .modal")).toBeNull();
    expect(invoke).toHaveBeenCalledWith("update_install");
  });

  it("does not install when it cannot tell whether a Native session is working", async () => {
    backend("self", () => Promise.resolve(info));
    const plain = invoke.getMockImplementation()!;
    invoke.mockImplementation((cmd: string) => (cmd === "native_working" ? Promise.reject(new Error("gone")) : plain(cmd)));
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(invoke).not.toHaveBeenCalledWith("update_install");
    expect(text()).toContain("could not tell whether a Native session is working");
  });

  it("asks first when a Native session is working, and installs on Restart anyway", async () => {
    backend("self", () => Promise.resolve(info), undefined, ["fix-ci"]);
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(document.querySelector("#quit-host p")?.textContent).toBe("This Native session is working and will stop when Maya restarts to update: fix-ci.");
    expect(invoke).not.toHaveBeenCalledWith("update_install");
    [...document.querySelectorAll<HTMLButtonElement>("#quit-host button")].find((b) => b.textContent === "Restart anyway")!.click();
    await flush();
    expect(invoke).toHaveBeenCalledWith("update_install");
    expect(text()).toBe("Downloading Maya 0.19.0…");
  });

  it("does not install the update when the user cancels", async () => {
    backend("self", () => Promise.resolve(info), undefined, ["fix-ci", "docs"]);
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    [...document.querySelectorAll<HTMLButtonElement>("#quit-host button")].find((b) => b.textContent === "Cancel")!.click();
    await flush();
    expect(invoke).not.toHaveBeenCalledWith("update_install");
    expect(text()).toBe("Maya 0.19.0 is ready.");
    expect(labels()).toEqual(["Update", "Later"]);
  });

  it("Download opens the release's link and installs nothing", async () => {
    backend("notice", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    press("Download");
    await flush();
    expect(invoke).toHaveBeenCalledWith("open_url", { url: info.download });
    expect(invoke).not.toHaveBeenCalledWith("update_install");
  });
});
