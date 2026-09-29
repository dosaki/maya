import { describe, expect, it, vi } from "vitest";
import { renderDebug, visibleLines, type LogLine } from "./debug";

const lines: LogLine[] = [
  { at: 1_700_000_000_000, source: "ear", text: "final: \"Maya what's waiting\"" },
  { at: 1_700_000_000_500, source: "wake", text: "heard (idle): \"Maya what's waiting\" → command \"what's waiting\"" },
  { at: 1_700_000_001_000, source: "interpreter", text: "asking haiku: what's waiting\nboard:\nhexgrid | claude-code | idle" },
  { at: 1_700_000_002_000, source: "speech", text: "saying: Nothing is waiting." },
];

describe("debug pane", () => {
  it("filters by source, keeping everything when no source is chosen", () => {
    expect(visibleLines(lines, new Set()).length).toBe(4);
    expect(visibleLines(lines, new Set(["wake", "interpreter"])).map((l) => l.source)).toEqual(["ear", "speech"]);
  });

  it("shows each line with its time and source, multi-line text intact, and one chip per source", () => {
    const h = { onToggle: vi.fn(), onClear: vi.fn(), onCopy: vi.fn() };
    const el = renderDebug({ lines, hidden: new Set(["speech"]), path: "/Users/x/.claude/maya/maya.log" }, h);
    const rows = [...el.querySelectorAll(".debug__line")];
    expect(rows.length).toBe(3);
    expect(rows[0].querySelector(".debug__source")?.textContent).toBe("ear");
    expect(rows[0].querySelector(".debug__time")?.textContent).toMatch(/^\d\d:\d\d:\d\d\.\d\d\d$/);
    expect(rows[2].querySelector(".debug__text")?.textContent).toContain("board:\nhexgrid");
    const chips = [...el.querySelectorAll<HTMLButtonElement>(".debug__chip")];
    expect(chips.map((c) => c.textContent)).toEqual(["ear", "interpreter", "speech", "wake"]);
    expect(chips.find((c) => c.textContent === "speech")?.classList.contains("debug__chip--off")).toBe(true);
    chips[0].click();
    expect(h.onToggle).toHaveBeenCalledWith("ear");
    el.querySelector<HTMLButtonElement>("button[data-action=log-clear]")!.click();
    expect(h.onClear).toHaveBeenCalled();
    el.querySelector<HTMLButtonElement>("button[data-action=log-copy]")!.click();
    expect(h.onCopy).toHaveBeenCalled();
    expect(el.querySelector(".debug__path")?.textContent).toContain("maya.log");
  });

  it("says so when there is nothing to show", () => {
    const el = renderDebug({ lines: [], hidden: new Set(), path: "" }, { onToggle: vi.fn(), onClear: vi.fn(), onCopy: vi.fn() });
    expect(el.querySelector(".debug__empty")?.textContent).toContain("Nothing logged yet");
  });
});
