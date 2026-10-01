// What differs in the page between macOS, Windows and Linux: names and shortcuts.

/** True when the page runs in Maya on Windows (WebView2's user agent says so). */
export function isWindows(): boolean {
  return /Windows/.test(navigator.userAgent);
}

/** True when the page runs in Maya on Linux (and not on Android, which also says "Linux"). */
export function isLinux(): boolean {
  return /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);
}

/** This machine in a machine picker: "This Mac", "This PC" or "This computer". */
export function thisComputer(): string {
  if (isLinux()) return "This computer";
  return isWindows() ? "This PC" : "This Mac";
}

/** `thisComputer()`, lowercased as it reads mid-sentence: "this Mac", "this PC", "this computer". */
export function thisComputerLower(): string {
  if (isLinux()) return "this computer";
  return isWindows() ? "this PC" : "this Mac";
}

/** The send shortcut as the keyboard shows it (both are accepted). */
export function sendShortcut(): string {
  return isWindows() || isLinux() ? "Ctrl+Enter" : "⌘↵";
}

/** The built-in voice's name. */
export function builtinVoiceName(): string {
  if (isLinux()) return "speech-dispatcher";
  return isWindows() ? "Zira" : "Samantha";
}

/** Where the ElevenLabs key is kept. */
export function secretStore(): string {
  if (isLinux()) return "GNOME Keyring";
  return isWindows() ? "Credential Manager" : "Keychain";
}

/** The speech recognisers this platform can run, as select options. */
export function recognizerOptions(): ReadonlyArray<readonly ["system" | "builtin", string]> {
  if (isLinux()) return [["builtin", "Built-in (Whisper, runs on this computer)"]];
  if (isWindows()) return [["builtin", "Built-in (Whisper, runs on this PC)"]];
  return [["system", "System (Apple)"], ["builtin", "Built-in (Whisper, runs on this Mac)"]];
}
