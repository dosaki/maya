// The last command list seen for each machine, agent and folder, so a window
// opens with suggestions before the session answers.

import type { Card, CommandInfo } from "./types";

const KEY = "maya.slashCommands.v1";
const KEEP = 40;

/** One cache entry per machine, agent and folder. */
export function commandsKey(card: Pick<Card, "machine" | "harness" | "cwd">): string {
  return [card.machine ?? "", card.harness, card.cwd].join("\u0000");
}

/** Entries oldest first; a damaged or missing store reads as empty. */
function read(): [string, CommandInfo[]][] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(parsed) ? (parsed as [string, CommandInfo[]][]).filter((e) => Array.isArray(e) && typeof e[0] === "string" && Array.isArray(e[1])) : [];
  } catch {
    return [];
  }
}

export function cachedCommands(key: string): CommandInfo[] | null {
  return read().find(([k]) => k === key)?.[1] ?? null;
}

export function rememberCommands(key: string, list: CommandInfo[]): void {
  try {
    const rest = read().filter(([k]) => k !== key);
    localStorage.setItem(KEY, JSON.stringify([...rest, [key, list]].slice(-KEEP)));
  } catch {
    // Storage may be blocked; the list just is not remembered.
  }
}
