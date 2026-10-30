import { describe, expect, it } from "vitest";
import { parseReleaseNotes, RELEASE_NOTES, RUNNING_VERSION, renderReleaseNotes } from "./release-notes";

describe("release notes", () => {
  it("reads each version's section, in the file's order", () => {
    expect(parseReleaseNotes("# 0.42.0\n\n## New\n\n- A thing.\n\n# 0.41.0\n\n## Fixed\n\n- Old.\n")).toEqual([
      { version: "0.42.0", body: "## New\n\n- A thing." },
      { version: "0.41.0", body: "## Fixed\n\n- Old." },
    ]);
  });

  it("ignores Windows line endings and trailing spaces on a version line", () => {
    expect(parseReleaseNotes("# 0.42.0 \r\n\r\n- A thing.\r\n").map((n) => n.version)).toEqual(["0.42.0"]);
  });

  it("keeps text with no version line as one section without a version", () => {
    expect(parseReleaseNotes("- A thing.")).toEqual([{ version: "", body: "- A thing." }]);
  });

  it("renders each version under its own heading, its groups beneath it", () => {
    const el = renderReleaseNotes([
      { version: "0.42.0", body: "## New\n\n- A thing." },
      { version: "0.41.0", body: "## Fixed\n\n- Old." },
    ]);
    expect([...el.querySelectorAll(".release-notes__version")].map((h) => h.textContent)).toEqual(["Maya 0.42.0", "Maya 0.41.0"]);
    expect([...el.querySelectorAll("h3")].map((h) => h.textContent)).toEqual(["New", "Fixed"]);
    expect([...el.querySelectorAll("li")].map((li) => li.textContent)).toEqual(["A thing.", "Old."]);
  });

  it("is built from RELEASE_NOTES.md, the running version first", () => {
    expect(RELEASE_NOTES.length).toBeGreaterThan(1);
    expect(RUNNING_VERSION).toBe(RELEASE_NOTES[0].version);
    expect(RUNNING_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("has no <word> in the notes that Markdown would drop as a tag", () => {
    const text = renderReleaseNotes().textContent ?? "";
    for (const { body } of RELEASE_NOTES) {
      for (const m of body.matchAll(/\\?<([a-z][\w ]*)>/gi)) expect(text).toContain(`<${m[1]}>`);
    }
  });
});
