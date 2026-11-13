import { describe, expect, it } from "vitest";
import { boardCards, countable, renderBoard, swapBoard } from "./board";
import { renderCard } from "./card";
import { formatAge, projectName } from "./format";
import type { Card } from "./types";
import styles from "./styles.css?raw";
import mobileStyles from "./mobile/mobile.css?raw";

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

describe("the brain card", () => {
  it("shows the brain card only in debug mode and never counts it", () => {
    const cards = [card({ sessionId: "a" }), card({ sessionId: "m", brain: true, state: "working" })];
    expect(boardCards(cards, false).map((c) => c.sessionId)).toEqual(["a"]);
    expect(boardCards(cards, true).map((c) => c.sessionId)).toEqual(["a", "m"]);
    expect(countable(cards).map((c) => c.sessionId)).toEqual(["a"]);
  });
});

describe("context ring", () => {
  it("shows the context as a ring with its percentage, band and tooltip in the footer, or nothing before the first turn", () => {
    expect(renderCard(card({}), NOW).querySelector(".card__ctx")).toBeNull();
    const el = renderCard(card({ context: { used: 124_000, window: 200_000, percent: 62 } }), NOW);
    const ctx = el.querySelector<HTMLElement>(".card__foot .card__ctx")!;
    expect(ctx.textContent).toBe("62%");
    expect(ctx.title).toBe("Context 62% · 124k of 200k");
    expect(ctx.dataset.band).toBe("mid");
    expect(ctx.style.getPropertyValue("--p")).toBe("62%");
    expect(el.querySelector(".card__meter")).toBeNull();
    expect(renderCard(card({ context: { used: 10_000, window: 200_000, percent: 5 } }), NOW).querySelector<HTMLElement>(".card__ctx")!.dataset.band).toBe("low");
    expect(renderCard(card({ context: { used: 900_000, window: 1_000_000, percent: 90 } }), NOW).querySelector<HTMLElement>(".card__ctx")!.dataset.band).toBe("high");
  });

  it("offers Compact from 75 percent, as Maya's compact icon in the footer", () => {
    expect(renderCard(card({ context: { used: 148_000, window: 200_000, percent: 74 } }), NOW).querySelector("button[data-action=compact]")).toBeNull();
    const el = renderCard(card({ context: { used: 150_000, window: 200_000, percent: 75 } }), NOW);
    const btn = el.querySelector<HTMLButtonElement>(".card__foot .card__actions button[data-action=compact]")!;
    expect(btn.querySelector("svg.icon-compact")).not.toBeNull();
    expect(btn.textContent?.trim()).toBe("");
    expect(btn.getAttribute("aria-label")).toBe("Compact context");
    expect(btn.title).toContain("/compact");
    expect(el.querySelector<HTMLElement>(".card__ctx")!.dataset.band).toBe("high");
  });
});

describe("the line under the name", () => {
  it("names the project, machine, branch and model, each only when known", () => {
    const branch = "feat/a-very-long-branch-name-that-overflows";
    const el = renderCard(card({ machine: "laptop", branch, model: "Opus 5.5" }), NOW);
    const sub = el.querySelector(".card__sub")!;
    expect(sub.querySelector(".card__project")!.textContent).toBe("eye");
    expect(sub.querySelector(".card__machine")!.textContent).toBe("laptop");
    expect(sub.querySelector(".card__remote")).not.toBeNull();
    const b = sub.querySelector<HTMLElement>(".card__branch")!;
    expect(b.textContent).toBe(branch);
    expect(b.title).toBe(branch);
    expect(sub.querySelector(".card__model")!.textContent).toBe("Opus 5.5");
  });

  it("shows only the agent and project when nothing else is known, with no stray separator", () => {
    const sub = renderCard(card({}), NOW).querySelector(".card__sub")!;
    expect(sub.querySelector(".card__branch, .card__model, .card__machine, .card__remote")).toBeNull();
    expect(sub.textContent).toBe("eye");
    expect(renderCard(card({ branch: null, model: null }), NOW).querySelector(".card__sub")!.textContent).toBe("eye");
  });
});

describe("the footer", () => {
  const usage = (micros: number) => ({ input: 1, cachedInput: 0, cacheWrite: 0, output: 1, cost: { micros, unit: "usd" as const, estimated: false, models: [] } });

  it("shows the PR as a small pill and the cost so far", () => {
    const el = renderCard(card({ pr: { number: 97, url: "https://github.com/o/r/pull/97", state: "open", forge: "github" }, usage: usage(1_240_000) }), NOW);
    const pr = el.querySelector<HTMLButtonElement>(".card__foot .card__facts button[data-action=pr]")!;
    expect(pr.textContent).toBe("#97");
    expect(pr.getAttribute("aria-label")).toBe("PR #97");
    expect(el.querySelector(".card__foot .card__facts .card__cost")!.textContent).toBe("$1.24");
  });

  it("shows no cost before one is known", () => {
    expect(renderCard(card({}), NOW).querySelector(".card__cost")).toBeNull();
    expect(renderCard(card({ usage: { input: 1, cachedInput: 0, cacheWrite: 0, output: 1, cost: null } }), NOW).querySelector(".card__cost")).toBeNull();
  });

  it("keeps the footer, with its buttons on the right, on a card that knows nothing else", () => {
    const foot = renderCard(card({}), NOW).querySelector(".card__foot")!;
    expect(foot.querySelector(".card__facts")!.children).toHaveLength(0);
    expect(foot.lastElementChild!.classList.contains("card__actions")).toBe(true);
  });

  it("has no Reply button: a click anywhere on the card opens it", () => {
    const el = renderCard(card({ snippet: "Done." }), NOW);
    expect(el.querySelector("[data-action=reply]")).toBeNull();
    expect(el.tabIndex).toBe(0);
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
    expect(btn.textContent).toBe("#781");
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

  it("shows the awaiting detail as the question, with its kind", () => {
    const el = renderCard(card({ state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm -rf build", questions: [] } }), NOW);
    expect(el.querySelector(".card__ask .card__question")?.textContent).toBe("Bash: rm -rf build");
    expect(el.querySelector(".card__question")?.getAttribute("data-kind")).toBe("permission");
    expect(el.querySelector(".card__awaiting")).toBeNull();
  });

  it("puts a question over numbered answers, then the last message", () => {
    const q = (header: string) => ({ question: "Which?", header, multiSelect: false, options: [{ label: "A", description: "" }, { label: "B", description: "" }, { label: "C", description: "" }] });
    const el = renderCard(card({ state: "awaiting", stateSince: NOW - 5000, snippet: "I looked.", awaiting: { kind: "question", detail: "Which approach?", questions: [q("Approach"), q("Scope")] } }), NOW);
    const ask = el.querySelector(".card__ask")!;
    expect(ask.firstElementChild?.textContent).toBe("Question 1 of 2 · Approach");
    expect(ask.querySelector(".card__question")?.textContent).toBe("Which approach?");
    expect(ask.querySelectorAll(".options__label")).toHaveLength(1);
    expect([...ask.querySelectorAll(".options__num")].map((n) => n.textContent)).toEqual(["1", "2", "3"]);
    expect([...ask.querySelectorAll("button[data-action=answer]")].map((b) => b.querySelector(".options__text")?.textContent)).toEqual(["A", "B", "C"]);
    expect(ask.nextElementSibling?.classList.contains("card__snippet")).toBe(true);
  });

  it("renders option buttons for an open question", () => {
    const el = renderCard(card({ state: "awaiting", stateSince: NOW - 5000, awaiting: { kind: "question", detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "" }] }] } }), NOW);
    expect(el.querySelector("button[data-action=answer] .options__text")?.textContent).toBe("A");
  });

  it("escapes text content", () => {
    const el = renderCard(card({ snippet: "<img src=x onerror=alert(1)>" }), NOW);
    expect(el.querySelector(".card__snippet img")).toBeNull();
    expect(el.querySelector(".card__snippet")?.textContent).toBe("<img src=x onerror=alert(1)>");
    // The only image is the harness icon.
    expect([...el.querySelectorAll("img")].map((i) => i.className)).toEqual(["card__harness"]);
  });
});

describe("other harnesses", () => {
  it("shows the harness icon and the Compact button for a Codex card", () => {
    const el = renderCard(card({ harness: "codex", hasInbox: false, context: { used: 160_000, window: 200_000, percent: 80 } }), NOW);
    const badge = el.querySelector<HTMLImageElement>("img.card__harness")!;
    expect(badge.alt).toBe("Codex");
    expect(badge.src).not.toBe("");
    expect(el.querySelector("button[data-action=compact]")).not.toBeNull();
    const agy = renderCard(card({ harness: "antigravity", hasInbox: false }), NOW);
    expect(agy.querySelector<HTMLImageElement>("img.card__harness")!.alt).toBe("Antigravity");
    const grok = renderCard(card({ harness: "grok", hasInbox: false }), NOW);
    expect(grok.querySelector<HTMLImageElement>("img.card__harness")!.alt).toBe("Grok Build");
  });
});

describe("card action icons", () => {
  it("uses plain icons with labels for Terminal and Stop", () => {
    const el = renderCard(card({ state: "working", stateSince: NOW - 5000 }), NOW);
    const term = el.querySelector<HTMLButtonElement>(".card__actions button[data-action=terminal]")!;
    expect(term.querySelector("svg.icon-terminal")).not.toBeNull();
    expect(term.textContent?.trim()).toBe("");
    expect(term.getAttribute("aria-label")).toBe("Open terminal");
    expect(term.title).toBe("Open terminal");
    const stop = el.querySelector<HTMLButtonElement>(".card__actions button[data-action=stop]")!;
    expect(stop.querySelector("svg.icon-stop")).not.toBeNull();
    expect(stop.textContent?.trim()).toBe("");
    expect(stop.getAttribute("aria-label")).toBe("Stop");
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

  // WebKit drops :hover from a replaced node until the pointer moves, so a
  // repaint that rebuilt the cards blinked the one under the pointer.
  it("keeps the nodes of cards still on the board and updates them in place", () => {
    const host = document.createElement("div");
    const before = [card({ sessionId: "a" }), card({ sessionId: "b", state: "working" }), card({ sessionId: "gone", state: "awaiting" })];
    swapBoard(host, renderBoard(before, NOW));
    const board = host.querySelector(".board");
    const a = host.querySelector('[data-session-id="a"]');
    const b = host.querySelector('[data-session-id="b"]');
    const after = [card({ sessionId: "a", name: "renamed" }), card({ sessionId: "new", stateSince: NOW }), card({ sessionId: "b", state: "working" })];
    swapBoard(host, renderBoard(after, NOW + 120_000));
    expect(host.querySelector(".board")).toBe(board);
    expect(host.querySelector('[data-session-id="a"]')).toBe(a);
    expect(host.querySelector('[data-session-id="b"]')).toBe(b);
    expect(host.querySelector('[data-session-id="gone"]')).toBeNull();
    expect(host.innerHTML).toBe(renderBoard(after, NOW + 120_000).outerHTML);
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

describe("swapBoard on a sideways-scrolling board", () => {
  it("keeps the board's own horizontal scroll across a repaint", () => {
    const host = document.createElement("div");
    host.append(renderBoard([card({})], NOW));
    const old = host.querySelector<HTMLElement>(".board")!;
    Object.defineProperty(old, "scrollLeft", { value: 640, writable: true });
    swapBoard(host, renderBoard([card({})], NOW + 60_000));
    expect(host.querySelector<HTMLElement>(".board")!.scrollLeft).toBe(640);
  });
});

describe("card styles", () => {
  it("keeps Stop red while the pointer is on the card, as hovering turns the other icons to the text colour", () => {
    expect(styles).toMatch(/\.card:hover \.card__icon\.card__btn--stop[^{]*\{[^}]*color:\s*#f87171/);
    expect(styles).toMatch(/:root\[data-theme="light"\] \.card:hover \.card__icon\.card__btn--stop[^{]*\{[^}]*color:\s*#b91c1c/);
  });

  it("gives the PR pill the phone's 40px tap height", () => {
    const rule = mobileStyles.match(/([^{}]*\.card__pr[^{}]*)\{([^}]*)\}/);
    expect(rule?.[2]).toMatch(/min-height:\s*40px/);
  });

  it("makes the footer's facts as tall as the phone's PR pill, so the row does not clip it", () => {
    const rule = mobileStyles.match(/([^{}]*\.card__facts[^{}]*)\{([^}]*)\}/);
    expect(rule?.[2]).toMatch(/(^|[^-])height:\s*40px/);
    // A wrapped fact must still start below the 40px, even after a 24px line with no pill.
    expect(rule?.[2]).toMatch(/row-gap:\s*40px/);
  });

  it("clips the line under the name and the footer's facts instead of spilling past a narrow card", () => {
    expect(styles).toMatch(/\.card__sub \{[^}]*overflow:\s*hidden/);
    expect(styles).toMatch(/\.card__sub > :is\(\.card__project, \.card__model\)[^{]*\{[^}]*text-overflow:\s*ellipsis/);
    expect(styles).toMatch(/\.card__facts \{[^}]*overflow:\s*hidden/);
    expect(styles).toMatch(/\.card__facts \{[^}]*flex-wrap:\s*wrap/);
    expect(styles).toMatch(/\.card__sub > \.card__project \{[^}]*flex-shrink:\s*0/);
  });

  it("names a long model in its tooltip, since the line may cut it short", () => {
    const model = "anthropic/claude-sonnet-4-5-20250929";
    expect(renderCard(card({ model }), NOW).querySelector<HTMLElement>(".card__model")!.title).toBe(model);
  });
});
