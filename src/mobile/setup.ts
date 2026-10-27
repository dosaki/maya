// The phone's views when it shows no board of its own: until it has joined,
// an overlay with the Join card; after it has left, the board shows the same
// card; and a joined phone whose main shares nothing says why.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import type { NetworkStatus } from "./network";

export interface SetupModel {
  firstRun: boolean;
  /** Joined, but the main shares no sessions: whose, and whether it is too old to. */
  refused: { main: string; older: boolean } | null;
  name: string;
  /** What the join form holds, kept across repaints so an error does not lose it. */
  host: string;
  joinPort: number;
  code: string;
  error: string | null;
  busy: boolean;
}

/** The phone has never joined a main: say what Maya is for. */
export function isFirstRun(status: NetworkStatus): boolean {
  return status.role === "off";
}

/** Which view a status calls for; `ran` is whether the phone was joined since the page loaded. */
export function setupView(status: NetworkStatus, ran: boolean): "none" | "overlay" | "board" {
  if (status.role === "assistant") {
    const m = status.assistant.mirror;
    return m === "refused" || m === "unsupported" ? "board" : "none";
  }
  return isFirstRun(status) && !ran ? "overlay" : "board";
}

/** Why a joined phone's board is empty. */
export function refusedLine(main: string, phone: string, older: boolean): string {
  return older
    ? `${main} runs an older Maya that cannot share its sessions. Update it.`
    : `${main} has not allowed this phone to see its sessions. Turn on "Sees and controls all sessions" for ${phone} in ${main}'s Settings → Network.`;
}

const JOIN_HINT = "Open Settings → Network on the main, choose Show pairing code, and type the code it shows.";

function field(label: string, name: string, value: string, type: string, onInput: (value: string) => void): { label: HTMLLabelElement; input: HTMLInputElement } {
  const l = document.createElement("label");
  l.textContent = label;
  const input = document.createElement("input");
  input.name = name;
  input.type = type;
  input.value = value;
  input.addEventListener("input", () => onInput(input.value));
  l.append(input);
  return { label: l, input };
}

function primary(label: string, action: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "card__btn card__btn--primary";
  b.dataset.action = action;
  b.textContent = label;
  return b;
}

/** The Join card, or why a joined phone's board is empty. */
export function renderSetup(
  m: SetupModel,
  h: { onJoin(name: string, host: string, port: number, code: string): void },
): HTMLElement {
  const card = document.createElement("div");
  const title = document.createElement("h2");
  const blurb = document.createElement("p");
  const error = () => {
    if (!m.error) return;
    const err = document.createElement("p");
    err.className = "setup__error";
    err.textContent = m.error;
    card.append(err);
  };
  if (m.refused) {
    card.className = "setup__card setup__card--stopped";
    title.textContent = "No sessions to show";
    blurb.textContent = refusedLine(m.refused.main, m.name, m.refused.older);
    card.append(title, blurb);
    error();
    return card;
  }
  card.className = m.firstRun ? "setup__card" : "setup__card setup__card--stopped";
  title.textContent = m.firstRun ? "Maya for Android" : "Not joined to a main.";
  blurb.textContent = "Join a Maya that is already the main, to see and answer every session it shows, and to chat with it.";
  const name = field("Name", "name", m.name, "text", (v) => { m.name = v; });
  const host = field("Host", "host", m.host, "text", (v) => { m.host = v; });
  const port = field("Port", "joinPort", String(m.joinPort), "number", (v) => { m.joinPort = Number(v) || 0; });
  const code = field("Pairing code", "code", m.code, "text", (v) => { m.code = v; });
  code.input.inputMode = "numeric";
  const hint = document.createElement("p");
  hint.textContent = JOIN_HINT;
  const join = primary(m.busy ? "Joining…" : "Join", "join");
  join.disabled = m.busy;
  join.addEventListener("click", () => h.onJoin(name.input.value, host.input.value, Number(port.input.value) || 0, code.input.value));
  card.append(title, blurb, name.label, host.label, port.label, code.label, hint, join);
  error();
  return card;
}

export interface SetupHooks {
  /** What the board shows instead of its columns while the phone is not joined; `null` once it is. */
  onStopped(view: HTMLElement | null): void;
  /** Join asked Android for the notification permission (or found it granted). */
  onPermissionAsked(): void;
}

/** Shows the first run's overlay or the board's Join card whenever the phone is not joined, and joins on request. */
export function initSetup(hooks: SetupHooks): void {
  const host = document.getElementById("setup");
  if (!host) return;
  const model: SetupModel = { firstRun: true, refused: null, name: "", host: "", joinPort: 4127, code: "", error: null, busy: false };
  let ran = false;
  let last: NetworkStatus | null = null;
  const show = () => {
    const view = last ? setupView(last, ran) : "none";
    host.hidden = view !== "overlay";
    model.firstRun = view === "overlay";
    const card = view === "none" ? null : renderSetup(model, {
      onJoin: (name, host, port, code) => void join(name, host, port, code),
    });
    if (view === "overlay") host.replaceChildren(card!);
    else host.replaceChildren();
    hooks.onStopped(view === "board" ? card : null);
  };
  const paint = (status: NetworkStatus) => {
    last = status;
    if (status.role === "assistant") ran = true;
    const m = status.assistant.mirror;
    model.refused = status.role === "assistant" && (m === "refused" || m === "unsupported") ? { main: status.assistant.mainName ?? "The main Maya", older: m === "unsupported" } : null;
    model.error = status.mainError ?? null;
    show();
  };
  const join = async (name: string, host: string, port: number, code: string) => {
    model.name = name;
    model.busy = true;
    model.error = null;
    show();
    try {
      if (!(await isPermissionGranted().catch(() => true))) await requestPermission().catch(() => undefined);
      hooks.onPermissionAsked();
      model.busy = false;
      paint(await invoke<NetworkStatus>("network_pair", { host, port, name, code }));
      return;
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    show();
  };
  void listen<NetworkStatus>("network", (e) => paint(e.payload));
  void (async () => {
    const config = await invoke<{ network: { name: string } }>("get_config");
    model.name = config.network.name;
    paint(await invoke<NetworkStatus>("network_status"));
  })();
}
