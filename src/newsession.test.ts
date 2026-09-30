import { describe, expect, it, vi } from "vitest";
import { renderNewSession } from "./newsession";

const handlers = () => ({ onStart: vi.fn(), onClose: vi.fn(), onOpenSettings: vi.fn(), onMachine: vi.fn() });
const base = { dirs: ["a", "sonarqube"], dir: null, prompt: "", status: null, busy: false, needsSetup: false, options: {}, machines: [{ name: "This Mac", value: "" }], machine: "" };
const twoMachines = [{ name: "This Mac", value: "" }, { name: "laptop", value: "laptop" }];
const defaults = { model: "", effort: "", mode: "" };

describe("renderNewSession", () => {
  it("lists Let Claude choose first, then the folders", () => {
    const el = renderNewSession(base, handlers());
    const opts = [...el.querySelectorAll<HTMLOptionElement>("select[name=dir] option")];
    expect(opts.map((o) => o.textContent)).toEqual(["Let Claude choose", "a", "sonarqube"]);
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
    expect(h.onStart).toHaveBeenCalledWith("", null, "go", { model: "opus", effort: "high", mode: "plan" });
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
    expect(h.onStart).toHaveBeenCalledWith("laptop", null, "go", defaults);
  });

  it("disables Let Claude choose with a hint when a remote machine is selected", () => {
    const el = renderNewSession({ ...base, machines: twoMachines, machine: "laptop" }, handlers());
    const auto = el.querySelector<HTMLOptionElement>("select[name=dir] option[value='']")!;
    expect(auto.disabled).toBe(true);
    expect(el.querySelector(".newsession__hint")?.textContent).toContain("choose");
    const local = renderNewSession({ ...base, machines: twoMachines, machine: "" }, handlers());
    expect(local.querySelector<HTMLOptionElement>("select[name=dir] option[value='']")!.disabled).toBe(false);
    expect(local.querySelector(".newsession__hint")).toBeNull();
  });
});
