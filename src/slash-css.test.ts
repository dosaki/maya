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

describe("the slash list on a narrow screen", () => {
  // A long plugin name with `flex: none` pushed the row past a 360 px phone.
  it("cuts a long name with an ellipsis, at most part of the row", () => {
    const name = block(".slash__name");
    expect(name).not.toMatch(/flex: none/);
    expect(name).toMatch(/min-width: 0/);
    expect(name).toMatch(/max-width: \d+%/);
    expect(name).toMatch(/overflow: hidden/);
    expect(name).toMatch(/text-overflow: ellipsis/);
    expect(name).toMatch(/white-space: nowrap/);
  });

  it("never scrolls sideways", () => {
    expect(block(".slash")).toMatch(/overflow-x: hidden/);
  });
});
