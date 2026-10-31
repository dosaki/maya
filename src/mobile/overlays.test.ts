import { describe, expect, it, vi } from "vitest";
import mobileCss from "./mobile.css?raw";

vi.mock("../modal", () => ({ dismissModal: vi.fn() }));
vi.mock("../newsession", () => ({ closeNewSession: vi.fn() }));
vi.mock("../resume", () => ({ closeResume: vi.fn() }));

import { dismissModal } from "../modal";
import { closeNewSession } from "../newsession";
import { closeResume } from "../resume";
import { closeOverlays } from "./overlays";

describe("closeOverlays", () => {
  it("closes the session window, the new-session form and the resume list", () => {
    closeOverlays();
    expect(dismissModal).toHaveBeenCalledTimes(1);
    expect(closeNewSession).toHaveBeenCalledTimes(1);
    expect(closeResume).toHaveBeenCalledTimes(1);
  });
});

describe("the phone's card layer", () => {
  const lines = mobileCss.split("\n");
  const rule = (start: string) => lines.find((l) => l.startsWith(start));

  it("is a pane under the top bar that takes the space left, while it holds something", () => {
    const host = rule("#modal-host:not(:empty) {");
    expect(host).toContain("flex: 1;");
    expect(host).toContain("min-height: 0;");
  });

  it("is not pinned over the screen, nor sized to the visible area", () => {
    const modal = rule(".modal {");
    expect(modal).toContain("position: static;");
    expect(modal).not.toContain("--visible-");
  });

  it("hides the board and the rows above it while it holds something", () => {
    const hide = rule("#app:has(> #modal-host:not(:empty)) >");
    for (const id of ["#board", "#strip-host", "#voice-host", "#remembered"]) expect(hide).toContain(id);
    expect(hide).toContain("display: none;");
  });
});
