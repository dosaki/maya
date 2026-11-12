// The phone's Settings tab, laid out as Android's own: a list of groups,
// each opening its page. Network (this phone's name, joining a main or
// leaving it), Notifications (the switches and the battery exemption),
// Voice, the session viewer, and the release notes.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { NetworkStatus } from "../settings";
import { showToast } from "../toast";
import { setViewerTheme } from "../modal";
import { renderReleaseNotes, RUNNING_VERSION } from "../release-notes";
import { makeGroupHistory } from "./settings-history";
import type { ViewerTheme } from "../types";
import { morph } from "../morph";
import { renderDevices, type DevicesHandlers, type DevicesModel } from "../devices";
import { btn, card, choice, footRow, group, notice, row, screenHead, state, toggle } from "../settings-ui";
import art from "../assets/maya-full-body.png";

export type { NetworkStatus } from "../settings";

export type PhoneGroup = "network" | "devices" | "notifications" | "voice" | "viewer" | "releaseNotes";

/** The list's cards of groups, in its order. */
export const PHONE_LIST: readonly (readonly PhoneGroup[])[] = [["network", "devices"], ["notifications", "voice"], ["viewer"], ["releaseNotes"]];

const PHONE_NAMES: Record<PhoneGroup, string> = {
  network: "Network",
  devices: "Devices",
  notifications: "Notifications",
  voice: "Voice",
  viewer: "Session viewer",
  releaseNotes: "Release Notes",
};

/** The line under a page's title. Release Notes' names the running version, so it is built with the page. */
export const PHONE_LEDES: Partial<Record<PhoneGroup, string>> = {
  network: "This phone joins the Maya on your computer.",
  devices: "The computers and phones in your group.",
  notifications: "When the phone tells you a session needs you.",
  voice: "While Maya is on screen. She never listens with the app closed.",
};

export interface NetworkModel {
  /** The group whose page is open; the list when absent or null. */
  group?: PhoneGroup | null;
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
  /** The device whose Join form is open. */
  devicesJoining?: string | null;
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
  /**
   * The Devices list's handlers. The list keeps its first render's root
   * across repaints, so this must be one object for the life of the page.
   */
  devices: DevicesHandlers;
  onOpenGroup(g: PhoneGroup): void;
  onBack(): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

const JOIN_HINT = "Open Settings → Network on the main, choose Show pairing code, and type the code it shows.";

/** The actions at the foot of a card, side by side or one full-width. */
function bar(...buttons: HTMLButtonElement[]): HTMLElement {
  const b = el("div", "settings__bar");
  b.append(...buttons);
  return b;
}

/** A field under its label. */
function field(label: string, name: string, value: string, type = "text"): { row: HTMLElement; input: HTMLInputElement } {
  const input = el("input", "");
  input.name = name;
  input.type = type;
  input.value = value;
  return { row: row({ label, control: input, stack: true }), input };
}

/** Two fields in one row, each under its label. */
function splitFields(...fields: { label: string; input: HTMLInputElement }[]): HTMLElement {
  const r = el("div", "settings__row settings__row--split");
  for (const f of fields) {
    const l = el("label", "settings__field");
    l.append(el("span", "settings__label", f.label), f.input);
    r.append(l);
  }
  return r;
}

function plainInput(name: string, value: string, type = "text"): HTMLInputElement {
  const i = el("input", "");
  i.name = name;
  i.type = type;
  i.value = value;
  return i;
}

/** Join a main; `error` is why the phone could not reconnect at start-up, if it could not. */
function joinGroup(h: NetworkHandlers, error: string | null): HTMLElement {
  const host = field("Host", "host", "");
  const port = plainInput("joinPort", "4127", "number");
  const code = plainInput("code", "");
  code.inputMode = "numeric";
  const join = btn("Join", { action: "join", kind: "gold" }, () => h.onJoin(host.input.value, Number(port.value) || 0, code.value));
  const g = group({ title: "Join a main", note: JOIN_HINT }, card(host.row, splitFields({ label: "Port", input: port }, { label: "Pairing code", input: code }), bar(join)));
  if (error) g.append(notice(error, "error"));
  return g;
}

/** This phone, then Join a main (or Key, once joined). */
function networkCards(m: NetworkModel, h: NetworkHandlers): HTMLElement[] {
  const joined = m.status.role === "assistant";
  const name = field("Name", "name", m.name);
  const save = btn("Save name", { action: "save" }, () => h.onSave(name.input.value));
  const out: HTMLElement[] = [];
  if (m.status.pairAgain) out.push(notice("This Maya was updated to secure pairing — pair again.", "info"));
  if (!joined) {
    out.push(joinGroup(h, m.status.mainError ?? null), group({ title: "This phone" }, card(name.row, bar(save))));
    return out;
  }
  const link = m.status.assistant;
  const rows: Node[] = [name.row];
  const where = m.mainHost ? el("span", "settings__addr mono", `${m.mainHost}:${m.mainPort || 4127}`) : undefined;
  rows.push(footRow(link.connected && link.mainName ? state(`Connected to ${link.mainName}`, "on") : state("Reconnecting…", "off"), where));
  if (!link.connected && link.error) rows.push(row({ help: link.error }));
  rows.push(bar(save, btn("Leave", { action: "leave", kind: "danger" }, h.onStopListening)));
  out.push(group({ title: "This phone" }, card(...rows)));

  const keyRows: Node[] = [];
  if (m.newKey) {
    keyRows.push(
      row({ label: "Drop every pairing and make a new key?" }),
      bar(btn(m.busy ? "Working…" : "Drop and make a new key", { action: "new-key-confirm", kind: "danger" }, h.onNewKey), btn("Cancel", { action: "new-key-cancel" }, h.onNewKeyCancel)),
    );
  } else {
    keyRows.push(row({ label: "New key", help: "Drops every pairing, so each device must pair again." }), bar(btn("New key", { action: "new-key", kind: "danger" }, h.onNewKeyAsk)));
  }
  out.push(group({ title: "Key" }, card(...keyRows)));
  return out;
}

/** The notification switches, then Battery, which keeps them coming with the screen off. */
function notificationCards(m: NetworkModel, h: NetworkHandlers): HTMLElement[] {
  const switches = card(
    row({ label: "Awaiting Decision", control: toggle("notifyAwaiting", m.notifyAwaiting, (on) => h.onSwitch("awaiting", on)) }),
    row({ label: "Completed", control: toggle("notifyCompleted", m.notifyCompleted, (on) => h.onSwitch("completed", on)) }),
  );
  const notifications = group({}, switches);
  if (!m.notificationsAllowed) notifications.append(notice("Notifications are off for Maya in Android settings.", "error"));
  const battery = group(
    { title: "Battery" },
    card(
      row({ help: "Android may otherwise stop Maya with the screen off, and her notifications with her." }),
      bar(btn("Keep Maya awake", { action: "battery" }, h.onBattery)),
    ),
  );
  return [notifications, battery];
}

function voiceCards(m: NetworkModel, h: NetworkHandlers): HTMLElement[] {
  // The phone answers to its main's word; "Maya" until the main says.
  const word = m.status.assistant.wakeWord?.trim() || "Maya";
  return [
    group(
      { foot: "Voice needs a desktop main." },
      card(
        row({ label: "Speak", help: "Reads out sessions that need you.", control: toggle("speak", m.speak, (on) => h.onSwitch("speak", on)) }),
        row({ label: `Wake word "${word}"`, help: "Experimental. The word is set on your computer.", control: toggle("wakeWord", m.wakeWord, (on) => h.onSwitch("wakeWord", on)) }),
      ),
    ),
  ];
}

function viewerCards(m: NetworkModel, h: NetworkHandlers): HTMLElement[] {
  const options = [["chat", "Chat"], ["terminal", "Terminal"]] as const;
  return [group({}, card(row({ label: "Show sessions as", control: choice<ViewerTheme>("viewerTheme", options, m.viewerTheme ?? "chat", (t) => h.onViewerTheme?.(t)) })))];
}

function releaseNotesCards(): HTMLElement[] {
  return [group({}, renderReleaseNotes())];
}

/** The group's devices: this phone and the others. A phone is never a main, so no pairing and no Make main. */
function devicesCards(m: NetworkModel, h: NetworkHandlers): HTMLElement[] {
  if (m.status.role === "off") {
    const none = el("p", "settings__group-note", "Join a main in Settings › Network to see your devices.");
    none.dataset.row = "devices-off";
    return [group({}, none)];
  }
  // The model's fields behind the view; a typed host stays in its input.
  const view: DevicesModel = {
    get joining() { return m.devicesJoining ?? null; },
    set joining(v) { m.devicesJoining = v; },
    joinHost: "",
    joinPort: 0,
    promote: null,
    promoting: false,
  };
  return [group({}, renderDevices(m.status, view, h.devices, Date.now(), "this phone", { stateUnderName: true }))];
}

const CARDS: Record<PhoneGroup, (m: NetworkModel, h: NetworkHandlers) => HTMLElement[]> = {
  network: networkCards,
  devices: devicesCards,
  notifications: notificationCards,
  voice: voiceCards,
  viewer: viewerCards,
  releaseNotes: releaseNotesCards,
};

/** The line under a group's name in the list: what is set under it. */
export function groupSummary(m: NetworkModel, g: PhoneGroup): string {
  switch (g) {
    case "network": {
      if (m.status.role !== "assistant") return "Not joined";
      const link = m.status.assistant;
      return link.connected && link.mainName ? `Connected to ${link.mainName}` : "Reconnecting…";
    }
    case "devices": {
      if (m.status.role !== "assistant") return "Not joined";
      const n = m.status.devices?.length ?? 0;
      return n === 1 ? "1 device" : `${n} devices`;
    }
    case "notifications": {
      if (!m.notificationsAllowed) return "Off in Android settings";
      const on = [m.notifyAwaiting && "Awaiting decision", m.notifyCompleted && "Completed"].filter(Boolean);
      return on.length > 0 ? on.join(", ") : "Off";
    }
    case "voice":
      return `Speak ${m.speak ? "on" : "off"}, wake word ${m.wakeWord ? "on" : "off"}`;
    case "viewer":
      return m.viewerTheme === "terminal" ? "Terminal" : "Chat";
    case "releaseNotes":
      return `Maya ${RUNNING_VERSION}`;
  }
}

/** The list of groups, or the open group's page: a back link, its title and lede, the error if any, then its cards. */
export function renderNetwork(m: NetworkModel, h: NetworkHandlers): HTMLElement {
  if (!m.group) {
    const list = el("div", "settings__body settings-list");
    const head = el("div", "settings__head");
    head.append(el("h2", "settings__title", "Settings"));
    list.append(head);
    for (const names of PHONE_LIST) {
      const rows = names.map((g) => {
        const item = el("button", "settings-list__item");
        item.type = "button";
        item.dataset.group = g;
        const summary = el("span", "settings-list__summary");
        const text = groupSummary(m, g);
        summary.append(g === "network" && m.status.role === "assistant" && m.status.assistant.connected && m.status.assistant.mainName ? state(text, "on") : text);
        item.append(el("span", "settings-list__name", PHONE_NAMES[g]), summary);
        item.addEventListener("click", () => h.onOpenGroup(g));
        return item;
      });
      list.append(card(...rows));
    }
    return list;
  }
  const root = el("div", "settings__body network");
  root.dataset.group = m.group;
  const back = el("button", "settings-page__back", "Settings");
  back.type = "button";
  back.dataset.action = "back";
  back.addEventListener("click", h.onBack);
  root.append(back);
  const title = PHONE_NAMES[m.group];
  const lede = m.group === "releaseNotes" ? `You're running Maya ${RUNNING_VERSION}.` : PHONE_LEDES[m.group];
  const head = screenHead(title, lede ?? "");
  if (!lede) head.querySelector(".settings__lede")?.remove();
  if (m.group === "releaseNotes") {
    const banner = el("img", "settings__rn-banner");
    banner.src = art;
    banner.alt = "";
    head.classList.add("settings__rn-head");
    root.append(banner);
  }
  root.append(head);
  // On the page the user is on, whichever group's save failed.
  // Keyed, so a repaint that adds or clears it leaves the cards (and the list's hover) where they are.
  if (m.error) {
    const err = notice(m.error, "error");
    err.dataset.row = "error";
    root.append(err);
  }
  root.append(...CARDS[m.group](m, h));
  for (const b of root.querySelectorAll<HTMLButtonElement>("button:not([data-action=back])")) if (m.busy) b.disabled = true;
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
  // What was typed but not saved, kept while its page is closed (another
  // group, the list, another tab) and put back when it opens again.
  let kept: Edits = {};
  const render = () => {
    const edits = carryEdits(pane, model);
    // A field on screen says for itself whether it still differs.
    for (const f of CARRIED) if (pane.querySelector(`input[name=${f}]`)) delete kept[f];
    kept = { ...kept, ...edits, focus: undefined };
    const page = renderNetwork(model, handlers);
    const shown = pane.firstElementChild;
    // The Devices page is patched in place: its list is never detached, so a
    // hovered row keeps its :hover (WebKit drops it on a replaced node). Its
    // one delegated listener reads the DOM, and `handlers` is one object.
    if (model.group === "devices" && pane.childElementCount === 1 && shown instanceof HTMLElement && shown.dataset.group === "devices") morph(shown, page);
    else pane.replaceChildren(page);
    restoreEdits(pane, { ...kept, focus: edits.focus });
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
    // The page the save started on: its error shows there, or in a toast
    // when the user has left it.
    const at = model.group;
    model.busy = true;
    model.error = null;
    paint();
    try {
      await f();
    } catch (e) {
      if (model.group === at) model.error = String(e);
      else showToast(String(e));
    }
    model.busy = false;
    paint();
  };
  const devices: DevicesHandlers = {
    onRemoveMember: (key) => void busy(async () => {
      setStatus(await invoke<NetworkStatus>("network_remove_member", { key }));
    }),
    onJoinAsk: (key) => { model.devicesJoining = key; paint(); },
    onJoinCancel: () => { model.devicesJoining = null; paint(); },
    onJoin: (key, host, port) => void busy(async () => {
      setStatus(await invoke<NetworkStatus>("network_join", { key, host, port }));
      model.devicesJoining = null;
      await loadConfig();
    }),
    // A phone is never a main: these rows are never drawn.
    onRemoveAssistant: () => undefined,
    onMakeMain: () => undefined,
    onMirror: () => undefined,
    onPromote: () => undefined,
    onPromoteCancel: () => undefined,
  };
  const handlers: NetworkHandlers = {
    devices,
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
      // Joined: the next Join starts empty.
      kept = { name: kept.name };
    }),
    onViewerTheme: (t) => void busy(() => saveConfig((c) => { c.viewerTheme = t; })),
    onBattery: () => void invoke("request_battery_exemption").catch((e) => showToast(String(e))),
    onOpenGroup: (g) => {
      model.group = g;
      model.error = null;
      groupHistory.opened(g);
      paint();
    },
    onBack: () => closeGroup(),
  };
  const groupHistory = makeGroupHistory();
  const closeGroup = () => {
    if (!model.group) return;
    model.group = null;
    model.error = null;
    groupHistory.closed();
    paint();
  };
  // Android's Back popped the group's entry: back to the list.
  window.addEventListener("popstate", () => {
    if (model.group && !groupHistory.holding()) {
      model.group = null;
      model.error = null;
      paint();
    }
  });
  // Another tab shown: Settings opens on the list next time.
  window.addEventListener("maya:tab", (e) => {
    if ((e as CustomEvent<string>).detail !== "settings") closeGroup();
  });
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
