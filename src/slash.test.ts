import { beforeEach, describe, expect, it, vi } from "vitest";

const morphed = vi.hoisted(() => ({ count: 0 }));
vi.mock("./morph", async (actual) => {
  const real = await actual<typeof import("./morph")>();
  return { ...real, morph: (...args: Parameters<typeof real.morph>) => (morphed.count++, real.morph(...args)) };
});

import { attachSlashMenu, matchCommands, slashQuery } from "./slash";
import type { CommandInfo } from "./types";

const cmd = (name: string, source = "builtin"): CommandInfo => ({ name, description: `${name} does it`, source });
const key = (k: string, extra: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key: k, cancelable: true, bubbles: true, ...extra });

let box: HTMLTextAreaElement;
let host: HTMLElement;
function setup(list = [cmd("compact"), cmd("clear"), cmd("review", "plugin")]) {
  host = document.createElement("div");
  box = document.createElement("textarea");
  host.append(box);
  document.body.append(host);
  const menu = attachSlashMenu(box);
  host.append(menu.element);
  menu.setCommands(list);
  return menu;
}
function type(value: string, caret = value.length) {
  box.value = value;
  box.setSelectionRange(caret, caret);
  box.dispatchEvent(new Event("input", { bubbles: true }));
}
const rows = () => [...host.querySelectorAll<HTMLElement>("[role=option]")];

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("slashQuery", () => {
  it("reads the typed word only in the first word", () => {
    expect(slashQuery("/co", 3)).toBe("co");
    expect(slashQuery("/", 1)).toBe("");
    expect(slashQuery("/compact now", 12)).toBeNull();
    expect(slashQuery("hello /co", 9)).toBeNull();
    expect(slashQuery("hi\n/co", 6)).toBeNull();
  });
});

describe("matchCommands", () => {
  it("matches the start of a name first, then anywhere", () => {
    expect(matchCommands([cmd("code-review"), cmd("review"), cmd("rename")], "re").map((c) => c.name)).toEqual(["review", "rename", "code-review"]);
    expect(matchCommands([cmd("a"), cmd("b")], "")).toHaveLength(2);
  });

  it("counts the start of a plugin command's own name, after its last colon, as a start", () => {
    expect(matchCommands([cmd("unbrain"), cmd("superpowers:brainstorming", "plugin")], "brain").map((c) => c.name)).toEqual(["superpowers:brainstorming", "unbrain"]);
  });
});

describe("the menu", () => {
  it("opens for a slash at the start and not elsewhere", () => {
    const menu = setup();
    type("/");
    expect(rows()).toHaveLength(3);
    expect(box.getAttribute("aria-expanded")).toBe("true");
    type("hello /co");
    expect(rows()).toHaveLength(0);
    expect(box.getAttribute("aria-expanded")).toBe("false");
    type("/compact now", 12);
    expect(menu.element.hidden).toBe(true);
  });

  it("Tab fills the highlighted command and keeps the rest", () => {
    const menu = setup();
    type("/co rest", 3);
    const ev = key("Tab");
    expect(menu.onKey(ev)).toBe(true);
    expect(ev.defaultPrevented).toBe(true);
    expect(box.value).toBe("/compact rest");
    expect(box.selectionStart).toBe(9);
  });

  it("Tab on a bare word fills with a trailing space", () => {
    const menu = setup();
    type("/co");
    menu.onKey(key("Tab"));
    expect(box.value).toBe("/compact ");
    expect(box.selectionStart).toBe(9);
  });

  it("arrows move and wrap", () => {
    const menu = setup();
    type("/");
    expect(rows()[0].getAttribute("aria-selected")).toBe("true");
    menu.onKey(key("ArrowUp"));
    expect(rows()[2].getAttribute("aria-selected")).toBe("true");
    menu.onKey(key("ArrowDown"));
    expect(rows()[0].getAttribute("aria-selected")).toBe("true");
    expect(box.getAttribute("aria-activedescendant")).toBe(rows()[0].id);
  });

  it("Enter fills a partial name but sends a whole one", () => {
    const menu = setup();
    type("/co");
    expect(menu.onKey(key("Enter"))).toBe(true);
    expect(box.value).toBe("/compact ");
    type("/compact");
    expect(menu.onKey(key("Enter"))).toBe(false);
  });

  it("Escape closes the list and stops the event", () => {
    const menu = setup();
    type("/");
    const ev = key("Escape");
    let stopped = false;
    ev.stopPropagation = () => {
      stopped = true;
    };
    expect(menu.onKey(ev)).toBe(true);
    expect(stopped).toBe(true);
    expect(rows()).toHaveLength(0);
    expect(menu.onKey(key("Escape"))).toBe(false);
  });

  it("ignores keys while composing", () => {
    const menu = setup();
    type("/co");
    expect(menu.onKey(key("Tab", { isComposing: true }))).toBe(false);
    expect(box.value).toBe("/co");
  });

  it("a click fills without moving focus out of the box", () => {
    setup();
    type("/co");
    const row = rows()[0];
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    row.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    row.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(box.value).toBe("/compact ");
  });

  it("shows the source of plugin and project commands", () => {
    setup();
    type("/rev");
    expect(rows()[0].textContent).toContain("· plugin");
    expect(rows()[0].textContent).toContain("/review");
  });

  it("a new list keeps the open list's rows and highlight", () => {
    const menu = setup([cmd("compact"), cmd("clear")]);
    type("/");
    menu.onKey(key("ArrowDown"));
    const first = rows()[0];
    menu.setCommands([cmd("compact"), cmd("clear"), cmd("cost")]);
    expect(rows()).toHaveLength(3);
    expect(rows()[0]).toBe(first);
    expect(rows()[1].getAttribute("aria-selected")).toBe("true");
  });

  it("opens when the list arrives after the slash was typed", () => {
    const menu = setup([]);
    box.focus();
    type("/");
    expect(rows()).toHaveLength(0);
    menu.setCommands([cmd("compact")]);
    expect(rows()).toHaveLength(1);
  });

  it("closes on blur, but a row click still fills", () => {
    setup();
    box.focus();
    type("/co");
    rows()[0].dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    rows()[0].dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(box.value).toBe("/compact ");
    type("/co");
    box.dispatchEvent(new Event("blur"));
    expect(rows()).toHaveLength(0);
  });

  it("leaves Shift+Tab and modified Enter alone", () => {
    const menu = setup();
    type("/co");
    expect(menu.onKey(key("Tab", { shiftKey: true }))).toBe(false);
    for (const m of [{ shiftKey: true }, { altKey: true }, { metaKey: true }, { ctrlKey: true }]) expect(menu.onKey(key("Enter", m))).toBe(false);
    expect(box.value).toBe("/co");
  });

  it("matches a whole name without regard to case on Enter, and sends the name as listed", () => {
    const menu = setup();
    type("/COMPACT keep the plan", 8);
    expect(menu.onKey(key("Enter"))).toBe(false);
    expect(box.value).toBe("/compact keep the plan");
  });

  it("Enter sends a plugin command typed by its own name, as Claude Code takes it", () => {
    const menu = setup([cmd("compact"), cmd("superpowers:brainstorming", "plugin")]);
    type("/brainstorming");
    expect(menu.onKey(key("Enter"))).toBe(false);
    expect(box.value).toBe("/brainstorming");
    type("/Brainstorming");
    expect(menu.onKey(key("Enter"))).toBe(false);
    expect(box.value).toBe("/brainstorming");
  });

  it("a project command in a folder has no short name", () => {
    const menu = setup([cmd("frontend:component", "project"), cmd("compact")]);
    expect(matchCommands([cmd("frontend:component", "project"), cmd("compo")], "comp").map((c) => c.name)).toEqual(["compo", "frontend:component"]);
    type("/frontend:component");
    expect(menu.onKey(key("Enter"))).toBe(false);
    type("/component");
    expect(menu.onKey(key("Tab"))).toBe(true);
    expect(box.value).toBe("/frontend:component ");
  });

  it("a synced skill is taken by its own name too", () => {
    const menu = setup([cmd("anthropic-skills:docx", "user"), cmd("compact")]);
    type("/docx");
    expect(menu.onKey(key("Enter"))).toBe(false);
    expect(box.value).toBe("/docx");
  });

  it("a list that lands after Escape does not reopen it until the next edit", () => {
    const menu = setup();
    box.focus();
    type("/c");
    menu.onKey(key("Escape"));
    menu.setCommands([cmd("compact"), cmd("clear"), cmd("config")]);
    expect(rows()).toHaveLength(0);
    box.dispatchEvent(new KeyboardEvent("keyup", { key: "ArrowLeft", bubbles: true }));
    expect(rows()).toHaveLength(0);
    type("/co");
    expect(rows().map((r) => r.dataset.row)).toEqual(["compact", "config"]);
  });

  it("Enter still fills the start of a plugin command's own name", () => {
    const menu = setup([cmd("compact"), cmd("superpowers:brainstorming", "plugin")]);
    type("/brain");
    expect(menu.onKey(key("Enter"))).toBe(true);
    expect(box.value).toBe("/superpowers:brainstorming ");
  });

  it("Enter sends a word only found inside names, which Tab still fills", () => {
    const menu = setup([cmd("compact"), cmd("code-review")]);
    type("/view");
    expect(menu.onKey(key("Enter"))).toBe(false);
    expect(box.value).toBe("/view");
    expect(menu.onKey(key("Tab"))).toBe(true);
    expect(box.value).toBe("/code-review ");
  });

  it("typing an ordinary message does no list work", () => {
    setup();
    box.focus();
    type("hello");
    morphed.count = 0;
    type("hello there");
    box.dispatchEvent(new KeyboardEvent("keyup", { key: "e" }));
    expect(morphed.count).toBe(0);
    // A list that is open still closes when the slash goes.
    type("/co");
    expect(rows()).toHaveLength(1);
    type("co");
    expect(rows()).toHaveLength(0);
    expect(box.getAttribute("aria-expanded")).toBe("false");
  });

  it("a bare modifier key does not reopen the list after Escape", () => {
    const menu = setup();
    type("/");
    menu.onKey(key("Escape"));
    box.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift" }));
    expect(rows()).toHaveLength(0);
  });
});
