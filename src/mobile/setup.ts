// The overlay over the board while the server is not running: the first
// run's name and port, or "The server is stopped." with Start.

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

export function renderSetup(m: SetupModel, h: { onStart(name: string, port: number): void }): HTMLElement {
  const card = document.createElement("div");
  card.className = "setup__card";
  const title = document.createElement("h2");
  title.textContent = m.firstRun ? "Maya for Android" : "The server is stopped.";
  const blurb = document.createElement("p");
  blurb.textContent = m.firstRun
    ? "Maya is the main for your assistants: start the server, then pair them with the code it shows."
    : "Start it again to see your assistants' sessions.";
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
  const start = document.createElement("button");
  start.type = "button";
  start.className = "card__btn card__btn--primary";
  start.dataset.action = "start";
  start.textContent = m.busy ? "Starting…" : "Start";
  start.disabled = m.busy;
  start.addEventListener("click", () => h.onStart(nameInput.value, Number(portInput.value) || 0));
  card.append(title, blurb, name, port, start);
  if (m.error) {
    const err = document.createElement("p");
    err.className = "setup__error";
    err.textContent = m.error;
    card.append(err);
  }
  return card;
}

/** Shows the overlay whenever the server is down and starts it on request. */
export function initSetup(): void {
  const host = document.getElementById("setup");
  if (!host) return;
  const model: SetupModel = { firstRun: true, name: "", port: 4127, error: null, busy: false };
  const paint = (status: NetworkStatus) => {
    host.hidden = status.role === "main";
    model.firstRun = isFirstRun(status);
    model.error = status.mainError ?? null;
    host.replaceChildren(renderSetup(model, { onStart: (name, port) => void start(name, port) }));
  };
  const start = async (name: string, port: number) => {
    model.name = name;
    model.port = port;
    model.busy = true;
    model.error = null;
    host.replaceChildren(renderSetup(model, { onStart: () => undefined }));
    try {
      const config = await invoke<{ network: { name: string; port: number } }>("get_config");
      config.network.name = name;
      config.network.port = port;
      await invoke("set_config", { config });
      if (!(await isPermissionGranted().catch(() => true))) await requestPermission().catch(() => undefined);
      paint(await invoke<NetworkStatus>("server_start"));
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    if (!host.hidden) host.replaceChildren(renderSetup(model, { onStart: (n, p) => void start(n, p) }));
  };
  void listen<NetworkStatus>("network", (e) => paint(e.payload));
  void (async () => {
    const config = await invoke<{ network: { name: string; port: number } }>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    paint(await invoke<NetworkStatus>("network_status"));
  })();
}
