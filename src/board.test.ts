import { describe, expect, it } from "vitest";
import { renderBoard, swapBoard } from "./board";
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
    harness: "claude-code",
    pr: null,
    context: null,
    ...over,
  };
}

describe("context meter", () => {
  it("shows a meter with the percent, colour band and tooltip, or nothing before the first turn", () => {
    expect(renderCard(card({}), NOW).querySelector(".card__meter")).toBeNull();
    const el = renderCard(card({ context: { used: 124_000, window: 200_000, percent: 62 } }), NOW);
    const meter = el.querySelector<HTMLElement>(".card__meter")!;
    expect(meter.title).toBe("Context 62% · 124k of 200k");
    expect(meter.dataset.band).toBe("mid");
    expect(el.querySelector<HTMLElement>(".card__meter-fill")!.style.height).toBe("62%");
    expect(renderCard(card({ context: { used: 10_000, window: 200_000, percent: 5 } }), NOW).querySelector<HTMLElement>(".card__meter")!.dataset.band).toBe("low");
    expect(renderCard(card({ context: { used: 900_000, window: 1_000_000, percent: 90 } }), NOW).querySelector<HTMLElement>(".card__meter")!.title).toBe("Context 90% · 900k of 1M");
  });

  it("offers Compact from 75 percent", () => {
    expect(renderCard(card({ context: { used: 148_000, window: 200_000, percent: 74 } }), NOW).querySelector("button[data-action=compact]")).toBeNull();
    const el = renderCard(card({ context: { used: 150_000, window: 200_000, percent: 75 } }), NOW);
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=compact]")!;
    expect(btn.querySelector("svg.icon-compact")).not.toBeNull();
    expect(btn.textContent?.trim()).toBe("");
    expect(btn.getAttribute("aria-label")).toBe("Compact context");
    expect(btn.title).toContain("/compact");
    expect(el.querySelector<HTMLElement>(".card__meter")!.dataset.band).toBe("high");
  });
});

describe("card badges and PR button", () => {
  it("names the harness and the PR, and hides the PR button when none is known", () => {
    const plain = renderCard(card({}), NOW);
    const badge = plain.querySelector<HTMLImageElement>("img.card__harness")!;
    expect(badge.alt).toBe("Claude Code");
    expect(badge.title).toContain("Claude Code");
    expect(badge.src).toContain("claude-code");
    expect(plain.querySelector("button[data-action=pr]")).toBeNull();
    const withPr = renderCard(card({ pr: { number: 781, url: "https://github.com/o/r/pull/781", state: "merged" } }), NOW);
    const btn = withPr.querySelector<HTMLButtonElement>("button[data-action=pr]")!;
    expect(btn.textContent).toBe("PR #781");
    expect(btn.title).toContain("merged");
    expect(btn.title).toContain("https://github.com/o/r/pull/781");
  });
});

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

  it("renders option buttons for an open question", () => {
    const el = renderCard(card({ state: "awaiting", stateSince: NOW - 5000, awaiting: { kind: "question", detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "" }] }] } }), NOW);
    expect(el.querySelector("button[data-action=answer]")?.textContent).toBe("A");
  });

  it("escapes text content", () => {
    const el = renderCard(card({ snippet: "<img src=x onerror=alert(1)>" }), NOW);
    expect(el.querySelector(".card__snippet img")).toBeNull();
    expect(el.querySelector(".card__snippet")?.textContent).toBe("<img src=x onerror=alert(1)>");
    // The only image is the harness icon.
    expect([...el.querySelectorAll("img")].map((i) => i.className)).toEqual(["card__harness"]);
  });
});

describe("card action icons", () => {
  it("uses a console icon for Terminal and a speech bubble for Reply, with labels", () => {
    const el = renderCard(card({}), NOW);
    const term = el.querySelector<HTMLButtonElement>("button[data-action=terminal]")!;
    const reply = el.querySelector<HTMLButtonElement>("button[data-action=reply]")!;
    expect(term.querySelector("svg.icon-terminal")).not.toBeNull();
    expect(reply.querySelector("svg.icon-reply")).not.toBeNull();
    expect(term.textContent?.trim()).toBe("");
    expect(term.getAttribute("aria-label")).toBe("Open terminal");
    expect(term.title).toBe("Open terminal");
    expect(reply.getAttribute("aria-label")).toBe("Reply");
    expect(reply.title).toBe("Reply");
  });
});

describe("column header buttons", () => {
  it("puts a resume button next to the new-session button on the Idle column only", () => {
    const board = renderBoard([], NOW);
    const idle = board.querySelector(".column--idle")!;
    const resume = idle.querySelector<HTMLButtonElement>("button[data-action=resume-session]")!;
    expect(resume.querySelector("svg.icon-resume")).not.toBeNull();
    expect(resume.getAttribute("aria-label")).toContain("Resume");
    expect(resume.title).toContain("Resume");
    expect(idle.querySelector("button[data-action=new-session]")).not.toBeNull();
    expect(board.querySelector(".column--working button[data-action=resume-session]")).toBeNull();
  });
});

describe("swapBoard", () => {
  it("keeps each column's scroll position across a repaint", () => {
    const host = document.createElement("div");
    host.append(renderBoard([card({ state: "idle" }), card({ sessionId: "w", state: "working" })], NOW));
    const idle = host.querySelector<HTMLElement>(".column--idle .column__cards")!;
    const working = host.querySelector<HTMLElement>(".column--working .column__cards")!;
    idle.scrollTop = 120;
    working.scrollTop = 40;
    swapBoard(host, renderBoard([card({ state: "idle" }), card({ sessionId: "w", state: "working" })], NOW + 1000));
    expect(host.querySelector<HTMLElement>(".column--idle .column__cards")!.scrollTop).toBe(120);
    expect(host.querySelector<HTMLElement>(".column--working .column__cards")!.scrollTop).toBe(40);
    expect(host.querySelector<HTMLElement>(".column--completed .column__cards")!.scrollTop).toBe(0);
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
    expect(cols.map((c) => c.dataset.state)).toEqual(["idle", "working", "awaiting", "completed"]);
    expect(cols.map((c) => c.querySelector(".column__title")?.textContent)).toEqual(["Idle", "Working", "Awaiting Decision", "Completed"]);
    expect(cols.map((c) => c.querySelector(".column__count")?.textContent)).toEqual(["1", "2", "1", "1"]);
    expect([...cols[1].querySelectorAll(".card")].map((c) => (c as HTMLElement).dataset.sessionId)).toEqual(["a", "e"]);
  });

  it("puts a new-session button only in the Idle column header", () => {
    const board = renderBoard([], NOW);
    const withAdd = [...board.querySelectorAll<HTMLElement>(".column")].filter((c) => c.querySelector("button[data-action=new-session]"));
    expect(withAdd.map((c) => c.dataset.state)).toEqual(["idle"]);
    expect(board.querySelector("button[data-action=new-session]")?.textContent).toBe("+");
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
    expect(el.querySelector("button[data-action=terminal] svg.icon-terminal")).not.toBeNull();
    expect(el.querySelector("button[data-action=reply] svg.icon-reply")).not.toBeNull();
  });
});
