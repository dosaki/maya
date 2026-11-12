import { describe, expect, it, vi } from "vitest";
import { card, cardHead, choice, group, notice, row, source, state, toggle } from "./settings-ui";

function select(name: string): HTMLSelectElement {
  const s = document.createElement("select");
  s.name = name;
  return s;
}

describe("row", () => {
  it("is a label with the help linked to a named control", () => {
    const ctl = select("agent");
    const r = row({ label: "Agent", help: "x", control: ctl });
    expect(r.tagName).toBe("LABEL");
    expect(r.classList.contains("settings__row")).toBe(true);
    expect(r.querySelector(".settings__label")!.textContent).toBe("Agent");
    expect(r.querySelector("#settings-help-agent.settings__help")!.textContent).toBe("x");
    expect(ctl.getAttribute("aria-describedby")).toBe("settings-help-agent");
  });
  it("scopes the help id when asked, so two rows with one control name stay unique", () => {
    const a = row({ label: "A", help: "x", control: select("same"), idScope: "github" });
    const b = row({ label: "B", help: "y", control: select("same"), idScope: "gitlab" });
    document.body.append(a, b);
    const ids = [...document.querySelectorAll("[id]")].map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(a.querySelector("select")!.getAttribute("aria-describedby")).toBe(a.querySelector(".settings__help")!.id);
    a.remove();
    b.remove();
  });
  it("is a div with several controls", () => {
    const a = document.createElement("button");
    const b = document.createElement("button");
    expect(row({ label: "L", control: [a, b] }).tagName).toBe("DIV");
  });
  it("can be stacked", () => {
    expect(row({ label: "L", stack: true }).classList.contains("settings__row--stack")).toBe(true);
  });
});

describe("group", () => {
  it("keeps title, note, children, foot in order", () => {
    const c = card();
    const g = group({ title: "Agent", note: "n", foot: "f" }, c);
    expect([...g.children].map((e) => e.className)).toEqual([
      "settings__group-title",
      "settings__group-note",
      "settings__card",
      "settings__group-foot",
    ]);
    expect(g.children[0]!.tagName).toBe("H3");
  });
  it("holds only the card when nothing else is given", () => {
    expect(group({}, card()).children).toHaveLength(1);
  });
});

describe("toggle", () => {
  it("is a named checkbox that reports its state", () => {
    const fn = vi.fn();
    const t = toggle("listen", true, fn);
    expect(t.type).toBe("checkbox");
    expect(t.checked).toBe(true);
    expect(t.name).toBe("listen");
    expect(t.classList.contains("settings__toggle")).toBe(true);
    t.checked = false;
    t.dispatchEvent(new Event("change"));
    expect(fn).toHaveBeenCalledWith(false);
  });
});

describe("choice", () => {
  it("makes a radio group", () => {
    const fn = vi.fn();
    const c = choice("gitClone-github", [["ssh", "SSH"], ["https", "HTTPS"]] as const, "https", fn);
    document.body.append(c);
    const radios = c.querySelectorAll<HTMLInputElement>("input[type=radio]");
    expect(radios).toHaveLength(2);
    expect([...radios].every((r) => r.name === "gitClone-github")).toBe(true);
    expect(c.querySelector<HTMLInputElement>("input[value=https]")!.checked).toBe(true);
    c.querySelector<HTMLInputElement>("input[value=ssh]")!.click();
    expect(fn).toHaveBeenCalledWith("ssh");
    expect(c.getAttribute("role")).toBe("radiogroup");
    c.remove();
  });
});

describe("state and notice", () => {
  it("state carries its tone", () => {
    const s = state("Installed", "on");
    expect(s.classList.contains("settings__state")).toBe(true);
    expect(s.classList.contains("settings__state--on")).toBe(true);
  });
  it("an error notice keeps settings__error", () => {
    const n = notice("boom", "error");
    expect(n.classList.contains("settings__notice")).toBe(true);
    expect(n.classList.contains("settings__error")).toBe(true);
  });
});

describe("cardHead", () => {
  it("numbers the head", () => {
    const h = cardHead({ label: "ElevenLabs", order: 1 });
    expect(h.querySelector(".settings__order")!.textContent).toBe("1");
    expect(h.classList.contains("settings__row--head")).toBe(true);
  });
});

describe("source", () => {
  function make(open: boolean, onToggle = vi.fn()) {
    const control = toggle(null, true, () => {});
    const el = source({
      key: "github", name: "GitHub", summary: "sum", stateText: "On", stateOn: true,
      open, onToggle, control, body: [document.createElement("div")],
    });
    return { el, control, onToggle };
  }
  it("opens and closes in place", () => {
    const { el, onToggle } = make(false);
    const t = el.querySelector<HTMLButtonElement>(".settings__source-toggle")!;
    expect(el.hasAttribute("data-open")).toBe(false);
    expect(t.getAttribute("aria-expanded")).toBe("false");
    expect(el.querySelector(".settings__source-sum")!.textContent).toBe("sum");
    t.click();
    expect(el.hasAttribute("data-open")).toBe(true);
    expect(t.getAttribute("aria-expanded")).toBe("true");
    expect(onToggle).toHaveBeenCalledWith(true);
  });
  it("leaves the open state alone when the control is clicked", () => {
    const { el, control, onToggle } = make(false);
    control.click();
    expect(el.hasAttribute("data-open")).toBe(false);
    expect(onToggle).not.toHaveBeenCalled();
  });
  it("uses a button for the toggle", () => {
    const { el } = make(false);
    expect(el.querySelector(".settings__source-toggle")!.tagName).toBe("BUTTON");
    expect(el.dataset.source).toBe("github");
  });
});

describe("a div row names its controls", () => {
  it("points a choice at the row's label", () => {
    const r = row({ label: "Token", control: choice("kind", [["a", "A"], ["b", "B"]] as const, "a", () => undefined), tag: "div" });
    const label = r.querySelector<HTMLElement>(".settings__label")!;
    expect(label.id).toBe("settings-label-kind");
    expect(r.querySelector("[role=radiogroup]")!.getAttribute("aria-labelledby")).toBe(label.id);
    // The radios are named by their own labels.
    expect(r.querySelector("input[type=radio]")!.hasAttribute("aria-labelledby")).toBe(false);
  });

  it("names each field of a multi-control row, scoping the id, and keeps a name already given", () => {
    const a = document.createElement("input");
    a.name = "timeout";
    const b = toggle("other", true, () => undefined, "Own name");
    const r = row({ label: "Wait", control: [a, b], idScope: "x" });
    const id = r.querySelector(".settings__label")!.id;
    expect(id).toBe("settings-label-x-timeout");
    expect(a.getAttribute("aria-labelledby")).toBe(id);
    expect(b.hasAttribute("aria-labelledby")).toBe(false);
  });

  it("leaves a label row to its wrapping label", () => {
    const ctl = select("agent");
    const r = row({ label: "Agent", control: ctl });
    expect(r.tagName).toBe("LABEL");
    expect(ctl.hasAttribute("aria-labelledby")).toBe(false);
  });

  it("names the card-head switches the Voice screen draws", () => {
    const sw = toggle("elevenEnabled", true, () => undefined, "Use ElevenLabs");
    expect(cardHead({ label: "ElevenLabs", control: sw }).querySelector("input")!.getAttribute("aria-label")).toBe("Use ElevenLabs");
  });
});
