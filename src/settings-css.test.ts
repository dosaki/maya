import { describe, expect, it } from "vitest";
import css from "./styles.css?raw";

// jsdom applies no stylesheet, so these read the rules the WebKit screenshots
// of every Settings screen showed were needed.

/** The declarations of the first rule whose selector is exactly `selector`, inside `within` (the whole sheet by default). */
function rule(selector: string, within = css): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const m = new RegExp(`(?:^|[}{])\\s*${escaped}\\s*\\{([^}]*)\\}`, "m").exec(within);
  if (!m) throw new Error(`no rule for ${selector}`);
  return m[1]!;
}

/** The body of the first `@media (max-width: <px>px)` block that mentions `marker`. */
function media(px: number, marker: string): string {
  const re = new RegExp(`@media \\(max-width: ${px}px\\) \\{`, "g");
  for (let m = re.exec(css); m; m = re.exec(css)) {
    let depth = 1;
    let i = re.lastIndex;
    for (; i < css.length && depth > 0; i++) depth += css[i] === "{" ? 1 : css[i] === "}" ? -1 : 0;
    const body = css.slice(re.lastIndex, i - 1);
    if (body.includes(marker)) return body;
  }
  throw new Error(`no @media (max-width: ${px}px) block with ${marker}`);
}

describe("the Settings screens' spacing", () => {
  it("spaces a screen's groups by their own margin, with no gap added between them", () => {
    expect(rule(".settings.pane .settings__section")).toContain("gap: 0");
  });

  it("draws the sidebar's items as the mockup does: 8px radius, 8px 12px padding, a soft hover", () => {
    const item = rule(".settings__body--sidebar .settings__nav-item");
    expect(item).toContain("border-radius: 8px");
    expect(item).toContain("padding: 8px 12px");
    expect(rule(".settings__body--sidebar .settings__nav-item:hover")).toContain("background: var(--soft)");
  });
});

describe("Settings fields", () => {
  it("keeps a number field 88px wide: only other fields widen the control to 300px", () => {
    const wide = /\.settings\.pane \.settings__control:has\(([^{]*)\)\s*\{\s*flex-basis: 300px/.exec(css)!;
    expect(wide[1]).toContain(":not([type=number])");
    expect(css).toMatch(/\.settings\.pane \.settings__control > input:not\(\[type=checkbox\]\):not\(\[type=radio\]\):not\(\[type=number\]\), \.settings\.pane \.settings__control > select \{ flex: 1; \}/);
    expect(rule(".settings.pane .settings__card input[type=number]")).toContain("flex: 0 0 88px");
  });

  it("gives the volume slider the 300px field width, in gold", () => {
    expect(rule(".settings.pane .settings__control:has(> .settings__range)")).toContain("flex-basis: 300px");
    expect(rule(".settings.pane .settings__range")).toContain("width: 100%");
    expect(rule(".settings.pane .settings__range input[type=range]")).toContain("accent-color: var(--gold)");
  });

  it("draws a download's progress bar in gold", () => {
    expect(rule(".settings.pane .settings__card progress")).toContain("accent-color: var(--gold)");
  });

  it("rounds a head that is its card's only row at every corner", () => {
    expect(rule(".settings.pane .settings__row--head:last-child")).toContain("border-radius: 12px");
  });

  it("shows a Git test's outcome as a muted line that wraps a long word", () => {
    const out = rule(".settings.pane .settings__git-test");
    for (const d of ["min-width: 0", "color: var(--muted)", "font-size: 13px", "overflow-wrap: anywhere"]) expect(out).toContain(d);
  });

  it("turns the details' marker about its own middle", () => {
    const marker = rule(".settings.pane .settings__details > summary::before");
    expect(marker).toContain("width: 10px");
    expect(marker).toContain("text-align: center");
  });
});

describe("Settings under 560px", () => {
  const narrow = media(560, ".settings.pane .settings__row");

  it("stacks only the rows whose control is a field; a switch, a choice or a button stays beside its label", () => {
    const stack = /([^{}]*)\{\s*flex-direction: column; align-items: stretch;[^}]*\}/.exec(narrow)!;
    expect(stack[1]).toContain(".settings__row:has(> .settings__control > :is(input:not([type=checkbox]):not([type=radio]):not([type=number]), select, textarea))");
    expect(narrow).not.toMatch(/\.settings\.pane \.settings__row \{ flex-direction: column/);
  });

  it("lets a stacked control size to its field, not to the 300px basis, which would become a height", () => {
    expect(narrow).toMatch(/\.settings__control \{[^}]*flex-basis: auto/);
  });

  it("puts a device's state and actions on a line under its name, after the rows' own rules so it wins", () => {
    const narrow = media(560, ".settings__device-text");
    expect(css.indexOf(narrow)).toBeGreaterThan(css.indexOf(".settings.pane .settings__device-text { flex: 1; min-width: 0; }"));
    expect(rule(".settings.pane .settings__device", narrow)).toContain("flex-wrap: wrap");
    expect(rule(".settings.pane .settings__device-text", narrow)).toContain("flex: 1 1 calc(100% - 68px)");
    expect(rule(".settings.pane .settings__device > .settings__state", narrow)).toContain("margin-left: 68px");
  });
});

describe("the Devices rows", () => {
  it("puts the mirror switch's words right beside it", () => {
    expect(rule(".settings.pane .settings__device-extra")).toContain("justify-content: flex-start");
  });

  it("leaves a row's actions in DOM order: Remove is last in the markup, so no `order` moves it", () => {
    expect(css).not.toMatch(/settings__assistant-actions > \.settings__btn--danger/);
    expect(rule(".settings.pane .settings__assistant-actions")).not.toContain("order");
  });

  it("draws the Join form as a wrapping row: the host fills it, the port is 88px, fields look like a card's", () => {
    const form = ".settings.pane .settings__devices [data-for=join-form]";
    const r = rule(form);
    expect(r).toContain("flex-direction: row");
    expect(r).toContain("flex-wrap: wrap");
    const host = rule(`${form} input[name=join-host]`);
    expect(host).toContain("flex: 1 1 220px");
    expect(host).toContain("width: auto");
    const port = rule(`${form} input[name=join-port]`);
    expect(port).toContain("flex: 0 0 88px");
    expect(port).toContain("width: 88px");
    const field = rule(`${form} input`);
    for (const d of ["height: 36px", "background: var(--field)", "border-radius: 8px"]) expect(field).toContain(d);
  });
});

describe("Work Items and Keyboard details", () => {
  it("dims an off source's body except the row holding the link to Git settings", () => {
    const r = rule(".settings.pane .settings__source--off .settings__source-body > :not(:has(.settings__link))");
    expect(r).toContain("opacity: .5");
    expect(css).not.toMatch(/\.settings__source--off \.settings__source-body \{/);
  });

  it("draws the Keyboard row's error in the danger colour inside the pane", () => {
    expect(rule(".settings.pane .settings__keys .settings__error")).toContain("color: var(--danger)");
  });

  it("pads the hint a card holds on Devices", () => {
    expect(rule(".settings.pane .settings__card > .settings__hint")).toContain("padding: 16px 20px");
  });
});

describe("the Release Notes art", () => {
  it("fits its column, 64px into the gap, so her hand never reaches the notes", () => {
    const art = rule(".settings.pane .settings__rn-art");
    for (const d of ["width: calc(100% + 64px)", "height: auto", "max-height: min(76vh, 600px)", "object-fit: contain", "margin-left: -64px"]) expect(art).toContain(d);
  });
});
