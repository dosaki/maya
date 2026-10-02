import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { answerGuard } from "./answer";
import { composeMessage, makeAttachments, pastedFiles, renderChips, type Attachment } from "./attachments";
import { renderMarkdown } from "./markdown";
import { projectName } from "./format";
import { iconElement } from "./icons";
import { EFFORT_CHOICES, MODEL_CHOICES, renderChoice } from "./newsession";
import { OPEN_DELAY_MS, nextEnableDelay, renderOptions } from "./options";
import type { Progress } from "./progress";
import { harnessBadge } from "./harness";
import { COMPACT_AT, STATE_LABEL, compactButton, formatTokens, prButton, remoteTitle, type Card, type Turn } from "./types";
import { sendShortcut } from "./platform";

export interface ModalModel {
  card: Card;
  turns: Turn[];
  status: { ok: boolean; text: string } | null;
  draft: string;
  /** Index of the next unanswered question when the session is asking one. */
  next?: number;
  /** Files the next message will point the session at. */
  attachments?: Attachment[];
}

export interface ModalHandlers {
  onSend(text: string): void;
  onTerminal(): void;
  onClose(): void;
  onAnswer(questionIndex: number, optionIndex: number, button: HTMLElement): void;
  /** Change the running session's model or effort (typed as a slash command). */
  onSetOption(setting: "model" | "effort", value: string): void;
  /** Send Shift+Tab to the session to move it to its next permission mode. */
  onCycleMode(): void;
  /** Open the session's pull request in the browser. */
  onOpenPr(): void;
  /** Give the session a new name (typed as `/rename`). */
  onRename(name: string): void;
  /** Type a slash command from the composer into the session's terminal. */
  onCommand(text: string): void;
  /** Compact the session's context (typed as `/compact`). */
  onCompact(): void;
  /** Open an http(s) link from a rendered turn in the browser. */
  onOpenLink(url: string): void;
  /** Drop an attachment from the composer. */
  onRemoveAttachment?(path: string): void;
  /** Files pasted into the composer, to be saved and attached. */
  onPasteFiles?(files: { name: string; file: File }[]): void;
}

/**
 * The session name as a heading that turns into an input on click. Enter
 * commits a changed, non-blank name; Escape, or leaving the field, cancels.
 * Escape is stopped here so the modal's own Escape handler does not close it.
 */
function renderTitle(name: string, h: ModalHandlers, renamable: boolean): HTMLElement {
  const title = el("h2", "modal__title", name);
  if (!renamable) {
    title.classList.add("modal__title--fixed");
    return title;
  }
  title.title = "Click to rename this session";
  title.tabIndex = 0;
  const edit = () => {
    const input = document.createElement("input");
    input.type = "text";
    input.className = "modal__title-input";
    input.value = name;
    input.maxLength = 60;
    let done = false;
    const finish = (commit: boolean) => {
      if (done) return;
      done = true;
      const next = input.value.trim();
      input.replaceWith(title);
      if (commit && next && next !== name) h.onRename(next);
    };
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        finish(true);
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        finish(false);
      }
    });
    input.addEventListener("blur", () => finish(false));
    title.replaceWith(input);
    input.focus();
    input.select();
  };
  title.addEventListener("click", edit);
  title.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") edit();
  });
  return title;
}

/**
 * Model and effort pickers plus a Cycle mode button. The pickers start blank
 * so a background repaint never re-applies a choice; Apply sends each one
 * that was changed and clears them.
 */
function renderTweaks(h: ModalHandlers): HTMLElement {
  const row = el("div", "modal__tweaks");
  const model = renderChoice("model", "", MODEL_CHOICES, "", "Model");
  const effort = renderChoice("effort", "", EFFORT_CHOICES, "", "Effort");
  const selects: ["model" | "effort", HTMLSelectElement][] = [
    ["model", model.querySelector("select")!],
    ["effort", effort.querySelector("select")!],
  ];
  const apply = el("button", "card__btn", "Apply");
  apply.type = "button";
  apply.dataset.action = "apply";
  const sync = () => {
    apply.disabled = selects.every(([, sel]) => sel.value === "");
  };
  for (const [, sel] of selects) sel.addEventListener("change", sync);
  apply.addEventListener("click", () => {
    if (apply.disabled) return;
    for (const [name, sel] of selects) {
      if (sel.value) h.onSetOption(name, sel.value);
      sel.value = "";
    }
    sync();
  });
  sync();
  const cycle = el("button", "card__btn", "Cycle mode");
  cycle.type = "button";
  cycle.dataset.action = "cycle-mode";
  cycle.title = "Sends Shift+Tab to the terminal: the next permission mode. Check the terminal to see which.";
  cycle.addEventListener("click", () => h.onCycleMode());
  row.append(model, effort, apply, cycle);
  return row;
}

/** Where the panel size the user dragged is kept, across opens and restarts. */
export const PANEL_SIZE_KEY = "maya.modal.size";
/** The smallest panel the grip allows (the stylesheet's min-width and min-height). */
export const PANEL_MIN = { width: 480, height: 360 };

export interface PanelSize {
  width: number;
  height: number;
}

/** A size worth keeping: two finite numbers, at least `PANEL_MIN`. */
export function panelSize(width: unknown, height: unknown): PanelSize | null {
  const w = Number(width);
  const h = Number(height);
  if (!Number.isFinite(w) || !Number.isFinite(h) || w < PANEL_MIN.width || h < PANEL_MIN.height) return null;
  return { width: Math.round(w), height: Math.round(h) };
}

/** Parses a stored size; anything odd is ignored. */
export function parsePanelSize(raw: string | null): PanelSize | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as { width?: unknown; height?: unknown } | null;
    return panelSize(v?.width, v?.height);
  } catch {
    return null;
  }
}

/**
 * The size the browser wrote inline on the panel while its grip was
 * dragged. The rendered size is not used: the stylesheet clamps it to the
 * window, and a smaller window must not overwrite the chosen size.
 */
export function draggedPanelSize(panel: HTMLElement): PanelSize | null {
  return panelSize(parseFloat(panel.style.width), parseFloat(panel.style.height));
}

function loadPanelSize(): PanelSize | null {
  try {
    return parsePanelSize(localStorage.getItem(PANEL_SIZE_KEY));
  } catch {
    return null;
  }
}

function savePanelSize(size: PanelSize): void {
  try {
    localStorage.setItem(PANEL_SIZE_KEY, JSON.stringify(size));
  } catch {
    // Storage may be off; the size then lasts for this open only.
  }
}

/** Gives the panel the size last dragged to, if any; the stylesheet clamps it to the window. */
export function applyPanelSize(panel: HTMLElement, size: PanelSize | null): void {
  if (!size) return;
  panel.style.width = `${size.width}px`;
  panel.style.height = `${size.height}px`;
}

/** The grip's reach from the panel's bottom-right corner, in pixels. */
export const GRIP_PX = 20;

/** True when a pointer at (x, y) is on the panel's resize grip. */
export function onGrip(rect: DOMRect, x: number, y: number): boolean {
  return x >= rect.right - GRIP_PX && y >= rect.bottom - GRIP_PX && x <= rect.right && y <= rect.bottom;
}

/**
 * Pins the panel where it is as the pointer reaches its grip. Centred, a drag of
 * the corner would move the panel's centre too, doubling the change and
 * running the grip away from the pointer. Pinned, only the dragged corner
 * moves; the next open is centred again at the size kept.
 */
export function anchorPanel(root: HTMLElement, panel: HTMLElement): void {
  if (root.classList.contains("modal--anchored")) return;
  const r = panel.getBoundingClientRect();
  const host = root.getBoundingClientRect();
  const left = Math.max(0, r.left - host.left);
  const top = Math.max(0, r.top - host.top);
  root.classList.add("modal--anchored");
  panel.style.left = `${left}px`;
  panel.style.top = `${top}px`;
  // The grip cannot go past the window's edges.
  panel.style.maxWidth = `calc(100vw - ${left + 8}px)`;
  panel.style.maxHeight = `calc(100vh - ${top + 8}px)`;
}

let panelObserver: ResizeObserver | null = null;

/** Remembers every size the grip is dragged to, for the panel on screen. */
function watchPanelSize(panel: HTMLElement | null): void {
  panelObserver?.disconnect();
  panelObserver = null;
  if (!panel || typeof ResizeObserver === "undefined") return;
  panelObserver = new ResizeObserver(() => {
    const size = draggedPanelSize(panel);
    if (size) savePanelSize(size);
  });
  panelObserver.observe(panel);
}

/** True when the composer holds a slash command: its first non-blank character is `/`. */
export function isSlashCommand(text: string): boolean {
  return text.trimStart().startsWith("/");
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

export function renderModal(m: ModalModel, h: ModalHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = el("div", "modal");
  const backdrop = el("div", "modal__backdrop");
  backdrop.addEventListener("click", () => h.onClose());
  const panel = el("section", "modal__panel");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");
  applyPanelSize(panel, loadPanelSize());
  // Pinned as the pointer reaches the grip, before the press: a change of
  // position during the press itself cancels WebKit's native resize.
  panel.addEventListener("pointermove", (ev) => {
    if (onGrip(panel.getBoundingClientRect(), ev.clientX, ev.clientY)) anchorPanel(root, panel);
  });

  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  const claude = m.card.harness === "claude-code";
  titles.append(renderTitle(m.card.name, h, claude));
  if (m.card.machine) {
    const remote = el("span", "card__remote");
    remote.title = remoteTitle(m.card.machine, m.card.machineAddress, m.card.machinePlatform, m.card.terminal);
    remote.append(iconElement("remote", 12));
    titles.append(remote);
  }
  titles.append(el("span", "modal__project", projectName(m.card.cwd)));
  titles.append(harnessBadge(m.card.harness, "modal__harness"));
  if (m.card.pr) {
    const pr = prButton(m.card.pr);
    pr.addEventListener("click", () => h.onOpenPr());
    titles.append(pr);
  }
  if (m.card.context) {
    const ctx = el("span", "modal__context", `ctx ${m.card.context.percent}%`);
    ctx.title = `Context ${m.card.context.percent}% · ${formatTokens(m.card.context.used)} of ${formatTokens(m.card.context.window)}`;
    titles.append(ctx);
    if (claude && m.card.context.percent >= COMPACT_AT) {
      const compact = compactButton();
      compact.addEventListener("click", () => h.onCompact());
      titles.append(compact);
    }
  }
  const state = el("span", `modal__state modal__state--${m.card.state}`, STATE_LABEL[m.card.state]);
  const close = el("button", "modal__close", "×");
  close.type = "button";
  close.dataset.action = "close";
  close.setAttribute("aria-label", "Close");
  close.addEventListener("click", () => h.onClose());
  head.append(titles, state, close);
  panel.append(head);

  if (m.card.state === "awaiting") {
    const banner = el("div", "modal__banner");
    const options = renderOptions(m.card, m.next ?? 0, { descriptions: true, enabled: nowMs - m.card.stateSince >= OPEN_DELAY_MS });
    const prose = m.card.awaiting?.kind === "text";
    const remote = !!m.card.machine;
    const text = options
      ? remote
        ? "This session is asking a question. Pick an answer here."
        : "This session is asking a question. Pick an answer here or in its terminal."
      : prose
        ? remote
          ? `This session asked you something. Reply below. "${m.card.awaiting?.detail ?? ""}"`
          : `This session asked you something. Reply below or in its terminal. "${m.card.awaiting?.detail ?? ""}"`
        : remote
          ? `This session is waiting for a decision on ${m.card.machine}. A reply will queue behind it.`
          : "This session is waiting for a decision in its terminal. A reply will queue behind it.";
    banner.append(el("span", "", text));
    if (!m.card.machine) {
      const open = el("button", "card__btn", "Open terminal");
      open.type = "button";
      open.addEventListener("click", () => h.onTerminal());
      banner.append(open);
    }
    if (options) {
      options.addEventListener("click", (ev) => {
        const btn = (ev.target as HTMLElement).closest<HTMLElement>("button[data-action=answer]");
        if (btn && !(btn as HTMLButtonElement).disabled) h.onAnswer(Number(btn.dataset.q), Number(btn.dataset.opt), btn);
      });
      banner.append(options);
    }
    panel.append(banner);
  }

  const history = el("div", "modal__history");
  if (m.turns.length === 0) history.append(el("div", "modal__empty", "No transcript found."));
  for (const t of m.turns) {
    const turn = el("div", `turn turn--${t.kind}`);
    const who = { user: "You", assistant: "Claude", peer: "Message", tool: "" }[t.kind];
    if (who) turn.append(el("div", "turn__who", who));
    const text = el("div", "turn__text");
    if (t.kind === "tool") text.textContent = t.text;
    else text.append(renderMarkdown(t.text));
    turn.append(text);
    history.append(turn);
  }
  history.addEventListener("click", (ev) => {
    const a = (ev.target as Element).closest("a[href]");
    if (!a) return;
    ev.preventDefault();
    h.onOpenLink(a.getAttribute("href") ?? "");
  });
  panel.append(history);
  if (claude) panel.append(renderTweaks(h));

  // Other harnesses have no inbox; a reply is typed into their terminal instead.
  if (m.card.hasInbox || !claude) {
    const attachments = m.attachments ?? [];
    panel.append(renderChips(attachments, (path) => h.onRemoveAttachment?.(path)));
    const form = el("div", "modal__composer");
    const ta = el("textarea", "modal__input");
    ta.placeholder = claude
      ? `Message this session… (${sendShortcut()} to send, paste or drop files to attach; a /command is typed into its terminal)`
      : `Message this session… (${sendShortcut()} to send; typed into its terminal as one line)`;
    ta.value = m.draft;
    ta.rows = 3;
    const trySend = () => {
      // A slash command is for Claude Code itself, not a message: it is
      // typed into the terminal. Other harnesses type every line anyway.
      if (claude && isSlashCommand(ta.value)) {
        h.onCommand(ta.value.trim());
        return;
      }
      const text = composeMessage(ta.value, attachments);
      if (text) h.onSend(text);
    };
    ta.addEventListener("paste", (ev) => {
      const items = ev.clipboardData?.items;
      if (!items) return;
      const files = pastedFiles(items, Date.now());
      if (files.length === 0) return;
      ev.preventDefault();
      h.onPasteFiles?.(files);
    });
    ta.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && (ev.metaKey || ev.ctrlKey)) {
        ev.preventDefault();
        trySend();
      }
    });
    const send = el("button", "card__btn card__btn--primary", "Send");
    send.type = "button";
    send.dataset.action = "send";
    send.addEventListener("click", trySend);
    form.append(ta, send);
    panel.append(form);
  } else {
    panel.append(el("div", "modal__noinbox", "This session has no inbox. Use the terminal."));
  }
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.ok ? "ok" : "error"}`, m.status.text));

  root.append(backdrop, panel);
  return root;
}

/**
 * Updates an already-rendered modal from a fresh render without touching the
 * composer, so focus, caret, selection and a half-typed draft survive
 * background refreshes. History, state badge, banner and status line are
 * transplanted from `fresh`.
 */
export function patchModal(root: HTMLElement, fresh: HTMLElement): void {
  const panel = root.querySelector(".modal__panel");
  const freshPanel = fresh.querySelector(".modal__panel");
  if (!panel || !freshPanel) return;

  const swap = (selector: string, before?: string) => {
    const old = panel.querySelector(selector);
    const next = freshPanel.querySelector(selector);
    if (old && next) old.replaceWith(next);
    else if (old && !next) old.remove();
    else if (!old && next) {
      const anchor = before ? panel.querySelector(before) : null;
      if (anchor) anchor.before(next);
      else panel.append(next);
    }
  };
  // A rename in progress keeps its input; otherwise the name follows the registry.
  if (!panel.querySelector(".modal__title-input")) swap(".modal__title");
  swap(".modal__state");
  swap(".modal__banner", ".modal__history");
  swap(".modal__history");
  swap(".modal__chips", ".modal__composer");
  swap(".modal__status");
}

/** True when both lists hold the same turns in the same order. */
export function sameTurns(a: Turn[], b: Turn[]): boolean {
  return a.length === b.length && a.every((t, i) => t.kind === b[i].kind && t.text === b[i].text);
}

/** Wraps an async send so that calls made while one is in flight are dropped. */
export function makeSendGuard(send: (text: string) => Promise<void>): (text: string) => Promise<void> {
  let inFlight = false;
  return async (text: string) => {
    if (inFlight) return;
    inFlight = true;
    try {
      await send(text);
    } finally {
      inFlight = false;
    }
  };
}

/** `lastRemoteFetchAt`: when a remote card's history was last requested (ms). */
let current: { model: ModalModel; keyHandler: (e: KeyboardEvent) => void; lastRemoteFetchAt: number } | null = null;
let progress: Progress | null = null;
const attachments = makeAttachments();
let unlistenDrop: (() => void) | null = null;

/** Saves pasted files through the backend and attaches the saved paths. */
async function attachPasted(files: { name: string; file: File }[]): Promise<void> {
  if (!current) return;
  const me = current;
  for (const { name, file } of files) {
    try {
      const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
      const path = await invoke<string>("save_attachment", { name, bytes });
      attachments.add({ name, path });
    } catch (e) {
      if (current === me) setStatus(false, String(e));
      return;
    }
  }
  if (current === me) paint();
}

function attachDropped(paths: string[]): void {
  if (!current) return;
  for (const p of paths) attachments.add({ name: p.split("/").filter(Boolean).pop() ?? p, path: p });
  paint();
}

async function watchDrops(): Promise<void> {
  if (unlistenDrop) return;
  try {
    unlistenDrop = await getCurrentWebview().onDragDropEvent((ev) => {
      if (ev.payload.type === "drop" && current) attachDropped(ev.payload.paths);
    });
  } catch {
    unlistenDrop = null;
  }
}
let enableTimer: ReturnType<typeof setTimeout> | undefined;

/** Share the board's question-progress tracker with the modal. */
export function setProgress(p: Progress): void {
  progress = p;
}

/**
 * Re-renders the open modal. Keeps the draft, keeps the history scroll
 * position unless it was at the bottom, and focuses the composer only when
 * asked (opening, or after a send), so a background refresh never steals
 * focus or a text selection.
 */
function paint(opts: { focusInput: boolean } = { focusInput: false }): void {
  const host = document.getElementById("modal-host");
  if (!host || !current) return;
  const m = current.model;
  const ta = host.querySelector<HTMLTextAreaElement>("textarea");
  if (ta) m.draft = ta.value;
  const oldHist = host.querySelector(".modal__history");
  const wasAtBottom = !oldHist || oldHist.scrollTop + oldHist.clientHeight >= oldHist.scrollHeight - 8;
  const oldScroll = oldHist?.scrollTop ?? 0;

  m.next = progress?.next(m.card) ?? 0;
  m.attachments = attachments.list();
  const fresh = renderModal(m, {
    onSend: (text) => void guardedSend(text),
    onTerminal: () => void invoke("focus_session", { pid: m.card.pid }).catch((e) => setStatus(false, String(e))),
    onClose: closeModal,
    onAnswer: (q, opt, btn) => void answer(q, opt, btn),
    onSetOption: (setting, value) => void setOption(setting, value),
    onCycleMode: () => void cycleMode(),
    onOpenPr: () => void invoke("open_pr", { sessionId: m.card.sessionId }).catch((e) => setStatus(false, String(e))),
    onRename: (name) => void rename(name),
    onCommand: (text) => void guardedCommand(text),
    onCompact: () => void invoke("compact_session", { sessionId: m.card.sessionId }).then(() => setStatus(true, "Sent /compact to the terminal")).catch((e) => setStatus(false, String(e))),
    onOpenLink: (url) => void invoke("open_url", { url }).catch((e) => setStatus(false, String(e))),
    onRemoveAttachment: (path) => {
      attachments.remove(path);
      paint();
    },
    onPasteFiles: (files) => void attachPasted(files),
  });
  // Buttons rendered inside the open delay: repaint once it has elapsed.
  const delay = nextEnableDelay([m.card], Date.now());
  if (enableTimer) clearTimeout(enableTimer);
  enableTimer = delay === null ? undefined : setTimeout(() => { enableTimer = undefined; paint(); }, delay + 50);
  const existing = host.querySelector<HTMLElement>(".modal");
  const composerUnchanged = !!existing && !!existing.querySelector("textarea") === (m.card.hasInbox || m.card.harness !== "claude-code");
  if (existing && composerUnchanged) patchModal(existing, fresh);
  else {
    host.replaceChildren(fresh);
    watchPanelSize(host.querySelector<HTMLElement>(".modal__panel"));
  }
  const hist = host.querySelector(".modal__history");
  if (hist) hist.scrollTop = wasAtBottom ? hist.scrollHeight : oldScroll;
  if (opts.focusInput) host.querySelector<HTMLTextAreaElement>("textarea")?.focus();
}

async function answer(questionIndex: number, optionIndex: number, button: HTMLElement): Promise<void> {
  if (!current) return;
  const me = current;
  const { card } = me.model;
  try {
    const outcome = await answerGuard.answer(card, questionIndex, optionIndex, button);
    if (current !== me || outcome === "dropped") return;
    progress?.advance(card);
    me.model.status = { ok: true, text: "Answer sent to the terminal" };
    paint();
  } catch (e) {
    if (current === me) setStatus(false, String(e));
  }
}

async function setOption(setting: "model" | "effort", value: string): Promise<void> {
  if (!current) return;
  const me = current;
  try {
    await invoke("set_session_option", { sessionId: me.model.card.sessionId, setting, value });
    if (current === me) setStatus(true, `Sent /${setting} ${value} to the terminal`);
  } catch (e) {
    if (current === me) setStatus(false, String(e));
  }
}

async function rename(name: string): Promise<void> {
  if (!current) return;
  const me = current;
  try {
    await invoke("rename_session", { sessionId: me.model.card.sessionId, name });
    if (current === me) setStatus(true, `Renamed to ${name}. The board will catch up shortly.`);
  } catch (e) {
    if (current === me) setStatus(false, String(e));
  }
}

async function cycleMode(): Promise<void> {
  if (!current) return;
  const me = current;
  try {
    await invoke("cycle_session_mode", { sessionId: me.model.card.sessionId });
    if (current === me) setStatus(true, "Sent Shift+Tab to the terminal; check which mode it shows now");
  } catch (e) {
    if (current === me) setStatus(false, String(e));
  }
}

function setStatus(ok: boolean, text: string): void {
  if (!current) return;
  current.model.status = { ok, text };
  paint();
}

const guardedSend = makeSendGuard(async (text: string) => {
  if (!current) return;
  const { card } = current.model;
  try {
    await invoke("send_reply", { sessionId: card.sessionId, text, attachments: current.model.attachments?.map((a) => a.path) ?? [] });
    const ta = document.getElementById("modal-host")?.querySelector<HTMLTextAreaElement>("textarea");
    if (ta) ta.value = "";
    current.model.draft = "";
    attachments.clear();
    current.model.status = { ok: true, text: "Delivered" };
    await loadTurns({ force: true, focusInput: true });
  } catch (e) {
    setStatus(false, String(e));
  }
});

/** Types a slash command into the terminal; attachments have nowhere to go with it. */
const guardedCommand = makeSendGuard(async (text: string) => {
  if (!current) return;
  const me = current;
  const { card } = me.model;
  if (attachments.list().length > 0) {
    setStatus(false, "Remove the attachments to send a command; they cannot go with it.");
    return;
  }
  try {
    await invoke("send_slash_command", { sessionId: card.sessionId, text });
    if (current !== me) return;
    const ta = document.getElementById("modal-host")?.querySelector<HTMLTextAreaElement>("textarea");
    if (ta) ta.value = "";
    me.model.draft = "";
    me.model.status = { ok: true, text: `Sent ${text} to the terminal` };
    await loadTurns({ force: true, focusInput: true });
  } catch (e) {
    if (current === me) setStatus(false, String(e));
  }
});

/** Fetches history; repaints only when something visible changed (or `force`). */
async function loadTurns(opts: { force?: boolean; focusInput?: boolean } = {}): Promise<void> {
  if (!current) return;
  const { card } = current.model;
  if (card.machine) current.lastRemoteFetchAt = Date.now();
  let turns: Turn[];
  try {
    turns = await invoke<Turn[]>("session_history", { sessionId: card.sessionId });
  } catch (e) {
    turns = [];
    current.model.status = { ok: false, text: String(e) };
    opts = { ...opts, force: true };
  }
  if (!current || current.model.card.sessionId !== card.sessionId) return;
  const changed = !sameTurns(current.model.turns, turns);
  current.model.turns = turns;
  if (changed || opts.force) paint({ focusInput: opts.focusInput ?? false });
}

export async function openModal(card: Card): Promise<void> {
  closeModal();
  const keyHandler = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeModal();
  };
  current = { model: { card, turns: [], status: null, draft: "" }, keyHandler, lastRemoteFetchAt: 0 };
  attachments.clear();
  document.addEventListener("keydown", keyHandler);
  void watchDrops();
  paint({ focusInput: true });
  await loadTurns({ force: true, focusInput: true });
}

export function closeModal(): void {
  if (!current) return;
  if (enableTimer) clearTimeout(enableTimer);
  enableTimer = undefined;
  document.removeEventListener("keydown", current.keyHandler);
  watchPanelSize(null);
  current = null;
  attachments.clear();
  document.getElementById("modal-host")?.replaceChildren();
}

/** How often a working remote card's history is refetched when nothing on the card moved. */
export const REMOTE_WORKING_REFETCH_MS = 3_000;

/**
 * Whether a board refresh should refetch the open card's history. A local
 * card's is a cheap file read, so always; a remote card's is a round trip
 * to its assistant (pushed about once a second while it works), so only
 * when its state, state time or snippet moved, or while it works (tool
 * calls change none of those) once `REMOTE_WORKING_REFETCH_MS` passed since
 * `lastRemoteFetchAt`.
 */
export function historyRefetchDue(before: Card, fresh: Card, lastRemoteFetchAt: number, now: number): boolean {
  if (!fresh.machine) return true;
  if (fresh.state !== before.state || fresh.stateSince !== before.stateSince || fresh.snippet !== before.snippet) return true;
  return fresh.state === "working" && now - lastRemoteFetchAt >= REMOTE_WORKING_REFETCH_MS;
}

/** Called on every board refresh: keeps the open modal's card and history current. */
export function refreshModal(cards: Card[]): void {
  if (!current) return;
  const id = current.model.card.sessionId;
  const fresh = cards.find((c) => c.sessionId === id);
  if (!fresh) {
    setStatus(false, "Session is no longer running.");
    return;
  }
  const stateChanged =
    fresh.state !== current.model.card.state || fresh.stateSince !== current.model.card.stateSince || fresh.hasInbox !== current.model.card.hasInbox;
  const refetch = historyRefetchDue(current.model.card, fresh, current.lastRemoteFetchAt, Date.now());
  current.model.card = fresh;
  if (refetch) void loadTurns({ force: stateChanged });
  else if (stateChanged) paint();
}
