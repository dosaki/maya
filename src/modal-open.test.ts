import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { dismissModal, openModal, setOnDismiss, setViewerTheme } from "./modal";
import type { Card } from "./types";

const flush = () => new Promise((r) => setTimeout(r, 0));
const kiro: Card = { sessionId: "k1", pid: 95441, name: "run ls", cwd: "/x", state: "working", stateSince: 0, snippet: "shell: ls", awaiting: null, hasInbox: false, harness: "kiro", pr: null, context: { used: 160_000, window: 200_000, percent: 80 } };

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
    expect(document.querySelector("button[data-action=compact]")).not.toBeNull();
  });
});

describe("the viewer's look", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
  });

  it("the header toggle switches the open panel at once and saves the look", async () => {
    let saved: unknown = null;
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "session_history") return Promise.resolve([]);
      if (cmd === "get_config") return Promise.resolve({ completedTimeoutMinutes: 30, viewerTheme: "chat" });
      if (cmd === "set_config") {
        saved = args?.config;
        return Promise.resolve(args?.config);
      }
      return Promise.reject(new Error(cmd));
    });
    await openModal({ ...kiro, harness: "claude-code" });
    document.querySelector<HTMLButtonElement>("button[data-action=theme]")!.click();
    expect(document.querySelector(".modal")!.classList.contains("modal--terminal")).toBe(true);
    await flush();
    await flush();
    expect(saved).toEqual({ completedTimeoutMinutes: 30, viewerTheme: "terminal" });
    setViewerTheme("chat");
  });
});

describe("dismissModal", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
  });

  it("closes the open window and tells the board once", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "session_history" ? Promise.resolve([]) : Promise.reject(new Error(cmd))));
    const onDismiss = vi.fn();
    setOnDismiss(onDismiss);
    await openModal(kiro);
    dismissModal();
    expect(document.getElementById("modal-host")!.childNodes.length).toBe(0);
    expect(onDismiss).toHaveBeenCalledTimes(1);
    dismissModal();
    expect(onDismiss).toHaveBeenCalledTimes(1);
    setOnDismiss(null);
  });
});
