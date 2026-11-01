import { describe, expect, it, vi } from "vitest";
import { endedText, folderName, localNativeCard, openStarted, setNativeOpener } from "./nativeopen";
import type { Card } from "./types";

describe("opening a Native session the app just started", () => {
  it("opens the terminal only when the start put the session in one", () => {
    const open = vi.fn();
    setNativeOpener(open);
    expect(openStarted(null, "/x/eye")).toBe(false);
    expect(openStarted(undefined, "/x/eye")).toBe(false);
    expect(open).not.toHaveBeenCalled();
    expect(openStarted(0, "/x/eye")).toBe(true);
    expect(open).toHaveBeenCalledWith(0, "/x/eye");
    setNativeOpener(null);
    expect(openStarted(3, "/x/eye")).toBe(false);
  });
});

describe("the toast for a Native session that ended before its card", () => {
  it("names the folder and shows the last lines", () => {
    expect(endedText("/Users/me/dev/eye/", ["zsh:1: command not found: claude"])).toBe(
      "The session in eye ended before it showed on the board:\nzsh:1: command not found: claude",
    );
    expect(endedText("C:\\work\\eye", [])).toBe("The session in eye ended before it showed on the board.");
  });

  it("takes a folder's name from either kind of path", () => {
    expect(folderName("/a/b/c")).toBe("c");
    expect(folderName("C:\\a\\b")).toBe("b");
    expect(folderName("eye")).toBe("eye");
    expect(folderName("/")).toBe("/");
  });
});

describe("the card whose live terminal a session's pid opens", () => {
  const card = (sessionId: string, pid: number, more: Partial<Card> = {}): Card =>
    ({ sessionId, pid, name: sessionId, cwd: "/x", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: false, harness: "claude-code", pr: null, context: null, ...more }) as Card;

  it("is this machine's Native card with that pid", () => {
    const cards = [card("plain", 7), card("remote", 7, { native: true, machine: "laptop" }), card("here", 7, { native: true }), card("other", 8, { native: true })];
    expect(localNativeCard(cards, 7)?.sessionId).toBe("here");
    expect(localNativeCard(cards, 9)).toBeUndefined();
    expect(localNativeCard([card("plain", 7), card("remote", 7, { native: true, machine: "laptop" })], 7)).toBeUndefined();
  });
});
