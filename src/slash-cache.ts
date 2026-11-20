// The last command list seen for each machine, agent and folder, so a window
// opens with suggestions before the session answers.

import type { Card, CommandInfo } from "./types";

const KEY = "maya.slashCommands.v1";
const KEEP = 40;

/** One cache entry per machine, agent and folder. */
export function commandsKey(card: Pick<Card, "machine" | "harness" | "cwd">): string {
  return [card.machine ?? "", card.harness, card.cwd].join("\u0000");
}

/** A stored command, read back with the shape the list needs; null when it is damaged. */
function command(raw: unknown): CommandInfo | null {
  if (!raw || typeof raw !== "object") return null;
  const { name, description, source } = raw as Record<string, unknown>;
  if (typeof name !== "string" || name === "") return null;
  return { name, description: typeof description === "string" ? description : "", source: typeof source === "string" ? source : "other" };
}

/** Entries oldest first; a damaged or missing store, or a damaged entry, reads as empty. */
function read(): [string, CommandInfo[]][] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((e): [string, CommandInfo[]][] => {
      if (!Array.isArray(e) || typeof e[0] !== "string" || !Array.isArray(e[1])) return [];
      const list = e[1].map(command);
      return list.every((c): c is CommandInfo => c !== null) ? [[e[0], list]] : [];
    });
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
