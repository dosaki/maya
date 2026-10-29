import { describe, expect, it } from "vitest";
import { renderAnsi, stripAnsi } from "./ansi";

const host = (text: string) => {
  const d = document.createElement("div");
  d.append(renderAnsi(text));
  return d;
};

describe("renderAnsi", () => {
  it("bold_dim_and_the_sixteen_colours_become_spans", () => {
    const d = host("\u001b[1mA\u001b[0m \u001b[2mB\u001b[0m \u001b[31mC\u001b[0m");
    const spans = d.querySelectorAll("span");
    expect(spans.length).toBe(3);
    expect(spans[0].classList.contains("ansi-b")).toBe(true);
    expect(spans[1].classList.contains("ansi-d")).toBe(true);
    expect(spans[2].style.color).not.toBe("");
    expect(d.textContent).toBe("A B C");
  });

  it("256_and_truecolor_set_inline_colours", () => {
    expect(host("\u001b[38;5;208mX\u001b[0m").querySelector("span")!.getAttribute("style")).toContain("rgb(");
    expect(host("\u001b[38;2;255;0;0mY\u001b[0m").querySelector("span")!.getAttribute("style")).toContain("rgb(");
  });

  it("non_sgr_sequences_are_dropped", () => {
    const d = host("a\u001b[2Kb\u001b]0;title\u0007c\u001b[Hd");
    expect(d.textContent).toBe("abcd");
    expect(d.querySelector("[style],[class]")).toBeNull();
  });

  it("drops an OSC ended by ST, a two-byte escape and a lone trailing ESC", () => {
    expect(host("a\u001b]8;;u\u001b\\b\u001bMc\u001b").textContent).toBe("abc");
  });

  it("markup_in_the_text_stays_text", () => {
    const d = host("<b>x</b>");
    expect(d.textContent).toBe("<b>x</b>");
    expect(d.querySelector("b")).toBeNull();
  });

  it("22 clears bold and dim; 39 resets the colour", () => {
    const d = host("\u001b[1;2;31mA\u001b[22mB\u001b[39mC");
    expect(d.querySelectorAll("span").length).toBe(2);
    expect(d.textContent).toBe("ABC");
  });

  it("stripAnsi removes every escape sequence", () => {
    expect(stripAnsi("\u001b[1mA\u001b[0m\u001b]0;t\u0007b\u001b")).toBe("Ab");
  });
});
