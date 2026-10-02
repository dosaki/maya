import { invoke } from "@tauri-apps/api/core";
import { showToast } from "./toast";
import { sendShortcut, thisComputer } from "./platform";
import { harnessLabel } from "./harness";
import type { Harness } from "./types";

/** Choices for a new session; "" means "use the agent's default". */
export interface SessionOptions {
  model: string;
  effort: string;
  mode: string;
}

/** Value/label pairs, in the order the dropdowns show them. */
export const MODEL_CHOICES: [string, string][] = [["fable", "Fable"], ["opus", "Opus"], ["sonnet", "Sonnet"], ["haiku", "Haiku"]];
export const EFFORT_CHOICES: [string, string][] = ["low", "medium", "high", "xhigh", "max"].map((v) => [v, v]);
export const MODE_CHOICES: [string, string][] = ["manual", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"].map((v) => [v, v]);

export interface ModelInfo {
  id: string;
  label: string;
  /** The efforts this model takes; empty when they are the agent's. */
  efforts: string[];
}

/** One agent a machine can start, as `list_agents` reports it. */
export interface AgentInfo {
  harness: Harness;
  models: ModelInfo[];
  /** With the Default model: the efforts every model takes. */
  efforts: string[];
  modes: string[];
}

/** What Start sends: the agent, the name ("" for none) and its options. */
export type StartChoices = SessionOptions & { agent: Harness; name: string };

/** Claude Code as Maya always knows it, before or without a listing. */
export const CLAUDE_AGENT: AgentInfo = {
  harness: "claude-code",
  models: MODEL_CHOICES.map(([id, label]) => ({ id, label, efforts: [] })),
  efforts: EFFORT_CHOICES.map(([v]) => v),
  modes: MODE_CHOICES.map(([v]) => v),
};

type OptionField = { name: keyof SessionOptions; label: string; choices: [string, string][] };

/** The option fields `agent` has, for the chosen `model`; fields without choices are left out. */
export function optionFields(agent: AgentInfo, model: string): OptionField[] {
  const chosen = agent.models.find((m) => m.id === model);
  const efforts = chosen && chosen.efforts.length > 0 ? chosen.efforts : agent.efforts;
  const fields: OptionField[] = [
    { name: "model", label: "Model", choices: agent.models.map((m) => [m.id, m.label]) },
    { name: "effort", label: "Effort", choices: efforts.map((v) => [v, v]) },
    { name: "mode", label: "Mode", choices: agent.modes.map((v) => [v, v]) },
  ];
  return fields.filter((f) => f.choices.length > 0);
}

/** A labelled select with a "Default" (empty) entry first. */
export function renderChoice(name: string, label: string, choices: [string, string][], value: string, first = "Default"): HTMLLabelElement {
  const field = el("label", "newsession__field");
  field.append(el("span", "newsession__label", label));
  const select = el("select", "newsession__select");
  select.name = name;
  const def = document.createElement("option");
  def.value = "";
  def.textContent = first;
  select.append(def);
  for (const [v, text] of choices) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    select.append(o);
  }
  select.value = choices.some(([v]) => v === value) ? value : "";
  field.append(select);
  return field;
}

/** A machine choice for the picker: "This Mac" first, then each connected assistant. */
export interface MachineChoice {
  name: string;
  value: string;
}

export interface NewSessionModel {
  dirs: string[];
  dir: string | null;
  prompt: string;
  options: Partial<SessionOptions>;
  status: { ok: boolean; text: string } | null;
  busy: boolean;
  needsSetup: boolean;
  /** Set once a start succeeded: the prompt is spent and must not come back as a draft. */
  done?: boolean;
  /** "This Mac" first, then each connected assistant. */
  machines: MachineChoice[];
  /** The chosen machine's value; "" is this Mac. */
  machine: string;
  /** The chosen machine's agents, Claude Code first. */
  agents: AgentInfo[];
  agent: Harness;
  /** The machine's Maya takes names (an older one does not). */
  names: boolean;
  name: string;
}

export interface NewSessionHandlers {
  onStart(machine: string, dir: string | null, prompt: string, choices: StartChoices): void;
  onClose(): void;
  onOpenSettings(): void;
  onMachine(machine: string): void;
  onAgent(agent: Harness): void;
  /** The model changed: Codex's efforts depend on it. */
  onModel(model: string): void;
}

interface StartResult {
  dir: string;
  how: "chosen" | "classifier" | "fallback";
  /** The terminal's name for the new session, where it has names (tmux). */
  terminal?: string;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/**
 * What to do when the chosen machine has no folders to offer: on this Mac,
 * set the projects directory here; on an assistant, set it over there.
 */
function renderSetup(m: { machine: string; machines: MachineChoice[] }, h: { onOpenSettings(): void }): HTMLElement {
  const setup = el("div", "modal__setup");
  if (m.machine !== "") {
    const name = m.machines.find((mm) => mm.value === m.machine)?.name ?? m.machine;
    setup.append(el("p", "", `Set a projects directory in Settings on ${name}. Its subfolders become the choices here.`));
    return setup;
  }
  setup.append(el("p", "", "Set a projects directory in Settings first. Its subfolders become the choices here."));
  const open = el("button", "card__btn card__btn--primary", "Open Settings");
  open.type = "button";
  open.dataset.action = "open-settings";
  open.addEventListener("click", () => h.onOpenSettings());
  setup.append(open);
  return setup;
}

export function renderNewSession(m: NewSessionModel, h: NewSessionHandlers): HTMLElement {
  const root = el("div", "modal");
  const backdrop = el("div", "modal__backdrop");
  backdrop.addEventListener("click", () => h.onClose());
  const panel = el("section", "modal__panel modal__panel--compact");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");

  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", "New session"));
  const close = el("button", "modal__close", "×");
  close.type = "button";
  close.dataset.action = "close";
  close.setAttribute("aria-label", "Close");
  close.addEventListener("click", () => h.onClose());
  head.append(titles, close);
  panel.append(head);

  const form = el("div", "newsession");

  // The picker comes first, so a machine without folders can still be left for another.
  if (m.machines.length > 1) {
    const machineLabel = el("label", "newsession__field");
    machineLabel.append(el("span", "newsession__label", "Machine"));
    const machineSelect = el("select", "newsession__select");
    machineSelect.name = "machine";
    for (const mm of m.machines) {
      const o = document.createElement("option");
      o.value = mm.value;
      o.textContent = mm.name;
      machineSelect.append(o);
    }
    machineSelect.value = m.machine;
    machineSelect.addEventListener("change", () => h.onMachine(machineSelect.value));
    machineLabel.append(machineSelect);
    form.append(machineLabel);
  }

  if (m.needsSetup) {
    if (form.childElementCount > 0) panel.append(form);
    panel.append(renderSetup(m, h));
    root.append(backdrop, panel);
    return root;
  }

  const agent = m.agents.find((a) => a.harness === m.agent) ?? m.agents[0] ?? CLAUDE_AGENT;
  // One agent is no choice: the field shows only when the machine has more.
  let agentLabel: HTMLLabelElement | null = null;
  if (m.agents.length > 1) {
    agentLabel = el("label", "newsession__field");
    agentLabel.append(el("span", "newsession__label", "Agent"));
    const agentSelect = el("select", "newsession__select");
    agentSelect.name = "agent";
    for (const a of m.agents) {
      const o = document.createElement("option");
      o.value = a.harness;
      o.textContent = harnessLabel(a.harness);
      agentSelect.append(o);
    }
    agentSelect.value = agent.harness;
    agentSelect.addEventListener("change", () => h.onAgent(agentSelect.value as Harness));
    agentLabel.append(agentSelect);
  }

  const remoteMachine = m.machine !== "";
  // Claude cannot choose on a remote machine: a folder is always picked there.
  if (remoteMachine && !m.dir && m.dirs.length > 0) m.dir = m.dirs[0];
  const dirLabel = el("label", "newsession__field");
  dirLabel.append(el("span", "newsession__label", "Directory"));
  const select = el("select", "newsession__select");
  select.name = "dir";
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Let Claude choose";
  auto.disabled = remoteMachine;
  select.append(auto);
  for (const d of m.dirs) {
    const o = document.createElement("option");
    o.value = d;
    o.textContent = d;
    select.append(o);
  }
  select.value = m.dir ?? "";
  dirLabel.append(select);
  if (remoteMachine) dirLabel.append(el("p", "newsession__hint", "Claude can't choose for you on a remote machine; pick a folder."));

  // An older Maya over there would ignore a name, so it is not offered.
  let nameInput: HTMLInputElement | null = null;
  let nameLabel: HTMLLabelElement | null = null;
  if (m.names) {
    nameLabel = el("label", "newsession__field");
    nameLabel.append(el("span", "newsession__label", "Name"));
    nameInput = el("input", "newsession__select");
    nameInput.name = "name";
    nameInput.type = "text";
    nameInput.maxLength = 60;
    nameInput.placeholder = "Optional; the agent names it otherwise";
    nameInput.value = m.name;
    nameLabel.append(nameInput);
  }

  const promptLabel = el("label", "newsession__field");
  promptLabel.append(el("span", "newsession__label", "Prompt"));
  const ta = el("textarea", "modal__input");
  ta.name = "prompt";
  ta.rows = 5;
  ta.placeholder = `What should this session do? (${sendShortcut()} to start)`;
  ta.value = m.prompt;
  promptLabel.append(ta);

  const optionRow = el("div", "newsession__options");
  const optionSelects: [keyof SessionOptions, HTMLSelectElement][] = [];
  for (const f of optionFields(agent, m.options.model ?? "")) {
    const field = renderChoice(f.name, f.label, f.choices, m.options[f.name] ?? "");
    const sel = field.querySelector("select")!;
    if (f.name === "model") sel.addEventListener("change", () => h.onModel(sel.value));
    optionSelects.push([f.name, sel]);
    optionRow.append(field);
  }
  const readChoices = (): StartChoices => {
    const o: StartChoices = { agent: agent.harness, name: nameInput?.value.trim() ?? "", model: "", effort: "", mode: "" };
    for (const [name, sel] of optionSelects) o[name] = sel.value;
    return o;
  };

  const start = el("button", "card__btn card__btn--primary", m.busy ? "Starting…" : "Start");
  start.type = "button";
  start.dataset.action = "start";
  const sync = () => {
    start.disabled = m.busy || ta.value.trim() === "" || (remoteMachine && select.value === "");
  };
  const tryStart = () => {
    if (start.disabled) return;
    const prompt = ta.value.trim();
    if (!prompt) return;
    h.onStart(m.machine, select.value || null, prompt, readChoices());
  };
  ta.addEventListener("input", sync);
  select.addEventListener("change", sync);
  const startOnShortcut = (ev: KeyboardEvent) => {
    if (ev.key === "Enter" && (ev.metaKey || ev.ctrlKey)) {
      ev.preventDefault();
      tryStart();
    }
  };
  ta.addEventListener("keydown", startOnShortcut);
  nameInput?.addEventListener("keydown", startOnShortcut);
  start.addEventListener("click", tryStart);
  sync();

  const actions = el("div", "newsession__actions");
  actions.append(start);
  const fields: (HTMLElement | null)[] = [agentLabel, dirLabel, optionRow, nameLabel, promptLabel, actions];
  form.append(...fields.filter((n): n is HTMLElement => n !== null));
  panel.append(form);
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.ok ? "ok" : "error"}`, m.status.text));

  root.append(backdrop, panel);
  return root;
}

let current: { model: NewSessionModel; keyHandler: (e: KeyboardEvent) => void } | null = null;
let draft = "";
let draftName = "";
/** The agent last chosen, offered again wherever the machine has it. */
let lastAgent: Harness = "claude-code";
/** The last options per agent; unlike the prompt they are kept after a start. */
const lastOptions: Partial<Record<Harness, SessionOptions>> = {};
const EMPTY: SessionOptions = { model: "", effort: "", mode: "" };
const optionsFor = (a: Harness): SessionOptions => ({ ...EMPTY, ...lastOptions[a] });

/**
 * The options on screen, a field the agent lacks reading "" (Grok has no
 * Effort); null when none are shown, as on the setup hint.
 */
function readOptionsFrom(host: ParentNode): SessionOptions | null {
  const sel = (name: string) => host.querySelector<HTMLSelectElement>(`select[name=${name}]`);
  const model = sel("model");
  const effort = sel("effort");
  const mode = sel("mode");
  if (!model && !effort && !mode) return null;
  return { model: model?.value ?? "", effort: effort?.value ?? "", mode: mode?.value ?? "" };
}

/** Keeps what the person typed or chose under the agent it was shown for. */
function saveOptions(host: ParentNode, agent: Harness): void {
  const opts = readOptionsFrom(host);
  if (opts) lastOptions[agent] = opts;
}

function startedText(r: StartResult): string {
  const name = r.dir.split("/").filter(Boolean).pop() ?? r.dir;
  if (r.how === "chosen") return `Started in ${name}`;
  if (r.how === "classifier") return `Started in ${name} (chosen by Claude)`;
  return `Started in ${name} (no clear match, so the projects directory itself)`;
}

/** Reads what is on screen back into the model, so a repaint keeps it. */
function capture(): void {
  const host = document.getElementById("modal-host");
  if (!host || !current) return;
  const m = current.model;
  const ta = host.querySelector<HTMLTextAreaElement>("textarea[name=prompt]");
  if (ta && !m.done) m.prompt = ta.value;
  const name = host.querySelector<HTMLInputElement>("input[name=name]");
  if (name && !m.done) m.name = name.value;
  const sel = host.querySelector<HTMLSelectElement>("select[name=dir]");
  if (sel) m.dir = sel.value || null;
  saveOptions(host, m.agent);
  m.options = optionsFor(m.agent);
}

function repaint(): void {
  const host = document.getElementById("modal-host");
  if (!host || !current) return;
  // A repaint replaces every field; the one being used must keep the focus
  // (and the caret), or a late reply sends the next keystrokes to the Prompt.
  const active = document.activeElement;
  const focused = active instanceof HTMLElement && host.contains(active) ? active.getAttribute("name") : null;
  const caret = active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement ? [active.selectionStart, active.selectionEnd] : null;
  host.replaceChildren(
    renderNewSession(current.model, {
      onStart: (machine, dir, prompt, choices) => void start(machine, dir, prompt, choices),
      onClose: closeNewSession,
      onOpenSettings: () => {
        closeNewSession();
        document.querySelector<HTMLElement>("[data-tab=settings]")?.click();
      },
      onMachine: (machine) => void chooseMachine(machine),
      onAgent: (agent) => {
        if (!current) return;
        capture();
        current.model.agent = lastAgent = agent;
        current.model.options = optionsFor(agent);
        // Not paint(): the selects on screen belong to the old agent.
        repaint();
      },
      onModel: () => paint(),
    }),
  );
  const again = focused ? host.querySelector<HTMLElement>(`[name="${focused}"]`) : null;
  if (!again) {
    host.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")?.focus();
    return;
  }
  again.focus();
  if (caret && (again instanceof HTMLInputElement || again instanceof HTMLTextAreaElement)) again.setSelectionRange(caret[0], caret[1]);
}

function paint(): void {
  capture();
  repaint();
}

async function loadDirs(machine: string): Promise<void> {
  if (!current) return;
  const me = current;
  try {
    const dirs = await invoke<string[]>("list_project_dirs", { machine });
    if (current !== me || me.model.machine !== machine) return;
    me.model.dirs = dirs;
    // An assistant with no folders has no projects directory set over there.
    me.model.needsSetup = machine !== "" && dirs.length === 0;
  } catch (e) {
    if (current !== me || me.model.machine !== machine) return;
    const msg = String(e);
    if (machine === "" && msg.includes("projects directory")) me.model.needsSetup = true;
    else me.model.status = { ok: false, text: msg };
  }
  paint();
}

/**
 * Asks the machine which agents it can start. The modal is usable before the
 * answer (locally the first one waits seconds for the agents' model lists),
 * and a failure leaves it offering Claude Code alone.
 */
async function loadAgents(machine: string): Promise<void> {
  const me = current;
  if (!me) return;
  let reply: { agents: AgentInfo[]; names: boolean };
  try {
    reply = await invoke<{ agents: AgentInfo[]; names: boolean }>("list_agents", { machine });
  } catch {
    reply = { agents: [CLAUDE_AGENT], names: machine === "" };
  }
  if (current !== me || me.model.machine !== machine) return;
  capture();
  me.model.agents = reply.agents.length > 0 ? reply.agents : [CLAUDE_AGENT];
  me.model.names = reply.names;
  const wanted = me.model.agents.some((a) => a.harness === lastAgent) ? lastAgent : "claude-code";
  me.model.agent = wanted;
  me.model.options = optionsFor(wanted);
  repaint();
}

async function chooseMachine(machine: string): Promise<void> {
  if (!current) return;
  // Keep what was on screen before the fields change under it.
  capture();
  const m = current.model;
  m.machine = machine;
  m.dir = null;
  m.dirs = [];
  // Setup belongs to the machine that needed it; `loadDirs` decides again for this one.
  m.needsSetup = false;
  // Claude Code only until the machine says what else it has; `loadAgents` brings back the last agent.
  m.agents = [CLAUDE_AGENT];
  m.agent = "claude-code";
  m.names = machine === "";
  m.options = optionsFor("claude-code");
  repaint();
  void loadAgents(machine);
  await loadDirs(machine);
}

async function start(machine: string, dir: string | null, prompt: string, choices: StartChoices): Promise<void> {
  if (!current || current.model.busy) return;
  const me = current;
  me.model.busy = true;
  me.model.dir = dir;
  me.model.prompt = prompt;
  me.model.name = choices.name;
  const { model, effort, mode } = choices;
  me.model.options = lastOptions[choices.agent] = { model, effort, mode };
  me.model.status = { ok: true, text: dir ? "Starting…" : "Choosing a repository…" };
  paint();
  try {
    const r = await invoke<StartResult>("start_session", { dir, prompt, options: choices, machine });
    draft = "";
    draftName = "";
    if (current !== me) {
      showToast(startedText(r));
      return;
    }
    // The prompt is spent: clear it everywhere and keep Start disabled until the modal closes.
    me.model.done = true;
    me.model.busy = true;
    me.model.prompt = "";
    me.model.name = "";
    me.model.status = { ok: true, text: startedText(r) };
    paint();
    setTimeout(() => {
      if (current === me) closeNewSession();
    }, 1500);
  } catch (e) {
    if (current !== me) {
      showToast(String(e));
      return;
    }
    me.model.busy = false;
    me.model.status = { ok: false, text: String(e) };
    paint();
  }
}

export async function openNewSession(): Promise<void> {
  closeNewSession();
  const keyHandler = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeNewSession();
  };
  current = {
    model: {
      dirs: [],
      dir: null,
      prompt: draft,
      options: optionsFor("claude-code"),
      status: null,
      busy: false,
      needsSetup: false,
      machines: [{ name: thisComputer(), value: "" }],
      machine: "",
      // Claude Code until this Mac's listing arrives; `loadAgents` brings back the last agent.
      agents: [CLAUDE_AGENT],
      agent: "claude-code",
      names: true,
      name: draftName,
    },
    keyHandler,
  };
  document.addEventListener("keydown", keyHandler);
  paint();
  void invoke<{ name: string; hostname: string; platform: string; connected: boolean }[]>("list_machines")
    .then((machines) => {
      if (!current) return;
      current.model.machines = [{ name: thisComputer(), value: "" }, ...machines.filter((m) => m.connected).map((m) => ({ name: m.name, value: m.name }))];
      paint();
    })
    .catch(() => undefined);
  void loadAgents("");
  await loadDirs("");
}

export function closeNewSession(): void {
  if (!current) return;
  const host = document.getElementById("modal-host");
  const ta = host?.querySelector<HTMLTextAreaElement>("textarea[name=prompt]");
  const name = host?.querySelector<HTMLInputElement>("input[name=name]");
  // Keep the drafts even mid-start; only a successful start spends them.
  if (ta && !current.model.done) draft = ta.value;
  if (name && !current.model.done) draftName = name.value;
  if (host) saveOptions(host, current.model.agent);
  document.removeEventListener("keydown", current.keyHandler);
  current = null;
  document.getElementById("modal-host")?.replaceChildren();
}
