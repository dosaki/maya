import { describe, expect, it } from "vitest";
import { parseReleaseNotes, RELEASE_NOTES, renderReleaseNotes } from "./release-notes";

describe("release notes", () => {
  it("reads the version from the first line and keeps the rest as the body", () => {
    expect(parseReleaseNotes("# 0.42.0\n\n## New\n\n- A thing.\n")).toEqual({ version: "0.42.0", body: "## New\n\n- A thing.\n" });
  });

  it("ignores Windows line endings and trailing spaces on the version line", () => {
    expect(parseReleaseNotes("# 0.42.0 \r\n\r\n- A thing.\r\n").version).toBe("0.42.0");
  });

  it("has no version when the first line is not a heading", () => {
    expect(parseReleaseNotes("- A thing.")).toEqual({ version: "", body: "- A thing." });
  });

  it("renders Maya <version> then the notes as Markdown", () => {
    const el = renderReleaseNotes({ version: "0.42.0", body: "## New\n\n- A thing." });
    expect(el.querySelector(".settings__hint")!.textContent).toBe("Maya 0.42.0");
    expect(el.querySelector("h2")!.textContent).toBe("New");
    expect(el.querySelector("li")!.textContent).toBe("A thing.");
  });

  it("is built from RELEASE_NOTES.md", () => {
    expect(RELEASE_NOTES.version).toMatch(/^\d+\.\d+\.\d+$/);
  });
});
