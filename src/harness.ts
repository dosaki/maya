import antigravityIcon from "./assets/icons/antigravity.png";
import claudeCodeIcon from "./assets/claude-code.png";
import codexIcon from "./assets/icons/codex.svg";
import grokIcon from "./assets/icons/grok.png";
import kiroIcon from "./assets/icons/kiro.png";
import opencodeIcon from "./assets/icons/opencode.svg";
import type { Card, Harness } from "./types";

export const HARNESS_LABEL: Record<Harness, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  antigravity: "Antigravity",
  grok: "Grok Build",
  kiro: "Kiro CLI",
  opencode: "OpenCode",
  other: "Unknown agent",
};

/** Icons by harness; a harness without one falls back to its label as text. */
export const HARNESS_ICON: Partial<Record<Harness, string>> = {
  "claude-code": claudeCodeIcon,
  codex: codexIcon,
  antigravity: antigravityIcon,
  grok: grokIcon,
  kiro: kiroIcon,
  opencode: opencodeIcon,
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
  /** `/exit` ends the session, so Close can type it and then close the terminal. */
  close: boolean;
  /** Stop presses the agent's stop key (or calls its server). */
  stop: boolean;
}

export const CAPABILITIES: Record<Harness, Capabilities> = {
  "claude-code": { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true, stop: true },
  codex: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: true, shellLines: false, close: true, stop: true },
  antigravity: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false, close: true, stop: false },
  grok: { compact: true, modelSwitch: true, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false, close: true, stop: false },
  kiro: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true, stop: false },
  opencode: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true, close: true, stop: true },
  other: { compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false, close: false, stop: false },
};

const NO_CAPABILITIES: Capabilities = { compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false, close: false, stop: false };

/** An agent this build does not know offers no controls, rather than Claude Code's. */
export function capabilitiesOf(h: Harness): Capabilities {
  return CAPABILITIES[h] ?? NO_CAPABILITIES;
}

/** Whether Stop applies to the card: the same rules as `launch::can_stop` in Rust. */
export function canStop(card: Card): boolean {
  const turn = card.state === "working" ? !card.shell : card.state === "awaiting" && !!card.awaiting && card.awaiting.kind !== "text";
  return turn && !card.brain && capabilitiesOf(card.harness).stop;
}
