import { formatAge } from "./format";
import type { ReviewPr, ReviewState } from "./types";

export type ReviewAction = { kind: "open-pr" | "review"; repo: string; number: number };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

const REASON_LABEL: Record<ReviewPr["reasons"][number], string> = { review: "review requested", assigned: "assigned" };

export function renderReviewCard(pr: ReviewPr, nowMs: number): HTMLElement {
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

export function renderReviews(state: ReviewState, nowMs: number): HTMLElement {
  const root = el("section", "reviews");
  if (state.error) root.append(el("div", "reviews__error", `GitHub check failed: ${state.error}`));
  if (state.prs.length === 0) {
    root.append(el("div", "reviews__empty", state.fetchedAt === null ? "Checking GitHub…" : "Nothing waiting on you."));
    return root;
  }
  const list = el("div", "reviews__list");
  for (const pr of state.prs) list.append(renderReviewCard(pr, nowMs));
  root.append(list);
  return root;
}

/** What a click in the Pull Requests pane means: one of the two buttons, or nothing. */
export function reviewActionFor(target: Element): ReviewAction | null {
  const btn = target.closest<HTMLElement>("button[data-action]");
  const card = target.closest<HTMLElement>(".pr");
  if (!btn || !card?.dataset.repo || !card.dataset.number) return null;
  const kind = btn.dataset.action;
  if (kind !== "open-pr" && kind !== "review") return null;
  return { kind, repo: card.dataset.repo, number: Number(card.dataset.number) };
}
