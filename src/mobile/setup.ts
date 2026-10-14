// The phone's views when it shows no board of its own: on a first run, an
// overlay that runs the server here or joins a main; once it has run, the
// board says the server is stopped; and a joined phone whose main shares
// nothing says why.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import type { NetworkStatus } from "./network";

export interface SetupModel {
  firstRun: boolean;
  /** The first run's choice: run the server here, or join a main. */
  mode: "main" | "join";
  /** Joined, but the main shares no sessions: whose, and whether it is too old to. */
  refused: { main: string; older: boolean } | null;
  name: string;
  /** The port of the server this phone runs. */
  port: number;
  /** What the join form holds, kept across repaints so an error does not lose it. */
  host: string;
  joinPort: number;
  code: string;
  error: string | null;
  busy: boolean;
}

/** Nothing has ever paired and the phone is neither a main nor joined: say what Maya is for. */
export function isFirstRun(status: NetworkStatus): boolean {
  return status.role === "off" && status.assistants.length === 0;
}

/** Which view a status calls for; `ran` is whether the phone was a main or joined since the page loaded. */
export function setupView(status: NetworkStatus, ran: boolean): "none" | "overlay" | "board" {
  if (status.role === "main") return "none";
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

/** The first run's card (Be the main / Join a main), the stopped server's (Start alone), or why a joined phone's board is empty. */
export function renderSetup(
  m: SetupModel,
  h: {
    onStart(name: string, port: number): void;
    onJoin(name: string, host: string, port: number, code: string): void;
    onMode(mode: "main" | "join"): void;
  },
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
  title.textContent = m.firstRun ? "Maya for Android" : "The server is stopped.";
  card.append(title);
  if (m.firstRun) {
    const modes = document.createElement("div");
    modes.className = "setup__modes";
    for (const [mode, label] of [["main", "Be the main"], ["join", "Join a main"]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "card__btn";
      b.dataset.action = `mode-${mode}`;
      b.textContent = label;
      b.setAttribute("aria-pressed", String(m.mode === mode));
      b.addEventListener("click", () => h.onMode(mode));
      modes.append(b);
    }
    card.append(modes);
  }
  if (m.firstRun && m.mode === "join") {
    blurb.textContent = "Join a Maya that is already the main, to see and answer every session it shows.";
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
    card.append(blurb, name.label, host.label, port.label, code.label, hint, join);
    error();
    return card;
  }
  blurb.textContent = m.firstRun
    ? "Maya is the main for your assistants: start the server, then pair them with the code it shows."
    : "Start it again to see your assistants' sessions.";
  card.append(blurb);
  const start = primary(m.busy ? "Starting…" : "Start", "start");
  start.disabled = m.busy;
  if (m.firstRun) {
    const name = field("Name", "name", m.name, "text", (v) => { m.name = v; });
    const port = field("Port", "port", String(m.port), "number", (v) => { m.port = Number(v) || 0; });
    start.addEventListener("click", () => h.onStart(name.input.value, Number(port.input.value) || 0));
    card.append(name.label, port.label);
  } else {
    start.addEventListener("click", () => h.onStart(m.name, m.port));
  }
  card.append(start);
  error();
  return card;
}

export interface SetupHooks {
  /** What the board shows instead of its columns while the server is stopped; `null` once it runs. */
  onStopped(view: HTMLElement | null): void;
  /** Start asked Android for the notification permission (or found it granted). */
  onPermissionAsked(): void;
}

/** Shows the first run's overlay or the board's stopped card whenever the server is down, and starts it on request. */
export function initSetup(hooks: SetupHooks): void {
  const host = document.getElementById("setup");
  if (!host) return;
  const model: SetupModel = { firstRun: true, mode: "main", refused: null, name: "", port: 4127, host: "", joinPort: 4127, code: "", error: null, busy: false };
  let ran = false;
  let last: NetworkStatus | null = null;
  const show = () => {
    const view = last ? setupView(last, ran) : "none";
    host.hidden = view !== "overlay";
    model.firstRun = view === "overlay";
    const card = view === "none" ? null : renderSetup(model, {
      onStart: (name, port) => void start(name, port),
      onJoin: (name, host, port, code) => void join(name, host, port, code),
      onMode: (mode) => { model.mode = mode; show(); },
    });
    if (view === "overlay") host.replaceChildren(card!);
    else host.replaceChildren();
    hooks.onStopped(view === "board" ? card : null);
  };
  const paint = (status: NetworkStatus) => {
    last = status;
    if (status.role === "main" || status.role === "assistant") ran = true;
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
  const start = async (name: string, port: number) => {
    model.name = name;
    model.port = port;
    model.busy = true;
    model.error = null;
    show();
    try {
      const config = await invoke<{ network: { name: string; port: number } }>("get_config");
      config.network.name = name;
      config.network.port = port;
      await invoke("set_config", { config });
      if (!(await isPermissionGranted().catch(() => true))) await requestPermission().catch(() => undefined);
      hooks.onPermissionAsked();
      model.busy = false;
      paint(await invoke<NetworkStatus>("server_start"));
      return;
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    show();
  };
  void listen<NetworkStatus>("network", (e) => paint(e.payload));
  void (async () => {
    const config = await invoke<{ network: { name: string; port: number } }>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    paint(await invoke<NetworkStatus>("network_status"));
  })();
}
