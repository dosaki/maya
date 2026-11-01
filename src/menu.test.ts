import { describe, expect, it, vi } from "vitest";
import { MENU_PICK, renderMenu, type MenuPick } from "./menu";
import type { Card, Question } from "./types";

const db: Question = {
  question: "Which database should the cache use?",
  header: "Database",
  multiSelect: false,
  options: [{ label: "SQLite", description: "One file, no server" }, { label: "Postgres", description: "Shared with the API" }],
  own: { label: "Type something", row: 2 },
  chatRow: 3,
};
const asking: Card = { sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince: 500, snippet: "", hasInbox: true, harness: "claude-code", pr: null, context: null,
  awaiting: { kind: "question", detail: db.question, questions: [db] } };
const on = { enabled: true, keyboardHints: true };

/** The menu in the document, with every pick it emits. */
function mount(card: Card = asking, opts = on) {
  const menu = renderMenu(card, 0, opts)!;
  document.body.replaceChildren(menu);
  const picks: MenuPick[] = [];
  menu.addEventListener(MENU_PICK, (ev) => picks.push((ev as CustomEvent<MenuPick>).detail));
  return { menu, picks };
}

const key = (target: EventTarget, k: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

describe("renderMenu", () => {
  it("menu shows options, Type something and Chat about this in CLI order", () => {
    const { menu } = mount();
    expect(menu.querySelector(".menu__header")?.textContent).toBe("Database");
    expect(menu.querySelector(".menu__question")?.textContent).toBe("Which database should the cache use?");
    const rows = [...menu.querySelectorAll<HTMLElement>("[data-action=pick]")];
    expect(rows.map((r) => r.dataset.row)).toEqual(["0", "1", "2", "3"]);
    expect(rows.map((r) => r.querySelector(".menu__num")?.textContent)).toEqual(["1", "2", "3", "4"]);
    expect(rows.map((r) => r.querySelector(".menu__label")?.textContent)).toEqual(["SQLite", "Postgres", "Type something", "Chat about this"]);
    expect(rows[0].querySelector(".menu__desc")?.textContent).toBe("One file, no server");
    expect(rows[2].querySelector("textarea")).not.toBeNull();
    // The divider sits between the own row and Chat about this, as in the CLI.
    expect(rows[2].nextElementSibling?.className).toBe("menu__divider");
    expect(rows[3].previousElementSibling?.className).toBe("menu__divider");
    expect(menu.querySelector(".menu__hints")?.textContent).toBe("↑/↓ or 1–4 · Enter to choose");
  });

  it("leaves out the own and chat rows the question does not have, and the hints on the phone", () => {
    const plain = { ...db, own: undefined, chatRow: undefined };
    const { menu } = mount({ ...asking, awaiting: { kind: "question", detail: "Q", questions: [plain] } }, { enabled: true, keyboardHints: false });
    expect([...menu.querySelectorAll<HTMLElement>("[data-action=pick]")].map((r) => r.dataset.row)).toEqual(["0", "1"]);
    expect(menu.querySelector("textarea")).toBeNull();
    expect(menu.querySelector(".menu__divider")).toBeNull();
    expect(menu.querySelector(".menu__hints")).toBeNull();
    expect(menu.querySelector(".menu__foot")).not.toBeNull();
  });

  it("renders a menu for any ask with a question, and nothing without one", () => {
    expect(renderMenu({ ...asking, awaiting: { kind: "permission", detail: "Bash", questions: [db] } }, 0, on)).not.toBeNull();
    expect(renderMenu({ ...asking, awaiting: { kind: "permission", detail: "Bash", questions: [] } }, 0, on)).toBeNull();
    expect(renderMenu(asking, 1, on)).toBeNull();
    expect(renderMenu({ ...asking, state: "working" }, 0, on)).toBeNull();
  });

  it("labels the question it is on and warns against mixing when there are several", () => {
    const two = { ...asking, awaiting: { kind: "question" as const, detail: "Q", questions: [db, { ...db, header: "Port" }] } };
    const menu = renderMenu(two, 1, on)!;
    expect(menu.querySelector(".menu__header")?.textContent).toBe("Question 2 of 2 · Port");
    expect(menu.querySelector(".menu__warn")?.textContent).toContain("not both");
    expect(renderMenu(asking, 0, on)!.querySelector(".menu__warn")).toBeNull();
  });

  it("ticks a multi-select question's rows, then sends them together", () => {
    const multi = { ...db, multiSelect: true, own: undefined, chatRow: 4 };
    const { menu, picks } = mount({ ...asking, awaiting: { kind: "question", detail: "Q", questions: [multi] } });
    const rows = [...menu.querySelectorAll<HTMLElement>("[data-action=pick]")];
    // The CLI's Submit row has no number, so Chat about this keeps the one it shows there.
    expect(rows.map((r) => r.querySelector(".menu__num")?.textContent)).toEqual(["1", "2", "4"]);
    expect(rows.map((r) => r.querySelector(".menu__label")?.textContent)).toEqual(["SQLite", "Postgres", "Chat about this"]);
    expect(menu.querySelector("textarea")).toBeNull();
    expect(menu.querySelector(".menu__hints")?.textContent).toBe("↑/↓ or 1–4 · Enter to tick");
    const send = menu.querySelector<HTMLButtonElement>("button[data-action=options]")!;
    expect(send.disabled).toBe(true);

    rows[0].click();
    menu.focus();
    key(menu, "2");
    expect(rows.map((r) => r.getAttribute("aria-selected"))).toEqual(["true", "true", null]);
    rows[0].click();
    expect(rows[0].classList.contains("menu__row--on")).toBe(false);
    expect(picks).toEqual([]);
    expect(send.disabled).toBe(false);

    send.click();
    expect(picks).toEqual([{ questionIndex: 0, pick: { kind: "options", indexes: [1] } }]);
    rows[2].click();
    expect(picks[1]).toEqual({ questionIndex: 0, pick: { kind: "chat" } });
  });

  it("Enter ticks the selected row of a multi-select question", () => {
    const multi = { ...db, multiSelect: true, own: undefined, chatRow: 4 };
    const { menu, picks } = mount({ ...asking, awaiting: { kind: "question", detail: "Q", questions: [multi] } });
    menu.focus();
    key(menu, "ArrowDown");
    key(menu, "ArrowDown");
    key(menu, "Enter");
    key(menu, "Enter");
    key(menu, "Enter");
    expect(menu.querySelector("[data-row='1']")?.getAttribute("aria-selected")).toBe("true");
    menu.querySelector<HTMLButtonElement>("button[data-action=options]")!.click();
    expect(picks).toEqual([{ questionIndex: 0, pick: { kind: "options", indexes: [1] } }]);
  });

  it("picks an option on click, and focuses the own box on its row", () => {
    const { menu, picks } = mount();
    menu.querySelector<HTMLElement>("[data-row='1']")!.click();
    expect(picks).toEqual([{ questionIndex: 0, pick: { kind: "option", index: 1 } }]);
    menu.querySelector<HTMLElement>("[data-row='2']")!.click();
    expect(document.activeElement).toBe(menu.querySelector("textarea"));
    menu.querySelector<HTMLElement>("[data-row='3']")!.click();
    expect(picks[picks.length - 1]).toEqual({ questionIndex: 0, pick: { kind: "chat" } });
    expect(picks).toHaveLength(2);
  });

  it("number keys and arrows act only once the menu has focus", () => {
    const { menu, picks } = mount();
    key(document, "1");
    key(document.body, "ArrowDown");
    key(document.body, "Enter");
    expect(picks).toEqual([]);
    menu.focus();
    key(menu, "1");
    expect(picks).toEqual([{ questionIndex: 0, pick: { kind: "option", index: 0 } }]);
    // "1" left the first row selected; down twice and up once lands on the second.
    key(menu, "ArrowDown");
    key(menu, "ArrowDown");
    key(menu, "ArrowUp");
    expect(menu.querySelector(".menu__row--sel")?.getAttribute("data-row")).toBe("1");
    key(menu, "Enter");
    expect(picks[picks.length - 1]).toEqual({ questionIndex: 0, pick: { kind: "option", index: 1 } });
    key(menu, "4");
    expect(picks[picks.length - 1]).toEqual({ questionIndex: 0, pick: { kind: "chat" } });
    key(menu, "3");
    expect(document.activeElement).toBe(menu.querySelector("textarea"));
  });

  it("leaves Enter and digits alone on a button inside the menu, such as Stop", () => {
    const { menu, picks } = mount();
    const stop = document.createElement("button");
    menu.querySelector(".menu__foot")!.append(stop);
    menu.focus();
    key(menu, "ArrowDown");
    stop.focus();
    key(stop, "Enter");
    key(stop, "1");
    expect(picks).toEqual([]);
  });

  it("Enter in the own box sends {kind:\"own\"}; empty box sends nothing", () => {
    const { menu, picks } = mount();
    const box = menu.querySelector<HTMLTextAreaElement>("textarea")!;
    box.focus();
    key(box, "Enter");
    box.value = "   ";
    key(box, "Enter");
    expect(picks).toEqual([]);
    // Digits typed in the box are text, not row numbers.
    box.value = "Use 2 shards";
    key(box, "2");
    expect(picks).toEqual([]);
    key(box, "Enter");
    expect(picks).toEqual([{ questionIndex: 0, pick: { kind: "own", text: "Use 2 shards" } }]);
    box.value = "Redis";
    menu.querySelector<HTMLButtonElement>("button[data-action=own]")!.click();
    expect(picks[picks.length - 1]).toEqual({ questionIndex: 0, pick: { kind: "own", text: "Redis" } });
  });

  it("the menu is disabled inside the open delay", () => {
    const { menu, picks } = mount(asking, { enabled: false, keyboardHints: true });
    expect(menu.querySelector(".menu__rows")?.getAttribute("aria-disabled")).toBe("true");
    expect(menu.querySelector<HTMLTextAreaElement>("textarea")!.disabled).toBe(true);
    expect(menu.querySelector<HTMLButtonElement>("button[data-action=own]")!.disabled).toBe(true);
    menu.querySelector<HTMLElement>("[data-row='0']")!.click();
    menu.focus();
    key(menu, "1");
    key(menu, "4");
    expect(picks).toEqual([]);
  });

  it("keys the menu by ask, question and whether it is enabled, so a repaint can keep it", () => {
    expect(renderMenu(asking, 0, on)!.dataset.key).toBe("500:0:1");
    expect(renderMenu(asking, 0, { ...on, enabled: false })!.dataset.key).toBe("500:0:0");
  });

  it("emits picks that bubble to whoever holds the menu", () => {
    const { menu } = mount();
    const outer = vi.fn();
    document.body.addEventListener(MENU_PICK, outer);
    menu.querySelector<HTMLElement>("[data-row='0']")!.click();
    document.body.removeEventListener(MENU_PICK, outer);
    expect(outer).toHaveBeenCalledTimes(1);
  });
});
