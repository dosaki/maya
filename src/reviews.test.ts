import { describe, expect, it } from "vitest";
import { applyFilters, makeReviewsView, tabCount, relatedSession, renderReviews, repoOptions, reviewActionFor, waitingCount } from "./reviews";
import type { Card, ReviewPr } from "./types";

const NOW = Date.parse("2026-09-29T10:00:00Z");
const pr = (over: Partial<ReviewPr> = {}): ReviewPr => ({
  number: 451,
  forge: "github",
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

  it("tells a GitLab MR from a GitHub PR of the same number: by session name and by clone folder", () => {
    const mr = pr({ forge: "gitlab", repo: "grp/sub/bedrock", number: 12, url: "https://gitlab.acme.io/grp/sub/bedrock/-/merge_requests/12" });
    const ghPr = pr({ number: 12, url: "https://github.com/Org/bedrock/pull/12" });
    const ghNamed = session({ name: "review bedrock #12" });
    const glNamed = session({ name: "review bedrock !12" });
    expect(relatedSession(mr, [ghNamed], null)).toBeNull();
    expect(relatedSession(mr, [glNamed], null)).toBe(glNamed);
    expect(relatedSession(ghPr, [glNamed], null)).toBeNull();
    expect(relatedSession(ghPr, [ghNamed], null)).toBe(ghNamed);
    const ghDir = session({ cwd: "/Users/x/dev/reviews/bedrock-12" });
    const glDir = session({ cwd: "/Users/x/dev/reviews/bedrock-mr12/src" });
    expect(relatedSession(mr, [ghDir], "/Users/x/dev/reviews")).toBeNull();
    expect(relatedSession(mr, [glDir], "/Users/x/dev/reviews")).toBe(glDir);
    expect(relatedSession(ghPr, [glDir], "/Users/x/dev/reviews")).toBeNull();
  });
});

describe("renderReviews", () => {
  it("renders a card per PR with repo, number, title, author, age and a draft chip", () => {
    const el = renderReviews({ prs: [pr(), pr({ number: 3, repo: "Org/plugins", isDraft: true, updatedAt: "2026-09-28T10:00:00Z" })], errors: [], off: false, fetchedAt: NOW }, NOW);
    const cards = el.querySelectorAll(".pr");
    expect(cards.length).toBe(2);
    const first = cards[0] as HTMLElement;
    expect(first.dataset.repo).toBe("Org/bedrock");
    expect(first.dataset.number).toBe("451");
    expect(first.querySelector(".pr__repo")?.textContent).toBe("bedrock");
    expect(first.querySelector(".pr__number")?.textContent).toBe("#451");
    expect(first.querySelector(".pr__title")?.textContent).toBe("docs: expand upgrade notes");
    expect(first.querySelector(".pr__meta")?.textContent).toContain("jane");
    expect(first.querySelector(".pr__meta")?.textContent).toContain("2h");
    expect(first.querySelectorAll(".pr__chip").length).toBe(0);
    expect([...cards[1].querySelectorAll(".pr__chip")].map((c) => c.textContent)).toEqual(["draft"]);
    expect(first.querySelector("button[data-action=open-pr]")?.textContent).toBe("Open");
    expect(first.querySelector("button[data-action=review]")?.textContent).toBe("Review");
  });

  it("lays a PR out as a column: the head, the title and its meta, then the buttons", () => {
    const live = [session({ sessionId: "a", pid: 42, name: "review bedrock #451", state: "awaiting" })];
    const el = renderReviews({ prs: [pr({ isDraft: true })], errors: [], off: false, fetchedAt: NOW }, NOW, { cards: live, clonesDir: null });
    const row = el.querySelector<HTMLElement>(".reviews__list > .pr")!;
    expect([...row.children].map((c) => c.className)).toEqual(["pr__main", "card__actions pr__foot"]);
    expect([...row.querySelectorAll(".pr__main > *")].map((c) => c.className)).toEqual(["pr__head", "pr__title", "pr__meta"]);
    const head = row.querySelector(".pr__main > .pr__head")!;
    expect(head.querySelector(".pr__repo")?.textContent).toBe("bedrock");
    expect(head.querySelector(".pr__repo")?.getAttribute("title")).toBe("Org/bedrock");
    expect(head.querySelector(".pr__number")?.textContent).toBe("#451");
    expect(row.querySelector(".pr__main > .pr__title")?.textContent).toBe("docs: expand upgrade notes");
    expect(row.querySelector(".pr__main > .pr__meta")?.textContent).toBe("jane · updated 2h ago");
    expect([...head.querySelectorAll(".pr__marks > *")].map((c) => c.className)).toEqual(["pr__chip pr__chip--draft"]);
    const foot = [...row.querySelector(":scope > .pr__foot")!.children].map((c) => (c as HTMLElement).dataset.action ?? c.className);
    expect(foot).toEqual(["terminal", "open-pr", "review"]);
  });

  it("marks the user's own review with a coloured dot, and none when not reviewed", () => {
    const dot = (myReview: ReviewPr["myReview"]) => renderReviews({ prs: [pr({ myReview })], errors: [], off: false, fetchedAt: NOW }, NOW).querySelector<HTMLElement>(".pr__dot");
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
    const el = renderReviews({ prs: [pr()], errors: [], off: false, fetchedAt: NOW }, NOW, { cards: live, clonesDir: null });
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=terminal]")!;
    expect(btn.querySelector("svg.icon-terminal")).not.toBeNull();
    expect(btn.getAttribute("aria-label")).toContain("review bedrock #451");
    expect(btn.title).toContain("review bedrock #451");
    expect(btn.title).toContain("Awaiting Decision");
    expect(reviewActionFor(btn)).toEqual({ kind: "terminal", forge: "github", repo: "Org/bedrock", number: 451, pid: 42 });
    expect(renderReviews({ prs: [pr()], errors: [], off: false, fetchedAt: NOW }, NOW).querySelector("button[data-action=terminal]")).toBeNull();
  });

  it("names the machine instead of a Terminal button when the related session is remote", () => {
    const remote = [session({ sessionId: "r", pid: 42, name: "review bedrock #451", state: "working", machine: "laptop" })];
    const el = renderReviews({ prs: [pr()], errors: [], off: false, fetchedAt: NOW }, NOW, { cards: remote, clonesDir: null });
    expect(el.querySelector("button[data-action=terminal]")).toBeNull();
    expect(el.querySelector(".pr__machine")?.textContent).toBe("on laptop");
  });

  it("shows an empty state, and the error when the last fetch failed", () => {
    const empty = renderReviews({ prs: [], errors: [], off: false, fetchedAt: NOW }, NOW);
    expect(empty.querySelector(".reviews__empty")?.textContent).toContain("Nothing waiting");
    const loading = renderReviews({ prs: [], errors: [], off: false, fetchedAt: null }, NOW);
    expect(loading.querySelector(".reviews__empty")?.textContent).toContain("Checking…");
    const failed = renderReviews({ prs: [pr()], errors: [{ forge: "github", message: "GitHub check failed: no token" }], off: false, fetchedAt: NOW }, NOW);
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
    expect(numbers(applyFilters(prs, { state: "all", repo: "github:Org/plugins" }))).toEqual([2, 4]);
    expect(numbers(applyFilters(prs, { state: "approved", repo: "github:Org/bedrock" }))).toEqual([]);
  });

  it("offer each repository once, sorted, by short name unless two share it", () => {
    expect(repoOptions(prs)).toEqual([{ value: "github:Org/bedrock", label: "bedrock" }, { value: "github:Org/plugins", label: "plugins" }]);
    expect(repoOptions([pr({ repo: "B/x" }), pr({ repo: "A/x" }), pr({ repo: "A/y" })])).toEqual([
      { value: "github:A/x", label: "A/x" },
      { value: "github:B/x", label: "B/x" },
      { value: "github:A/y", label: "y" },
    ]);
  });
});

describe("makeReviewsView", () => {
  const state = (prs: ReviewPr[]) => ({ prs, errors: [], off: false, fetchedAt: NOW });
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
    choose(select(host, "repo"), "github:Org/plugins");
    expect(shown(host)).toEqual([2]);
    subTab(host, "assigned").click();
    expect(select(host, "repo").value).toBe("");
    subTab(host, "review").click();
    expect(select(host, "state").value).toBe("all");
    expect(select(host, "repo").value).toBe("github:Org/plugins");
    expect(shown(host)).toEqual([2]);
  });

  it("says when nothing matches the filters, and when the sub-tab is empty", () => {
    const host = setup();
    choose(select(host, "state"), "commented");
    expect(shown(host)).toEqual([]);
    expect(host.querySelector(".reviews__empty")?.textContent).toBe("No pull requests match these filters.");
    const done = document.createElement("div");
    makeReviewsView(done).update(state([pr({ myReview: "approved" })]), NOW);
    expect(done.querySelector(".reviews__empty")?.textContent).toBe("Nothing waiting on you.");
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
    choose(repo, "github:Org/plugins");
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
    const el = renderReviews({ prs: assigned, errors: [], off: false, fetchedAt: NOW }, NOW, undefined, "assigned");
    const dots = [...el.querySelectorAll<HTMLElement>(".pr")].map((c) => c.querySelector<HTMLElement>(".pr__dot")?.title ?? null);
    expect(dots).toEqual(["Approved by jane, bob", "Changes requested by al", null]);
    expect(applyFilters(assigned, { state: "approved", repo: null }, "assigned").map((p) => p.number)).toEqual([3]);
    expect(applyFilters(assigned, { state: "toReview", repo: null }, "assigned").map((p) => p.number)).toEqual([5]);
    expect(applyFilters(assigned, { state: "commented", repo: null }, "assigned").map((p) => p.number)).toEqual([]); // the user's own comment does not count here
  });
});

describe("forges", () => {
  const mr = (over: Partial<ReviewPr> = {}) => pr({ forge: "gitlab", repo: "grp/sub/bedrock", number: 12, url: "https://gitlab.acme.io/grp/sub/bedrock/-/merge_requests/12", ...over });

  it("marks each card with its forge and names a merge request with a bang", () => {
    const root = renderReviews({ prs: [mr(), pr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    const cards = [...root.querySelectorAll<HTMLElement>(".pr")];
    expect(cards.map((c) => `${c.querySelector(".pr__repo")?.textContent} ${c.querySelector(".pr__number")?.textContent}`)).toEqual(["bedrock !12", "bedrock #451"]);
    expect(cards.map((c) => c.dataset.forge)).toEqual(["gitlab", "github"]);
    expect(cards[0].querySelector(".pr__forge")?.getAttribute("aria-label")).toBe("GitLab");
    expect(cards[0].querySelector(".pr__forge")?.hasAttribute("aria-hidden")).toBe(false);
  });

  it("says each forge's error on its own line and still lists the PRs", () => {
    const root = renderReviews({ prs: [pr()], errors: [{ forge: "gitlab", message: "GitLab check failed: the token was refused (401)." }, { forge: "github", message: "No GitHub token: paste one in Settings → Git." }], off: false, fetchedAt: NOW }, NOW);
    expect([...root.querySelectorAll(".reviews__error")].map((e) => e.textContent)).toEqual(["GitLab check failed: the token was refused (401).", "No GitHub token: paste one in Settings → Git."]);
    expect(root.querySelectorAll(".pr")).toHaveLength(1);
  });

  it("says the integration is off, with a way to Settings → Git", () => {
    const root = renderReviews({ prs: [], errors: [], off: true, fetchedAt: NOW }, NOW);
    expect(root.querySelector(".reviews__off")?.textContent).toContain("Git integration is off. Turn on GitHub or GitLab in Settings → Git.");
    expect(reviewActionFor(root.querySelector("button[data-action=git-settings]")!)).toEqual({ kind: "git-settings" });
  });

  it("keeps the same path on both forges as two repository options", () => {
    const opts = repoOptions([pr({ repo: "acme/api" }), mr({ repo: "acme/api" })]);
    expect(opts.map((o) => o.value)).toEqual(["github:acme/api", "gitlab:acme/api"]);
    expect(applyFilters([pr({ repo: "acme/api" }), mr({ repo: "acme/api" })], { state: "all", repo: "gitlab:acme/api" }).map((p) => p.forge)).toEqual(["gitlab"]);
  });

  it("a click carries the forge", () => {
    const root = renderReviews({ prs: [mr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    expect(reviewActionFor(root.querySelector("button[data-action=review]")!)).toEqual({ kind: "review", forge: "gitlab", repo: "grp/sub/bedrock", number: 12 });
  });
});

describe("reviewActionFor", () => {
  it("routes the two buttons with the PR's repo and number, and nothing else", () => {
    const el = renderReviews({ prs: [pr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    document.body.replaceChildren(el);
    expect(reviewActionFor(el.querySelector("button[data-action=open-pr]")!)).toEqual({ kind: "open-pr", forge: "github", repo: "Org/bedrock", number: 451 });
    expect(reviewActionFor(el.querySelector("button[data-action=review]")!)).toEqual({ kind: "review", forge: "github", repo: "Org/bedrock", number: 451 });
    expect(reviewActionFor(el.querySelector(".pr__title")!)).toBeNull();
    expect(reviewActionFor(document.body)).toBeNull();
  });
});

describe("the Copilot review strip", () => {
  const strip = (copilot: ReviewPr["copilot"]) => renderReviews({ prs: [pr({ copilot })], errors: [], off: false, fetchedAt: NOW }, NOW).querySelector<HTMLElement>(".pr__copilot");

  it("is left out when Copilot is not involved in the PR", () => {
    expect(strip(null)).toBeNull();
    expect(strip(undefined)).toBeNull();
  });

  it("says Running, with a spinner, while Copilot has not answered", () => {
    const row = strip({ running: true, comments: 0 })!;
    expect(row.querySelector(".pr__copilot-label")?.textContent).toBe("Copilot review");
    expect(row.querySelector(".pr__chip")?.className).toBe("pr__chip pr__chip--running");
    expect(row.querySelector(".pr__chip-text")?.textContent).toBe("Running");
    expect(row.querySelector(".icon-spinner")).not.toBeNull();
  });

  it("says Reviewed, with a tick, when Copilot found nothing", () => {
    const row = strip({ running: false, comments: 0 })!;
    expect(row.querySelector(".pr__chip")?.className).toBe("pr__chip pr__chip--ok");
    expect(row.querySelector(".pr__chip-text")?.textContent).toBe("Reviewed");
    expect(row.querySelector(".icon-tick")).not.toBeNull();
  });

  it("counts what Copilot had to say, and keeps one comment singular", () => {
    const many = strip({ running: false, comments: 3 })!;
    expect(many.querySelector(".pr__chip")?.className).toBe("pr__chip pr__chip--warn");
    expect(many.querySelector(".pr__chip-text")?.textContent).toBe("Reviewed · 3 comments");
    expect(many.querySelector(".icon-comment")).not.toBeNull();
    expect(strip({ running: false, comments: 1 })!.querySelector(".pr__chip-text")?.textContent).toBe("Reviewed · 1 comment");
  });
});

describe("the reviewer list", () => {
  const listed = (reviewers: ReviewPr["reviewers"]) => renderReviews({ prs: [pr({ reviewers })], errors: [], off: false, fetchedAt: NOW }, NOW).querySelector<HTMLElement>(".pr__reviewers");
  const rows = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>(".pr__reviewer")].map((r) => [r.querySelector(".pr__who")?.textContent ?? r.textContent, r.querySelector(".pr__chip")?.className ?? ""]);

  it("is left out when nobody has been asked", () => {
    expect(listed([])).toBeNull();
    expect(listed(undefined)).toBeNull();
  });

  it("gives every state its own icon as well as its own colour", () => {
    const root = listed([
      { login: "sue", state: "changesRequested", team: false },
      { login: "ann", state: "awaiting", team: false },
      { login: "bob", state: "commented", team: false },
      { login: "kim", state: "approved", team: false },
    ])!;
    expect(rows(root)).toEqual([
      ["sue", "pr__chip pr__chip--bad"],
      ["ann", "pr__chip pr__chip--awaiting"],
      ["bob", "pr__chip pr__chip--neutral"],
      ["kim", "pr__chip pr__chip--ok"],
    ]);
    expect([...root.querySelectorAll(".pr__chip .icon")].map((i) => i.getAttribute("class")?.split(" ")[1])).toEqual(["icon-x-circle", "icon-clock", "icon-comment", "icon-check-circle"]);
  });

  it("counts how many of them approved", () => {
    const note = (reviewers: ReviewPr["reviewers"]) => listed(reviewers)!.querySelector(".pr__section-note")?.textContent;
    expect(note([{ login: "bob", state: "approved", team: false }, { login: "sue", state: "awaiting", team: false }])).toBe("1 of 2 approved");
    expect(note([{ login: "bob", state: "approved", team: false }, { login: "sue", state: "approved", team: false }])).toBe("2 of 2 approved");
    expect(note([{ login: "sue", state: "changesRequested", team: false }])).toBe("0 of 1 approved");
  });

  it("shows four reviewers, but three and a count once there are more", () => {
    const four = ["a", "b", "c", "d"].map((login) => ({ login, state: "awaiting" as const, team: false }));
    expect(rows(listed(four)!).length).toBe(4);
    const five = [...four, { login: "e", state: "awaiting" as const, team: false }];
    const root = listed(five)!;
    expect(rows(root).map(([who]) => who)).toEqual(["a", "b", "c", "+2 more"]);
    expect(root.querySelector(".pr__more")?.textContent).toBe("+2 more");
  });

  it("shows initials for a person and an icon for a team", () => {
    const root = listed([
      { login: "jane-doe", state: "awaiting", team: false },
      { login: "bob", state: "awaiting", team: false },
      { login: "QA", state: "awaiting", team: true },
    ])!;
    const faces = [...root.querySelectorAll<HTMLElement>(".pr__face")];
    expect(faces.map((f) => f.textContent?.trim())).toEqual(["JD", "BO", ""]);
    expect(faces[2].querySelector(".icon-team")).not.toBeNull();
    expect(root.querySelectorAll(".pr__who")[2].getAttribute("title")).toBe("QA (team)");
  });
});

describe("a branch behind its base", () => {
  const row = (over: Partial<ReviewPr>) => renderReviews({ prs: [pr(over)], errors: [], off: false, fetchedAt: NOW }, NOW).querySelector<HTMLElement>(".pr")!;

  it("says nothing when the branch is up to date, or when the forge could not say", () => {
    for (const over of [{ behindBy: 0, baseRef: "main" }, { behindBy: null }, {}]) {
      const r = row(over);
      expect(r.querySelector(".pr__behind"), JSON.stringify(over)).toBeNull();
      expect(r.querySelector("button[data-action=update-branch]"), JSON.stringify(over)).toBeNull();
    }
  });

  it("names the base branch and offers to update it", () => {
    const r = row({ behindBy: 7, baseRef: "develop" });
    expect(r.querySelector(".pr__behind .pr__chip-text")?.textContent).toBe("7 behind develop");
    expect(r.querySelector(".pr__behind")?.getAttribute("title")).toBe("This branch is 7 commits behind develop.");
    const update = r.querySelector<HTMLButtonElement>("button[data-action=update-branch]")!;
    expect(update.querySelector(".pr__chip-text")?.textContent).toBe("Update branch");
    expect(update.disabled).toBe(false);
    expect(row({ behindBy: 1, baseRef: "main" }).querySelector(".pr__behind")?.getAttribute("title")).toBe("This branch is 1 commit behind main.");
  });

  it("puts Review last, after Open and Update branch", () => {
    const foot = [...row({ behindBy: 2, baseRef: "main" }).querySelectorAll<HTMLElement>(".pr__foot button")];
    expect(foot.map((b) => b.dataset.action)).toEqual(["open-pr", "update-branch", "review"]);
  });
});

describe("updating a branch", () => {
  const behind = pr({ behindBy: 2, baseRef: "main" });
  const render = (ui: { busy: Set<string>; errors: Map<string, string> }) =>
    renderReviews({ prs: [behind], errors: [], off: false, fetchedAt: NOW }, NOW, { cards: [], clonesDir: null }, "review", { state: "all", repo: null }, ui);

  it("shows the button working and cannot be pressed twice", () => {
    const root = render({ busy: new Set([behind.url]), errors: new Map() });
    const update = root.querySelector<HTMLButtonElement>("button[data-action=update-branch]")!;
    expect(update.disabled).toBe(true);
    expect(update.querySelector(".pr__chip-text")?.textContent).toBe("Updating…");
    expect(update.querySelector(".icon-spinner")).not.toBeNull();
  });

  it("says in the row itself why the forge refused", () => {
    const root = render({ busy: new Set(), errors: new Map([[behind.url, "422: merge conflict between base and head"]]) });
    expect(root.querySelector(".pr__rowerror")?.textContent).toBe("422: merge conflict between base and head");
    expect(root.querySelector<HTMLButtonElement>("button[data-action=update-branch]")!.disabled).toBe(false);
  });

  it("reads the button as an action on its PR", () => {
    const root = render({ busy: new Set(), errors: new Map() });
    expect(reviewActionFor(root.querySelector("button[data-action=update-branch]")!)).toEqual({ kind: "update-branch", forge: "github", repo: "Org/bedrock", number: 451 });
  });
});

describe("a repaint", () => {
  it("keeps each row's node, so hover and a spinner in flight survive it", () => {
    try { localStorage.clear(); } catch { /* none */ }
    const host = document.createElement("div");
    const view = makeReviewsView(host);
    const state = { prs: [pr(), pr({ number: 3, url: "https://github.com/Org/bedrock/pull/3" })], errors: [], off: false, fetchedAt: NOW };
    view.update(state, NOW);
    const before = [...host.querySelectorAll(".pr")];
    expect(before.length).toBe(2);
    // The same list an hour later: the age labels move, the nodes do not.
    view.update(state, NOW + 3_600_000);
    const after = [...host.querySelectorAll(".pr")];
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
    expect(after[0].querySelector(".pr__meta")?.textContent).toContain("3h");
  });

  it("keeps a row's node when the one above it drops off the list", () => {
    try { localStorage.clear(); } catch { /* none */ }
    const host = document.createElement("div");
    const view = makeReviewsView(host);
    const gone = pr({ number: 9, url: "https://github.com/Org/bedrock/pull/9", updatedAt: "2026-09-29T09:00:00Z" });
    view.update({ prs: [gone, pr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    const kept = host.querySelectorAll(".pr")[1];
    view.update({ prs: [pr()], errors: [], off: false, fetchedAt: NOW }, NOW);
    expect(host.querySelectorAll(".pr").length).toBe(1);
    expect(host.querySelector(".pr")).toBe(kept);
  });
});
