import close from "./assets/icons/close.svg?raw";
import compact from "./assets/icons/compact.svg?raw";
import remote from "./assets/icons/remote.svg?raw";
import reply from "./assets/icons/reply.svg?raw";
import resume from "./assets/icons/resume.svg?raw";
import terminal from "./assets/icons/terminal.svg?raw";

/**
 * Card icons, kept as plain SVG files in src/assets/icons so they can be
 * edited in Inkscape. Draw strokes and fills with `currentColor` so the
 * icon takes the button's text colour.
 */
const FILES = { terminal, reply, compact, resume, remote, close } as const;

export type IconName = keyof typeof FILES;

/** The icon's `<svg>` element, sized and classed for a button. */
export function iconElement(name: IconName, size = 14): SVGSVGElement {
  const tpl = document.createElement("template");
  tpl.innerHTML = FILES[name].replace(/<\?xml[^>]*\?>/, "").trim();
  const svg = tpl.content.querySelector("svg");
  if (!svg) throw new Error(`icon ${name} has no <svg> root`);
  svg.classList.add("icon", `icon-${name}`);
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  return svg as SVGSVGElement;
}

/** A button showing only an icon, with the label as tooltip and accessible name. */
export function iconButton(icon: IconName, label: string, className = "card__btn"): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = `${className} card__btn--icon`;
  b.title = label;
  b.setAttribute("aria-label", label);
  b.replaceChildren(iconElement(icon));
  return b;
}
