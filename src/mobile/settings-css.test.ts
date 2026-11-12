import { describe, expect, it } from "vitest";
import css from "./mobile.css?raw";

/** The declarations of the first rule whose selector is exactly `selector`. */
function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const m = new RegExp(`(?:^|\\})\\s*${escaped}\\s*\\{([^}]*)\\}`, "m").exec(css);
  if (!m) throw new Error(`no rule for ${selector}`);
  return m[1]!;
}

describe("the phone's Settings sizes", () => {
  it("gives titles 26px, list rows 64px and rows 56px", () => {
    expect(rule(".settings.pane .settings__title")).toContain("font-size: 26px");
    expect(rule(".settings.pane .settings-list__item")).toContain("min-height: 64px");
    expect(rule(".settings.pane .settings__row")).toContain("min-height: 56px");
  });
  it("makes fields 44px at 15px, switches 44x26 and bar buttons 46px", () => {
    const fields = rule(".settings.pane .settings__card input:is([type=text], [type=password], [type=number], [type=search], [type=url]), .settings.pane .settings__card select");
    expect(fields).toContain("height: 44px");
    expect(fields).toContain("font-size: 15px");
    const toggle = rule(".settings.pane .settings__toggle");
    expect(toggle).toContain("width: 44px");
    expect(toggle).toContain("height: 26px");
    expect(rule(".settings.pane .settings__bar .settings__btn")).toContain("min-height: 46px");
  });
  it("keeps a 16px gutter: the page's own padding, not the pane's on top of it", () => {
    expect(rule(".settings.pane")).toContain("padding: 0");
    expect(rule(".settings.pane .settings__body")).toContain("padding: 16px 16px 48px");
  });
  it("stretches each of two side-by-side fields under its label, the number field 44px tall too", () => {
    expect(rule(".settings.pane .settings__field")).toContain("align-items: stretch");
    expect(rule(".settings.pane .settings__card .settings__field > input")).toContain("flex: none");
    // It must outrank the number field's own rule, which comes first.
    expect(css.indexOf(".settings.pane .settings__card .settings__field > input")).toBeGreaterThan(css.indexOf(".settings.pane .settings__card input[type=number]"));
  });
  it("keeps buttons and segmented options at 40px or more", () => {
    expect(rule(".settings.pane .settings__btn")).toContain("min-height: 40px");
    expect(rule(".settings.pane .settings__seg span")).toContain("min-height: 40px");
  });
  it("draws the Release Notes banner as the What's new window does", () => {
    const banner = rule(".settings.pane .settings__rn-banner");
    for (const d of ["height: 200px", "object-fit: cover", "object-position: 50% 6%", "mask-image: linear-gradient(#000 55%, transparent)"]) expect(banner).toContain(d);
    const head = rule(".settings.pane .settings__rn-head");
    for (const d of ["margin: -72px 0 20px", "text-align: right", "text-shadow: 0 1px 8px var(--panel), 0 0 2px var(--panel)"]) expect(head).toContain(d);
    const back = rule(".settings.pane .settings-page__back");
    expect(back).toContain("position: relative");
    expect(back).toContain("z-index: 1");
  });
});

describe("the phone's Join form", () => {
  it("makes its fields 44px", () => {
    expect(rule(".settings.pane .settings__devices [data-for=join-form] input")).toContain("height: 44px");
  });
});
