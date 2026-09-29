import { formatAge } from "./format";
import { iconButton } from "./icons";
import { STATE_LABEL, type Card, type ReviewPr, type ReviewState } from "./types";

export type ReviewAction = { kind: "open-pr" | "review"; repo: string; number: number } | { kind: "terminal"; repo: string; number: number; pid: number };

/** What the pane needs beyond the PR list to link PRs to live sessions. */
export interface ReviewContext {
  cards: Card[];
  /** The clones directory with `~` expanded, or null when unknown. */
  clonesDir: string | null;
}

/**
 * The live session working on `pr`, if any: one started from the Review
 * button (named `review <repo> #<n>`), one whose branch has this PR, or one
 * inside the PR's clone folder. The most recently active match wins.
 */
export function relatedSession(pr: ReviewPr, cards: Card[], clonesDir: string | null): Card | null {
  const repoName = pr.repo.split("/").pop() ?? pr.repo;
  const name = `review ${repoName} #${pr.number}`;
  const cloneDir = clonesDir ? `${clonesDir.replace(/\/+$/, "")}/${repoName}-${pr.number}` : null;
  const matches = cards.filter(
    (c) => c.name === name || c.pr?.url === pr.url || (cloneDir !== null && (c.cwd === cloneDir || c.cwd.startsWith(`${cloneDir}/`))),
  );
  if (matches.length === 0) return null;
  return matches.sort((a, b) => b.stateSince - a.stateSince)[0];
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

const REASON_LABEL: Record<ReviewPr["reasons"][number], string> = { review: "review requested", assigned: "assigned" };

export function renderReviewCard(pr: ReviewPr, nowMs: number, related: Card | null = null): HTMLElement {
  const root = el("article", "pr");
  root.dataset.repo = pr.repo;
  root.dataset.number = String(pr.number);
  const head = el("header", "pr__head");
  head.append(el("span", "pr__repo", `${pr.repo.split("/").pop() ?? pr.repo} #${pr.number}`));
  const chips = el("span", "pr__chips");
  if (pr.isDraft) chips.append(el("span", "pr__chip pr__chip--draft", "draft"));
  for (const r of pr.reasons) chips.append(el("span", "pr__chip", REASON_LABEL[r] ?? r));
  head.append(chips);
  root.append(head, el("div", "pr__title", pr.title));
  const updated = Date.parse(pr.updatedAt);
  root.append(el("div", "pr__meta", `${pr.author} · updated ${Number.isFinite(updated) ? formatAge(updated, nowMs) : "?"} ago · ${pr.repo}`));
  const actions = el("div", "card__actions");
  if (related) {
    const term = iconButton("terminal", `Open terminal: ${related.name} · ${STATE_LABEL[related.state]}`);
    term.dataset.action = "terminal";
    term.dataset.pid = String(related.pid);
    actions.append(term);
  }
  const open = el("button", "card__btn", "Open");
  open.type = "button";
  open.dataset.action = "open-pr";
  open.title = pr.url;
  const review = el("button", "card__btn card__btn--primary", "Review");
  review.type = "button";
  review.dataset.action = "review";
  review.title = "Open a terminal in the checkout (or a fresh clone) and run /should-i-approve";
  actions.append(open, review);
  root.append(actions);
  return root;
}

export function renderReviews(state: ReviewState, nowMs: number, ctx: ReviewContext = { cards: [], clonesDir: null }): HTMLElement {
  const root = el("section", "reviews");
  if (state.error) root.append(el("div", "reviews__error", `GitHub check failed: ${state.error}`));
  if (state.prs.length === 0) {
    root.append(el("div", "reviews__empty", state.fetchedAt === null ? "Checking GitHub…" : "Nothing waiting on you."));
    return root;
  }
  const list = el("div", "reviews__list");
  for (const pr of state.prs) list.append(renderReviewCard(pr, nowMs, relatedSession(pr, ctx.cards, ctx.clonesDir)));
  root.append(list);
  return root;
}

/** What a click in the Pull Requests pane means: one of the two buttons, or nothing. */
export function reviewActionFor(target: Element): ReviewAction | null {
  const btn = target.closest<HTMLElement>("button[data-action]");
  const card = target.closest<HTMLElement>(".pr");
  if (!btn || !card?.dataset.repo || !card.dataset.number) return null;
  const kind = btn.dataset.action;
  const repo = card.dataset.repo;
  const number = Number(card.dataset.number);
  if (kind === "terminal") return { kind, repo, number, pid: Number(btn.dataset.pid) };
  if (kind !== "open-pr" && kind !== "review") return null;
  return { kind, repo, number };
}
