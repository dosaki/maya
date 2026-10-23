import { describe, expect, it } from "vitest";
import styles from "./styles.css?raw";

// A leading newline, so the first rule in the file is found like every other.
const css = "\n" + styles;

const DARK = ":root";
const LIGHT = ':root[data-theme="light"]';
const TERMINAL_UNDER_LIGHT = ':root[data-theme="light"] .modal--terminal';

/** The text between the braces of the rule whose selector is exactly `selector`. */
function block(selector: string): string {
  const start = css.indexOf(`\n${selector} {`);
  if (start < 0) throw new Error(`styles.css has no "${selector}" rule`);
  const open = css.indexOf("{", start);
  return css.slice(open + 1, css.indexOf("}", open));
}

/** The custom properties a rule defines. */
function vars(selector: string): Record<string, string> {
  const found: Record<string, string> = {};
  for (const match of block(selector).matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) found[match[1]] = match[2].trim();
  return found;
}

/** What Maya looked like before themes existed: the default must not move. */
const ORIGINAL_DARK: Record<string, string> = {
  "--bg": "#0c0b09",
  "--panel": "#16140f",
  "--text": "#f1e9d8",
  "--muted": "#a39a86",
  "--border": "#2c2719",
  "--gold": "#d9ab3c",
  "--gold-bright": "#f0c85a",
  "--gold-deep": "#8f6a1c",
  "--awaiting": "#d97706",
  "--working": "#2563eb",
  "--completed": "#16a34a",
  "--idle": "#9ca3af",
  "--on-gold": "#1a1408",
  "--shadow": "0 1px 2px rgba(0,0,0,.5)",
};

/** The state colours read the same on both themes, so the light theme does not repeat them. */
const SHARED = ["--awaiting", "--working", "--completed", "--idle"];

function luminance(hex: string): number {
  const [red, green, blue] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

/** WCAG contrast ratio of two #rrggbb colours. */
function contrast(first: string, second: string): number {
  const [lighter, darker] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

describe("the dark theme, which is the default", () => {
  it("keeps every colour it had before themes existed", () => {
    expect(vars(DARK)).toMatchObject(ORIGINAL_DARK);
    expect(block(DARK)).toContain("color-scheme: dark");
  });
});

describe("the light theme", () => {
  it("defines every variable the dark theme does, except the shared state colours", () => {
    const missing = Object.keys(vars(DARK)).filter((name) => !(name in vars(LIGHT)) && !SHARED.includes(name));
    expect(missing).toEqual([]);
  });

  it("tells the browser it is light, so scrollbars and form controls follow", () => {
    expect(block(LIGHT)).toContain("color-scheme: light");
  });

  it("does not redefine the shared state colours, so they cannot drift apart", () => {
    for (const name of SHARED) expect(vars(LIGHT)).not.toHaveProperty(name);
  });

  it("keeps the terminal-look viewer dark, like the terminal it imitates", () => {
    const terminal = vars(TERMINAL_UNDER_LIGHT);
    for (const [name, value] of Object.entries(ORIGINAL_DARK)) {
      if (!SHARED.includes(name)) expect(terminal[name], name).toBe(value);
    }
    expect(block(TERMINAL_UNDER_LIGHT)).toContain("color-scheme: dark");
  });
});

describe.each([
  ["dark", vars(DARK)],
  ["light", { ...vars(DARK), ...vars(LIGHT) }],
])("readability of the %s theme", (_name, colors) => {
  const ratio = (foreground: string, background: string) => contrast(colors[foreground], colors[background]);

  it("makes body text easy to read on the page and on panels", () => {
    expect(ratio("--text", "--bg")).toBeGreaterThanOrEqual(7);
    expect(ratio("--text", "--panel")).toBeGreaterThanOrEqual(7);
  });

  it("keeps muted text and gold headings above the 4.5:1 minimum for small text", () => {
    for (const background of ["--bg", "--panel"]) {
      expect(ratio("--muted", background), `muted on ${background}`).toBeGreaterThanOrEqual(4.5);
      expect(ratio("--gold-bright", background), `gold-bright on ${background}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("keeps the gold accent visible as a border or icon (3:1) and readable text on gold buttons", () => {
    expect(ratio("--gold", "--bg")).toBeGreaterThanOrEqual(3);
    expect(ratio("--gold", "--panel")).toBeGreaterThanOrEqual(3);
    expect(ratio("--on-gold", "--gold")).toBeGreaterThanOrEqual(4.5);
  });
});
