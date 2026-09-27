import { describe, expect, it, vi } from "vitest";
import { patchModal, renderModal } from "./modal";
import type { Card } from "./types";

const base: Card = { sessionId: "s", pid: 1, name: "eye-1", cwd: "/x/dev/eye", state: "working", stateSince: 0, snippet: "", awaiting: null, hasInbox: true };
const handlers = () => ({ onSend: vi.fn(), onTerminal: vi.fn(), onClose: vi.fn() });

describe("patchModal", () => {
  it("keeps the composer node, its focus and text while history, badge and status change", () => {
    document.body.replaceChildren();
    const root = renderModal({ card: base, turns: [{ kind: "user", text: "old" }], status: null, draft: "" }, handlers());
    document.body.append(root);
    const ta = root.querySelector<HTMLTextAreaElement>("textarea")!;
    ta.focus();
    ta.value = "half typed";
    ta.setSelectionRange(4, 4);

    const fresh = renderModal(
      { card: { ...base, state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm" } }, turns: [{ kind: "user", text: "old" }, { kind: "assistant", text: "new" }], status: { ok: true, text: "Delivered" }, draft: "" },
      handlers(),
    );
    patchModal(root, fresh);

    expect(root.querySelector("textarea")).toBe(ta);
    expect(document.activeElement).toBe(ta);
    expect(ta.value).toBe("half typed");
    expect(ta.selectionStart).toBe(4);
    expect([...root.querySelectorAll(".turn")].length).toBe(2);
    expect(root.querySelector(".modal__state")?.textContent).toBe("Awaiting Decision");
    expect(root.querySelector(".modal__banner")).not.toBeNull();
    expect(root.querySelector(".modal__status")?.textContent).toBe("Delivered");
  });

  it("removes the banner and status when the fresh render has none", () => {
    const root = renderModal({ card: { ...base, state: "awaiting", awaiting: { kind: "plan", detail: "x" } }, turns: [], status: { ok: false, text: "e" }, draft: "" }, handlers());
    patchModal(root, renderModal({ card: base, turns: [], status: null, draft: "" }, handlers()));
    expect(root.querySelector(".modal__banner")).toBeNull();
    expect(root.querySelector(".modal__status")).toBeNull();
  });
});
