import { invoke } from "@tauri-apps/api/core";
import { formatAge } from "./format";
import { showToast } from "./toast";
import { thisComputer } from "./platform";

export interface ResumableSession {
  id: string;
  title: string;
  lastActiveMs: number;
  running: boolean;
}

/** A machine choice for the picker: "This Mac" first, then each connected assistant. */
export interface MachineChoice {
  name: string;
  value: string;
}

export interface ResumeModel {
  dirs: string[];
  dir: string | null;
  sessions: ResumableSession[];
  loading: boolean;
  status: { ok: boolean; text: string } | null;
  needsSetup: boolean;
  /** "This Mac" first, then each connected assistant. */
  machines: MachineChoice[];
  /** The chosen machine's value; "" is this Mac. */
  machine: string;
}

export interface ResumeHandlers {
  onDir(dir: string): void;
  onResume(dir: string, sessionId: string): void;
  onClose(): void;
  onOpenSettings(): void;
  onMachine(machine: string): void;
}

const DIR_KEY = "maya.resume.dir";

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

/** Step one: pick a directory. Step two: pick one of its sessions. */
export function renderResume(m: ResumeModel, h: ResumeHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = el("div", "modal");
  const backdrop = el("div", "modal__backdrop");
  backdrop.addEventListener("click", () => h.onClose());
  const panel = el("section", "modal__panel modal__panel--compact");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");

  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", "Resume session"));
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

  const dirLabel = el("label", "newsession__field");
  dirLabel.append(el("span", "newsession__label", "Directory"));
  const select = el("select", "newsession__select");
  select.name = "dir";
  const none = document.createElement("option");
  none.value = "";
  none.textContent = "Choose a directory…";
  select.append(none);
  for (const d of m.dirs) {
    const o = document.createElement("option");
    o.value = d;
    o.textContent = d;
    select.append(o);
  }
  select.value = m.dir ?? "";
  select.addEventListener("change", () => {
    if (select.value) h.onDir(select.value);
  });
  dirLabel.append(select);
  form.append(dirLabel);

  if (!m.dir) {
    form.append(el("p", "resume__hint", "Pick a directory to see its sessions."));
  } else if (m.loading) {
    form.append(el("p", "resume__hint", "Loading sessions…"));
  } else if (m.sessions.length === 0) {
    form.append(el("p", "resume__hint", "No sessions recorded for this directory."));
  } else {
    const list = el("div", "resume__list");
    for (const s of m.sessions) {
      const row = el("button", "resume__row");
      row.type = "button";
      row.dataset.action = "resume";
      row.dataset.id = s.id;
      row.disabled = s.running;
      row.title = s.id;
      const top = el("div", "resume__top");
      top.append(el("span", "resume__title", s.title));
      if (s.running) top.append(el("span", "resume__chip", "running"));
      row.append(top, el("div", "resume__age", `${formatAge(s.lastActiveMs, nowMs)} ago`));
      row.addEventListener("click", () => {
        if (!row.disabled && m.dir) h.onResume(m.dir, s.id);
      });
      list.append(row);
    }
    form.append(list);
  }
  panel.append(form);
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.ok ? "ok" : "error"}`, m.status.text));
  root.append(backdrop, panel);
  return root;
}

let current: { model: ResumeModel; keyHandler: (e: KeyboardEvent) => void } | null = null;

function rememberedDir(): string | null {
  try {
    return localStorage.getItem(DIR_KEY);
  } catch {
    return null;
  }
}

function paint(): void {
  const host = document.getElementById("modal-host");
  if (!host || !current) return;
  host.replaceChildren(
    renderResume(current.model, {
      onDir: (dir) => void loadSessions(dir),
      onResume: (dir, id) => void resume(dir, id),
      onClose: closeResume,
      onOpenSettings: () => {
        closeResume();
        document.querySelector<HTMLElement>("[data-tab=settings]")?.click();
      },
      onMachine: (machine) => void chooseMachine(machine),
    }),
  );
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
    const last = rememberedDir();
    if (machine === "" && last && dirs.includes(last)) {
      await loadSessions(last);
      return;
    }
  } catch (e) {
    if (current !== me || me.model.machine !== machine) return;
    const msg = String(e);
    if (machine === "" && msg.includes("projects directory")) me.model.needsSetup = true;
    else me.model.status = { ok: false, text: msg };
  }
  paint();
}

async function chooseMachine(machine: string): Promise<void> {
  if (!current) return;
  current.model.machine = machine;
  current.model.dir = null;
  current.model.dirs = [];
  // Setup belongs to the machine that needed it; `loadDirs` decides again for this one.
  current.model.needsSetup = false;
  current.model.sessions = [];
  paint();
  await loadDirs(machine);
}

async function loadSessions(dir: string): Promise<void> {
  if (!current) return;
  const me = current;
  const machine = me.model.machine;
  me.model.dir = dir;
  me.model.loading = true;
  me.model.status = null;
  if (machine === "") {
    try {
      localStorage.setItem(DIR_KEY, dir);
    } catch {
      /* nothing to remember */
    }
  }
  paint();
  try {
    const sessions = await invoke<ResumableSession[]>("list_resumable_sessions", { dir, machine });
    // A late answer for another folder or machine (the same folder name can exist on both) is dropped.
    if (current !== me || me.model.dir !== dir || me.model.machine !== machine) return;
    me.model.sessions = sessions;
  } catch (e) {
    if (current !== me || me.model.dir !== dir || me.model.machine !== machine) return;
    me.model.sessions = [];
    me.model.status = { ok: false, text: String(e) };
  }
  me.model.loading = false;
  paint();
}

async function resume(dir: string, sessionId: string): Promise<void> {
  if (!current) return;
  const me = current;
  const machine = me.model.machine;
  try {
    await invoke("resume_session", { dir, sessionId, machine });
    if (current === me) closeResume();
    showToast(`Resuming in ${dir}`);
  } catch (e) {
    if (current !== me) {
      showToast(String(e));
      return;
    }
    me.model.status = { ok: false, text: String(e) };
    paint();
  }
}

export async function openResume(): Promise<void> {
  closeResume();
  const keyHandler = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeResume();
  };
  current = {
    model: { dirs: [], dir: null, sessions: [], loading: false, status: null, needsSetup: false, machines: [{ name: thisComputer(), value: "" }], machine: "" },
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
  await loadDirs("");
}

export function closeResume(): void {
  if (!current) return;
  document.removeEventListener("keydown", current.keyHandler);
  current = null;
  document.getElementById("modal-host")?.replaceChildren();
}
