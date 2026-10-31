import type { Card } from "./types";

/**
 * A Native session the app just started or resumed opens its live terminal
 * at once, before its card shows (a folder-trust question, a login or an
 * error comes up there). The page says how a terminal is opened.
 */

let opener: ((term: number, dir: string) => void) | null = null;

export function setNativeOpener(f: ((term: number, dir: string) => void) | null): void {
  opener = f;
}

/**
 * Opens the live terminal `term` of a session just started in `dir`, when
 * the start put it in one. True when it did.
 */
export function openStarted(term: number | null | undefined, dir: string): boolean {
  if (term === null || term === undefined || !opener) return false;
  opener(term, dir);
  return true;
}

/** The folder a path ends in, whichever slash separates its parts. */
export function folderName(dir: string): string {
  const parts = dir.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : dir;
}

/** The toast for a Native session that ended before it showed on the board, with its last lines. */
export function endedText(dir: string, lines: string[]): string {
  const head = `The session in ${folderName(dir)} ended before it showed on the board`;
  return lines.length ? `${head}:\n${lines.join("\n")}` : `${head}.`;
}

/** The card of the session `pid` when it runs on this machine in a terminal Maya hosts, whose viewer is then its terminal. */
export function localNativeCard(cards: Card[], pid: number): Card | undefined {
  return cards.find((c) => c.pid === pid && !c.machine && c.native);
}
