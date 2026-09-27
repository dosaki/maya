import { formatAge, projectName } from "./format";
import { OPEN_DELAY_MS, renderOptions } from "./options";
import type { Card } from "./types";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function renderCard(card: Card, nowMs: number, next = 0): HTMLElement {
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
  const options = renderOptions(card, next, { descriptions: false, enabled: nowMs - card.stateSince >= OPEN_DELAY_MS });
  if (options) root.append(options);
  if (card.snippet) root.append(el("p", "card__snippet", card.snippet));

  const actions = el("div", "card__actions");
  const terminal = el("button", "card__btn", "Terminal");
  terminal.type = "button";
  terminal.dataset.action = "terminal";
  const reply = el("button", "card__btn card__btn--primary", "Reply");
  reply.type = "button";
  reply.dataset.action = "reply";
  actions.append(terminal, reply);
  root.append(actions);
  return root;
}
