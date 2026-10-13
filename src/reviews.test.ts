import { describe, expect, it } from "vitest";
import { applyFilters, makeReviewsView, tabCount, relatedSession, renderReviews, repoOptions, reviewActionFor, waitingCount } from "./reviews";
import type { Card, ReviewPr } from "./types";

const NOW = Date.parse("2026-09-29T10:00:00Z");
const pr = (over: Partial<ReviewPr> = {}): ReviewPr => ({
  number: 451,
  repo: "Org/bedrock",
  title: "docs: expand upgrade notes",
  author: "jane",
  url: "https://github.com/Org/bedrock/pull/451",
  isDraft: false,
  updatedAt: "2026-09-29T08:00:00Z",
  reasons: ["review"],
  myReview: null,
  prReview: null,
  prReviewers: [],
  ...over,
});

const session = (over: Partial<Card>): Card => ({
  sessionId: "s", pid: 1, name: "n", cwd: "/Users/x/dev/other", state: "working", stateSince: 100, snippet: "", awaiting: null,
  hasInbox: true, harness: "claude-code", pr: null, context: null, ...over,
});

describe("relatedSession", () => {
  it("matches by review session name, by the session's PR, or by the clone folder, newest first", () => {
    const named = session({ sessionId: "a", name: "review bedrock #451", stateSince: 100 });
    const byPr = session({ sessionId: "b", name: "bedrock-d5", pr: { number: 451, url: "https://github.com/Org/bedrock/pull/451", state: "open" }, stateSince: 300 });
    const byDir = session({ sessionId: "c", name: "x", cwd: "/Users/x/dev/reviews/bedrock-451", stateSince: 200 });
    const other = session({ sessionId: "d", name: "review bedrock #450", pr: { number: 450, url: "https://github.com/Org/bedrock/pull/450", state: "open" } });
    expect(relatedSession(pr(), [named], "/Users/x/dev/reviews")?.sessionId).toBe("a");
    expect(relatedSession(pr(), [byPr], "/Users/x/dev/reviews")?.sessionId).toBe("b");
    expect(relatedSession(pr(), [byDir], "/Users/x/dev/reviews")?.sessionId).toBe("c");
    expect(relatedSession(pr(), [other], "/Users/x/dev/reviews")).toBeNull();
    expect(relatedSession(pr(), [named, byDir, byPr], "/Users/x/dev/reviews")?.sessionId).toBe("b");
    // A clone folder for a different PR of the same repo does not match.
    expect(relatedSession(pr(), [session({ cwd: "/Users/x/dev/reviews/bedrock-4510" })], "/Users/x/dev/reviews")).toBeNull();
    // Without a clones directory the folder rule is skipped.
    expect(relatedSession(pr(), [byDir], null)).toBeNull();
  });
});

describe("renderReviews", () => {
  it("renders a card per PR with repo, number, title, author, age and a draft chip", () => {
    const el = renderReviews({ prs: [pr(), pr({ number: 3, repo: "Org/plugins", isDraft: true, updatedAt: "2026-09-28T10:00:00Z" })], error: null, fetchedAt: NOW }, NOW);
    const cards = el.querySelectorAll(".pr");
    expect(cards.length).toBe(2);
    const first = cards[0] as HTMLElement;
    expect(first.dataset.repo).toBe("Org/bedrock");
    expect(first.dataset.number).toBe("451");
    expect(first.querySelector(".pr__repo")?.textContent).toBe("bedrock #451");
    expect(first.querySelector(".pr__title")?.textContent).toBe("docs: expand upgrade notes");
    expect(first.querySelector(".pr__meta")?.textContent).toContain("jane");
    expect(first.querySelector(".pr__meta")?.textContent).toContain("2h");
    expect(first.querySelectorAll(".pr__chip").length).toBe(0);
    expect([...cards[1].querySelectorAll(".pr__chip")].map((c) => c.textContent)).toEqual(["draft"]);
    expect(first.querySelector("button[data-action=open-pr]")?.textContent).toBe("Open");
    expect(first.querySelector("button[data-action=review]")?.textContent).toBe("Review");
  });

  it("marks the user's own review with a coloured dot, and none when not reviewed", () => {
    const dot = (myReview: ReviewPr["myReview"]) => renderReviews({ prs: [pr({ myReview })], error: null, fetchedAt: NOW }, NOW).querySelector<HTMLElement>(".pr__dot");
    expect(dot(null)).toBeNull();
    expect(dot("approved")?.className).toBe("pr__dot pr__dot--approved");
    expect(dot("approved")?.title).toBe("You approved");
    expect(dot("commented")?.className).toBe("pr__dot pr__dot--commented");
    expect(dot("commented")?.getAttribute("aria-label")).toBe("You commented");
    expect(dot("changesRequested")?.className).toBe("pr__dot pr__dot--changes");
    expect(dot("changesRequested")?.title).toBe("You requested changes");
  });

  it("shows a Terminal button when a session is related, with its name and state", () => {
    const live = [session({ sessionId: "a", pid: 42, name: "review bedrock #451", state: "awaiting" })];
    const el = renderReviews({ prs: [pr()], error: null, fetchedAt: NOW }, NOW, { cards: live, clonesDir: null });
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=terminal]")!;
    expect(btn.querySelector("svg.icon-terminal")).not.toBeNull();
    expect(btn.getAttribute("aria-label")).toContain("review bedrock #451");
    expect(btn.title).toContain("review bedrock #451");
    expect(btn.title).toContain("Awaiting Decision");
    expect(reviewActionFor(btn)).toEqual({ kind: "terminal", repo: "Org/bedrock", number: 451, pid: 42 });
    expect(renderReviews({ prs: [pr()], error: null, fetchedAt: NOW }, NOW).querySelector("button[data-action=terminal]")).toBeNull();
  });

  it("names the machine instead of a Terminal button when the related session is remote", () => {
    const remote = [session({ sessionId: "r", pid: 42, name: "review bedrock #451", state: "working", machine: "laptop" })];
    const el = renderReviews({ prs: [pr()], error: null, fetchedAt: NOW }, NOW, { cards: remote, clonesDir: null });
    expect(el.querySelector("button[data-action=terminal]")).toBeNull();
    expect(el.querySelector(".pr__machine")?.textContent).toBe("on laptop");
  });

  it("shows an empty state, and the error when the last fetch failed", () => {
    const empty = renderReviews({ prs: [], error: null, fetchedAt: NOW }, NOW);
    expect(empty.querySelector(".reviews__empty")?.textContent).toContain("Nothing waiting");
    const loading = renderReviews({ prs: [], error: null, fetchedAt: null }, NOW);
    expect(loading.querySelector(".reviews__empty")?.textContent).toContain("Checking GitHub");
    const failed = renderReviews({ prs: [pr()], error: "gh failed: no token", fetchedAt: NOW }, NOW);
    expect(failed.querySelector(".reviews__error")?.textContent).toContain("no token");
    expect(failed.querySelectorAll(".pr").length).toBe(1);
  });
});

describe("filters", () => {
  const prs = [
    pr({ number: 1, repo: "Org/bedrock" }),
    pr({ number: 2, repo: "Org/plugins", myReview: "approved" }),
    pr({ number: 3, repo: "Org/bedrock", myReview: "commented" }),
    pr({ number: 4, repo: "Org/plugins", myReview: "changesRequested" }),
  ];
  const numbers = (list: ReviewPr[]) => list.map((p) => p.number);

  it("filter by the user's review state and by repository", () => {
    expect(numbers(applyFilters(prs, { state: "all", repo: null }))).toEqual([1, 2, 3, 4]);
    expect(numbers(applyFilters(prs, { state: "toReview", repo: null }))).toEqual([1]);
    expect(numbers(applyFilters(prs, { state: "approved", repo: null }))).toEqual([2]);
    expect(numbers(applyFilters(prs, { state: "commented", repo: null }))).toEqual([3]);
    expect(numbers(applyFilters(prs, { state: "changesRequested", repo: null }))).toEqual([4]);
    expect(numbers(applyFilters(prs, { state: "all", repo: "Org/plugins" }))).toEqual([2, 4]);
    expect(numbers(applyFilters(prs, { state: "approved", repo: "Org/bedrock" }))).toEqual([]);
  });

  it("offer each repository once, sorted, by short name unless two share it", () => {
    expect(repoOptions(prs)).toEqual([{ value: "Org/bedrock", label: "bedrock" }, { value: "Org/plugins", label: "plugins" }]);
    expect(repoOptions([pr({ repo: "B/x" }), pr({ repo: "A/x" }), pr({ repo: "A/y" })])).toEqual([
      { value: "A/x", label: "A/x" },
      { value: "B/x", label: "B/x" },
      { value: "A/y", label: "y" },
    ]);
  });
});

describe("makeReviewsView", () => {
  const state = (prs: ReviewPr[]) => ({ prs, error: null, fetchedAt: NOW });
  const prs = [
    pr({ number: 1, repo: "Org/bedrock" }),
    pr({ number: 2, repo: "Org/plugins", myReview: "approved" }),
    pr({ number: 3, repo: "Org/assigned", reasons: ["assigned"], myReview: "commented", prReview: "approved", prReviewers: ["jane"] }),
    pr({ number: 4, repo: "Org/assigned", reasons: ["assigned"] }),
  ];
  const setup = () => {
    try { localStorage.clear(); } catch { /* none */ }
    const host = document.createElement("div");
    document.body.replaceChildren(host);
    const view = makeReviewsView(host);
    view.update(state(prs), NOW);
    return host;
  };
  const shown = (host: HTMLElement) => [...host.querySelectorAll<HTMLElement>(".pr")].map((c) => Number(c.dataset.number));
  const select = (host: HTMLElement, name: string) => host.querySelector<HTMLSelectElement>(`select[name=${name}]`)!;
  const choose = (sel: HTMLSelectElement, value: string) => {
    sel.value = value;
    sel.dispatchEvent(new Event("change", { bubbles: true }));
  };
  const subTab = (host: HTMLElement, tab: string) => host.querySelector<HTMLButtonElement>(`button[data-review-tab=${tab}]`)!;

  it("opens on Review Requested showing only what is left to review, with a count per sub-tab", () => {
    const host = setup();
    expect(subTab(host, "review").getAttribute("aria-selected")).toBe("true");
    expect(select(host, "state").value).toBe("toReview");
    expect(shown(host)).toEqual([1]);
    expect(subTab(host, "review").textContent).toBe("Review Requested1");
    expect(subTab(host, "assigned").textContent).toBe("Assignee1");
    subTab(host, "assigned").click();
    expect(host.querySelector<HTMLElement>(".pr[data-number='3'] .pr__dot")?.title).toBe("Approved by jane");
  });

  it("shows every state under Assignee, with repositories from that sub-tab only", () => {
    const host = setup();
    subTab(host, "assigned").click();
    expect(select(host, "state").value).toBe("all");
    expect(shown(host)).toEqual([3, 4]);
    expect([...select(host, "repo").options].map((o) => o.textContent)).toEqual(["All Repositories", "assigned"]);
  });

  it("keeps each sub-tab's own filters when switching", () => {
    const host = setup();
    choose(select(host, "state"), "all");
    choose(select(host, "repo"), "Org/plugins");
    expect(shown(host)).toEqual([2]);
    subTab(host, "assigned").click();
    expect(select(host, "repo").value).toBe("");
    subTab(host, "review").click();
    expect(select(host, "state").value).toBe("all");
    expect(select(host, "repo").value).toBe("Org/plugins");
    expect(shown(host)).toEqual([2]);
  });

  it("says when nothing matches the filters, and when the sub-tab is empty", () => {
    const host = setup();
    choose(select(host, "state"), "commented");
    expect(shown(host)).toEqual([]);
    expect(host.querySelector(".reviews__empty")?.textContent).toBe("No pull requests match these filters.");
    const lone = document.createElement("div");
    makeReviewsView(lone).update(state([pr()]), NOW);
    subTab(lone, "assigned").click();
    expect(lone.querySelector(".reviews__empty")?.textContent).toBe("Nothing here.");
  });

  it("keeps the toolbar elements across updates, so an open dropdown survives the repaint", () => {
    try { localStorage.clear(); } catch { /* none */ }
    const host = document.createElement("div");
    const view = makeReviewsView(host);
    view.update(state(prs), NOW);
    const sel = select(host, "state");
    const repo = select(host, "repo");
    view.update(state(prs), NOW + 10_000);
    expect(select(host, "state")).toBe(sel);
    expect(select(host, "repo")).toBe(repo);
    choose(select(host, "state"), "all");
    choose(repo, "Org/plugins");
    view.update(state(prs.filter((p) => p.repo !== "Org/plugins")), NOW);
    expect(repo.value).toBe("");
    expect(shown(host)).toEqual([1]);
  });

  it("remembers the sub-tab, not the filters", () => {
    const host = setup();
    subTab(host, "assigned").click();
    choose(select(host, "state"), "commented");
    const again = document.createElement("div");
    const view = makeReviewsView(again);
    view.update(state(prs), NOW);
    expect(subTab(again, "assigned").getAttribute("aria-selected")).toBe("true");
    expect(select(again, "state").value).toBe("all");
  });
});

describe("counts", () => {
  const prs = [
    pr({ number: 1 }),
    pr({ number: 2, myReview: "approved" }),
    pr({ number: 3, reasons: ["assigned"], prReview: "approved" }),
    pr({ number: 4, reasons: ["assigned"], prReview: "commented" }),
    pr({ number: 5, reasons: ["assigned"], prReview: "changesRequested" }),
    pr({ number: 6, reasons: ["assigned"] }),
    pr({ number: 7, reasons: ["review", "assigned"], prReview: "approved" }),
  ];

  it("count, per sub-tab, what waits on the user: reviews to give, and assigned PRs decided by others", () => {
    expect(tabCount(prs, "review")).toBe(2);
    expect(tabCount(prs, "assigned")).toBe(3);
  });

  it("count each PR once for the Pull Requests tab", () => {
    expect(waitingCount(prs)).toBe(4);
  });
});

describe("the Assignee sub-tab", () => {
  it("shows and filters by the PR's review state, naming the reviewers", () => {
    const assigned = [
      pr({ number: 3, reasons: ["assigned"], myReview: "commented", prReview: "approved", prReviewers: ["jane", "bob"] }),
      pr({ number: 4, reasons: ["assigned"], prReview: "changesRequested", prReviewers: ["al"] }),
      pr({ number: 5, reasons: ["assigned"] }),
    ];
    const el = renderReviews({ prs: assigned, error: null, fetchedAt: NOW }, NOW, undefined, "assigned");
    const dots = [...el.querySelectorAll<HTMLElement>(".pr")].map((c) => c.querySelector<HTMLElement>(".pr__dot")?.title ?? null);
    expect(dots).toEqual(["Approved by jane, bob", "Changes requested by al", null]);
    expect(applyFilters(assigned, { state: "approved", repo: null }, "assigned").map((p) => p.number)).toEqual([3]);
    expect(applyFilters(assigned, { state: "toReview", repo: null }, "assigned").map((p) => p.number)).toEqual([5]);
    expect(applyFilters(assigned, { state: "commented", repo: null }, "assigned").map((p) => p.number)).toEqual([]); // the user's own comment does not count here
  });
});

describe("reviewActionFor", () => {
  it("routes the two buttons with the PR's repo and number, and nothing else", () => {
    const el = renderReviews({ prs: [pr()], error: null, fetchedAt: NOW }, NOW);
    document.body.replaceChildren(el);
    expect(reviewActionFor(el.querySelector("button[data-action=open-pr]")!)).toEqual({ kind: "open-pr", repo: "Org/bedrock", number: 451 });
    expect(reviewActionFor(el.querySelector("button[data-action=review]")!)).toEqual({ kind: "review", repo: "Org/bedrock", number: 451 });
    expect(reviewActionFor(el.querySelector(".pr__title")!)).toBeNull();
    expect(reviewActionFor(document.body)).toBeNull();
  });
});
