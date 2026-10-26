import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { addCombo, asKeyBindings, comboLabel, comboOf, composerKey, defaultKeys, removeCombo, sendShortcut, setKeyBindings } from "./keys";

const original = navigator.userAgent;
const pretend = (ua: string) => Object.defineProperty(navigator, "userAgent", { value: ua, configurable: true });
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0";
const LINUX = "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko)";

const enter = (mods: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key: "Enter", cancelable: true, ...mods });

beforeEach(() => {
  pretend(MAC);
  setKeyBindings(defaultKeys());
});

afterEach(() => pretend(original));

describe("the default keys", () => {
  it("send on Enter, and make a new line on Option+Enter and ⌘+Enter on a Mac", () => {
    expect(defaultKeys()).toEqual({ send: ["Enter"], newLine: ["Alt+Enter", "Meta+Enter"] });
  });

  it("make a new line on Alt+Enter and Ctrl+Enter on Windows and Linux", () => {
    for (const ua of [WINDOWS, LINUX]) {
      pretend(ua);
      expect(defaultKeys()).toEqual({ send: ["Enter"], newLine: ["Alt+Enter", "Ctrl+Enter"] });
    }
  });
});

describe("comboOf", () => {
  it("names an Enter press by its modifiers, always in the same order", () => {
    expect(comboOf(enter())).toBe("Enter");
    expect(comboOf(enter({ metaKey: true, altKey: true }))).toBe("Alt+Meta+Enter");
    expect(comboOf(enter({ shiftKey: true, ctrlKey: true }))).toBe("Ctrl+Shift+Enter");
  });

  it("is null for any key other than Enter", () => {
    expect(comboOf(new KeyboardEvent("keydown", { key: "a", metaKey: true }))).toBeNull();
    expect(comboOf(new KeyboardEvent("keydown", { key: "Meta", metaKey: true }))).toBeNull();
  });
});

describe("asKeyBindings", () => {
  it("keeps saved keys and falls back to the defaults when none were saved", () => {
    expect(asKeyBindings({ send: ["Meta+Enter", "Ctrl+Enter"], newLine: [] })).toEqual({ send: ["Meta+Enter", "Ctrl+Enter"], newLine: [] });
    expect(asKeyBindings(undefined)).toEqual(defaultKeys());
    expect(asKeyBindings(null)).toEqual(defaultKeys());
    expect(asKeyBindings("Enter")).toEqual(defaultKeys());
  });

  it("drops what is not an Enter combination, and never leaves Send empty", () => {
    expect(asKeyBindings({ send: ["Enter", "Meta+K", 3, "Hyper+Enter"], newLine: ["Shift+Enter", null] })).toEqual({ send: ["Enter"], newLine: ["Shift+Enter"] });
    expect(asKeyBindings({ send: [], newLine: ["Shift+Enter"] })).toEqual({ send: ["Enter"], newLine: ["Shift+Enter"] });
    expect(asKeyBindings({ send: ["bad"] })).toEqual({ send: ["Enter"], newLine: [] });
  });
});

describe("composerKey", () => {
  function box(value: string, caret: number) {
    const ta = document.createElement("textarea");
    ta.value = value;
    ta.setSelectionRange(caret, caret);
    const send = vi.fn();
    const typed = vi.fn();
    ta.addEventListener("input", typed);
    return { ta, send, typed };
  }

  it("sends on a Send combination and keeps the box from adding a line", () => {
    const { ta, send } = box("hello", 5);
    const ev = enter();
    composerKey(ev, ta, send);
    expect(send).toHaveBeenCalledTimes(1);
    expect(ev.defaultPrevented).toBe(true);
    expect(ta.value).toBe("hello");
  });

  it("puts a new line at the caret on a New line combination, and tells the box it changed", () => {
    const { ta, send, typed } = box("ab", 1);
    const ev = enter({ altKey: true });
    composerKey(ev, ta, send);
    expect(ta.value).toBe("a\nb");
    expect(ta.selectionStart).toBe(2);
    expect(ev.defaultPrevented).toBe(true);
    expect(typed).toHaveBeenCalledTimes(1);
    expect(send).not.toHaveBeenCalled();
  });

  it("leaves a combination in neither list to the box, so Shift+Enter still adds a line by itself", () => {
    const { ta, send } = box("x", 1);
    const ev = enter({ shiftKey: true });
    composerKey(ev, ta, send);
    expect(ev.defaultPrevented).toBe(false);
    expect(send).not.toHaveBeenCalled();
    expect(ta.value).toBe("x");
  });

  it("does nothing while an input method is composing", () => {
    const { ta, send } = box("x", 1);
    const ev = enter({ isComposing: true });
    composerKey(ev, ta, send);
    expect(send).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(false);
  });

  it("follows the keys set last, with several combinations for one action", () => {
    setKeyBindings({ send: ["Meta+Enter", "Ctrl+Enter"], newLine: ["Enter"] });
    const { ta, send } = box("", 0);
    composerKey(enter({ metaKey: true }), ta, send);
    composerKey(enter({ ctrlKey: true }), ta, send);
    expect(send).toHaveBeenCalledTimes(2);
    composerKey(enter(), ta, send);
    expect(send).toHaveBeenCalledTimes(2);
    expect(ta.value).toBe("\n");
  });

  it("adds no line to a one-line input, but still sends from it", () => {
    const input = document.createElement("input");
    input.value = "name";
    const send = vi.fn();
    const ev = enter({ metaKey: true });
    composerKey(ev, input, send);
    expect(ev.defaultPrevented).toBe(true);
    expect(input.value).toBe("name");
    composerKey(enter(), input, send);
    expect(send).toHaveBeenCalledTimes(1);
  });
});

describe("addCombo and removeCombo", () => {
  const keys = { send: ["Enter"], newLine: ["Alt+Enter"] };

  it("adds a combination to the end of its list", () => {
    expect(addCombo(keys, "send", "Meta+Enter")).toEqual({ send: ["Enter", "Meta+Enter"], newLine: ["Alt+Enter"] });
    expect(addCombo(keys, "newLine", "Shift+Enter")).toEqual({ send: ["Enter"], newLine: ["Alt+Enter", "Shift+Enter"] });
  });

  it("refuses a combination the other action uses, and names that action", () => {
    expect(addCombo(keys, "send", "Alt+Enter")).toBe("New line already uses ⌥↵.");
    expect(addCombo(keys, "newLine", "Enter")).toBe("Send already uses ↵.");
  });

  it("refuses a combination the same action already has", () => {
    expect(addCombo(keys, "send", "Enter")).toBe("Send already uses ↵.");
  });

  it("removes a combination, but never the last one Send has", () => {
    expect(removeCombo({ send: ["Enter", "Meta+Enter"], newLine: ["Alt+Enter"] }, "send", "Enter")).toEqual({ send: ["Meta+Enter"], newLine: ["Alt+Enter"] });
    expect(removeCombo(keys, "newLine", "Alt+Enter")).toEqual({ send: ["Enter"], newLine: [] });
    expect(removeCombo(keys, "send", "Enter")).toEqual(keys);
  });
});

describe("labels", () => {
  it("shows Mac symbols on a Mac, in the Mac's modifier order", () => {
    expect(comboLabel("Enter")).toBe("↵");
    expect(comboLabel("Meta+Enter")).toBe("⌘↵");
    expect(comboLabel("Alt+Enter")).toBe("⌥↵");
    expect(comboLabel("Ctrl+Alt+Shift+Meta+Enter")).toBe("⌃⌥⇧⌘↵");
  });

  it("spells the keys out on Windows and Linux, with the system's name for the Meta key", () => {
    pretend(WINDOWS);
    expect(comboLabel("Ctrl+Enter")).toBe("Ctrl+Enter");
    expect(comboLabel("Meta+Enter")).toBe("Win+Enter");
    pretend(LINUX);
    expect(comboLabel("Alt+Shift+Enter")).toBe("Alt+Shift+Enter");
    expect(comboLabel("Meta+Enter")).toBe("Super+Enter");
  });

  it("names the first Send combination as the shortcut", () => {
    expect(sendShortcut()).toBe("↵");
    setKeyBindings({ send: ["Meta+Enter", "Enter"], newLine: [] });
    expect(sendShortcut()).toBe("⌘↵");
    pretend(WINDOWS);
    setKeyBindings({ send: ["Ctrl+Enter"], newLine: [] });
    expect(sendShortcut()).toBe("Ctrl+Enter");
  });
});
