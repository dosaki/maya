// The notes of the version this build is, from RELEASE_NOTES.md at build
// time: Settings → Release Notes on the desktop and the phone.

import raw from "../RELEASE_NOTES.md?raw";
import { renderMarkdown } from "./markdown";

export interface ReleaseNotes {
  version: string;
  body: string;
}

/** The version from a first line `# <version>`, and the rest as the body; no such line, no version. */
export function parseReleaseNotes(text: string): ReleaseNotes {
  const m = /^# (.*)\r?\n?/.exec(text);
  if (!m) return { version: "", body: text };
  return { version: m[1].trim(), body: text.slice(m[0].length).replace(/^(\s*\r?\n)+/, "") };
}

export const RELEASE_NOTES: ReleaseNotes = parseReleaseNotes(raw);

/** "Maya <version>", then the notes as Markdown. */
export function renderReleaseNotes(notes: ReleaseNotes = RELEASE_NOTES): HTMLElement {
  const root = document.createElement("div");
  root.className = "release-notes";
  if (notes.version) {
    const hint = document.createElement("p");
    hint.className = "settings__hint";
    hint.textContent = `Maya ${notes.version}`;
    root.append(hint);
  }
  root.append(renderMarkdown(notes.body));
  return root;
}
