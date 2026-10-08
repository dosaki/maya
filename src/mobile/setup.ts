// The server's two stopped states. On a first run an overlay asks for the
// phone's name and port; once the server has run, the board itself says
// "The server is stopped." with Start, and the tabs stay reachable (the
// Network tab to remove an assistant, Debug to read why the port is taken).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import type { NetworkStatus } from "./network";

export interface SetupModel {
  firstRun: boolean;
  name: string;
  port: number;
  error: string | null;
  busy: boolean;
}

/** Nothing has ever paired and the server is not meant to run: say what Maya is for. */
export function isFirstRun(status: NetworkStatus): boolean {
  return status.role !== "main" && status.assistants.length === 0;
}

/** Which stopped view a status calls for; `ran` is whether the server ran since the page loaded. */
export function setupView(status: NetworkStatus, ran: boolean): "none" | "overlay" | "board" {
  if (status.role === "main") return "none";
  return isFirstRun(status) && !ran ? "overlay" : "board";
}

/** The first run's card (name, port, Start), or the stopped server's (Start alone). */
export function renderSetup(m: SetupModel, h: { onStart(name: string, port: number): void }): HTMLElement {
  const card = document.createElement("div");
  card.className = m.firstRun ? "setup__card" : "setup__card setup__card--stopped";
  const title = document.createElement("h2");
  title.textContent = m.firstRun ? "Maya for Android" : "The server is stopped.";
  const blurb = document.createElement("p");
  blurb.textContent = m.firstRun
    ? "Maya is the main for your assistants: start the server, then pair them with the code it shows."
    : "Start it again to see your assistants' sessions.";
  card.append(title, blurb);
  const start = document.createElement("button");
  start.type = "button";
  start.className = "card__btn card__btn--primary";
  start.dataset.action = "start";
  start.textContent = m.busy ? "Starting…" : "Start";
  start.disabled = m.busy;
  if (m.firstRun) {
    const name = document.createElement("label");
    name.textContent = "Name";
    const nameInput = document.createElement("input");
    nameInput.name = "name";
    nameInput.value = m.name;
    name.append(nameInput);
    const port = document.createElement("label");
    port.textContent = "Port";
    const portInput = document.createElement("input");
    portInput.name = "port";
    portInput.type = "number";
    portInput.value = String(m.port);
    port.append(portInput);
    start.addEventListener("click", () => h.onStart(nameInput.value, Number(portInput.value) || 0));
    card.append(name, port);
  } else {
    start.addEventListener("click", () => h.onStart(m.name, m.port));
  }
  card.append(start);
  if (m.error) {
    const err = document.createElement("p");
    err.className = "setup__error";
    err.textContent = m.error;
    card.append(err);
  }
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
  const model: SetupModel = { firstRun: true, name: "", port: 4127, error: null, busy: false };
  let ran = false;
  let last: NetworkStatus | null = null;
  const show = () => {
    const view = last ? setupView(last, ran) : "none";
    host.hidden = view !== "overlay";
    model.firstRun = view === "overlay";
    const card = view === "none" ? null : renderSetup(model, { onStart: (name, port) => void start(name, port) });
    if (view === "overlay") host.replaceChildren(card!);
    else host.replaceChildren();
    hooks.onStopped(view === "board" ? card : null);
  };
  const paint = (status: NetworkStatus) => {
    last = status;
    if (status.role === "main") ran = true;
    model.error = status.mainError ?? null;
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
