import { describe, expect, it, vi } from "vitest";
import { parseReleaseNotes } from "./release-notes";
import { compareVersions, notesSince, renderWhatsNew } from "./whatsnew";

const f = parseReleaseNotes("# 0.50.0\n\n## New\n\n- a\n\n# 0.49.3\n\n## Fixed\n\n- b\n\n# 0.49.0\n\n## New\n\n- c\n\n# 0.10.0\n\n## New\n\n- d\n");
const versions = (n: { version: string }[]) => n.map((s) => s.version);

describe("compareVersions", () => {
  it("compares by number, and refuses what is not a version", () => {
    expect(compareVersions("0.10.0", "0.9.9")).toBeGreaterThan(0);
    expect(compareVersions("0.9.0", "0.10.0")).toBeLessThan(0);
    expect(compareVersions("1.2.3", "1.2.3")).toBe(0);
    expect(compareVersions("0.9", "0.8.0")).toBeNull();
    expect(compareVersions("", "0.8.0")).toBeNull();
  });
});

describe("notesSince", () => {
  it("keeps the versions after the last one shown, up to this one, newest first", () => {
    expect(versions(notesSince({ from: "0.49.0", running: "0.50.0" }, f))).toEqual(["0.50.0", "0.49.3"]);
  });

  it("keeps only this version when none was shown before", () => {
    expect(versions(notesSince({ from: null, running: "0.50.0" }, f))).toEqual(["0.50.0"]);
  });

  it("orders by number, not as text", () => {
    expect(versions(notesSince({ from: "0.9.0", running: "0.50.0" }, f))).toContain("0.10.0");
  });

  it("finds nothing when this version is not in the notes", () => {
    expect(notesSince({ from: null, running: "0.51.0" }, f)).toEqual([]);
  });

  it("leaves out a section with no version", () => {
    const withPreamble = parseReleaseNotes("intro\n\n# 0.50.0\n\n- a\n");
    expect(versions(notesSince({ from: "0.49.0", running: "0.50.0" }, withPreamble))).toEqual(["0.50.0"]);
  });
});

describe("renderWhatsNew", () => {
  const two = notesSince({ from: "0.49.0", running: "0.50.0" }, f);

  it("says where it updated from, how many versions, and marks this one", () => {
    const el = renderWhatsNew({ from: "0.49.0", running: "0.50.0" }, two, vi.fn());
    const text = el.textContent ?? "";
    expect(text).toContain("Updated from 0.49.0");
    expect(text).toContain("What's new in Maya 0.50.0");
    expect(text).toContain("2 versions since you last opened Maya.");
    expect(text).toContain("All release notes are in Settings › Release Notes.");
    const chips = el.querySelectorAll(".whatsnew__chip");
    expect(chips).toHaveLength(1);
    expect(chips[0].textContent).toBe("This version");
    expect(chips[0].closest(".release-notes__version")?.textContent).toContain("Maya 0.50.0");
    expect(el.querySelector<HTMLImageElement>("img.whatsnew__art")?.alt).toBe("");
    expect(el.querySelector(".whatsnew__close")?.getAttribute("aria-label")).toBe("Close");
  });

  it("says one version in the singular", () => {
    const one = notesSince({ from: "0.49.3", running: "0.50.0" }, f);
    expect(renderWhatsNew({ from: "0.49.3", running: "0.50.0" }, one, vi.fn()).textContent).toContain("1 version since you last opened Maya.");
  });

  it("has no Updated from and no count when no version was shown before", () => {
    const el = renderWhatsNew({ from: null, running: "0.50.0" }, notesSince({ from: null, running: "0.50.0" }, f), vi.fn());
    expect(el.textContent).not.toContain("Updated from");
    expect(el.textContent).not.toContain("since you last opened");
    expect(el.textContent).toContain("What's new in Maya 0.50.0");
  });

  it("closes from Got it, the × and the backdrop, not from inside the panel", () => {
    const onClose = vi.fn();
    const el = renderWhatsNew({ from: "0.49.0", running: "0.50.0" }, two, onClose);
    el.querySelector<HTMLElement>(".whatsnew__notes")!.click();
    expect(onClose).not.toHaveBeenCalled();
    const ok = el.querySelector<HTMLButtonElement>(".whatsnew__ok")!;
    expect(ok.textContent).toBe("Got it");
    ok.click();
    el.querySelector<HTMLElement>(".whatsnew__close")!.click();
    el.querySelector<HTMLElement>(".modal__backdrop")!.click();
    expect(onClose).toHaveBeenCalledTimes(3);
  });
});
