import { invoke } from "@tauri-apps/api/core";
import { showToast } from "./toast";

export interface NewSessionModel {
  dirs: string[];
  dir: string | null;
  prompt: string;
  status: { ok: boolean; text: string } | null;
  busy: boolean;
  needsSetup: boolean;
  /** Set once a start succeeded: the prompt is spent and must not come back as a draft. */
  done?: boolean;
}

export interface NewSessionHandlers {
  onStart(dir: string | null, prompt: string): void;
  onClose(): void;
  onOpenSettings(): void;
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
  const dirLabel = el("label", "newsession__field");
  dirLabel.append(el("span", "newsession__label", "Directory"));
  const select = el("select", "newsession__select");
  select.name = "dir";
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Let Claude choose";
  select.append(auto);
  for (const d of m.dirs) {
    const o = document.createElement("option");
    o.value = d;
    o.textContent = d;
    select.append(o);
  }
  select.value = m.dir ?? "";
  dirLabel.append(select);

  const promptLabel = el("label", "newsession__field");
  promptLabel.append(el("span", "newsession__label", "Prompt"));
  const ta = el("textarea", "modal__input");
  ta.name = "prompt";
  ta.rows = 5;
  ta.placeholder = "What should this session do? (⌘↵ to start)";
  ta.value = m.prompt;
  promptLabel.append(ta);

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
    h.onStart(select.value || null, prompt);
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
  form.append(dirLabel, promptLabel, actions);
  panel.append(form);
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.ok ? "ok" : "error"}`, m.status.text));

  root.append(backdrop, panel);
  return root;
}

let current: { model: NewSessionModel; keyHandler: (e: KeyboardEvent) => void } | null = null;
let draft = "";

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
  host.replaceChildren(
    renderNewSession(m, {
      onStart: (dir, prompt) => void start(dir, prompt),
      onClose: closeNewSession,
      onOpenSettings: () => {
        closeNewSession();
        document.getElementById("settings-toggle")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      },
    }),
  );
  host.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")?.focus();
}

async function start(dir: string | null, prompt: string): Promise<void> {
  if (!current || current.model.busy) return;
  const me = current;
  me.model.busy = true;
  me.model.dir = dir;
  me.model.prompt = prompt;
  me.model.status = { ok: true, text: dir ? "Starting…" : "Choosing a repository…" };
  paint();
  try {
    const r = await invoke<StartResult>("start_session", { dir, prompt });
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
  current = { model: { dirs: [], dir: null, prompt: draft, status: null, busy: false, needsSetup: false }, keyHandler };
  document.addEventListener("keydown", keyHandler);
  paint();
  try {
    const dirs = await invoke<string[]>("list_project_dirs");
    if (!current) return;
    current.model.dirs = dirs;
  } catch (e) {
    if (!current) return;
    const msg = String(e);
    if (msg.includes("projects directory")) current.model.needsSetup = true;
    else current.model.status = { ok: false, text: msg };
  }
  paint();
}

export function closeNewSession(): void {
  if (!current) return;
  const ta = document.getElementById("modal-host")?.querySelector<HTMLTextAreaElement>("textarea[name=prompt]");
  // Keep the draft even mid-start; only a successful start spends it.
  if (ta && !current.model.done) draft = ta.value;
  document.removeEventListener("keydown", current.keyHandler);
  current = null;
  document.getElementById("modal-host")?.replaceChildren();
}
