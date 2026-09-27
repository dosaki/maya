import { renderCard } from "./card";
import { COLUMNS, type Card } from "./types";

export function renderBoard(cards: Card[], nowMs: number): HTMLElement {
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

    const list = document.createElement("div");
    list.className = "column__cards";
    if (inCol.length === 0) {
      const empty = document.createElement("div");
      empty.className = "column__empty";
      empty.textContent = "Nothing here";
      list.append(empty);
    }
    for (const c of inCol) list.append(renderCard(c, nowMs));

    section.append(head, list);
    board.append(section);
  }
  return board;
}
