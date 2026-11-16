import { describe, expect, it } from "vitest";
import styles from "./styles.css?raw";

// A leading newline, so the first rule in the file is found like every other.
const css = "\n" + styles;

/** The text between the braces of the rule whose selector is exactly `selector`. */
function block(selector: string): string {
  const start = css.indexOf(`\n${selector} {`);
  if (start < 0) throw new Error(`styles.css has no "${selector}" rule`);
  const open = css.indexOf("{", start);
  return css.slice(open + 1, css.indexOf("}", open));
}

describe("a multi-select row's box", () => {
  // The ticked rule must out-rank the unticked one: with fewer parts it lost,
  // and a ticked row still showed ☐, so ticking looked like it did nothing.
  it("shows ☐ on a row not ticked and ☑ on a ticked one, by a rule at least as specific", () => {
    expect(block(".menu__row[data-pick=tick] .menu__label::before")).toContain('content: "☐ "');
    expect(block(".menu__row[data-pick=tick].menu__row--on .menu__label::before")).toContain('content: "☑ "');
  });

  it("marks a ticked row itself, not only its box", () => {
    expect(block(".menu__row--on")).toMatch(/border-color/);
  });
});
