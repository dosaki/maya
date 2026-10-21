import { describe, expect, it, vi } from "vitest";
import { renderChat, type ChatTurn } from "./chat";

const turn = (who: string, text: string, via: ChatTurn["via"]): ChatTurn => ({ who, text, at: 1, via });

describe("renderChat", () => {
  it("renders the conversation, a countdown with Cancel, and Stop while working", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn() };
    const el = renderChat({ turns: [turn("user", "make it a PR", "voice"), turn("maya", "On it.", "voice")], status: { dot: "working", pending: { say: "Telling api-fix: open the PR.", until: Date.now() + 5000 }, job: "make it a PR", ready: true }, draft: "" }, h);
    expect([...el.querySelectorAll(".chat__turn")].map((t) => t.textContent)).toEqual(["make it a PR", "On it."]);
    expect(el.querySelector(".chat__turn--user")).not.toBeNull();
    expect(el.querySelector(".chat__turn--maya")).not.toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=window-cancel]")!.click();
    expect(h.onCancel).toHaveBeenCalled();
    el.querySelector<HTMLButtonElement>("button[data-action=brain-stop]")!.click();
    expect(h.onStop).toHaveBeenCalled();
  });

  it("sends on Enter and hides Stop when idle", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn() };
    const el = renderChat({ turns: [], status: { dot: "idle", pending: null, job: null, ready: true }, draft: "" }, h);
    const box = el.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
    box.value = "what's waiting?";
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(h.onSend).toHaveBeenCalledWith("what's waiting?");
    expect(el.querySelector("button[data-action=brain-stop]")).toBeNull();
    expect(el.querySelector("button[data-action=window-cancel]")).toBeNull();
  });

  it("keeps Shift+Enter for a new line, ignores blank input and never reads her text as markup", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn() };
    const el = renderChat({ turns: [turn("maya", "<b>hi</b>", "chat")], status: { dot: "idle", pending: null, job: null, ready: true }, draft: "half" }, h);
    const box = el.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
    expect(box.value).toBe("half");
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true }));
    box.value = "   ";
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(h.onSend).not.toHaveBeenCalled();
    expect(el.querySelector(".chat__turn b")).toBeNull();
    expect(el.querySelector(".chat__turn")!.textContent).toBe("<b>hi</b>");
  });

  it("is read-only while her brain is dormant: history and the note, no input, Cancel or Stop", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn() };
    const el = renderChat({ turns: [turn("user", "make it a PR", "chat"), turn("maya", "Done.", "chat")], status: { dot: "working", pending: { say: "Focusing x.", until: Date.now() + 5000 }, job: "j", ready: false, dormant: "Dormant: desk's Maya is awake." }, draft: "half" }, h);
    expect([...el.querySelectorAll(".chat__turn")].map((t) => t.textContent)).toEqual(["make it a PR", "Done."]);
    expect(el.querySelector(".chat__dormant")!.textContent).toBe("Dormant: desk's Maya is awake.");
    expect(el.querySelector("textarea")).toBeNull();
    expect(el.querySelector("button[data-action=brain-stop]")).toBeNull();
    expect(el.querySelector("button[data-action=window-cancel]")).toBeNull();
    expect(el.querySelector(".chat__pending")).toBeNull();
  });
});
