import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "./format";
import type { VoiceStatus } from "./voice";
import { builtinVoiceName, recognizerOptions, secretStore, thisComputerLower } from "./platform";
import type { Harness } from "./types";
import { CLAUDE_AGENT, type AgentInfo } from "./newsession";
import { harnessLabel } from "./harness";
import { DEFAULT_REVIEW_PROMPT } from "./reviewprompt";

/** Which network role this Maya plays, and what Settings › Network shows. */
export type NetworkRole = "off" | "main" | "assistant";

export interface PairingCode {
  code: string;
  expiresAt: number;
}

export interface AssistantStatus {
  id: string;
  name: string;
  hostname: string;
  platform: string;
  /** Its IP address as the main last saw it; empty when never recorded. */
  address?: string;
  connected: boolean;
  lastSeen: number | null;
  /** A different Maya version, or a board this Maya could not read. */
  note?: string | null;
}

export interface AssistantLink {
  connected: boolean;
  mainName: string | null;
  error: string | null;
  /** The client waits to try again; `error` is then the last failure. */
  retrying?: boolean;
}

/** The live `network_status`/`network` event payload. */
export interface NetworkStatus {
  role: NetworkRole;
  code: PairingCode | null;
  assistants: AssistantStatus[];
  assistant: AssistantLink;
  /** Why the main's server is not running (the port is taken…). */
  mainError?: string | null;
}

const DEFAULT_NETWORK_STATUS: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } };

export interface SettingsModel {
  hookInstalled: boolean | null;
  codexHookInstalled?: boolean | null;
  completedTimeoutMinutes: number;
  projectsDir: string;
  clonesDir: string;
  notifyOnAwaiting: boolean;
  speakNotifications: boolean;
  /** False when Maya cannot tell whether a Focus mode is on (macOS without Full Disk Access). */
  focusVisible?: boolean;
  voiceProvider: VoiceProvider;
  elevenKeySet: boolean;
  elevenVoices: ElevenVoice[];
  elevenVoiceId: string;
  error: string | null;
  listen: boolean;
  microphone: string;
  microphones: string[];
  /** The agent that powers Maya; null until chosen (the first-start modal asks). */
  agent: Harness | null;
  /** That agent's model id; "" is the agent's default. */
  agentModel: string;
  /** The agents installed on this machine, with their models; empty until listed. */
  agents: AgentInfo[];
  /** The review prompt; "" means the built-in one. */
  reviewPrompt: string;
  /** Why the listener stopped (Dictation off, no microphone…), shown under the toggle. */
  listenError: string | null;
  recognizer: Recognizer;
  whisperModel: string;
  models: ModelInfo[];
  downloading: { id: string; received: number; total: number } | null;
  /** The live status from `network_status`/the `network` event. */
  network?: NetworkStatus;
  /** The Network select's current value; can diverge from `network.role` while previewing "Assistant" before Pair succeeds. */
  networkRole?: NetworkRole;
  /** This Maya's listening port as a main. */
  networkPort?: number;
  /** The assistant's target: the main's host/address. */
  networkMainHost?: string;
  /** The assistant's target: the main's port. */
  networkMainPort?: number;
  /** The assistant's display name; blank means the hostname. */
  networkName?: string;
  /** The pairing code as typed, held here (not persisted) so a repaint never wipes it mid-entry. */
  networkCode?: string;
  /** The message from a failed Pair attempt, shown in the status line; cleared by the next attempt or a success. */
  networkError?: string | null;
  /** The config holds an assistant id and token: choosing Assistant reconnects without pairing. */
  networkPaired?: boolean;
  /** "Pair again" was clicked: the pairing form shows even though credentials are stored. */
  networkRepair?: boolean;
  /** A Pair is in flight: the button is disabled and further clicks are ignored. */
  networkPairing?: boolean;
}

export type VoiceProvider = "builtin" | "elevenlabs";
export interface ElevenVoice {
  voiceId: string;
  name: string;
}

export interface ModelInfo {
  id: string;
  label: string;
  bytes: number;
  downloaded: boolean;
}
export type Recognizer = "system" | "builtin";

export interface SettingsHandlers {
  onInstall(): void;
  onCodexInstall?(): void;
  onCodexRemove?(): void;
  onRemove(): void;
  onTimeout(minutes: number): void;
  onProjectsDir(path: string): void;
  onNotify(enabled: boolean): void;
  onClonesDir(path: string): void;
  onSpeak(enabled: boolean): void;
  onFullDiskAccess?(): void;
  onVoiceProvider(provider: VoiceProvider): void;
  onElevenKey(key: string): void;
  onElevenVoice(voiceId: string): void;
  onTryVoice(): void;
  onListen(on: boolean): void;
  onMicrophone(name: string): void;
  onAgent(agent: Harness): void;
  /** Saves the model paired with the agent the Agent select shows. */
  onAgentModel(agent: Harness, id: string): void;
  onReviewPrompt(text: string): void;
  onRecognizer(r: Recognizer): void;
  onWhisperModel(id: string): void;
  onDownloadModel(id: string): void;
  onRemoveModel(id: string): void;
  /** Off or Main save immediately, and so does Assistant when credentials are stored; otherwise Assistant only previews the pairing form (see `onPair`). */
  onRole(role: NetworkRole): void;
  /** Reveals the pairing form on an already-paired assistant. */
  onPairAgain(): void;
  onPort(port: number): void;
  onMainHost(host: string): void;
  onMainPort(port: number): void;
  onName(name: string): void;
  /** Opens pairing (mints a code) or, if one is already open, regenerates it. */
  onRegenerate(): void;
  onRemoveAssistant(id: string): void;
  onPair(host: string, port: number, name: string, code: string): void;
}

interface NetworkConfigJson {
  role: NetworkRole;
  port: number;
  mainHost: string;
  mainPort: number;
  name: string;
  assistantId: string;
  token: string;
  assistants: { id: string; name: string; hostname: string; platform: string; token: string; address?: string; lastSeen?: number | null }[];
}

interface ConfigJson {
  completedTimeoutMinutes: number;
  projectsDir?: string | null;
  clonesDir?: string | null;
  notifyOnAwaiting: boolean;
  speakNotifications: boolean;
  voiceProvider: VoiceProvider;
  elevenlabsVoiceId?: string | null;
  listen: boolean;
  microphone?: string | null;
  agent?: Harness | null;
  agentModel?: string;
  reviewPrompt?: string;
  recognizer: Recognizer;
  whisperModel: string;
  network?: NetworkConfigJson;
}

/** A titled card in the settings grid. */
function section(title: string): HTMLElement {
  const s = document.createElement("section");
  s.className = "settings__section";
  const heading = document.createElement("h2");
  heading.className = "settings__heading";
  heading.textContent = title;
  s.append(heading);
  return s;
}

/** "483 921" from "483921": a space after the first three digits. */
function formatPairingCode(code: string): string {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function expiresInMinutes(expiresAt: number, nowMs: number): number {
  return Math.max(0, Math.ceil((expiresAt - nowMs) / 60_000));
}

type Field = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;

/** The named field in `root` that has focus, and its selection when it has one. */
interface FocusedField {
  name: string;
  start: number | null;
  end: number | null;
  direction: "forward" | "backward" | "none" | null;
}

function isField(el: Element | null): el is Field {
  return el instanceof HTMLInputElement || el instanceof HTMLSelectElement || el instanceof HTMLTextAreaElement;
}

function focusedField(root: HTMLElement): FocusedField | null {
  const el = document.activeElement;
  if (!isField(el) || !el.name || !root.contains(el)) return null;
  const focused: FocusedField = { name: el.name, start: null, end: null, direction: null };
  if (el instanceof HTMLSelectElement) return focused;
  try {
    // Null (or a throw, in some engines) for inputs with no caret, such as checkboxes.
    focused.start = el.selectionStart;
    focused.end = el.selectionEnd;
    focused.direction = el.selectionDirection;
  } catch {
    // no selection to keep
  }
  return focused;
}

function restoreFocus(root: HTMLElement, f: FocusedField): void {
  const el = [...root.querySelectorAll("input, select, textarea")].find((e): e is Field => isField(e) && e.name === f.name);
  if (!el) return;
  el.focus();
  if (el instanceof HTMLSelectElement || f.start === null || f.end === null) return;
  try {
    el.setSelectionRange(f.start, f.end, f.direction ?? undefined);
  } catch {
    // the field changed kind; focus alone will do
  }
}

/** How often the pairing code's "expires in N min" line is repainted. */
export const PAIRING_TICK_MS = 30_000;

/**
 * Refreshes the code's expiry line in place, or once it expired removes the
 * code and sets the button back to "Show pairing code".
 */
export function tickPairingCode(panel: ParentNode, model: SettingsModel, nowMs: number): void {
  const code = model.network?.code;
  const box = panel.querySelector<HTMLElement>(".settings__code");
  const expiry = panel.querySelector<HTMLElement>(".settings__code-expiry");
  if (!code || !box) return;
  if (nowMs > code.expiresAt) {
    box.remove();
    expiry?.remove();
    const regen = panel.querySelector<HTMLButtonElement>("button[data-action=regenerate-code]");
    if (regen) regen.textContent = "Show pairing code";
  } else if (expiry) {
    expiry.textContent = `expires in ${expiresInMinutes(code.expiresAt, nowMs)} min`;
  }
}

/**
 * True while a pairing code is on screen or has just expired: the timer
 * ticks then, so the countdown moves and the code goes when it expires.
 */
export function pairingRepaintDue(model: SettingsModel, nowMs: number): boolean {
  const code = model.network?.code;
  return (model.networkRole ?? model.network?.role) === "main" && !!code && nowMs <= code.expiresAt + PAIRING_TICK_MS;
}

export function renderSettings(model: SettingsModel, h: SettingsHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = document.createElement("div");
  root.className = "settings__body";
  const maya = section("Maya");
  const sessions = section("Sessions");
  const notifications = section("Notifications");
  const assistant = section("Voice assistant");
  const network = section("Network");
  root.append(maya, sessions, notifications, assistant, network);

  const agents = model.agents.length > 0 ? model.agents : [CLAUDE_AGENT];
  // An agent that is chosen but not listed (uninstalled, or the listing has
  // not landed yet) stays selected, with only its Default model, rather than
  // the select silently showing another agent.
  const chosenAgent: AgentInfo = agents.find((a) => a.harness === model.agent) ?? (model.agent ? { harness: model.agent, models: [], efforts: [], modes: [] } : agents[0]);
  const agentLabel = document.createElement("label");
  agentLabel.textContent = "Agent";
  const agentSel = document.createElement("select");
  agentSel.name = "agent";
  for (const a of agents) {
    const o = document.createElement("option");
    o.value = a.harness;
    o.textContent = harnessLabel(a.harness);
    agentSel.append(o);
  }
  if (!agents.includes(chosenAgent)) {
    const o = document.createElement("option");
    o.value = chosenAgent.harness;
    o.textContent = harnessLabel(chosenAgent.harness) + (model.agents.length > 0 ? " (not installed)" : "");
    agentSel.append(o);
  }
  agentSel.value = chosenAgent.harness;
  agentSel.addEventListener("change", () => h.onAgent(agentSel.value as Harness));
  agentLabel.append(agentSel);
  maya.append(agentLabel);
  const agentHint = document.createElement("div");
  agentHint.className = "settings__hint";
  agentHint.dataset.for = "agent";
  agentHint.textContent = "Interprets your voice commands, picks folders for 'Let Maya choose', and is the default for new and resumed sessions and reviews.";
  maya.append(agentHint);

  const agentModelLabel = document.createElement("label");
  agentModelLabel.textContent = "Model";
  const agentModelSel = document.createElement("select");
  agentModelSel.name = "agentModel";
  const def = document.createElement("option");
  def.value = "";
  def.textContent = "Default";
  agentModelSel.append(def);
  for (const m of chosenAgent.models) {
    const o = document.createElement("option");
    o.value = m.id;
    o.textContent = m.label;
    agentModelSel.append(o);
  }
  agentModelSel.value = chosenAgent.models.some((m) => m.id === model.agentModel) ? model.agentModel : "";
  agentModelSel.addEventListener("change", () => h.onAgentModel(agentSel.value as Harness, agentModelSel.value));
  agentModelLabel.append(agentModelSel);
  maya.append(agentModelLabel);

  const net = model.network ?? DEFAULT_NETWORK_STATUS;
  const netRole = model.networkRole ?? net.role;
  const netPort = model.networkPort ?? 0;
  const netMainHost = model.networkMainHost ?? "";
  const netMainPort = model.networkMainPort ?? 0;
  const netName = model.networkName ?? "";
  const netCode = model.networkCode ?? "";

  const status = document.createElement("div");
  status.className = "settings__status";
  status.textContent =
    model.hookInstalled === null
      ? "Checking Claude Code hook…"
      : model.hookInstalled
        ? "Claude Code hook is installed. Awaiting Decision and Completed are precise."
        : "Claude Code hook is not installed. Permission prompts will show as Working.";
  sessions.append(status);

  const btn = document.createElement("button");
  btn.type = "button";
  if (model.hookInstalled) {
    btn.dataset.action = "remove";
    btn.textContent = "Remove Claude hook";
    btn.addEventListener("click", () => h.onRemove());
  } else {
    btn.dataset.action = "install";
    btn.textContent = "Install Claude hook";
    btn.disabled = model.hookInstalled === null;
    btn.addEventListener("click", () => h.onInstall());
  }
  sessions.append(btn);

  const codexStatus = document.createElement("div");
  codexStatus.className = "settings__status";
  codexStatus.textContent = model.codexHookInstalled == null
    ? "Checking Codex hook…"
    : model.codexHookInstalled
      ? "Codex hook is installed. Restart Codex and use /hooks to review and trust it for session updates."
      : "Codex hook is not installed. Session state is inferred from transcripts.";
  const codexButton = document.createElement("button");
  codexButton.type = "button";
  codexButton.dataset.action = model.codexHookInstalled ? "remove-codex" : "install-codex";
  codexButton.textContent = model.codexHookInstalled ? "Remove Codex hook" : "Install Codex hook";
  codexButton.disabled = model.codexHookInstalled == null;
  codexButton.addEventListener("click", () => model.codexHookInstalled ? h.onCodexRemove?.() : h.onCodexInstall?.());
  sessions.append(codexStatus, codexButton);

  const label = document.createElement("label");
  label.textContent = "Completed decays to Idle after (minutes)";
  const input = document.createElement("input");
  input.type = "number";
  input.name = "timeout";
  input.min = "1";
  input.value = String(model.completedTimeoutMinutes);
  input.addEventListener("change", () => {
    const n = Number(input.value);
    if (Number.isFinite(n) && n >= 1) h.onTimeout(Math.floor(n));
  });
  label.append(input);
  sessions.append(label);

  const dirLabel = document.createElement("label");
  dirLabel.textContent = "Projects directory";
  const dirInput = document.createElement("input");
  dirInput.type = "text";
  dirInput.name = "projectsDir";
  dirInput.placeholder = "~/dev";
  dirInput.value = model.projectsDir;
  dirInput.addEventListener("change", () => h.onProjectsDir(dirInput.value.trim()));
  dirLabel.append(dirInput);
  sessions.append(dirLabel);

  const clonesLabel = document.createElement("label");
  clonesLabel.textContent = "Clones directory (for PR reviews)";
  const clonesInput = document.createElement("input");
  clonesInput.type = "text";
  clonesInput.name = "clonesDir";
  clonesInput.placeholder = "~/dev/reviews";
  clonesInput.value = model.clonesDir;
  clonesInput.addEventListener("change", () => h.onClonesDir(clonesInput.value.trim()));
  clonesLabel.append(clonesInput);
  sessions.append(clonesLabel);

  const reviewLabel = document.createElement("label");
  reviewLabel.textContent = "Review prompt (for the Review button)";
  const reviewInput = document.createElement("textarea");
  reviewInput.name = "reviewPrompt";
  reviewInput.rows = 4;
  reviewInput.placeholder = DEFAULT_REVIEW_PROMPT;
  reviewInput.value = model.reviewPrompt;
  reviewInput.addEventListener("change", () => h.onReviewPrompt(reviewInput.value.trim()));
  reviewLabel.append(reviewInput);
  sessions.append(reviewLabel);
  const reviewHint = document.createElement("div");
  reviewHint.className = "settings__hint";
  reviewHint.dataset.for = "reviewPrompt";
  reviewHint.textContent = "Blank uses the built-in prompt. {number}, {repo} and {url} are filled in; a prompt that uses none gets \" PR #<number> (<url>)\" appended.";
  sessions.append(reviewHint);

  const notifyLabel = document.createElement("label");
  notifyLabel.className = "settings__check";
  const notifyBox = document.createElement("input");
  notifyBox.type = "checkbox";
  notifyBox.name = "notify";
  notifyBox.checked = model.notifyOnAwaiting;
  notifyBox.addEventListener("change", () => h.onNotify(notifyBox.checked));
  notifyLabel.append(notifyBox, document.createTextNode(" Notify me when a session awaits a decision"));
  notifications.append(notifyLabel);

  const speakLabel = document.createElement("label");
  speakLabel.className = "settings__check";
  const speakBox = document.createElement("input");
  speakBox.type = "checkbox";
  speakBox.name = "speak";
  speakBox.checked = model.speakNotifications;
  speakBox.addEventListener("change", () => h.onSpeak(speakBox.checked));
  speakLabel.append(speakBox, document.createTextNode(" Speak instead of a sound (\"needs a decision\", \"is finished\")"));
  notifications.append(speakLabel);

  if (model.focusVisible === false) {
    // macOS keeps the Focus state from apps without Full Disk Access, so
    // Maya would speak through Do Not Disturb.
    const focusHint = document.createElement("div");
    focusHint.className = "settings__hint";
    focusHint.dataset.for = "focus";
    focusHint.textContent = "Maya cannot tell when a Focus mode such as Do Not Disturb is on, so she keeps speaking. Give her Full Disk Access to let a Focus mode keep her quiet; macOS then asks to quit and reopen her. ";
    const open = document.createElement("button");
    open.type = "button";
    open.dataset.action = "full-disk-access";
    open.textContent = "Open Full Disk Access";
    open.addEventListener("click", () => h.onFullDiskAccess?.());
    focusHint.append(open);
    notifications.append(focusHint);
  }

  // The live role, not the select: previewing Assistant before a Pair
  // succeeds must not untick listening on a machine that is not one yet.
  const isAssistant = model.network?.role === "assistant";
  const listenLabel = document.createElement("label");
  listenLabel.className = "settings__check";
  const listenBox = document.createElement("input");
  listenBox.type = "checkbox";
  listenBox.name = "listen";
  listenBox.checked = isAssistant ? false : model.listen;
  listenBox.disabled = isAssistant;
  listenBox.addEventListener("change", () => h.onListen(listenBox.checked));
  listenLabel.append(listenBox, document.createTextNode(' Listen for "Maya" (on-device speech recognition)'));
  assistant.append(listenLabel);
  if (isAssistant) {
    // The main Maya owns notifications and listening while this machine is
    // an assistant: say so right under the toggle rather than leave it
    // looking merely unticked.
    const note = document.createElement("div");
    note.className = "settings__hint";
    note.dataset.for = "listen";
    note.textContent = "The main Maya notifies and listens for this machine.";
    assistant.append(note);
  } else if (model.listenError) {
    // The listener stopped on its own (Dictation off, no microphone…): say
    // why right under the toggle, or an unticked box looks like a glitch.
    const err = document.createElement("div");
    err.className = "settings__error";
    err.dataset.for = "listen";
    err.textContent = model.listenError;
    assistant.append(err);
  }

  const recLabel = document.createElement("label");
  recLabel.textContent = "Speech recognition";
  const rec = document.createElement("select");
  rec.name = "recognizer";
  for (const [v, text] of recognizerOptions()) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    rec.append(o);
  }
  rec.value = model.recognizer;
  rec.addEventListener("change", () => h.onRecognizer(rec.value === "builtin" ? "builtin" : "system"));
  recLabel.append(rec);
  assistant.append(recLabel);

  if (model.recognizer === "builtin") {
    const mb = (n: number) => `${Math.round(n / 1_000_000)} MB`;
    const chosen = model.models.find((m) => m.id === model.whisperModel);
    const modelPickerLabel = document.createElement("label");
    modelPickerLabel.textContent = "Model";
    const sel = document.createElement("select");
    sel.name = "whisperModel";
    for (const m of model.models) {
      const o = document.createElement("option");
      o.value = m.id;
      o.textContent = m.downloaded ? `${m.label} ✓` : m.label;
      sel.append(o);
    }
    sel.value = model.whisperModel;
    sel.addEventListener("change", () => h.onWhisperModel(sel.value));
    modelPickerLabel.append(sel);
    assistant.append(modelPickerLabel);

    const row = document.createElement("div");
    row.className = "settings__row";
    if (chosen && !chosen.downloaded) {
      const dl = document.createElement("button");
      dl.type = "button";
      dl.dataset.action = "download-model";
      if (model.downloading?.id === chosen.id) {
        dl.textContent = `Downloading ${chosen.label}…`;
      } else if (model.downloading) {
        dl.textContent = "Wait for the current download";
      } else {
        dl.textContent = `Download (${mb(chosen.bytes)})`;
      }
      dl.disabled = model.downloading !== null;
      dl.addEventListener("click", () => h.onDownloadModel(chosen.id));
      row.append(dl);
      if (!model.downloading) {
        const hint = document.createElement("div");
        hint.className = "settings__hint";
        hint.textContent = `Download the model once; it stays on ${thisComputerLower()}.`;
        row.append(hint);
      }
    }
    if (chosen?.downloaded) {
      const rm = document.createElement("button");
      rm.type = "button";
      rm.dataset.action = "remove-model";
      rm.dataset.model = chosen.id;
      rm.textContent = "Remove";
      rm.disabled = true;
      rm.title = "The model in use cannot be removed; pick another first.";
      row.append(rm);
    }
    for (const m of model.models) {
      if (!m.downloaded || m.id === model.whisperModel) continue;
      const rm = document.createElement("button");
      rm.type = "button";
      rm.dataset.action = "remove-model";
      rm.dataset.model = m.id;
      rm.textContent = `Remove ${m.label.split(" (")[0]}`;
      rm.addEventListener("click", () => h.onRemoveModel(m.id));
      row.append(rm);
    }
    assistant.append(row);
    if (model.downloading) {
      const bar = document.createElement("progress");
      bar.setAttribute("name", "modelDownload");
      bar.max = model.downloading.total;
      bar.value = model.downloading.received;
      const downloadingLabel = model.models.find((m) => m.id === model.downloading!.id)?.label ?? model.downloading.id;
      const pct = document.createElement("div");
      pct.className = "settings__progress";
      pct.textContent = `Downloading ${downloadingLabel}… ${Math.round((100 * model.downloading.received) / Math.max(1, model.downloading.total))}%`;
      assistant.append(bar, pct);
    }
  }

  const micLabel = document.createElement("label");
  micLabel.textContent = "Microphone";
  const mic = document.createElement("select");
  mic.name = "microphone";
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Built-in (recommended)";
  mic.append(auto);
  for (const name of model.microphones) {
    const o = document.createElement("option");
    o.value = name;
    o.textContent = name;
    mic.append(o);
  }
  mic.value = model.microphones.includes(model.microphone) ? model.microphone : "";
  mic.addEventListener("change", () => h.onMicrophone(mic.value));
  micLabel.append(mic);
  assistant.append(micLabel);

  const providerLabel = document.createElement("label");
  providerLabel.textContent = "Voice";
  const provider = document.createElement("select");
  provider.name = "voiceProvider";
  for (const [v, text] of [["builtin", `${builtinVoiceName()} (built in)`], ["elevenlabs", "ElevenLabs"]] as const) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    provider.append(o);
  }
  provider.value = model.voiceProvider;
  provider.addEventListener("change", () => h.onVoiceProvider(provider.value === "elevenlabs" ? "elevenlabs" : "builtin"));
  providerLabel.append(provider);
  notifications.append(providerLabel);

  if (model.voiceProvider === "elevenlabs") {
    const keyLabel = document.createElement("label");
    keyLabel.textContent = "ElevenLabs API key";
    const key = document.createElement("input");
    key.type = "password";
    key.name = "elevenKey";
    key.placeholder = model.elevenKeySet ? `saved in ${secretStore()}; paste to replace` : "paste your key";
    key.autocomplete = "off";
    key.addEventListener("change", () => {
      const k = key.value.trim();
      if (k) h.onElevenKey(k);
      key.value = "";
    });
    keyLabel.append(key);
    notifications.append(keyLabel);

    const voiceLabel = document.createElement("label");
    voiceLabel.textContent = "ElevenLabs voice";
    const voice = document.createElement("select");
    voice.name = "elevenVoice";
    for (const v of model.elevenVoices) {
      const o = document.createElement("option");
      o.value = v.voiceId;
      o.textContent = v.name;
      voice.append(o);
    }
    voice.value = model.elevenVoices.some((v) => v.voiceId === model.elevenVoiceId) ? model.elevenVoiceId : (model.elevenVoices[0]?.voiceId ?? "");
    voice.disabled = model.elevenVoices.length === 0;
    voice.addEventListener("change", () => h.onElevenVoice(voice.value));
    voiceLabel.append(voice);
    notifications.append(voiceLabel);
  }

  const tryBtn = document.createElement("button");
  tryBtn.type = "button";
  tryBtn.dataset.action = "try-voice";
  tryBtn.textContent = "Try the voice";
  tryBtn.addEventListener("click", () => h.onTryVoice());
  notifications.append(tryBtn);

  const roleLabel = document.createElement("label");
  roleLabel.textContent = "Network";
  const roleSel = document.createElement("select");
  roleSel.name = "networkRole";
  for (const [v, text] of [["off", "Off"], ["main", "Act as main Maya"], ["assistant", "Assistant to a main Maya"]] as const) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    roleSel.append(o);
  }
  roleSel.value = netRole;
  roleSel.addEventListener("change", () => h.onRole(roleSel.value === "main" || roleSel.value === "assistant" ? roleSel.value : "off"));
  roleLabel.append(roleSel);
  network.append(roleLabel);

  // Both roles carry a name: assistants show the main's in "Connected to …",
  // and the main shows an assistant's on its cards.
  const nameField = () => {
    const nameLabel = document.createElement("label");
    nameLabel.textContent = "This machine's name";
    const nameInput = document.createElement("input");
    nameInput.type = "text";
    nameInput.name = "networkName";
    nameInput.placeholder = "blank uses this computer's hostname";
    nameInput.value = netName;
    nameInput.addEventListener("input", () => { model.networkName = nameInput.value; });
    nameInput.addEventListener("change", () => h.onName(nameInput.value.trim()));
    nameLabel.append(nameInput);
    return nameLabel;
  };
  if (netRole === "main" && net.mainError) {
    const err = document.createElement("div");
    err.className = "settings__error";
    err.dataset.for = "network";
    err.textContent = net.mainError;
    network.append(err);
  }

  if (netRole === "main") {
    const portLabel = document.createElement("label");
    portLabel.textContent = "Port";
    const portInput = document.createElement("input");
    portInput.type = "number";
    portInput.name = "networkPort";
    portInput.placeholder = "4127";
    portInput.value = netPort ? String(netPort) : "";
    const readPort = () => {
      const n = Number(portInput.value);
      return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
    };
    portInput.addEventListener("input", () => { model.networkPort = readPort(); });
    portInput.addEventListener("change", () => h.onPort(readPort()));
    portLabel.append(portInput);
    network.append(portLabel);
    network.append(nameField());

    if (net.code && nowMs <= net.code.expiresAt) {
      const codeBox = document.createElement("div");
      codeBox.className = "settings__code";
      codeBox.textContent = formatPairingCode(net.code.code);
      network.append(codeBox);
      const expiry = document.createElement("div");
      expiry.className = "settings__hint settings__code-expiry";
      expiry.textContent = `expires in ${expiresInMinutes(net.code.expiresAt, nowMs)} min`;
      network.append(expiry);
    }

    const regen = document.createElement("button");
    regen.type = "button";
    regen.dataset.action = "regenerate-code";
    regen.textContent = net.code && nowMs <= net.code.expiresAt ? "Regenerate" : "Show pairing code";
    regen.addEventListener("click", () => h.onRegenerate());
    network.append(regen);

    const list = document.createElement("div");
    list.className = "settings__assistants";
    for (const a of net.assistants) {
      const row = document.createElement("div");
      row.className = "settings__assistant";
      const info = document.createElement("span");
      info.textContent = `${a.name} (${[a.platform, a.address].filter(Boolean).join(", ")}) — ${a.connected ? "Connected" : a.lastSeen !== null ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected"}`;
      if (a.note) {
        const note = document.createElement("div");
        note.className = "settings__hint";
        note.dataset.for = "assistant-note";
        note.textContent = a.note;
        info.append(note);
      }
      const rm = document.createElement("button");
      rm.type = "button";
      rm.dataset.action = "remove-assistant";
      rm.dataset.id = a.id;
      rm.textContent = "Remove";
      rm.addEventListener("click", () => h.onRemoveAssistant(a.id));
      row.append(info, rm);
      list.append(row);
    }
    network.append(list);
  } else if (netRole === "assistant") {
    // Values live on the model, updated on every keystroke (not just on
    // `change`), so an unrelated repaint (a `voice` or `network` event) never
    // wipes a half-typed field; `change` still drives when each is saved.
    const hostLabel = document.createElement("label");
    hostLabel.textContent = "Main's host or address";
    const hostInput = document.createElement("input");
    hostInput.type = "text";
    hostInput.name = "networkHost";
    hostInput.placeholder = "e.g. 192.168.1.42 or maya-mini.local";
    hostInput.value = netMainHost;
    hostInput.addEventListener("input", () => { model.networkMainHost = hostInput.value; });
    hostInput.addEventListener("change", () => h.onMainHost(hostInput.value.trim()));
    hostLabel.append(hostInput);
    network.append(hostLabel);

    const mainPortLabel = document.createElement("label");
    mainPortLabel.textContent = "Port";
    const mainPortInput = document.createElement("input");
    mainPortInput.type = "number";
    mainPortInput.name = "networkMainPort";
    mainPortInput.placeholder = "4127";
    mainPortInput.value = netMainPort ? String(netMainPort) : "";
    const readMainPort = () => {
      const n = Number(mainPortInput.value);
      return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
    };
    mainPortInput.addEventListener("input", () => { model.networkMainPort = readMainPort(); });
    mainPortInput.addEventListener("change", () => h.onMainPort(readMainPort()));
    mainPortLabel.append(mainPortInput);
    network.append(mainPortLabel);

    network.append(nameField());

    if (model.networkPaired && !model.networkRepair) {
      const again = document.createElement("button");
      again.type = "button";
      again.className = "settings__link";
      again.dataset.action = "pair-again";
      again.textContent = "Pair again";
      again.addEventListener("click", () => h.onPairAgain());
      network.append(again);
    }
  }
  if (netRole === "assistant" && (!model.networkPaired || model.networkRepair)) {
    const codeLabel = document.createElement("label");
    codeLabel.textContent = "Pairing code";
    const codeInput = document.createElement("input");
    codeInput.type = "text";
    codeInput.name = "networkCode";
    codeInput.placeholder = "483921";
    codeInput.value = netCode;
    codeInput.addEventListener("input", () => { model.networkCode = codeInput.value; });
    codeLabel.append(codeInput);
    network.append(codeLabel);

    const pairBtn = document.createElement("button");
    pairBtn.type = "button";
    pairBtn.dataset.action = "pair";
    pairBtn.textContent = model.networkPairing ? "Pairing…" : "Pair";
    pairBtn.disabled = !!model.networkPairing;
    pairBtn.addEventListener("click", () => {
      if (model.networkPairing) return;
      h.onPair((model.networkMainHost ?? "").trim(), model.networkMainPort ?? 0, (model.networkName ?? "").trim(), (model.networkCode ?? "").trim());
    });
    network.append(pairBtn);
  }

  // Shown whenever the select shows Assistant, saved or still being
  // previewed, so a failed Pair on a never-paired machine has somewhere to
  // put its message instead of only the page-level error banner.
  if (netRole === "assistant") {
    const statusText = model.networkError
      ? model.networkError
      : net.role === "assistant"
        ? net.assistant.connected
          ? `Connected to ${net.assistant.mainName ?? ""}`
          : net.assistant.error && !net.assistant.retrying
            ? net.assistant.error
            : "Reconnecting…"
        : null;
    if (statusText !== null) {
      const statusEl = document.createElement("div");
      statusEl.className = "settings__status settings__status--network";
      statusEl.textContent = statusText;
      network.append(statusEl);
      // While it retries, the last failure sits under "Reconnecting…".
      if (!model.networkError && net.role === "assistant" && !net.assistant.connected && net.assistant.retrying && net.assistant.error) {
        statusEl.title = net.assistant.error;
        const last = document.createElement("div");
        last.className = "settings__hint";
        last.dataset.for = "network-last-error";
        last.textContent = `Last error: ${net.assistant.error}`;
        network.append(last);
      }
    }
  }

  if (model.error) {
    const err = document.createElement("div");
    err.className = "settings__error";
    err.textContent = model.error;
    root.append(err);
  }
  return root;
}

export async function initSettings(): Promise<void> {
  const panel = document.getElementById("settings");
  if (!panel) return;

  const model: SettingsModel = {
    hookInstalled: null,
    completedTimeoutMinutes: 30,
    projectsDir: "",
    clonesDir: "",
    notifyOnAwaiting: true,
    speakNotifications: true,
    voiceProvider: "builtin",
    elevenKeySet: false,
    elevenVoices: [],
    elevenVoiceId: "",
    error: null,
    listen: false,
    microphone: "",
    microphones: [],
    agent: null,
    agentModel: "",
    agents: [],
    reviewPrompt: "",
    listenError: null,
    recognizer: "system",
    whisperModel: "base.en-q5_1",
    models: [],
    downloading: null,
    network: DEFAULT_NETWORK_STATUS,
    networkRole: "off",
    networkPort: 0,
    networkMainHost: "",
    networkMainPort: 0,
    networkName: "",
  };

  const applyNetworkConfig = (c: ConfigJson) => {
    model.networkPaired = !!(c.network?.assistantId && c.network?.token);
    model.networkPort = c.network?.port ?? 0;
    model.networkMainHost = c.network?.mainHost ?? "";
    model.networkMainPort = c.network?.mainPort ?? 0;
    model.networkName = c.network?.name ?? "";
  };

  const loadVoices = async () => {
    if (model.voiceProvider !== "elevenlabs") return;
    model.elevenKeySet = await invoke<boolean>("has_elevenlabs_key");
    if (!model.elevenKeySet) return;
    model.elevenVoices = await invoke<ElevenVoice[]>("list_elevenlabs_voices");
  };

  const loadModels = async () => {
    try {
      model.models = await invoke<ModelInfo[]>("list_whisper_models");
    } catch {
      model.models = [];
    }
  };

  const applyConfig = (c: ConfigJson) => {
    model.completedTimeoutMinutes = c.completedTimeoutMinutes;
    model.projectsDir = c.projectsDir ?? "";
    model.clonesDir = c.clonesDir ?? "";
    model.notifyOnAwaiting = c.notifyOnAwaiting;
    model.speakNotifications = c.speakNotifications;
    model.voiceProvider = c.voiceProvider;
    model.elevenVoiceId = c.elevenlabsVoiceId ?? "";
    model.listen = c.listen;
    model.microphone = c.microphone ?? "";
    model.agent = c.agent ?? null;
    model.agentModel = c.agentModel ?? "";
    model.reviewPrompt = c.reviewPrompt ?? "";
    model.recognizer = c.recognizer;
    model.whisperModel = c.whisperModel;
    applyNetworkConfig(c);
  };

  const saveConfig = async (patch: Partial<ConfigJson>) => {
    // The local model can be stale for fields it doesn't own (e.g. `listen`,
    // which the voice panel can flip independently), so fetch the freshest
    // config and merge the patch onto that rather than onto `model`.
    const fresh = await invoke<ConfigJson>("get_config");
    const c = await invoke<ConfigJson>("set_config", {
      config: { ...fresh, ...patch },
    });
    applyConfig(c);
  };

  const saveNetwork = (patch: Partial<NetworkConfigJson>) =>
    run(async () => {
      const fresh = await invoke<ConfigJson>("get_config");
      const net: NetworkConfigJson = { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", assistantId: "", token: "", assistants: [], ...fresh.network, ...patch };
      const c = await invoke<ConfigJson>("set_config", { config: { ...fresh, network: net } });
      applyNetworkConfig(c);
    });

  let stale = false;
  // A repaint replaces every field: the one being used gets its focus and
  // caret back, so a status push never pulls the cursor out from under you.
  const paint = () => {
    const focused = focusedField(panel);
    panel.replaceChildren(renderSettings(model, handlers));
    if (focused) restoreFocus(panel, focused);
  };

  const run = async (action: () => Promise<void>) => {
    model.error = null;
    try {
      await action();
    } catch (e) {
      model.error = String(e);
    }
    paint();
  };

  const handlers: SettingsHandlers = {
    onCodexInstall: () => void run(async () => { model.codexHookInstalled = await invoke<boolean>("install_codex_hook"); }),
    onCodexRemove: () => void run(async () => { model.codexHookInstalled = await invoke<boolean>("remove_codex_hook"); }),
    onInstall: () => void run(async () => { model.hookInstalled = await invoke<boolean>("install_hook"); }),
    onRemove: () => void run(async () => { model.hookInstalled = await invoke<boolean>("remove_hook"); }),
    onTimeout: (minutes) => void run(() => saveConfig({ completedTimeoutMinutes: minutes })),
    onProjectsDir: (path) => void run(() => saveConfig({ projectsDir: path || null })),
    onNotify: (enabled) => void run(() => saveConfig({ notifyOnAwaiting: enabled })),
    onClonesDir: (path) => void run(() => saveConfig({ clonesDir: path || null })),
    onSpeak: (enabled) => void run(() => saveConfig({ speakNotifications: enabled })),
    onFullDiskAccess: () => void run(() => invoke<void>("open_full_disk_access")),
    onVoiceProvider: (provider) => void run(async () => { await saveConfig({ voiceProvider: provider }); await loadVoices(); }),
    onElevenKey: (key) => void run(async () => { await invoke("set_elevenlabs_key", { key }); await loadVoices(); }),
    onElevenVoice: (voiceId) => void run(() => saveConfig({ elevenlabsVoiceId: voiceId || null })),
    onTryVoice: () => void run(() => invoke("try_voice")),
    onListen: (on) => void run(async () => { await invoke("voice_listen", { on }); model.listen = on; }),
    onMicrophone: (name) => void run(() => saveConfig({ microphone: name || null })),
    onAgent: (agent) => void run(() => saveConfig({ agent, agentModel: "" })),
    onAgentModel: (agent, id) => void run(() => saveConfig({ agent, agentModel: id })),
    onReviewPrompt: (text) => void run(() => saveConfig({ reviewPrompt: text })),
    onRecognizer: (r) => void run(() => saveConfig({ recognizer: r })),
    onWhisperModel: (id) => void run(() => saveConfig({ whisperModel: id })),
    onDownloadModel: (id) => {
      model.downloading = { id, received: 0, total: model.models.find((m) => m.id === id)?.bytes ?? 0 };
      paint();
      void run(async () => {
        try {
          await invoke("download_whisper_model", { id });
        } finally {
          model.downloading = null;
          await loadModels();
        }
      });
    },
    onRemoveModel: (id) => void run(async () => { await invoke("remove_whisper_model", { id }); await loadModels(); }),
    onRole: (role) => {
      // A failed Pair's message belongs to the pairing form; leaving it drops the message.
      if (role !== "assistant") model.networkError = null;
      if (role === "assistant" && !model.networkPaired) {
        // Previewing the pairing form does not save anything; only a
        // successful Pair (see `onPair`) commits the assistant role. With
        // credentials stored, Assistant saves at once and reconnects.
        model.networkRole = "assistant";
        paint();
        return;
      }
      model.networkRole = role;
      void saveNetwork({ role });
    },
    onPort: (port) => void saveNetwork({ port }),
    onMainHost: (host) => void saveNetwork({ mainHost: host }),
    onMainPort: (port) => void saveNetwork({ mainPort: port }),
    onName: (name) => void saveNetwork({ name }),
    onPairAgain: () => {
      model.networkRepair = true;
      paint();
    },
    onRegenerate: () => void run(async () => { model.network = await invoke<NetworkStatus>("network_pairing_code"); }),
    onRemoveAssistant: (id) => void run(async () => { model.network = await invoke<NetworkStatus>("network_remove_assistant", { id }); }),
    onPair: (host, port, name, code) => {
      // One Pair at a time: a second would pair twice with one code.
      if (model.networkPairing) return;
      model.networkPairing = true;
      // Shown next to the pairing form itself (`.settings__status--network`),
      // not just the page-level banner: cleared by this attempt starting, and
      // by either a success or a fresh attempt afterwards.
      model.error = null;
      model.networkError = null;
      paint();
      void (async () => {
        try {
          model.network = await invoke<NetworkStatus>("network_pair", { host, port, name, code });
          model.networkRole = model.network.role;
          model.networkMainHost = host;
          model.networkMainPort = port;
          model.networkName = name;
          model.networkCode = "";
          // The new pairing's id and token replaced the stored ones.
          model.networkPaired = true;
          model.networkRepair = false;
          model.networkPairing = false;
        } catch (e) {
          model.networkError = String(e);
          model.networkPairing = false;
        }
        paint();
      })();
    },
  };

  // Others write the config too (the first-start modal saves the agent), and
  // a stale agent here would pair a model with the wrong agent: read it again
  // when the tab is shown, and when a save elsewhere says it changed. A
  // hidden pane is only marked stale, and repainted once shown.
  const refetchAndPaint = () =>
    void invoke<ConfigJson>("get_config")
      .then((c) => {
        applyConfig(c);
        stale = true;
      })
      .catch(() => undefined)
      .finally(() => {
        if (!panel.hidden && stale) {
          stale = false;
          paint();
        }
      });
  // Registered before the first await, so an early first-start save (with
  // Settings already the visible tab) cannot be missed.
  window.addEventListener("maya:config-changed", refetchAndPaint);

  // The voice panel can turn listening on or off on its own; mirror that
  // into the model so the Settings checkbox doesn't show a stale state.
  // Every partial transcript emits `voice`: repaint only when listening
  // actually changed, or a repaint would wipe a field being typed in.
  await listen<VoiceStatus>("voice", (e) => {
    const listenError = e.payload.state === "error" && e.payload.detail ? e.payload.detail : null;
    if (model.listen === e.payload.listening && model.listenError === listenError) return;
    model.listen = e.payload.listening;
    model.listenError = listenError;
    if (!panel.hidden) paint();
  });

  // Sent on every change; repaint only when the status actually changed, or
  // an unrelated push (e.g. a ping-driven refresh) would repaint for nothing.
  await listen<NetworkStatus>("network", (e) => {
    if (JSON.stringify(model.network) === JSON.stringify(e.payload)) return;
    // Previewing "Assistant" before Pair succeeds is local only; an
    // unrelated event must not snap the form back to the saved role.
    const previewingAssistant = model.networkRole === "assistant" && e.payload.role !== "assistant";
    model.network = e.payload;
    if (!previewingAssistant) model.networkRole = e.payload.role;
    if (!panel.hidden) paint();
    else stale = true;
  });

  // Events that arrive while the tab is hidden update the model only; a
  // repaint when the tab is shown brings the pane up to date.
  new MutationObserver(() => {
    if (!panel.hidden) refetchAndPaint();
  }).observe(panel, { attributes: true, attributeFilter: ["hidden"] });

  // The pairing code's countdown, and its end. Updated in place: a full
  // repaint here would wipe unsaved text in the other sections every 30 s.
  setInterval(() => {
    const now = Date.now();
    if (panel.hidden || !pairingRepaintDue(model, now)) return;
    tickPairingCode(panel, model, now);
  }, PAIRING_TICK_MS);

  await listen<{ id: string; received: number; total: number }>("voice-model", (e) => {
    // Only one model downloads at a time; ignore stray events for any other
    // id (e.g. a late event from a download that was superseded).
    if (model.downloading === null || e.payload.id !== model.downloading.id) return;
    model.downloading = e.payload.received >= e.payload.total ? null : e.payload;
    if (!panel.hidden) paint();
  });

  await run(async () => {
    // An unreadable ~/.codex/hooks.json must not keep the rest of Settings
    // from loading: the Codex hook then shows as not installed, and
    // installing it reports what is wrong with the file.
    const codexStatus = invoke<boolean>("codex_hook_status").catch(() => false);
    const focusVisible = invoke<boolean>("focus_visible").catch(() => true);
    const [installed, config, codexInstalled] = await Promise.all([invoke<boolean>("hook_status"), invoke<ConfigJson>("get_config"), codexStatus]);
    model.focusVisible = await focusVisible;
    model.hookInstalled = installed;
    model.codexHookInstalled = codexInstalled;
    applyConfig(config);
    await loadVoices();
    await loadModels();
    try {
      const raw = await invoke<string>("voice_selftest");
      const parsed = JSON.parse(raw) as { devices?: string[] };
      model.microphones = parsed.devices ?? [];
    } catch {
      // ignore: the microphone list is a nicety, not required to use settings
    }
    try {
      model.network = await invoke<NetworkStatus>("network_status");
      model.networkRole = model.network.role;
    } catch {
      // ignore: an older backend or a stray failure just leaves Network off
    }
  });

  // Listing installed agents can take seconds (it probes each harness), so
  // it runs after the rest of Settings is already usable rather than inside
  // the `run` above, which would otherwise block the whole panel on it.
  void invoke<{ agents: AgentInfo[] }>("list_agents", { machine: "" })
    .then((r) => {
      model.agents = r.agents;
      if (!panel.hidden) paint();
      else stale = true;
    })
    .catch(() => undefined);
}
