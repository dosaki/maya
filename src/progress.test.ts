import { describe, expect, it } from "vitest";
import { makeProgress } from "./progress";
import type { Card } from "./types";

const c = (stateSince: number): Card => ({ sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince, snippet: "", hasInbox: true, harness: "claude-code", pr: null,
  awaiting: { kind: "question", detail: "q", questions: [] } });

describe("makeProgress", () => {
  it("advances per session and resets when the question set changes", () => {
    const p = makeProgress();
    expect(p.next(c(1))).toBe(0);
    p.advance(c(1));
    expect(p.next(c(1))).toBe(1);
    expect(p.next(c(2))).toBe(0);
    p.reset("s");
    expect(p.next(c(1))).toBe(0);
  });
});
