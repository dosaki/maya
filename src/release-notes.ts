// The notes of every version of this major release, newest first, from
// RELEASE_NOTES.md at build time: Settings → Release Notes on the desktop
// and the phone.

import raw from "../RELEASE_NOTES.md?raw";
import { renderMarkdown } from "./markdown";

export interface ReleaseNotes {
  version: string;
  body: string;
}

/** One section per `# <version>` line, in the file's order; text before any such line is a section without a version. */
export function parseReleaseNotes(text: string): ReleaseNotes[] {
  const sections: ReleaseNotes[] = [];
  let current: ReleaseNotes | null = null;
  const lines: string[] = [];
  const flush = () => {
    const body = lines.join("\n").trim();
    if (current) sections.push({ version: current.version, body });
    else if (body) sections.push({ version: "", body });
    lines.length = 0;
  };
  for (const line of text.replace(/\r/g, "").split("\n")) {
    if (line.startsWith("# ")) {
      flush();
      current = { version: line.slice(2).trim(), body: "" };
    } else {
      lines.push(line);
    }
  }
  flush();
  return sections;
}

export const RELEASE_NOTES: ReleaseNotes[] = parseReleaseNotes(raw);

/** The version this build is: the file's first section, which CI keeps equal to the version files. */
export const RUNNING_VERSION = RELEASE_NOTES[0]?.version ?? "";

/** Each version under a "Maya <version>" heading, newest first, its New and Fixed beneath it. */
export function renderReleaseNotes(notes: ReleaseNotes[] = RELEASE_NOTES): HTMLElement {
  const root = document.createElement("div");
  root.className = "release-notes";
  for (const { version, body } of notes) {
    if (version) {
      const heading = document.createElement("h2");
      heading.className = "release-notes__version";
      heading.textContent = `Maya ${version}`;
      root.append(heading);
    }
    // A version's own groups sit one level under its heading.
    root.append(renderMarkdown(body.replace(/^## /gm, "### ")));
  }
  return root;
}
