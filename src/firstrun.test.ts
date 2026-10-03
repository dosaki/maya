import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { maybeShowFirstRun, needsFirstRun, renderFirstRun } from "./firstrun";

const flush = () => new Promise((r) => setTimeout(r, 0));
const claude = { harness: "claude-code" as const, models: [], efforts: [], modes: [] };
const codex = { harness: "codex" as const, models: [], efforts: [], modes: [] };
const handlers = () => ({ onChoose: vi.fn(), onContinue: vi.fn() });

describe("renderFirstRun", () => {
  it("lists the installed agents with the first preselected, and continues", () => {
    const h = handlers();
    const el = renderFirstRun({ agents: [claude, codex], chosen: "claude-code", saving: false, error: null }, h);
    expect(el.querySelector("h2")?.textContent).toBe("Which agent should power Maya?");
    const radios = el.querySelectorAll<HTMLInputElement>("input[name=agent]");
    expect([...radios].map((r) => r.value)).toEqual(["claude-code", "codex"]);
    expect(radios[0].checked).toBe(true);
    radios[1].click();
    expect(h.onChoose).toHaveBeenCalledWith("codex");
    el.querySelector<HTMLButtonElement>("button[data-action=continue]")!.click();
    expect(h.onContinue).toHaveBeenCalled();
    expect(el.querySelector(".modal__backdrop")).not.toBeNull();
    expect(el.querySelector("button[data-action=close]")).toBeNull();
  });

  it("says when nothing is installed and falls back to Claude Code", () => {
    const el = renderFirstRun({ agents: [], chosen: "claude-code", saving: false, error: null }, handlers());
    expect(el.textContent).toContain("Claude Code, Codex, Antigravity or Grok Build");
    expect(el.querySelector("button[data-action=continue]")?.textContent).toBe("Continue with Claude Code");
    expect(renderFirstRun({ agents: null, chosen: "claude-code", saving: false, error: null }, handlers()).textContent).toContain("Looking for agents");
  });
});

describe("maybeShowFirstRun", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
  });

  it("does nothing when an agent is chosen", async () => {
    expect(needsFirstRun({ agent: "codex" })).toBe(false);
    expect(needsFirstRun({})).toBe(true);
    invoke.mockImplementation((cmd: string) => (cmd === "get_config" ? Promise.resolve({ agent: "codex" }) : Promise.reject(new Error(cmd))));
    await maybeShowFirstRun();
    expect(document.querySelector(".modal")).toBeNull();
  });

  it("asks once, saves the choice with a cleared model for a non-Claude agent, and closes", async () => {
    let config: Record<string, unknown> = { completedTimeoutMinutes: 30, agentModel: "sonnet" };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "list_agents") return Promise.resolve({ agents: [claude, codex], names: true });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      return Promise.reject(new Error("unexpected " + cmd));
    });
    const shown = maybeShowFirstRun();
    await flush();
    expect(document.querySelector("h2")?.textContent).toBe("Which agent should power Maya?");
    await flush();
    document.querySelector<HTMLInputElement>("input[name=agent][value=codex]")!.click();
    document.querySelector<HTMLButtonElement>("button[data-action=continue]")!.click();
    await flush();
    await flush();
    await shown;
    expect(invoke).toHaveBeenCalledWith("set_config", { config: expect.objectContaining({ agent: "codex", agentModel: "" }) });
    expect(document.querySelector(".modal")).toBeNull();
  });
});
