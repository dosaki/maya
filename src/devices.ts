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

/** A button with no listener of its own: the list's one delegated listener reads `data-*` at click time. */
function button(action: string, text: string, data: { key?: string; id?: string }, disabled = false): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
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
  row.className = "settings__assistant";
  row.dataset.row = id;
  return row;
}

function selfRow(d: DeviceRow): HTMLElement {
  const row = rowEl(`self:${d.key}`);
  row.dataset.key = d.key;
  row.classList.add("settings__device--self");
  const info = document.createElement("span");
  info.textContent = `This device (${d.name}) — ${[platformLabel(d.platform), d.address].filter(Boolean).join(", ")}`;
  row.append(info);
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
  port.value = String(m.joinPort || DEFAULT_PORT);
  form.append(host, port, button("join-confirm", "Join", { key: d.key }), button("join-cancel", "Cancel", { key: d.key }));
  return form;
}

function memberRow(d: DeviceRow, net: NetworkStatus, m: DevicesModel, nowMs: number): HTMLElement[] {
  const row = rowEl(d.key);
  row.dataset.key = d.key;
  const connectedMain = d.isMain && net.assistant.connected;
  const status = connectedMain ? "Connected" : d.lastSeen !== null ? `Last seen ${formatAge(d.lastSeen, nowMs)} ago` : "Never connected";
  const info = document.createElement("span");
  info.textContent = `${d.name} (${[platformLabel(d.platform), d.address].filter(Boolean).join(", ")}) — ${status}`;
  if (d.isMain) {
    const badge = document.createElement("span");
    badge.className = "settings__badge";
    badge.textContent = "Main";
    info.append(" ", badge);
  }
  const actions = document.createElement("div");
  actions.className = "settings__assistant-actions";
  if (!d.isSelf && !d.isMain) {
    const rm = button("remove-member", "Remove", { key: d.key }, !net.assistant.connected);
    if (!net.assistant.connected) rm.title = REMOVE_NEEDS_MAIN;
    actions.append(rm);
  }
  if (!d.isSelf && d.platform !== "android" && !connectedMain) {
    const join = button("join", "Join", { key: d.key });
    actions.append(join);
  }
  row.append(info, actions);
  const out = [row];
  if (m.joining === d.key) out.push(joinForm(d, m));
  return out;
}

function assistantRows(net: NetworkStatus, m: DevicesModel, nowMs: number, thisDevice: string): HTMLElement[] {
  const out: HTMLElement[] = [];
  for (const a of net.assistants) {
    const row = rowEl(a.id);
    row.dataset.id = a.id;
    const info = document.createElement("span");
    info.textContent = `${a.name} (${[platformLabel(a.platform), a.address].filter(Boolean).join(", ")}) — ${a.connected ? "Connected" : a.lastSeen !== null ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected"}`;
    if (a.note) {
      const note = document.createElement("div");
      note.className = "settings__hint";
      note.dataset.for = "assistant-note";
      note.textContent = a.note;
      info.append(note);
    }
    const rm = button("remove-assistant", "Remove", { id: a.id }, m.promoting);
    // Name and status on one line, the buttons on their own row beneath,
    // so a long status never squeezes them into a column.
    const actions = document.createElement("div");
    actions.className = "settings__assistant-actions";
    actions.append(rm);
    row.append(info, actions);
    if (a.connected && a.canLead && !(a.wantsMirror || a.mirror)) {
      const mk = button("make-main", "Make main", { id: a.id }, m.promoting);
      actions.append(mk);
    }
    if (a.wantsMirror || a.mirror) {
      const sw = document.createElement("label");
      sw.className = "settings__switch";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.dataset.action = "mirror";
      box.dataset.id = a.id;
      box.checked = !!a.mirror;
      box.disabled = m.promoting;
      sw.append(box, document.createTextNode(" Sees and controls all sessions"));
      actions.append(sw);
    }
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
        button("promote-confirm", m.promoting ? "Handing over…" : "Make main", { id: a.id }, m.promoting),
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
 * this machine ("this Mac").
 */
export function renderDevices(net: NetworkStatus, m: DevicesModel, h: DevicesHandlers, nowMs: number, thisDevice = "this device"): HTMLElement {
  const list = document.createElement("div");
  list.className = "settings__assistants settings__devices";
  const devices = net.devices ?? [];
  const self = devices.find((d) => d.isSelf);
  if (self) list.append(selfRow(self));
  if (net.role === "main") list.append(...assistantRows(net, m, nowMs, thisDevice));
  else for (const d of devices) if (!d.isSelf) list.append(...memberRow(d, net, m, nowMs));
  delegate(list, h);
  return list;
}
