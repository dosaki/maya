import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { openModal } from "./modal";
import type { Card } from "./types";

const flush = () => new Promise((r) => setTimeout(r, 0));
const kiro: Card = { sessionId: "k1", pid: 95441, name: "run ls", cwd: "/x", state: "working", stateSince: 0, snippet: "shell: ls", awaiting: null, hasInbox: false, harness: "kiro", pr: null, context: null };

describe("openModal for another agent's card", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
  });

  it("fetches the agent list and shows that agent's model and effort pickers", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "session_history") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [{ harness: "kiro", models: [{ id: "auto", label: "auto", efforts: [] }], efforts: ["low", "high"], modes: ["default", "trust-all"] }], names: true });
      return Promise.reject(new Error(cmd));
    });
    await openModal(kiro);
    await flush();
    await flush();
    const model = document.querySelector<HTMLSelectElement>("select[name=model]");
    expect(model, "a model picker").not.toBeNull();
    expect([...model!.options].map((o) => o.value)).toEqual(["", "auto"]);
    expect(document.querySelector("select[name=effort]")).not.toBeNull();
    expect(document.querySelector("button[data-action=compact]")).toBeNull();
  });
});
