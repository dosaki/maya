import antigravityIcon from "./assets/icons/antigravity.png";
import claudeCodeIcon from "./assets/claude-code.png";
import codexIcon from "./assets/icons/codex.svg";
import grokIcon from "./assets/icons/grok.png";
import type { Harness } from "./types";

export const HARNESS_LABEL: Record<Harness, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  antigravity: "Antigravity",
  grok: "Grok Build",
};

/** Icons by harness; a harness without one falls back to its label as text. */
export const HARNESS_ICON: Partial<Record<Harness, string>> = {
  "claude-code": claudeCodeIcon,
  codex: codexIcon,
  antigravity: antigravityIcon,
  grok: grokIcon,
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

/** What Maya can type into a running session of each agent: the Rust table in `core/src/launch.rs`. */
export interface Capabilities {
  compact: boolean;
  modelSwitch: boolean;
  effortSwitch: boolean;
  modeCycle: boolean;
  slashLines: boolean;
  shellLines: boolean;
}

export const CAPABILITIES: Record<Harness, Capabilities> = {
  "claude-code": { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true },
  codex: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: true, shellLines: false },
  antigravity: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
  grok: { compact: true, modelSwitch: true, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
};

export function capabilitiesOf(h: Harness): Capabilities {
  return CAPABILITIES[h] ?? CAPABILITIES["claude-code"];
}
