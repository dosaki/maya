import { projectName } from "./format";
import { harnessLabel } from "./harness";
import { iconElement } from "./icons";
import { thisComputer } from "./platform";
import type { Card } from "./types";

/**
 * What a search looks through on one card: the repository, the session
 * name, the agent (its label and its id) and the machine, with a local
 * session's machine read as "This Mac".
 */
function fields(card: Card): string[] {
  return [projectName(card.cwd), card.name, harnessLabel(card.harness), card.harness, card.machine || thisComputer()].map((f) => f.toLowerCase());
}

/**
 * True when every word of `query` appears, ignoring case, in one of the
 * card's fields, so "maya codex" finds the Codex sessions in maya. A blank
 * query matches every card.
 */
export function matchesSearch(card: Card, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const haystack = fields(card);
  return words.every((w) => haystack.some((f) => f.includes(w)));
}

export function filterCards(cards: Card[], query: string): Card[] {
  return query.trim() ? cards.filter((c) => matchesSearch(c, query)) : cards;
}

/**
 * The search box over the Sessions board, built once so typing survives the
 * board's repaints. `onChange` hears each edit; Escape clears the box.
 */
export function makeSessionSearch(host: HTMLElement, onChange: (query: string) => void) {
  const field = document.createElement("label");
  field.className = "session-search__field";
  const input = document.createElement("input");
  input.type = "search";
  input.className = "session-search__input";
  input.placeholder = "Search repository, session, agent or machine";
  input.setAttribute("aria-label", "Search sessions");
  input.spellcheck = false;
  input.autocomplete = "off";
  field.append(iconElement("search", 13), input);
  const count = document.createElement("span");
  count.className = "session-search__count";
  host.replaceChildren(field, count);

  input.addEventListener("input", () => onChange(input.value));
  input.addEventListener("keydown", (ev) => {
    if (ev.key !== "Escape" || !input.value) return;
    ev.preventDefault();
    input.value = "";
    onChange("");
  });

  return {
    query: () => input.value,
    focus: () => {
      input.focus();
      input.select();
    },
    /** "3 of 12" while a search narrows the board; nothing otherwise. */
    setCount: (shown: number, total: number) => {
      count.textContent = input.value.trim() ? `${shown} of ${total}` : "";
    },
  };
}
