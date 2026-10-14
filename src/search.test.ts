import { describe, expect, it, vi } from "vitest";
import { renderBoard } from "./board";
import { filterCards, makeSessionSearch, matchesSearch } from "./search";
import type { Card } from "./types";

function card(over: Partial<Card>): Card {
  return {
    sessionId: "s",
    pid: 1,
    name: "fix the login",
    cwd: "/Users/x/dev/maya",
    state: "idle",
    stateSince: 0,
    snippet: "",
    awaiting: null,
    hasInbox: true,
    harness: "claude-code",
    pr: null,
    context: null,
    ...over,
  };
}

describe("matchesSearch", () => {
  it("finds the repository, session name, agent and machine, ignoring case", () => {
    const c = card({ machine: "Gnowee" });
    expect(matchesSearch(c, "MAYA")).toBe(true);
    expect(matchesSearch(c, "login")).toBe(true);
    expect(matchesSearch(c, "claude")).toBe(true);
    expect(matchesSearch(c, "claude-code")).toBe(true);
    expect(matchesSearch(c, "gnow")).toBe(true);
    expect(matchesSearch(c, "codex")).toBe(false);
  });

  it("does not search the rest of the path", () => {
    expect(matchesSearch(card({}), "Users")).toBe(false);
  });

  it("reads a local session's machine as this computer", () => {
    expect(matchesSearch(card({}), "this")).toBe(true);
    expect(matchesSearch(card({ machine: "Gnowee" }), "this")).toBe(false);
  });

  it("needs every word, each in any field", () => {
    const c = card({ harness: "codex" });
    expect(matchesSearch(c, "maya codex")).toBe(true);
    expect(matchesSearch(c, "  maya   login ")).toBe(true);
    expect(matchesSearch(c, "maya claude")).toBe(false);
  });

  it("matches everything on a blank query", () => {
    const cards = [card({ sessionId: "a" }), card({ sessionId: "b", cwd: "/x/eye" })];
    expect(filterCards(cards, "  ")).toBe(cards);
    expect(filterCards(cards, "eye").map((c) => c.sessionId)).toEqual(["b"]);
  });
});

describe("makeSessionSearch", () => {
  it("reports each edit, clears on Escape, and counts only while searching", () => {
    const host = document.createElement("div");
    const onChange = vi.fn();
    const search = makeSessionSearch(host, onChange);
    const input = host.querySelector<HTMLInputElement>("input")!;
    expect(input.getAttribute("aria-label")).toBe("Search sessions");
    expect(host.querySelector("svg.icon-search")).not.toBeNull();

    search.setCount(5, 5);
    expect(host.querySelector(".session-search__count")!.textContent).toBe("");
    input.value = "maya";
    input.dispatchEvent(new Event("input"));
    expect(onChange).toHaveBeenLastCalledWith("maya");
    expect(search.query()).toBe("maya");
    search.setCount(2, 5);
    expect(host.querySelector(".session-search__count")!.textContent).toBe("2 of 5");

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(input.value).toBe("");
    expect(onChange).toHaveBeenLastCalledWith("");
  });
});

describe("renderBoard empty text", () => {
  it("says Nothing here by default and the given text while searching", () => {
    expect(renderBoard([], 0).querySelector(".column__empty")!.textContent).toBe("Nothing here");
    expect(renderBoard([], 0, undefined, "No matches").querySelector(".column__empty")!.textContent).toBe("No matches");
  });
});
