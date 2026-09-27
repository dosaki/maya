import { describe, expect, it } from "vitest";
import { renderBoard } from "./board";
import { renderCard } from "./card";
import { formatAge, projectName } from "./format";
import type { Card } from "./types";

const NOW = 1_790_600_000_000;

function card(over: Partial<Card>): Card {
  return {
    sessionId: "s",
    pid: 1,
    name: "eye-1",
    cwd: "/Users/x/dev/eye",
    state: "idle",
    stateSince: NOW - 90_000,
    snippet: "",
    awaiting: null,
    hasInbox: true,
    ...over,
  };
}

describe("formatAge", () => {
  it("formats seconds, minutes, hours and days", () => {
    expect(formatAge(NOW - 5_000, NOW)).toBe("5s");
    expect(formatAge(NOW - 90_000, NOW)).toBe("1m");
    expect(formatAge(NOW - 3 * 3_600_000, NOW)).toBe("3h");
    expect(formatAge(NOW - 2 * 86_400_000, NOW)).toBe("2d");
    expect(formatAge(NOW + 1000, NOW)).toBe("0s");
  });
});

describe("projectName", () => {
  it("uses the last path segment", () => {
    expect(projectName("/Users/x/dev/eye")).toBe("eye");
    expect(projectName("/Users/x")).toBe("x");
    expect(projectName("/")).toBe("/");
  });
});

describe("renderCard", () => {
  it("shows name, project, age and snippet", () => {
    const el = renderCard(card({ snippet: "All done." }), NOW);
    expect(el.dataset.sessionId).toBe("s");
    expect(el.dataset.pid).toBe("1");
    expect(el.className).toContain("card--idle");
    expect(el.querySelector(".card__name")?.textContent).toBe("eye-1");
    expect(el.querySelector(".card__project")?.textContent).toBe("eye");
    expect(el.querySelector(".card__age")?.textContent).toBe("1m");
    expect(el.querySelector(".card__snippet")?.textContent).toBe("All done.");
  });

  it("shows the awaiting detail with its kind", () => {
    const el = renderCard(card({ state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm -rf build", questions: [] } }), NOW);
    expect(el.querySelector(".card__awaiting")?.textContent).toBe("Bash: rm -rf build");
    expect(el.querySelector(".card__awaiting")?.getAttribute("data-kind")).toBe("permission");
  });

  it("escapes text content", () => {
    const el = renderCard(card({ snippet: "<img src=x onerror=alert(1)>" }), NOW);
    expect(el.querySelector("img")).toBeNull();
  });
});

describe("renderBoard", () => {
  it("renders four columns in order with counts and cards in the right column", () => {
    const cards: Card[] = [
      card({ sessionId: "a", name: "a", state: "working" }),
      card({ sessionId: "b", name: "b", state: "awaiting", awaiting: { kind: "question", detail: "Which?", questions: [] } }),
      card({ sessionId: "c", name: "c", state: "idle" }),
      card({ sessionId: "d", name: "d", state: "completed" }),
      card({ sessionId: "e", name: "e", state: "working" }),
    ];
    const board = renderBoard(cards, NOW);
    const cols = [...board.querySelectorAll<HTMLElement>(".column")];
    expect(cols.map((c) => c.dataset.state)).toEqual(["awaiting", "working", "completed", "idle"]);
    expect(cols.map((c) => c.querySelector(".column__title")?.textContent)).toEqual(["Awaiting Decision", "Working", "Completed", "Idle"]);
    expect(cols.map((c) => c.querySelector(".column__count")?.textContent)).toEqual(["1", "2", "1", "1"]);
    expect([...cols[1].querySelectorAll(".card")].map((c) => (c as HTMLElement).dataset.sessionId)).toEqual(["a", "e"]);
  });

  it("orders cards in a column by most recent state change first", () => {
    const cards: Card[] = [
      card({ sessionId: "old", state: "idle", stateSince: NOW - 10_000 }),
      card({ sessionId: "new", state: "idle", stateSince: NOW - 1_000 }),
    ];
    const ids = [...renderBoard(cards, NOW).querySelectorAll<HTMLElement>(".card")].map((c) => c.dataset.sessionId);
    expect(ids).toEqual(["new", "old"]);
  });

  it("keeps two sessions with the same cwd as separate cards", () => {
    const cards: Card[] = [
      card({ sessionId: "x1", name: "envoy-1b", cwd: "/Users/x/dev/envoy" }),
      card({ sessionId: "x2", name: "envoy-ab", cwd: "/Users/x/dev/envoy" }),
    ];
    expect(renderBoard(cards, NOW).querySelectorAll(".card").length).toBe(2);
  });

  it("shows an empty hint in an empty column", () => {
    const board = renderBoard([], NOW);
    expect(board.querySelectorAll(".column__empty").length).toBe(4);
  });
});

describe("card actions", () => {
  it("renders Terminal and Reply buttons", () => {
    const el = renderCard(card({}), NOW);
    expect(el.querySelector("button[data-action=terminal]")?.textContent).toBe("Terminal");
    expect(el.querySelector("button[data-action=reply]")?.textContent).toBe("Reply");
  });
});
