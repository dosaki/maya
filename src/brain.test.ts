import { describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { brainAgent, defaultAgent } from "./brain";

const info = (h: "claude-code" | "codex" | "grok") => ({ harness: h, models: [], efforts: [], modes: [] });

describe("brain", () => {
  it("reads Maya's agent from the config, Claude Code when unset or unreadable", async () => {
    invoke.mockResolvedValueOnce({ agent: "grok" });
    expect(await brainAgent()).toBe("grok");
    invoke.mockResolvedValueOnce({});
    expect(await brainAgent()).toBe("claude-code");
    invoke.mockRejectedValueOnce(new Error("no backend"));
    expect(await brainAgent()).toBe("claude-code");
  });

  it("defaults to the brain when the machine has it, else the first agent listed", () => {
    expect(defaultAgent([info("claude-code"), info("grok")], "grok")).toBe("grok");
    expect(defaultAgent([info("claude-code"), info("codex")], "grok")).toBe("claude-code");
    expect(defaultAgent([info("codex")], "grok")).toBe("codex");
    expect(defaultAgent([], "grok")).toBe("claude-code");
  });
});
