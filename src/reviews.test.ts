import { describe, expect, it } from "vitest";
import { relatedSession, renderReviews, reviewActionFor } from "./reviews";
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
  it("renders a card per PR with repo, number, title, author, age and reason chips", () => {
    const el = renderReviews({ prs: [pr(), pr({ number: 3, repo: "Org/plugins", isDraft: true, reasons: ["review", "assigned"], updatedAt: "2026-09-28T10:00:00Z" })], error: null, fetchedAt: NOW }, NOW);
    const cards = el.querySelectorAll(".pr");
    expect(cards.length).toBe(2);
    const first = cards[0] as HTMLElement;
    expect(first.dataset.repo).toBe("Org/bedrock");
    expect(first.dataset.number).toBe("451");
    expect(first.querySelector(".pr__repo")?.textContent).toBe("bedrock #451");
    expect(first.querySelector(".pr__title")?.textContent).toBe("docs: expand upgrade notes");
    expect(first.querySelector(".pr__meta")?.textContent).toContain("jane");
    expect(first.querySelector(".pr__meta")?.textContent).toContain("2h");
    expect([...first.querySelectorAll(".pr__chip")].map((c) => c.textContent)).toEqual(["review requested"]);
    const second = cards[1];
    expect([...second.querySelectorAll(".pr__chip")].map((c) => c.textContent)).toEqual(["draft", "review requested", "assigned"]);
    expect(first.querySelector("button[data-action=open-pr]")?.textContent).toBe("Open");
    expect(first.querySelector("button[data-action=review]")?.textContent).toBe("Review");
  });

  it("shows a Terminal button when a session is related, with its name and state", () => {
    const live = [session({ sessionId: "a", pid: 42, name: "review bedrock #451", state: "awaiting" })];
    const el = renderReviews({ prs: [pr()], error: null, fetchedAt: NOW }, NOW, { cards: live, clonesDir: null });
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=terminal]")!;
    expect(btn.textContent).toBe("Terminal");
    expect(btn.title).toContain("review bedrock #451");
    expect(btn.title).toContain("Awaiting Decision");
    expect(reviewActionFor(btn)).toEqual({ kind: "terminal", repo: "Org/bedrock", number: 451, pid: 42 });
    expect(renderReviews({ prs: [pr()], error: null, fetchedAt: NOW }, NOW).querySelector("button[data-action=terminal]")).toBeNull();
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
