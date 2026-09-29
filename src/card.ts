import { formatAge, projectName } from "./format";
import { OPEN_DELAY_MS, renderOptions } from "./options";
import { harnessBadge } from "./harness";
import { iconButton } from "./icons";
import { COMPACT_AT, compactButton, contextMeter, prButton, type Card } from "./types";

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
  if (card.context) root.append(contextMeter(card.context));

  const head = el("header", "card__head");
  head.append(el("span", "card__name", card.name), el("span", "card__age", formatAge(card.stateSince, nowMs)));
  const meta = el("div", "card__meta");
  meta.append(el("div", "card__project", projectName(card.cwd)), harnessBadge(card.harness, "card__harness"));
  root.append(head, meta);

  if (card.awaiting) {
    const a = el("div", "card__awaiting", card.awaiting.detail);
    a.dataset.kind = card.awaiting.kind;
    root.append(a);
  }
  const options = renderOptions(card, next, { descriptions: false, enabled: nowMs - card.stateSince >= OPEN_DELAY_MS });
  if (options) root.append(options);
  if (card.snippet) root.append(el("p", "card__snippet", card.snippet));

  const actions = el("div", "card__actions");
  const terminal = iconButton("terminal", "Open terminal");
  terminal.dataset.action = "terminal";
  const reply = iconButton("reply", "Reply", "card__btn card__btn--primary");
  reply.dataset.action = "reply";
  actions.append(terminal);
  if (card.pr) actions.append(prButton(card.pr));
  if (card.harness === "claude-code" && card.context && card.context.percent >= COMPACT_AT) actions.append(compactButton());
  actions.append(reply);
  root.append(actions);
  return root;
}
