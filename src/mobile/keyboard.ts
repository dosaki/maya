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
