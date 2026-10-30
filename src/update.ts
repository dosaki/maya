import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { askQuit } from "./quitguard";

/** How this install updates: by itself, by a link to the release, or never (a debug build). */
export type UpdateKind = "self" | "notice" | "off";

/** A newer release, and the link a page that cannot install it opens. */
export interface UpdateInfo {
  version: string;
  download: string;
}

export type UpdateState =
  | { phase: "idle" }
  | { phase: "available"; info: UpdateInfo }
  | { phase: "downloading"; info: UpdateInfo; percent: number | null }
  | { phase: "error"; info: UpdateInfo; message: string };

/** Six hours between background checks. */
export const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

function button(label: string, action: string, primary: boolean): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = primary ? "update-bar__action update-bar__action--primary" : "update-bar__action";
  b.dataset.update = action;
  b.textContent = label;
  return b;
}

/** The bar for `state`, or null when nothing should show. */
export function renderUpdateBar(state: UpdateState, kind: UpdateKind, dismissed: string | null): HTMLElement | null {
  if (state.phase === "idle" || kind === "off") return null;
  if (state.phase === "available" && dismissed === state.info.version) return null;
  const bar = document.createElement("div");
  bar.className = "update-bar";
  bar.setAttribute("role", "status");
  const text = document.createElement("span");
  text.className = "update-bar__text";
  const v = state.info.version;
  if (state.phase === "available") text.textContent = kind === "self" ? `Maya ${v} is ready.` : `Maya ${v} is out.`;
  else if (state.phase === "downloading") text.textContent = state.percent === null ? `Downloading Maya ${v}…` : `Downloading Maya ${v}… ${state.percent}%`;
  else text.textContent = `Maya ${v} could not be installed: ${state.message}`;
  bar.append(text);
  if (state.phase === "downloading") return bar;
  const main = kind === "notice" ? "Download" : state.phase === "error" ? "Try again" : "Update";
  bar.append(button(main, "go", true), button("Later", "later", false));
  return bar;
}

/** The page's updates: the bar's state, its checks and its buttons. */
export function makeUpdates() {
  let kind: UpdateKind = "off";
  let state: UpdateState = { phase: "idle" };
  let dismissed: string | null = null;

  function paint(): void {
    const host = document.getElementById("update-bar");
    if (!host) return;
    const bar = renderUpdateBar(state, kind, dismissed);
    host.replaceChildren(...(bar ? [bar] : []));
    host.hidden = !bar;
  }

  /** Asks for a newer release and repaints. `manual` (Settings) brings a dismissed bar back. Rejects when the check fails. */
  async function check(manual = false): Promise<UpdateInfo | null> {
    if (kind === "off") return null;
    if (manual) dismissed = null;
    const info = await invoke<UpdateInfo | null>("update_check");
    if (state.phase !== "downloading") state = info ? { phase: "available", info } : { phase: "idle" };
    paint();
    return info;
  }

  async function act(action: string): Promise<void> {
    if (state.phase === "idle" || state.phase === "downloading") return;
    const info = state.info;
    if (action === "later") {
      dismissed = info.version;
      state = { phase: "available", info };
      paint();
      return;
    }
    if (kind === "notice") {
      try {
        await invoke("open_url", { url: info.download });
      } catch (e) {
        state = { phase: "error", info, message: String(e) };
        paint();
      }
      return;
    }
    // A failed install took the update it found, so a retry finds it again.
    const retry = state.phase === "error";
    // The restart ends Native sessions: a working one is asked about first.
    // Not knowing is not "none": the update waits rather than stop one unasked.
    let working: string[];
    try {
      working = await invoke<string[]>("native_working");
    } catch (e) {
      state = { phase: "error", info, message: `could not tell whether a Native session is working: ${e}` };
      paint();
      return;
    }
    if (working.length > 0 && !(await askQuit(working, true))) {
      state = { phase: "available", info };
      paint();
      return;
    }
    state = { phase: "downloading", info, percent: null };
    paint();
    try {
      if (retry) await invoke("update_check");
      await invoke("update_install");
    } catch (e) {
      state = { phase: "error", info, message: String(e) };
      paint();
    }
  }

  async function start(): Promise<void> {
    try {
      kind = await invoke<UpdateKind>("update_kind");
    } catch {
      kind = "off";
    }
    if (kind === "off") return;
    document.getElementById("update-bar")?.addEventListener("click", (ev) => {
      const action = (ev.target as Element).closest<HTMLElement>("[data-update]")?.dataset.update;
      if (action) void act(action);
    });
    await listen<{ received: number; total: number }>("update-progress", (e) => {
      if (state.phase !== "downloading") return;
      const { received, total } = e.payload;
      state = { ...state, percent: total > 0 ? Math.min(100, Math.floor((received * 100) / total)) : null };
      paint();
    });
    // The backend logs a failed check; in the background it shows nothing.
    const quiet = () => void check().catch(() => undefined);
    quiet();
    setInterval(quiet, CHECK_EVERY_MS);
  }

  // Settings reads `kind()` once this settles, since `main.ts` starts
  // both without waiting for either.
  let ready: Promise<void> = Promise.resolve();
  const init = (): Promise<void> => (ready = start());

  return { init, ready: () => ready, check, kind: () => kind };
}

/** The page's one instance, shared with Settings. */
export const updates = makeUpdates();
