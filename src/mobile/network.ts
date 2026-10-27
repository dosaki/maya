// The Network tab: this phone's name, joining a main or leaving it, the
// notification and voice switches, the session viewer and the battery exemption.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { NetworkStatus } from "../settings";
import { showToast } from "../toast";
import { setViewerTheme } from "../modal";
import type { ViewerTheme } from "../types";

export type { NetworkStatus } from "../settings";

export interface NetworkModel {
  /** The session viewer's look; chat when absent. */
  viewerTheme?: ViewerTheme;
  status: NetworkStatus;
  /** The joined main's host and port, from the config. */
  mainHost: string;
  mainPort: number;
  name: string;
  notifyAwaiting: boolean;
  notifyCompleted: boolean;
  speak: boolean;
  wakeWord: boolean;
  notificationsAllowed: boolean;
  busy: boolean;
  error: string | null;
  /** New key's confirmation is open. */
  newKey?: boolean;
}

export interface NetworkHandlers {
  onViewerTheme?(t: ViewerTheme): void;
  onSave(name: string): void;
  onSwitch(which: "awaiting" | "completed" | "speak" | "wakeWord", on: boolean): void;
  onBattery(): void;
  onNewKeyAsk(): void;
  onNewKey(): void;
  onNewKeyCancel(): void;
  onStopListening(): void;
  onJoin(host: string, port: number, code: string): void;
}

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

/** Join a main; `error` is why the phone could not reconnect at start-up, if it could not. */
function joinSection(h: NetworkHandlers, error: string | null): HTMLElement {
  const join = section("Join a main");
  const host = input("Host", "host", "");
  const port = input("Port", "joinPort", "4127", "number");
  const code = input("Pairing code", "code", "");
  code.input.inputMode = "numeric";
  join.append(host.label, port.label, code.label, el("p", "settings__hint", JOIN_HINT));
  join.append(button("Join", "join", () => h.onJoin(host.input.value, Number(port.input.value) || 0, code.input.value), true));
  if (error) join.append(el("p", "settings__error", error));
  return join;
}

export function renderNetwork(m: NetworkModel, h: NetworkHandlers): HTMLElement {
  const root = el("div", "settings__body network");
  const joined = m.status.role === "assistant";

  const phone = section("This phone");
  const name = el("label", "", "Name ");
  const nameInput = el("input", "");
  nameInput.name = "name";
  nameInput.value = m.name;
  name.append(nameInput);
  const row = el("div", "network__row");
  row.append(button("Save", "save", () => h.onSave(nameInput.value)));
  phone.append(name, row);
  if (joined) {
    const link = m.status.assistant;
    phone.append(el("p", "network__listening", link.connected && link.mainName ? `Connected to ${link.mainName}` : "Reconnecting…"));
    if (m.mainHost) phone.append(el("p", "settings__hint", `${m.mainHost}:${m.mainPort || 4127}`));
    if (!link.connected && link.error) phone.append(el("p", "settings__hint", link.error));
    phone.append(button("Leave", "leave", h.onStopListening));
  }
  if (m.status.pairAgain) phone.append(el("p", "settings__hint", "This Maya was updated to secure pairing — pair again."));
  if (m.error) phone.append(el("p", "settings__error", m.error));
  root.append(phone);
  if (!joined) root.append(joinSection(h, m.status.mainError ?? null));

  if (joined) {
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

  const voice = section("Voice");
  // The phone answers to its main's word; "Maya" until the main says.
  const word = m.status.assistant.wakeWord?.trim() || "Maya";
  for (const [which, label, on] of [["speak", "Speak", m.speak], ["wakeWord", `Wake word "${word}" (experimental)`, m.wakeWord]] as const) {
    const l = el("label", "settings__switch");
    const box = el("input", "");
    box.type = "checkbox";
    box.name = which;
    box.checked = on;
    box.addEventListener("change", () => h.onSwitch(which, box.checked));
    l.append(box, document.createTextNode(` ${label}`));
    voice.append(l);
  }
  voice.append(el("p", "settings__hint", `While Maya is on screen she reads out sessions that need you, and with the wake word on she listens for "${word}", the word set on the desktop main. She never listens with the app closed. Voice needs a desktop main.`));
  root.append(voice);

  const viewer = section("Session viewer");
  const viewerSel = el("select", "");
  viewerSel.name = "viewerTheme";
  for (const [value, text] of [["chat", "Chat"], ["terminal", "Terminal"]] as const) {
    const o = el("option", "", text);
    o.value = value;
    viewerSel.append(o);
  }
  viewerSel.value = m.viewerTheme ?? "chat";
  viewerSel.addEventListener("change", () => h.onViewerTheme?.(viewerSel.value as ViewerTheme));
  viewer.append(viewerSel);
  root.append(viewer);

  const battery = section("Battery");
  battery.append(button("Keep Maya awake", "battery", h.onBattery));
  battery.append(el("p", "settings__hint", "Android may otherwise stop Maya with the screen off."));
  root.append(battery);

  for (const b of root.querySelectorAll("button")) if (m.busy) b.disabled = true;
  return root;
}

interface ConfigView {
  viewerTheme?: ViewerTheme;
  network: { name: string; mainHost?: string; mainPort?: number };
  notifyOnAwaiting: boolean;
  notifyOnCompleted: boolean;
  speakNotifications: boolean;
  wakeWord: boolean;
}

const CARRIED = ["name", "host", "joinPort", "code"] as const;
type Carried = (typeof CARRIED)[number];
export type Edits = Partial<Record<Carried, string>> & { focus?: Carried };

/** Carry forward typed values across repaints: each field that differs from what a fresh render shows, and the focused one. */
export function carryEdits(pane: ParentNode, model: { name: string }): Edits {
  const fresh: Record<Carried, string> = { name: model.name, host: "", joinPort: "4127", code: "" };
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
    notifyAwaiting: true,
    notifyCompleted: true,
    speak: true,
    wakeWord: false,
    notificationsAllowed: true,
    busy: false,
    error: null,
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
    model.mainHost = config.network.mainHost ?? "";
    model.mainPort = config.network.mainPort ?? 0;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
    model.speak = config.speakNotifications ?? true;
    model.wakeWord = config.wakeWord ?? false;
    model.viewerTheme = config.viewerTheme ?? "chat";
    setViewerTheme(model.viewerTheme);
  };
  const saveConfig = async (edit: (c: ConfigView) => void) => {
    const full = await invoke<ConfigView>("get_config");
    edit(full);
    config = await invoke<ConfigView>("set_config", { config: full });
    model.name = config.network.name;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
    model.speak = config.speakNotifications ?? true;
    model.wakeWord = config.wakeWord ?? false;
    model.viewerTheme = config.viewerTheme ?? "chat";
    setViewerTheme(model.viewerTheme);
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
    onSave: (name) => void busy(async () => {
      await saveConfig((c) => {
        c.network.name = name;
      });
      showToast("Saved");
    }),
    onSwitch: (which, on) => void busy(() => saveConfig((c) => {
      if (which === "awaiting") c.notifyOnAwaiting = on;
      else if (which === "completed") c.notifyOnCompleted = on;
      else if (which === "speak") c.speakNotifications = on;
      else if (which === "wakeWord") c.wakeWord = on;
    })),
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
    onViewerTheme: (t) => void busy(() => saveConfig((c) => { c.viewerTheme = t; })),
    onBattery: () => void invoke("request_battery_exemption").catch((e) => showToast(String(e))),
  };
  void listen<NetworkStatus>("network", (e) => setStatus(e.payload));
  // The viewer's own toggle saved the look.
  window.addEventListener("maya:config-changed", () => void loadConfig().then(paint).catch(() => undefined));
  void (async () => {
    await loadConfig();
    setStatus(await invoke<NetworkStatus>("network_status"));
  })();
  // Android's notification permission moves without an event.
  setInterval(() => {
    if (!pane.hidden) paint();
  }, 30_000);
  return recheck;
}
