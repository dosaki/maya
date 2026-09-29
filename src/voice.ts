import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface VoiceStatus {
  listening: boolean;
  state: "off" | "idle" | "awaiting-command" | "awaiting-confirm" | "thinking" | "error";
  detail: string;
  level: number;
  heard: string;
  said: string;
  pending: string | null;
  /** Counts turns added to the history, so the page knows when to refetch it. */
  turns: number;
}

export interface VoiceTurn {
  who: "user" | "maya";
  text: string;
  at: number;
}

export interface VoiceHandlers {
  onConfirm(yes: boolean): void;
  onListen(on: boolean): void;
}

const LABEL: Record<VoiceStatus["state"], string> = {
  off: "Not listening. Click to open.",
  idle: "Listening for \"Maya\".",
  "awaiting-command": "Yes? Say your command.",
  "awaiting-confirm": "Waiting for yes or no.",
  thinking: "Thinking…",
  error: "Listener stopped",
};

/** The top-bar microphone button; its class carries the state, --level the input level. */
export function renderIndicator(s: VoiceStatus): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.dataset.action = "voice";
  const cls = s.state === "awaiting-command" || s.state === "awaiting-confirm" ? "awaiting" : s.state;
  b.className = `voice voice--${cls}`;
  b.title = s.state === "error" && s.detail ? `${LABEL.error}: ${s.detail}` : LABEL[s.state];
  b.setAttribute("aria-label", b.title);
  b.style.setProperty("--level", String(Math.max(0, Math.min(1, s.level))));
  b.innerHTML = '<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><rect x="5.5" y="1.5" width="5" height="8" rx="2.5" fill="currentColor"/><path d="M3.5 7.5 a4.5 4.5 0 0 0 9 0 M8 12 V14.5 M5.5 14.5 H10.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>';
  return b;
}

export function renderVoicePanel(s: VoiceStatus, turns: VoiceTurn[], h: VoiceHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "voice-panel";
  const head = document.createElement("label");
  head.className = "settings__check";
  const toggle = document.createElement("input");
  toggle.type = "checkbox";
  toggle.name = "listen";
  toggle.checked = s.listening;
  toggle.addEventListener("change", () => h.onListen(toggle.checked));
  head.append(toggle, document.createTextNode(" Listen for \"Maya\""));
  root.append(head);
  if (s.state === "error" && s.detail) {
    const err = document.createElement("div");
    err.className = "voice__error";
    err.textContent = s.detail;
    root.append(err);
  }
  const list = document.createElement("div");
  list.className = "voice__turns";
  if (turns.length === 0) {
    const empty = document.createElement("div");
    empty.className = "voice__empty";
    empty.textContent = 'Say "Maya" and then what you need: what\'s waiting, reply to a session, focus, compact, resume or start one.';
    list.append(empty);
  }
  for (const t of turns) {
    const row = document.createElement("div");
    row.className = `voice__turn voice__turn--${t.who}`;
    row.textContent = t.text;
    list.append(row);
  }
  root.append(list);
  if (s.pending) {
    const pending = document.createElement("div");
    pending.className = "voice__pending";
    pending.textContent = s.pending;
    const yes = document.createElement("button");
    yes.type = "button";
    yes.className = "card__btn card__btn--primary";
    yes.dataset.action = "voice-yes";
    yes.textContent = "Yes";
    yes.addEventListener("click", () => h.onConfirm(true));
    const no = document.createElement("button");
    no.type = "button";
    no.className = "card__btn";
    no.dataset.action = "voice-no";
    no.textContent = "No";
    no.addEventListener("click", () => h.onConfirm(false));
    pending.append(document.createElement("br"), yes, no);
    root.append(pending);
  }
  return root;
}

/** Mounts the indicator and panel and keeps them current. */
export async function initVoice(): Promise<void> {
  const host = document.getElementById("voice-host");
  const panel = document.getElementById("voice-panel");
  if (!host || !panel) return;
  let status: VoiceStatus = { listening: false, state: "off", detail: "", level: 0, heard: "", said: "", pending: null, turns: 0 };
  let turns: VoiceTurn[] = [];
  const handlers: VoiceHandlers = {
    onConfirm: (yes) => void invoke("voice_confirm", { yes }),
    onListen: (on) => void invoke("voice_listen", { on }).catch((e) => { status = { ...status, state: "error", detail: String(e) }; paint(); }),
  };
  const paint = () => {
    host.replaceChildren(renderIndicator(status));
    if (!panel.hidden) panel.replaceChildren(renderVoicePanel(status, turns, handlers));
  };
  host.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
    if (!panel.hidden) void invoke<VoiceTurn[]>("voice_history").then((t) => { turns = t; paint(); });
    paint();
  });
  await listen<VoiceStatus>("voice", (e) => {
    const was = status;
    status = e.payload;
    // Partials arrive several times a second: refetch only when a turn was added.
    if (status.said !== was.said || status.turns !== was.turns) void invoke<VoiceTurn[]>("voice_history").then((t) => { turns = t; paint(); });
    paint();
  });
  await listen<number>("voice-level", (e) => {
    status = { ...status, level: e.payload };
    host.replaceChildren(renderIndicator(status));
  });
  status = await invoke<VoiceStatus>("voice_status");
  paint();
}
