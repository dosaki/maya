import { describe, expect, it, vi } from "vitest";
import { renderNewSession } from "./newsession";

const handlers = () => ({ onStart: vi.fn(), onClose: vi.fn(), onOpenSettings: vi.fn() });
const base = { dirs: ["a", "sonarqube"], dir: null, prompt: "", status: null, busy: false, needsSetup: false };

describe("renderNewSession", () => {
  it("lists Let Claude choose first, then the folders", () => {
    const el = renderNewSession(base, handlers());
    const opts = [...el.querySelectorAll<HTMLOptionElement>("select[name=dir] option")];
    expect(opts.map((o) => o.textContent)).toEqual(["Let Claude choose", "a", "sonarqube"]);
    expect(opts[0].value).toBe("");
  });

  it("starts only with a non-blank prompt, passing the selected folder or null", () => {
    const h = handlers();
    const el = renderNewSession(base, h);
    const start = el.querySelector<HTMLButtonElement>("button[data-action=start]")!;
    const ta = el.querySelector<HTMLTextAreaElement>("textarea[name=prompt]")!;
    expect(start.disabled).toBe(true);
    ta.value = "  fix ci  ";
    ta.dispatchEvent(new Event("input"));
    expect(start.disabled).toBe(false);
    start.click();
    expect(h.onStart).toHaveBeenCalledWith(null, "fix ci");
    el.querySelector<HTMLSelectElement>("select[name=dir]")!.value = "sonarqube";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onStart).toHaveBeenLastCalledWith("sonarqube", "fix ci");
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
});
