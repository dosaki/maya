import { describe, expect, it } from "vitest";
import { renderCard } from "./card";
import type { Card } from "./types";

const base: Card = {
  sessionId: "s",
  pid: 1,
  name: "eye-1",
  cwd: "/Users/x/dev/proj",
  state: "idle",
  stateSince: 0,
  snippet: "",
  awaiting: null,
  hasInbox: true,
  harness: "claude-code",
  pr: null,
  context: null,
};

describe("renderCard: remote cards", () => {
  it("marks a remote card with the machine and drops the Terminal button", () => {
    const el = renderCard({ ...base, machine: "laptop", stale: false }, 0);
    expect(el.querySelector(".card__remote")?.getAttribute("title")).toBe("Runs on laptop");
    expect(el.querySelector(".card__project")?.textContent).toBe("proj on laptop");
    expect(el.querySelector("button[data-action=terminal]")).toBeNull();
    expect(el.querySelector("button[data-action=reply]")).not.toBeNull();
    expect(renderCard({ ...base, machine: "laptop", stale: true }, 0).classList.contains("card--stale")).toBe(true);
    expect(renderCard(base, 0).querySelector("button[data-action=terminal]")).not.toBeNull();
  });

  it("shows no remote glyph or machine subtitle for a local card", () => {
    const el = renderCard(base, 0);
    expect(el.querySelector(".card__remote")).toBeNull();
    expect(el.querySelector(".card__project")?.textContent).toBe("proj");
    expect(el.classList.contains("card--stale")).toBe(false);
  });
});
