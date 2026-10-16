// The phone's mic bar. The conversation runs in Rust, which hears the
// voice plugin's events and asks for the microphone itself; the page only
// shows the `voice` view and sends taps and Yes/No.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface VoiceView {
  state: "off" | "idle" | "listening" | "thinking" | "awaiting-command" | "awaiting-confirm" | "speaking";
  heard: string;
  said: string;
  pending: string | null;
  error: string | null;
  /** Why voice is not offered; null when it is. */
  offer: string | null;
  /** Shown while idle: tap-to-talk goes through an online recogniser. */
  note: string | null;
}

export interface VoiceHandlers {
  onTap(): void;
  onConfirm(yes: boolean): void;
}

const NO_MAIN = "Voice needs a desktop main.";

function line(className: string, text: string): HTMLElement {
  const d = document.createElement("div");
  d.className = className;
  d.textContent = text;
  return d;
}

export function renderVoiceBar(v: VoiceView, h: VoiceHandlers): HTMLElement {
  const bar = document.createElement("div");
  bar.className = `voice voice--${v.state}`;
  bar.hidden = v.offer === NO_MAIN;
  const mic = document.createElement("button");
  mic.type = "button";
  mic.className = "voice__mic";
  mic.dataset.action = "mic";
  mic.setAttribute("aria-label", "Talk to Maya");
  mic.textContent = "🎙";
  mic.disabled = v.offer !== null;
  mic.addEventListener("click", () => h.onTap());
  const said = document.createElement("div");
  said.className = "voice__line";
  if (v.offer) said.textContent = v.offer;
  else if (v.state === "thinking") said.textContent = "Thinking…";
  else {
    if (v.heard) said.append(line("voice__heard", v.heard));
    if (v.said) said.append(line("voice__said", v.said));
  }
  bar.append(mic, said);
  if (v.pending && v.state === "awaiting-confirm") {
    const answer = document.createElement("div");
    answer.className = "voice__answer";
    for (const [label, yes] of [["Yes", true], ["No", false]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.dataset.action = yes ? "yes" : "no";
      b.textContent = label;
      b.addEventListener("click", () => h.onConfirm(yes));
      answer.append(b);
    }
    bar.append(answer);
  }
  if (v.note && v.state === "idle" && !v.offer) bar.append(line("voice__note", v.note));
  if (v.error) bar.append(line("voice__error", v.error));
  return bar;
}

export async function initVoice(): Promise<void> {
  const host = document.getElementById("voice-host");
  if (!host) return;
  const handlers: VoiceHandlers = {
    onTap: () => void invoke("voice_tap").catch(() => undefined),
    onConfirm: (yes) => void invoke("voice_confirm", { yes }).catch(() => undefined),
  };
  const paint = (v: VoiceView) => host.replaceChildren(renderVoiceBar(v, handlers));
  await listen<VoiceView>("voice", (e) => paint(e.payload));
  const off: VoiceView = { state: "off", heard: "", said: "", pending: null, error: null, offer: NO_MAIN, note: null };
  paint(await invoke<VoiceView>("voice_view").catch(() => off));
}
