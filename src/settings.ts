import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "./format";
import type { VoiceStatus } from "./voice";
import { builtinVoiceName, isLinux, isMac, isWindows, recognizerOptions, secretStore, thisComputerLower } from "./platform";
import type { ColorTheme, GitProvider, Harness, NewSessionTerminal, ViewerTheme } from "./types";
import { setViewerTheme } from "./modal";
import { COLOR_THEMES, applyColorTheme, asColorTheme } from "./theme";
import { KEY_ACTIONS, addCombo, asKeyBindings, comboLabel, comboOf, defaultKeys, removeCombo, setKeyBindings, type KeyBindings } from "./keys";
import { OPEN_SETTINGS_GROUP } from "./reviews";
import { CLAUDE_AGENT, type AgentInfo } from "./newsession";
import { harnessLabel } from "./harness";
import { promoteLines } from "./promote";
import { renderReleaseNotes } from "./release-notes";
import { DEFAULT_REVIEW_PROMPT } from "./reviewprompt";
import { updates } from "./update";

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
  /** Connected and able to become the main (`lead`). */
  canLead?: boolean;
  /** Connected and able to follow a new main (`follow`); false for an older Maya. */
  canFollow?: boolean;
  /** The main lets it see and control every session. */
  mirror?: boolean;
  /** Connected and asked to mirror: its switch shows. */
  wantsMirror?: boolean;
}

export interface AssistantLink {
  connected: boolean;
  mainName: string | null;
  error: string | null;
  /** The client waits to try again; `error` is then the last failure. */
  retrying?: boolean;
  /** What the main said about mirroring at the last connect. */
  mirror?: "allowed" | "refused" | "unsupported" | null;
  /** The word the main listens for, from its last board; none from an older main. */
  wakeWord?: string | null;
}

/** The live `network_status`/`network` event payload. */
export interface NetworkStatus {
  role: NetworkRole;
  code: PairingCode | null;
  assistants: AssistantStatus[];
  assistant: AssistantLink;
  /** Why the main's server is not running (the port is taken…). */
  mainError?: string | null;
  /** This machine's IPv4 addresses, for an assistant to type as the main's host. */
  addresses?: string[];
  /** The open code was struck out by 3 wrong tries. */
  codeUsedUp?: boolean;
  /** This Maya was updated to secure pairing and must pair again. */
  pairAgain?: boolean;
}

/**
 * The role the form shows after a `network` event. Previewing Assistant
 * (picked, not yet paired) survives an event that still reports the live
 * role; a live role change, from a promotion or another machine, wins.
 */
export function nextNetworkRole(prev: NetworkStatus | undefined, selected: NetworkRole | undefined, payload: NetworkStatus): NetworkRole {
  const previewing = selected === "assistant" && prev?.role !== "assistant" && payload.role !== "assistant";
  return previewing ? "assistant" : payload.role;
}

const DEFAULT_NETWORK_STATUS: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } };

type StatuslineStatus = { installed: boolean; blockedBy?: string | null };

export interface SettingsModel {
  /** The session viewer's look; chat when absent. */
  viewerTheme?: ViewerTheme;
  /** The colours of the windows; dark when absent. */
  colorTheme?: ColorTheme;
  /** The keys that send and make a new line; the defaults when absent. */
  keys?: KeyBindings;
  /** The macOS app new sessions open in; Terminal when absent. */
  newSessionTerminal?: NewSessionTerminal;
  hookInstalled: boolean | null;
  codexHookInstalled?: boolean | null;
  /** Whether Maya's usage status line wrapper is installed; null while loading. */
  statusline?: { installed: boolean; blockedBy?: string | null } | null;
  completedTimeoutMinutes: number;
  projectsDir: string;
  clonesDir: string;
  notifyOnAwaiting: boolean;
  /** Banner when a PR assigned to the user is approved or needs changes; absent reads as on. */
  notifyOnPrReview?: boolean;
  /** Show the Debug tab and Maya's own card; absent reads as off. */
  debugMode?: boolean;
  speakNotifications: boolean;
  /** False when Maya cannot tell whether a Focus mode is on (macOS without Full Disk Access). */
  focusVisible?: boolean;
  elevenEnabled: boolean;
  elevenKeySet: boolean;
  elevenVoices: ElevenVoice[];
  elevenVoiceId: string;
  serverEnabled: boolean;
  serverUrl: string;
  serverVoice: string;
  serverModel: string;
  serverKeySet: boolean;
  /** The voices the server lists; empty when it has no list or the listing failed. */
  serverVoices: string[];
  serverVoicesError: string | null;
  /** What the last voice test says each voice did. */
  voiceReport: string | null;
  /** How loud Maya speaks, in percent; absent reads as 100. */
  voiceVolume?: number;
  error: string | null;
  listen: boolean;
  /** The word Maya listens for; blank (or absent) is "Maya". */
  listenWord?: string;
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
  /** The app version, shown under General; unset until Tauri answers. */
  version?: string;
  /** The group the sidebar shows; General when unset. */
  settingsGroup?: SettingsGroup;
  /** GitHub then GitLab, from `gitConnections`. */
  git?: GitConnectionJson[];
  /** Whether a pasted token is stored, and the last Test's outcome. */
  gitStatus?: Record<GitProvider, { hasToken: boolean; test?: string | null }>;
  /** This install can check for updates (`update_kind` is not `off`). */
  updatesOn?: boolean;
  /** The outcome of the last Check for updates; unset before one. */
  updateStatus?: string;
  /** The live status from `network_status`/the `network` event. */
  network?: NetworkStatus;
  /** The Network select's current value; can diverge from `network.role` while previewing "Assistant" before Pair succeeds. */
  networkRole?: NetworkRole;
  /** The assistant whose Make main confirmation is open. */
  networkPromote?: string | null;
  /** A handover is running; it can take up to 30 s. */
  networkPromoting?: boolean;
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
  /** The config holds the main's key: choosing Assistant reconnects without pairing. */
  networkPaired?: boolean;
  /** "Pair again" was clicked: the pairing form shows even though credentials are stored. */
  networkRepair?: boolean;
  /** A Pair is in flight: the button is disabled and further clicks are ignored. */
  networkPairing?: boolean;
  /** New key's confirmation is open. */
  networkNewKeyConfirm?: boolean;
}

export interface VoiceServerJson {
  enabled: boolean;
  url: string;
  voice: string;
  model: string;
}
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
  onViewerTheme?(t: ViewerTheme): void;
  onColorTheme?(t: ColorTheme): void;
  onKeys?(k: KeyBindings): void;
  onNewSessionTerminal?(t: NewSessionTerminal): void;
  onGitEnabled?(p: GitProvider, on: boolean): void;
  onGitTokenKind?(p: GitProvider, kind: "stored" | "env"): void;
  onGitEnvName?(p: GitProvider, name: string): void;
  onGitBaseUrl?(url: string): void;
  onGitClone?(p: GitProvider, protocol: "ssh" | "https"): void;
  onGitToken?(p: GitProvider, token: string): void;
  onGitClearToken?(p: GitProvider): void;
  onGitTest?(p: GitProvider): void;
  onInstall(): void;
  onCodexInstall?(): void;
  onCodexRemove?(): void;
  onStatuslineInstall?(): void;
  onStatuslineRemove?(): void;
  onRemove(): void;
  onTimeout(minutes: number): void;
  onProjectsDir(path: string): void;
  onNotify(enabled: boolean): void;
  onNotifyPrReview?(enabled: boolean): void;
  onDebugMode?(on: boolean): void;
  onClonesDir(path: string): void;
  onSpeak(enabled: boolean): void;
  onFullDiskAccess?(): void;
  onCheckUpdates?(): void;
  onElevenEnabled(on: boolean): void;
  onElevenKey(key: string): void;
  onElevenVoice(voiceId: string): void;
  onServerEnabled(on: boolean): void;
  onServerUrl(url: string): void;
  onServerVoice(voice: string): void;
  onServerModel(model: string): void;
  onServerKey(key: string): void;
  onTryVoice(): void;
  onVoiceVolume(percent: number): void;
  onListen(on: boolean): void;
  onListenWord?(word: string): void;
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
  onMakeMain(id: string): void;
  onMirror(id: string, on: boolean): void;
  onPromote(id: string): void;
  onPromoteCancel(): void;
  onNewKeyAsk(): void;
  onNewKey(): void;
  onNewKeyCancel(): void;
  onForgetMain(): void;
  onPair(host: string, port: number, name: string, code: string): void;
}

interface NetworkConfigJson {
  role: NetworkRole;
  port: number;
  mainHost: string;
  mainPort: number;
  name: string;
  mainKey: string;
  pairAgain?: boolean;
  assistants: { id: string; name: string; hostname: string; platform: string; token: string; address?: string; lastSeen?: number | null }[];
}

export interface GitConnectionJson {
  provider: GitProvider;
  enabled: boolean;
  /** GitLab only; null is https://gitlab.com. */
  baseUrl?: string | null;
  token: { kind: "stored" } | { kind: "env"; name: string };
  /** How a review clones; ssh when absent. */
  clone?: "ssh" | "https";
}

const GIT_DEFAULT_ENV: Record<GitProvider, string> = { github: "GH_TOKEN", gitlab: "GITLAB_TOKEN" };
const GIT_LABEL: Record<GitProvider, string> = { github: "GitHub", gitlab: "GitLab" };

/** One GitHub and one GitLab connection, in that order, as the Rust side defaults them. */
export function gitConnections(raw: GitConnectionJson[] | null | undefined): GitConnectionJson[] {
  return (["github", "gitlab"] as const).map((provider) => {
    const found = raw?.find((c) => c.provider === provider);
    return found ? { ...found, baseUrl: found.baseUrl ?? null, clone: found.clone ?? "ssh" } : { provider, enabled: provider === "github", baseUrl: null, token: { kind: "env", name: GIT_DEFAULT_ENV[provider] }, clone: "ssh" };
  });
}

interface ConfigJson {
  viewerTheme?: ViewerTheme;
  colorTheme?: ColorTheme;
  keys?: KeyBindings | null;
  newSessionTerminal?: NewSessionTerminal;
  git?: GitConnectionJson[] | null;
  completedTimeoutMinutes: number;
  projectsDir?: string | null;
  clonesDir?: string | null;
  notifyOnAwaiting: boolean;
  notifyOnPrReview?: boolean;
  debugMode?: boolean;
  speakNotifications: boolean;
  /** Read only to migrate: an old config that chose ElevenLabs has no `elevenlabsEnabled`. */
  voiceProvider?: string;
  elevenlabsEnabled?: boolean | null;
  elevenlabsVoiceId?: string | null;
  voiceServer?: VoiceServerJson;
  voiceVolume?: number;
  listen: boolean;
  listenWord?: string | null;
  microphone?: string | null;
  agent?: Harness | null;
  agentModel?: string;
  reviewPrompt?: string;
  recognizer: Recognizer;
  whisperModel: string;
  network?: NetworkConfigJson;
}

/** The sidebar's groups, in its order. */
export const SETTINGS_GROUPS = [
  ["general", "General"],
  ["keyboard", "Keyboard"],
  ["sessions", "Sessions"],
  ["git", "Git"],
  ["reviews", "Pull requests"],
  ["notifications", "Notifications"],
  ["voice", "Voice"],
  ["network", "Network"],
  ["debug", "Debug"],
  ["releaseNotes", "Release Notes"],
] as const;
export type SettingsGroup = (typeof SETTINGS_GROUPS)[number][0];

export const SETTINGS_GROUP_KEY = "maya.settingsGroup";

function asSettingsGroup(v: string | null | undefined): SettingsGroup {
  return SETTINGS_GROUPS.find(([key]) => key === v)?.[0] ?? "general";
}

/** The group open last time, per browser; General when none or unknown. */
export function storedSettingsGroup(): SettingsGroup {
  try {
    return asSettingsGroup(localStorage.getItem(SETTINGS_GROUP_KEY));
  } catch {
    return "general";
  }
}

/** A titled card, one per sidebar group. */
function section(title: string, key: SettingsGroup): HTMLElement {
  const s = document.createElement("section");
  s.className = `settings__section settings__section--${key}`;
  s.dataset.group = key;
  const heading = document.createElement("h2");
  heading.className = "settings__heading";
  heading.textContent = title;
  s.append(heading);
  return s;
}

/** A titled part of a section, such as Voice's Speaking and Listening. */
function subsection(title: string): HTMLElement {
  const s = document.createElement("div");
  s.className = "settings__subsection";
  const heading = document.createElement("h3");
  heading.className = "settings__subheading";
  heading.textContent = title;
  s.append(heading);
  return s;
}

/**
 * Settings › Keyboard: each action's combinations as chips, an Add button
 * that records the next Enter press, and Reset to defaults.
 */
function renderKeyboard(section: HTMLElement, keys: KeyBindings, h: SettingsHandlers): void {
  const hint = document.createElement("div");
  hint.className = "settings__hint";
  hint.textContent = "The keys for the Chat tab, a session's reply box and the new-session prompt. An Enter press in neither list does what the box does by itself.";
  section.append(hint);
  for (const [action, name] of KEY_ACTIONS) {
    const row = document.createElement("div");
    row.className = "settings__row settings__keys";
    row.dataset.keys = action;
    const label = document.createElement("span");
    label.className = "settings__keys-name";
    label.textContent = name;
    row.append(label);
    for (const combo of keys[action]) {
      const chip = document.createElement("span");
      chip.className = "chip";
      chip.append(comboLabel(combo));
      if (action !== "send" || keys.send.length > 1) {
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "chip__remove";
        remove.setAttribute("aria-label", `Remove ${comboLabel(combo)}`);
        remove.textContent = "×";
        remove.addEventListener("click", () => h.onKeys?.(removeCombo(keys, action, combo)));
        chip.append(remove);
      }
      row.append(chip);
    }
    const add = document.createElement("button");
    add.type = "button";
    add.dataset.action = "keys-add";
    add.textContent = "Add";
    const error = document.createElement("span");
    error.className = "settings__error";
    let recording = false;
    const stop = () => {
      recording = false;
      add.textContent = "Add";
    };
    add.addEventListener("click", () => {
      recording = true;
      error.textContent = "";
      add.textContent = "Press keys… (Esc to cancel)";
    });
    add.addEventListener("blur", stop);
    add.addEventListener("keydown", (ev) => {
      if (!recording) return;
      if (ev.key === "Escape") {
        ev.preventDefault();
        stop();
        return;
      }
      const combo = comboOf(ev);
      if (!combo) return;
      ev.preventDefault();
      stop();
      const next = addCombo(keys, action, combo);
      if (typeof next === "string") error.textContent = next;
      else h.onKeys?.(next);
    });
    row.append(add, error);
    section.append(row);
  }
  const reset = document.createElement("button");
  reset.type = "button";
  reset.dataset.action = "keys-reset";
  reset.textContent = "Reset to defaults";
  reset.addEventListener("click", () => h.onKeys?.(defaultKeys()));
  section.append(reset);
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
  /** Set for radios, which share a name across the group. */
  value?: string;
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
  if (el instanceof HTMLInputElement && el.type === "radio") focused.value = el.value;
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
  const named = [...root.querySelectorAll("input, select, textarea")].filter((e): e is Field => isField(e) && e.name === f.name);
  const el = (f.value !== undefined && named.find((e) => e.value === f.value)) || named[0];
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

function addressesLine(addresses: string[]): HTMLElement {
  const line = document.createElement("div");
  line.className = "settings__hint settings__addresses";
  line.textContent = `This machine's addresses: ${addresses.join(", ")}`;
  return line;
}

/**
 * Puts a fresh address list on the page in place, since no event follows
 * a Wi‑Fi, VPN or sleep/wake change. A repaint would wipe unsaved edits in
 * the other fields. Returns whether anything changed.
 */
export function tickAddresses(panel: ParentNode, model: SettingsModel, addresses: string[]): boolean {
  const before = model.network?.addresses ?? [];
  if (before.length === addresses.length && before.every((a, i) => a === addresses[i])) return false;
  if (model.network) model.network.addresses = addresses;
  const line = panel.querySelector<HTMLElement>(".settings__addresses");
  if (addresses.length === 0) {
    line?.remove();
  } else if (line) {
    line.textContent = addressesLine(addresses).textContent;
  } else {
    panel.querySelector("input[name=networkPort]")?.closest("label")?.insertAdjacentElement("afterend", addressesLine(addresses));
  }
  return true;
}

/**
 * True while a pairing code is on screen or has just expired: the timer
 * ticks then, so the countdown moves and the code goes when it expires.
 */
export function pairingRepaintDue(model: SettingsModel, nowMs: number): boolean {
  const code = model.network?.code;
  return (model.networkRole ?? model.network?.role) === "main" && !!code && nowMs <= code.expiresAt + PAIRING_TICK_MS;
}

/** The places a new session can open on this platform, as [value, label]. */
export function terminalChoices(): [NewSessionTerminal, string][] {
  if (isMac()) return [["terminal", "Terminal"], ["iterm", "iTerm2"], ["native", "Native"]];
  if (isWindows()) return [["terminal", "Windows Terminal"], ["native", "Native"]];
  if (isLinux()) return [["terminal", "Terminal (tmux)"], ["native", "Native"]];
  return [["terminal", "Terminal"], ["native", "Native"]];
}

export function renderSettings(model: SettingsModel, h: SettingsHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = document.createElement("div");
  root.className = "settings__body settings__body--sidebar";
  const nav = document.createElement("nav");
  nav.className = "settings__nav";
  nav.setAttribute("aria-label", "Settings");
  const content = document.createElement("div");
  content.className = "settings__content";
  root.append(nav, content);

  // Every group is built on each paint and the sidebar only toggles
  // `hidden`, so a switch keeps half-typed fields and the in-place ticks
  // (pairing code, addresses) still find their elements.
  const sections = Object.fromEntries(SETTINGS_GROUPS.map(([key, title]) => [key, section(title, key)])) as Record<SettingsGroup, HTMLElement>;
  const navButtons = SETTINGS_GROUPS.map(([key, title]) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "settings__nav-item";
    b.dataset.group = key;
    b.textContent = title;
    const choose = () => {
      model.settingsGroup = key;
      try {
        localStorage.setItem(SETTINGS_GROUP_KEY, key);
      } catch {
        /* private mode: nothing to remember */
      }
      show(key);
    };
    // On the press, not the click: the press blurs a just-edited field, and
    // the save its `change` starts can repaint (and replace this button)
    // before the release, so the click would never land. `click` still
    // serves the keyboard.
    b.addEventListener("pointerdown", (e) => { if (e.button === 0) choose(); });
    b.addEventListener("click", choose);
    return b;
  });
  const show = (group: SettingsGroup) => {
    for (const [key, s] of Object.entries(sections)) s.hidden = key !== group;
    for (const b of navButtons) {
      if (b.dataset.group === group) b.setAttribute("aria-current", "true");
      else b.removeAttribute("aria-current");
    }
  };
  nav.append(...navButtons);
  content.append(...Object.values(sections));
  show(model.settingsGroup ?? "general");
  const { general, keyboard, sessions, git, reviews, notifications, voice, network, debug, releaseNotes } = sections;
  const speaking = subsection("Speaking");
  const listening = subsection("Listening");
  voice.append(speaking, listening);

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
  agentSel.addEventListener("change", () => {
    // The old agent's models stay on screen until the save repaints; a pick
    // in that window would pair the new agent with one of them.
    agentModelSel.disabled = true;
    h.onAgent(agentSel.value as Harness);
  });
  agentLabel.append(agentSel);
  general.append(agentLabel);
  const agentHint = document.createElement("div");
  agentHint.className = "settings__hint";
  agentHint.dataset.for = "agent";
  agentHint.textContent = "Interprets your voice commands, picks folders for 'Let Maya choose', and is the default for new and resumed sessions and reviews.";
  general.append(agentHint);

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
  general.append(agentModelLabel);

  const themeLabel = document.createElement("label");
  themeLabel.textContent = "Theme";
  const themeSel = document.createElement("select");
  themeSel.name = "colorTheme";
  for (const [value, text] of COLOR_THEMES) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    themeSel.append(o);
  }
  themeSel.value = model.colorTheme ?? "dark";
  themeSel.addEventListener("change", () => h.onColorTheme?.(asColorTheme(themeSel.value)));
  themeLabel.append(themeSel);
  general.append(themeLabel);

  renderKeyboard(keyboard, model.keys ?? defaultKeys(), h);

  if (model.version) {
    const version = document.createElement("div");
    version.className = "settings__hint settings__version";
    version.textContent = `Maya ${model.version}`;
    if (model.updatesOn && h.onCheckUpdates) {
      const check = document.createElement("button");
      check.type = "button";
      check.dataset.action = "check-updates";
      check.textContent = "Check for updates";
      check.addEventListener("click", () => h.onCheckUpdates?.());
      version.append(" ", check);
      if (model.updateStatus) {
        const status = document.createElement("span");
        status.className = "settings__update-status";
        status.textContent = model.updateStatus;
        version.append(" ", status);
      }
    }
    general.append(version);
  }

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

  const line = model.statusline ?? null;
  const lineStatus = document.createElement("div");
  lineStatus.className = "settings__status";
  lineStatus.textContent =
    line === null
      ? "Checking status line…"
      : line.blockedBy
        ? `Usage and limits cannot be recorded: the status line is set in ${line.blockedBy}.`
        : line.installed
          ? "Usage and limits are recorded from Claude Code's status line."
          : "Usage and limits are not recorded. Install the status line wrapper to see them in session windows.";
  const lineButton = document.createElement("button");
  lineButton.type = "button";
  lineButton.dataset.action = line?.installed ? "remove-statusline" : "install-statusline";
  lineButton.textContent = line?.installed ? "Remove usage status line" : "Install usage status line";
  lineButton.disabled = line === null || !!line.blockedBy;
  lineButton.addEventListener("click", () => line?.installed ? h.onStatuslineRemove?.() : h.onStatuslineInstall?.());
  sessions.append(lineStatus, lineButton);

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
  const viewerLabel = document.createElement("label");
  viewerLabel.textContent = "Session viewer";
  const viewerSel = document.createElement("select");
  viewerSel.name = "viewerTheme";
  for (const [value, text] of [["chat", "Chat"], ["terminal", "Terminal"]] as const) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    viewerSel.append(o);
  }
  viewerSel.value = model.viewerTheme ?? "chat";
  viewerSel.addEventListener("change", () => h.onViewerTheme?.(viewerSel.value as ViewerTheme));
  viewerLabel.append(viewerSel);
  sessions.append(viewerLabel);
  const terminalLabel = document.createElement("label");
  terminalLabel.textContent = "New sessions open in";
  const terminalSel = document.createElement("select");
  terminalSel.name = "newSessionTerminal";
  const choices = terminalChoices();
  for (const [value, text] of choices) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    terminalSel.append(o);
  }
  const saved = model.newSessionTerminal ?? "terminal";
  terminalSel.value = choices.some(([value]) => value === saved) ? saved : "terminal";
  terminalSel.addEventListener("change", () => h.onNewSessionTerminal?.(terminalSel.value as NewSessionTerminal));
  terminalLabel.append(terminalSel);
  sessions.append(terminalLabel);

  const clonesLabel = document.createElement("label");
  clonesLabel.textContent = "Clones directory (for reviews and worktrees)";
  const clonesInput = document.createElement("input");
  clonesInput.type = "text";
  clonesInput.name = "clonesDir";
  clonesInput.placeholder = "~/dev/reviews";
  clonesInput.value = model.clonesDir;
  clonesInput.addEventListener("change", () => h.onClonesDir(clonesInput.value.trim()));
  clonesLabel.append(clonesInput);
  reviews.append(clonesLabel);

  const reviewLabel = document.createElement("label");
  reviewLabel.textContent = "Review prompt (for the Review button)";
  const reviewInput = document.createElement("textarea");
  reviewInput.name = "reviewPrompt";
  reviewInput.rows = 4;
  reviewInput.placeholder = DEFAULT_REVIEW_PROMPT;
  reviewInput.value = model.reviewPrompt;
  reviewInput.addEventListener("change", () => h.onReviewPrompt(reviewInput.value.trim()));
  reviewLabel.append(reviewInput);
  reviews.append(reviewLabel);
  const reviewHint = document.createElement("div");
  reviewHint.className = "settings__hint";
  reviewHint.dataset.for = "reviewPrompt";
  reviewHint.textContent = "Blank uses the built-in prompt. {number}, {repo} and {url} are filled in; a prompt that uses none gets \" PR #<number> (<url>)\" appended.";
  reviews.append(reviewHint);

  const notifyLabel = document.createElement("label");
  notifyLabel.className = "settings__check";
  const notifyBox = document.createElement("input");
  notifyBox.type = "checkbox";
  notifyBox.name = "notify";
  notifyBox.checked = model.notifyOnAwaiting;
  notifyBox.addEventListener("change", () => h.onNotify(notifyBox.checked));
  notifyLabel.append(notifyBox, document.createTextNode(" Notify me when a session awaits a decision"));
  notifications.append(notifyLabel);

  const prLabel = document.createElement("label");
  prLabel.className = "settings__check";
  const prBox = document.createElement("input");
  prBox.type = "checkbox";
  prBox.name = "notifyPrReview";
  prBox.checked = model.notifyOnPrReview !== false;
  prBox.addEventListener("change", () => h.onNotifyPrReview?.(prBox.checked));
  prLabel.append(prBox, document.createTextNode(" Notify me when a PR assigned to me is approved or needs changes"));
  reviews.append(prLabel);

  const debugLabel = document.createElement("label");
  debugLabel.className = "settings__check";
  const debugBox = document.createElement("input");
  debugBox.type = "checkbox";
  debugBox.name = "debug-mode";
  debugBox.checked = model.debugMode === true;
  debugBox.addEventListener("change", () => h.onDebugMode?.(debugBox.checked));
  debugLabel.append(debugBox, document.createTextNode(" Debug mode: show the Debug tab and Maya's own card"));
  debug.append(debugLabel);

  releaseNotes.append(renderReleaseNotes());

  for (const conn of model.git ?? gitConnections(null)) {
    const p = conn.provider;
    const status = model.gitStatus?.[p] ?? { hasToken: false };
    const box = document.createElement("div");
    box.className = "settings__git";
    box.dataset.provider = p;

    const onLabel = document.createElement("label");
    onLabel.className = "settings__check";
    const on = document.createElement("input");
    on.type = "checkbox";
    on.name = `gitEnabled-${p}`;
    on.checked = conn.enabled;
    on.addEventListener("change", () => h.onGitEnabled?.(p, on.checked));
    onLabel.append(on, document.createTextNode(` ${GIT_LABEL[p]}`));
    box.append(onLabel);

    if (p === "gitlab") {
      const urlLabel = document.createElement("label");
      urlLabel.textContent = "URL";
      const url = document.createElement("input");
      url.type = "text";
      url.name = "gitBaseUrl-gitlab";
      url.placeholder = "https://gitlab.com";
      url.value = conn.baseUrl ?? "";
      url.disabled = !conn.enabled;
      url.addEventListener("change", () => h.onGitBaseUrl?.(url.value.trim()));
      urlLabel.append(url);
      box.append(urlLabel);
    }

    const kinds = document.createElement("div");
    kinds.className = "settings__git-kinds";
    for (const [kind, text] of [["stored", "Paste a token"], ["env", "From env var"]] as const) {
      const l = document.createElement("label");
      l.className = "settings__check";
      const r = document.createElement("input");
      r.type = "radio";
      r.name = `gitTokenKind-${p}`;
      r.value = kind;
      r.checked = conn.token.kind === kind;
      r.disabled = !conn.enabled;
      r.addEventListener("change", () => h.onGitTokenKind?.(p, kind));
      l.append(r, document.createTextNode(` ${text}`));
      kinds.append(l);
    }
    box.append(kinds);

    const clones = document.createElement("div");
    clones.className = "settings__git-kinds";
    clones.append(document.createTextNode("Clone with "));
    for (const [protocol, text] of [["ssh", "SSH"], ["https", "HTTPS"]] as const) {
      const l = document.createElement("label");
      l.className = "settings__check";
      const r = document.createElement("input");
      r.type = "radio";
      r.name = `gitClone-${p}`;
      r.value = protocol;
      r.checked = (conn.clone ?? "ssh") === protocol;
      r.disabled = !conn.enabled;
      r.addEventListener("change", () => h.onGitClone?.(p, protocol));
      l.append(r, document.createTextNode(` ${text}`));
      clones.append(l);
    }
    box.append(clones);

    if (conn.token.kind === "stored") {
      const tokLabel = document.createElement("label");
      tokLabel.textContent = `${GIT_LABEL[p]} token`;
      const tok = document.createElement("input");
      tok.type = "password";
      tok.name = `gitToken-${p}`;
      tok.autocomplete = "off";
      tok.disabled = !conn.enabled;
      tok.placeholder = status.hasToken ? `saved in ${secretStore()}; paste to replace` : "paste your token";
      tok.addEventListener("change", () => {
        const t = tok.value.trim();
        tok.value = "";
        if (t) h.onGitToken?.(p, t);
      });
      tokLabel.append(tok);
      box.append(tokLabel);
      if (status.hasToken) {
        const clear = document.createElement("button");
        clear.type = "button";
        clear.className = "settings__link";
        clear.dataset.action = "git-clear";
        clear.textContent = "Clear";
        clear.addEventListener("click", () => h.onGitClearToken?.(p));
        box.append(clear);
      }
    } else {
      const envLabel = document.createElement("label");
      envLabel.textContent = "Variable";
      const env = document.createElement("input");
      env.type = "text";
      env.name = `gitEnvName-${p}`;
      env.placeholder = GIT_DEFAULT_ENV[p];
      env.value = conn.token.name;
      env.disabled = !conn.enabled;
      env.addEventListener("change", () => h.onGitEnvName?.(p, env.value.trim()));
      envLabel.append(env);
      box.append(envLabel);
    }

    const test = document.createElement("button");
    test.type = "button";
    test.className = "settings__link";
    test.dataset.action = "git-test";
    test.textContent = "Test";
    test.disabled = !conn.enabled;
    test.addEventListener("click", () => h.onGitTest?.(p));
    box.append(test);
    if (status.test) {
      const out = document.createElement("div");
      out.className = "settings__hint settings__git-test";
      out.textContent = status.test;
      box.append(out);
    }
    git.append(box);
  }
  const gitHint = document.createElement("div");
  gitHint.className = "settings__hint";
  gitHint.textContent = "Maya reads pull requests and merge requests with these tokens. GitHub needs the repo scope; GitLab needs read_api. Reviews clone over SSH or HTTPS with your own git credentials.";
  git.append(gitHint);

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
  const word = model.listenWord?.trim() || "Maya";
  listenLabel.append(listenBox, document.createTextNode(` Listen for "${word}" (on-device speech recognition)`));
  listening.append(listenLabel);
  if (isAssistant) {
    // The main Maya owns notifications and listening while this machine is
    // an assistant: say so right under the toggle rather than leave it
    // looking merely unticked.
    const note = document.createElement("div");
    note.className = "settings__hint";
    note.dataset.for = "listen";
    note.textContent = "The main Maya notifies and listens for this machine.";
    listening.append(note);
  } else if (model.listenError) {
    // The listener stopped on its own (Dictation off, no microphone…): say
    // why right under the toggle, or an unticked box looks like a glitch.
    const err = document.createElement("div");
    err.className = "settings__error";
    err.dataset.for = "listen";
    err.textContent = model.listenError;
    listening.append(err);
  }

  const wordLabel = document.createElement("label");
  wordLabel.textContent = "Wake word";
  const wordInput = document.createElement("input");
  wordInput.type = "text";
  wordInput.name = "listenWord";
  wordInput.placeholder = "Maya";
  wordInput.value = model.listenWord ?? "";
  wordInput.disabled = isAssistant;
  wordInput.addEventListener("change", () => h.onListenWord?.(wordInput.value.trim()));
  wordLabel.append(wordInput);
  listening.append(wordLabel);
  const wordHint = document.createElement("div");
  wordHint.className = "settings__hint";
  wordHint.dataset.for = "listenWord";
  wordHint.textContent = "Blank is \"Maya\". A phone joined to this Maya listens for the same word. Pick a word the speech recognition spells the way you type it.";
  listening.append(wordHint);

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
  listening.append(recLabel);

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
    listening.append(modelPickerLabel);

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
    listening.append(row);
    if (model.downloading) {
      const bar = document.createElement("progress");
      bar.setAttribute("name", "modelDownload");
      bar.max = model.downloading.total;
      bar.value = model.downloading.received;
      const downloadingLabel = model.models.find((m) => m.id === model.downloading!.id)?.label ?? model.downloading.id;
      const pct = document.createElement("div");
      pct.className = "settings__progress";
      pct.textContent = `Downloading ${downloadingLabel}… ${Math.round((100 * model.downloading.received) / Math.max(1, model.downloading.total))}%`;
      listening.append(bar, pct);
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
  listening.append(micLabel);

  const order = document.createElement("p");
  order.dataset.for = "voice-order";
  order.textContent = `Maya speaks with the first voice that works: ElevenLabs, then the voice server, then ${builtinVoiceName()} (built in).`;
  speaking.append(order);

  const checkbox = (name: string, text: string, checked: boolean, onChange: (on: boolean) => void) => {
    const label = document.createElement("label");
    label.className = "settings__check";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.name = name;
    box.checked = checked;
    box.addEventListener("change", () => onChange(box.checked));
    label.append(box, document.createTextNode(` ${text}`));
    return label;
  };
  const textField = (name: string, text: string, value: string, placeholder: string, onChange: (v: string) => void) => {
    const label = document.createElement("label");
    label.textContent = text;
    const input = document.createElement("input");
    input.type = "text";
    input.name = name;
    input.value = value;
    input.placeholder = placeholder;
    input.addEventListener("change", () => onChange(input.value.trim()));
    label.append(input);
    return label;
  };
  const keyField = (name: string, text: string, placeholder: string, onKey: (k: string) => void) => {
    const label = document.createElement("label");
    label.textContent = text;
    const key = document.createElement("input");
    key.type = "password";
    key.name = name;
    key.placeholder = placeholder;
    key.autocomplete = "off";
    key.addEventListener("change", () => {
      const k = key.value.trim();
      if (k) onKey(k);
      key.value = "";
    });
    label.append(key);
    return label;
  };

  speaking.append(checkbox("elevenEnabled", "Use ElevenLabs", model.elevenEnabled, (on) => h.onElevenEnabled(on)));
  if (model.elevenEnabled) {
    speaking.append(keyField("elevenKey", "ElevenLabs API key", model.elevenKeySet ? `saved in ${secretStore()}; paste to replace` : "paste your key", (k) => h.onElevenKey(k)));

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
    speaking.append(voiceLabel);
  }

  speaking.append(checkbox("serverEnabled", "Use a voice server (OpenAI-compatible)", model.serverEnabled, (on) => h.onServerEnabled(on)));
  if (model.serverEnabled) {
    speaking.append(textField("serverUrl", "Server address", model.serverUrl, "http://localhost:8004", (u) => h.onServerUrl(u)));

    if (model.serverVoices.length > 0) {
      const voiceLabel = document.createElement("label");
      voiceLabel.textContent = "Voice";
      const voice = document.createElement("select");
      voice.name = "serverVoice";
      // A saved voice the server no longer lists stays selectable.
      // A blank one is the server's own default: the list's `default` when it has one.
      const blankIsListed = model.serverVoice === "" && model.serverVoices.includes("default");
      const saved = blankIsListed ? "default" : model.serverVoice;
      for (const v of model.serverVoices.includes(saved) ? model.serverVoices : [saved, ...model.serverVoices]) {
        const o = document.createElement("option");
        o.value = v;
        o.textContent = v || "server default";
        voice.append(o);
      }
      voice.value = saved;
      voice.addEventListener("change", () => h.onServerVoice(voice.value));
      voiceLabel.append(voice);
      speaking.append(voiceLabel);
    } else {
      speaking.append(textField("serverVoice", "Voice", model.serverVoice, "default", (v) => h.onServerVoice(v)));
    }
    if (model.serverVoicesError) {
      const listed = document.createElement("p");
      listed.dataset.for = "server-voices";
      listed.textContent = `Could not list the server's voices: ${model.serverVoicesError}`;
      speaking.append(listed);
    }

    speaking.append(textField("serverModel", "Model", model.serverModel, "optional", (m) => h.onServerModel(m)));
    speaking.append(keyField("serverKey", "API key", model.serverKeySet ? `saved in ${secretStore()}; paste to replace` : "optional; paste a key", (k) => h.onServerKey(k)));
  }

  // The number follows the drag; the volume is saved once the slider is let go.
  const volumeLabel = document.createElement("label");
  volumeLabel.textContent = "Voice volume ";
  const volume = document.createElement("input");
  volume.type = "range";
  volume.name = "voiceVolume";
  volume.min = "10";
  volume.max = "100";
  volume.step = "5";
  volume.value = String(model.voiceVolume ?? 100);
  const volumeShown = document.createElement("span");
  volumeShown.dataset.for = "voice-volume";
  volumeShown.textContent = `${volume.value}%`;
  volume.addEventListener("input", () => { volumeShown.textContent = `${volume.value}%`; });
  volume.addEventListener("change", () => h.onVoiceVolume(Number(volume.value)));
  const volumeControl = document.createElement("span");
  volumeControl.className = "settings__range";
  volumeControl.append(volume, volumeShown);
  volumeLabel.append(volumeControl);
  speaking.append(volumeLabel);

  const tryBtn = document.createElement("button");
  tryBtn.type = "button";
  tryBtn.dataset.action = "try-voice";
  tryBtn.textContent = "Try the voice";
  tryBtn.addEventListener("click", () => h.onTryVoice());
  speaking.append(tryBtn);

  if (model.voiceReport) {
    const report = document.createElement("p");
    report.dataset.for = "voice-report";
    report.textContent = model.voiceReport;
    speaking.append(report);
  }

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
  roleSel.disabled = !!model.networkPromoting;
  roleSel.addEventListener("change", () => h.onRole(roleSel.value === "main" || roleSel.value === "assistant" ? roleSel.value : "off"));
  roleLabel.append(roleSel);
  network.append(roleLabel);

  if (netRole !== "off" && net.pairAgain) {
    const note = document.createElement("div");
    note.className = "settings__hint";
    note.dataset.for = "pair-again-note";
    note.textContent = "This Maya was updated to secure pairing — pair again.";
    network.append(note);
  }

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
    nameInput.disabled = !!model.networkPromoting;
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
    portInput.disabled = !!model.networkPromoting;
    const readPort = () => {
      const n = Number(portInput.value);
      return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
    };
    portInput.addEventListener("input", () => { model.networkPort = readPort(); });
    portInput.addEventListener("change", () => h.onPort(readPort()));
    portLabel.append(portInput);
    network.append(portLabel);
    if (net.addresses && net.addresses.length > 0) network.append(addressesLine(net.addresses));
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
    } else if (net.codeUsedUp) {
      const used = document.createElement("div");
      used.className = "settings__hint";
      used.textContent = "Code used up after 3 wrong tries — Regenerate.";
      network.append(used);
    }

    const regen = document.createElement("button");
    regen.type = "button";
    regen.dataset.action = "regenerate-code";
    regen.textContent = net.code && nowMs <= net.code.expiresAt ? "Regenerate" : "Show pairing code";
    // Reopening pairing mid-handover would let a new assistant pair with a main about to stop.
    regen.disabled = !!model.networkPromoting;
    regen.addEventListener("click", () => h.onRegenerate());
    network.append(regen);

    const list = document.createElement("div");
    list.className = "settings__assistants";
    for (const a of net.assistants) {
      const row = document.createElement("div");
      row.className = "settings__assistant";
      const info = document.createElement("span");
      info.textContent = `${a.name} (${[a.platform === "android" ? "Android" : a.platform, a.address].filter(Boolean).join(", ")}) — ${a.connected ? "Connected" : a.lastSeen !== null ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected"}`;
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
      rm.disabled = !!model.networkPromoting;
      rm.addEventListener("click", () => h.onRemoveAssistant(a.id));
      // Name and status on one line, the buttons on their own row beneath,
      // so a long status never squeezes them into a column.
      const actions = document.createElement("div");
      actions.className = "settings__assistant-actions";
      actions.append(rm);
      row.append(info, actions);
      if (a.connected && a.canLead && !(a.wantsMirror || a.mirror)) {
        const mk = document.createElement("button");
        mk.type = "button";
        mk.dataset.action = "make-main";
        mk.dataset.id = a.id;
        mk.textContent = "Make main";
        mk.disabled = !!model.networkPromoting;
        mk.addEventListener("click", () => h.onMakeMain(a.id));
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
        box.disabled = !!model.networkPromoting;
        box.addEventListener("change", () => h.onMirror(a.id, box.checked));
        sw.append(box, document.createTextNode(" Sees and controls all sessions"));
        actions.append(sw);
      }
      list.append(row);
      if (model.networkPromote === a.id) {
        const confirm = document.createElement("div");
        confirm.className = "settings__confirm";
        confirm.dataset.for = "promote-confirm";
        for (const line of promoteLines(net.assistants, a.id, thisComputerLower())) {
          const p = document.createElement("p");
          p.textContent = line;
          confirm.append(p);
        }
        const yes = document.createElement("button");
        yes.type = "button";
        yes.dataset.action = "promote-confirm";
        yes.textContent = model.networkPromoting ? "Handing over…" : "Make main";
        yes.disabled = !!model.networkPromoting;
        yes.addEventListener("click", () => h.onPromote(a.id));
        const no = document.createElement("button");
        no.type = "button";
        no.dataset.action = "promote-cancel";
        no.textContent = "Cancel";
        no.disabled = !!model.networkPromoting;
        no.addEventListener("click", () => h.onPromoteCancel());
        confirm.append(yes, no);
        list.append(confirm);
      }
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
    hostInput.disabled = !!model.networkPromoting;
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
    mainPortInput.disabled = !!model.networkPromoting;
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

      const forget = document.createElement("button");
      forget.type = "button";
      forget.className = "settings__link";
      forget.dataset.action = "forget-main";
      forget.textContent = "Forget main";
      forget.addEventListener("click", () => h.onForgetMain());
      network.append(forget);
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

  if (netRole !== "off") {
    if (model.networkNewKeyConfirm) {
      const confirm = document.createElement("div");
      confirm.className = "settings__confirm";
      confirm.dataset.for = "new-key-confirm";
      const p = document.createElement("p");
      p.textContent = "Drop every pairing and make a new key?";
      confirm.append(p);
      const yes = document.createElement("button");
      yes.type = "button";
      yes.dataset.action = "new-key-confirm";
      yes.textContent = "Drop and make a new key";
      yes.addEventListener("click", () => h.onNewKey());
      const no = document.createElement("button");
      no.type = "button";
      no.dataset.action = "new-key-cancel";
      no.textContent = "Cancel";
      no.addEventListener("click", () => h.onNewKeyCancel());
      confirm.append(yes, no);
      network.append(confirm);
    } else {
      const fresh = document.createElement("button");
      fresh.type = "button";
      fresh.className = "settings__link";
      fresh.dataset.action = "new-key";
      fresh.textContent = "New key";
      fresh.disabled = !!model.networkPromoting;
      fresh.addEventListener("click", () => h.onNewKeyAsk());
      network.append(fresh);
    }
  }

  if (model.error) {
    const err = document.createElement("div");
    err.className = "settings__error";
    err.textContent = model.error;
    // Above the sections, so it shows whichever group is open.
    content.prepend(err);
  }
  return root;
}

export async function initSettings(): Promise<void> {
  const panel = document.getElementById("settings");
  if (!panel) return;

  const model: SettingsModel = {
    hookInstalled: null,
    statusline: null,
    completedTimeoutMinutes: 30,
    projectsDir: "",
    clonesDir: "",
    notifyOnAwaiting: true,
    speakNotifications: true,
    elevenEnabled: false,
    elevenKeySet: false,
    elevenVoices: [],
    elevenVoiceId: "",
    serverEnabled: false,
    serverUrl: "",
    serverVoice: "",
    serverModel: "",
    serverKeySet: false,
    serverVoices: [],
    serverVoicesError: null,
    voiceReport: null,
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
    settingsGroup: storedSettingsGroup(),
  };

  const applyNetworkConfig = (c: ConfigJson) => {
    model.networkPaired = !!c.network?.mainKey;
    model.networkPort = c.network?.port ?? 0;
    model.networkMainHost = c.network?.mainHost ?? "";
    model.networkMainPort = c.network?.mainPort ?? 0;
    model.networkName = c.network?.name ?? "";
  };

  const loadVoices = async () => {
    // Each voice loads on its own: one failing (a bad key) must not stop the
    // other refreshing. The ElevenLabs failure still reaches the page error.
    let failed: unknown = null;
    if (model.elevenEnabled) {
      try {
        model.elevenKeySet = await invoke<boolean>("has_elevenlabs_key");
        if (model.elevenKeySet) model.elevenVoices = await invoke<ElevenVoice[]>("list_elevenlabs_voices");
      } catch (e) {
        failed = e;
      }
    }
    model.serverVoices = [];
    model.serverVoicesError = null;
    if (model.serverEnabled) {
      model.serverKeySet = await invoke<boolean>("has_voice_server_key");
      if (model.serverUrl) {
        try {
          model.serverVoices = await invoke<string[]>("list_server_voices");
        } catch (e) {
          model.serverVoicesError = String(e);
        }
      }
    }
    if (failed) throw failed;
  };

  const loadGitStatus = async () => {
    try {
      const s = await invoke<Record<GitProvider, { hasToken: boolean }>>("get_git_status");
      model.gitStatus = { github: { ...model.gitStatus?.github, ...s.github }, gitlab: { ...model.gitStatus?.gitlab, ...s.gitlab } };
    } catch {
      model.gitStatus ??= { github: { hasToken: false }, gitlab: { hasToken: false } };
    }
  };

  /** Applies `change` to one connection of the freshest config and saves the list. */
  const saveGit = (p: GitProvider, change: (c: GitConnectionJson) => void) =>
    run(async () => {
      const fresh = await invoke<ConfigJson>("get_config");
      const git = gitConnections(fresh.git);
      change(git.find((c) => c.provider === p)!);
      const c = await invoke<ConfigJson>("set_config", { config: { ...fresh, git } });
      applyConfig(c);
    });

  const setTest = (p: GitProvider, text: string) => {
    model.gitStatus = { github: { hasToken: false }, gitlab: { hasToken: false }, ...model.gitStatus, [p]: { ...model.gitStatus?.[p], hasToken: model.gitStatus?.[p]?.hasToken ?? false, test: text } };
  };

  const loadModels = async () => {
    try {
      model.models = await invoke<ModelInfo[]>("list_whisper_models");
    } catch {
      model.models = [];
    }
  };

  const applyConfig = (c: ConfigJson) => {
    model.voiceReport = null;
    model.completedTimeoutMinutes = c.completedTimeoutMinutes;
    model.projectsDir = c.projectsDir ?? "";
    model.clonesDir = c.clonesDir ?? "";
    model.notifyOnAwaiting = c.notifyOnAwaiting;
    model.notifyOnPrReview = c.notifyOnPrReview !== false;
    model.debugMode = c.debugMode === true;
    model.speakNotifications = c.speakNotifications;
    model.elevenEnabled = c.elevenlabsEnabled ?? c.voiceProvider === "elevenlabs";
    model.serverEnabled = c.voiceServer?.enabled ?? false;
    model.serverUrl = c.voiceServer?.url ?? "";
    model.serverVoice = c.voiceServer?.voice ?? "";
    model.serverModel = c.voiceServer?.model ?? "";
    model.elevenVoiceId = c.elevenlabsVoiceId ?? "";
    model.voiceVolume = c.voiceVolume ?? 100;
    model.listen = c.listen;
    model.listenWord = c.listenWord ?? "";
    model.microphone = c.microphone ?? "";
    model.agent = c.agent ?? null;
    model.agentModel = c.agentModel ?? "";
    model.reviewPrompt = c.reviewPrompt ?? "";
    model.recognizer = c.recognizer;
    model.whisperModel = c.whisperModel;
    model.git = gitConnections(c.git);
    model.viewerTheme = c.viewerTheme ?? "chat";
    setViewerTheme(model.viewerTheme);
    model.colorTheme = asColorTheme(c.colorTheme);
    applyColorTheme(model.colorTheme);
    model.keys = asKeyBindings(c.keys);
    setKeyBindings(model.keys);
    model.newSessionTerminal = c.newSessionTerminal ?? "terminal";
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

  const saveServer = async (patch: Partial<VoiceServerJson>) => {
    const fresh = await invoke<ConfigJson>("get_config");
    const server: VoiceServerJson = { enabled: false, url: "", voice: "", model: "", ...fresh.voiceServer, ...patch };
    const c = await invoke<ConfigJson>("set_config", { config: { ...fresh, voiceServer: server } });
    applyConfig(c);
  };

  const saveNetwork = (patch: Partial<NetworkConfigJson>) =>
    run(async () => {
      const fresh = await invoke<ConfigJson>("get_config");
      const net: NetworkConfigJson = { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [], ...fresh.network, ...patch };
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
    onGitEnabled: (p, on) => void saveGit(p, (c) => { c.enabled = on; }),
    onGitBaseUrl: (url) => void saveGit("gitlab", (c) => { c.baseUrl = url || null; }),
    onGitClone: (p, protocol) => void saveGit(p, (c) => { c.clone = protocol; }),
    onGitTokenKind: (p, kind) => void saveGit(p, (c) => { c.token = kind === "stored" ? { kind: "stored" } : { kind: "env", name: GIT_DEFAULT_ENV[p] }; }),
    onGitEnvName: (p, name) => void saveGit(p, (c) => { c.token = { kind: "env", name }; }),
    onGitToken: (p, token) => void run(async () => { await invoke("store_git_token", { provider: p, token }); await loadGitStatus(); }),
    onGitClearToken: (p) => void run(async () => { await invoke("clear_git_token", { provider: p }); await loadGitStatus(); }),
    onGitTest: (p) => {
      setTest(p, "Testing…");
      paint();
      invoke<string>("test_git_connection", { provider: p })
        .then((login) => setTest(p, `Connected as @${login}`))
        .catch((e) => setTest(p, String(e)))
        .finally(() => { if (!panel.hidden) paint(); });
    },
    onCheckUpdates: () => {
      model.updateStatus = "Checking…";
      paint();
      updates
        .check(true)
        .then((info) => {
          model.updateStatus = info ? `Maya ${info.version} is ${updates.kind() === "self" ? "ready" : "out"}: see the bar above.` : "Up to date.";
        })
        .catch((e) => {
          model.updateStatus = String(e);
        })
        .finally(() => {
          if (!panel.hidden) paint();
        });
    },
    onCodexInstall: () => void run(async () => { model.codexHookInstalled = await invoke<boolean>("install_codex_hook"); }),
    onCodexRemove: () => void run(async () => { model.codexHookInstalled = await invoke<boolean>("remove_codex_hook"); }),
    onStatuslineInstall: () => void run(async () => { model.statusline = await invoke<StatuslineStatus>("install_statusline"); }),
    onStatuslineRemove: () => void run(async () => { model.statusline = await invoke<StatuslineStatus>("remove_statusline"); }),
    onInstall: () => void run(async () => { model.hookInstalled = await invoke<boolean>("install_hook"); }),
    onRemove: () => void run(async () => { model.hookInstalled = await invoke<boolean>("remove_hook"); }),
    onTimeout: (minutes) => void run(() => saveConfig({ completedTimeoutMinutes: minutes })),
    onProjectsDir: (path) => void run(() => saveConfig({ projectsDir: path || null })),
    onNotify: (enabled) => void run(() => saveConfig({ notifyOnAwaiting: enabled })),
    onNotifyPrReview: (enabled) => void run(() => saveConfig({ notifyOnPrReview: enabled })),
    onDebugMode: (on) =>
      void run(async () => {
        await saveConfig({ debugMode: on });
        window.dispatchEvent(new CustomEvent("maya:config-changed"));
      }),
    onClonesDir: (path) => void run(() => saveConfig({ clonesDir: path || null })),
    onSpeak: (enabled) => void run(() => saveConfig({ speakNotifications: enabled })),
    onFullDiskAccess: () => void run(() => invoke<void>("open_full_disk_access")),
    onElevenEnabled: (on) => void run(async () => { await saveConfig({ elevenlabsEnabled: on }); await loadVoices(); }),
    onElevenKey: (key) => void run(async () => { await invoke("set_elevenlabs_key", { key }); await loadVoices(); }),
    onElevenVoice: (voiceId) => void run(() => saveConfig({ elevenlabsVoiceId: voiceId || null })),
    onServerEnabled: (on) => void run(async () => { await saveServer({ enabled: on }); await loadVoices(); }),
    onServerUrl: (url) => void run(async () => { await saveServer({ url }); await loadVoices(); }),
    onServerVoice: (voice) => void run(() => saveServer({ voice })),
    onServerModel: (model) => void run(() => saveServer({ model })),
    onServerKey: (key) => void run(async () => { await invoke("set_voice_server_key", { key }); await loadVoices(); }),
    onTryVoice: () => void run(async () => { model.voiceReport = null; model.voiceReport = await invoke<string>("try_voice"); }),
    onVoiceVolume: (percent) => void run(() => saveConfig({ voiceVolume: percent })),
    onListen: (on) => void run(async () => { await invoke("voice_listen", { on }); model.listen = on; }),
    onListenWord: (word) => void run(() => saveConfig({ listenWord: word || null })),
    onMicrophone: (name) => void run(() => saveConfig({ microphone: name || null })),
    onAgent: (agent) => void run(() => saveConfig({ agent, agentModel: "" })),
    onAgentModel: (agent, id) => void run(() => saveConfig({ agent, agentModel: id })),
    onReviewPrompt: (text) => void run(() => saveConfig({ reviewPrompt: text })),
    onRecognizer: (r) => void run(() => saveConfig({ recognizer: r })),
    onViewerTheme: (t) => void run(() => saveConfig({ viewerTheme: t })),
    onColorTheme: (t) => {
      // At once, without waiting for the save: the window should change as the choice is made.
      model.colorTheme = t;
      applyColorTheme(t);
      void run(() => saveConfig({ colorTheme: t }));
    },
    onKeys: (k) => {
      // At once, like the theme: the boxes follow the new keys before the save lands.
      model.keys = k;
      setKeyBindings(k);
      paint();
      void run(() => saveConfig({ keys: k }));
    },
    onNewSessionTerminal: (t) => void run(() => saveConfig({ newSessionTerminal: t })),
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
    onMirror: (id, on) => void run(async () => { model.network = await invoke<NetworkStatus>("network_set_mirror", { id, on }); }),
    onRemoveAssistant: (id) => void run(async () => { model.network = await invoke<NetworkStatus>("network_remove_assistant", { id }); }),
    onMakeMain: (id) => {
      model.networkPromote = id;
      paint();
    },
    onPromoteCancel: () => {
      model.networkPromote = null;
      paint();
    },
    onNewKeyAsk: () => {
      model.networkNewKeyConfirm = true;
      paint();
    },
    onNewKeyCancel: () => {
      model.networkNewKeyConfirm = false;
      paint();
    },
    onNewKey: () => {
      model.networkNewKeyConfirm = false;
      void run(async () => {
        model.network = await invoke<NetworkStatus>("network_new_key");
        model.networkPaired = false;
      });
    },
    onForgetMain: () => void run(async () => {
      model.network = await invoke<NetworkStatus>("network_forget_main");
      model.networkPaired = false;
    }),
    onPromote: (id) => {
      if (model.networkPromoting) return;
      model.networkPromoting = true;
      paint();
      void run(async () => {
        try {
          model.network = await invoke<NetworkStatus>("network_promote", { id });
          model.networkRole = model.network.role;
          model.networkPromote = null;
          refetchAndPaint();
        } finally {
          model.networkPromoting = false;
        }
      });
    },
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

  // Registered before the first await too: the Pull requests tab can ask for
  // the Git group while Settings is still loading.
  let loaded = false;
  document.addEventListener(OPEN_SETTINGS_GROUP, (e) => {
    const group = (e as CustomEvent<string>).detail;
    if (!SETTINGS_GROUPS.some(([key]) => key === group)) return;
    model.settingsGroup = group as SettingsGroup;
    try {
      localStorage.setItem(SETTINGS_GROUP_KEY, group);
    } catch {
      /* private mode: nothing to remember */
    }
    // Before the first load there is nothing to draw yet; its own paint shows the group.
    if (loaded) paint();
  });

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
    const role = nextNetworkRole(model.network, model.networkRole, e.payload);
    // A promotion, or a move made from another machine, changes the role or
    // the main's name; the Host and Port fields come from the config.
    const moved = model.network?.role !== e.payload.role || model.network?.assistant?.mainName !== e.payload.assistant?.mainName;
    model.network = e.payload;
    model.networkRole = role;
    if (moved) refetchAndPaint();
    if (!panel.hidden) paint();
    else stale = true;
  });

  // Events that arrive while the tab is hidden update the model only; a
  // repaint when the tab is shown brings the pane up to date.
  new MutationObserver(() => {
    if (!panel.hidden) refetchAndPaint();
  }).observe(panel, { attributes: true, attributeFilter: ["hidden"] });

  // The pairing code's countdown and its end, and the main's addresses
  // (no event follows an interface change). Updated in place: a full
  // repaint here would wipe unsaved text in the other sections every 30 s.
  setInterval(() => {
    const now = Date.now();
    if (panel.hidden) return;
    if ((model.networkRole ?? model.network?.role) === "main") {
      void invoke<NetworkStatus>("network_status").then((s) => tickAddresses(panel, model, s.addresses ?? [])).catch(() => undefined);
    }
    if (pairingRepaintDue(model, now)) tickPairingCode(panel, model, now);
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
    const statusline = invoke<StatuslineStatus>("statusline_status").catch((): StatuslineStatus => ({ installed: false }));
    const focusVisible = invoke<boolean>("focus_visible").catch(() => true);
    // Purely informational, so a failure just leaves the line out.
    const version = getVersion().catch(() => "");
    const [installed, config, codexInstalled] = await Promise.all([invoke<boolean>("hook_status"), invoke<ConfigJson>("get_config"), codexStatus]);
    model.focusVisible = await focusVisible;
    model.version = await version;
    await updates.ready();
    model.updatesOn = updates.kind() !== "off";
    model.hookInstalled = installed;
    model.codexHookInstalled = codexInstalled;
    model.statusline = await statusline;
    applyConfig(config);
    await loadVoices();
    await loadGitStatus();
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
  loaded = true;

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
