/**
 * The question before a quit or an update's restart stops a working Native
 * session. It has its own host above everything else, so a session viewer
 * open in `#modal-host` stays put and its repaints cannot wipe the question.
 */

/** The question's title, its sentence and its yes button, for these sessions. */
export function quitText(names: string[], update: boolean): { title: string; text: string; yes: string } {
  const who = names.length === 1 ? "This Native session is working" : "These Native sessions are working";
  const when = update ? "when Maya restarts to update" : "if Maya quits";
  return {
    title: update ? "Restart Maya to update?" : "Quit Maya?",
    text: `${who} and will stop ${when}: ${names.join(", ")}.`,
    yes: update ? "Restart anyway" : "Quit anyway",
  };
}

/** Answers the question showing now, if any, with Cancel. */
let pending: ((yes: boolean) => void) | null = null;

function host(): HTMLElement {
  let h = document.getElementById("quit-host");
  if (!h) {
    h = document.createElement("div");
    h.id = "quit-host";
    document.body.append(h);
  }
  return h;
}

function button(label: string, primary: boolean, onClick: () => void): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = primary ? "card__btn card__btn--primary" : "card__btn";
  b.textContent = label;
  b.addEventListener("click", onClick);
  return b;
}

/**
 * Asks whether to go on: true on the yes button, false on Cancel or Escape.
 * A second question replaces the first, which counts as Cancel. Tab and
 * Shift+Tab stay on its two buttons.
 */
export function askQuit(names: string[], update: boolean): Promise<boolean> {
  pending?.(false);
  const h = host();
  return new Promise<boolean>((resolve) => {
    const answer = (yes: boolean) => {
      if (pending !== answer) return;
      pending = null;
      document.removeEventListener("keydown", onKey, true);
      h.replaceChildren();
      resolve(yes);
    };
    pending = answer;
    const { title, text, yes } = quitText(names, update);
    const root = document.createElement("div");
    root.className = "modal modal--quit";
    const backdrop = document.createElement("div");
    backdrop.className = "modal__backdrop";
    const panel = document.createElement("section");
    panel.className = "modal__panel modal__panel--compact quit";
    panel.setAttribute("role", "alertdialog");
    panel.setAttribute("aria-modal", "true");
    const head = document.createElement("header");
    head.className = "modal__head";
    const titles = document.createElement("div");
    titles.className = "modal__titles";
    const heading = document.createElement("h2");
    heading.className = "modal__title";
    heading.id = "quit-title";
    heading.textContent = title;
    titles.append(heading);
    head.append(titles);
    panel.setAttribute("aria-labelledby", heading.id);
    const body = document.createElement("div");
    body.className = "modal__setup";
    const p = document.createElement("p");
    p.id = "quit-text";
    p.textContent = text;
    panel.setAttribute("aria-describedby", p.id);
    const actions = document.createElement("div");
    actions.className = "quit__actions";
    const cancel = button("Cancel", false, () => answer(false));
    const go = button(yes, true, () => answer(true));
    actions.append(cancel, go);
    body.append(p, actions);
    panel.append(head, body);
    // The viewer behind closes on Escape; while asking, Escape only cancels.
    // Tab moves between the two buttons and never to the page behind.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Tab") {
        e.preventDefault();
        e.stopPropagation();
        const order = e.shiftKey ? [go, cancel] : [cancel, go];
        const at = order.indexOf(document.activeElement as HTMLButtonElement);
        order[at === 0 ? 1 : 0].focus();
        return;
      }
      if (e.key !== "Escape") return;
      e.stopPropagation();
      answer(false);
    };
    document.addEventListener("keydown", onKey, true);
    root.append(backdrop, panel);
    h.replaceChildren(root);
    cancel.focus();
  });
}
