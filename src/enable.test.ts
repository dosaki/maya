import { describe, expect, it } from "vitest";
import { nextEnableDelay } from "./options";
import type { Card } from "./types";

const asking = (stateSince: number): Card => ({ sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince, snippet: "", hasInbox: true, harness: "claude-code", pr: null, context: null,
  awaiting: { kind: "question", detail: "Q", questions: [{ question: "Q", header: "H", multiSelect: false, options: [{ label: "A", description: "" }] }] } });

describe("nextEnableDelay", () => {
  it("returns how long until the earliest not-yet-enabled question can be answered", () => {
    expect(nextEnableDelay([asking(1000), asking(1400)], 1200)).toBe(800);
    expect(nextEnableDelay([asking(0)], 5000)).toBeNull();
    expect(nextEnableDelay([{ ...asking(1000), state: "working", awaiting: null }], 1200)).toBeNull();
  });
});
