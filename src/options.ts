import type { Card } from "./types";

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
  return root;
}
