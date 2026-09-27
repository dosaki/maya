import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { closeNewSession, openNewSession } from "./newsession";

const flush = () => new Promise((r) => setTimeout(r, 0));

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

  it("clears the prompt after a successful start, keeps Start disabled, ignores a second click and closes", async () => {
    let resolveStart: (v: unknown) => void = () => {};
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
      if (cmd === "start_session") return new Promise((r) => { resolveStart = r; });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openNewSession();
    await flush();
    const ta = document.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "fix ci";
    ta.dispatchEvent(new Event("input"));
    document.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(invoke).toHaveBeenCalledWith("start_session", { dir: null, prompt: "fix ci" });
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
      if (cmd === "list_project_dirs") return Promise.resolve(["a"]);
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
