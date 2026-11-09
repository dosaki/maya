import { describe, expect, it, vi } from "vitest";
import { applyWorkFilters, areas, guardStart, makeWorkItemsView, needsDialog, renderItemCard, renderStartDialog, sessionFor, startWorkItem, workItemActionFor } from "./workitems";
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

  it("asks before taking over someone else's issue", () => {
    const el = renderStartDialog(item({ id: "a/b#1", title: "T" }), { takeover: ["maria", "joe"], pick: null, branch: "issue-a-b-1-t" }, { onStart: vi.fn(), onCancel: vi.fn() });
    expect(el.textContent).toContain("This issue is already assigned to maria, joe. Are you sure you want to take over?");
    expect(el.querySelector<HTMLButtonElement>(".card__btn--primary")?.textContent).toBe("Yes, take over and start");
    expect(el.textContent).toContain("Start a/b#1: T");
  });

  it("shows Maya's pick preselected and the options to change it", () => {
    const api = { forge: "github", path: "acme/api" } as const;
    const web = { forge: "gitlab", path: "acme/web" } as const;
    const on = { onStart: vi.fn(), onCancel: vi.fn() };
    const el = renderStartDialog(item({}), { takeover: [], pick: { picked: api, why: "The endpoint lives there.", options: [api, web] }, branch: "b" }, on);
    const sel = el.querySelector<HTMLSelectElement>("select[name=repo]")!;
    expect(sel.value).toBe("github:acme/api");
    expect([...sel.options].map((o) => o.textContent)).toEqual(["acme/api (GitHub)", "acme/web (GitLab)"]);
    expect(el.textContent).toContain("Maya picked acme/api — The endpoint lives there.");
    el.querySelector<HTMLButtonElement>(".card__btn--primary")!.click();
    expect(on.onStart).toHaveBeenCalledWith(api);
  });

  it("waits for a choice when Maya could not tell", () => {
    const api = { forge: "github", path: "acme/api" } as const;
    const on = { onStart: vi.fn(), onCancel: vi.fn() };
    const el = renderStartDialog(item({}), { takeover: [], pick: { picked: null, why: "", options: [api] }, branch: "b" }, on);
    const go = el.querySelector<HTMLButtonElement>(".card__btn--primary")!;
    expect(el.textContent).toContain("Maya could not tell which repository; choose one.");
    expect(go.textContent).toBe("Start session");
    expect(go.disabled).toBe(true);
    const sel = el.querySelector<HTMLSelectElement>("select[name=repo]")!;
    sel.value = "github:acme/api";
    sel.dispatchEvent(new Event("change"));
    expect(go.disabled).toBe(false);
    go.click();
    expect(on.onStart).toHaveBeenCalledWith(api);
  });

  it("needs a dialog only for a takeover or a pick", () => {
    expect(needsDialog({ takeover: [], pick: null, branch: "b" })).toBe(false);
    expect(needsDialog({ takeover: ["x"], pick: null, branch: "b" })).toBe(true);
    expect(needsDialog({ takeover: [], pick: { picked: null, why: "", options: [] }, branch: "b" })).toBe(true);
  });

  it("starts at once when nothing needs asking", async () => {
    const invoke = vi.fn().mockResolvedValueOnce({ takeover: [], pick: null, branch: "b" }).mockResolvedValueOnce({ dir: "/c/api-issue-1", nativeTerm: 4, notice: "Heads up." });
    const toast = vi.fn();
    const opened = vi.fn();
    await startWorkItem(item({ key: "k" }), { invoke, host: document.createElement("div"), toast, opened });
    expect(invoke).toHaveBeenNthCalledWith(2, "work_item_start", { key: "k", repo: null, takeOver: false });
    expect(toast).toHaveBeenNthCalledWith(1, "Working on a/b#1 in api-issue-1");
    expect(toast).toHaveBeenNthCalledWith(2, "Heads up.");
    expect(opened).toHaveBeenCalledWith(4, "/c/api-issue-1");
  });

  it("starts with the chosen repository and the takeover after the dialog", async () => {
    const api = { forge: "github", path: "acme/api" } as const;
    const host = document.createElement("div");
    const invoke = vi.fn().mockResolvedValueOnce({ takeover: ["maria"], pick: { picked: api, why: "w", options: [api] }, branch: "b" }).mockResolvedValueOnce({ dir: "/c/x", nativeTerm: null, notice: null });
    const done = startWorkItem(item({ key: "k" }), { invoke, host, toast: vi.fn(), opened: vi.fn() });
    await vi.waitFor(() => expect(host.querySelector(".card__btn--primary")).not.toBeNull());
    host.querySelector<HTMLButtonElement>(".card__btn--primary")!.click();
    await done;
    expect(invoke).toHaveBeenNthCalledWith(2, "work_item_start", { key: "k", repo: api, takeOver: true });
    expect(host.children).toHaveLength(0);
  });

  it("cancel starts nothing", async () => {
    const host = document.createElement("div");
    const invoke = vi.fn().mockResolvedValueOnce({ takeover: ["maria"], pick: null, branch: "b" });
    const done = startWorkItem(item({ key: "k" }), { invoke, host, toast: vi.fn(), opened: vi.fn() });
    await vi.waitFor(() => expect(host.querySelector("button")).not.toBeNull());
    [...host.querySelectorAll("button")].find((b) => b.textContent === "Cancel")!.click();
    await done;
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(host.children).toHaveLength(0);
  });

  it("Escape starts nothing", async () => {
    const host = document.createElement("div");
    const invoke = vi.fn().mockResolvedValueOnce({ takeover: ["maria"], pick: null, branch: "b" });
    const done = startWorkItem(item({ key: "k" }), { invoke, host, toast: vi.fn(), opened: vi.fn() });
    await vi.waitFor(() => expect(host.querySelector("button")).not.toBeNull());
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await done;
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(host.children).toHaveLength(0);
  });

  it("toasts an error as it is", async () => {
    const invoke = vi.fn().mockRejectedValueOnce("That issue was closed.");
    const toast = vi.fn();
    await startWorkItem(item({}), { invoke, host: document.createElement("div"), toast, opened: vi.fn() });
    expect(toast).toHaveBeenCalledWith("That issue was closed.");
  });

  it("ignores a second Start of an item while one is in flight, and disables its button", async () => {
    const inFlight = new Set<string>();
    let release!: () => void;
    const run = vi.fn(() => new Promise<void>((r) => (release = r)));
    const changed = vi.fn();
    const first = guardStart(inFlight, "k", run, changed);
    await guardStart(inFlight, "k", run, changed);
    expect(run).toHaveBeenCalledTimes(1);
    expect(inFlight.has("k")).toBe(true);
    release();
    await first;
    expect(inFlight.has("k")).toBe(false);
    expect(changed).toHaveBeenCalledTimes(2);
    expect(renderItemCard(item({}), 0, null, true).querySelector<HTMLButtonElement>("[data-action=start]")!.disabled).toBe(true);
    expect(renderItemCard(item({}), 0, null).querySelector<HTMLButtonElement>("[data-action=start]")!.disabled).toBe(false);
  });

  it("treats a dialog replaced in the host as a cancel", async () => {
    const host = document.createElement("div");
    const invoke = vi.fn().mockResolvedValueOnce({ takeover: ["maria"], pick: null, branch: "b" });
    const inFlight = new Set<string>();
    const done = guardStart(inFlight, "k", () => startWorkItem(item({ key: "k" }), { invoke, host, toast: vi.fn(), opened: vi.fn() }));
    await vi.waitFor(() => expect(host.querySelector("button")).not.toBeNull());
    host.replaceChildren();
    await done;
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(inFlight.size).toBe(0);
  });

  it("starts nothing, not even the check, while another window is open", async () => {
    const host = document.createElement("div");
    host.append(document.createElement("div"));
    const invoke = vi.fn();
    const toast = vi.fn();
    await startWorkItem(item({}), { invoke, host, toast, opened: vi.fn() });
    expect(invoke).not.toHaveBeenCalled();
    expect(toast).toHaveBeenCalledWith("Close the open window first.");
    expect(host.childElementCount).toBe(1);
  });
});
