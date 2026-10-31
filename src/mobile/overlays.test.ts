import { beforeEach, describe, expect, it, vi } from "vitest";
import mobileCss from "./mobile.css?raw";

vi.mock("../modal", () => ({ dismissModal: vi.fn() }));
vi.mock("../newsession", () => ({ closeNewSession: vi.fn() }));
vi.mock("../resume", () => ({ closeResume: vi.fn() }));

import { dismissModal } from "../modal";
import { closeNewSession } from "../newsession";
import { closeResume } from "../resume";
import { makeTabs } from "../tabs";
import { closeOverlays, closeOverlaysOnTabTaps } from "./overlays";

describe("closeOverlays", () => {
  it("closes the session window, the new-session form and the resume list", () => {
    closeOverlays();
    expect(dismissModal).toHaveBeenCalledTimes(1);
    expect(closeNewSession).toHaveBeenCalledTimes(1);
    expect(closeResume).toHaveBeenCalledTimes(1);
  });
});

describe("closeOverlaysOnTabTaps", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.innerHTML = '<button data-tab="sessions"></button><button data-tab="chat"></button><div id="board"></div><section id="chat"></section>';
  });

  it("closes what is open when a tab is tapped, Sessions included", () => {
    makeTabs();
    closeOverlaysOnTabTaps();
    document.querySelector<HTMLElement>("[data-tab=chat]")!.click();
    document.querySelector<HTMLElement>("[data-tab=sessions]")!.click();
    expect(dismissModal).toHaveBeenCalledTimes(2);
    expect(closeNewSession).toHaveBeenCalledTimes(2);
    expect(closeResume).toHaveBeenCalledTimes(2);
  });

  it("leaves a card alone when the page switches tab itself, as a notification tap does after opening it", () => {
    const tabs = makeTabs();
    closeOverlaysOnTabTaps();
    tabs.show("sessions");
    expect(dismissModal).not.toHaveBeenCalled();
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
    // In the page, but still the box an edge drag's absolute panel is placed in, not the screen.
    expect(modal).toContain("position: relative;");
    expect(modal).not.toContain("--visible-");
  });

  it("hides the board and the rows above it while it holds something", () => {
    const hide = rule("#app:has(> #modal-host:not(:empty)) >");
    for (const id of ["#board", "#strip-host", "#voice-host", "#remembered"]) expect(hide).toContain(id);
    expect(hide).toContain("display: none;");
  });
});
