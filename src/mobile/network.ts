// The Network tab: this phone's name and port, the pairing code and the
// addresses to type into an assistant, the paired assistants, the two
// notification switches, and the battery exemption.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "../format";
import type { NetworkStatus } from "../settings";
import { showToast } from "../toast";

export type { NetworkStatus } from "../settings";

export interface NetworkModel {
  status: NetworkStatus;
  name: string;
  port: number;
  notifyAwaiting: boolean;
  notifyCompleted: boolean;
  addresses: string[];
  notificationsAllowed: boolean;
  busy: boolean;
  error: string | null;
}

export interface NetworkHandlers {
  onSave(name: string, port: number): void;
  onStart(): void;
  onStop(): void;
  onCode(): void;
  onRemove(id: string): void;
  onSwitch(which: "awaiting" | "completed", on: boolean): void;
  onBattery(): void;
}

const PLATFORM_NAMES: Record<string, string> = { macos: "macOS", linux: "Linux", windows: "Windows" };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function section(title: string): HTMLElement {
  const s = el("section", "settings__section");
  s.append(el("h2", "settings__heading", title));
  return s;
}

function button(label: string, action: string, onClick: () => void, primary = false): HTMLButtonElement {
  const b = el("button", primary ? "card__btn card__btn--primary" : "card__btn", label);
  b.type = "button";
  b.dataset.action = action;
  b.addEventListener("click", onClick);
  return b;
}

/** "483 921" from "483921". */
function formatCode(code: string): string {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function minutesLeft(expiresAt: number, nowMs: number): number {
  return Math.max(0, Math.ceil((expiresAt - nowMs) / 60_000));
}

export function renderNetwork(m: NetworkModel, h: NetworkHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = el("div", "settings__body network");
  const running = m.status.role === "main";

  const phone = section("This phone");
  const name = el("label", "", "Name ");
  const nameInput = el("input", "");
  nameInput.name = "name";
  nameInput.value = m.name;
  name.append(nameInput);
  const port = el("label", "", "Port ");
  const portInput = el("input", "");
  portInput.name = "port";
  portInput.type = "number";
  portInput.value = String(m.port);
  port.append(portInput);
  const row = el("div", "network__row");
  row.append(button("Save", "save", () => h.onSave(nameInput.value, Number(portInput.value) || 0)));
  row.append(running ? button("Stop server", "stop", h.onStop) : button("Start server", "start", h.onStart, true));
  phone.append(name, port, row);
  if (m.status.mainError) phone.append(el("p", "settings__error", m.status.mainError));
  if (m.error) phone.append(el("p", "settings__error", m.error));
  root.append(phone);

  if (running) {
    const pairing = section("Pairing");
    const code = m.status.code;
    if (code && nowMs <= code.expiresAt) {
      pairing.append(el("div", "network__code", formatCode(code.code)));
      pairing.append(el("p", "settings__code-expiry", `expires in ${minutesLeft(code.expiresAt, nowMs)} min`));
    }
    pairing.append(button(code && nowMs <= code.expiresAt ? "Regenerate" : "Show pairing code", "regenerate-code", h.onCode));
    pairing.append(el("p", "network__addresses", m.addresses.length > 0 ? `Assistants reach this phone at ${m.addresses.join(", ")}, port ${m.port}.` : "Connect the phone to Wi‑Fi to see its address."));
    root.append(pairing);
  }

  const assistants = section("Assistants");
  if (m.status.assistants.length === 0) {
    assistants.append(el("p", "", "No assistants paired yet."));
  }
  for (const a of m.status.assistants) {
    const r = el("div", "network__assistant");
    const text = el("div", "network__assistant-text");
    text.append(el("div", "network__assistant-name", a.name));
    const platform = PLATFORM_NAMES[a.platform] ?? a.platform;
    const seen = a.connected ? "Connected" : a.lastSeen ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected";
    text.append(el("div", "network__assistant-meta", [platform, a.address, seen].filter(Boolean).join(" · ")));
    if (a.note) text.append(el("div", "network__assistant-meta", a.note));
    const rm = button("Remove", "remove-assistant", () => h.onRemove(a.id));
    rm.dataset.id = a.id;
    r.append(text, rm);
    assistants.append(r);
  }
  root.append(assistants);

  const notifications = section("Notifications");
  for (const [which, label, on, nameAttr] of [["awaiting", "Notify on Awaiting Decision", m.notifyAwaiting, "notifyAwaiting"], ["completed", "Notify on Completed", m.notifyCompleted, "notifyCompleted"]] as const) {
    const l = el("label", "settings__switch");
    const box = el("input", "");
    box.type = "checkbox";
    box.name = nameAttr;
    box.checked = on;
    box.addEventListener("change", () => h.onSwitch(which, box.checked));
    l.append(box, document.createTextNode(` ${label}`));
    notifications.append(l);
  }
  if (!m.notificationsAllowed) notifications.append(el("p", "settings__error", "Notifications are off for Maya in Android settings."));
  root.append(notifications);

  const battery = section("Battery");
  battery.append(button("Keep Maya awake", "battery", h.onBattery));
  battery.append(el("p", "settings__hint", "Android may otherwise stop the server with the screen off."));
  root.append(battery);

  for (const b of root.querySelectorAll("button")) if (m.busy) b.disabled = true;
  return root;
}

interface ConfigView {
  network: { name: string; port: number };
  notifyOnAwaiting: boolean;
  notifyOnCompleted: boolean;
}

/** Carry forward name/port edits across repaints. Returns typed values that differ from the model. */
export function carryEdits(pane: ParentNode, model: { name: string; port: number }): { name?: string; port?: string; focus?: "name" | "port" } {
  const nameInput = pane.querySelector<HTMLInputElement>("input[name=name]");
  const portInput = pane.querySelector<HTMLInputElement>("input[name=port]");
  const edits: { name?: string; port?: string; focus?: "name" | "port" } = {};
  if (nameInput && nameInput.value !== model.name) edits.name = nameInput.value;
  if (portInput && portInput.value !== String(model.port)) edits.port = portInput.value;
  const doc = pane.ownerDocument;
  if (doc && nameInput && doc.activeElement === nameInput) edits.focus = "name";
  else if (doc && portInput && doc.activeElement === portInput) edits.focus = "port";
  return edits;
}

/** Restore carried edits into the pane after a repaint. */
export function restoreEdits(pane: ParentNode, edits: ReturnType<typeof carryEdits>): void {
  if (edits.name !== undefined) {
    const nameInput = pane.querySelector<HTMLInputElement>("input[name=name]");
    if (nameInput) nameInput.value = edits.name;
  }
  if (edits.port !== undefined) {
    const portInput = pane.querySelector<HTMLInputElement>("input[name=port]");
    if (portInput) portInput.value = edits.port;
  }
  if (edits.focus === "name") {
    const nameInput = pane.querySelector<HTMLInputElement>("input[name=name]");
    if (nameInput) nameInput.focus();
  } else if (edits.focus === "port") {
    const portInput = pane.querySelector<HTMLInputElement>("input[name=port]");
    if (portInput) portInput.focus();
  }
}

/**
 * Mounts the tab into `#settings`, keeps it current, and reports every
 * status to `onStatus`. Returns a recheck of Android's notification
 * permission, for after something asked for it.
 */
export function initNetwork(onStatus: (s: NetworkStatus) => void): () => void {
  const pane = document.getElementById("settings");
  if (!pane) return () => undefined;
  let config: ConfigView | null = null;
  const model: NetworkModel = {
    status: { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null },
    name: "",
    port: 4127,
    notifyAwaiting: true,
    notifyCompleted: true,
    addresses: [],
    notificationsAllowed: true,
    busy: false,
    error: null,
  };
  const render = () => {
    const edits = carryEdits(pane, model);
    pane.replaceChildren(renderNetwork(model, handlers));
    restoreEdits(pane, edits);
  };
  // The permission changes behind the page (the first run's prompt, Android's
  // settings), so every repaint reads it again and repaints if it moved.
  const recheck = () => {
    void invoke<boolean>("notifications_allowed")
      .catch(() => true)
      .then((allowed) => {
        if (allowed === model.notificationsAllowed) return;
        model.notificationsAllowed = allowed;
        render();
      });
  };
  const paint = () => {
    render();
    recheck();
  };
  const setStatus = (s: NetworkStatus) => {
    model.status = s;
    onStatus(s);
    paint();
  };
  const saveConfig = async (edit: (c: ConfigView) => void) => {
    const full = await invoke<ConfigView>("get_config");
    edit(full);
    config = await invoke<ConfigView>("set_config", { config: full });
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
  };
  const busy = async (f: () => Promise<void>) => {
    model.busy = true;
    model.error = null;
    paint();
    try {
      await f();
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    paint();
  };
  const handlers: NetworkHandlers = {
    onSave: (name, port) => void busy(async () => {
      await saveConfig((c) => {
        c.network.name = name;
        c.network.port = port;
      });
      showToast("Saved");
    }),
    onStart: () => void busy(async () => setStatus(await invoke<NetworkStatus>("server_start"))),
    onStop: () => void busy(async () => setStatus(await invoke<NetworkStatus>("server_stop"))),
    onCode: () => void busy(async () => setStatus(await invoke<NetworkStatus>("network_pairing_code"))),
    onRemove: (id) => void busy(async () => setStatus(await invoke<NetworkStatus>("network_remove_assistant", { id }))),
    onSwitch: (which, on) => void busy(() => saveConfig((c) => {
      if (which === "awaiting") c.notifyOnAwaiting = on;
      else c.notifyOnCompleted = on;
    })),
    onBattery: () => void invoke("request_battery_exemption").catch((e) => showToast(String(e))),
  };
  void listen<NetworkStatus>("network", (e) => setStatus(e.payload));
  void (async () => {
    config = await invoke<ConfigView>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
    model.addresses = await invoke<string[]>("local_addresses").catch(() => []);
    setStatus(await invoke<NetworkStatus>("network_status"));
  })();
  // The code's countdown and the addresses move without an event.
  setInterval(() => {
    void invoke<string[]>("local_addresses").then((a) => { model.addresses = a; }).catch(() => undefined);
    if (!pane.hidden) paint();
  }, 30_000);
  return recheck;
}
