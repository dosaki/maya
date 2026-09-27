import { describe, expect, it, vi } from "vitest";
import { renderModal } from "./modal";
import type { Card, Turn } from "./types";

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true };
const turns: Turn[] = [{ kind: "user", text: "hi" }, { kind: "assistant", text: "hello" }, { kind: "tool", text: "Bash: ls" }, { kind: "peer", text: "from eye" }];
const handlers = () => ({ onSend: vi.fn(), onTerminal: vi.fn(), onClose: vi.fn() });

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
