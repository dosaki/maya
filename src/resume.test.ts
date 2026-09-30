import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { closeResume, openResume, renderResume } from "./resume";

const NOW = Date.parse("2026-09-29T11:00:00Z");
const handlers = () => ({ onDir: vi.fn(), onResume: vi.fn(), onClose: vi.fn(), onOpenSettings: vi.fn(), onMachine: vi.fn() });
const base = { dirs: ["eye", "maya"], dir: null, sessions: [], loading: false, status: null, needsSetup: false, machines: [{ name: "This Mac", value: "" }], machine: "" };
const twoMachines = [{ name: "This Mac", value: "" }, { name: "laptop", value: "laptop" }];
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

  it("renders no Machine picker with a single machine", () => {
    const el = renderResume(base, handlers(), NOW);
    expect(el.querySelector("select[name=machine]")).toBeNull();
  });

  it("offers a Machine picker before the folder select when more than one machine is known", () => {
    const h = handlers();
    const el = renderResume({ ...base, machines: twoMachines, machine: "" }, h, NOW);
    const sel = el.querySelector<HTMLSelectElement>("select[name=machine]")!;
    expect([...sel.options].map((o) => [o.value, o.textContent])).toEqual([["", "This Mac"], ["laptop", "laptop"]]);
    expect(sel.value).toBe("");
    sel.value = "laptop";
    sel.dispatchEvent(new Event("change"));
    expect(h.onMachine).toHaveBeenCalledWith("laptop");
  });

  it("passes the selected machine to onResume", () => {
    const h = handlers();
    const sessions = [{ id: "a1", title: "T", lastActiveMs: NOW - 1000, running: false }];
    const el = renderResume({ ...base, machines: twoMachines, machine: "laptop", dir: "maya", sessions }, h, NOW);
    el.querySelector<HTMLButtonElement>("button[data-action=resume]")!.click();
    expect(h.onResume).toHaveBeenCalledWith("maya", "a1");
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
    invoke.mockImplementation((cmd: string, args: { dir?: string; machine?: string }) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      if (cmd === "list_project_dirs") return Promise.resolve(["eye", "maya"]);
      if (cmd === "list_resumable_sessions") return Promise.resolve(args.dir === "maya" ? [{ id: "a1", title: "T", lastActiveMs: 1, running: false }] : []);
      if (cmd === "resume_session") return Promise.resolve();
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openResume();
    await flush();
    expect(invoke).toHaveBeenCalledWith("list_project_dirs", { machine: "" });
    const select = document.querySelector<HTMLSelectElement>("select[name=dir]")!;
    select.value = "maya";
    select.dispatchEvent(new Event("change"));
    await flush();
    expect(invoke).toHaveBeenCalledWith("list_resumable_sessions", { dir: "maya", machine: "" });
    const row = document.querySelector<HTMLButtonElement>("button[data-action=resume]")!;
    row.click();
    await flush();
    expect(invoke).toHaveBeenCalledWith("resume_session", { dir: "maya", sessionId: "a1", machine: "" });
    expect(document.querySelector(".modal")).toBeNull();
    await openResume();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=dir]")!.value).toBe("maya");
    expect(invoke).toHaveBeenLastCalledWith("list_resumable_sessions", { dir: "maya", machine: "" });
  });

  it("shows the setup hint when no projects directory is set", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "list_machines") return Promise.resolve([]);
      return cmd === "list_project_dirs" ? Promise.reject("Set a projects directory in Settings first.") : Promise.reject("x");
    });
    await openResume();
    await flush();
    expect(document.querySelector(".modal__setup")).not.toBeNull();
  });

  it("offers a Machine picker once list_machines reports a connected assistant, and reloads dirs for it", async () => {
    invoke.mockImplementation((cmd: string, args: { dir?: string; machine?: string }) => {
      if (cmd === "list_machines") return Promise.resolve([{ name: "laptop", hostname: "laptop.local", platform: "macos", connected: true }]);
      if (cmd === "list_project_dirs") return Promise.resolve(args.machine ? ["remote-proj"] : ["eye", "maya"]);
      if (cmd === "list_resumable_sessions") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openResume();
    await flush();
    const machineSel = document.querySelector<HTMLSelectElement>("select[name=machine]")!;
    expect([...machineSel.options].map((o) => o.textContent)).toEqual(["This Mac", "laptop"]);
    machineSel.value = "laptop";
    machineSel.dispatchEvent(new Event("change"));
    await flush();
    expect(invoke).toHaveBeenLastCalledWith("list_project_dirs", { machine: "laptop" });
    expect([...document.querySelectorAll<HTMLOptionElement>("select[name=dir] option")].map((o) => o.textContent)).toEqual(["Choose a directory…", "remote-proj"]);
  });

  it("keeps the Machine picker when this Mac has no projects directory, and lists a chosen assistant's folders", async () => {
    invoke.mockImplementation((cmd: string, args: { dir?: string; machine?: string }) => {
      if (cmd === "list_machines") return Promise.resolve([{ name: "laptop", hostname: "laptop.local", platform: "macos", connected: true }]);
      if (cmd === "list_project_dirs") return args.machine ? Promise.resolve(["remote-proj"]) : Promise.reject("Set a projects directory in Settings first.");
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openResume();
    await flush();
    expect(document.querySelector(".modal__setup")).not.toBeNull();
    const machineSel = document.querySelector<HTMLSelectElement>("select[name=machine]")!;
    expect([...machineSel.options].map((o) => o.textContent)).toEqual(["This Mac", "laptop"]);
    machineSel.value = "laptop";
    machineSel.dispatchEvent(new Event("change"));
    await flush();
    expect(document.querySelector(".modal__setup")).toBeNull();
    expect([...document.querySelectorAll<HTMLOptionElement>("select[name=dir] option")].map((o) => o.textContent)).toEqual(["Choose a directory…", "remote-proj"]);
    // Back on this Mac, the setup hint returns.
    const again = document.querySelector<HTMLSelectElement>("select[name=machine]")!;
    again.value = "";
    again.dispatchEvent(new Event("change"));
    await flush();
    expect(document.querySelector(".modal__setup")?.textContent).toContain("Set a projects directory in Settings first.");
  });

  it("says to set the projects directory on the assistant when a remote machine lists no folders", async () => {
    invoke.mockImplementation((cmd: string, args: { dir?: string; machine?: string }) => {
      if (cmd === "list_machines") return Promise.resolve([{ name: "laptop", hostname: "laptop.local", platform: "macos", connected: true }]);
      if (cmd === "list_project_dirs") return Promise.resolve(args.machine ? [] : ["eye"]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await openResume();
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
