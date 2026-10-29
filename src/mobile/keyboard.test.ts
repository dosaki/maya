import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import mobileCss from "./mobile.css?raw";
import { isTextField, keepFocusedAboveKeyboard, trackVisibleArea } from "./keyboard";

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

describe("the phone's cards and dialogs", () => {
  it("fill the layer they sit in, which shrinks with the keyboard, not the screen", () => {
    const rule = mobileCss.split("\n").find((l) => l.startsWith(".modal__panel, .modal__panel--compact {"));
    expect(rule).toContain("width: 100%; height: 100%;");
    expect(rule).not.toContain("100vh");
  });
});

describe("trackVisibleArea", () => {
  afterEach(() => {
    document.documentElement.style.removeProperty("--visible-top");
    document.documentElement.style.removeProperty("--visible-height");
  });

  it("names the part of the page left above the keyboard, as it changes", () => {
    const viewport = Object.assign(new EventTarget(), { offsetTop: 0, height: 800 });
    trackVisibleArea(document.documentElement, viewport);
    const style = document.documentElement.style;
    expect(style.getPropertyValue("--visible-top")).toBe("0px");
    expect(style.getPropertyValue("--visible-height")).toBe("800px");
    Object.assign(viewport, { offsetTop: 40, height: 420 });
    viewport.dispatchEvent(new Event("resize"));
    expect(style.getPropertyValue("--visible-top")).toBe("40px");
    expect(style.getPropertyValue("--visible-height")).toBe("420px");
    Object.assign(viewport, { offsetTop: 10 });
    viewport.dispatchEvent(new Event("scroll"));
    expect(style.getPropertyValue("--visible-top")).toBe("10px");
  });
});

describe("the phone's card layer", () => {
  it("covers the part of the page left above the keyboard, not the whole screen", () => {
    const rule = mobileCss.split("\n").find((l) => l.startsWith(".modal {"));
    expect(rule).toContain("top: var(--visible-top, 0px);");
    expect(rule).toContain("height: var(--visible-height, 100%);");
    expect(rule).toContain("bottom: auto;");
  });
});
