import { describe, expect, it, vi } from "vitest";
import { renderChat, type ChatTurn } from "./chat";
import { defaultKeys, setKeyBindings } from "./keys";

const turn = (who: string, text: string, via: ChatTurn["via"]): ChatTurn => ({ who, text, at: 1, via });

describe("renderChat", () => {
  const idle = { dot: "idle", pending: null, job: null, ready: true } as const;

  it("renders her turns as markdown and keeps the user's as text", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const el = renderChat({ turns: [turn("user", "**hi**", "chat"), turn("maya", "**Done.** See [the PR](https://github.com/o/r/pull/1).", "maya")], status: idle, draft: "" }, h);
    const [user, maya] = el.querySelectorAll(".chat__turn");
    expect(user.textContent).toBe("**hi**");
    expect(maya.querySelector(".md strong")!.textContent).toBe("Done.");
  });

  it("her links open in the browser and unsafe ones are inert", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const el = renderChat({ turns: [turn("maya", "[pr](https://github.com/o/r/pull/1) [bad](javascript:alert(1)) [rel](/x)", "maya")], status: idle, draft: "" }, h);
    const links = el.querySelectorAll("a");
    const ev = new MouseEvent("click", { bubbles: true, cancelable: true });
    links[0].dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(true);
    expect(h.onOpenLink).toHaveBeenCalledWith("https://github.com/o/r/pull/1");
    expect(links[1].hasAttribute("href")).toBe(false);
    expect(links[2].hasAttribute("href")).toBe(false);
  });

  it("renders the conversation, a countdown with Cancel, and Stop while working", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const el = renderChat({ turns: [turn("user", "make it a PR", "voice"), turn("maya", "On it.", "voice")], status: { dot: "working", pending: { say: "Telling api-fix: open the PR.", until: Date.now() + 5000 }, job: "make it a PR", ready: true }, draft: "" }, h);
    expect([...el.querySelectorAll(".chat__turn")].map((t) => t.textContent!.trim())).toEqual(["make it a PR", "On it."]);
    expect(el.querySelector(".chat__turn--user")).not.toBeNull();
    expect(el.querySelector(".chat__turn--maya")).not.toBeNull();
    el.querySelector<HTMLButtonElement>("button[data-action=window-cancel]")!.click();
    expect(h.onCancel).toHaveBeenCalled();
    el.querySelector<HTMLButtonElement>("button[data-action=brain-stop]")!.click();
    expect(h.onStop).toHaveBeenCalled();
  });

  it("sends on Enter and hides Stop when idle", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const el = renderChat({ turns: [], status: { dot: "idle", pending: null, job: null, ready: true }, draft: "" }, h);
    const box = el.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
    box.value = "what's waiting?";
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(h.onSend).toHaveBeenCalledWith("what's waiting?");
    expect(el.querySelector("button[data-action=brain-stop]")).toBeNull();
    expect(el.querySelector("button[data-action=window-cancel]")).toBeNull();
  });

  it("sends and adds lines with the keys chosen in Settings", () => {
    setKeyBindings({ send: ["Alt+Enter", "Meta+Enter"], newLine: ["Enter"] });
    try {
      const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
      const el = renderChat({ turns: [], status: { dot: "idle", pending: null, job: null, ready: true }, draft: "" }, h);
      const box = el.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
      box.value = "one";
      box.setSelectionRange(3, 3);
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", cancelable: true }));
      expect(h.onSend).not.toHaveBeenCalled();
      expect(box.value).toBe("one\n");
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, cancelable: true }));
      expect(h.onSend).toHaveBeenCalledWith("one");
      box.value = "two";
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", altKey: true, cancelable: true }));
      expect(h.onSend).toHaveBeenLastCalledWith("two");
    } finally {
      setKeyBindings(defaultKeys());
    }
  });

  it("keeps Shift+Enter for a new line, ignores blank input and sanitises her markup", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const el = renderChat({ turns: [turn("maya", "<b>hi</b><script>x()</script><img src=x onerror=y()>", "chat")], status: { dot: "idle", pending: null, job: null, ready: true }, draft: "half" }, h);
    const box = el.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
    expect(box.value).toBe("half");
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true }));
    box.value = "   ";
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(h.onSend).not.toHaveBeenCalled();
    expect(el.querySelector(".chat__turn b")!.textContent).toBe("hi");
    expect(el.querySelector(".chat__turn script, .chat__turn img")).toBeNull();
  });

  it("is read-only while her brain is dormant: history and the note, no input, Cancel or Stop", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const el = renderChat({ turns: [turn("user", "make it a PR", "chat"), turn("maya", "Done.", "chat")], status: { dot: "working", pending: { say: "Focusing x.", until: Date.now() + 5000 }, job: "j", ready: false, dormant: "Dormant: desk's Maya is awake." }, draft: "half" }, h);
    expect([...el.querySelectorAll(".chat__turn")].map((t) => t.textContent!.trim())).toEqual(["make it a PR", "Done."]);
    expect(el.querySelector(".chat__dormant")!.textContent).toBe("Dormant: desk's Maya is awake.");
    expect(el.querySelector("textarea")).toBeNull();
    expect(el.querySelector("button[data-action=brain-stop]")).toBeNull();
    expect(el.querySelector("button[data-action=window-cancel]")).toBeNull();
    expect(el.querySelector(".chat__pending")).toBeNull();
  });

  it("a Send button sends the trimmed text and clears the box", () => {
    const h = { onSend: vi.fn(), onCancel: vi.fn(), onStop: vi.fn(), onOpenLink: vi.fn() };
    const idle = { dot: "idle" as const, pending: null, job: null, ready: true };
    const el = renderChat({ turns: [], status: idle, draft: " hi ", sendButton: true }, h);
    const box = el.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
    el.querySelector<HTMLButtonElement>("[data-action=chat-send]")!.click();
    expect(h.onSend).toHaveBeenCalledWith("hi");
    expect(box.value).toBe("");
    el.querySelector<HTMLButtonElement>("[data-action=chat-send]")!.click();
    expect(h.onSend).toHaveBeenCalledTimes(1);
    expect(renderChat({ turns: [], status: idle, draft: "" }, h).querySelector("[data-action=chat-send]")).toBeNull();
  });
});
