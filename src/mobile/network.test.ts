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
const base: NetworkModel = { status: running, name: "Pixel 8", port: 4127, notifyAwaiting: true, notifyCompleted: false, addresses: ["192.168.1.20"], notificationsAllowed: true, busy: false, error: null };
const handlers = () => ({ onSave: vi.fn(), onStart: vi.fn(), onStop: vi.fn(), onCode: vi.fn(), onRemove: vi.fn(), onSwitch: vi.fn(), onBattery: vi.fn() });

describe("renderNetwork", () => {
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
