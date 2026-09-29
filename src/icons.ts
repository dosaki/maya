/** Inline SVG icons for card buttons. Each is 14px, drawn with currentColor. */
const ICONS = {
  terminal:
    '<svg class="icon icon-terminal" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M4.5 6 L7 8 L4.5 10" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 10.5 H11.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>',
  reply:
    '<svg class="icon icon-reply" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M8 2.5 C4.4 2.5 1.8 4.7 1.8 7.4 c0 1.5 0.8 2.8 2.1 3.7 L3.2 13.8 L6.4 12.1 c0.5 0.1 1 0.2 1.6 0.2 c3.6 0 6.2-2.2 6.2-4.9 S11.6 2.5 8 2.5 Z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/></svg>',
} as const;

export type IconName = keyof typeof ICONS;

/** A button showing only an icon, with the label as tooltip and accessible name. */
export function iconButton(icon: IconName, label: string, className = "card__btn"): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = `${className} card__btn--icon`;
  b.title = label;
  b.setAttribute("aria-label", label);
  b.innerHTML = ICONS[icon];
  return b;
}
