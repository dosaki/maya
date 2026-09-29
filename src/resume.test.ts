import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { closeResume, openResume, renderResume } from "./resume";

const NOW = Date.parse("2026-09-29T11:00:00Z");
const handlers = () => ({ onDir: vi.fn(), onResume: vi.fn(), onClose: vi.fn(), onOpenSettings: vi.fn() });
const base = { dirs: ["eye", "maya"], dir: null, sessions: [], loading: false, status: null, needsSetup: false };
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("renderResume", () => {
  it("asks for a directory first, with no 'let Claude choose' entry, and reports the pick", () => {
    const h = handlers();
    const el = renderResume(base, h, NOW);
    const select = el.querySelector<HTMLSelectElement>("select[name=dir]")!;
    expect([...select.options].map((o) => o.textContent)).toEqual(["Choose a directory…", "eye", "maya"]);
    expect(el.querySelector(".resume__hint")?.textContent).toContain("Pick a directory");
    select.value = "maya";
    select.dispatchEvent(new Event("change"));
    expect(h.onDir).toHaveBeenCalledWith("maya");
  });

  it("lists sessions newest first with title and age, disables running ones, and resumes on click", () => {
    const h = handlers();
    const sessions = [
      { id: "a1", title: "Add resume button", lastActiveMs: NOW - 120_000, running: false },
      { id: "b2", title: "Live one", lastActiveMs: NOW - 3_600_000, running: true },
    ];
    const el = renderResume({ ...base, dir: "maya", sessions }, h, NOW);
    const rows = el.querySelectorAll<HTMLButtonElement>("button[data-action=resume]");
    expect(rows.length).toBe(2);
    expect(rows[0].querySelector(".resume__title")?.textContent).toBe("Add resume button");
    expect(rows[0].querySelector(".resume__age")?.textContent).toBe("2m ago");
    expect(rows[0].disabled).toBe(false);
    expect(rows[1].disabled).toBe(true);
    expect(rows[1].querySelector(".resume__chip")?.textContent).toBe("running");
    rows[1].click();
    expect(h.onResume).not.toHaveBeenCalled();
    rows[0].click();
    expect(h.onResume).toHaveBeenCalledWith("maya", "a1");
  });

  it("shows loading, empty and setup states", () => {
    const h = handlers();
    expect(renderResume({ ...base, dir: "maya", loading: true }, h, NOW).querySelector(".resume__hint")?.textContent).toContain("Loading");
    expect(renderResume({ ...base, dir: "maya", sessions: [] }, h, NOW).querySelector(".resume__hint")?.textContent).toContain("No sessions");
    const setup = renderResume({ ...base, needsSetup: true }, h, NOW);
    expect(setup.querySelector("select")).toBeNull();
    setup.querySelector<HTMLButtonElement>("button[data-action=open-settings]")!.click();
    expect(h.onOpenSettings).toHaveBeenCalled();
  });
});

describe("resume flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div><div id="toast" class="toast" hidden></div>';
    invoke.mockReset();
    localStorage.clear();
  });
  afterEach(() => closeResume());

  it("loads directories, then sessions for the chosen one, resumes on click and remembers the directory", async () => {
    invoke.mockImplementation((cmd: string, args: { dir?: string }) => {
      if (cmd === "list_project_dirs") return Promise.resolve(["eye", "maya"]);
      if (cmd === "list_resumable_sessions") return Promise.resolve(args.dir === "maya" ? [{ id: "a1", title: "T", lastActiveMs: 1, running: false }] : []);
      if (cmd === "resume_session") return Promise.resolve();
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openResume();
    await flush();
    expect(invoke).toHaveBeenCalledWith("list_project_dirs");
    const select = document.querySelector<HTMLSelectElement>("select[name=dir]")!;
    select.value = "maya";
    select.dispatchEvent(new Event("change"));
    await flush();
    expect(invoke).toHaveBeenCalledWith("list_resumable_sessions", { dir: "maya" });
    const row = document.querySelector<HTMLButtonElement>("button[data-action=resume]")!;
    row.click();
    await flush();
    expect(invoke).toHaveBeenCalledWith("resume_session", { dir: "maya", sessionId: "a1" });
    expect(document.querySelector(".modal")).toBeNull();
    await openResume();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=dir]")!.value).toBe("maya");
    expect(invoke).toHaveBeenLastCalledWith("list_resumable_sessions", { dir: "maya" });
  });

  it("shows the setup hint when no projects directory is set", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "list_project_dirs" ? Promise.reject("Set a projects directory in Settings first.") : Promise.reject("x")));
    await openResume();
    await flush();
    expect(document.querySelector(".modal__setup")).not.toBeNull();
  });
});
