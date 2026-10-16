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

const NOW = Date.parse("2026-10-07T10:00:00Z");
const running: NetworkStatus = {
  role: "main",
  code: { code: "483921", expiresAt: NOW + 4 * 60_000 },
  assistants: [
    { id: "a", name: "laptop (10.0.0.2)", hostname: "laptop", platform: "macos", address: "10.0.0.2", connected: true, lastSeen: NOW, note: null },
    { id: "b", name: "laptop (10.0.0.3)", hostname: "laptop", platform: "linux", address: "10.0.0.3", connected: false, lastSeen: NOW - 3_600_000, note: "runs Maya 0.11.0; this phone runs 0.13.0" },
  ],
  assistant: { connected: false, mainName: null, error: null },
  mainError: null,
};
const base: NetworkModel = { mainHost: "", mainPort: 0, status: running, name: "Pixel 8", port: 4127, notifyAwaiting: true, notifyCompleted: false, speak: true, wakeWord: false, addresses: ["192.168.1.20"], notificationsAllowed: true, busy: false, error: null, promote: null };
const handlers = () => ({ onSave: vi.fn(), onStart: vi.fn(), onStop: vi.fn(), onCode: vi.fn(), onRemove: vi.fn(), onSwitch: vi.fn(), onBattery: vi.fn(), onMakeMain: vi.fn(), onPromote: vi.fn(), onPromoteCancel: vi.fn(), onNewKeyAsk: vi.fn(), onNewKey: vi.fn(), onNewKeyCancel: vi.fn(), onStopListening: vi.fn(), onJoin: vi.fn(), onMirror: vi.fn() });

describe("renderNetwork", () => {
  it("shows Connected to with the main's address and Leave while the phone has joined a main", () => {
    const h = handlers();
    const joined: NetworkModel = { ...base, mainHost: "192.168.1.10", mainPort: 4127, status: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "laptop", error: null } } };
    const el = renderNetwork(joined, h, NOW);
    expect(el.textContent).toContain("Connected to laptop");
    expect(el.textContent).toContain("192.168.1.10:4127");
    expect(el.querySelector("button[data-action=start]")).toBeNull();
    expect(el.querySelector("button[data-action=join]")).toBeNull();
    expect(el.querySelector(".network__code")).toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=leave]")!.click();
    expect(h.onStopListening).toHaveBeenCalled();
  });

  it("offers Join a main while the phone is off, and keeps what was typed across a repaint", () => {
    const h = handlers();
    const off: NetworkModel = { ...base, status: { ...running, role: "off", code: null, assistants: [] } };
    const el = renderNetwork(off, h, NOW);
    el.querySelector<HTMLInputElement>("input[name=host]")!.value = "desk.local";
    el.querySelector<HTMLInputElement>("input[name=code]")!.value = "483921";
    const edits = carryEdits(el, off);
    const again = renderNetwork(off, h, NOW);
    restoreEdits(again, edits);
    expect(again.querySelector<HTMLInputElement>("input[name=host]")!.value).toBe("desk.local");
    again.querySelector<HTMLButtonElement>("button[data-action=join]")!.click();
    expect(h.onJoin).toHaveBeenCalledWith("desk.local", 4127, "483921");
    expect(renderNetwork(base, h, NOW).querySelector("button[data-action=join]")).toBeNull();
  });

  it("shows the mirror switch for a Maya that asked, and Android by name", () => {
    const h = handlers();
    const fold = { id: "f", name: "Fold", hostname: "localhost", platform: "android", address: "192.168.1.31", connected: true, lastSeen: null, mirror: false, wantsMirror: true };
    const el = renderNetwork({ ...base, status: { ...running, assistants: [fold] } }, h, NOW);
    expect(el.textContent).toContain("Android");
    const box = el.querySelector<HTMLInputElement>("input[data-action=mirror]")!;
    expect(box.checked).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change"));
    expect(h.onMirror).toHaveBeenCalledWith("f", true);
  });

  it("says Reconnecting with the last error while it cannot reach the main", () => {
    const lost: NetworkModel = { ...base, status: { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: "The main Maya stopped answering.", retrying: true } } };
    const text = renderNetwork(lost, handlers(), NOW).textContent;
    expect(text).toContain("Reconnecting…");
    expect(text).toContain("The main Maya stopped answering.");
  });

  it("offers Make main for a connected assistant that can lead and confirms with the lines", () => {
    const h = handlers();
    const laptop = { id: "g", name: "laptop", hostname: "h", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, canLead: true, canFollow: true };
    const cli = { ...laptop, id: "c", name: "cli", canLead: false };
    const m: NetworkModel = { ...base, status: { ...running, assistants: [laptop, cli] }, promote: "g" };
    const el = renderNetwork(m, h, NOW);
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=make-main]")].length).toBe(1);
    expect(el.textContent).toContain("Make laptop the main Maya? Every connected Maya will assist laptop, and so will this phone.");
    el.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.click();
    expect(h.onPromote).toHaveBeenCalledWith("g");
  });

  it("shows no confirmation for an assistant that is no longer connected", () => {
    const gone = { id: "g", name: "laptop", hostname: "h", platform: "macos", address: "192.168.55.70", connected: false, lastSeen: null, canLead: true, canFollow: true };
    const m: NetworkModel = { ...base, status: { ...running, assistants: [gone] }, promote: "g" };
    expect(renderNetwork(m, handlers(), NOW).querySelector("button[data-action=promote-confirm]")).toBeNull();
  });

  it("shows the code, when it expires, and the phone's addresses while the server runs", () => {
    const el = renderNetwork(base, handlers(), NOW);
    expect(el.querySelector(".network__code")!.textContent).toBe("483 921");
    expect(el.textContent).toContain("expires in 4 min");
    expect(el.textContent).toContain("192.168.1.20");
    expect(el.querySelector<HTMLButtonElement>("button[data-action=stop]")).not.toBeNull();
    expect(el.querySelector("button[data-action=start]")).toBeNull();
  });

  it("lists assistants with the labels the cards use, their state and notes, and removes on tap", () => {
    const h = handlers();
    const el = renderNetwork(base, h, NOW);
    const rows = [...el.querySelectorAll<HTMLElement>(".network__assistant")];
    expect(rows.map((r) => r.querySelector(".network__assistant-name")!.textContent)).toEqual(["laptop (10.0.0.2)", "laptop (10.0.0.3)"]);
    expect(rows[0].textContent).toContain("Connected");
    expect(rows[1].textContent).toContain("Last seen");
    expect(rows[1].textContent).toContain("runs Maya 0.11.0");
    rows[1].querySelector<HTMLButtonElement>("button[data-action=remove-assistant]")!.click();
    expect(h.onRemove).toHaveBeenCalledWith("b");
  });

  it("offers Start with the error under Port while the server is down, and no code", () => {
    const h = handlers();
    const down: NetworkModel = { ...base, status: { ...running, role: "off", code: null, assistants: [], mainError: "Could not listen on port 4127: address in use. Choose another port." } };
    const el = renderNetwork(down, h, NOW);
    expect(el.querySelector(".network__code")).toBeNull();
    expect(el.textContent).toContain("Could not listen on port 4127");
    expect(el.textContent).toContain("No assistants paired yet.");
    el.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalled();
  });

  it("saves name and port, flips the switches, and asks for the battery exemption", () => {
    const h = handlers();
    const el = renderNetwork(base, h, NOW);
    el.querySelector<HTMLInputElement>("input[name=name]")!.value = "Fold";
    el.querySelector<HTMLInputElement>("input[name=port]")!.value = "5000";
    el.querySelector<HTMLButtonElement>("button[data-action=save]")!.click();
    expect(h.onSave).toHaveBeenCalledWith("Fold", 5000);
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
    const el = renderNetwork({ ...base, notificationsAllowed: false }, handlers(), NOW);
    expect(el.textContent).toContain("Notifications are off for Maya in Android settings.");
    expect(renderNetwork(base, handlers(), NOW).textContent).not.toContain("Notifications are off");
  });

  it("drops an expired code and offers to show one", () => {
    const el = renderNetwork(base, handlers(), NOW + 10 * 60_000);
    expect(el.querySelector(".network__code")).toBeNull();
    expect(el.querySelector<HTMLButtonElement>("button[data-action=regenerate-code]")!.textContent).toBe("Show pairing code");
  });
});

describe("carryEdits and restoreEdits", () => {
  it("carries typed name when it differs from the model", () => {
    const el = renderNetwork(base, handlers(), NOW);
    const nameInput = el.querySelector<HTMLInputElement>("input[name=name]")!;
    nameInput.value = "Fold";
    const edits = carryEdits(el, base);
    expect(edits.name).toBe("Fold");
    expect(edits.port).toBeUndefined();
  });

  it("carries typed port when it differs from the model", () => {
    const el = renderNetwork(base, handlers(), NOW);
    const portInput = el.querySelector<HTMLInputElement>("input[name=port]")!;
    portInput.value = "5000";
    const edits = carryEdits(el, base);
    expect(edits.port).toBe("5000");
    expect(edits.name).toBeUndefined();
  });

  it("carries nothing when inputs match the model", () => {
    const el = renderNetwork(base, handlers(), NOW);
    const edits = carryEdits(el, base);
    expect(edits.name).toBeUndefined();
    expect(edits.port).toBeUndefined();
  });

  it("restores typed values into fresh inputs", () => {
    const el1 = renderNetwork(base, handlers(), NOW);
    const nameInput = el1.querySelector<HTMLInputElement>("input[name=name]")!;
    nameInput.value = "Fold";
    const edits = carryEdits(el1, base);
    const el2 = renderNetwork(base, handlers(), NOW);
    restoreEdits(el2, edits);
    const restored = el2.querySelector<HTMLInputElement>("input[name=name]")!;
    expect(restored.value).toBe("Fold");
  });
});

describe("initNetwork", () => {
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it("reads Android's notification permission again on every repaint and after a recheck", async () => {
    document.body.innerHTML = '<div id="settings"></div>';
    tauri.replies = { get_config: { network: { name: "Pixel 8", port: 4127 }, notifyOnAwaiting: true, notifyOnCompleted: true }, local_addresses: [], network_status: running, notifications_allowed: false };
    const recheck = initNetwork(() => undefined);
    await settle();
    const pane = document.getElementById("settings")!;
    expect(pane.textContent).toContain("Notifications are off for Maya in Android settings.");

    // Allowed in the permission prompt: the next repaint drops the line.
    tauri.replies.notifications_allowed = true;
    tauri.listeners.network({ payload: running });
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
  it("says the code is used up, and that an updated phone must pair again", () => {
    const el = renderNetwork({ ...base, status: { ...running, code: null, codeUsedUp: true, pairAgain: true } }, handlers(), NOW);
    expect(el.textContent).toContain("Code used up after 3 wrong tries — Regenerate.");
    expect(el.textContent).toContain("This Maya was updated to secure pairing — pair again.");
  });

  it("offers New key, which asks before it drops every pairing", () => {
    const h = handlers();
    renderNetwork(base, h, NOW).querySelector<HTMLButtonElement>("button[data-action=new-key]")!.click();
    expect(h.onNewKeyAsk).toHaveBeenCalled();
    const asking = renderNetwork({ ...base, newKey: true }, h, NOW);
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
});
