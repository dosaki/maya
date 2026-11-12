// The building blocks every Settings screen is made of: a screen's head, a
// group of cards, rows with a label on the left and a control on the right,
// switches, segmented choices, buttons and the collapsible Work Items source.
// They return plain elements with stable structure and ids derived from the
// control's name, so a repaint with `morph` keeps hover and focus.

type Content = string | Node;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, content?: Content): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (content !== undefined) e.append(content);
  return e;
}

export function screenHead(title: string, lede: string): HTMLElement {
  const head = el("div", "settings__head");
  head.append(el("h2", "settings__title", title), el("p", "settings__lede", lede));
  return head;
}

export function group(opts: { title?: string; note?: Content; foot?: Content }, ...children: Node[]): HTMLElement {
  const g = el("div", "settings__group");
  if (opts.title) g.append(el("h3", "settings__group-title", opts.title));
  if (opts.note) g.append(el("p", "settings__group-note", opts.note));
  g.append(...children);
  if (opts.foot) g.append(el("p", "settings__group-foot", opts.foot));
  return g;
}

export function card(...rows: Node[]): HTMLElement {
  const c = el("div", "settings__card");
  c.append(...rows);
  return c;
}

const LABELABLE = new Set(["INPUT", "SELECT", "TEXTAREA"]);

export function row(opts: {
  label?: Content;
  help?: Content | null;
  control?: Node | Node[];
  tag?: "label" | "div";
  stack?: boolean;
  /** Keeps the help's id unique when two rows hold a control of the same name (one per forge, say). */
  idScope?: string;
}): HTMLElement {
  const controls = opts.control === undefined ? [] : Array.isArray(opts.control) ? opts.control : [opts.control];
  const single = controls.length === 1 ? controls[0]! : null;
  const tag = opts.tag ?? (single instanceof HTMLElement && LABELABLE.has(single.tagName) ? "label" : "div");
  const r = el(tag, opts.stack ? "settings__row settings__row--stack" : "settings__row");
  const text = el("span", "settings__row-text");
  const labelEl = opts.label !== undefined ? el("span", "settings__label", opts.label) : null;
  if (labelEl) text.append(labelEl);
  if (opts.help) {
    const help = el("span", "settings__help", opts.help);
    const name = single instanceof HTMLElement ? (single as HTMLInputElement).name : "";
    if (name) {
      help.id = opts.idScope ? `settings-help-${opts.idScope}-${name}` : `settings-help-${name}`;
      (single as HTMLElement).setAttribute("aria-describedby", help.id);
    }
    text.append(help);
  }
  r.append(text);
  if (controls.length > 0) {
    const ctl = el("div", "settings__control");
    ctl.append(...controls);
    r.append(ctl);
  }
  if (tag === "div" && labelEl && typeof opts.label === "string") nameControls(r, labelEl, opts.idScope);
  return r;
}

/**
 * A `div` row has no wrapping `<label>`, so its text must name its controls
 * itself: the label gets a stable id (from the first named control, scoped
 * like the help's) and each control with no name of its own points at it.
 */
function nameControls(r: HTMLElement, labelEl: HTMLElement, idScope?: string): void {
  const own = [...r.querySelectorAll<HTMLElement>(":scope > .settings__control > *")];
  const targets = own.filter((c) => (LABELABLE.has(c.tagName) || c.getAttribute("role") === "radiogroup") && !c.hasAttribute("aria-label") && !c.hasAttribute("aria-labelledby"));
  const first = targets[0];
  if (!first) return;
  const named = first.matches("[name]") ? first : first.querySelector("[name]");
  const name = (named as HTMLInputElement | null)?.name;
  if (!name) return;
  labelEl.id = idScope ? `settings-label-${idScope}-${name}` : `settings-label-${name}`;
  for (const c of targets) c.setAttribute("aria-labelledby", labelEl.id);
}

export function cardHead(opts: { label: Content; help?: string; control?: Node; order?: number }): HTMLElement {
  const r = row({ label: opts.label, help: opts.help, control: opts.control, tag: "div" });
  r.classList.add("settings__row--head");
  if (opts.order !== undefined) r.prepend(el("span", "settings__order", String(opts.order)));
  return r;
}

export function footRow(left: Content, right?: HTMLElement): HTMLElement {
  const r = el("div", "settings__row settings__row--foot");
  r.append(left);
  if (right) {
    right.classList.add("settings__foot-end");
    r.append(right);
  }
  return r;
}

export function toggle(name: string | null, checked: boolean, onChange: (on: boolean) => void, ariaLabel?: string): HTMLInputElement {
  const t = el("input", "settings__toggle");
  t.type = "checkbox";
  if (name) t.name = name;
  t.checked = checked;
  if (ariaLabel) t.setAttribute("aria-label", ariaLabel);
  t.addEventListener("change", () => onChange(t.checked));
  return t;
}

export function choice<T extends string>(
  name: string,
  options: readonly (readonly [T, string])[],
  value: T,
  onChange: (v: T) => void,
  disabled = false,
): HTMLElement {
  const seg = el("div", "settings__seg");
  seg.setAttribute("role", "radiogroup");
  for (const [v, text] of options) {
    const label = el("label");
    const input = el("input");
    input.type = "radio";
    input.name = name;
    input.value = v;
    input.checked = v === value;
    input.disabled = disabled;
    input.addEventListener("change", () => { if (input.checked) onChange(v); });
    label.append(input, el("span", undefined, text));
    seg.append(label);
  }
  return seg;
}

export function state(text: string, tone: "on" | "main" | "off"): HTMLElement {
  return el("span", `settings__state settings__state--${tone}`, text);
}

export function btn(
  text: string,
  opts: { action?: string; kind?: "plain" | "quiet" | "danger" | "gold"; disabled?: boolean },
  onClick?: () => void,
): HTMLButtonElement {
  const b = el("button", opts.kind && opts.kind !== "plain" ? `settings__btn settings__btn--${opts.kind}` : "settings__btn", text);
  b.type = "button";
  if (opts.action) b.dataset.action = opts.action;
  if (opts.disabled) b.disabled = true;
  if (onClick) b.addEventListener("click", onClick);
  return b;
}

export function addButton(text: string, action: string, onClick: () => void): HTMLButtonElement {
  const b = el("button", "settings__add", text);
  b.type = "button";
  b.dataset.action = action;
  b.addEventListener("click", onClick);
  return b;
}

export function notice(text: string, tone: "error" | "info"): HTMLElement {
  return el("div", tone === "error" ? "settings__notice settings__error" : "settings__notice", text);
}

export function source(opts: {
  key: string;
  name: string;
  tag?: string;
  summary: string;
  monoSummary?: boolean;
  stateText: string;
  stateOn: boolean;
  open: boolean;
  onToggle: (open: boolean) => void;
  control: HTMLInputElement;
  body: Node[];
}): HTMLElement {
  const root = el("div", "settings__card settings__source");
  root.dataset.source = opts.key;
  if (opts.open) root.setAttribute("data-open", "");
  if (!opts.stateOn) root.classList.add("settings__source--off");

  const toggleBtn = el("button", "settings__source-toggle");
  toggleBtn.type = "button";
  toggleBtn.setAttribute("aria-expanded", String(opts.open));
  toggleBtn.append(el("span", "settings__source-chev"), el("span", "settings__source-name", opts.name));
  if (opts.tag) toggleBtn.append(el("span", "settings__source-tag", opts.tag));
  toggleBtn.append(el("span", opts.monoSummary ? "settings__source-sum mono" : "settings__source-sum", opts.summary));
  toggleBtn.addEventListener("click", () => {
    const open = !root.hasAttribute("data-open");
    root.toggleAttribute("data-open", open);
    toggleBtn.setAttribute("aria-expanded", String(open));
    opts.onToggle(open);
  });

  const head = el("div", "settings__source-head");
  head.append(toggleBtn, el("span", "settings__source-state", opts.stateText), opts.control);
  const body = el("div", "settings__source-body");
  body.append(...opts.body);
  root.append(head, body);
  return root;
}
