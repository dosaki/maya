import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { CLAUDE_AGENT, closeNewSession, openNewSession } from "./newsession";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("new-session options", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div><div id="toast" class="toast" hidden></div>';
    invoke.mockReset();
  });
  afterEach(() => closeNewSession());

  it("sends the chosen options and remembers them for the next open", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
      if (cmd === "start_session") return Promise.resolve({ dir: "/x/dev/a", how: "chosen" });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    document.querySelector<HTMLSelectElement>("select[name=model]")!.value = "opus";
    document.querySelector<HTMLSelectElement>("select[name=mode]")!.value = "plan";
    // Options survive a close before any start, like the prompt draft.
    closeNewSession();
    await openNewSession();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("opus");
    document.querySelector<HTMLSelectElement>("select[name=dir]")!.value = "a";
    const ta = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "fix ci";
    ta.dispatchEvent(new Event("input"));
    document.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(invoke).toHaveBeenCalledWith("start_session", { dir: "a", prompt: "fix ci", options: { agent: "claude-code", name: "", model: "opus", effort: "", mode: "plan" }, machine: "" });
    await flush();
    await flush();
    closeNewSession();
    await openNewSession();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("opus");
    expect(document.querySelector<HTMLSelectElement>("select[name=mode]")!.value).toBe("plan");
    // Remembered options are module state: put them back to Default for the other tests.
    for (const sel of document.querySelectorAll<HTMLSelectElement>("select")) sel.value = "";
  });

  it("enables Start only once the agent listing has landed, so a prompt cannot start under the wrong agent", async () => {
    let settle: (v: { agents: unknown[]; names: boolean }) => void = () => {};
    const listing = new Promise<{ agents: unknown[]; names: boolean }>((r) => { settle = r; });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return listing;
      if (cmd === "get_config") return Promise.resolve({ agent: "claude-code" });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const ta = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "fix the build";
    ta.dispatchEvent(new Event("input"));
    expect(document.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(true);
    settle({ agents: [CLAUDE_AGENT], names: true });
    await flush();
    await flush();
    expect(document.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(false);
    closeNewSession();
  });

  it("defaults to Maya's agent, and remembers each agent's options", async () => {
    const codex = { harness: "codex", models: [{ id: "gpt-5.5", label: "GPT-5.5", efforts: ["low"] }], efforts: ["low"], modes: ["read-only"] };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT, codex], names: true });
      if (cmd === "get_config") return Promise.resolve({ agent: "codex" });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    // Maya's own agent (codex, from the config) is offered without choosing it.
    expect(document.querySelector<HTMLSelectElement>("select[name=agent]")!.value).toBe("codex");
    document.querySelector<HTMLSelectElement>("select[name=model]")!.value = "gpt-5.5";
    closeNewSession();
    await openNewSession();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=agent]")!.value).toBe("codex");
    expect(document.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("gpt-5.5");
    // Back to Claude Code with Default options for the other tests.
    const back = document.querySelector<HTMLSelectElement>("select[name=agent]")!;
    back.value = "claude-code";
    back.dispatchEvent(new Event("change"));
    for (const sel of document.querySelectorAll<HTMLSelectElement>("select[name=model], select[name=effort], select[name=mode]")) sel.value = "";
  });

  it("a list_agents failure leaves Claude Code alone", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      return Promise.reject(new Error("no"));
    });
    await openNewSession();
    await flush();
    expect(document.querySelector("select[name=agent]")).toBeNull();
    expect(document.querySelector("select[name=model]")).not.toBeNull();
    // This Mac's Maya takes names whatever the listing did.
    expect(document.querySelector("input[name=name]")).not.toBeNull();
  });

  it("keeps the focus and the typed name when the agents arrive", async () => {
    let resolveAgents: (v: unknown) => void = () => {};
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return new Promise((r) => { resolveAgents = r; });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const name = document.querySelector<HTMLInputElement>("input[name=name]")!;
    name.focus();
    name.value = "Fix CI";
    name.dispatchEvent(new Event("input"));
    resolveAgents({ agents: [CLAUDE_AGENT], names: true });
    await flush();
    const after = document.querySelector<HTMLInputElement>("input[name=name]")!;
    const focused = document.activeElement;
    const typed = after.value;
    // The name is a draft like the prompt: clear it for the other tests, even if this one fails.
    after.value = "";
    expect(after).not.toBe(name);
    expect(focused).toBe(after);
    expect(typed).toBe("Fix CI");
    // Changing the model repaints too, and the Model select keeps the focus.
    const model = document.querySelector<HTMLSelectElement>("select[name=model]")!;
    model.focus();
    model.value = "opus";
    model.dispatchEvent(new Event("change"));
    const modelAfter = document.querySelector<HTMLSelectElement>("select[name=model]")!;
    const focusedAfter = document.activeElement;
    modelAfter.value = "";
    expect(modelAfter).not.toBe(model);
    expect(focusedAfter).toBe(modelAfter);
  });
});

describe("new-session flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div><div id="toast" class="toast" hidden></div>';
    invoke.mockReset();
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    closeNewSession();
    vi.useRealTimers();
  });

  it("says a session with no clear folder starts in the projects directory, naming no agent", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
      if (cmd === "start_session") return Promise.resolve({ dir: "/x/dev", how: "fallback" });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const ta = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "fix ci";
    ta.dispatchEvent(new Event("input"));
    document.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    await flush();
    await flush();
    expect(document.querySelector(".modal__status")?.textContent).toBe("Started in dev (no clear match, so the projects directory itself)");
  });

  it("clears the prompt after a successful start, keeps Start disabled, ignores a second click and closes", async () => {
    let resolveStart: (v: unknown) => void = () => {};
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
      if (cmd === "start_session") return new Promise((r) => { resolveStart = r; });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const ta = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "fix ci";
    ta.dispatchEvent(new Event("input"));
    document.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(invoke).toHaveBeenCalledWith("start_session", { dir: null, prompt: "fix ci", options: { agent: "claude-code", name: "", model: "", effort: "", mode: "" }, machine: "" });
    resolveStart({ dir: "/x/dev/a", how: "classifier" });
    await flush();
    await flush();
    const after = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    expect(after.value).toBe("");
    expect(document.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(true);
    expect(document.querySelector(".modal__status")?.textContent).toContain("Started in a");
    document.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(invoke.mock.calls.filter((c) => c[0] === "start_session").length).toBe(1);
    await vi.advanceTimersByTimeAsync(1600);
    expect(document.querySelector(".modal")).toBeNull();
    await openNewSession();
    await flush();
    expect(document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!.value).toBe("");
  });

  it("keeps the draft and reports the outcome when the modal is closed mid-start", async () => {
    let rejectStart: (e: unknown) => void = () => {};
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
      if (cmd === "start_session") return new Promise((_, rej) => { rejectStart = rej; });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const ta = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "deploy it";
    ta.dispatchEvent(new Event("input"));
    document.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    closeNewSession();
    rejectStart("boom");
    await flush();
    const toast = document.getElementById("toast")!;
    expect(toast.hidden).toBe(false);
    expect(toast.textContent).toBe("boom");
    await openNewSession();
    await flush();
    expect(document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!.value).toBe("deploy it");
  });
});

describe("new-session machine picker", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div><div id="toast" class="toast" hidden></div>';
    invoke.mockReset();
  });
  afterEach(() => closeNewSession());

  it("keeps the Machine picker when this Mac has no projects directory, and lists a chosen assistant's folders", async () => {
    invoke.mockImplementation((cmd: string, args: { machine?: string }) => {
      if (cmd === "list_machines") return Promise.resolve([{ name: "laptop", hostname: "laptop.local", platform: "macos", connected: true }]);
      if (cmd === "list_project_dirs") return args.machine ? Promise.resolve(["remote-proj"]) : Promise.reject("Set a projects directory in Settings first.");
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    expect(document.querySelector(".modal__setup")).not.toBeNull();
    expect(document.querySelector("textarea[name=prompt]")).toBeNull();
    const machineSel = document.querySelector<HTMLSelectElement>("select[name=machine]")!;
    expect([...machineSel.options].map((o) => o.textContent)).toEqual(["This Mac", "laptop"]);
    machineSel.value = "laptop";
    machineSel.dispatchEvent(new Event("change"));
    await flush();
    expect(document.querySelector(".modal__setup")).toBeNull();
    expect([...document.querySelectorAll<HTMLOptionElement>("select[name=dir] option")].map((o) => o.value)).toEqual(["", "remote-proj"]);
    expect(document.querySelector<HTMLSelectElement>("select[name=dir]")!.value).toBe("remote-proj");
    // Back on this Mac, the setup hint returns.
    const again = document.querySelector<HTMLSelectElement>("select[name=machine]")!;
    again.value = "";
    again.dispatchEvent(new Event("change"));
    await flush();
    expect(document.querySelector(".modal__setup")?.textContent).toContain("Set a projects directory in Settings first.");
  });

  it("says to set the projects directory on the assistant when a remote machine lists no folders", async () => {
    invoke.mockImplementation((cmd: string, args: { machine?: string }) => {
      if (cmd === "list_machines") return Promise.resolve([{ name: "laptop", hostname: "laptop.local", platform: "macos", connected: true }]);
      if (cmd === "list_project_dirs") return Promise.resolve(args.machine ? [] : ["a"]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const machineSel = document.querySelector<HTMLSelectElement>("select[name=machine]")!;
    machineSel.value = "laptop";
    machineSel.dispatchEvent(new Event("change"));
    await flush();
    const setup = document.querySelector(".modal__setup")!;
    expect(setup.textContent).toContain("Set a projects directory in Settings on laptop");
    expect(setup.querySelector("button[data-action=open-settings]")).toBeNull();
    expect(document.querySelector("select[name=machine]")).not.toBeNull();
  });
});
