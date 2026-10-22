import { describe, expect, it } from "vitest";
import { CAPABILITIES, canStop, capabilitiesOf, harnessBadge, harnessLabel } from "./harness";
import type { Card, Harness } from "./types";

describe("capabilities", () => {
  it("mirror the Rust table", () => {
    expect(CAPABILITIES).toEqual({
      "claude-code": { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true, stop: true },
      codex: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: true, shellLines: false, close: true, stop: true },
      antigravity: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false, close: true, stop: false },
      grok: { compact: true, modelSwitch: true, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false, close: true, stop: false },
      kiro: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true, stop: false },
      opencode: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true, stop: true },
      other: { compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false, close: false, stop: false },
    });
    expect(capabilitiesOf("antigravity")).toBe(CAPABILITIES.antigravity);
  });

  it("offer nothing for an agent this build does not know", () => {
    expect(capabilitiesOf("nonesuch" as Harness)).toEqual({ compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false, close: false, stop: false });
  });
});

describe("canStop", () => {
  const card = (over: Partial<Card>): Card => ({ sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "working", stateSince: 0, snippet: "", awaiting: null, hasInbox: false, harness: "claude-code", pr: null, context: null, ...over });
  const asking = (kind: "question" | "plan" | "permission" | "text") => ({ kind, detail: "", questions: [] });

  it("mirrors the Rust table", () => {
    expect(canStop(card({}))).toBe(true);
    for (const kind of ["question", "permission", "plan"] as const) {
      expect(canStop(card({ state: "awaiting", awaiting: asking(kind) }))).toBe(true);
    }
    expect(canStop(card({ shell: true }))).toBe(false);
    expect(canStop(card({ state: "awaiting", awaiting: asking("text") }))).toBe(false);
    expect(canStop(card({ state: "idle" }))).toBe(false);
    expect(canStop(card({ state: "completed" }))).toBe(false);
    expect(canStop(card({ brain: true }))).toBe(false);
    expect(canStop(card({ harness: "grok" }))).toBe(false);
    expect(canStop(card({ harness: "opencode" }))).toBe(true);
  });
});

describe("labels and badges", () => {
  it("name Kiro and show an unknown agent as text", () => {
    expect(harnessLabel("kiro")).toBe("Kiro CLI");
    expect(harnessLabel("opencode")).toBe("OpenCode");
    expect(harnessBadge("opencode", "b").tagName).toBe("IMG");
    expect(harnessLabel("other")).toBe("Unknown agent");
    expect(harnessBadge("kiro", "b").tagName).toBe("IMG");
    const badge = harnessBadge("other", "b");
    expect(badge.tagName).toBe("SPAN");
    expect(badge.textContent).toBe("Unknown agent");
  });
});
