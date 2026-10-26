// Which Enter combinations send a message and which make a new line, in
// every box the user types a message into: the Chat tab, a session's reply
// box and the new-session prompt.

import { isLinux, isMac, isWindows } from "./platform";

/** The key combinations for each action, as "Ctrl+Alt+Shift+Meta+Enter" with only the modifiers held. */
export interface KeyBindings {
  send: string[];
  newLine: string[];
}

export type KeyAction = keyof KeyBindings;

/** The actions in Settings' order, with their names. */
export const KEY_ACTIONS = [
  ["send", "Send"],
  ["newLine", "New line"],
] as const;

const MODIFIERS = ["Ctrl", "Alt", "Shift", "Meta"] as const;
const COMBO = /^(Ctrl\+)?(Alt\+)?(Shift\+)?(Meta\+)?Enter$/;

/** Enter sends; Option or Alt with Enter, and ⌘ or Ctrl with Enter, make a new line. */
export function defaultKeys(): KeyBindings {
  return { send: ["Enter"], newLine: ["Alt+Enter", isMac() ? "Meta+Enter" : "Ctrl+Enter"] };
}

let current: KeyBindings = defaultKeys();

/** The keys the boxes follow from now on. */
export function setKeyBindings(keys: KeyBindings): void {
  current = keys;
}

export function keyBindings(): KeyBindings {
  return current;
}

/** Saved keys as key bindings: the defaults when none were saved, and never an empty Send. */
export function asKeyBindings(raw: unknown): KeyBindings {
  if (!raw || typeof raw !== "object") return defaultKeys();
  const saved = raw as { send?: unknown; newLine?: unknown };
  const combos = (v: unknown) => (Array.isArray(v) ? v.filter((c): c is string => typeof c === "string" && COMBO.test(c)) : []);
  const send = combos(saved.send);
  return { send: send.length > 0 ? send : defaultKeys().send, newLine: combos(saved.newLine) };
}

/** The combination an Enter press makes; null for any other key. */
export function comboOf(ev: KeyboardEvent): string | null {
  if (ev.key !== "Enter") return null;
  const held = { Ctrl: ev.ctrlKey, Alt: ev.altKey, Shift: ev.shiftKey, Meta: ev.metaKey };
  return [...MODIFIERS.filter((m) => held[m]), "Enter"].join("+");
}

/**
 * A key press in a message box: a Send combination calls `send`, a New line
 * combination puts a line break at the caret, and anything else is left to
 * the box (so Shift+Enter still adds a line when no action has it).
 */
export function composerKey(ev: KeyboardEvent, box: HTMLTextAreaElement | HTMLInputElement, send: () => void): void {
  if (ev.isComposing) return;
  const combo = comboOf(ev);
  if (!combo) return;
  if (current.send.includes(combo)) {
    ev.preventDefault();
    send();
  } else if (current.newLine.includes(combo)) {
    ev.preventDefault();
    if (box instanceof HTMLTextAreaElement) {
      box.setRangeText("\n", box.selectionStart ?? box.value.length, box.selectionEnd ?? box.value.length, "end");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    }
  }
}

/** The keys with `combo` added to `action`, or why it cannot be. */
export function addCombo(keys: KeyBindings, action: KeyAction, combo: string): KeyBindings | string {
  for (const [other, name] of KEY_ACTIONS) {
    if (keys[other].includes(combo)) return `${name} already uses ${comboLabel(combo)}.`;
  }
  return { ...keys, [action]: [...keys[action], combo] };
}

/** The keys without `combo` in `action`; Send keeps its last combination. */
export function removeCombo(keys: KeyBindings, action: KeyAction, combo: string): KeyBindings {
  if (action === "send" && keys.send.length <= 1) return keys;
  return { ...keys, [action]: keys[action].filter((c) => c !== combo) };
}

/** A combination as the keyboard shows it: "⌥⌘↵" on a Mac, "Ctrl+Alt+Enter" elsewhere. */
export function comboLabel(combo: string): string {
  const keys = combo.split("+");
  if (isMac()) {
    const symbol: Record<string, string> = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Meta: "⌘", Enter: "↵" };
    return keys.map((k) => symbol[k] ?? k).join("");
  }
  const meta = isWindows() ? "Win" : isLinux() ? "Super" : "Meta";
  return keys.map((k) => (k === "Meta" ? meta : k)).join("+");
}

/** The first Send combination, for hints such as "⌘↵ to send". */
export function sendShortcut(): string {
  return comboLabel(current.send[0]);
}
