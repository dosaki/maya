// Settings › Devices: this device, and the others in the group. Shared with
// the phone, so nothing here may import desktop-only code.

import { formatAge } from "./format";
import { promoteLines } from "./promote";
import type { NetworkStatus } from "./settings";

export interface DeviceRow {
  id: string;
  key: string;
  name: string;
  hostname: string;
  platform: string;
  address: string;
  lastSeen: number | null;
  isMain: boolean;
  isSelf: boolean;
  /** The port it was last reached on; known only for the main this Maya follows. */
  port?: number | null;
}

/** What the list needs to remember between repaints. */
export interface DevicesModel {
  /** The key of the row whose Join form is open. */
  joining: string | null;
  joinHost: string;
  joinPort: number;
  /** The assistant id awaiting a Make main confirmation. */
  promote: string | null;
  promoting: boolean;
}

export interface DevicesHandlers {
  onRemoveMember(key: string): void;
  onJoinAsk(key: string): void;
  onJoin(key: string, host: string, port: number): void;
  onJoinCancel(): void;
  onRemoveAssistant(id: string): void;
  onMakeMain(id: string): void;
  onMirror(id: string, on: boolean): void;
  onPromote(id: string): void;
  onPromoteCancel(): void;
}

export const DEFAULT_PORT = 4127;
export const REMOVE_NEEDS_MAIN = "Connect to the main Maya to remove a device.";

const platformLabel = (p: string) => (p === "android" ? "Android" : p);

const MARKS: Record<string, string> = { macos: "Mac", windows: "Windows", linux: "Linux", android: "Phone" };
const markText = (p: string) => MARKS[p] ?? p;

/** A button with no listener of its own: the list's one delegated listener reads `data-*` at click time. */
function button(action: string, text: string, data: { key?: string; id?: string }, disabled = false, kind?: "quiet" | "danger" | "gold"): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = kind === "danger" ? "settings__btn settings__btn--quiet settings__btn--danger" : kind ? `settings__btn settings__btn--${kind}` : "settings__btn";
  b.dataset.action = action;
  if (data.key !== undefined) b.dataset.key = data.key;
  if (data.id !== undefined) b.dataset.id = data.id;
  b.textContent = text;
  b.disabled = disabled;
  return b;
}

/** A row; `data-row` is what a repaint matches it by, so the node survives. */
function rowEl(id: string): HTMLElement {
  const row = document.createElement("div");
  row.className = "settings__assistant settings__device";
  row.dataset.row = id;
  return row;
}

function div(cls: string, text?: string): HTMLElement {
  const d = document.createElement("div");
  d.className = cls;
  if (text !== undefined) d.textContent = text;
  return d;
}

function stateEl(text: string, tone: "on" | "main" | "off"): HTMLElement {
  const s = document.createElement("span");
  s.className = `settings__state settings__state--${tone}`;
  s.textContent = text;
  return s;
}

interface Parts {
  platform: string;
  name: string;
  badge?: string;
  /** The meta line: platform, hostname and address. */
  meta: string;
  state?: HTMLElement;
  /** Further lines in the text column (a note, the mirror switch). */
  extra?: HTMLElement[];
  actions?: HTMLElement;
}

/** The row's children, always in this order: mark, text (name, meta, extras), state, actions. */
function fill(row: HTMLElement, p: Parts, stateUnderName: boolean): void {
  const mark = document.createElement("span");
  mark.className = "settings__device-mark";
  mark.textContent = markText(p.platform);
  const text = div("settings__device-text");
  const name = div("settings__device-name");
  name.append(p.name);
  if (p.badge) {
    const badge = document.createElement("span");
    badge.className = "settings__badge";
    badge.textContent = p.badge;
    name.append(badge);
  }
  const meta = div("settings__device-meta");
  if (stateUnderName && p.state) meta.append(p.state);
  else meta.append(p.meta);
  text.append(name, meta, ...(p.extra ?? []));
  row.append(mark, text);
  if (!stateUnderName && p.state) row.append(p.state);
  if (p.actions) row.append(p.actions);
}

/** The parts after a member's name: its hostname too when another device has the same name. */
function details(d: DeviceRow, clash: Set<string>): string {
  const host = clash.has(d.name.toLowerCase()) ? d.hostname : "";
  return [platformLabel(d.platform), host, d.address].filter(Boolean).join(", ");
}

function selfRow(d: DeviceRow, clash: Set<string>, thisDevice: string, under: boolean): HTMLElement {
  const row = rowEl(`self:${d.key}`);
  row.dataset.key = d.key;
  row.classList.add("settings__device--self");
  fill(row, { platform: d.platform, name: d.name, badge: thisDevice === "this phone" ? "This phone" : "This device", meta: details(d, clash), state: d.isMain ? stateEl("Main", "main") : undefined }, under);
  return row;
}

function joinForm(d: DeviceRow, m: DevicesModel): HTMLElement {
  const form = document.createElement("div");
  form.className = "settings__confirm";
  form.dataset.for = "join-form";
  form.dataset.row = `join:${d.key}`;
  const host = document.createElement("input");
  host.type = "text";
  host.name = "join-host";
  host.placeholder = "Host or address";
  host.value = m.joinHost || d.address;
  const port = document.createElement("input");
  port.type = "number";
  port.name = "join-port";
  port.placeholder = String(DEFAULT_PORT);
  port.value = String(m.joinPort || d.port || DEFAULT_PORT);
  form.append(host, port, button("join-confirm", "Join", { key: d.key }, false, "gold"), button("join-cancel", "Cancel", { key: d.key }));
  return form;
}

function lastSeen(connected: boolean, lastSeenAt: number | null, nowMs: number): HTMLElement {
  if (connected) return stateEl("Connected", "on");
  return stateEl(lastSeenAt !== null ? `Last seen ${formatAge(lastSeenAt, nowMs)} ago` : "Never connected", "off");
}

function memberRow(d: DeviceRow, net: NetworkStatus, m: DevicesModel, nowMs: number, clash: Set<string>, under: boolean): HTMLElement[] {
  const row = rowEl(d.key);
  row.dataset.key = d.key;
  const connectedMain = d.isMain && net.assistant.connected;
  const actions = div("settings__assistant-actions");
  if (!d.isSelf && !d.isMain) {
    const rm = button("remove-member", "Remove", { key: d.key }, !net.assistant.connected, "danger");
    if (!net.assistant.connected) rm.title = REMOVE_NEEDS_MAIN;
    actions.append(rm);
  }
  if (!d.isSelf && d.platform !== "android" && !connectedMain) {
    actions.append(button("join", "Join", { key: d.key }));
  }
  fill(row, { platform: d.platform, name: d.name, meta: details(d, clash), state: d.isMain ? stateEl("Main", "main") : lastSeen(false, d.lastSeen, nowMs), actions }, under);
  const out = [row];
  if (m.joining === d.key) out.push(joinForm(d, m));
  return out;
}

function assistantRows(net: NetworkStatus, m: DevicesModel, nowMs: number, thisDevice: string, under: boolean): HTMLElement[] {
  const out: HTMLElement[] = [];
  for (const a of net.assistants) {
    const row = rowEl(a.id);
    row.dataset.id = a.id;
    const extra: HTMLElement[] = [];
    if (a.note) {
      const note = div("settings__hint", a.note);
      note.dataset.for = "assistant-note";
      extra.push(note);
    }
    if (a.wantsMirror || a.mirror) {
      const sw = document.createElement("label");
      sw.className = "settings__device-extra";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.className = "settings__toggle";
      box.dataset.action = "mirror";
      box.dataset.id = a.id;
      box.setAttribute("aria-label", "Sees and controls all sessions");
      box.checked = !!a.mirror;
      box.disabled = m.promoting;
      sw.append(box, document.createTextNode("Sees and controls all sessions"));
      extra.push(sw);
    }
    const actions = div("settings__assistant-actions");
    actions.append(button("remove-assistant", "Remove", { id: a.id }, m.promoting, "danger"));
    if (a.connected && a.canLead && !(a.wantsMirror || a.mirror)) {
      actions.append(button("make-main", "Make main", { id: a.id }, m.promoting));
    }
    fill(row, { platform: a.platform, name: a.name, meta: [platformLabel(a.platform), a.address].filter(Boolean).join(", "), state: lastSeen(a.connected, a.lastSeen, nowMs), extra, actions }, under);
    out.push(row);
    if (m.promote === a.id) {
      const confirm = document.createElement("div");
      confirm.className = "settings__confirm";
      confirm.dataset.for = "promote-confirm";
      confirm.dataset.row = `promote:${a.id}`;
      for (const line of promoteLines(net.assistants, a.id, thisDevice)) {
        const p = document.createElement("p");
        p.textContent = line;
        confirm.append(p);
      }
      confirm.append(
        button("promote-confirm", m.promoting ? "Handing over…" : "Make main", { id: a.id }, m.promoting, "gold"),
        button("promote-cancel", "Cancel", { id: a.id }, m.promoting),
      );
      out.push(confirm);
    }
  }
  return out;
}

/**
 * The list's only listeners. A repaint may keep the first render's root (so
 * its rows keep :hover), so nothing here depends on a render: `h` must stay
 * the same object, and everything else is read from the DOM at event time.
 */
function delegate(list: HTMLElement, h: DevicesHandlers): void {
  list.addEventListener("click", (e) => {
    const b = (e.target as Element).closest<HTMLButtonElement>("button[data-action]");
    if (!b || b.disabled || !list.contains(b)) return;
    const { action, key = "", id = "" } = b.dataset;
    switch (action) {
      case "remove-member": return h.onRemoveMember(key);
      case "join": return h.onJoinAsk(key);
      case "join-cancel": return h.onJoinCancel();
      case "join-confirm": {
        const form = b.closest("[data-for=join-form]");
        const host = form?.querySelector<HTMLInputElement>("input[name=join-host]")?.value.trim() ?? "";
        const port = Math.floor(Number(form?.querySelector<HTMLInputElement>("input[name=join-port]")?.value)) || DEFAULT_PORT;
        return h.onJoin(key, host, port);
      }
      case "remove-assistant": return h.onRemoveAssistant(id);
      case "make-main": return h.onMakeMain(id);
      case "promote-confirm": return h.onPromote(id);
      case "promote-cancel": return h.onPromoteCancel();
    }
  });
  list.addEventListener("change", (e) => {
    const box = e.target;
    if (box instanceof HTMLInputElement && box.dataset.action === "mirror") h.onMirror(box.dataset.id ?? "", box.checked);
  });
}

/**
 * The device list: this device first, then, on a main, its paired devices
 * (with Remove, Make main and the phone's switch) or, on any other device,
 * the other members of the group. `thisDevice` is how a hand-over names
 * this machine ("this Mac"); `opts.stateUnderName` puts each row's state
 * in its meta line (the phone).
 */
export function renderDevices(net: NetworkStatus, m: DevicesModel, h: DevicesHandlers, nowMs: number, thisDevice = "this device", opts: { stateUnderName?: boolean } = {}): HTMLElement {
  const under = !!opts.stateUnderName;
  const list = document.createElement("div");
  list.className = "settings__assistants settings__devices";
  list.dataset.row = "devices-list";
  const devices = net.devices ?? [];
  const self = devices.find((d) => d.isSelf);
  const seen = new Set<string>();
  const clash = new Set<string>();
  for (const d of devices) {
    const name = d.name.toLowerCase();
    if (seen.has(name)) clash.add(name);
    seen.add(name);
  }
  if (self) list.append(selfRow(self, clash, thisDevice, under));
  if (net.role === "main") list.append(...assistantRows(net, m, nowMs, thisDevice, under));
  else for (const d of devices) if (!d.isSelf) list.append(...memberRow(d, net, m, nowMs, clash, under));
  delegate(list, h);
  return list;
}
