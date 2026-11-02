import type { Card, Pick } from "./types";

/** The event a menu fires on the row that was chosen; it bubbles. */
export const MENU_PICK = "maya:menu-pick";

export interface MenuPick {
  questionIndex: number;
  pick: Pick;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  if (text !== undefined) n.textContent = text;
  return n;
}

/** One numbered row: the number, the label and, when there is one, a description under it. */
function row(num: number, label: string, description?: string): HTMLElement {
  const r = el("div", "menu__row");
  const body = el("div", "menu__body");
  body.append(el("span", "menu__label", label));
  if (description) body.append(el("span", "menu__desc", description));
  r.append(el("span", "menu__num", String(num)), body);
  return r;
}

/**
 * The session view's answer box while a session asks, laid out as the
 * CLI's menu: the header, the question, its numbered options, the own row
 * with a box for the user's words, a divider, and Chat about this. Null
 * unless the card is awaiting with a question at `next`.
 *
 * Choosing a row fires `MENU_PICK` from it. Number keys, ↑/↓ and Enter
 * act only once the menu has focus (clicked or tabbed into), never from
 * elsewhere in the view; digits typed in the own box are text. On a
 * multi-select question choosing an option ticks it, and Send, where the
 * CLI has its Submit row, fires the ticked ones together.
 * `data-key` changes only when the ask, the question or `enabled` does,
 * so a repaint can keep the menu, its focus and a half-typed answer.
 */
export function renderMenu(card: Card, next: number, opts: { enabled: boolean; keyboardHints: boolean }): HTMLElement | null {
  const aw = card.awaiting;
  if (card.state !== "awaiting" || !aw) return null;
  const q = aw.questions[next];
  if (!q) return null;

  const menu = el("div", "menu");
  menu.tabIndex = 0;
  menu.dataset.key = `${card.stateSince}:${next}:${opts.enabled ? 1 : 0}`;
  const header = aw.questions.length > 1 ? `Question ${next + 1} of ${aw.questions.length} · ${q.header}` : q.header;
  menu.append(el("div", "menu__header", header), el("div", "menu__question", q.question));

  const rows = el("div", "menu__rows");
  rows.setAttribute("role", "listbox");
  if (q.multiSelect) rows.setAttribute("aria-multiselectable", "true");
  if (!opts.enabled) rows.setAttribute("aria-disabled", "true");
  q.options.forEach((o, i) => {
    const r = row(i + 1, o.label, o.description);
    r.dataset.action = "pick";
    r.dataset.row = String(i);
    r.dataset.pick = q.multiSelect ? "tick" : "option";
    r.dataset.index = String(i);
    if (q.multiSelect) r.setAttribute("aria-selected", "false");
    rows.append(r);
  });
  let box: HTMLTextAreaElement | null = null;
  if (q.own && !q.multiSelect) {
    const r = row(q.own.row + 1, q.own.label);
    r.dataset.action = "pick";
    r.dataset.row = String(q.own.row);
    r.dataset.pick = "own";
    box = el("textarea", "menu__own");
    box.rows = 2;
    box.placeholder = "Your own answer, sent as the answer to this question";
    box.disabled = !opts.enabled;
    const send = el("button", "card__btn menu__send", "Send");
    send.type = "button";
    send.dataset.action = "own";
    send.disabled = !opts.enabled;
    const line = el("div", "menu__own-line");
    line.append(box, send);
    r.querySelector(".menu__body")!.append(line);
    rows.append(r);
  }
  let submit: HTMLButtonElement | null = null;
  if (q.multiSelect) {
    const line = el("div", "menu__submit", "Tick one or more, then send them.");
    submit = el("button", "card__btn menu__send", "Send");
    submit.type = "button";
    submit.dataset.action = "options";
    submit.disabled = true;
    line.append(submit);
    rows.append(line);
  }
  if (q.chatRow !== undefined) {
    rows.append(el("div", "menu__divider"));
    // The CLI does not number a multi-select's Submit row, so there the
    // number is the row's index, as it shows it.
    const r = row(q.multiSelect ? q.chatRow : q.chatRow + 1, "Chat about this", "Set the question aside and talk it over");
    r.dataset.action = "pick";
    r.dataset.row = String(q.chatRow);
    r.dataset.pick = "chat";
    rows.append(r);
  }
  menu.append(rows);

  if (aw.questions.length > 1) {
    // Maya cannot see answers given in the terminal mid-ask; its key
    // sequences assume the picker is on the question shown here.
    menu.append(el("div", "menu__warn", "Answer every question here or every question in the terminal, not both."));
  }
  const foot = el("div", "menu__foot");
  const choosable = [...rows.querySelectorAll<HTMLElement>("[data-action=pick]")];
  if (opts.keyboardHints && choosable.length > 0) {
    const last = choosable[choosable.length - 1].querySelector(".menu__num")?.textContent;
    foot.append(el("span", "menu__hints", `↑/↓ or 1–${last} · Enter to ${q.multiSelect ? "tick" : "choose"}`));
  }
  menu.append(foot);

  const emit = (from: HTMLElement, pick: Pick) =>
    from.dispatchEvent(new CustomEvent<MenuPick>(MENU_PICK, { detail: { questionIndex: next, pick }, bubbles: true }));
  const ownText = () => box?.value.trim() ?? "";
  const select = (r: HTMLElement) => {
    for (const c of choosable) c.classList.toggle("menu__row--sel", c === r);
  };
  const ticked = () => choosable.filter((c) => c.classList.contains("menu__row--on")).map((c) => Number(c.dataset.index));
  const choose = (r: HTMLElement) => {
    if (rows.getAttribute("aria-disabled") === "true") return;
    select(r);
    if (r.dataset.pick === "tick") {
      const on = r.classList.toggle("menu__row--on");
      r.setAttribute("aria-selected", String(on));
      if (submit) submit.disabled = !opts.enabled || ticked().length === 0;
    } else if (r.dataset.pick === "option") emit(r, { kind: "option", index: Number(r.dataset.index) });
    else if (r.dataset.pick === "chat") emit(r, { kind: "chat" });
    else box?.focus();
  };
  const sendOwn = (from: HTMLElement) => {
    if (rows.getAttribute("aria-disabled") === "true" || !ownText()) return;
    emit(from, { kind: "own", text: ownText() });
  };

  rows.addEventListener("click", (ev) => {
    const target = ev.target as HTMLElement;
    if (target.closest("textarea")) return;
    const send = target.closest<HTMLButtonElement>("button[data-action=own]");
    if (send) {
      sendOwn(send);
      return;
    }
    const sendTicked = target.closest<HTMLButtonElement>("button[data-action=options]");
    if (sendTicked) {
      if (rows.getAttribute("aria-disabled") !== "true" && ticked().length > 0) emit(sendTicked, { kind: "options", indexes: ticked() });
      return;
    }
    const r = target.closest<HTMLElement>("[data-action=pick]");
    if (r) choose(r);
  });
  menu.addEventListener("keydown", (ev) => {
    if (box && ev.target === box) {
      if (ev.key === "Enter" && !ev.shiftKey && !ev.isComposing) {
        ev.preventDefault();
        sendOwn(box);
      }
      return;
    }
    // Keys on a button in the menu (Send, Stop) are that button's own.
    if (ev.target !== menu || choosable.length === 0) return;
    const at = choosable.findIndex((c) => c.classList.contains("menu__row--sel"));
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
      ev.preventDefault();
      const step = ev.key === "ArrowDown" ? 1 : -1;
      const to = at < 0 ? (step > 0 ? 0 : choosable.length - 1) : Math.min(choosable.length - 1, Math.max(0, at + step));
      select(choosable[to]);
    } else if (ev.key === "Enter" && at >= 0) {
      ev.preventDefault();
      choose(choosable[at]);
    } else if (/^[1-9]$/.test(ev.key)) {
      const r = choosable.find((c) => c.querySelector(".menu__num")?.textContent === ev.key);
      if (!r) return;
      ev.preventDefault();
      choose(r);
    }
  });
  return menu;
}
