import { beforeEach, describe, expect, it } from "vitest";
import { makeTabs } from "./tabs";

function dom() {
  document.body.innerHTML = `
    <nav><button data-tab="sessions">Sessions <span class="tab__count tab__count--idle" data-count="idle"></span><span class="tab__count tab__count--working" data-count="working"></span><span class="tab__count tab__count--awaiting" data-count="awaiting"></span><span class="tab__count tab__count--completed" data-count="completed"></span></button><button data-tab="reviews">Pull Requests <span id="reviews-count"></span></button><button data-tab="debug">Debug</button><button data-tab="settings">Settings</button></nav>
    <div id="board"></div><div id="reviews" hidden></div><div id="debug" hidden></div><div id="settings" hidden></div>`;
}

describe("makeTabs", () => {
  beforeEach(() => {
    dom();
    localStorage.clear();
  });

  it("starts on Sessions and switches panes and the active button", () => {
    const tabs = makeTabs();
    expect(tabs.current()).toBe("sessions");
    expect(document.getElementById("board")!.hidden).toBe(false);
    expect(document.getElementById("reviews")!.hidden).toBe(true);
    expect(document.querySelector("[data-tab=sessions]")?.classList.contains("tab--active")).toBe(true);
    document.querySelector<HTMLButtonElement>("[data-tab=reviews]")!.click();
    expect(tabs.current()).toBe("reviews");
    expect(document.getElementById("board")!.hidden).toBe(true);
    expect(document.getElementById("reviews")!.hidden).toBe(false);
    expect(document.querySelector("[data-tab=reviews]")?.classList.contains("tab--active")).toBe(true);
  });

  it("shows the Debug and Settings panes as tabs, hiding the others", () => {
    const tabs = makeTabs();
    document.querySelector<HTMLButtonElement>("[data-tab=settings]")!.click();
    expect(tabs.current()).toBe("settings");
    expect(document.getElementById("settings")!.hidden).toBe(false);
    expect(document.getElementById("board")!.hidden).toBe(true);
    expect(document.getElementById("debug")!.hidden).toBe(true);
    document.querySelector<HTMLButtonElement>("[data-tab=debug]")!.click();
    expect(tabs.current()).toBe("debug");
    expect(document.getElementById("debug")!.hidden).toBe(false);
    expect(document.getElementById("settings")!.hidden).toBe(true);
    dom();
    expect(makeTabs().current()).toBe("debug");
  });

  it("remembers the chosen tab across restarts, but never a bad value", () => {
    makeTabs().show("reviews");
    dom();
    expect(makeTabs().current()).toBe("reviews");
    localStorage.setItem("maya.tab", "nonsense");
    dom();
    expect(makeTabs().current()).toBe("sessions");
  });

  it("shows one badge per column on the Sessions tab, zeros included, with tooltips", () => {
    const tabs = makeTabs();
    tabs.setSessionCounts({ idle: 12, working: 3, awaiting: 0, completed: 1 });
    const badge = (state: string) => document.querySelector<HTMLElement>(`[data-count=${state}]`)!;
    expect(badge("idle").textContent).toBe("12");
    expect(badge("working").textContent).toBe("3");
    expect(badge("awaiting").textContent).toBe("0");
    expect(badge("completed").textContent).toBe("1");
    expect(badge("awaiting").title).toBe("0 awaiting decision");
    expect(badge("idle").title).toBe("12 idle");
  });

  it("shows the PR count on the tab, blank at zero", () => {
    const tabs = makeTabs();
    tabs.setCount(3);
    expect(document.getElementById("reviews-count")!.textContent).toBe("3");
    tabs.setCount(0);
    expect(document.getElementById("reviews-count")!.textContent).toBe("");
  });
});
