import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface VoiceStatus {
  listening: boolean;
  state: "off" | "idle" | "awaiting-command" | "window" | "thinking" | "error";
  detail: string;
  level: number;
  heard: string;
  said: string;
  /** The announced batch waiting in the action window, as she said it. */
  pending: string | null;
  /** When that batch runs (ms since the epoch); null until she has said it. */
  until?: number | null;
  /** Counts turns added to the history, so the page knows when to refetch it. */
  turns: number;
  /** The word she listens for; absent from an older app reads as "Maya". */
  word?: string;
}

export interface VoiceTurn {
  who: "user" | "maya";
  text: string;
  at: number;
}

export interface VoiceHandlers {
  onCancel(): void;
}

/** What the microphone button says; idle names the wake word (`stateLabel`). */
const LABEL: Record<Exclude<VoiceStatus["state"], "idle">, string> = {
  off: "Not listening.",
  "awaiting-command": "Listening for your reply.",
  window: "Say stop to cancel.",
  thinking: "Thinking…",
  error: "Listener stopped",
};

function wakeWord(s: VoiceStatus): string {
  return s.word?.trim() || "Maya";
}

function stateLabel(s: VoiceStatus): string {
  return s.state === "idle" ? `Listening for "${wakeWord(s)}".` : LABEL[s.state];
}

/**
 * The top-bar microphone button; its class carries the state, --level the
 * input level. Nothing is shown while listening is off: the switch lives in
 * Settings, and the microphone appears once it is on.
 */
export function renderIndicator(s: VoiceStatus): HTMLButtonElement | null {
  if (!s.listening) return null;
  const b = document.createElement("button");
  b.type = "button";
  b.dataset.action = "voice";
  const cls = s.state === "awaiting-command" || s.state === "window" ? "awaiting" : s.state;
  b.className = `voice voice--${cls}`;
  b.title = s.state === "error" && s.detail ? `${LABEL.error}: ${s.detail}` : stateLabel(s);
  b.setAttribute("aria-label", b.title);
  b.style.setProperty("--level", String(Math.max(0, Math.min(1, s.level))));
  b.innerHTML = '<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><rect x="5.5" y="1.5" width="5" height="8" rx="2.5" fill="currentColor"/><path d="M3.5 7.5 a4.5 4.5 0 0 0 9 0 M8 12 V14.5 M5.5 14.5 H10.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>';
  return b;
}

export function renderVoicePanel(s: VoiceStatus, turns: VoiceTurn[], h: VoiceHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "voice-panel";
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
    empty.textContent = `Say "${wakeWord(s)}" and then what you need: what's waiting, reply to a session, focus, compact, resume or start one.`;
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
    pending.append(document.createElement("br"));
    if (s.until != null) {
      const left = Math.ceil((s.until - Date.now()) / 1000);
      const countdown = document.createElement("span");
      countdown.className = "voice__countdown";
      countdown.textContent = left > 0 ? `Running in ${left} s.` : "Running now.";
      pending.append(countdown, " ");
    }
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.className = "card__btn";
    cancel.dataset.action = "window-cancel";
    cancel.textContent = "Cancel";
    cancel.addEventListener("click", () => h.onCancel());
    pending.append(cancel);
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
    onCancel: () => void invoke("window_cancel"),
  };
  // Repaints the countdown each second while a batch waits with its clock running.
  let tick: ReturnType<typeof setInterval> | undefined;
  const keepCounting = () => {
    const counting = !!status.pending && status.until != null;
    if (counting && tick === undefined) tick = setInterval(() => { if (!panel.hidden) panel.replaceChildren(renderVoicePanel(status, turns, handlers)); }, 1000);
    if (!counting && tick !== undefined) { clearInterval(tick); tick = undefined; }
  };
  const paintIndicator = () => {
    const b = renderIndicator(status);
    if (b) host.replaceChildren(b); else host.replaceChildren();
  };
  const paint = () => {
    keepCounting();
    if (!status.listening) panel.hidden = true;
    paintIndicator();
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
    paintIndicator();
  });
  status = await invoke<VoiceStatus>("voice_status");
  paint();
}
