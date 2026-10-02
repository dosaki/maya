import { formatAge, projectName } from "./format";
import { OPEN_DELAY_MS, renderOptions } from "./options";
import { harnessBadge } from "./harness";
import { iconButton, iconElement } from "./icons";
import { isLinux } from "./platform";
import { COMPACT_AT, compactButton, contextMeter, prButton, remoteTitle, type Card } from "./types";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/**
 * Close types into the session's terminal, so it needs an idle or completed
 * Claude Code session whose terminal takes keys: on Linux, only one in tmux;
 * remote, only while its machine is connected.
 */
function canClose(card: Card): boolean {
  if (card.harness !== "claude-code" || (card.state !== "idle" && card.state !== "completed") || card.stale) return false;
  const linux = card.machine ? card.machinePlatform === "linux" : isLinux();
  return !linux || Boolean(card.terminal);
}

export function renderCard(card: Card, nowMs: number, next = 0): HTMLElement {
  const root = el("article", `card card--${card.state}${card.stale ? " card--stale" : ""}`);
  root.dataset.sessionId = card.sessionId;
  root.dataset.pid = String(card.pid);
  root.tabIndex = 0;
  root.title = card.cwd;
  if (card.context) root.append(contextMeter(card.context));

  const head = el("header", "card__head");
  head.append(el("span", "card__name", card.name));
  if (card.machine) {
    const remote = el("span", "card__remote");
    remote.title = remoteTitle(card.machine, card.machineAddress, card.machinePlatform, card.terminal);
    remote.append(iconElement("remote", 12));
    head.append(remote);
  }
  head.append(el("span", "card__age", formatAge(card.stateSince, nowMs)));
  if (canClose(card)) {
    const close = el("button", "card__close");
    close.type = "button";
    close.dataset.action = "close";
    close.title = "Close: types /exit, then exit to close the terminal";
    close.setAttribute("aria-label", "Close");
    close.append(iconElement("close", 10));
    head.append(close);
  }
  const meta = el("div", "card__meta");
  const project = card.machine ? `${projectName(card.cwd)} on ${card.machine}` : projectName(card.cwd);
  meta.append(el("div", "card__project", project), harnessBadge(card.harness, "card__harness"));
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
  const reply = iconButton("reply", "Reply", "card__btn card__btn--primary");
  reply.dataset.action = "reply";
  if (!card.machine && (!isLinux() || card.terminal)) {
    const label = isLinux() && card.terminal ? `Open the terminal attached to ${card.terminal}` : "Open terminal";
    const terminal = iconButton("terminal", label);
    terminal.dataset.action = "terminal";
    actions.append(terminal);
  }
  if (card.pr) actions.append(prButton(card.pr));
  if (card.harness === "claude-code" && card.context && card.context.percent >= COMPACT_AT) actions.append(compactButton());
  actions.append(reply);
  root.append(actions);
  return root;
}
