// What differs in the page between macOS and Windows: names and shortcuts.

/** True when the page runs in Maya on Windows (WebView2's user agent says so). */
export function isWindows(): boolean {
  return /Windows/.test(navigator.userAgent);
}

/** This machine in a machine picker: "This Mac" or "This PC". */
export function thisComputer(): string {
  return isWindows() ? "This PC" : "This Mac";
}

/** The send shortcut as the keyboard shows it (both are accepted). */
export function sendShortcut(): string {
  return isWindows() ? "Ctrl+Enter" : "⌘↵";
}

/** The built-in voice's name. */
export function builtinVoiceName(): string {
  return isWindows() ? "Zira" : "Samantha";
}

/** Where the ElevenLabs key is kept. */
export function secretStore(): string {
  return isWindows() ? "Credential Manager" : "Keychain";
}

/** The speech recognisers this platform can run, as select options. */
export function recognizerOptions(): ReadonlyArray<readonly ["system" | "builtin", string]> {
  if (isWindows()) return [["builtin", "Built-in (Whisper, runs on this PC)"]];
  return [["system", "System (Apple)"], ["builtin", "Built-in (Whisper, runs on this Mac)"]];
}
