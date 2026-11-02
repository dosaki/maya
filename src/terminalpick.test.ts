import { describe, expect, it } from "vitest";
import { renderTerminalPick } from "./terminalpick";

const texts = (l: HTMLLabelElement) => [...l.querySelectorAll("option")].map((o) => o.textContent);
const select = (l: HTMLLabelElement) => l.querySelector<HTMLSelectElement>("select[name=terminal]")!;

describe("renderTerminalPick", () => {
  const linux = { platform: "linux", choices: ["terminal", "native"] as const, default: "native" as const };

  it("lists the machine's choices with its labels, no blank entry, and selects the default", () => {
    const l = renderTerminalPick({ ...linux, choices: [...linux.choices] }, null);
    expect(l.textContent).toContain("Terminal");
    expect(texts(l)).toEqual(["Terminal (tmux)", "Native"]);
    expect(select(l).value).toBe("native");
  });

  it("falls back to the default for a value the machine does not list", () => {
    const l = renderTerminalPick({ ...linux, choices: [...linux.choices] }, "iterm");
    expect(select(l).value).toBe("native");
  });

  it("keeps a listed value, and labels macOS choices", () => {
    const l = renderTerminalPick({ platform: "macos", choices: ["terminal", "iterm", "native"], default: "terminal" }, "iterm");
    expect(texts(l)).toEqual(["Terminal", "iTerm2", "Native"]);
    expect(select(l).value).toBe("iterm");
  });
});
