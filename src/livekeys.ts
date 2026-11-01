// The phone's extra keys under a live terminal: the keys an on-screen
// keyboard cannot send and the agents' menus need. Kept apart from live.ts
// so the viewer can draw them without loading xterm.js.

export type ExtraKey = "esc" | "tab" | "ctrl-c" | "left" | "up" | "down" | "right";

/** The extra keys in the order the row shows them, with their labels. */
export const EXTRA_KEYS: readonly (readonly [ExtraKey, string])[] = [
  ["esc", "Esc"],
  ["tab", "Tab"],
  ["ctrl-c", "Ctrl+C"],
  ["left", "←"],
  ["up", "↑"],
  ["down", "↓"],
  ["right", "→"],
];

const ARROWS = { up: "A", down: "B", right: "C", left: "D" } as const;

/** What `key` types: the arrows as the program asked for them, `ESC O` in its application cursor mode. */
export function keyBytes(key: ExtraKey, appCursor: boolean): string {
  if (key === "esc") return "\x1b";
  if (key === "tab") return "\t";
  if (key === "ctrl-c") return "\x03";
  return (appCursor ? "\x1bO" : "\x1b[") + ARROWS[key];
}
