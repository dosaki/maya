// The Network tab: this phone's name and port, the pairing code and the
// addresses to type into an assistant, the paired assistants, the two
// notification switches, and the battery exemption.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "../format";
import { promoteLines } from "../promote";
import type { NetworkStatus } from "../settings";
import { showToast } from "../toast";

export type { NetworkStatus } from "../settings";

export interface NetworkModel {
  status: NetworkStatus;
  /** The joined main's host and port, from the config. */
  mainHost: string;
  mainPort: number;
  name: string;
  port: number;
  notifyAwaiting: boolean;
  notifyCompleted: boolean;
  addresses: string[];
  notificationsAllowed: boolean;
  busy: boolean;
  error: string | null;
  /** The assistant whose Make main confirmation is open. */
  promote: string | null;
  /** New key's confirmation is open. */
  newKey?: boolean;
}

export interface NetworkHandlers {
  onSave(name: string, port: number): void;
  onStart(): void;
  onStop(): void;
  onCode(): void;
  onRemove(id: string): void;
  onSwitch(which: "awaiting" | "completed", on: boolean): void;
  onBattery(): void;
  onMakeMain(id: string): void;
  onPromote(id: string): void;
  onPromoteCancel(): void;
  onNewKeyAsk(): void;
  onNewKey(): void;
  onNewKeyCancel(): void;
  onStopListening(): void;
  onJoin(host: string, port: number, code: string): void;
  onMirror(id: string, on: boolean): void;
}

const PLATFORM_NAMES: Record<string, string> = { android: "Android", macos: "macOS", linux: "Linux", windows: "Windows" };

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

const JOIN_HINT = "Open Settings → Network on the main, choose Show pairing code, and type the code it shows.";

function input(label: string, name: string, value: string, type = "text"): { label: HTMLLabelElement; input: HTMLInputElement } {
  const l = el("label", "", `${label} `);
  const i = el("input", "");
  i.name = name;
  i.type = type;
  i.value = value;
  l.append(i);
  return { label: l, input: i };
}

function joinSection(h: NetworkHandlers): HTMLElement {
  const join = section("Join a main");
  const host = input("Host", "host", "");
  const port = input("Port", "joinPort", "4127", "number");
  const code = input("Pairing code", "code", "");
  code.input.inputMode = "numeric";
  join.append(host.label, port.label, code.label, el("p", "settings__hint", JOIN_HINT));
  join.append(button("Join", "join", () => h.onJoin(host.input.value, Number(port.input.value) || 0, code.input.value), true));
  return join;
}

export function renderNetwork(m: NetworkModel, h: NetworkHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = el("div", "settings__body network");
  const running = m.status.role === "main";
  const listening = m.status.role === "assistant";

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
  if (!listening) row.append(running ? button("Stop server", "stop", h.onStop) : button("Start server", "start", h.onStart, true));
  phone.append(name, port, row);
  if (listening) {
    const link = m.status.assistant;
    phone.append(el("p", "network__listening", link.connected && link.mainName ? `Connected to ${link.mainName}` : "Reconnecting…"));
    if (m.mainHost) phone.append(el("p", "settings__hint", `${m.mainHost}:${m.mainPort || 4127}`));
    if (!link.connected && link.error) phone.append(el("p", "settings__hint", link.error));
    phone.append(button("Leave", "leave", h.onStopListening));
  }
  if (m.status.pairAgain) phone.append(el("p", "settings__hint", "This Maya was updated to secure pairing — pair again."));
  if (m.status.mainError) phone.append(el("p", "settings__error", m.status.mainError));
  if (m.error) phone.append(el("p", "settings__error", m.error));
  root.append(phone);
  if (!running && !listening) root.append(joinSection(h));

  if (running) {
    const pairing = section("Pairing");
    const code = m.status.code;
    if (code && nowMs <= code.expiresAt) {
      pairing.append(el("div", "network__code", formatCode(code.code)));
      pairing.append(el("p", "settings__code-expiry", `expires in ${minutesLeft(code.expiresAt, nowMs)} min`));
    } else if (m.status.codeUsedUp) {
      pairing.append(el("p", "settings__hint", "Code used up after 3 wrong tries — Regenerate."));
    }
    pairing.append(button(code && nowMs <= code.expiresAt ? "Regenerate" : "Show pairing code", "regenerate-code", h.onCode));
    pairing.append(el("p", "network__addresses", m.addresses.length > 0 ? `Assistants reach this phone at ${m.addresses.join(", ")}, port ${m.port}.` : "Connect the phone to Wi‑Fi to see its address."));
    root.append(pairing);
  }

  const assistants = section("Assistants");
  if (!listening && m.status.assistants.length === 0) {
    assistants.append(el("p", "", "No assistants paired yet."));
  }
  for (const a of listening ? [] : m.status.assistants) {
    const r = el("div", "network__assistant");
    const text = el("div", "network__assistant-text");
    text.append(el("div", "network__assistant-name", a.name));
    const platform = PLATFORM_NAMES[a.platform] ?? a.platform;
    const seen = a.connected ? "Connected" : a.lastSeen ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected";
    text.append(el("div", "network__assistant-meta", [platform, a.address, seen].filter(Boolean).join(" · ")));
    if (a.note) text.append(el("div", "network__assistant-meta", a.note));
    if (a.wantsMirror || a.mirror) {
      const sw = el("label", "settings__switch");
      const box = el("input", "");
      box.type = "checkbox";
      box.dataset.action = "mirror";
      box.dataset.id = a.id;
      box.checked = !!a.mirror;
      box.addEventListener("change", () => h.onMirror(a.id, box.checked));
      sw.append(box, document.createTextNode(" Sees and controls all sessions"));
      text.append(sw);
    }
    const rm = button("Remove", "remove-assistant", () => h.onRemove(a.id));
    rm.dataset.id = a.id;
    r.append(text);
    if (a.connected && a.canLead) r.append(button("Make main", "make-main", () => h.onMakeMain(a.id)));
    r.append(rm);
    assistants.append(r);
    if (m.promote === a.id && a.connected && a.canLead) {
      const c = el("div", "network__confirm");
      for (const line of promoteLines(m.status.assistants, a.id, "this phone")) c.append(el("p", "", line));
      c.append(button(m.busy ? "Handing over…" : "Make main", "promote-confirm", () => h.onPromote(a.id), true), button("Cancel", "promote-cancel", h.onPromoteCancel));
      assistants.append(c);
    }
  }
  if (!listening) root.append(assistants);

  if (running || listening) {
    const key = section("Key");
    if (m.newKey) {
      const c = el("div", "network__confirm");
      c.append(el("p", "", "Drop every pairing and make a new key?"));
      c.append(button(m.busy ? "Working…" : "Drop and make a new key", "new-key-confirm", h.onNewKey, true), button("Cancel", "new-key-cancel", h.onNewKeyCancel));
      key.append(c);
    } else {
      key.append(button("New key", "new-key", h.onNewKeyAsk));
    }
    root.append(key);
  }

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
  network: { name: string; port: number; mainHost?: string; mainPort?: number };
  notifyOnAwaiting: boolean;
  notifyOnCompleted: boolean;
}

const CARRIED = ["name", "port", "host", "joinPort", "code"] as const;
type Carried = (typeof CARRIED)[number];
export type Edits = Partial<Record<Carried, string>> & { focus?: Carried };

/** Carry forward typed values across repaints: each field that differs from what a fresh render shows, and the focused one. */
export function carryEdits(pane: ParentNode, model: { name: string; port: number }): Edits {
  const fresh: Record<Carried, string> = { name: model.name, port: String(model.port), host: "", joinPort: "4127", code: "" };
  const edits: Edits = {};
  const doc = pane.ownerDocument;
  for (const f of CARRIED) {
    const input = pane.querySelector<HTMLInputElement>(`input[name=${f}]`);
    if (!input) continue;
    if (input.value !== fresh[f]) edits[f] = input.value;
    if (doc && doc.activeElement === input) edits.focus = f;
  }
  return edits;
}

/** Restore carried edits into the pane after a repaint. */
export function restoreEdits(pane: ParentNode, edits: Edits): void {
  for (const f of CARRIED) {
    const input = pane.querySelector<HTMLInputElement>(`input[name=${f}]`);
    if (!input) continue;
    const v = edits[f];
    if (v !== undefined) input.value = v;
    if (edits.focus === f) input.focus();
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
    mainHost: "",
    mainPort: 0,
    status: { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null },
    name: "",
    port: 4127,
    notifyAwaiting: true,
    notifyCompleted: true,
    addresses: [],
    notificationsAllowed: true,
    busy: false,
    error: null,
    promote: null,
    newKey: false,
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
  const loadConfig = async () => {
    config = await invoke<ConfigView>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    model.mainHost = config.network.mainHost ?? "";
    model.mainPort = config.network.mainPort ?? 0;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
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
    onMakeMain: (id) => { model.promote = id; paint(); },
    onPromoteCancel: () => { model.promote = null; paint(); },
    onPromote: (id) => void busy(async () => {
      setStatus(await invoke<NetworkStatus>("network_promote", { id }));
      model.promote = null;
    }),
    onNewKeyAsk: () => { model.newKey = true; paint(); },
    onNewKeyCancel: () => { model.newKey = false; paint(); },
    onNewKey: () => void busy(async () => {
      model.newKey = false;
      setStatus(await invoke<NetworkStatus>("network_new_key"));
      await loadConfig();
    }),
    onStopListening: () => void busy(async () => {
      setStatus(await invoke<NetworkStatus>("listen_stop"));
      await loadConfig();
    }),
    onJoin: (host, port, code) => void busy(async () => {
      setStatus(await invoke<NetworkStatus>("network_pair", { host, port, name: model.name, code }));
      await loadConfig();
    }),
    onMirror: (id, on) => void busy(async () => setStatus(await invoke<NetworkStatus>("network_set_mirror", { id, on }))),
    onBattery: () => void invoke("request_battery_exemption").catch((e) => showToast(String(e))),
  };
  void listen<NetworkStatus>("network", (e) => setStatus(e.payload));
  void (async () => {
    await loadConfig();
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
