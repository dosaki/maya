import { formatAge } from "./format";
import { iconButton, iconElement } from "./icons";
import { STATE_LABEL, prRef, type Card, type GitProvider, type ReviewMark, type ReviewPr, type ReviewState } from "./types";

/** Settings listens for this on `document`; `detail` names the group to open. */
export const OPEN_SETTINGS_GROUP = "maya:settings-group";

export type ReviewAction =
  | { kind: "open-pr" | "review"; forge: GitProvider; repo: string; number: number }
  | { kind: "terminal"; forge: GitProvider; repo: string; number: number; pid: number }
  | { kind: "git-settings" };

const repoKey = (p: ReviewPr) => `${p.forge}:${p.repo}`;

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

/** The dot's colour, and its label for the user's own review and for other reviewers'. */
const MARK: Record<ReviewMark, { modifier: string; mine: string; theirs: string }> = {
  approved: { modifier: "approved", mine: "You approved", theirs: "Approved" },
  commented: { modifier: "commented", mine: "You commented", theirs: "Commented" },
  changesRequested: { modifier: "changes", mine: "You requested changes", theirs: "Changes requested" },
};

/** The pane's sub-tabs: why a PR is on the list. */
export type ReviewTab = ReviewPr["reasons"][number];

export type StateFilter = "all" | "toReview" | ReviewMark;

export interface ReviewFilters {
  state: StateFilter;
  /** `<forge>:<repo>`, or null for every repository. */
  repo: string | null;
}

const SUB_TABS: { tab: ReviewTab; label: string }[] = [
  { tab: "review", label: "Review Requested" },
  { tab: "assigned", label: "Assignee" },
];

const STATE_OPTIONS: { value: StateFilter; label: string }[] = [
  { value: "all", label: "All States" },
  { value: "approved", label: "Approved" },
  { value: "commented", label: "Commented" },
  { value: "changesRequested", label: "Changes requested" },
  { value: "toReview", label: "To review" },
];

/** Review Requested opens on what is left to review; Assignee on everything. */
const DEFAULT_FILTERS: Record<ReviewTab, ReviewFilters> = { review: { state: "toReview", repo: null }, assigned: { state: "all", repo: null } };

const TAB_KEY = "maya.reviewsTab";

/** The review a sub-tab goes by: the user's own under Review Requested, the PR's under Assignee. */
export function markFor(pr: ReviewPr, tab: ReviewTab): ReviewMark | null {
  return tab === "assigned" ? pr.prReview : pr.myReview;
}

export function applyFilters(prs: ReviewPr[], f: ReviewFilters, tab: ReviewTab = "review"): ReviewPr[] {
  return prs.filter((p) => {
    const mark = markFor(p, tab);
    return (f.repo === null || repoKey(p) === f.repo) && (f.state === "all" || (f.state === "toReview" ? mark === null : mark === f.state));
  });
}

/** True when `pr` waits on the user from `tab`: a review to give, or an assigned PR others approved or sent back. */
function waitsOnUser(pr: ReviewPr, tab: ReviewTab): boolean {
  if (!pr.reasons.includes(tab)) return false;
  return tab === "review" ? pr.myReview === null : pr.prReview === "approved" || pr.prReview === "changesRequested";
}

export function tabCount(prs: ReviewPr[], tab: ReviewTab): number {
  return prs.filter((p) => waitsOnUser(p, tab)).length;
}

/** One option per repository in `prs`, sorted, named by the short name unless two share it. */
export function repoOptions(prs: ReviewPr[]): { value: string; label: string }[] {
  const keys = [...new Map(prs.map((p) => [repoKey(p), p])).values()];
  const short = (p: ReviewPr) => p.repo.split("/").pop() ?? p.repo;
  const label = (p: ReviewPr) => {
    if (keys.filter((o) => short(o) === short(p)).length === 1) return short(p);
    const clash = keys.some((o) => o.repo === p.repo && o.forge !== p.forge);
    return clash && p.forge === "gitlab" ? `${p.repo} (GitLab)` : p.repo;
  };
  return keys.map((p) => ({ value: repoKey(p), label: label(p) })).sort((a, b) => a.label.localeCompare(b.label) || a.value.localeCompare(b.value));
}

/** The PRs waiting on the user from either sub-tab, each counted once. */
export function waitingCount(prs: ReviewPr[]): number {
  return prs.filter((p) => waitsOnUser(p, "review") || waitsOnUser(p, "assigned")).length;
}

export function renderReviewCard(pr: ReviewPr, nowMs: number, related: Card | null = null, tab: ReviewTab = "review"): HTMLElement {
  const root = el("article", "pr");
  root.dataset.forge = pr.forge;
  root.dataset.repo = pr.repo;
  root.dataset.number = String(pr.number);
  const head = el("header", "pr__head");
  const name = el("span", "pr__name");
  const mark = markFor(pr, tab);
  if (mark) {
    const { modifier, mine, theirs } = MARK[mark];
    const label = tab === "assigned" ? (pr.prReviewers.length > 0 ? `${theirs} by ${pr.prReviewers.join(", ")}` : theirs) : mine;
    const dot = el("span", `pr__dot pr__dot--${modifier}`);
    dot.title = label;
    dot.setAttribute("role", "img");
    dot.setAttribute("aria-label", label);
    name.append(dot);
  }
  const forgeMark = iconElement(pr.forge, 12);
  forgeMark.classList.add("pr__forge");
  forgeMark.setAttribute("role", "img");
  forgeMark.setAttribute("aria-label", pr.forge === "gitlab" ? "GitLab" : "GitHub");
  name.append(forgeMark);
  name.append(el("span", "pr__repo", `${pr.repo.split("/").pop() ?? pr.repo} ${prRef(pr)}`));
  const chips = el("span", "pr__chips");
  if (pr.isDraft) chips.append(el("span", "pr__chip pr__chip--draft", "draft"));
  head.append(name, chips);
  root.append(head, el("div", "pr__title", pr.title));
  const updated = Date.parse(pr.updatedAt);
  root.append(el("div", "pr__meta", `${pr.author} · updated ${Number.isFinite(updated) ? formatAge(updated, nowMs) : "?"} ago · ${pr.repo}`));
  const actions = el("div", "card__actions");
  if (related?.machine) {
    // Its terminal is on another Mac: name the machine instead of a button.
    const where = el("span", "pr__machine", `on ${related.machine}`);
    where.title = `${related.name} · ${STATE_LABEL[related.state]} · runs on ${related.machine}`;
    actions.append(where);
  } else if (related) {
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
  review.title = "Open a terminal in the checkout (or a fresh clone) and review it with Maya's agent on the review prompt from Settings";
  actions.append(open, review);
  root.append(actions);
  return root;
}

/**
 * The cards of one sub-tab under its filters: the error from the last fetch
 * if it failed, then the cards or a line saying why there are none.
 */
export function renderReviews(
  state: ReviewState,
  nowMs: number,
  ctx: ReviewContext = { cards: [], clonesDir: null },
  tab: ReviewTab = "review",
  filters: ReviewFilters = { state: "all", repo: null },
): HTMLElement {
  const root = el("section", "reviews");
  if (state.off) {
    const off = el("div", "reviews__off", "Git integration is off. Turn on GitHub or GitLab in Settings → Git. ");
    const go = el("button", "card__btn", "Open Settings");
    go.type = "button";
    go.dataset.action = "git-settings";
    off.append(go);
    root.append(off);
    return root;
  }
  for (const e of state.errors) root.append(el("div", "reviews__error", e.message));
  if (state.prs.length === 0) {
    root.append(el("div", "reviews__empty", state.fetchedAt === null ? "Checking…" : "Nothing waiting on you."));
    return root;
  }
  const inTab = state.prs.filter((p) => p.reasons.includes(tab));
  const prs = applyFilters(inTab, filters, tab);
  if (prs.length === 0) {
    const defaults = DEFAULT_FILTERS[tab];
    const unfiltered = filters.state === defaults.state && filters.repo === defaults.repo;
    root.append(el("div", "reviews__empty", inTab.length === 0 ? "Nothing here." : unfiltered && tab === "review" ? "Nothing waiting on you." : "No pull requests match these filters."));
    return root;
  }
  const list = el("div", "reviews__list");
  for (const pr of prs) list.append(renderReviewCard(pr, nowMs, relatedSession(pr, ctx.cards, ctx.clonesDir), tab));
  root.append(list);
  return root;
}

function storedTab(): ReviewTab {
  try {
    return localStorage.getItem(TAB_KEY) === "assigned" ? "assigned" : "review";
  } catch {
    return "review";
  }
}

function fillSelect(sel: HTMLSelectElement, options: { value: string; label: string }[]): void {
  sel.replaceChildren(
    ...options.map(({ value, label }) => {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = label;
      return o;
    }),
  );
}

/**
 * The Pull Requests pane in `host`: the sub-tabs and the State and
 * Repository filters, built once so an open dropdown survives the repaints,
 * over the list that `update` redraws. Each sub-tab keeps its own filters
 * for the life of the window; only the sub-tab is remembered across starts.
 */
export function makeReviewsView(host: HTMLElement) {
  let tab = storedTab();
  const filters: Record<ReviewTab, ReviewFilters> = { review: { ...DEFAULT_FILTERS.review }, assigned: { ...DEFAULT_FILTERS.assigned } };
  let state: ReviewState = { prs: [], errors: [], off: false, fetchedAt: null };
  let nowMs = Date.now();
  let ctx: ReviewContext = { cards: [], clonesDir: null };
  let optionsKey: string | null = null;

  const bar = el("div", "reviews__bar");
  const tabs = el("div", "reviews__tabs");
  tabs.setAttribute("role", "tablist");
  const tabButtons = SUB_TABS.map(({ tab: t, label }) => {
    const b = el("button", "tab", label);
    b.type = "button";
    b.setAttribute("role", "tab");
    b.dataset.reviewTab = t;
    b.append(el("span", "tab__count"));
    b.addEventListener("click", () => {
      tab = t;
      try {
        localStorage.setItem(TAB_KEY, t);
      } catch {
        /* private mode: nothing to remember */
      }
      paint();
    });
    return b;
  });
  tabs.append(...tabButtons);
  const stateSel = el("select", "reviews__filter");
  stateSel.name = "state";
  stateSel.setAttribute("aria-label", "State");
  fillSelect(stateSel, STATE_OPTIONS);
  stateSel.addEventListener("change", () => {
    filters[tab].state = stateSel.value as StateFilter;
    paint();
  });
  const repoSel = el("select", "reviews__filter");
  repoSel.name = "repo";
  repoSel.setAttribute("aria-label", "Repository");
  repoSel.addEventListener("change", () => {
    filters[tab].repo = repoSel.value || null;
    paint();
  });
  bar.append(tabs, el("span", "tabs__spacer"), stateSel, repoSel);
  const body = el("div", "reviews__body");
  host.replaceChildren(bar, body);

  function paint(): void {
    const inTab = state.prs.filter((p) => p.reasons.includes(tab));
    const repos = repoOptions(inTab);
    const f = filters[tab];
    if (f.repo !== null && !repos.some((r) => r.value === f.repo)) f.repo = null;
    const key = `${tab}\n${repos.map((r) => `${r.value}=${r.label}`).join("\n")}`;
    if (key !== optionsKey) {
      fillSelect(repoSel, [{ value: "", label: "All Repositories" }, ...repos]);
      optionsKey = key;
    }
    repoSel.value = f.repo ?? "";
    stateSel.value = f.state;
    for (const b of tabButtons) {
      const t = b.dataset.reviewTab as ReviewTab;
      b.classList.toggle("tab--active", t === tab);
      b.setAttribute("aria-selected", String(t === tab));
      const n = tabCount(state.prs, t);
      b.querySelector(".tab__count")!.textContent = n > 0 ? String(n) : "";
    }
    body.replaceChildren(renderReviews(state, nowMs, ctx, tab, f));
  }

  return {
    update(next: ReviewState, now: number, context: ReviewContext = { cards: [], clonesDir: null }): void {
      state = next;
      nowMs = now;
      ctx = context;
      paint();
    },
  };
}

/** What a click in the Pull Requests pane means: one of the two buttons, or nothing. */
export function reviewActionFor(target: Element): ReviewAction | null {
  const btn = target.closest<HTMLElement>("button[data-action]");
  if (btn?.dataset.action === "git-settings") return { kind: "git-settings" };
  const card = target.closest<HTMLElement>(".pr");
  if (!btn || !card?.dataset.repo || !card.dataset.number) return null;
  const kind = btn.dataset.action;
  const forge: GitProvider = card.dataset.forge === "gitlab" ? "gitlab" : "github";
  const repo = card.dataset.repo;
  const number = Number(card.dataset.number);
  if (kind === "terminal") return { kind, forge, repo, number, pid: Number(btn.dataset.pid) };
  if (kind !== "open-pr" && kind !== "review") return null;
  return { kind, forge, repo, number };
}
