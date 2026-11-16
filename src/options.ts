import { canStop } from "./harness";
import type { Card } from "./types";

/** Mirrors `answer::OPEN_DELAY_MS` in src-tauri/src/answer.rs: the picker must be on screen first. */
export const OPEN_DELAY_MS = 1000;

/**
 * Buttons for the current question of a prompt (a question, or a
 * permission or plan ask once its menu is known), or null when the card is
 * not waiting on one (or every question is answered).
 */
/**
 * `numbered` stacks the answers as rows, each led by its number (the card);
 * `label: false` leaves "Question N of M" to the caller, which shows it
 * above the question.
 */
/**
 * On a multi-select question the answers are boxes to tick, `ticked` the
 * ones ticked so far, and Send fires them together.
 */
export function renderOptions(card: Card, next: number, opts: { descriptions: boolean; enabled: boolean; numbered?: boolean; label?: boolean; ticked?: number[] }): HTMLElement | null {
  const aw = card.awaiting;
  if (card.state !== "awaiting" || !aw) return null;
  const q = aw.questions[next];
  if (!q) return null;

  const root = document.createElement("div");
  root.className = "options";
  if (aw.questions.length > 1 && opts.label !== false) {
    const label = document.createElement("div");
    label.className = "options__label";
    label.textContent = `Question ${next + 1} of ${aw.questions.length} · ${q.header}`;
    root.append(label);
  }
  const list = document.createElement("div");
  list.className = opts.numbered ? "options__list options--rows" : "options__list";
  q.options.forEach((o, i) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "options__btn";
    const on = q.multiSelect && (opts.ticked ?? []).includes(i);
    btn.dataset.action = q.multiSelect ? "tick" : "answer";
    btn.dataset.ask = String(card.stateSince);
    btn.dataset.q = String(next);
    btn.dataset.opt = String(i);
    btn.title = o.description;
    btn.disabled = !opts.enabled;
    if (q.multiSelect) {
      btn.classList.toggle("options__btn--on", on);
      btn.setAttribute("aria-pressed", String(on));
    }
    if (opts.descriptions && o.description) {
      const name = document.createElement("span");
      name.className = "options__name";
      name.textContent = o.label;
      const desc = document.createElement("span");
      desc.className = "options__desc";
      desc.textContent = o.description;
      btn.append(name, desc);
    } else if (opts.numbered) {
      const num = document.createElement("span");
      num.className = "options__num";
      num.textContent = String(i + 1);
      const text = document.createElement("span");
      text.className = "options__text";
      text.textContent = o.label;
      btn.append(num, text);
    } else {
      const text = document.createElement("span");
      text.className = "options__text";
      text.textContent = o.label;
      btn.append(text);
    }
    if (q.multiSelect) {
      const box = document.createElement("span");
      box.className = "options__box";
      box.textContent = on ? "☑" : "☐";
      btn.querySelector(".options__name, .options__text")!.before(box);
    }
    list.append(btn);
  });
  root.append(list);
  if (q.multiSelect) {
    const line = document.createElement("div");
    line.className = "options__note options__submit";
    line.textContent = "Tick one or more, then send them.";
    const send = document.createElement("button");
    send.type = "button";
    send.className = "card__btn options__send";
    send.textContent = "Send";
    send.dataset.action = "send-ticks";
    send.dataset.ask = String(card.stateSince);
    send.dataset.q = String(next);
    send.disabled = !opts.enabled || (opts.ticked ?? []).length === 0;
    line.append(send);
    root.append(line);
  }
  if (aw.questions.length > 1) {
    // Maya cannot see answers given in the terminal mid-ask; its key
    // sequences assume the picker is on the question shown here.
    const warn = document.createElement("div");
    warn.className = "options__warn";
    warn.textContent = "Answer every question here or every question in the terminal, not both.";
    root.append(warn);
  }
  return root;
}

/**
 * The options ticked on each card's multi-select question, kept across
 * repaints of the board. A session's ticks hold for one question of one
 * ask: once the ask or the question moves on they are gone.
 */
export function makeTicks() {
  const bySession = new Map<string, { at: string; on: Set<number> }>();
  const at = (card: Card, question: number) => `${card.stateSince}:${question}`;
  const ticked = (card: Card, question: number): number[] => {
    const t = bySession.get(card.sessionId);
    return t && t.at === at(card, question) ? [...t.on].sort((a, b) => a - b) : [];
  };
  return {
    ticked,
    /** Ticks option `index`, or unticks it if ticked; returns what is ticked now. */
    toggle(card: Card, question: number, index: number): number[] {
      let t = bySession.get(card.sessionId);
      if (!t || t.at !== at(card, question)) {
        t = { at: at(card, question), on: new Set() };
        bySession.set(card.sessionId, t);
      }
      if (!t.on.delete(index)) t.on.add(index);
      return ticked(card, question);
    },
    clear(card: Card): void {
      bySession.delete(card.sessionId);
    },
  };
}

/** The board's ticks, read by the cards and changed by their clicks. */
export const cardTicks = makeTicks();

/**
 * Ticks or unticks option `index` of the card's question, from a click on
 * `target` (in its box), and shows it on the box and its Send at once: the
 * board does not repaint while the pointer is moving over it.
 */
export function tickOnCard(card: Card, question: number, index: number, target: Element): void {
  const ticked = cardTicks.toggle(card, question, index);
  const btn = target.closest<HTMLElement>("[data-action=tick]");
  if (btn) {
    const on = ticked.includes(index);
    btn.classList.toggle("options__btn--on", on);
    btn.setAttribute("aria-pressed", String(on));
    const box = btn.querySelector(".options__box");
    if (box) box.textContent = on ? "☑" : "☐";
  }
  const send = target.closest(".options")?.querySelector<HTMLButtonElement>("button[data-action=send-ticks]");
  if (send) send.disabled = ticked.length === 0;
}

/**
 * Milliseconds until the earliest question or Stop button that is still
 * inside the open delay becomes usable, or null when nothing is waiting on that.
 */
export function nextEnableDelay(cards: Card[], nowMs: number): number | null {
  let best: number | null = null;
  for (const c of cards) {
    const asking = c.state === "awaiting" && !!c.awaiting && c.awaiting.questions.length > 0;
    if (!asking && !canStop(c)) continue;
    const remaining = c.stateSince + OPEN_DELAY_MS - nowMs;
    if (remaining > 0 && (best === null || remaining < best)) best = remaining;
  }
  return best;
}
