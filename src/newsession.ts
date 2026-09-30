import { invoke } from "@tauri-apps/api/core";
import { showToast } from "./toast";

/** Choices for a new `claude` session; "" means "use the defaults". */
export interface SessionOptions {
  model: string;
  effort: string;
  mode: string;
}

/** Value/label pairs, in the order the dropdowns show them. */
export const MODEL_CHOICES: [string, string][] = [["fable", "Fable"], ["opus", "Opus"], ["sonnet", "Sonnet"], ["haiku", "Haiku"]];
export const EFFORT_CHOICES: [string, string][] = ["low", "medium", "high", "xhigh", "max"].map((v) => [v, v]);
export const MODE_CHOICES: [string, string][] = ["manual", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"].map((v) => [v, v]);

export const OPTION_FIELDS: { name: keyof SessionOptions; label: string; choices: [string, string][] }[] = [
  { name: "model", label: "Model", choices: MODEL_CHOICES },
  { name: "effort", label: "Effort", choices: EFFORT_CHOICES },
  { name: "mode", label: "Mode", choices: MODE_CHOICES },
];

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
}

export interface NewSessionHandlers {
  onStart(machine: string, dir: string | null, prompt: string, options: SessionOptions): void;
  onClose(): void;
  onOpenSettings(): void;
  onMachine(machine: string): void;
}

interface StartResult {
  dir: string;
  how: "chosen" | "classifier" | "fallback";
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
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

  if (m.needsSetup) {
    const setup = el("div", "modal__setup");
    setup.append(el("p", "", "Set a projects directory in Settings first. Its subfolders become the choices here."));
    const open = el("button", "card__btn card__btn--primary", "Open Settings");
    open.type = "button";
    open.dataset.action = "open-settings";
    open.addEventListener("click", () => h.onOpenSettings());
    setup.append(open);
    panel.append(setup);
    root.append(backdrop, panel);
    return root;
  }

  const form = el("div", "newsession");

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

  const remoteMachine = m.machine !== "";
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

  const promptLabel = el("label", "newsession__field");
  promptLabel.append(el("span", "newsession__label", "Prompt"));
  const ta = el("textarea", "modal__input");
  ta.name = "prompt";
  ta.rows = 5;
  ta.placeholder = "What should this session do? (⌘↵ to start)";
  ta.value = m.prompt;
  promptLabel.append(ta);

  const optionRow = el("div", "newsession__options");
  const optionSelects: [keyof SessionOptions, HTMLSelectElement][] = [];
  for (const f of OPTION_FIELDS) {
    const field = renderChoice(f.name, f.label, f.choices, m.options[f.name] ?? "");
    optionSelects.push([f.name, field.querySelector("select")!]);
    optionRow.append(field);
  }
  const readOptions = (): SessionOptions => {
    const o: SessionOptions = { model: "", effort: "", mode: "" };
    for (const [name, sel] of optionSelects) o[name] = sel.value;
    return o;
  };

  const start = el("button", "card__btn card__btn--primary", m.busy ? "Starting…" : "Start");
  start.type = "button";
  start.dataset.action = "start";
  const sync = () => {
    start.disabled = m.busy || ta.value.trim() === "";
  };
  const tryStart = () => {
    if (start.disabled) return;
    const prompt = ta.value.trim();
    if (!prompt) return;
    h.onStart(m.machine, select.value || null, prompt, readOptions());
  };
  ta.addEventListener("input", sync);
  ta.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" && (ev.metaKey || ev.ctrlKey)) {
      ev.preventDefault();
      tryStart();
    }
  });
  start.addEventListener("click", tryStart);
  sync();

  const actions = el("div", "newsession__actions");
  actions.append(start);
  form.append(dirLabel, optionRow, promptLabel, actions);
  panel.append(form);
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.ok ? "ok" : "error"}`, m.status.text));

  root.append(backdrop, panel);
  return root;
}

let current: { model: NewSessionModel; keyHandler: (e: KeyboardEvent) => void } | null = null;
let draft = "";
/** The last chosen options; unlike the prompt they are kept after a start. */
let lastOptions: SessionOptions = { model: "", effort: "", mode: "" };

function readOptionsFrom(host: ParentNode): SessionOptions | null {
  const sel = (name: string) => host.querySelector<HTMLSelectElement>(`select[name=${name}]`);
  const model = sel("model");
  const effort = sel("effort");
  const mode = sel("mode");
  if (!model || !effort || !mode) return null;
  return { model: model.value, effort: effort.value, mode: mode.value };
}

function startedText(r: StartResult): string {
  const name = r.dir.split("/").filter(Boolean).pop() ?? r.dir;
  if (r.how === "chosen") return `Started in ${name}`;
  if (r.how === "classifier") return `Started in ${name} (chosen by Claude)`;
  return `Started in ${name} (no clear match, Claude will work it out)`;
}

function paint(): void {
  const host = document.getElementById("modal-host");
  if (!host || !current) return;
  const m = current.model;
  const ta = host.querySelector<HTMLTextAreaElement>("textarea[name=prompt]");
  if (ta && !m.done) m.prompt = ta.value;
  const sel = host.querySelector<HTMLSelectElement>("select[name=dir]");
  if (sel) m.dir = sel.value || null;
  const opts = readOptionsFrom(host);
  if (opts) m.options = lastOptions = opts;
  host.replaceChildren(
    renderNewSession(m, {
      onStart: (machine, dir, prompt, options) => void start(machine, dir, prompt, options),
      onClose: closeNewSession,
      onOpenSettings: () => {
        closeNewSession();
        document.querySelector<HTMLElement>("[data-tab=settings]")?.click();
      },
      onMachine: (machine) => void chooseMachine(machine),
    }),
  );
  host.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")?.focus();
}

async function loadDirs(machine: string): Promise<void> {
  if (!current) return;
  const me = current;
  try {
    const dirs = await invoke<string[]>("list_project_dirs", { machine });
    if (current !== me || me.model.machine !== machine) return;
    me.model.dirs = dirs;
  } catch (e) {
    if (current !== me || me.model.machine !== machine) return;
    const msg = String(e);
    if (msg.includes("projects directory")) me.model.needsSetup = true;
    else me.model.status = { ok: false, text: msg };
  }
  paint();
}

async function chooseMachine(machine: string): Promise<void> {
  if (!current) return;
  current.model.machine = machine;
  current.model.dir = null;
  current.model.dirs = [];
  paint();
  await loadDirs(machine);
}

async function start(machine: string, dir: string | null, prompt: string, options: SessionOptions): Promise<void> {
  if (!current || current.model.busy) return;
  const me = current;
  me.model.busy = true;
  me.model.dir = dir;
  me.model.prompt = prompt;
  me.model.options = lastOptions = options;
  me.model.status = { ok: true, text: dir ? "Starting…" : "Choosing a repository…" };
  paint();
  try {
    const r = await invoke<StartResult>("start_session", { dir, prompt, options, machine });
    draft = "";
    if (current !== me) {
      showToast(startedText(r));
      return;
    }
    // The prompt is spent: clear it everywhere and keep Start disabled until the modal closes.
    me.model.done = true;
    me.model.busy = true;
    me.model.prompt = "";
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
    model: { dirs: [], dir: null, prompt: draft, options: { ...lastOptions }, status: null, busy: false, needsSetup: false, machines: [{ name: "This Mac", value: "" }], machine: "" },
    keyHandler,
  };
  document.addEventListener("keydown", keyHandler);
  paint();
  void invoke<{ name: string; hostname: string; platform: string; connected: boolean }[]>("list_machines")
    .then((machines) => {
      if (!current) return;
      current.model.machines = [{ name: "This Mac", value: "" }, ...machines.filter((m) => m.connected).map((m) => ({ name: m.name, value: m.name }))];
      paint();
    })
    .catch(() => undefined);
  await loadDirs("");
}

export function closeNewSession(): void {
  if (!current) return;
  const host = document.getElementById("modal-host");
  const ta = host?.querySelector<HTMLTextAreaElement>("textarea[name=prompt]");
  // Keep the draft even mid-start; only a successful start spends it.
  if (ta && !current.model.done) draft = ta.value;
  const opts = host ? readOptionsFrom(host) : null;
  if (opts) lastOptions = opts;
  document.removeEventListener("keydown", current.keyHandler);
  current = null;
  document.getElementById("modal-host")?.replaceChildren();
}
