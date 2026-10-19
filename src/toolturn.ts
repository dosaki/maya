import type { Turn } from "./types";

/** Output lines a tool call shows before "… +N lines". */
export const PREVIEW_LINES = 4;

/**
 * What the user opened, by `toolKey`: chat rows (`opened`) and results shown
 * whole (`expanded`). Kept outside the DOM so the once-a-second repaint of a
 * working session leaves them open; a newly opened panel starts with none.
 */
export const toolUi = { opened: new Set<string>(), expanded: new Set<string>() };

export function resetToolUi(): void {
  toolUi.opened.clear();
  toolUi.expanded.clear();
}

/** A tool turn's identity across repaints, where its index moves as turns arrive and old ones drop off. */
export function toolKey(t: Turn): string {
  return [t.text, (t.output ?? "").slice(0, 200), t.added ?? 0, t.removed ?? 0].join("\u0000");
}

/** The call's name and argument: `Bash(cargo test)`, or an older Maya's `Bash: cargo test`. */
export function toolHead(text: string): { name: string; arg: string } {
  const call = /^([\w.:-]+)\(([\s\S]*)\)$/.exec(text);
  if (call) return { name: call[1], arg: call[2] };
  const older = /^([\w.:-]+): ([\s\S]*)$/.exec(text);
  if (older) return { name: older[1], arg: older[2] };
  return { name: text, arg: "" };
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/** "(+a −b)": the lines an edit adds and removes. */
function counts(t: Turn): string {
  return `(+${t.added ?? 0} −${t.removed ?? 0})`;
}

/** The chat look's row: "▸ Bash · cmd", "▸ Update · path (+a −b)". */
export function rowText(t: Turn): string {
  const { name, arg } = toolHead(t.text);
  const shown = t.diff ? "Update" : name;
  const head = arg ? `▸ ${shown} · ${arg}` : `▸ ${shown}`;
  return t.diff ? `${head} ${counts(t)}` : head;
}

/** The terminal look's call: as the agent wrote it, an edit as "Update(path)". */
export function headText(t: Turn): string {
  return t.diff ? `Update(${toolHead(t.text).arg})` : t.text;
}

/** Output under ⎿: the first PREVIEW_LINES lines and "… +N lines", which shows the rest in place. */
function renderOutput(t: Turn, key: string): HTMLElement {
  const result = el("div", "turn__result turn__result--out");
  const lines = (t.output ?? "").split("\n");
  const total = Math.max(t.outputLines ?? 0, lines.length);
  const pre = el("pre", "turn__out");
  result.append(el("span", "turn__elbow", "⎿"), pre);
  const showAll = () => {
    pre.classList.remove("turn__out--preview");
    pre.textContent = lines.join("\n");
    // Lines a cap kept out of the turn cannot be shown, only counted.
    if (total > lines.length) result.append(el("span", "turn__left", `… +${total - lines.length} lines`));
  };
  const hidden = total - PREVIEW_LINES;
  if (hidden <= 0 || toolUi.expanded.has(key)) {
    showAll();
    return result;
  }
  // Clamped too, so a few very long lines cannot fill the view.
  pre.classList.add("turn__out--preview");
  pre.textContent = lines.slice(0, PREVIEW_LINES).join("\n");
  const more = el("button", "turn__more", `… +${hidden} lines`);
  more.type = "button";
  more.dataset.action = "more";
  more.addEventListener("click", (ev) => {
    ev.stopPropagation();
    toolUi.expanded.add(key);
    more.remove();
    showAll();
  });
  result.append(more);
  return result;
}

/** An edit: "⎿ Updated path (+a −b)", then its lines in red and green with their numbers when known. */
function renderDiff(t: Turn): HTMLElement {
  const result = el("div", "turn__result turn__result--diff");
  const summary = el("div", "turn__summary");
  summary.append(el("span", "turn__elbow", "⎿"), el("span", "turn__summary-text", `Updated ${toolHead(t.text).arg} ${counts(t)}`));
  const table = el("div", "turn__diff");
  const diff = t.diff ?? [];
  for (const d of diff) {
    const line = el("div", `diff__line diff__line--${d.op}`);
    line.append(el("span", "diff__no", d.line != null ? String(d.line) : ""), el("span", "diff__sign", d.op === "add" ? "+" : d.op === "del" ? "-" : " "), el("span", "diff__text", d.text));
    table.append(line);
  }
  const left = (t.added ?? 0) + (t.removed ?? 0) - diff.filter((d) => d.op !== "same").length;
  if (left > 0) table.append(el("span", "turn__left", `… +${left} lines`));
  result.append(summary, table);
  return result;
}

/**
 * A tool call for the viewer, drawn for both looks (the stylesheet shows
 * one): the chat look's row, the terminal look's call, and its result.
 * Every string from the transcript goes in as text.
 */
export function renderToolTurn(t: Turn): HTMLElement {
  const key = toolKey(t);
  const body = t.diff ? renderDiff(t) : t.output ? renderOutput(t, key) : null;
  const turn = el("div", "turn turn--tool");
  turn.classList.toggle("turn--failed", !!t.isError);
  const open = !!body && toolUi.opened.has(key);
  turn.classList.toggle("turn--open", open);
  const row = el("button", "turn__row", rowText(t));
  row.type = "button";
  row.dataset.action = "tool";
  row.disabled = !body;
  row.setAttribute("aria-expanded", String(open));
  if (t.isError) row.append(el("span", "turn__fail", " ✕"));
  row.addEventListener("click", () => {
    const now = !toolUi.opened.has(key);
    if (now) toolUi.opened.add(key);
    else toolUi.opened.delete(key);
    turn.classList.toggle("turn--open", now);
    row.setAttribute("aria-expanded", String(now));
  });
  turn.append(el("span", "turn__mark", "⏺"), row, el("div", "turn__text", headText(t)));
  if (body) turn.append(body);
  return turn;
}
