import { afterEach, describe, expect, it } from "vitest";
import { builtinVoiceName, isWindows, recognizerOptions, secretStore, sendShortcut, thisComputer } from "./platform";

const original = navigator.userAgent;

function pretend(ua: string) {
  Object.defineProperty(navigator, "userAgent", { value: ua, configurable: true });
}

afterEach(() => pretend(original));

describe("platform wording", () => {
  it("names the Mac and its shortcuts on macOS", () => {
    pretend("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15");
    expect(isWindows()).toBe(false);
    expect(thisComputer()).toBe("This Mac");
    expect(sendShortcut()).toBe("⌘↵");
    expect(builtinVoiceName()).toBe("Samantha");
    expect(secretStore()).toBe("Keychain");
    expect(recognizerOptions().map(([v]) => v)).toEqual(["system", "builtin"]);
  });

  it("names the PC and offers only the built-in recogniser on Windows", () => {
    pretend("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0");
    expect(isWindows()).toBe(true);
    expect(thisComputer()).toBe("This PC");
    expect(sendShortcut()).toBe("Ctrl+Enter");
    expect(builtinVoiceName()).toBe("Zira");
    expect(secretStore()).toBe("Credential Manager");
    expect(recognizerOptions()).toEqual([["builtin", "Built-in (Whisper, runs on this PC)"]]);
  });
});
