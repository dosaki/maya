import type { NewSessionTerminal, TerminalChoices } from "./types";

/** What a place to open a session is called on `platform` ("macos", "windows" or "linux"). */
export function terminalLabel(value: NewSessionTerminal, platform: string): string {
  if (value === "native") return "Native";
  if (value === "iterm") return "iTerm2";
  if (platform === "windows") return "Windows Terminal";
  if (platform === "linux") return "Terminal (tmux)";
  return "Terminal";
}

/** The Terminal select of New Session and Resume: only the machine's own choices, no blank entry. */
export function renderTerminalPick(t: TerminalChoices, value: NewSessionTerminal | null): HTMLLabelElement {
  const field = document.createElement("label");
  field.className = "newsession__field";
  const label = document.createElement("span");
  label.className = "newsession__label";
  label.textContent = "Terminal";
  const select = document.createElement("select");
  select.className = "newsession__select";
  select.name = "terminal";
  for (const c of t.choices) {
    const o = document.createElement("option");
    o.value = c;
    o.textContent = terminalLabel(c, t.platform);
    select.append(o);
  }
  select.value = value && t.choices.includes(value) ? value : t.default;
  field.append(label, select);
  return field;
}
