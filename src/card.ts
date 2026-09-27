import { formatAge, projectName } from "./format";
import type { Card } from "./types";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function renderCard(card: Card, nowMs: number): HTMLElement {
  const root = el("article", `card card--${card.state}`);
  root.dataset.sessionId = card.sessionId;
  root.dataset.pid = String(card.pid);
  root.tabIndex = 0;
  root.title = card.cwd;

  const head = el("header", "card__head");
  head.append(el("span", "card__name", card.name), el("span", "card__age", formatAge(card.stateSince, nowMs)));
  root.append(head, el("div", "card__project", projectName(card.cwd)));

  if (card.awaiting) {
    const a = el("div", "card__awaiting", card.awaiting.detail);
    a.dataset.kind = card.awaiting.kind;
    root.append(a);
  }
  if (card.snippet) root.append(el("p", "card__snippet", card.snippet));
  return root;
}
