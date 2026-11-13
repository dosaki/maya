import { stopButton } from "./answer";
import { formatAge, projectName } from "./format";
import { OPEN_DELAY_MS, renderOptions } from "./options";
import { capabilitiesOf, harnessBadge } from "./harness";
import { iconButton, iconElement } from "./icons";
import { isLinux } from "./platform";
import { costText } from "./usage";
import { COMPACT_AT, compactButton, contextBand, formatTokens, prRef, remoteTitle, type Card, type ContextUsage, type PullRequest } from "./types";

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
  if (!capabilitiesOf(card.harness).close || (card.state !== "idle" && card.state !== "completed") || card.stale) return false;
  const linux = card.machine ? card.machinePlatform === "linux" : isLinux();
  return !linux || Boolean(card.terminal);
}

/** The context as a small ring and its percentage, red from `COMPACT_AT`; the tooltip says the tokens. */
function contextRing(ctx: ContextUsage): HTMLElement {
  const ring = el("span", "card__ctx", `${ctx.percent}%`);
  ring.dataset.band = contextBand(ctx.percent);
  ring.style.setProperty("--p", `${ctx.percent}%`);
  ring.title = `Context ${ctx.percent}% · ${formatTokens(ctx.used)} of ${formatTokens(ctx.window)}`;
  return ring;
}

/** The PR as a small pill, "#97" (or "!12" for a GitLab merge request); it still opens the PR. */
function prPill(pr: PullRequest): HTMLButtonElement {
  const b = el("button", "card__pr", prRef(pr));
  b.type = "button";
  b.dataset.action = "pr";
  b.setAttribute("aria-label", `${pr.forge === "gitlab" ? "MR" : "PR"} ${prRef(pr)}`);
  b.title = `${pr.state} · ${pr.url}`;
  return b;
}

/**
 * A session card: the head (name, age, Close), a line naming where it runs,
 * the ask while it waits on the user, the last message, and a footer of
 * facts (context, PR, cost) and plain icon buttons. A click anywhere else
 * on the card opens its chat, so there is no Reply button.
 */
export function renderCard(card: Card, nowMs: number, next = 0): HTMLElement {
  const root = el("article", `card card--${card.state}${card.stale ? " card--stale" : ""}`);
  root.dataset.sessionId = card.sessionId;
  root.dataset.pid = String(card.pid);
  root.tabIndex = 0;
  root.title = card.cwd;

  const head = el("header", "card__head");
  head.append(el("span", "card__name", card.name), el("span", "card__age", formatAge(card.stateSince, nowMs)));
  if (canClose(card)) {
    const close = el("button", "card__close");
    close.type = "button";
    close.dataset.action = "close";
    close.title = "Close: types /exit, then exit to close the terminal";
    close.setAttribute("aria-label", "Close");
    close.append(iconElement("close", 10));
    head.append(close);
  }

  // Separators are drawn by the stylesheet, so a part Maya does not know leaves no stray "·".
  const sub = el("div", "card__sub");
  sub.append(harnessBadge(card.harness, "card__harness"), el("span", "card__project", projectName(card.cwd)));
  if (card.machine) {
    const remote = el("span", "card__remote");
    remote.title = remoteTitle(card.machine, card.machineAddress, card.machinePlatform, card.terminal);
    remote.append(iconElement("remote", 11));
    sub.append(remote, el("span", "card__machine", card.machine));
  }
  if (card.branch) {
    const branch = el("span", "card__branch", card.branch);
    branch.title = card.branch;
    sub.append(branch);
  }
  if (card.model) {
    const model = el("span", "card__model", card.model);
    model.title = card.model;
    sub.append(model);
  }
  root.append(head, sub);

  if (card.awaiting) {
    const ask = el("div", "card__ask");
    const q = card.awaiting.questions[next];
    if (card.awaiting.questions.length > 1 && q) ask.append(el("div", "options__label", `Question ${next + 1} of ${card.awaiting.questions.length} · ${q.header}`));
    const question = el("div", "card__question", card.awaiting.detail);
    question.dataset.kind = card.awaiting.kind;
    ask.append(question);
    const options = renderOptions(card, next, { descriptions: false, enabled: nowMs - card.stateSince >= OPEN_DELAY_MS, numbered: true, label: false });
    if (options) ask.append(options);
    root.append(ask);
  }
  if (card.snippet) root.append(el("p", "card__snippet", card.snippet));

  const facts = el("div", "card__facts");
  if (card.context) facts.append(contextRing(card.context));
  if (card.pr) facts.append(prPill(card.pr));
  if (card.usage?.cost) facts.append(el("span", "card__cost", costText(card.usage.cost)));

  const actions = el("div", "card__actions");
  if (capabilitiesOf(card.harness).compact && card.context && card.context.percent >= COMPACT_AT) actions.append(compactButton("card__icon"));
  if (!card.machine && (!isLinux() || card.terminal)) {
    const label = isLinux() && card.terminal ? `Open the terminal attached to ${card.terminal}` : "Open terminal";
    const terminal = iconButton("terminal", label, "card__icon");
    terminal.dataset.action = "terminal";
    actions.append(terminal);
  }
  const stop = stopButton(card, nowMs, "card__icon", "icon");
  if (stop) actions.append(stop);

  const foot = el("footer", "card__foot");
  foot.append(facts, actions);
  root.append(foot);
  return root;
}
