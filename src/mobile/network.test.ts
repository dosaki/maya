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

import { carryEdits, groupSummary, initNetwork, renderNetwork, restoreEdits, type NetworkModel, type NetworkStatus } from "./network";
import { RELEASE_NOTES } from "../release-notes";

const off: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };
const joinedStatus: NetworkStatus = { ...off, role: "assistant", assistant: { connected: true, mainName: "laptop", error: null } };
const base: NetworkModel = { mainHost: "", mainPort: 0, status: joinedStatus, name: "Pixel 8", notifyAwaiting: true, notifyCompleted: false, speak: true, wakeWord: false, notificationsAllowed: true, busy: false, error: null };
// The Network page, where most of these tests look.
const net: NetworkModel = { ...base, group: "network" };
const handlers = () => ({ onSave: vi.fn(), onSwitch: vi.fn(), onBattery: vi.fn(), onNewKeyAsk: vi.fn(), onNewKey: vi.fn(), onNewKeyCancel: vi.fn(), onStopListening: vi.fn(), onJoin: vi.fn(), onOpenGroup: vi.fn(), onBack: vi.fn() });

describe("renderNetwork", () => {
  it("shows Connected to with the main's address and Leave while the phone has joined a main", () => {
    const h = handlers();
    const el = renderNetwork({ ...net, mainHost: "192.168.1.10", mainPort: 4127 }, h);
    expect(el.textContent).toContain("Connected to laptop");
    expect(el.textContent).toContain("192.168.1.10:4127");
    expect(el.querySelector("button[data-action=join]")).toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=leave]")!.click();
    expect(h.onStopListening).toHaveBeenCalled();
  });

  it("offers Join a main while the phone is off, and keeps what was typed across a repaint", () => {
    const h = handlers();
    const notJoined: NetworkModel = { ...net, status: off };
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
      const el = renderNetwork({ ...net, status }, handlers());
      for (const gone of [".network__code", ".network__assistant", "button[data-action=start]", "button[data-action=stop]", "button[data-action=regenerate-code]", "button[data-action=make-main]", "input[name=port]", "input[data-action=mirror]"]) {
        expect(el.querySelector(gone)).toBeNull();
      }
      const headings = [...el.querySelectorAll("h2")].map((h) => h.textContent);
      expect(headings).not.toContain("Pairing");
      expect(headings).not.toContain("Assistants");
    }
  });

  it("says Reconnecting with the last error while it cannot reach the main", () => {
    const lost: NetworkModel = { ...net, status: { ...off, role: "assistant", assistant: { connected: false, mainName: null, error: "The main Maya stopped answering.", retrying: true } } };
    const text = renderNetwork(lost, handlers()).textContent;
    expect(text).toContain("Reconnecting…");
    expect(text).toContain("The main Maya stopped answering.");
  });

  it("shows why the phone could not reconnect at start-up, in the Join section", () => {
    const down: NetworkModel = { ...net, status: { ...off, mainError: "could not read this phone's key" } };
    const el = renderNetwork(down, handlers());
    const join = [...el.querySelectorAll("section")].find((s) => s.textContent?.includes("Join a main"))!;
    expect(join.querySelector(".settings__error")!.textContent).toBe("could not read this phone's key");
  });

  it("saves the name, flips the switches, and asks for the battery exemption", () => {
    const h = handlers();
    const page = renderNetwork(net, h);
    page.querySelector<HTMLInputElement>("input[name=name]")!.value = "Fold";
    page.querySelector<HTMLButtonElement>("button[data-action=save]")!.click();
    expect(h.onSave).toHaveBeenCalledWith("Fold");
    const el = renderNetwork({ ...base, group: "notifications" }, h);
    const completed = el.querySelector<HTMLInputElement>("input[name=notifyCompleted]")!;
    expect(completed.checked).toBe(false);
    completed.checked = true;
    completed.dispatchEvent(new Event("change"));
    expect(h.onSwitch).toHaveBeenCalledWith("completed", true);
    el.querySelector<HTMLButtonElement>("button[data-action=battery]")!.click();
    expect(h.onBattery).toHaveBeenCalled();
    expect(el.textContent).toContain("Android may otherwise stop Maya with the screen off.");
  });

  it("says when Android has notifications off for Maya", () => {
    const el = renderNetwork({ ...base, group: "notifications", notificationsAllowed: false }, handlers());
    expect(el.textContent).toContain("Notifications are off for Maya in Android settings.");
    expect(renderNetwork({ ...base, group: "notifications" }, handlers()).textContent).not.toContain("Notifications are off");
  });

});

describe("Settings as a list of groups", () => {
  it("opens on the list of groups, each with its summary", () => {
    const el = renderNetwork(base, handlers());
    const items = [...el.querySelectorAll<HTMLElement>(".settings-list__item")];
    expect(items.map((i) => i.querySelector(".settings-list__name")!.textContent)).toEqual(["Network", "Notifications", "Voice", "Session viewer", "Release Notes"]);
    expect(items.map((i) => i.querySelector(".settings-list__summary")!.textContent)).toEqual(["Connected to laptop", "Awaiting decision", "Speak on, wake word off", "Chat", `Maya ${RELEASE_NOTES.version}`]);
    expect(el.querySelector("input, select")).toBeNull();
  });

  it("summarises the network and notifications in each state", () => {
    expect(groupSummary({ ...base, status: off }, "network")).toBe("Not joined");
    expect(groupSummary({ ...base, status: { ...joinedStatus, assistant: { connected: false, mainName: null, error: null } } }, "network")).toBe("Reconnecting…");
    expect(groupSummary({ ...base, notifyAwaiting: false }, "notifications")).toBe("Off");
    expect(groupSummary({ ...base, notifyCompleted: true }, "notifications")).toBe("Awaiting decision, Completed");
    expect(groupSummary({ ...base, notificationsAllowed: false }, "notifications")).toBe("Off in Android settings");
    expect(groupSummary({ ...base, speak: false, wakeWord: true }, "voice")).toBe("Speak off, wake word on");
    expect(groupSummary({ ...base, viewerTheme: "terminal" }, "viewer")).toBe("Terminal");
  });

  it("taps into a group and back out", () => {
    const h = handlers();
    renderNetwork(base, h).querySelector<HTMLElement>("[data-group=voice]")!.click();
    expect(h.onOpenGroup).toHaveBeenCalledWith("voice");
    const page = renderNetwork({ ...base, group: "voice" }, h);
    expect(page.querySelector(".settings-page__title")!.textContent).toBe("Voice");
    expect(page.querySelector("input[name=speak]")).not.toBeNull();
    expect(page.querySelector("input[name=notifyAwaiting]")).toBeNull();
    page.querySelector<HTMLElement>("button[data-action=back]")!.click();
    expect(h.onBack).toHaveBeenCalled();
  });

  it("puts Battery under Notifications and the notes under Release Notes", () => {
    expect(renderNetwork({ ...base, group: "notifications" }, handlers()).querySelector("button[data-action=battery]")).not.toBeNull();
    expect(renderNetwork({ ...base, group: "network" }, handlers()).querySelector("button[data-action=battery]")).toBeNull();
    expect(renderNetwork({ ...base, group: "releaseNotes" }, handlers()).querySelector(".release-notes")).not.toBeNull();
  });

  it("shows an error on whichever page is open", () => {
    for (const group of ["network", "notifications", "voice", "viewer"] as const) {
      const errors = [...renderNetwork({ ...base, group, error: "boom" }, handlers()).querySelectorAll(".settings__error")].map((e) => e.textContent);
      expect(errors).toEqual(["boom"]);
    }
  });
});

describe("initNetwork's groups", () => {
  const settle = () => new Promise((r) => setTimeout(r, 0));
  const popped = () => new Promise((r) => window.addEventListener("popstate", () => setTimeout(r, 0), { once: true }));
  const mount = async () => {
    history.replaceState(null, "");
    document.body.innerHTML = '<div id="settings"></div>';
    tauri.replies = { get_config: { network: { name: "Pixel 8" }, notifyOnAwaiting: true, notifyOnCompleted: true }, network_status: joinedStatus, notifications_allowed: true };
    initNetwork(() => undefined);
    await settle();
    return document.getElementById("settings")!;
  };
  const open = (pane: HTMLElement, group: string) => pane.querySelector<HTMLElement>(`[data-group=${group}]`)!.click();

  it("opens a group's page with a history entry, and Android's Back returns to the list", async () => {
    const pane = await mount();
    open(pane, "voice");
    expect(pane.querySelector(".settings-page__title")!.textContent).toBe("Voice");
    expect((history.state as { settings?: string } | null)?.settings).toBe("voice");
    const back = popped();
    history.back();
    await back;
    expect(pane.querySelector(".settings-list")).not.toBeNull();
  });

  it("returns to the list from the back arrow and drops the history entry", async () => {
    const pane = await mount();
    open(pane, "network");
    const back = popped();
    pane.querySelector<HTMLElement>("button[data-action=back]")!.click();
    expect(pane.querySelector(".settings-list")).not.toBeNull();
    await back;
    expect((history.state as { settings?: string } | null)?.settings).toBeUndefined();
    expect(pane.querySelector(".settings-list")).not.toBeNull();
  });

  it("stays on the open page through a repaint", async () => {
    const pane = await mount();
    open(pane, "voice");
    tauri.listeners.network({ payload: joinedStatus });
    await settle();
    expect(pane.querySelector(".settings-page__title")!.textContent).toBe("Voice");
  });

  it("leaves the entry to a card that took it over, so a tapped notification's card stays open", async () => {
    const pane = await mount();
    open(pane, "notifications");
    // A notification tap opens its card first, over the group's entry, then shows Sessions.
    history.replaceState({ card: "s1" }, "");
    const back = vi.spyOn(history, "back");
    window.dispatchEvent(new CustomEvent("maya:tab", { detail: "sessions" }));
    expect(back).not.toHaveBeenCalled();
    expect(history.state).toEqual({ card: "s1" });
    expect(pane.querySelector(".settings-list")).not.toBeNull();
    back.mockRestore();
  });

  it("keeps a failed save's error on its own page, not on the next one opened", async () => {
    const pane = await mount();
    Object.defineProperty(tauri.replies, "set_config", { get: () => Promise.reject(new Error("disk full")), configurable: true });
    open(pane, "voice");
    const speak = pane.querySelector<HTMLInputElement>("input[name=speak]")!;
    speak.checked = false;
    speak.dispatchEvent(new Event("change"));
    await settle();
    expect(pane.querySelector(".settings__error")!.textContent).toContain("disk full");
    pane.querySelector<HTMLElement>("button[data-action=back]")!.click();
    open(pane, "releaseNotes");
    expect(pane.querySelector(".settings__error")).toBeNull();
  });

  it("says a save failed in a toast when its page was left before it finished", async () => {
    const pane = await mount();
    document.body.insertAdjacentHTML("beforeend", '<div id="toast" hidden></div>');
    let fail: (e: Error) => void = () => undefined;
    Object.defineProperty(tauri.replies, "set_config", { get: () => new Promise((_, reject) => { fail = reject; }), configurable: true });
    open(pane, "voice");
    const speak = pane.querySelector<HTMLInputElement>("input[name=speak]")!;
    speak.checked = false;
    speak.dispatchEvent(new Event("change"));
    await settle();
    pane.querySelector<HTMLElement>("button[data-action=back]")!.click();
    fail(new Error("disk full"));
    await settle();
    expect(document.getElementById("toast")!.textContent).toContain("disk full");
    open(pane, "voice");
    expect(pane.querySelector(".settings__error")).toBeNull();
  });

  it("lands on the list again after another tab was shown", async () => {
    const pane = await mount();
    open(pane, "notifications");
    window.dispatchEvent(new CustomEvent("maya:tab", { detail: "sessions" }));
    expect(pane.querySelector(".settings-list")).not.toBeNull();
    await settle();
  });
});

describe("carryEdits and restoreEdits", () => {
  it("carries typed name when it differs from the model", () => {
    const el = renderNetwork(net, handlers());
    const nameInput = el.querySelector<HTMLInputElement>("input[name=name]")!;
    nameInput.value = "Fold";
    const edits = carryEdits(el, net);
    expect(edits.name).toBe("Fold");
  });

  it("carries nothing when inputs match the model", () => {
    const el = renderNetwork(net, handlers());
    const edits = carryEdits(el, net);
    expect(edits.name).toBeUndefined();
  });

  it("restores typed values into fresh inputs", () => {
    const el1 = renderNetwork(net, handlers());
    const nameInput = el1.querySelector<HTMLInputElement>("input[name=name]")!;
    nameInput.value = "Fold";
    const edits = carryEdits(el1, net);
    const el2 = renderNetwork(net, handlers());
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
    pane.querySelector<HTMLElement>("[data-group=notifications]")!.click();
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
    const el = renderNetwork({ ...net, status: { ...off, pairAgain: true } }, handlers());
    expect(el.textContent).toContain("This Maya was updated to secure pairing — pair again.");
  });

  it("offers New key while joined, which asks before it drops every pairing", () => {
    const h = handlers();
    renderNetwork(net, h).querySelector<HTMLButtonElement>("button[data-action=new-key]")!.click();
    expect(h.onNewKeyAsk).toHaveBeenCalled();
    const asking = renderNetwork({ ...net, newKey: true }, h);
    expect(asking.textContent).toContain("Drop every pairing and make a new key?");
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-confirm]")!.click();
    expect(h.onNewKey).toHaveBeenCalled();
  });
});

describe("the Voice card", () => {
  it("has a Voice card with Speak and the wake word", () => {
    const h = handlers();
    const el = renderNetwork({ ...base, group: "voice", speak: true, wakeWord: false }, h);
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
    expect(renderNetwork({ ...base, group: "voice" }, h).textContent).toContain('Wake word "Maya"');
    const joined: NetworkModel = { ...base, group: "voice", status: { ...base.status, assistant: { connected: true, mainName: "desk", error: null, wakeWord: "Jarvis" } } };
    const el = renderNetwork(joined, h);
    expect(el.textContent).toContain('Wake word "Jarvis"');
    expect(el.textContent).toContain('listens for "Jarvis"');
  });
  it("offers the viewer's look and saves the choice", () => {
    const h = { ...handlers(), onViewerTheme: vi.fn() };
    const el = renderNetwork({ ...base, group: "viewer", viewerTheme: "terminal" }, h);
    expect(el.textContent).toContain("Session viewer");
    const sel = el.querySelector<HTMLSelectElement>("select[name=viewerTheme]")!;
    expect(sel.value).toBe("terminal");
    sel.value = "chat";
    sel.dispatchEvent(new Event("change"));
    expect(h.onViewerTheme).toHaveBeenCalledWith("chat");
  });
});
