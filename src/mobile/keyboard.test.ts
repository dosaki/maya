import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isTextField, keepFocusedAboveKeyboard } from "./keyboard";

describe("isTextField", () => {
  it("takes text boxes and leaves buttons, checkboxes and the page", () => {
    const text = document.createElement("input");
    const box = document.createElement("input");
    box.type = "checkbox";
    expect(isTextField(document.createElement("textarea"))).toBe(true);
    expect(isTextField(text)).toBe(true);
    expect(isTextField(box)).toBe(false);
    expect(isTextField(document.createElement("button"))).toBe(false);
    expect(isTextField(document.body)).toBe(false);
    expect(isTextField(null)).toBe(false);
  });
});

describe("keepFocusedAboveKeyboard", () => {
  let scroll: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll as unknown as Element["scrollIntoView"];
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((f) => {
      f(0);
      return 0;
    });
    keepFocusedAboveKeyboard(window);
  });

  afterEach(() => {
    document.body.replaceChildren();
    vi.restoreAllMocks();
  });

  it("scrolls the focused prompt into view when the keyboard shrinks the page", () => {
    const ta = document.createElement("textarea");
    document.body.append(ta);
    ta.focus();
    window.dispatchEvent(new Event("resize"));
    expect(scroll).toHaveBeenCalledWith({ block: "center" });
    expect(scroll.mock.contexts[0]).toBe(ta);
  });

  it("leaves the page alone when no text box has focus", () => {
    const b = document.createElement("button");
    document.body.append(b);
    b.focus();
    window.dispatchEvent(new Event("resize"));
    expect(scroll).not.toHaveBeenCalled();
  });
});
