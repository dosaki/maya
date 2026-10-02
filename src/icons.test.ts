import { describe, expect, it } from "vitest";
import { iconButton, iconElement } from "./icons";

describe("icons", () => {
  it("inlines each SVG file with the icon classes, a fixed size and no XML prolog", () => {
    for (const name of ["terminal", "reply", "compact", "resume", "close"] as const) {
      const svg = iconElement(name);
      expect(svg.tagName.toLowerCase()).toBe("svg");
      expect(svg.classList.contains("icon")).toBe(true);
      expect(svg.classList.contains(`icon-${name}`)).toBe(true);
      expect(svg.getAttribute("width")).toBe("14");
      expect(svg.getAttribute("height")).toBe("14");
      expect(svg.getAttribute("aria-hidden")).toBe("true");
    }
    const btn = iconButton("terminal", "Open terminal");
    expect(btn.querySelector("svg.icon-terminal")).not.toBeNull();
    expect(btn.childNodes.length).toBe(1);
    expect(iconElement("resume", 12).getAttribute("width")).toBe("12");
  });
});
