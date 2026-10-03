import { describe, expect, it, vi } from "vitest";
import { CLAUDE_AGENT, optionFields, renderNewSession, type AgentInfo, type NewSessionModel } from "./newsession";

const handlers = () => ({ onStart: vi.fn(), onClose: vi.fn(), onOpenSettings: vi.fn(), onMachine: vi.fn(), onAgent: vi.fn(), onModel: vi.fn() });
const base: NewSessionModel = {
  dirs: ["a", "sonarqube"],
  dir: null,
  prompt: "",
  status: null,
  busy: false,
  needsSetup: false,
  options: {},
  machines: [{ name: "This Mac", value: "" }],
  machine: "",
  agents: [CLAUDE_AGENT],
  agent: "claude-code",
  names: true,
  name: "",
};
const twoMachines = [{ name: "This Mac", value: "" }, { name: "laptop", value: "laptop" }];
const defaults = { agent: "claude-code", name: "", model: "", effort: "", mode: "" };

describe("renderNewSession", () => {
  it("lists Let Maya choose first, then the folders", () => {
    const el = renderNewSession(base, handlers());
    const opts = [...el.querySelectorAll<HTMLOptionElement>("select[name=dir] option")];
    expect(opts.map((o) => o.textContent)).toEqual(["Let Maya choose", "a", "sonarqube"]);
    expect(opts[0].value).toBe("");
  });

  it("starts only with a non-blank prompt, passing the machine, selected folder or null", () => {
    const h = handlers();
    const el = renderNewSession(base, h);
    const start = el.querySelector<HTMLButtonElement>("button[data-action=start]")!;
    const ta = el.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    expect(start.disabled).toBe(true);
    ta.value = "  fix ci  ";
    ta.dispatchEvent(new Event("input"));
    expect(start.disabled).toBe(false);
    start.click();
    expect(h.onStart).toHaveBeenCalledWith("", null, "fix ci", defaults);
    el.querySelector<HTMLSelectElement>("select[name=dir]")!.value = "sonarqube";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onStart).toHaveBeenLastCalledWith("", "sonarqube", "fix ci", defaults);
  });

  it("offers model, effort and mode with Default first and passes the choices to start", () => {
    const h = handlers();
    const el = renderNewSession(base, h);
    const texts = (name: string) => [...el.querySelectorAll<HTMLOptionElement>(`select[name=${name}] option`)].map((o) => o.textContent);
    expect(texts("model")).toEqual(["Default", "Fable", "Opus", "Sonnet", "Haiku"]);
    expect(texts("effort")).toEqual(["Default", "low", "medium", "high", "xhigh", "max"]);
    expect(texts("mode")).toEqual(["Default", "manual", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"]);
    for (const name of ["model", "effort", "mode"]) expect(el.querySelector<HTMLSelectElement>(`select[name=${name}]`)!.value).toBe("");
    el.querySelector<HTMLSelectElement>("select[name=model]")!.value = "opus";
    el.querySelector<HTMLSelectElement>("select[name=effort]")!.value = "high";
    el.querySelector<HTMLSelectElement>("select[name=mode]")!.value = "plan";
    const ta = el.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "go";
    ta.dispatchEvent(new Event("input"));
    el.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("", null, "go", { agent: "claude-code", name: "", model: "opus", effort: "high", mode: "plan" });
  });

  it("preselects the remembered options", () => {
    const el = renderNewSession({ ...base, options: { model: "sonnet", effort: "max", mode: "acceptEdits" } }, handlers());
    expect(el.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("sonnet");
    expect(el.querySelector<HTMLSelectElement>("select[name=effort]")!.value).toBe("max");
    expect(el.querySelector<HTMLSelectElement>("select[name=mode]")!.value).toBe("acceptEdits");
  });

  it("disables Start while busy and ignores clicks, showing the status", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, prompt: "go", busy: true, status: { ok: true, text: "Choosing a repository…" } }, h);
    const start = el.querySelector<HTMLButtonElement>("button[data-action=start]")!;
    expect(start.disabled).toBe(true);
    start.click();
    expect(h.onStart).not.toHaveBeenCalled();
    expect(el.querySelector(".modal__status")?.textContent).toBe("Choosing a repository…");
  });

  it("shows the setup hint with an Open Settings button when no projects directory is set", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, needsSetup: true }, h);
    expect(el.querySelector("textarea")).toBeNull();
    expect(el.querySelector(".modal__setup")?.textContent).toContain("projects directory");
    el.querySelector<HTMLButtonElement>("button[data-action=open-settings]")!.click();
    expect(h.onOpenSettings).toHaveBeenCalled();
  });

  it("closes on backdrop and the close button", () => {
    const h = handlers();
    const el = renderNewSession(base, h);
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=close]")!.click();
    expect(h.onClose).toHaveBeenCalledTimes(2);
  });

  it("renders no Machine picker with a single machine", () => {
    const el = renderNewSession(base, handlers());
    expect(el.querySelector("select[name=machine]")).toBeNull();
  });

  it("offers a Machine picker before the folder select when more than one machine is known", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "" }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=machine]")!;
    expect([...sel.options].map((o) => [o.value, o.textContent])).toEqual([["", "This Mac"], ["laptop", "laptop"]]);
    expect(sel.value).toBe("");
    const fields = [...el.querySelectorAll(".newsession__field")];
    expect(fields[0].querySelector("select[name=machine]")).not.toBeNull();
    expect(fields[1].querySelector("select[name=dir]")).not.toBeNull();
    sel.value = "laptop";
    sel.dispatchEvent(new Event("change"));
    expect(h.onMachine).toHaveBeenCalledWith("laptop");
  });

  it("passes the selected machine to onStart", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "laptop" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    ta.value = "go";
    ta.dispatchEvent(new Event("input"));
    el.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("laptop", "a", "go", defaults);
  });

  it("on a remote machine picks the first folder, and Start stays disabled without one", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "laptop", prompt: "go" }, h);
    expect(el.querySelector<HTMLSelectElement>("select[name=dir]")!.value).toBe("a");
    expect(el.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(false);
    // Its folders have not arrived yet: nothing to pick, nothing to start.
    const empty = renderNewSession({ ...base, dirs: [], machines: twoMachines, machine: "laptop", prompt: "go" }, h);
    expect(empty.querySelector<HTMLSelectElement>("select[name=dir]")!.value).toBe("");
    const start = empty.querySelector<HTMLButtonElement>("button[data-action=start]")!;
    expect(start.disabled).toBe(true);
    start.click();
    expect(h.onStart).not.toHaveBeenCalled();
  });

  it("disables Let Maya choose with a hint when a remote machine is selected", () => {
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "laptop" }, handlers());
    const auto = el.querySelector<HTMLOptionElement>("select[name=dir] option[value='']")!;
    expect(auto.disabled).toBe(true);
    expect(el.querySelector(".newsession__hint")?.textContent).toContain("choose");
    const local = renderNewSession({ ...base, machines: twoMachines, machine: "" }, handlers());
    expect(local.querySelector<HTMLOptionElement>("select[name=dir] option[value='']")!.disabled).toBe(false);
    expect(local.querySelector(".newsession__hint")).toBeNull();
  });
});

const CODEX: AgentInfo = {
  harness: "codex",
  models: [
    { id: "gpt-6.1-sol", label: "GPT-6.1-Sol", efforts: ["low", "medium", "high", "xhigh", "max", "ultra"] },
    { id: "gpt-5.5", label: "GPT-5.5", efforts: ["low", "medium", "high", "xhigh"] },
  ],
  efforts: ["low", "medium", "high", "xhigh"],
  modes: ["read-only", "workspace-write", "danger-full-access"],
};
const GROK: AgentInfo = { harness: "grok", models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: ["default", "plan"] };

describe("agents", () => {
  it("hides the Agent field when Claude Code is the only agent", () => {
    expect(renderNewSession(base, handlers()).querySelector("select[name=agent]")).toBeNull();
  });

  it("lists the installed agents and reports a change", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, CODEX, GROK] }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Claude Code", "Codex", "Grok Build"]);
    sel.value = "codex";
    sel.dispatchEvent(new Event("change"));
    expect(h.onAgent).toHaveBeenCalledWith("codex");
  });

  it("shows only the fields the agent has", () => {
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, GROK], agent: "grok" }, handlers());
    expect(el.querySelector("select[name=effort]")).toBeNull();
    expect([...el.querySelector<HTMLSelectElement>("select[name=mode]")!.options].map((o) => o.value)).toEqual(["", "default", "plan"]);
  });

  it("offers the chosen Codex model's efforts, and the common ones for Default", () => {
    expect(optionFields(CODEX, "gpt-6.1-sol").find((f) => f.name === "effort")!.choices.map(([v]) => v)).toContain("ultra");
    expect(optionFields(CODEX, "").find((f) => f.name === "effort")!.choices.map(([v]) => v)).toEqual(["low", "medium", "high", "xhigh"]);
  });

  it("a_remembered_model_no_longer_listed_falls_back_to_default", () => {
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, CODEX], agent: "codex", options: { model: "gpt-4", effort: "", mode: "" } }, handlers());
    expect(el.querySelector<HTMLSelectElement>("select[name=model]")!.value).toBe("");
  });

  it("starts with the agent and the name", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, agents: [CLAUDE_AGENT, CODEX], agent: "codex", dir: "a", prompt: "go" }, h);
    el.querySelector<HTMLInputElement>("input[name=name]")!.value = "  Fix CI ";
    el.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("", "a", "go", { agent: "codex", name: "Fix CI", model: "", effort: "", mode: "" });
  });

  it("starts from the Name field with the send shortcut", () => {
    const h = handlers();
    const el = renderNewSession({ ...base, dir: "a", prompt: "go" }, h);
    const name = el.querySelector<HTMLInputElement>("input[name=name]")!;
    name.value = "Fix CI";
    name.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onStart).toHaveBeenCalledWith("", "a", "go", { ...defaults, name: "Fix CI" });
  });

  it("an_older_remote_offers_claude_only_and_no_name", () => {
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "laptop", names: false }, handlers());
    expect(el.querySelector("input[name=name]")).toBeNull();
    expect(el.querySelector("select[name=agent]")).toBeNull();
  });
});
