import { describe, expect, it, vi } from "vitest";
import { renderModal } from "./modal";
import type { Card, Turn } from "./types";

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null };
const turns: Turn[] = [{ kind: "user", text: "hi" }, { kind: "assistant", text: "hello" }, { kind: "tool", text: "Bash: ls" }, { kind: "peer", text: "from eye" }];
const handlers = () => ({ onSend: vi.fn(), onTerminal: vi.fn(), onClose: vi.fn(), onAnswer: vi.fn(), onSetOption: vi.fn(), onCycleMode: vi.fn(), onOpenPr: vi.fn() });

describe("renderModal", () => {
  it("shows header, turns with kind classes and a composer", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns, status: null, draft: "" }, h);
    expect(el.querySelector(".modal__title")?.textContent).toBe("eye-1");
    expect(el.querySelector(".modal__project")?.textContent).toBe("eye");
    expect(el.querySelector(".modal__state")?.textContent).toBe("Idle");
    const kinds = [...el.querySelectorAll(".turn")].map((t) => t.className);
    expect(kinds).toEqual(["turn turn--user", "turn turn--assistant", "turn turn--tool", "turn turn--peer"]);
    expect([...el.querySelectorAll(".turn__who")].map((w) => w.textContent)).toEqual(["You", "Claude", "Message"]);
    expect(el.querySelector("textarea")).not.toBeNull();
    expect(el.querySelector(".modal__banner")).toBeNull();
  });

  it("shows the harness in the header and a PR button only when a PR is known", () => {
    const h = handlers();
    const none = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const badge = none.querySelector<HTMLImageElement>("img.modal__harness")!;
    expect(badge.alt).toBe("Claude Code");
    expect(badge.src).toContain("claude-code");
    expect(none.querySelector("button[data-action=pr]")).toBeNull();
    const el = renderModal({ card: { ...base, pr: { number: 12, url: "https://github.com/o/r/pull/12", state: "draft" } }, turns: [], status: null, draft: "" }, h);
    const btn = el.querySelector<HTMLButtonElement>(".modal__head button[data-action=pr]")!;
    expect(btn.textContent).toBe("PR #12");
    expect(btn.title).toContain("draft");
    btn.click();
    expect(h.onOpenPr).toHaveBeenCalledTimes(1);
  });

  it("applies only the model and effort that were changed, then resets the pickers", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const row = el.querySelector(".modal__tweaks")!;
    expect(row).not.toBeNull();
    const model = row.querySelector<HTMLSelectElement>("select[name=model]")!;
    const effort = row.querySelector<HTMLSelectElement>("select[name=effort]")!;
    expect(model.options[0].textContent).toBe("Model");
    expect(effort.options[0].textContent).toBe("Effort");
    expect([...model.options].map((o) => o.value)).toEqual(["", "fable", "opus", "sonnet", "haiku"]);
    expect([...effort.options].map((o) => o.value)).toEqual(["", "low", "medium", "high", "xhigh", "max"]);
    const apply = row.querySelector<HTMLButtonElement>("button[data-action=apply]")!;
    expect(apply.disabled).toBe(true);
    apply.click();
    expect(h.onSetOption).not.toHaveBeenCalled();
    effort.value = "xhigh";
    effort.dispatchEvent(new Event("change"));
    expect(apply.disabled).toBe(false);
    apply.click();
    expect(h.onSetOption).toHaveBeenCalledTimes(1);
    expect(h.onSetOption).toHaveBeenCalledWith("effort", "xhigh");
    expect(effort.value).toBe("");
    expect(apply.disabled).toBe(true);
    model.value = "opus";
    effort.value = "low";
    model.dispatchEvent(new Event("change"));
    apply.click();
    expect(h.onSetOption).toHaveBeenNthCalledWith(2, "model", "opus");
    expect(h.onSetOption).toHaveBeenNthCalledWith(3, "effort", "low");
  });

  it("has a Cycle mode button that sends Shift+Tab", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=cycle-mode]")!;
    expect(btn.textContent).toContain("mode");
    expect(btn.title).toContain("Shift+Tab");
    btn.click();
    expect(h.onCycleMode).toHaveBeenCalledTimes(1);
  });

  it("still offers the tweaks row when the session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector(".modal__tweaks")).not.toBeNull();
    expect(el.querySelector("textarea")).toBeNull();
  });

  it("sends trimmed text on button click and on Cmd+Enter, never blank", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.value = "   ";
    el.querySelector<HTMLButtonElement>("button[data-action=send]")!.click();
    expect(h.onSend).not.toHaveBeenCalled();
    ta.value = "  reply please  ";
    ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }));
    expect(h.onSend).toHaveBeenCalledWith("reply please");
  });

  it("shows the awaiting banner with a terminal button, and a status line", () => {
    const h = handlers();
    const el = renderModal({ card: { ...base, state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm", questions: [] } }, turns: [], status: { ok: false, text: "boom" }, draft: "" }, h);
    expect(el.querySelector(".modal__banner")?.textContent).toContain("waiting for a decision");
    const asking = { ...base, state: "awaiting" as const, stateSince: 5000, awaiting: { kind: "question" as const, detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "desc a" }] }] } };
    const early = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 5500);
    expect(early.querySelector<HTMLButtonElement>(".modal__banner button[data-action=answer]")!.disabled).toBe(true);
    const q = renderModal({ card: asking, turns: [], status: null, draft: "", next: 0 }, h, 6000);
    expect(q.querySelector(".modal__banner .options__desc")?.textContent).toBe("desc a");
    const btn = q.querySelector<HTMLButtonElement>(".modal__banner button[data-action=answer]")!;
    expect(btn.disabled).toBe(false);
    btn.click();
    expect(h.onAnswer).toHaveBeenCalledWith(0, 0, btn);
    el.querySelector<HTMLButtonElement>(".modal__banner button")!.click();
    expect(h.onTerminal).toHaveBeenCalled();
    expect(el.querySelector(".modal__status")?.textContent).toBe("boom");
    expect(el.querySelector(".modal__status")?.className).toContain("modal__status--error");
  });

  it("replaces the composer when the session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector("textarea")).toBeNull();
    expect(el.querySelector(".modal__noinbox")?.textContent).toContain("no inbox");
  });

  it("closes on backdrop click and on the close button", () => {
    const h = handlers();
    const el = renderModal({ card: base, turns: [], status: null, draft: "" }, h);
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=close]")!.click();
    expect(h.onClose).toHaveBeenCalledTimes(2);
  });
});
