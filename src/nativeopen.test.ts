import { describe, expect, it, vi } from "vitest";
import { endedText, folderName, openStarted, setNativeOpener } from "./nativeopen";

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
