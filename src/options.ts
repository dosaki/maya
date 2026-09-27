import type { Card } from "./types";

/** Mirrors `answer::OPEN_DELAY_MS` in src-tauri/src/answer.rs: the picker must be on screen first. */
export const OPEN_DELAY_MS = 1000;

/**
 * Buttons for the current question of an AskUserQuestion prompt, or null
 * when the card is not waiting on a question (or every question is answered).
 */
export function renderOptions(card: Card, next: number, opts: { descriptions: boolean; enabled: boolean }): HTMLElement | null {
  const aw = card.awaiting;
  if (card.state !== "awaiting" || !aw || aw.kind !== "question") return null;
  const q = aw.questions[next];
  if (!q) return null;

  const root = document.createElement("div");
  root.className = "options";
  if (aw.questions.length > 1) {
    const label = document.createElement("div");
    label.className = "options__label";
    label.textContent = `Question ${next + 1} of ${aw.questions.length} · ${q.header}`;
    root.append(label);
  }
  const list = document.createElement("div");
  list.className = "options__list";
  q.options.forEach((o, i) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "options__btn";
    btn.dataset.action = "answer";
    btn.dataset.ask = String(card.stateSince);
    btn.dataset.q = String(next);
    btn.dataset.opt = String(i);
    btn.title = o.description;
    btn.disabled = !opts.enabled || q.multiSelect;
    if (opts.descriptions && o.description) {
      const name = document.createElement("span");
      name.className = "options__name";
      name.textContent = o.label;
      const desc = document.createElement("span");
      desc.className = "options__desc";
      desc.textContent = o.description;
      btn.append(name, desc);
    } else {
      btn.textContent = o.label;
    }
    list.append(btn);
  });
  root.append(list);
  if (q.multiSelect) {
    const note = document.createElement("div");
    note.className = "options__note";
    note.textContent = "Multi-select: answer in the terminal";
    root.append(note);
  }
  if (aw.questions.length > 1) {
    // Eye cannot see answers given in the terminal mid-ask; its key
    // sequences assume the picker is on the question shown here.
    const warn = document.createElement("div");
    warn.className = "options__warn";
    warn.textContent = "Answer every question here or every question in the terminal, not both.";
    root.append(warn);
  }
  return root;
}

/**
 * Milliseconds until the earliest question that is still inside the open
 * delay becomes answerable, or null when nothing is waiting on that.
 */
export function nextEnableDelay(cards: Card[], nowMs: number): number | null {
  let best: number | null = null;
  for (const c of cards) {
    if (c.state !== "awaiting" || c.awaiting?.kind !== "question" || c.awaiting.questions.length === 0) continue;
    const remaining = c.stateSince + OPEN_DELAY_MS - nowMs;
    if (remaining > 0 && (best === null || remaining < best)) best = remaining;
  }
  return best;
}
