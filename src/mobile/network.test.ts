import { describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  replies: {} as Record<string, unknown>,
  listeners: {} as Record<string, (e: { payload: unknown }) => void>,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async (cmd: string) => tauri.replies[cmd]) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, f: (e: { payload: unknown }) => void) => {
    tauri.listeners[name] = f;
    return () => undefined;
  }),
}));

import { carryEdits, initNetwork, renderNetwork, restoreEdits, type NetworkModel, type NetworkStatus } from "./network";

const off: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };
const joinedStatus: NetworkStatus = { ...off, role: "assistant", assistant: { connected: true, mainName: "laptop", error: null } };
const base: NetworkModel = { mainHost: "", mainPort: 0, status: joinedStatus, name: "Pixel 8", notifyAwaiting: true, notifyCompleted: false, speak: true, wakeWord: false, notificationsAllowed: true, busy: false, error: null };
const handlers = () => ({ onSave: vi.fn(), onSwitch: vi.fn(), onBattery: vi.fn(), onNewKeyAsk: vi.fn(), onNewKey: vi.fn(), onNewKeyCancel: vi.fn(), onStopListening: vi.fn(), onJoin: vi.fn() });

describe("renderNetwork", () => {
  it("shows Connected to with the main's address and Leave while the phone has joined a main", () => {
    const h = handlers();
    const el = renderNetwork({ ...base, mainHost: "192.168.1.10", mainPort: 4127 }, h);
    expect(el.textContent).toContain("Connected to laptop");
    expect(el.textContent).toContain("192.168.1.10:4127");
    expect(el.querySelector("button[data-action=join]")).toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=leave]")!.click();
    expect(h.onStopListening).toHaveBeenCalled();
  });

  it("offers Join a main while the phone is off, and keeps what was typed across a repaint", () => {
    const h = handlers();
    const notJoined: NetworkModel = { ...base, status: off };
    const el = renderNetwork(notJoined, h);
    el.querySelector<HTMLInputElement>("input[name=host]")!.value = "desk.local";
    el.querySelector<HTMLInputElement>("input[name=code]")!.value = "483921";
    const edits = carryEdits(el, notJoined);
    const again = renderNetwork(notJoined, h);
    restoreEdits(again, edits);
    expect(again.querySelector<HTMLInputElement>("input[name=host]")!.value).toBe("desk.local");
    again.querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("desk.local", 4127, "483921");
  });

  it("has no pairing, assistants, server or port: a phone is never the main", () => {
    for (const status of [off, joinedStatus]) {
      const el = renderNetwork({ ...base, status }, handlers());
      for (const gone of [".network__code", ".network__assistant", "button[data-action=start]", "button[data-action=stop]", "button[data-action=regenerate-code]", "button[data-action=make-main]", "input[name=port]", "input[data-action=mirror]"]) {
        expect(el.querySelector(gone)).toBeNull();
      }
      const headings = [...el.querySelectorAll("h2")].map((h) => h.textContent);
      expect(headings).not.toContain("Pairing");
      expect(headings).not.toContain("Assistants");
    }
  });

  it("says Reconnecting with the last error while it cannot reach the main", () => {
    const lost: NetworkModel = { ...base, status: { ...off, role: "assistant", assistant: { connected: false, mainName: null, error: "The main Maya stopped answering.", retrying: true } } };
    const text = renderNetwork(lost, handlers()).textContent;
    expect(text).toContain("Reconnecting…");
    expect(text).toContain("The main Maya stopped answering.");
  });

  it("shows why the phone could not reconnect at start-up, in the Join section", () => {
    const down: NetworkModel = { ...base, status: { ...off, mainError: "could not read this phone's key" } };
    const el = renderNetwork(down, handlers());
    const join = [...el.querySelectorAll("section")].find((s) => s.textContent?.includes("Join a main"))!;
    expect(join.querySelector(".settings__error")!.textContent).toBe("could not read this phone's key");
  });

  it("saves the name, flips the switches, and asks for the battery exemption", () => {
    const h = handlers();
    const el = renderNetwork(base, h);
    el.querySelector<HTMLInputElement>("input[name=name]")!.value = "Fold";
    el.querySelector<HTMLButtonElement>("button[data-action=save]")!.click();
    expect(h.onSave).toHaveBeenCalledWith("Fold");
    const completed = el.querySelector<HTMLInputElement>("input[name=notifyCompleted]")!;
    expect(completed.checked).toBe(false);
    completed.checked = true;
    completed.dispatchEvent(new Event("change"));
    expect(h.onSwitch).toHaveBeenCalledWith("completed", true);
    el.querySelector<HTMLButtonElement>("button[data-action=battery]")!.click();
    expect(h.onBattery).toHaveBeenCalled();
    expect(el.textContent).toContain("Android may otherwise stop the server with the screen off.");
  });

  it("says when Android has notifications off for Maya", () => {
    const el = renderNetwork({ ...base, notificationsAllowed: false }, handlers());
    expect(el.textContent).toContain("Notifications are off for Maya in Android settings.");
    expect(renderNetwork(base, handlers()).textContent).not.toContain("Notifications are off");
  });

});

describe("carryEdits and restoreEdits", () => {
  it("carries typed name when it differs from the model", () => {
    const el = renderNetwork(base, handlers());
    const nameInput = el.querySelector<HTMLInputElement>("input[name=name]")!;
    nameInput.value = "Fold";
    const edits = carryEdits(el, base);
    expect(edits.name).toBe("Fold");
  });

  it("carries nothing when inputs match the model", () => {
    const el = renderNetwork(base, handlers());
    const edits = carryEdits(el, base);
    expect(edits.name).toBeUndefined();
  });

  it("restores typed values into fresh inputs", () => {
    const el1 = renderNetwork(base, handlers());
    const nameInput = el1.querySelector<HTMLInputElement>("input[name=name]")!;
    nameInput.value = "Fold";
    const edits = carryEdits(el1, base);
    const el2 = renderNetwork(base, handlers());
    restoreEdits(el2, edits);
    const restored = el2.querySelector<HTMLInputElement>("input[name=name]")!;
    expect(restored.value).toBe("Fold");
  });
});

describe("initNetwork", () => {
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it("reads Android's notification permission again on every repaint and after a recheck", async () => {
    document.body.innerHTML = '<div id="settings"></div>';
    tauri.replies = { get_config: { network: { name: "Pixel 8" }, notifyOnAwaiting: true, notifyOnCompleted: true }, network_status: joinedStatus, notifications_allowed: false };
    const recheck = initNetwork(() => undefined);
    await settle();
    const pane = document.getElementById("settings")!;
    expect(pane.textContent).toContain("Notifications are off for Maya in Android settings.");

    // Allowed in the permission prompt: the next repaint drops the line.
    tauri.replies.notifications_allowed = true;
    tauri.listeners.network({ payload: joinedStatus });
    await settle();
    expect(pane.textContent).not.toContain("Notifications are off");

    // Turned off in Android's settings: a recheck brings it back.
    tauri.replies.notifications_allowed = false;
    recheck();
    await settle();
    expect(pane.textContent).toContain("Notifications are off for Maya in Android settings.");
  });
});

describe("secure pairing on the phone", () => {
  it("says an updated phone must pair again", () => {
    const el = renderNetwork({ ...base, status: { ...off, pairAgain: true } }, handlers());
    expect(el.textContent).toContain("This Maya was updated to secure pairing — pair again.");
  });

  it("offers New key while joined, which asks before it drops every pairing", () => {
    const h = handlers();
    renderNetwork(base, h).querySelector<HTMLButtonElement>("button[data-action=new-key]")!.click();
    expect(h.onNewKeyAsk).toHaveBeenCalled();
    const asking = renderNetwork({ ...base, newKey: true }, h);
    expect(asking.textContent).toContain("Drop every pairing and make a new key?");
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-confirm]")!.click();
    expect(h.onNewKey).toHaveBeenCalled();
  });
});

describe("the Voice card", () => {
  it("has a Voice card with Speak and the wake word", () => {
    const h = handlers();
    const el = renderNetwork({ ...base, speak: true, wakeWord: false }, h);
    const speak = el.querySelector<HTMLInputElement>("input[name=speak]")!;
    const wake = el.querySelector<HTMLInputElement>("input[name=wakeWord]")!;
    expect(speak.checked).toBe(true);
    expect(wake.checked).toBe(false);
    wake.checked = true;
    wake.dispatchEvent(new Event("change"));
    expect(h.onSwitch).toHaveBeenCalledWith("wakeWord", true);
    expect(el.textContent).toContain("experimental");
  });
  it("names the main's wake word, and Maya until the main says", () => {
    const h = handlers();
    expect(renderNetwork(base, h).textContent).toContain('Wake word "Maya"');
    const joined = { ...base, status: { ...base.status, assistant: { connected: true, mainName: "desk", error: null, wakeWord: "Jarvis" } } };
    const el = renderNetwork(joined, h);
    expect(el.textContent).toContain('Wake word "Jarvis"');
    expect(el.textContent).toContain('listens for "Jarvis"');
  });
  it("offers the viewer's look and saves the choice", () => {
    const h = { ...handlers(), onViewerTheme: vi.fn() };
    const el = renderNetwork({ ...base, viewerTheme: "terminal" }, h);
    expect(el.textContent).toContain("Session viewer");
    const sel = el.querySelector<HTMLSelectElement>("select[name=viewerTheme]")!;
    expect(sel.value).toBe("terminal");
    sel.value = "chat";
    sel.dispatchEvent(new Event("change"));
    expect(h.onViewerTheme).toHaveBeenCalledWith("chat");
  });
});
