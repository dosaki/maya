import claudeCodeIcon from "./assets/claude-code.png";
import type { Harness } from "./types";

export const HARNESS_LABEL: Record<Harness, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  antigravity: "Antigravity",
};

/** Icons by harness; a harness without one falls back to its label as text. */
export const HARNESS_ICON: Partial<Record<Harness, string>> = {
  "claude-code": claudeCodeIcon,
};

export function harnessLabel(h: Harness): string {
  return HARNESS_LABEL[h] ?? h;
}

/** A small badge naming the harness: its icon when there is one, else the label. */
export function harnessBadge(h: Harness, className: string): HTMLElement {
  const label = harnessLabel(h);
  const icon = HARNESS_ICON[h];
  if (icon) {
    const img = document.createElement("img");
    img.className = className;
    img.src = icon;
    img.alt = label;
    img.title = `Running under ${label}`;
    return img;
  }
  const span = document.createElement("span");
  span.className = `${className} ${className}--text`;
  span.textContent = label;
  span.title = `Running under ${label}`;
  return span;
}
