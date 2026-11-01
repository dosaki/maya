import { renderCard } from "./card";
import { iconElement } from "./icons";
import { morph } from "./morph";
import { COLUMNS, type Card } from "./types";

/** `emptyText` fills a column with no cards: "No matches" while a search narrows the board. */
export function renderBoard(cards: Card[], nowMs: number, nextFor?: (c: Card) => number, emptyText = "Nothing here"): HTMLElement {
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
      resume.replaceChildren(iconElement("resume", 12));
      head.append(add, resume);
    }

    const list = document.createElement("div");
    list.className = "column__cards";
    if (inCol.length === 0) {
      const empty = document.createElement("div");
      empty.className = "column__empty";
      empty.textContent = emptyText;
      list.append(empty);
    }
    for (const c of inCol) list.append(renderCard(c, nowMs, nextFor?.(c) ?? 0));

    section.append(head, list);
    board.append(section);
  }
  return board;
}

/** The cards the board shows: Maya's own brain card only in debug mode. */
export function boardCards(cards: Card[], debug: boolean): Card[] {
  return debug ? cards : cards.filter((c) => !c.brain);
}

/** The cards that count (tab counts, search total): never the brain card. */
export function countable(cards: Card[]): Card[] {
  return cards.filter((c) => !c.brain);
}

/**
 * Shows `fresh` in `host`, patching the board already there in place: the
 * columns keep their nodes, so their scroll positions (and the phone's
 * sideways one), and each card that stays in its column keeps its node, so
 * the one under the pointer keeps its hover. A card that changes column is
 * built afresh: it is no longer under the pointer anyway.
 */
export function swapBoard(host: HTMLElement, fresh: HTMLElement): void {
  const old = host.querySelector<HTMLElement>(":scope > .board");
  if (old) morph(old, fresh);
  else host.replaceChildren(fresh);
}
