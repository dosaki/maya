import { describe, expect, it } from "vitest";
import { applyWorkFilters, areas, makeWorkItemsView, renderItemCard, sessionFor, workItemActionFor } from "./workitems";
import type { Card, WorkItem } from "./types";

const item = (over: Partial<WorkItem> = {}): WorkItem => ({
  key: "github:a/b#1", source: "github", id: "a/b#1", number: 1, title: "A title", url: "https://github.com/a/b/issues/1", description: "",
  status: null, labels: [], assignees: [], author: "jane", updatedAt: "2026-09-29T08:00:00Z", repo: { forge: "github", path: "a/b" }, tracker: false, ...over,
});

const card = (over: Partial<Card> = {}): Card => ({
  sessionId: "s", pid: 1, name: "n", cwd: "/Users/x/dev/other", state: "working", stateSince: 100, snippet: "", awaiting: null,
  hasInbox: true, harness: "claude-code", pr: null, context: null, ...over,
});

describe("work items", () => {
  it("puts an item with a session on the right and the rest on the left", () => {
    const a = item({ key: "github:a/b#1", id: "a/b#1" });
    const b = item({ key: "github:a/b#2", id: "a/b#2" });
    const s = card({ name: "issue a/b#1", state: "working" });
    const { toPick, working } = areas({ items: [a, b], started: [], errors: [], off: false, fetchedAt: 1 }, [s]);
    expect(toPick.map((i) => i.key)).toEqual(["github:a/b#2"]);
    expect(working).toEqual([{ item: a, card: s }]);
  });

  it("links a session by its folder when it was renamed", () => {
    const a = item({ key: "k", id: "a/b#1" });
    const started = [{ key: "k", item: a, dir: "/c/b-issue-1", startedAt: 0 }];
    expect(sessionFor(a, started, [card({ name: "x", cwd: "/c/b-issue-1" })])).not.toBeNull();
    expect(sessionFor(a, started, [card({ name: "x", cwd: "/c/b-issue-1/src" })])).not.toBeNull();
    expect(sessionFor(a, started, [card({ name: "x", cwd: "/c/b-issue-10" })])).toBeNull();
  });

  it("takes the most recently changed session", () => {
    const a = item({ id: "a/b#1" });
    const old = card({ sessionId: "o", name: "issue a/b#1", stateSince: 1 });
    const fresh = card({ sessionId: "f", name: "issue a/b#1", stateSince: 5 });
    expect(sessionFor(a, [], [old, fresh])?.sessionId).toBe("f");
  });

  it("filters by source, repository, status and words", () => {
    const items = [item({ key: "1", source: "github", title: "Login times out", status: "todo", repo: { forge: "github", path: "acme/api" } }), item({ key: "2", source: "gitlab", title: "VAT", labels: ["backend"] })];
    expect(applyWorkFilters(items, { source: "gitlab", repo: null, status: null, search: "" }).map((i) => i.key)).toEqual(["2"]);
    expect(applyWorkFilters(items, { source: null, repo: "github:acme/api", status: "todo", search: "LOGIN" }).map((i) => i.key)).toEqual(["1"]);
    expect(applyWorkFilters(items, { source: null, repo: null, status: null, search: "backend" }).map((i) => i.key)).toEqual(["2"]);
  });

  it("shows Start on an item to pick and says when Maya must pick the repository", () => {
    const el = renderItemCard(item({ repo: null }), Date.now(), null);
    expect(el.querySelector("[data-action=start]")?.textContent).toBe("Start");
    expect(el.textContent).toContain("No repo: Maya picks");
    expect(el.dataset.key).toBe("github:a/b#1");
  });

  it("shows Open session and the session's state on a working item", () => {
    const el = renderItemCard(item({}), Date.now(), card({ state: "awaiting", snippet: "Should the retry email them?", pid: 42 }));
    expect(el.querySelector("[data-action=session]")?.getAttribute("data-pid")).toBe("42");
    expect(el.classList.contains("item--awaiting")).toBe(true);
    expect(el.textContent).toContain("Should the retry email them?");
    expect(el.querySelector("[data-action=start]")).toBeNull();
  });

  it("reads clicks into actions", () => {
    const el = renderItemCard(item({}), Date.now(), null);
    document.body.append(el);
    expect(workItemActionFor(el.querySelector("[data-action=start]")!)).toEqual({ kind: "start", key: "github:a/b#1" });
    expect(workItemActionFor(el.querySelector("[data-action=open-item]")!)).toEqual({ kind: "open-item", key: "github:a/b#1" });
    const w = renderItemCard(item({}), Date.now(), card({ pid: 7 }));
    expect(workItemActionFor(w.querySelector("[data-action=session]")!)).toEqual({ kind: "session", key: "github:a/b#1", pid: 7 });
    const off = document.createElement("button");
    off.dataset.action = "settings";
    expect(workItemActionFor(off)).toEqual({ kind: "settings" });
    el.remove();
  });

  it("says issues are off", () => {
    const host = document.createElement("div");
    makeWorkItemsView(host).update({ items: [], started: [], errors: [], off: true, fetchedAt: null }, [], Date.now());
    expect(host.textContent).toContain("Issues are off. Turn on GitHub or GitLab in Settings → Git.");
    expect(host.querySelector("[data-action=settings]")).not.toBeNull();
  });

  it("keeps the filter dropdowns across repaints", () => {
    const host = document.createElement("div");
    const view = makeWorkItemsView(host);
    view.update({ items: [item({})], started: [], errors: [], off: false, fetchedAt: 1 }, [], Date.now());
    const sel = host.querySelector("select");
    view.update({ items: [item({}), item({ key: "2" })], started: [], errors: [], off: false, fetchedAt: 2 }, [], Date.now());
    expect(host.querySelector("select")).toBe(sel);
    expect(host.querySelector(".issues__area--pick .issues__count")?.textContent).toBe("2");
  });
});
