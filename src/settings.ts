import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { VoiceStatus } from "./voice";
import { builtinVoiceName, isLinux, isWindows, recognizerOptions, secretStore, thisComputerLower } from "./platform";
import type { ColorTheme, CoralItems, CoralStatus, CustomTool, ForgeItems, GitProvider, Harness, ItemRule, NewSessionTerminal, ViewerTheme, WorkItemsConfig } from "./types";
import { setViewerTheme } from "./modal";
import { COLOR_THEMES, applyColorTheme, asColorTheme } from "./theme";
import { KEY_ACTIONS, addCombo, asKeyBindings, comboLabel, comboOf, defaultKeys, removeCombo, setKeyBindings, type KeyBindings } from "./keys";
import { OPEN_SETTINGS_GROUP } from "./reviews";
import { CLAUDE_AGENT, type AgentInfo } from "./newsession";
import { harnessLabel } from "./harness";
import art from "./assets/maya-full-body.png";
import { morph } from "./morph";
import { renderDevices, type DeviceRow, type DevicesHandlers, type DevicesModel } from "./devices";
import { terminalLabel } from "./terminalpick";
import { RUNNING_VERSION, renderReleaseNotes } from "./release-notes";
import { DEFAULT_REVIEW_PROMPT } from "./reviewprompt";
import { updates } from "./update";
import { addButton, btn as settingsBtn, card, cardHead, choice, footRow, group, notice, row, screenHead, source, state, toggle } from "./settings-ui";

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
  /** The group's devices (this one first); empty while the network is Off. */
  devices?: DeviceRow[];
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
  /** The Work Items settings; the defaults when absent. */
  workItems?: WorkItemsConfig;
  /** Which Work Items sources are open, by `github`, `gitlab` or a custom tool's id; folded when absent. */
  sourcesOpen?: Record<string, boolean>;
  /** Whether a Coral token is stored. */
  coralHasToken?: boolean;
  /** Coral's statuses, once listed; null until they are. */
  coralStatuses?: CoralStatus[] | null;
  /** What the last Coral Test said. */
  coralTest?: string | null;
  /** Which custom tools have their "Output format" details open, by tool id. */
  customDetailsOpen?: Record<string, boolean>;
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
  /** The device whose Join form is open, and what was typed in it. */
  devicesJoining?: string | null;
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
  /** One service's Work Items settings, read from the page, to save. */
  onWorkItems?(source: GitProvider, items: ForgeItems): void;
  onWorkItems?(source: "coral", items: CoralItems): void;
  onCoralToken?(token: string): void;
  onCoralClearToken?(): void;
  /** Resolves to "Connected as <name>"; rejects with why not. */
  onCoralTest?(): Promise<string>;
  onCoralStatuses?(): Promise<CoralStatus[]>;
  onWorkPrompt?(text: string): void;
  onCustomTools?(tools: CustomTool[]): void;
  /** Shows another Settings group. */
  onOpenGroup?(group: SettingsGroup): void;
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
  onRemoveMember(key: string): void;
  onJoinAsk(key: string): void;
  onJoin(key: string, host: string, port: number): void;
  onJoinCancel(): void;
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
const CORAL_DEFAULT_ENV = "CORAL_TOKEN";
const GIT_LABEL: Record<GitProvider, string> = { github: "GitHub", gitlab: "GitLab" };

/** One GitHub and one GitLab connection, in that order, as the Rust side defaults them. */
export function gitConnections(raw: GitConnectionJson[] | null | undefined): GitConnectionJson[] {
  return (["github", "gitlab"] as const).map((provider) => {
    const found = raw?.find((c) => c.provider === provider);
    return found ? { ...found, baseUrl: found.baseUrl ?? null, clone: found.clone ?? "ssh" } : { provider, enabled: provider === "github", baseUrl: null, token: { kind: "env", name: GIT_DEFAULT_ENV[provider] }, clone: "ssh" };
  });
}

const CUSTOM_TOOL_FORMAT = `{"items": [{"id": "ENG-1204", "title": "Export reports as CSV",
  "url": "https://linear.app/acme/issue/ENG-1204", "status": "Todo",
  "labels": ["frontend"], "assignees": ["tiago"], "repo": "acme/web"}]}`;

const CUSTOM_TOOL_SCRIPT = `#!/bin/sh
# My open tickets as Maya's work items (needs curl and jq).
curl -s -H "Authorization: Bearer $TRACKER_TOKEN" https://tracker.example.com/api/my-tickets |
  jq '{items: [.[] | {id: .key, title: .summary, url: .link, status: .state, repo: .repository}]}'
`;

/** The Work Items settings as the Rust side defaults them: one rule for my issues. */
export function workItemsConfig(raw: Partial<WorkItemsConfig> | null | undefined): WorkItemsConfig {
  const forge = (f?: ForgeItems): ForgeItems => f ?? { enabled: true, rules: [{ assignee: { kind: "me" }, labels: [], repos: [] }], statusLabels: [], startStatus: null, trackerRepos: [] };
  const coral: CoralItems = raw?.coral ?? { enabled: false, baseUrl: "", token: { kind: "stored" }, rules: [{ assignee: { kind: "me" }, labels: [], repos: [] }], startStatus: null };
  return { github: forge(raw?.github), gitlab: forge(raw?.gitlab), coral, prompt: raw?.prompt ?? "", custom: raw?.custom ?? [] };
}

/** "a", "a or b", "a, b or c". */
function orList(items: string[]): string {
  return items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} or ${items[items.length - 1]}`;
}

/** A rule in a line, for its box's head: "Assigned to me, labelled bug, in dosaki/maya". */
export function ruleSummary(rule: ItemRule): string {
  const a = rule.assignee;
  let text = a.kind === "me" ? "Assigned to me" : a.kind === "anyone" ? "Anyone's issues" : `Assigned to ${a.login || "someone"}`;
  if (rule.labels.length > 0) text += `, labelled ${orList(rule.labels)}`;
  if (rule.repos.length > 0) text += `, in ${orList(rule.repos)}`;
  return text;
}

/** A forge's settings in a line, for its folded head: "2 rules; Start sets status to in progress". */
export function forgeSummary(items: ForgeItems): string {
  const n = items.rules.length;
  const rules = `${n} rule${n === 1 ? "" : "s"}`;
  return items.startStatus ? `${rules}; Start sets status to ${items.startStatus}` : rules;
}

/** Coral's settings in a line, for its folded head: "2 rules; Start sets a status". */
export function coralSummary(items: CoralItems): string {
  if (!items.enabled) return "Off";
  const n = items.rules.length;
  const rules = `${n} rule${n === 1 ? "" : "s"}`;
  return items.startStatus ? `${rules}; Start sets a status` : rules;
}

export const SOURCES_OPEN_KEY = "maya.settingsSources";

/** The Work Items sources open last time on this device; none when unset or unreadable. */
export function storedSourcesOpen(): Record<string, boolean> {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(SOURCES_OPEN_KEY) ?? "{}");
    if (typeof v !== "object" || v === null || Array.isArray(v)) return {};
    return Object.fromEntries(Object.entries(v).filter(([, open]) => typeof open === "boolean"));
  } catch {
    return {};
  }
}

/** Records which sources are open, in the model and on this device. */
function setSourcesOpen(model: SettingsModel, open: Record<string, boolean>): void {
  model.sourcesOpen = { ...model.sourcesOpen, ...open };
  try {
    localStorage.setItem(SOURCES_OPEN_KEY, JSON.stringify(model.sourcesOpen));
  } catch {
    /* private mode: nothing to remember */
  }
}

const WORK_ITEMS_HINT: Record<GitProvider, string> = {
  github: "Start assigns the issue and sets its status: the token needs Issues: read and write (fine-grained) or repo (classic).",
  gitlab: "Start assigns the issue and sets its status: the token needs the api scope.",
};

/** "a, b,,c" as ["a", "b", "c"]. */
function commaList(text: string): string[] {
  return text.split(",").map((t) => t.trim()).filter((t) => t !== "");
}

interface ConfigJson {
  workItems?: WorkItemsConfig;
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
  ["workItems", "Work Items"],
  ["notifications", "Notifications"],
  ["voice", "Voice"],
  ["network", "Network"],
  ["devices", "Devices"],
  ["debug", "Debug"],
  ["releaseNotes", "Release Notes"],
] as const;
export type SettingsGroup = (typeof SETTINGS_GROUPS)[number][0];

/** The sidebar's divided clusters, top to bottom. */
export const SETTINGS_NAV: readonly (readonly SettingsGroup[])[] = [
  ["general", "keyboard"],
  ["sessions", "git", "reviews", "workItems"],
  ["notifications", "voice"],
  ["network", "devices"],
  ["debug", "releaseNotes"],
];

/** What each screen's head says under its title. */
export const SETTINGS_LEDES: Record<SettingsGroup, string> = {
  general: "The agent behind Maya, and how she looks.",
  keyboard: "The keys for the Chat tab, a session's reply box and the new-session prompt.",
  sessions: "How Maya follows your sessions, and where new ones start.",
  git: "The accounts Maya reads pull requests and merge requests with. Reviews clone with your own git credentials.",
  reviews: "Where reviews are checked out, what the Review button asks the agent, and when Maya tells you about your own pull requests.",
  workItems: "Where the Work Items tab finds your work, and what Start does with an item.",
  notifications: "How Maya tells you a session needs you.",
  voice: "How Maya speaks to you, and how she hears you.",
  network: "Join your other computers and your phone to this Maya.",
  devices: "The computers and phones joined to this Maya.",
  debug: "For looking under the hood.",
  releaseNotes: "What changed in each version of Maya.",
};

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
  s.append(screenHead(title, SETTINGS_LEDES[key]));
  return s;
}

/**
 * Settings › Keyboard: each action's combinations as chips, an Add button
 * that records the next Enter press, and Reset to defaults.
 */
function renderKeyboard(section: HTMLElement, keys: KeyBindings, h: SettingsHandlers): void {
  const rows: HTMLElement[] = [];
  for (const [action, name] of KEY_ACTIONS) {
    const chips: Node[] = [];
    for (const combo of keys[action]) {
      const chip = document.createElement("span");
      chip.className = "settings__kbd";
      chip.append(comboLabel(combo));
      if (action !== "send" || keys.send.length > 1) {
        const remove = document.createElement("button");
        remove.type = "button";
        remove.setAttribute("aria-label", `Remove ${comboLabel(combo)}`);
        remove.textContent = "×";
        remove.addEventListener("click", () => h.onKeys?.(removeCombo(keys, action, combo)));
        chip.append(remove);
      } else {
        chip.classList.add("settings__kbd--only");
      }
      chips.push(chip);
    }
    const add = settingsBtn("+ Add", { action: "keys-add", kind: "quiet" });
    const error = document.createElement("span");
    error.className = "settings__error";
    let recording = false;
    const stop = () => {
      recording = false;
      add.textContent = "+ Add";
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
    const r = row({ label: name, control: [...chips, add, error] });
    r.classList.add("settings__keys");
    r.dataset.keys = action;
    r.querySelector(".settings__label")!.classList.add("settings__keys-name");
    rows.push(r);
  }
  section.append(
    group({ foot: "An Enter press in neither list does what the box does by itself." }, card(...rows)),
    settingsBtn("Reset to defaults", { action: "keys-reset" }, () => h.onKeys?.(defaultKeys())),
  );
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
  /** The Work Items source it is in: both forges name their fields alike. */
  source?: string;
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
  const source = el.closest<HTMLElement>("[data-source]")?.dataset.source;
  if (source !== undefined) focused.source = source;
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
  const scope = (f.source !== undefined && [...root.querySelectorAll<HTMLElement>("[data-source]")].find((s) => s.dataset.source === f.source)) || root;
  const named = [...scope.querySelectorAll("input, select, textarea")].filter((e): e is Field => isField(e) && e.name === f.name);
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
 * Refreshes the code's expiry line in place, or once it expired puts the
 * empty state ("No code yet") in the code's place and sets the button back
 * to "Show pairing code".
 */
export function tickPairingCode(panel: ParentNode, model: SettingsModel, nowMs: number): void {
  const code = model.network?.code;
  const box = panel.querySelector<HTMLElement>(".settings__code");
  const expiry = panel.querySelector<HTMLElement>(".settings__code-expiry");
  if (!code || !box) return;
  if (nowMs > code.expiresAt) {
    box.classList.add("settings__code--empty");
    box.textContent = "No code yet";
    expiry?.remove();
    panel.querySelector(".settings__pair-title")?.remove();
    const regen = panel.querySelector<HTMLButtonElement>("button[data-action=regenerate-code]");
    if (regen) regen.textContent = "Show pairing code";
  } else if (expiry) {
    expiry.textContent = `expires in ${expiresInMinutes(code.expiresAt, nowMs)} min`;
  }
}

function addressesText(addresses: string[]): string {
  return `This machine's addresses: ${addresses.join(", ")}`;
}

/**
 * Puts a fresh address list on the page in place, since no event follows
 * a Wi‑Fi, VPN or sleep/wake change. A repaint would wipe unsaved edits in
 * the other fields. The line is the Port row's help. Returns whether
 * anything changed.
 */
export function tickAddresses(panel: ParentNode, model: SettingsModel, addresses: string[]): boolean {
  const before = model.network?.addresses ?? [];
  if (before.length === addresses.length && before.every((a, i) => a === addresses[i])) return false;
  if (model.network) model.network.addresses = addresses;
  const line = panel.querySelector<HTMLElement>(".settings__addresses");
  const port = panel.querySelector<HTMLElement>("input[name=networkPort]");
  if (addresses.length === 0) {
    line?.remove();
    port?.removeAttribute("aria-describedby");
  } else if (line) {
    line.textContent = addressesText(addresses);
  } else {
    const text = port?.closest(".settings__row")?.querySelector(".settings__row-text");
    if (port && text) {
      const help = document.createElement("span");
      help.className = "settings__help settings__addresses";
      help.id = "settings-help-networkPort";
      help.textContent = addressesText(addresses);
      port.setAttribute("aria-describedby", help.id);
      text.append(help);
    }
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

/** The places a new session can open on this platform, as [value, label]. Settings shows only on a desktop: neither Windows nor Linux is a Mac. */
export function terminalChoices(): [NewSessionTerminal, string][] {
  const platform = isWindows() ? "windows" : isLinux() ? "linux" : "macos";
  const values: NewSessionTerminal[] = platform === "macos" ? ["terminal", "iterm", "native"] : ["terminal", "native"];
  return values.map((v) => [v, terminalLabel(v, platform)]);
}

/**
 * Whether Coral may have a token to ask with: a stored one, or a variable
 * (which may be unset: a failed ask just leaves the help text).
 */
function coralCanAsk(hasToken: boolean | undefined, cfg: CoralItems): boolean {
  return cfg.token.kind === "env" || hasToken === true;
}

/** The Coral card in the Sources group: address, token, rules and what Start sets. */
function coralSource(
  model: SettingsModel,
  h: SettingsHandlers,
  cfg: CoralItems,
  nextRuleId: () => number,
  remember: (key: string, open: boolean) => void,
): HTMLElement {
  const hasToken = model.coralHasToken === true;
  let tokenKind = cfg.token.kind;
  // A Test result says who a token was at an address: another of either makes it stale.
  const forgetTest = () => {
    model.coralTest = null;
  };

  const on = toggle("wi-coral-enabled", cfg.enabled, () => save(), "Use Coral");

  const url = document.createElement("input");
  url.type = "text";
  url.className = "mono";
  url.name = "wi-coral-baseUrl";
  url.placeholder = "https://coral.example.com";
  url.value = cfg.baseUrl;

  const env = document.createElement("input");
  env.type = "text";
  env.className = "mono";
  env.name = "wi-coral-envName";
  env.placeholder = CORAL_DEFAULT_ENV;
  env.value = cfg.token.kind === "env" ? cfg.token.name : "";

  const start = document.createElement("select");
  start.name = "wi-coral-startStatus";
  const fillStart = (statuses: CoralStatus[] | null, chosen: string | null) => {
    start.replaceChildren();
    const none = document.createElement("option");
    none.value = "";
    none.textContent = "Leave as it is";
    start.append(none);
    for (const st of statuses ?? []) {
      const o = document.createElement("option");
      o.value = st.id;
      o.textContent = st.name;
      start.append(o);
    }
    // A saved status that is not in the list (yet) stays, so a save does not drop it.
    if (chosen !== null && !(statuses ?? []).some((st) => st.id === chosen)) {
      const o = document.createElement("option");
      o.value = chosen;
      o.textContent = "Saved status";
      start.append(o);
    }
    start.value = chosen ?? "";
  };
  fillStart(model.coralStatuses ?? null, cfg.startStatus);
  const loadStatuses = () => {
    const chosen = start.value === "" ? null : start.value;
    h.onCoralStatuses?.().then((list) => fillStart(list, chosen), () => { /* the help already says how to list them */ });
  };

  const editor = rulesEditor({
    idPrefix: "wi-coral-rule",
    nextId: nextRuleId,
    rules: cfg.rules,
    words: CORAL_RULE_WORDS,
    onChange: () => save(),
    onAdd: () => {
      const items = read();
      items.rules.push({ assignee: { kind: "me" }, labels: [], repos: [] });
      save(items);
    },
  });

  /** The settings as the page shows them. */
  const read = (): CoralItems => ({
    enabled: on.checked,
    baseUrl: url.value.trim(),
    token: tokenKind === "env" ? { kind: "env", name: env.value.trim() || CORAL_DEFAULT_ENV } : { kind: "stored" },
    rules: editor.read(),
    startStatus: start.value === "" ? null : start.value,
  });
  // A rule for someone with no email yet would make the backend refuse the
  // whole box: nothing is saved until it is typed (or the rule goes).
  const save = (items: CoralItems = read()) => {
    if (items.rules.some((r) => r.assignee.kind === "user" && r.assignee.login === "")) return;
    if (items.baseUrl !== cfg.baseUrl || JSON.stringify(items.token) !== JSON.stringify(cfg.token)) forgetTest();
    h.onWorkItems?.("coral", items);
  };
  url.addEventListener("change", () => {
    url.value = url.value.trim();
    save();
  });
  env.addEventListener("change", () => save());
  start.addEventListener("change", () => save());

  const kind = choice("wi-coral-tokenKind", [["stored", "Paste a token"], ["env", "From variable"]] as const, tokenKind, (k) => {
    tokenKind = k;
    save();
  });

  const controls: HTMLElement[] = [];
  if (tokenKind === "stored") {
    const tok = document.createElement("input");
    tok.type = "password";
    tok.name = "wi-coral-token";
    tok.autocomplete = "off";
    tok.placeholder = hasToken ? "paste to replace" : "paste your token";
    tok.addEventListener("change", () => {
      const t = tok.value.trim();
      tok.value = "";
      if (!t) return;
      forgetTest();
      h.onCoralToken?.(t);
    });
    controls.push(tok);
    if (hasToken)
      controls.push(
        settingsBtn("Clear", { action: "coral-clear", kind: "quiet" }, () => {
          forgetTest();
          h.onCoralClearToken?.();
        }),
      );
  } else {
    controls.push(env);
  }
  const resting = tokenKind === "env" ? "Maya reads the token from this variable." : hasToken ? `Saved in ${secretStore()}.` : "Paste a token from your Coral account.";
  const tokenRow = row({ label: tokenKind === "env" ? "Variable" : "Coral token", help: model.coralTest ?? resting, control: controls });
  const result = tokenRow.querySelector<HTMLElement>(".settings__help")!;
  const test = settingsBtn("Test", { action: "coral-test" }, () => {
    const running = h.onCoralTest?.();
    if (!running) return;
    result.textContent = "Testing…";
    running.then(
      (text) => {
        result.textContent = text;
        loadStatuses();
      },
      (e) => { result.textContent = String(e); },
    );
  });
  tokenRow.querySelector(".settings__control")!.append(test);

  const startHelp = (model.coralStatuses ? "" : "Test the connection to list Coral's statuses. ") + "A defect gets the first allowed status of the same kind when this one is not allowed for defects.";

  const hint = document.createElement("span");
  hint.className = "settings__help";
  hint.textContent = "The token acts as you in Coral: Start assigns the item to you and sets its status.";
  const box = source({
    key: "coral",
    name: "Coral",
    summary: coralSummary(cfg),
    stateText: cfg.enabled ? "On" : "Off",
    stateOn: cfg.enabled,
    open: (model.sourcesOpen ?? {}).coral === true,
    onToggle: (open) => {
      remember("coral", open);
      if (open && coralCanAsk(model.coralHasToken, cfg) && model.coralStatuses == null) loadStatuses();
    },
    control: on,
    body: [
      row({ label: "Address", help: "Where your Coral is, starting with https://.", control: url }),
      row({ label: "Token", control: kind, tag: "div" }),
      tokenRow,
      row({ label: "Work items to pick up", help: "A work item shows when it matches any one of these rules.", control: editor.box, stack: true }),
      row({ label: "On Start, set status to", help: startHelp, control: start }),
      footRow(hint),
    ],
  });
  box.classList.add("settings__git", "settings__workitems");
  box.dataset.forge = "coral";
  return box;
}

/** The words a rules editor uses for what a rule can match on, and the hint in its someone field. */
interface RuleWords {
  labels: string;
  labelsPlaceholder: string;
  repos: string;
  reposPlaceholder: string;
  loginPlaceholder: string;
  loginLabel: string;
}

const FORGE_RULE_WORDS: RuleWords = {
  labels: "Labels",
  labelsPlaceholder: "bug, urgent",
  repos: "Repositories",
  reposPlaceholder: "any repository",
  loginPlaceholder: "login",
  loginLabel: "Login",
};

const CORAL_RULE_WORDS: RuleWords = {
  labels: "Tags",
  labelsPlaceholder: "team: api, urgent",
  repos: "Products",
  reposPlaceholder: "any product",
  loginPlaceholder: "their email",
  loginLabel: "Email",
};

/**
 * The boxes of rules a source picks up work items by, with "+ Add rule" last.
 * `onChange` runs when a field changes (a rule for someone with no name yet
 * is the caller's to hold back); `onAdd` when the add button is pressed.
 */
function rulesEditor(opts: {
  idPrefix: string;
  nextId: () => number;
  rules: ItemRule[];
  words: RuleWords;
  onChange: () => void;
  onAdd: () => void;
}): { box: HTMLElement; read: () => ItemRule[] } {
  const { words } = opts;
  const box = document.createElement("div");
  box.className = "settings__rules";
  const add = addButton("+ Add rule", "wi-add-rule", opts.onAdd);

  /** One rule as its box shows it; someone with no login yet reads as such. */
  const readRule = (r: HTMLElement): ItemRule => {
    const kind = r.querySelector<HTMLSelectElement>("select[name=wi-assignee]")!.value;
    const login = r.querySelector<HTMLInputElement>("input[name=wi-login]")!.value.trim();
    return {
      assignee: kind === "user" ? { kind: "user", login } : kind === "anyone" ? { kind: "anyone" } : { kind: "me" },
      labels: commaList(r.querySelector<HTMLInputElement>("input[name=wi-labels]")!.value),
      repos: commaList(r.querySelector<HTMLInputElement>("input[name=wi-repos]")!.value),
    };
  };

  const addRule = (rule: ItemRule) => {
    const id = `${opts.idPrefix}-${opts.nextId()}`;
    const r = document.createElement("div");
    r.className = "settings__rule";
    const fieldLabel = (text: string, forId: string) => {
      const l = document.createElement("label");
      l.htmlFor = forId;
      l.textContent = text;
      return l;
    };
    const assignee = document.createElement("select");
    assignee.name = "wi-assignee";
    assignee.id = `${id}-assignee`;
    for (const [value, text] of [["me", "Me"], ["user", "Someone"], ["anyone", "Anyone"]]) {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = text;
      assignee.append(o);
    }
    assignee.value = rule.assignee.kind;
    const login = document.createElement("input");
    login.type = "text";
    login.name = "wi-login";
    login.placeholder = words.loginPlaceholder;
    login.setAttribute("aria-label", words.loginLabel);
    login.value = rule.assignee.kind === "user" ? rule.assignee.login : "";
    login.hidden = rule.assignee.kind !== "user";
    // Someone with no login yet cannot be saved: wait for the login.
    assignee.addEventListener("change", () => {
      login.hidden = assignee.value !== "user";
      if (assignee.value !== "user") opts.onChange();
    });
    login.addEventListener("change", () => opts.onChange());
    const labels = document.createElement("input");
    labels.type = "text";
    labels.name = "wi-labels";
    labels.id = `${id}-labels`;
    labels.placeholder = words.labelsPlaceholder;
    labels.value = rule.labels.join(", ");
    labels.addEventListener("change", () => opts.onChange());
    const repos = document.createElement("input");
    repos.type = "text";
    repos.name = "wi-repos";
    repos.id = `${id}-repos`;
    repos.className = "mono";
    repos.placeholder = words.reposPlaceholder;
    repos.value = rule.repos.join(", ");
    repos.addEventListener("change", () => opts.onChange());
    const remove = settingsBtn("Remove", { action: "wi-remove-rule", kind: "quiet" }, () => {
      r.remove();
      opts.onChange();
    });
    remove.classList.add("settings__btn--danger");
    remove.setAttribute("aria-label", "Remove rule");

    const head = document.createElement("div");
    head.className = "settings__rule-head";
    const sum = document.createElement("span");
    sum.className = "settings__rule-sum";
    sum.textContent = ruleSummary(rule);
    head.append(sum, remove);
    // The head follows the fields as they are typed, before any save.
    const sumUp = () => { sum.textContent = ruleSummary(readRule(r)); };
    r.addEventListener("input", sumUp);
    r.addEventListener("change", sumUp);

    const who = document.createElement("div");
    who.className = "settings__rule-who";
    who.append(assignee, login);
    const body = document.createElement("div");
    body.className = "settings__rule-body";
    body.append(
      fieldLabel("Assignee", assignee.id), who,
      fieldLabel(words.labels, labels.id), labels,
      fieldLabel(words.repos, repos.id), repos,
    );
    r.append(head, body);
    box.insertBefore(r, add);
  };
  box.append(add);
  for (const rule of opts.rules) addRule(rule);

  return { box, read: () => [...box.querySelectorAll<HTMLElement>(".settings__rule")].map(readRule) };
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
  const titles = new Map<SettingsGroup, string>(SETTINGS_GROUPS);
  const navButtons = new Map<SettingsGroup, HTMLButtonElement>();
  for (const key of SETTINGS_NAV.flat()) {
    const title = titles.get(key)!;
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
    navButtons.set(key, b);
  }
  // The live sections, found when the nav is used: a repaint keeps the old
  // Devices section and drops this render's, so nothing may close over it.
  const show = (group: SettingsGroup) => {
    for (const s of nav.parentElement?.querySelectorAll<HTMLElement>(".settings__section") ?? []) s.hidden = s.dataset.group !== group;
    for (const b of navButtons.values()) {
      if (b.dataset.group === group) b.setAttribute("aria-current", "true");
      else b.removeAttribute("aria-current");
    }
  };
  for (const keys of SETTINGS_NAV) {
    const g = document.createElement("div");
    g.className = "settings__nav-group";
    g.append(...keys.map((k) => navButtons.get(k)!));
    nav.append(g);
  }
  content.append(...Object.values(sections));
  show(model.settingsGroup ?? "general");
  const { general, keyboard, sessions, git, reviews, workItems, notifications, voice, network, devices, debug, releaseNotes } = sections;
  const agents = model.agents.length > 0 ? model.agents : [CLAUDE_AGENT];
  // An agent that is chosen but not listed (uninstalled, or the listing has
  // not landed yet) stays selected, with only its Default model, rather than
  // the select silently showing another agent.
  const chosenAgent: AgentInfo = agents.find((a) => a.harness === model.agent) ?? (model.agent ? { harness: model.agent, models: [], efforts: [], modes: [] } : agents[0]);
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
  const agentHelp = "Interprets your voice commands, picks folders for 'Let Maya choose', and is the default for new and resumed sessions and reviews.";

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

  renderKeyboard(keyboard, model.keys ?? defaultKeys(), h);

  const agentRow = row({ label: "Agent", help: agentHelp, control: agentSel });
  agentRow.querySelector(".settings__help")!.setAttribute("data-for", "agent");
  const updateRows: HTMLElement[] = [];
  if (model.version) {
    const checking = model.updatesOn && h.onCheckUpdates;
    const versionRow = row({
      label: `Maya ${model.version}`,
      help: checking ? model.updateStatus : undefined,
      control: checking ? settingsBtn("Check for updates", { action: "check-updates" }, () => h.onCheckUpdates?.()) : undefined,
    });
    versionRow.querySelector(".settings__label")!.classList.add("settings__version");
    versionRow.querySelector(".settings__help")?.classList.add("settings__update-status");
    updateRows.push(versionRow);
  }
  general.append(
    group({ title: "Agent" }, card(agentRow, row({ label: "Model", control: agentModelSel }))),
    group({ title: "Appearance" }, card(row({ label: "Theme", control: themeSel }))),
    ...(updateRows.length > 0 ? [group({ title: "Updates" }, card(...updateRows))] : []),
  );

  const net = model.network ?? DEFAULT_NETWORK_STATUS;
  const netRole = model.networkRole ?? net.role;
  const netPort = model.networkPort ?? 0;
  const netMainHost = model.networkMainHost ?? "";
  const netMainPort = model.networkMainPort ?? 0;
  const netName = model.networkName ?? "";
  const netCode = model.networkCode ?? "";

  // A hook row: the status sentence is the help, the state and button are the
  // control. The button's full text stays as its accessible name.
  const hookRow = (
    label: string,
    help: string,
    installed: boolean | null,
    button: { action: string; text: string; disabled: boolean; onClick: () => void },
  ) => {
    const b = settingsBtn(installed ? "Remove" : "Install", { action: button.action, kind: installed ? "quiet" : "plain", disabled: button.disabled }, button.onClick);
    b.setAttribute("aria-label", button.text);
    return row({
      label,
      help,
      control: [...(installed === null ? [] : [state(installed ? "Installed" : "Not installed", installed ? "on" : "off")]), b],
    });
  };

  const hookRowEl = hookRow(
    "Claude Code hook",
    model.hookInstalled === null
      ? "Checking Claude Code hook…"
      : model.hookInstalled
        ? "Claude Code hook is installed. Awaiting Decision and Completed are precise."
        : "Claude Code hook is not installed. Permission prompts will show as Working.",
    model.hookInstalled,
    model.hookInstalled
      ? { action: "remove", text: "Remove Claude hook", disabled: false, onClick: () => h.onRemove() }
      : { action: "install", text: "Install Claude hook", disabled: model.hookInstalled === null, onClick: () => h.onInstall() },
  );

  const line = model.statusline ?? null;
  const lineRowEl = hookRow(
    "Usage status line",
    line === null
      ? "Checking status line…"
      : line.blockedBy
        ? `Usage and limits cannot be recorded: the status line is set in ${line.blockedBy}.`
        : line.installed
          ? "Usage and limits are recorded from Claude Code's status line."
          : "Usage and limits are not recorded. Install the status line wrapper to see them in session windows.",
    line === null ? null : line.installed,
    {
      action: line?.installed ? "remove-statusline" : "install-statusline",
      text: line?.installed ? "Remove usage status line" : "Install usage status line",
      disabled: line === null || !!line.blockedBy,
      onClick: () => line?.installed ? h.onStatuslineRemove?.() : h.onStatuslineInstall?.(),
    },
  );

  const codexRowEl = hookRow(
    "Codex hook",
    model.codexHookInstalled == null
      ? "Checking Codex hook…"
      : model.codexHookInstalled
        ? "Codex hook is installed. Restart Codex and use /hooks to review and trust it for session updates."
        : "Codex hook is not installed. Session state is inferred from transcripts.",
    model.codexHookInstalled ?? null,
    {
      action: model.codexHookInstalled ? "remove-codex" : "install-codex",
      text: model.codexHookInstalled ? "Remove Codex hook" : "Install Codex hook",
      disabled: model.codexHookInstalled == null,
      onClick: () => model.codexHookInstalled ? h.onCodexRemove?.() : h.onCodexInstall?.(),
    },
  );

  const input = document.createElement("input");
  input.type = "number";
  input.name = "timeout";
  input.min = "1";
  input.value = String(model.completedTimeoutMinutes);
  input.addEventListener("change", () => {
    const n = Number(input.value);
    if (Number.isFinite(n) && n >= 1) h.onTimeout(Math.floor(n));
  });
  const minutes = document.createElement("span");
  minutes.textContent = "minutes";

  const dirInput = document.createElement("input");
  dirInput.type = "text";
  dirInput.name = "projectsDir";
  dirInput.className = "mono";
  dirInput.placeholder = "~/dev";
  dirInput.value = model.projectsDir;
  dirInput.addEventListener("change", () => h.onProjectsDir(dirInput.value.trim()));

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

  sessions.append(
    group(
      { title: "Session tracking", note: "Hooks let the agents tell Maya what they are doing, rather than her guessing from transcripts." },
      card(hookRowEl, lineRowEl, codexRowEl),
    ),
    group({ title: "Board" }, card(
      row({ label: "Completed decays to Idle after", control: [input, minutes] }),
      row({ label: "Session viewer", control: viewerSel }),
    )),
    group({ title: "New sessions" }, card(
      row({ label: "Projects directory", control: dirInput }),
      row({ label: "Open in", control: terminalSel }),
    )),
  );

  const clonesInput = document.createElement("input");
  clonesInput.type = "text";
  clonesInput.name = "clonesDir";
  clonesInput.className = "mono";
  clonesInput.placeholder = "~/dev/reviews";
  clonesInput.value = model.clonesDir;
  clonesInput.addEventListener("change", () => h.onClonesDir(clonesInput.value.trim()));

  const reviewInput = document.createElement("textarea");
  reviewInput.name = "reviewPrompt";
  reviewInput.rows = 7;
  reviewInput.placeholder = DEFAULT_REVIEW_PROMPT;
  reviewInput.value = model.reviewPrompt;
  reviewInput.addEventListener("change", () => h.onReviewPrompt(reviewInput.value.trim()));

  reviews.append(
    group({ title: "Reviews" }, card(
      row({ label: "Clones directory", help: "For reviews and worktrees.", control: clonesInput }),
      row({
        label: "Review prompt",
        help: "Blank uses the built-in prompt. {number}, {repo} and {url} are filled in; a prompt that uses none gets \" PR #<number> (<url>)\" appended.",
        control: reviewInput,
        stack: true,
      }),
    )),
    group({ title: "Your pull requests" }, card(row({
      label: "Notify me when a PR assigned to me is approved or needs changes",
      control: toggle("notifyPrReview", model.notifyOnPrReview !== false, (on) => h.onNotifyPrReview?.(on)),
    }))),
  );

  debug.append(
    group({}, card(row({
      label: "Debug mode",
      help: "Shows the Debug tab and Maya's own card.",
      control: toggle("debug-mode", model.debugMode === true, (on) => h.onDebugMode?.(on)),
    }))),
  );

  const notes = renderReleaseNotes();
  for (const heading of notes.querySelectorAll(".release-notes__version")) {
    if (heading.textContent === `Maya ${RUNNING_VERSION}`) {
      const pill = document.createElement("span");
      pill.className = "settings__running";
      pill.textContent = "Running";
      heading.append(pill);
    }
  }
  const rn = document.createElement("div");
  rn.className = "settings__rn";
  const img = document.createElement("img");
  img.className = "settings__rn-art";
  img.alt = "";
  img.src = art;
  rn.append(img, notes);
  releaseNotes.append(rn);

  const gitCards: HTMLElement[] = [];
  for (const conn of model.git ?? gitConnections(null)) {
    const p = conn.provider;
    const status = model.gitStatus?.[p] ?? { hasToken: false };
    const off = !conn.enabled;
    const rows: HTMLElement[] = [
      cardHead({
        label: GIT_LABEL[p],
        control: toggle(`gitEnabled-${p}`, conn.enabled, (on) => h.onGitEnabled?.(p, on), `Use ${GIT_LABEL[p]}`),
      }),
    ];

    if (p === "gitlab") {
      const url = document.createElement("input");
      url.type = "text";
      url.className = "mono";
      url.name = "gitBaseUrl-gitlab";
      url.placeholder = "https://gitlab.com";
      url.value = conn.baseUrl ?? "";
      url.disabled = off;
      url.addEventListener("change", () => h.onGitBaseUrl?.(url.value.trim()));
      rows.push(row({ label: "URL", control: url }));
    }

    rows.push(row({
      label: "Token",
      help: p === "github" ? "Needs the repo scope." : "Needs read_api.",
      control: choice(`gitTokenKind-${p}`, [["stored", "Paste a token"], ["env", "From env var"]] as const, conn.token.kind, (kind) => h.onGitTokenKind?.(p, kind), off),
      tag: "div",
    }));

    if (conn.token.kind === "stored") {
      const tok = document.createElement("input");
      tok.type = "password";
      tok.name = `gitToken-${p}`;
      tok.autocomplete = "off";
      tok.disabled = off;
      tok.placeholder = status.hasToken ? "paste to replace" : "paste your token";
      tok.addEventListener("change", () => {
        const t = tok.value.trim();
        tok.value = "";
        if (t) h.onGitToken?.(p, t);
      });
      const controls: HTMLElement[] = [tok];
      if (status.hasToken) controls.push(settingsBtn("Clear", { action: "git-clear", kind: "quiet" }, () => h.onGitClearToken?.(p)));
      rows.push(row({ label: `${GIT_LABEL[p]} token`, help: status.hasToken ? `Saved in ${secretStore()}.` : null, control: controls }));
    } else {
      const env = document.createElement("input");
      env.type = "text";
      env.className = "mono";
      env.name = `gitEnvName-${p}`;
      env.placeholder = GIT_DEFAULT_ENV[p];
      env.value = conn.token.name;
      env.disabled = off;
      env.addEventListener("change", () => h.onGitEnvName?.(p, env.value.trim()));
      rows.push(row({ label: "Variable", control: env }));
    }

    rows.push(row({
      label: "Clone with",
      control: choice(`gitClone-${p}`, [["ssh", "SSH"], ["https", "HTTPS"]] as const, conn.clone ?? "ssh", (protocol) => h.onGitClone?.(p, protocol), off),
      tag: "div",
    }));

    let outcome: Node = document.createTextNode("");
    if (status.test) {
      const out = document.createElement("span");
      out.className = "settings__git-test";
      out.textContent = status.test;
      outcome = out;
    }
    rows.push(footRow(outcome, settingsBtn("Test", { action: "git-test", disabled: off }, () => h.onGitTest?.(p))));

    const c = card(...rows);
    c.classList.add("settings__git");
    if (off) c.classList.add("settings__card--off");
    c.dataset.provider = p;
    gitCards.push(c);
  }
  git.append(group({}, ...gitCards));

  const workItemsConfigNow = model.workItems ?? workItemsConfig(null);
  const connections = model.git ?? gitConnections(null);
  const sourcesOpen = model.sourcesOpen ?? {};
  // Opening or folding a source changes the DOM in place (see `source`); the
  // model and this device remember it, so a repaint draws it the same way.
  const rememberSource = (key: string, open: boolean) => setSourcesOpen(model, { [key]: open });
  const sources: HTMLElement[] = [];
  let ruleIds = 0;
  for (const forge of ["github", "gitlab"] as const) {
    const cfg = workItemsConfigNow[forge];
    const label = GIT_LABEL[forge];
    const gitOn = connections.find((c) => c.provider === forge)?.enabled === true;
    const gitOffText = `${label} is off in Settings → Git.`;
    const on = toggle(`wi-${forge}-enabled`, cfg.enabled, () => save(), `Use ${label}`);

    const status = document.createElement("input");
    status.type = "text";
    status.name = "wi-statusLabels";
    status.value = cfg.statusLabels.join(", ");

    const start = document.createElement("select");
    start.name = "wi-startStatus";
    const fillStart = (labels: string[], chosen: string | null) => {
      start.replaceChildren();
      const none = document.createElement("option");
      none.value = "";
      none.textContent = "Leave as it is";
      start.append(none);
      for (const l of labels) {
        const o = document.createElement("option");
        o.value = l;
        o.textContent = l;
        start.append(o);
      }
      start.value = chosen !== null && labels.includes(chosen) ? chosen : "";
    };
    fillStart(cfg.statusLabels, cfg.startStatus);

    const tracker = document.createElement("input");
    tracker.type = "text";
    tracker.name = "wi-trackerRepos";
    tracker.className = "mono";
    tracker.value = cfg.trackerRepos.join(", ");

    /** The settings as the page shows them. */
    const read = (): ForgeItems => ({
      enabled: on.checked,
      rules: editor.read(),
      statusLabels: commaList(status.value),
      startStatus: start.value === "" ? null : start.value,
      trackerRepos: commaList(tracker.value),
    });
    // A rule for someone with no login yet would make the backend refuse the
    // whole box: nothing is saved until the login is typed (or the rule goes).
    const save = (items: ForgeItems = read()) => {
      if (items.rules.some((r) => r.assignee.kind === "user" && r.assignee.login === "")) return;
      h.onWorkItems?.(forge, items);
    };

    const editor = rulesEditor({
      idPrefix: `wi-${forge}-rule`,
      nextId: () => ruleIds++,
      rules: cfg.rules,
      words: FORGE_RULE_WORDS,
      onChange: () => save(),
      onAdd: () => {
        const items = read();
        items.rules.push({ assignee: { kind: "me" }, labels: [], repos: [] });
        save(items);
      },
    });
    const rulesBox = editor.box;

    status.addEventListener("change", () => {
      // The On Start choices follow the labels just typed.
      fillStart(commaList(status.value), start.value);
      save();
    });
    start.addEventListener("change", () => save());
    tracker.addEventListener("change", () => save());

    const body: HTMLElement[] = [];
    if (!gitOn) {
      const open = document.createElement("button");
      open.type = "button";
      open.className = "settings__link";
      open.dataset.action = "wi-open-git";
      open.textContent = "Open Git settings";
      open.addEventListener("click", () => h.onOpenGroup?.("git"));
      body.push(row({ label: gitOffText, control: open }));
    }
    const hint = document.createElement("span");
    hint.className = "settings__help";
    hint.textContent = WORK_ITEMS_HINT[forge];
    body.push(
      row({ label: "Work items to pick up", help: "A work item shows when it matches any one of these rules.", control: rulesBox, stack: true }),
      row({ label: "Status labels", help: "In order, comma-separated.", control: status, idScope: forge }),
      row({ label: "On Start, set status to", control: start }),
      row({ label: "Tracker-only repositories", help: "Comma-separated.", control: tracker, idScope: forge }),
      footRow(hint),
    );
    if (!gitOn) {
      for (const part of body) {
        for (const el of part.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLButtonElement>("input, select, button")) {
          if (el.dataset.action !== "wi-open-git") el.disabled = true;
        }
      }
    }

    const box = source({
      key: forge,
      name: label,
      summary: gitOn ? forgeSummary(cfg) : gitOffText,
      stateText: !gitOn ? "Git off" : cfg.enabled ? "On" : "Off",
      stateOn: gitOn && cfg.enabled,
      open: sourcesOpen[forge] === true,
      onToggle: (open) => rememberSource(forge, open),
      control: on,
      body,
    });
    box.classList.add("settings__git", "settings__workitems");
    box.dataset.forge = forge;
    sources.push(box);
  }

  sources.push(coralSource(model, h, workItemsConfigNow.coral, () => ruleIds++, rememberSource));

  const customTools = workItemsConfigNow.custom ?? [];
  /** The custom tools as the page shows them now, not as they were when it was drawn. */
  const readCustom = (): CustomTool[] =>
    [...workItems.querySelectorAll<HTMLElement>(".settings__custom")].map((b) => {
      const v = (suffix: string) => (b.querySelector<HTMLInputElement>(`input[name$="-${suffix}"]`)?.value ?? "").trim();
      return {
        id: b.dataset.tool ?? "",
        name: v("name"),
        enabled: b.querySelector<HTMLInputElement>('input[name$="-enabled"]')?.checked ?? true,
        listCommand: v("list"),
        startCommand: v("start"),
        me: v("me"),
      };
    });
  const sendCustom = (tools: CustomTool[]) => h.onCustomTools?.(tools);
  const optional = (text: string) => {
    const f = document.createDocumentFragment();
    const unit = document.createElement("span");
    unit.className = "settings__unit";
    unit.textContent = "(optional)";
    f.append(`${text} `, unit);
    return f;
  };
  customTools.forEach((tool, i) => {
    const change = () => sendCustom(readCustom());
    const on = toggle(`ct-${i}-enabled`, tool.enabled, change, "Use this tool");

    const text = (key: "name" | "listCommand" | "startCommand" | "me", suffix: string, mono: boolean) => {
      const el = document.createElement("input");
      el.type = "text";
      el.name = `ct-${i}-${suffix}`;
      if (mono) el.className = "mono";
      el.value = tool[key];
      el.addEventListener("change", () => {
        el.value = el.value.trim();
        change();
      });
      return el;
    };
    const name = text("name", "name", false);
    name.setAttribute("aria-label", "Tool name");
    name.placeholder = "Linear (mine-only)";

    const details = document.createElement("details");
    details.className = "settings__details";
    details.open = model.customDetailsOpen?.[tool.id] === true;
    details.addEventListener("toggle", () => {
      model.customDetailsOpen = { ...model.customDetailsOpen, [tool.id]: details.open };
    });
    const summary = document.createElement("summary");
    summary.textContent = "Output format and an example script";
    const detailsBody = document.createElement("div");
    detailsBody.className = "settings__details-body";
    const listHint = document.createElement("span");
    listHint.textContent = "It prints JSON like this; id, title and url are required. repo is owner/name, github:owner/name, gitlab:group/name or a clone URL; without one, Maya picks the repository when you start.";
    const format = document.createElement("pre");
    format.textContent = CUSTOM_TOOL_FORMAT;
    const script = document.createElement("pre");
    script.textContent = CUSTOM_TOOL_SCRIPT;
    const stopHint = document.createElement("span");
    stopHint.textContent = "On macOS and Linux, anything a command leaves running is stopped when it ends.";
    detailsBody.append(listHint, format, script, stopHint);
    details.append(summary, detailsBody);

    const remove = settingsBtn("Remove this tool", { action: "ct-remove", kind: "quiet" }, () => {
      const at = [...workItems.querySelectorAll(".settings__custom")].indexOf(box);
      sendCustom(readCustom().filter((_, j) => j !== at));
    });
    remove.classList.add("settings__btn--danger");

    const box = source({
      key: tool.id,
      name: tool.name,
      tag: "Custom tool",
      summary: tool.listCommand,
      monoSummary: true,
      stateText: tool.enabled ? "On" : "Off",
      stateOn: tool.enabled,
      open: sourcesOpen[tool.id] === true,
      onToggle: (open) => rememberSource(tool.id, open),
      control: on,
      body: [
        row({ label: "Name", help: "How its items are labelled on the Work Items tab.", control: name }),
        row({ label: "List command", help: "Runs every 2 minutes through your shell, and must finish within 30 seconds.", control: text("listCommand", "list", true), stack: true }),
        row({ label: optional("On-start command"), help: "Runs when you press Start, with the item's JSON on its input, to assign or move it in your tool.", control: text("startCommand", "start", true), stack: true }),
        row({ label: optional("Your name there"), help: "Start asks before taking over an item assigned to someone else. Blank: it never asks.", control: text("me", "me", false) }),
        details,
        footRow(document.createTextNode(""), remove),
      ],
    });
    box.classList.add("settings__git", "settings__custom");
    box.dataset.tool = tool.id;
    sources.push(box);
  });
  const addTool = addButton("+ Add a custom tool", "ct-add", () =>
    sendCustom([...readCustom(), { id: "", name: "", enabled: true, listCommand: "", startCommand: "", me: "" }]),
  );

  const workInput = document.createElement("textarea");
  workInput.name = "workPrompt";
  workInput.rows = 6;
  workInput.setAttribute("aria-label", "Work prompt");
  workInput.value = workItemsConfigNow.prompt;
  workInput.addEventListener("change", () => h.onWorkPrompt?.(workInput.value.trim()));

  workItems.append(
    group({ title: "Sources", note: "Open one to change what the Work Items tab picks up from it." }, ...sources, addTool),
    group({ title: "Work prompt" }, card(row({
      help: "What Start tells the agent. Blank uses the built-in prompt. {id}, {title}, {url}, {description} and {repo} are filled in.",
      control: workInput,
      stack: true,
    }))),
  );

  let focusFoot: HTMLElement | undefined;
  if (model.focusVisible === false) {
    // macOS keeps the Focus state from apps without Full Disk Access, so
    // Maya would speak through Do Not Disturb.
    focusFoot = document.createElement("span");
    focusFoot.dataset.for = "focus";
    focusFoot.append("Maya cannot tell when a Focus mode such as Do Not Disturb is on, so she keeps speaking. Give her Full Disk Access to let a Focus mode keep her quiet; macOS then asks to quit and reopen her. ");
    focusFoot.append(settingsBtn("Open Full Disk Access", { action: "full-disk-access" }, () => h.onFullDiskAccess?.()));
  }
  notifications.append(
    group({ foot: focusFoot }, card(
      row({ label: "Notify me when a session awaits a decision", control: toggle("notify", model.notifyOnAwaiting, (on) => h.onNotify(on)) }),
      row({ label: "Speak instead of a sound", help: "\"needs a decision\", \"is finished\"", control: toggle("speak", model.speakNotifications, (on) => h.onSpeak(on)) }),
    )),
  );

  // The live role, not the select: previewing Assistant before a Pair
  // succeeds must not untick listening on a machine that is not one yet.
  const isAssistant = model.network?.role === "assistant";
  const listenBox = toggle("listen", isAssistant ? false : model.listen, (on) => h.onListen(on));
  listenBox.disabled = isAssistant;

  const wordInput = document.createElement("input");
  wordInput.type = "text";
  wordInput.name = "listenWord";
  wordInput.placeholder = "Maya";
  wordInput.value = model.listenWord ?? "";
  wordInput.disabled = isAssistant;
  wordInput.addEventListener("change", () => h.onListenWord?.(wordInput.value.trim()));
  const wordRow = row({
    label: "Wake word",
    help: "Blank is \"Maya\". A phone joined to this Maya listens for the same word. Pick a word the speech recognition spells the way you type it.",
    control: wordInput,
  });
  wordRow.querySelector(".settings__help")!.setAttribute("data-for", "listenWord");

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

  const listenRows: HTMLElement[] = [
    row({ label: "Listen for the wake word", help: "On-device speech recognition.", control: listenBox }),
    wordRow,
    row({ label: "Speech recognition", control: rec }),
  ];

  if (model.recognizer === "builtin") {
    const mb = (n: number) => `${Math.round(n / 1_000_000)} MB`;
    const chosen = model.models.find((m) => m.id === model.whisperModel);
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
    listenRows.push(row({ label: "Model", control: sel }));

    if (chosen && !chosen.downloaded) {
      const dl = settingsBtn(
        model.downloading?.id === chosen.id ? `Downloading ${chosen.label}…` : model.downloading ? "Wait for the current download" : `Download (${mb(chosen.bytes)})`,
        { action: "download-model", disabled: model.downloading !== null },
        () => h.onDownloadModel(chosen.id),
      );
      listenRows.push(row({
        label: chosen.label,
        help: model.downloading ? undefined : `Download the model once; it stays on ${thisComputerLower()}.`,
        control: [state("Not downloaded", "off"), dl],
      }));
    }
    if (chosen?.downloaded) {
      const rm = settingsBtn("Remove", { action: "remove-model", kind: "quiet", disabled: true });
      rm.dataset.model = chosen.id;
      rm.title = "The model in use cannot be removed; pick another first.";
      listenRows.push(row({ label: chosen.label, control: [state("In use", "on"), rm] }));
    }
    for (const m of model.models) {
      if (!m.downloaded || m.id === model.whisperModel) continue;
      const rm = settingsBtn(`Remove ${m.label.split(" (")[0]}`, { action: "remove-model", kind: "quiet" }, () => h.onRemoveModel(m.id));
      rm.dataset.model = m.id;
      listenRows.push(row({ label: m.label, control: [state("Downloaded", "on"), rm] }));
    }
    if (model.downloading) {
      const bar = document.createElement("progress");
      bar.setAttribute("name", "modelDownload");
      bar.max = model.downloading.total;
      bar.value = model.downloading.received;
      const downloadingLabel = model.models.find((m) => m.id === model.downloading!.id)?.label ?? model.downloading.id;
      const pct = document.createElement("div");
      pct.className = "settings__progress";
      pct.textContent = `Downloading ${downloadingLabel}… ${Math.round((100 * model.downloading.received) / Math.max(1, model.downloading.total))}%`;
      listenRows.push(row({ control: [bar, pct], stack: true }));
    }
  }

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
  listenRows.push(row({ label: "Microphone", control: mic }));

  // Why the toggle is off (or not Maya's to turn on) is the card's foot note.
  if (isAssistant) {
    // The main Maya owns notifications and listening while this machine is
    // an assistant: say so rather than leave it looking merely unticked.
    const note = document.createElement("span");
    note.className = "settings__hint";
    note.dataset.for = "listen";
    note.textContent = "The main Maya notifies and listens for this machine.";
    listenRows.push(footRow(note));
  } else if (model.listenError) {
    // The listener stopped on its own (Dictation off, no microphone…): say
    // why, or an unticked box looks like a glitch.
    const err = document.createElement("span");
    err.className = "settings__error";
    err.dataset.for = "listen";
    err.textContent = model.listenError;
    listenRows.push(footRow(err));
  }
  const listeningGroup = group({ title: "Listening" }, card(...listenRows));

  const textField = (name: string, label: string, value: string, placeholder: string, onChange: (v: string) => void, mono = false) => {
    const input = document.createElement("input");
    input.type = "text";
    input.name = name;
    input.value = value;
    input.placeholder = placeholder;
    if (mono) input.classList.add("mono");
    input.addEventListener("change", () => onChange(input.value.trim()));
    return row({ label, control: input });
  };
  const keyField = (name: string, set: boolean, placeholder: string, onKey: (k: string) => void) => {
    const key = document.createElement("input");
    key.type = "password";
    key.name = name;
    key.placeholder = set ? "paste to replace" : placeholder;
    key.autocomplete = "off";
    key.addEventListener("change", () => {
      const k = key.value.trim();
      if (k) onKey(k);
      key.value = "";
    });
    return row({ label: "API key", help: set ? `Saved in ${secretStore()}.` : undefined, control: key });
  };

  const elevenRows: HTMLElement[] = [
    cardHead({ label: "ElevenLabs", order: 1, control: toggle("elevenEnabled", model.elevenEnabled, (on) => h.onElevenEnabled(on), "Use ElevenLabs") }),
  ];
  if (model.elevenEnabled) {
    elevenRows.push(keyField("elevenKey", model.elevenKeySet, "paste your key", (k) => h.onElevenKey(k)));

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
    elevenRows.push(row({ label: "Voice", control: voice }));
  }

  const serverRows: HTMLElement[] = [
    cardHead({
      label: "Voice server",
      help: "Any OpenAI-compatible speech server.",
      order: 2,
      control: toggle("serverEnabled", model.serverEnabled, (on) => h.onServerEnabled(on), "Use a voice server"),
    }),
  ];
  if (model.serverEnabled) {
    serverRows.push(textField("serverUrl", "Server address", model.serverUrl, "http://localhost:8004", (u) => h.onServerUrl(u), true));

    const listError = model.serverVoicesError ? `Could not list the server's voices: ${model.serverVoicesError}` : undefined;
    let serverVoiceRow: HTMLElement;
    if (model.serverVoices.length > 0) {
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
      serverVoiceRow = row({ label: "Voice", help: listError, control: voice });
    } else {
      const text = document.createElement("input");
      text.type = "text";
      text.name = "serverVoice";
      text.value = model.serverVoice;
      text.placeholder = "default";
      text.addEventListener("change", () => h.onServerVoice(text.value.trim()));
      serverVoiceRow = row({ label: "Voice", help: listError, control: text });
    }
    serverVoiceRow.querySelector(".settings__help")?.setAttribute("data-for", "server-voices");
    serverRows.push(serverVoiceRow);

    serverRows.push(textField("serverModel", "Model", model.serverModel, "optional", (m) => h.onServerModel(m)));
    serverRows.push(keyField("serverKey", model.serverKeySet, "optional", (k) => h.onServerKey(k)));
  }

  // The number follows the drag; the volume is saved once the slider is let go.
  const volume = document.createElement("input");
  volume.type = "range";
  volume.name = "voiceVolume";
  volume.min = "10";
  volume.max = "100";
  volume.step = "5";
  volume.setAttribute("aria-label", "Voice volume");
  volume.value = String(model.voiceVolume ?? 100);
  const volumeShown = document.createElement("span");
  volumeShown.dataset.for = "voice-volume";
  volumeShown.textContent = `${volume.value}%`;
  volume.addEventListener("input", () => { volumeShown.textContent = `${volume.value}%`; });
  volume.addEventListener("change", () => h.onVoiceVolume(Number(volume.value)));
  const volumeControl = document.createElement("span");
  volumeControl.className = "settings__range";
  volumeControl.append(volume, volumeShown);

  let tryNote: HTMLElement;
  if (model.voiceReport) {
    tryNote = document.createElement("span");
    tryNote.className = "settings__hint";
    tryNote.dataset.for = "voice-report";
    tryNote.textContent = model.voiceReport;
  } else {
    tryNote = state("Hear how Maya sounds now", "off");
  }

  const elevenCard = card(...elevenRows);
  elevenCard.classList.toggle("settings__card--off", !model.elevenEnabled);
  const serverCard = card(...serverRows);
  serverCard.classList.toggle("settings__card--off", !model.serverEnabled);
  const speakingGroup = group(
    { title: "Speaking", note: `Maya speaks with the first voice that works, in this order.` },
    elevenCard,
    serverCard,
    card(cardHead({ label: builtinVoiceName(), help: "Built in; always there.", order: 3 })),
    card(
      row({ label: "Volume", control: volumeControl }),
      footRow(tryNote, settingsBtn("Try the voice", { action: "try-voice" }, () => h.onTryVoice())),
    ),
  );
  speakingGroup.querySelector(".settings__group-note")!.setAttribute("data-for", "voice-order");
  voice.append(speakingGroup, listeningGroup);

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

  // The first card: the role, then what that role needs. What would be said
  // about the card as a whole (an error, the status, "pair again") sits in its foot.
  const netRows: HTMLElement[] = [row({ label: "This Maya", control: roleSel })];
  const netFoot: HTMLElement[] = [];

  if (netRole !== "off" && net.pairAgain) {
    const note = document.createElement("div");
    note.className = "settings__hint";
    note.dataset.for = "pair-again-note";
    note.textContent = "This Maya was updated to secure pairing — pair again.";
    netFoot.push(footRow(note));
  }

  // Both roles carry a name: assistants show the main's in "Connected to …",
  // and the main shows an assistant's on its cards.
  const nameRow = () => {
    const nameInput = document.createElement("input");
    nameInput.type = "text";
    nameInput.name = "networkName";
    nameInput.placeholder = "blank uses this computer's hostname";
    nameInput.value = netName;
    nameInput.disabled = !!model.networkPromoting;
    nameInput.addEventListener("input", () => { model.networkName = nameInput.value; });
    nameInput.addEventListener("change", () => h.onName(nameInput.value.trim()));
    return row({ label: "This machine's name", control: nameInput });
  };
  const pairingGroups: HTMLElement[] = [];

  if (netRole === "main") {
    if (net.mainError) {
      const err = document.createElement("div");
      err.className = "settings__error";
      err.dataset.for = "network";
      err.textContent = net.mainError;
      netFoot.push(footRow(err));
    }
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
    const addresses = net.addresses && net.addresses.length > 0 ? addressesText(net.addresses) : null;
    const portRow = row({ label: "Port", help: addresses, control: portInput });
    portRow.querySelector(".settings__help")?.classList.add("settings__addresses");
    netRows.push(portRow, nameRow());

    const live = !!net.code && nowMs <= net.code.expiresAt;
    const pairCard = document.createElement("div");
    pairCard.className = "settings__pair";
    const codeBox = document.createElement("div");
    codeBox.className = live ? "settings__code" : "settings__code settings__code--empty";
    codeBox.textContent = live && net.code ? formatPairingCode(net.code.code) : "No code yet";
    const pairText = document.createElement("div");
    pairText.className = "settings__pair-text";
    if (live && net.code) {
      const title = document.createElement("strong");
      title.className = "settings__pair-title";
      title.textContent = "Type this on the machine you are pairing.";
      const expiry = document.createElement("div");
      expiry.className = "settings__hint settings__code-expiry";
      expiry.textContent = `expires in ${expiresInMinutes(net.code.expiresAt, nowMs)} min`;
      pairText.append(title, expiry);
    } else if (net.codeUsedUp) {
      const used = document.createElement("div");
      used.className = "settings__hint";
      used.textContent = "Code used up after 3 wrong tries — Regenerate.";
      pairText.append(used);
    }
    // Reopening pairing mid-handover would let a new assistant pair with a main about to stop.
    const regen = settingsBtn(live ? "Regenerate" : "Show pairing code", { action: "regenerate-code", disabled: !!model.networkPromoting }, () => h.onRegenerate());
    pairCard.append(codeBox, pairText, regen);
    pairingGroups.push(group({ title: "Pairing code", foot: "Paired devices are in Settings › Devices." }, pairCard));
  } else if (netRole === "assistant") {
    // Values live on the model, updated on every keystroke (not just on
    // `change`), so an unrelated repaint (a `voice` or `network` event) never
    // wipes a half-typed field; `change` still drives when each is saved.
    const hostInput = document.createElement("input");
    hostInput.type = "text";
    hostInput.name = "networkHost";
    hostInput.placeholder = "e.g. 192.168.1.42 or maya-mini.local";
    hostInput.value = netMainHost;
    hostInput.disabled = !!model.networkPromoting;
    hostInput.addEventListener("input", () => { model.networkMainHost = hostInput.value; });
    hostInput.addEventListener("change", () => h.onMainHost(hostInput.value.trim()));

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

    netRows.push(
      row({ label: "Main's host or address", control: hostInput }),
      row({ label: "Port", control: mainPortInput }),
      nameRow(),
    );

    const needsCode = !model.networkPaired || model.networkRepair;
    if (needsCode) {
      const codeInput = document.createElement("input");
      codeInput.type = "text";
      codeInput.name = "networkCode";
      codeInput.placeholder = "483921";
      codeInput.value = netCode;
      codeInput.addEventListener("input", () => { model.networkCode = codeInput.value; });
      netRows.push(row({ label: "Pairing code", control: codeInput }));
    }

    // Shown whenever the select shows Assistant, saved or still being
    // previewed, so a failed Pair on a never-paired machine has somewhere to
    // put its message instead of only the page-level error banner.
    const statusText = model.networkError
      ? model.networkError
      : net.role === "assistant"
        ? net.assistant.connected
          ? `Connected to ${net.assistant.mainName ?? ""}`
          : net.assistant.error && !net.assistant.retrying
            ? net.assistant.error
            : "Reconnecting…"
        : null;
    let status: HTMLElement | null = null;
    if (statusText !== null) {
      status = document.createElement("div");
      const statusEl = document.createElement("div");
      statusEl.className = "settings__status settings__status--network";
      statusEl.textContent = statusText;
      status.append(statusEl);
      // While it retries, the last failure sits under "Reconnecting…".
      if (!model.networkError && net.role === "assistant" && !net.assistant.connected && net.assistant.retrying && net.assistant.error) {
        statusEl.title = net.assistant.error;
        const last = document.createElement("div");
        last.className = "settings__hint";
        last.dataset.for = "network-last-error";
        last.textContent = `Last error: ${net.assistant.error}`;
        status.append(last);
      }
    }

    let actions: HTMLElement | null = null;
    if (needsCode) {
      actions = settingsBtn(model.networkPairing ? "Pairing…" : "Pair", { action: "pair", kind: "gold", disabled: !!model.networkPairing }, () => {
        if (model.networkPairing) return;
        h.onPair((model.networkMainHost ?? "").trim(), model.networkMainPort ?? 0, (model.networkName ?? "").trim(), (model.networkCode ?? "").trim());
      });
    } else {
      const forget = settingsBtn("Forget main", { action: "forget-main", kind: "danger" }, () => h.onForgetMain());
      forget.classList.add("settings__btn--quiet");
      actions = document.createElement("div");
      actions.className = "settings__foot-actions";
      actions.append(settingsBtn("Pair again", { action: "pair-again", kind: "quiet" }, () => h.onPairAgain()), forget);
    }
    netFoot.push(footRow(status ?? document.createElement("span"), actions));
  }
  network.append(group({}, card(...netRows, ...netFoot)), ...pairingGroups);

  if (netRole !== "off") {
    const keyHelp = "Drops every pairing, so each device must pair again.";
    let keyRow: HTMLElement;
    if (model.networkNewKeyConfirm) {
      const confirm = document.createElement("div");
      confirm.className = "settings__confirm";
      confirm.dataset.for = "new-key-confirm";
      const p = document.createElement("p");
      p.textContent = "Drop every pairing and make a new key?";
      confirm.append(
        p,
        settingsBtn("Drop and make a new key", { action: "new-key-confirm", kind: "danger" }, () => h.onNewKey()),
        settingsBtn("Cancel", { action: "new-key-cancel" }, () => h.onNewKeyCancel()),
      );
      keyRow = row({ label: "New key", help: keyHelp, control: confirm, stack: true });
    } else {
      const fresh = settingsBtn("New key", { action: "new-key", kind: "danger", disabled: !!model.networkPromoting }, () => h.onNewKeyAsk());
      keyRow = row({ label: "New key", help: keyHelp, control: fresh });
    }
    network.append(group({}, card(keyRow)));
  }

  if (net.role === "off") {
    const none = document.createElement("div");
    none.className = "settings__hint";
    none.dataset.for = "devices-off";
    none.dataset.row = "devices-off";
    none.textContent = "Turn the network on in Settings › Network to see your devices.";
    devices.append(group({}, card(none)));
  } else {
    // The model's fields behind the view; a typed host stays in its input.
    const view: DevicesModel = {
      get joining() { return model.devicesJoining ?? null; },
      set joining(v) { model.devicesJoining = v; },
      joinHost: "",
      joinPort: 0,
      get promote() { return model.networkPromote ?? null; },
      set promote(v) { model.networkPromote = v; },
      get promoting() { return !!model.networkPromoting; },
      set promoting(v) { model.networkPromoting = v; },
    };
    const dh: DevicesHandlers = h;
    devices.append(renderDevices(net, view, dh, nowMs, thisComputerLower()));
  }

  if (model.error) {
    // Above the sections, so it shows whichever group is open.
    content.prepend(notice(model.error, "error"));
  }
  return root;
}

/**
 * Puts a fresh render in the panel. Everything is replaced, as its listeners
 * close over what they were rendered with, except the Devices section: it is
 * never detached, so a hovered row keeps its :hover (WebKit drops it on a
 * node that is moved or replaced). Its one delegated listener reads the DOM,
 * so the old list is safe to keep.
 */
export function patchPanel(panel: HTMLElement, fresh: HTMLElement): void {
  const root = panel.firstElementChild;
  const content = root?.querySelector<HTMLElement>(":scope > .settings__content");
  const nav = root?.querySelector<HTMLElement>(":scope > .settings__nav");
  const freshContent = fresh.querySelector<HTMLElement>(":scope > .settings__content");
  const freshNav = fresh.querySelector<HTMLElement>(":scope > .settings__nav");
  const kept = content?.querySelector<HTMLElement>(":scope > .settings__section--devices");
  const freshKept = freshContent?.querySelector<HTMLElement>(":scope > .settings__section--devices");
  if (panel.childElementCount !== 1 || !root || !content || !nav || !freshContent || !freshNav || !kept || !freshKept) {
    panel.replaceChildren(fresh);
    return;
  }
  morph(kept, freshKept);
  for (const c of [...content.children]) if (c !== kept) c.remove();
  let seen = false;
  let cursor: Element = kept;
  for (const c of [...freshContent.children]) {
    if (c === freshKept) {
      seen = true;
    } else if (!seen) {
      content.insertBefore(c, kept);
    } else {
      content.insertBefore(c, cursor.nextSibling);
      cursor = c;
    }
  }
  nav.replaceWith(freshNav);
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
    sourcesOpen: storedSourcesOpen(),
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

  const loadCoralStatus = async () => {
    try {
      model.coralHasToken = (await invoke<{ hasToken: boolean }>("get_coral_status")).hasToken;
    } catch {
      model.coralHasToken ??= false;
    }
  };

  /** Lists Coral's statuses, for the card to offer; a card left open lists them on its own. */
  const loadCoralStatuses = async (): Promise<CoralStatus[]> => {
    const list = await invoke<CoralStatus[]>("coral_statuses");
    model.coralStatuses = list;
    if (!panel.hidden) paint();
    return list;
  };

  const saveWorkItems = (patch: (w: WorkItemsConfig) => void, saved?: () => void) =>
    run(async () => {
      const fresh = await invoke<ConfigJson>("get_config");
      const workItems = workItemsConfig(fresh.workItems);
      patch(workItems);
      const c = await invoke<ConfigJson>("set_config", { config: { ...fresh, workItems } });
      applyConfig(c);
      saved?.();
    });

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
    model.workItems = workItemsConfig(c.workItems);
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
    patchPanel(panel, renderSettings(model, handlers));
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
    onWorkItems: (source: GitProvider | "coral", items: ForgeItems | CoralItems) =>
      void saveWorkItems((w) => {
        if (source === "coral") w.coral = items as CoralItems;
        else w[source] = items as ForgeItems;
      }),
    onWorkPrompt: (text) => void saveWorkItems((w) => { w.prompt = text; }),
    onCustomTools: (tools) => {
      // A tool just added has no id until the backend names it: open
      // whichever tool the save brings back that was not there before.
      const before = new Set((model.workItems?.custom ?? []).map((t) => t.id));
      const adding = tools.some((t) => t.id === "");
      void saveWorkItems((w) => { w.custom = tools; }, () => {
        if (!adding) return;
        const added = (model.workItems?.custom ?? []).filter((t) => !before.has(t.id));
        if (added.length > 0) setSourcesOpen(model, Object.fromEntries(added.map((t) => [t.id, true])));
      });
    },
    onOpenGroup: (group) => document.dispatchEvent(new CustomEvent(OPEN_SETTINGS_GROUP, { detail: group })),
    onGitEnabled: (p, on) => void saveGit(p, (c) => { c.enabled = on; }),
    onGitBaseUrl: (url) => void saveGit("gitlab", (c) => { c.baseUrl = url || null; }),
    onGitClone: (p, protocol) => void saveGit(p, (c) => { c.clone = protocol; }),
    onCoralToken: (token) => void run(async () => { await invoke("store_coral_token", { token }); model.coralStatuses = null; model.coralTest = null; await loadCoralStatus(); }),
    onCoralClearToken: () => void run(async () => { await invoke("clear_coral_token"); model.coralStatuses = null; model.coralTest = null; await loadCoralStatus(); }),
    onCoralTest: async () => {
      try {
        model.coralTest = await invoke<string>("test_coral_connection");
      } catch (e) {
        model.coralTest = String(e);
        throw model.coralTest;
      } finally {
        if (!panel.hidden) paint();
      }
      return model.coralTest;
    },
    onCoralStatuses: loadCoralStatuses,
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
    onRemoveMember: (key) => void run(async () => { model.network = await invoke<NetworkStatus>("network_remove_member", { key }); }),
    onJoinAsk: (key) => {
      model.devicesJoining = key;
      paint();
    },
    onJoinCancel: () => {
      model.devicesJoining = null;
      paint();
    },
    onJoin: (key, host, port) => void run(async () => {
      model.network = await invoke<NetworkStatus>("network_join", { key, host, port });
      model.devicesJoining = null;
    }),
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
    await loadCoralStatus();
    if (model.workItems && coralCanAsk(model.coralHasToken, model.workItems.coral) && model.sourcesOpen?.coral) void loadCoralStatuses().catch(() => { /* the card says to Test */ });
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
