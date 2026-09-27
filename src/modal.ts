import { invoke } from "@tauri-apps/api/core";
import { OPEN_DELAY_MS } from "./card";
import { projectName } from "./format";
import { renderOptions } from "./options";
import type { Progress } from "./progress";
import { STATE_LABEL, type Card, type Turn } from "./types";

export interface ModalModel {
  card: Card;
  turns: Turn[];
  status: { ok: boolean; text: string } | null;
  draft: string;
  /** Index of the next unanswered question when the session is asking one. */
  next?: number;
}

export interface ModalHandlers {
  onSend(text: string): void;
  onTerminal(): void;
  onClose(): void;
  onAnswer(questionIndex: number, optionIndex: number): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

export function renderModal(m: ModalModel, h: ModalHandlers): HTMLElement {
  const root = el("div", "modal");
  const backdrop = el("div", "modal__backdrop");
  backdrop.addEventListener("click", () => h.onClose());
  const panel = el("section", "modal__panel");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");

  const head = el("header", "modal__head");
  const titles = el("div", "modal__titles");
  titles.append(el("h2", "modal__title", m.card.name), el("span", "modal__project", projectName(m.card.cwd)));
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
    const options = renderOptions(m.card, m.next ?? 0, { descriptions: true, enabled: Date.now() - m.card.stateSince >= OPEN_DELAY_MS });
    banner.append(
      el("span", "", options ? "This session is asking a question. Pick an answer here or in its terminal." : "This session is waiting for a decision in its terminal. A reply will queue behind it."),
    );
    const open = el("button", "card__btn", "Open terminal");
    open.type = "button";
    open.addEventListener("click", () => h.onTerminal());
    banner.append(open);
    if (options) {
      options.addEventListener("click", (ev) => {
        const btn = (ev.target as HTMLElement).closest<HTMLElement>("button[data-action=answer]");
        if (btn && !(btn as HTMLButtonElement).disabled) h.onAnswer(Number(btn.dataset.q), Number(btn.dataset.opt));
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
    turn.append(el("div", "turn__text", t.text));
    history.append(turn);
  }
  panel.append(history);

  if (m.card.hasInbox) {
    const form = el("div", "modal__composer");
    const ta = el("textarea", "modal__input");
    ta.placeholder = "Message this session… (⌘↵ to send)";
    ta.value = m.draft;
    ta.rows = 3;
    const trySend = () => {
      const text = ta.value.trim();
      if (text) h.onSend(text);
    };
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
  swap(".modal__state");
  swap(".modal__banner", ".modal__history");
  swap(".modal__history");
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

let current: { model: ModalModel; keyHandler: (e: KeyboardEvent) => void } | null = null;
let progress: Progress | null = null;

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
  const fresh = renderModal(m, {
    onSend: (text) => void guardedSend(text),
    onTerminal: () => void invoke("focus_session", { pid: m.card.pid }).catch((e) => setStatus(false, String(e))),
    onClose: closeModal,
    onAnswer: (q, opt) => void answer(q, opt),
  });
  const existing = host.querySelector<HTMLElement>(".modal");
  const composerUnchanged = !!existing && !!existing.querySelector("textarea") === m.card.hasInbox;
  if (existing && composerUnchanged) patchModal(existing, fresh);
  else host.replaceChildren(fresh);
  const hist = host.querySelector(".modal__history");
  if (hist) hist.scrollTop = wasAtBottom ? hist.scrollHeight : oldScroll;
  if (opts.focusInput) host.querySelector<HTMLTextAreaElement>("textarea")?.focus();
}

async function answer(questionIndex: number, optionIndex: number): Promise<void> {
  if (!current) return;
  const me = current;
  const { card } = me.model;
  try {
    await invoke("answer_question", { sessionId: card.sessionId, questionIndex, optionIndex });
    if (current !== me) return;
    progress?.advance(card);
    me.model.status = { ok: true, text: "Answer sent to the terminal" };
    paint();
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
    await invoke("send_reply", { sessionId: card.sessionId, text });
    const ta = document.getElementById("modal-host")?.querySelector<HTMLTextAreaElement>("textarea");
    if (ta) ta.value = "";
    current.model.draft = "";
    current.model.status = { ok: true, text: "Delivered" };
    await loadTurns({ force: true, focusInput: true });
  } catch (e) {
    setStatus(false, String(e));
  }
});

/** Fetches history; repaints only when something visible changed (or `force`). */
async function loadTurns(opts: { force?: boolean; focusInput?: boolean } = {}): Promise<void> {
  if (!current) return;
  const { card } = current.model;
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
  current = { model: { card, turns: [], status: null, draft: "" }, keyHandler };
  document.addEventListener("keydown", keyHandler);
  paint({ focusInput: true });
  await loadTurns({ force: true, focusInput: true });
}

export function closeModal(): void {
  if (!current) return;
  document.removeEventListener("keydown", current.keyHandler);
  current = null;
  document.getElementById("modal-host")?.replaceChildren();
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
  const stateChanged = fresh.state !== current.model.card.state || fresh.hasInbox !== current.model.card.hasInbox;
  current.model.card = fresh;
  void loadTurns({ force: stateChanged });
}
