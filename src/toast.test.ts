import { beforeEach, describe, expect, it, vi } from "vitest";
import { showToast } from "./toast";

describe("showToast", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="toast" class="toast" hidden></div>';
    vi.useFakeTimers();
  });

  it("shows the message then hides after the delay", () => {
    showToast("No Terminal tab found", 1000);
    const t = document.getElementById("toast")!;
    expect(t.hidden).toBe(false);
    expect(t.textContent).toBe("No Terminal tab found");
    vi.advanceTimersByTime(1001);
    expect(t.hidden).toBe(true);
  });

  it("resets the timer when called again", () => {
    showToast("one", 1000);
    vi.advanceTimersByTime(800);
    showToast("two", 1000);
    vi.advanceTimersByTime(800);
    expect(document.getElementById("toast")!.hidden).toBe(false);
    expect(document.getElementById("toast")!.textContent).toBe("two");
  });
});
