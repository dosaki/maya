// Keeping the box being typed in above the keyboard. Focusing a box scrolls
// it into view, but the keyboard opens after that: MainActivity then pads the
// web view by the keyboard, the page gets shorter, and nothing scrolls the
// box back up. So each resize while a text box has focus scrolls it to the
// middle of what is left: scrolled only as far as needed, it sat on the
// keyboard's edge with its bottom border under it.

/** Whether typing in `el` opens the keyboard. */
export function isTextField(el: Element | null): el is HTMLElement {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) return !["button", "checkbox", "radio", "range", "submit", "reset", "file", "color", "image"].includes(el.type);
  return el instanceof HTMLElement && el.isContentEditable === true;
}

/** Scrolls the focused text box into view after every resize of `win`. */
export function keepFocusedAboveKeyboard(win: Window = window): void {
  win.addEventListener("resize", () => {
    // After the resize's layout, so the scroll sees the new height.
    win.requestAnimationFrame(() => {
      const el = win.document.activeElement;
      if (isTextField(el)) el.scrollIntoView({ block: "center" });
    });
  });
}

// A session's card sits in a fixed layer, and some web views keep that layer
// the full screen with the keyboard up: its reply box stayed under the
// keyboard, and as the layer does not scroll, nothing could lift it. The
// visual viewport is the part of the page left on show, whatever the web
// view does with its layout, so the layer is sized to it (see mobile.css).

/** The part of the page on show: the visual viewport, or a stand-in. */
export interface VisibleArea extends EventTarget {
  offsetTop: number;
  height: number;
}

/** Keeps `--visible-top` and `--visible-height` on `root` naming the part of the page on show. */
export function trackVisibleArea(root: HTMLElement = document.documentElement, area: VisibleArea | null = window.visualViewport): void {
  if (!area) return;
  const update = () => {
    root.style.setProperty("--visible-top", `${area.offsetTop}px`);
    root.style.setProperty("--visible-height", `${area.height}px`);
  };
  update();
  area.addEventListener("resize", update);
  area.addEventListener("scroll", update);
}
