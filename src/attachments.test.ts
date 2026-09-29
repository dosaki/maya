import { describe, expect, it } from "vitest";
import { composeMessage, makeAttachments, pastedFiles, renderChips } from "./attachments";

describe("attachments", () => {
  it("keeps a de-duplicated list by path and renders removable chips", () => {
    const a = makeAttachments();
    a.add({ name: "shot.png", path: "/Users/x/.claude/maya/attachments/1-shot.png" });
    a.add({ name: "notes.md", path: "/Users/x/dev/notes.md" });
    a.add({ name: "shot.png", path: "/Users/x/.claude/maya/attachments/1-shot.png" });
    expect(a.list().map((x) => x.name)).toEqual(["shot.png", "notes.md"]);
    const chips = renderChips(a.list(), (path) => a.remove(path));
    expect([...chips.querySelectorAll(".chip__name")].map((c) => c.textContent)).toEqual(["shot.png", "notes.md"]);
    chips.querySelector<HTMLButtonElement>(".chip__remove")!.click();
    expect(a.list().map((x) => x.name)).toEqual(["notes.md"]);
    a.clear();
    expect(a.list()).toEqual([]);
    expect(renderChips([], () => undefined).childElementCount).toBe(0);
  });

  it("appends one line per attachment to the message, and sends attachments alone", () => {
    const files = [{ name: "a.png", path: "/p/a.png" }, { name: "b.txt", path: "/p/b.txt" }];
    expect(composeMessage("look at these", files)).toBe("look at these\n\nAttached file: /p/a.png\nAttached file: /p/b.txt");
    expect(composeMessage("   ", files)).toBe("Attached file: /p/a.png\nAttached file: /p/b.txt");
    expect(composeMessage("just text", [])).toBe("just text");
    expect(composeMessage("  ", [])).toBe("");
  });

  it("picks files out of pasted clipboard items, naming unnamed images", () => {
    const img = new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" });
    const doc = new File(["x"], "report.pdf", { type: "application/pdf" });
    const items = [
      { kind: "string", type: "text/plain", getAsFile: () => null },
      { kind: "file", type: "image/png", getAsFile: () => img },
      { kind: "file", type: "application/pdf", getAsFile: () => doc },
    ];
    const got = pastedFiles(items, 1_700_000_000_000);
    expect(got.map((f) => f.name)).toEqual(["pasted-1700000000000.png", "report.pdf"]);
    expect(got[0].file).toBe(img);
    expect(pastedFiles([{ kind: "string", type: "text/plain", getAsFile: () => null }], 1)).toEqual([]);
  });
});
