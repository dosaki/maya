import { renderCard } from "./card";
import { COLUMNS, type Card } from "./types";

export function renderBoard(cards: Card[], nowMs: number, nextFor?: (c: Card) => number): HTMLElement {
  const board = document.createElement("main");
  board.className = "board";
  for (const col of COLUMNS) {
    const inCol = cards.filter((c) => c.state === col.state).sort((a, b) => b.stateSince - a.stateSince);
    const section = document.createElement("section");
    section.className = `column column--${col.state}`;
    section.dataset.state = col.state;

    const head = document.createElement("header");
    head.className = "column__head";
    const title = document.createElement("h2");
    title.className = "column__title";
    title.textContent = col.title;
    const count = document.createElement("span");
    count.className = "column__count";
    count.textContent = String(inCol.length);
    head.append(title, count);
    if (col.state === "idle") {
      const add = document.createElement("button");
      add.type = "button";
      add.className = "column__add";
      add.dataset.action = "new-session";
      add.title = "New session";
      add.textContent = "+";
      const resume = document.createElement("button");
      resume.type = "button";
      resume.className = "column__add";
      resume.dataset.action = "resume-session";
      resume.title = "Resume a past session";
      resume.setAttribute("aria-label", "Resume a past session");
      // A bar and a play triangle, like a cassette deck's resume key.
      resume.innerHTML = '<svg class="icon-resume" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><rect x="2" y="3" width="2.5" height="10" rx="0.5"/><path d="M6.5 3 L14 8 L6.5 13 Z"/></svg>';
      head.append(add, resume);
    }

    const list = document.createElement("div");
    list.className = "column__cards";
    if (inCol.length === 0) {
      const empty = document.createElement("div");
      empty.className = "column__empty";
      empty.textContent = "Nothing here";
      list.append(empty);
    }
    for (const c of inCol) list.append(renderCard(c, nowMs, nextFor?.(c) ?? 0));

    section.append(head, list);
    board.append(section);
  }
  return board;
}

/**
 * Replaces the board inside `host` with `fresh`, carrying each column's
 * scroll position over so a repaint does not jump the lists back to the top.
 */
export function swapBoard(host: HTMLElement, fresh: HTMLElement): void {
  const scroll = new Map<string, number>();
  for (const col of host.querySelectorAll<HTMLElement>(".column")) {
    const list = col.querySelector<HTMLElement>(".column__cards");
    if (list && col.dataset.state) scroll.set(col.dataset.state, list.scrollTop);
  }
  host.replaceChildren(fresh);
  for (const col of fresh.querySelectorAll<HTMLElement>(".column")) {
    const list = col.querySelector<HTMLElement>(".column__cards");
    const top = col.dataset.state ? scroll.get(col.dataset.state) : undefined;
    if (list && top) list.scrollTop = top;
  }
}
