// The slash command list above the message box: typing "/" as the first
// character of the box offers the assistant's commands to filter and fill.

import { morph } from "./morph";
import type { CommandInfo } from "./types";

/** The word typed after "/" when the caret is in the box's first word and it starts with "/"; else null. */
export function slashQuery(value: string, caret: number): string | null {
  if (!value.startsWith("/")) return null;
  const end = value.search(/\s/);
  const wordEnd = end < 0 ? value.length : end;
  if (caret < 1 || caret > wordEnd) return null;
  return value.slice(1, wordEnd);
}

/** Names starting with `typed` first, then names containing it, each in list order; all for "". */
export function matchCommands(list: CommandInfo[], typed: string): CommandInfo[] {
  const t = typed.toLowerCase();
  if (!t) return [...list];
  const starts = list.filter((c) => c.name.toLowerCase().startsWith(t));
  const inside = list.filter((c) => !c.name.toLowerCase().startsWith(t) && c.name.toLowerCase().includes(t));
  return [...starts, ...inside];
}

export interface SlashMenu {
  element: HTMLElement;
  setCommands(list: CommandInfo[]): void;
  /** True when the key was handled and the composer must not act on it. */
  onKey(ev: KeyboardEvent): boolean;
  close(): void;
}

let counter = 0;

export function attachSlashMenu(box: HTMLTextAreaElement): SlashMenu {
  const id = `slash-${++counter}`;
  const element = document.createElement("div");
  element.className = "slash";
  element.id = id;
  element.setAttribute("role", "listbox");
  element.hidden = true;
  box.setAttribute("aria-controls", id);
  box.setAttribute("aria-expanded", "false");

  let commands: CommandInfo[] = [];
  let shown: CommandInfo[] = [];
  let typed = "";
  let active = 0;
  let open = false;

  const rowId = (c: CommandInfo) => `${id}-${c.name}`;

  function render(): void {
    const fresh = document.createElement("div");
    shown.forEach((c, i) => {
      const row = document.createElement("div");
      row.className = "slash__row";
      row.setAttribute("role", "option");
      row.id = rowId(c);
      row.setAttribute("data-row", c.name);
      row.setAttribute("aria-selected", String(i === active));
      const name = document.createElement("span");
      name.className = "slash__name";
      name.textContent = `/${c.name}`;
      const desc = document.createElement("span");
      desc.className = "slash__desc";
      desc.textContent = c.description + (c.source === "plugin" || c.source === "project" ? ` · ${c.source}` : "");
      row.append(name, desc);
      fresh.append(row);
    });
    fresh.className = "slash";
    fresh.id = id;
    fresh.setAttribute("role", "listbox");
    fresh.hidden = !open;
    morph(element, fresh);
    box.setAttribute("aria-expanded", String(open));
    if (open && shown[active]) {
      box.setAttribute("aria-activedescendant", rowId(shown[active]));
      (element.children[active] as HTMLElement | undefined)?.scrollIntoView?.({ block: "nearest" });
    } else box.removeAttribute("aria-activedescendant");
  }

  function close(): void {
    open = false;
    shown = [];
    active = 0;
    render();
  }

  function refresh(): void {
    const q = slashQuery(box.value, box.selectionStart ?? 0);
    const found = q === null ? [] : matchCommands(commands, q);
    if (q === null || !found.length) return close();
    const keep = open && shown[active] ? found.findIndex((c) => c.name === shown[active].name) : -1;
    typed = q;
    shown = found;
    active = keep >= 0 ? keep : 0;
    open = true;
    render();
  }

  function fill(c: CommandInfo): void {
    const end = box.value.search(/\s/);
    const rest = end < 0 ? "" : box.value.slice(end).replace(/^ /, "");
    const head = `/${c.name} `;
    box.value = head + rest;
    box.setSelectionRange(head.length, head.length);
    close();
    box.dispatchEvent(new Event("input", { bubbles: true }));
  }

  box.addEventListener("input", refresh);
  box.addEventListener("click", refresh);
  box.addEventListener("keyup", (ev) => {
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp" || ev.key === "Enter" || ev.key === "Tab" || ev.key === "Escape") return;
    refresh();
  });

  const rowOf = (ev: Event): CommandInfo | undefined => {
    const row = (ev.target as Element | null)?.closest?.("[data-row]");
    return shown.find((c) => c.name === row?.getAttribute("data-row"));
  };
  element.addEventListener("mousedown", (ev) => ev.preventDefault());
  element.addEventListener("click", (ev) => {
    const c = rowOf(ev);
    if (c) fill(c);
  });

  function onKey(ev: KeyboardEvent): boolean {
    if (ev.isComposing || !open || !shown.length) return false;
    switch (ev.key) {
      case "ArrowDown":
      case "ArrowUp": {
        ev.preventDefault();
        active = (active + (ev.key === "ArrowDown" ? 1 : shown.length - 1)) % shown.length;
        render();
        return true;
      }
      case "Tab":
        ev.preventDefault();
        fill(shown[active]);
        return true;
      case "Enter":
        if (commands.some((c) => c.name === typed)) return false;
        ev.preventDefault();
        fill(shown[active]);
        return true;
      case "Escape":
        ev.preventDefault();
        ev.stopPropagation();
        close();
        return true;
      default:
        return false;
    }
  }

  return {
    element,
    setCommands(list) {
      commands = list;
      if (open) refresh();
    },
    onKey,
    close,
  };
}
