import { describe, expect, it } from "vitest";
import { sessionTicks, makeTicks, renderOptions, tickOnCard } from "./options";
import type { Card } from "./types";

const two: Card = { sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince: 0, snippet: "", hasInbox: true, harness: "claude-code", pr: null, context: null,
  awaiting: { kind: "question", detail: "Size?", questions: [
    { question: "Size?", header: "Size", multiSelect: false, options: [{ label: "S", description: "small" }, { label: "L", description: "large" }] },
    { question: "Top?", header: "Top", multiSelect: true, options: [{ label: "A", description: "" }] },
  ] } };

describe("renderOptions", () => {
  it("renders the current question's options with indices and tooltips", () => {
    const el = renderOptions(two, 0, { descriptions: false, enabled: true })!;
    expect(el.querySelector(".options__label")?.textContent).toBe("Question 1 of 2 · Size");
    const btns = [...el.querySelectorAll<HTMLButtonElement>("button[data-action=answer]")];
    expect(btns.map((b) => b.textContent)).toEqual(["S", "L"]);
    expect(btns.map((b) => [b.dataset.q, b.dataset.opt])).toEqual([["0", "0"], ["0", "1"]]);
    expect(btns.every((b) => b.dataset.ask === "0")).toBe(true);
    expect(el.querySelector(".options__warn")?.textContent).toContain("not both");
    expect(btns[1].title).toBe("large");
    expect(btns.every((b) => !b.disabled)).toBe(true);
    expect(el.querySelector(".options__desc")).toBeNull();
  });

  it("numbers the answers as stacked rows, and leaves the label to the caller, when asked", () => {
    const el = renderOptions(two, 0, { descriptions: false, enabled: true, numbered: true, label: false })!;
    expect(el.querySelector(".options__label")).toBeNull();
    expect(el.querySelector(".options__list")!.classList.contains("options--rows")).toBe(true);
    const btns = [...el.querySelectorAll<HTMLButtonElement>("button[data-action=answer]")];
    expect(btns.map((b) => [b.querySelector(".options__num")?.textContent, b.querySelector(".options__text")?.textContent])).toEqual([["1", "S"], ["2", "L"]]);
    expect(el.querySelector(".options__warn")).not.toBeNull();
  });

  it("shows descriptions when asked and disables while not yet enabled", () => {
    const el = renderOptions(two, 0, { descriptions: true, enabled: false })!;
    expect([...el.querySelectorAll(".options__desc")].map((d) => d.textContent)).toEqual(["small", "large"]);
    expect([...el.querySelectorAll<HTMLButtonElement>("button")].every((b) => b.disabled)).toBe(true);
  });

  it("has no mixed-answer warning for a single question", () => {
    const one: Card = { ...two, awaiting: { kind: "question", detail: "Q", questions: [two.awaiting!.questions[0]] } };
    expect(renderOptions(one, 0, { descriptions: false, enabled: true })!.querySelector(".options__warn")).toBeNull();
  });

  it("offers a multi-select question's options as boxes to tick, and Send once one is ticked", () => {
    const el = renderOptions(two, 1, { descriptions: false, enabled: true, numbered: true })!;
    expect(el.querySelector("button[data-action=answer]")).toBeNull();
    const box = el.querySelector<HTMLButtonElement>("button[data-action=tick]")!;
    expect([box.dataset.ask, box.dataset.q, box.dataset.opt]).toEqual(["0", "1", "0"]);
    expect(box.disabled).toBe(false);
    expect(box.getAttribute("aria-pressed")).toBe("false");
    expect(box.querySelector(".options__box")?.textContent).toBe("☐");
    const send = el.querySelector<HTMLButtonElement>("button[data-action=send-ticks]")!;
    expect([send.textContent, send.dataset.ask, send.dataset.q]).toEqual(["Send", "0", "1"]);
    expect(send.disabled).toBe(true);
    expect(el.querySelector(".options__note")?.textContent).toMatch(/^Tick one or more, then send them\./);
  });

  it("shows the ticked options as ticked, with Send ready", () => {
    const el = renderOptions(two, 1, { descriptions: false, enabled: true, numbered: true, ticked: [0] })!;
    const box = el.querySelector<HTMLButtonElement>("button[data-action=tick]")!;
    expect(box.classList.contains("options__btn--on")).toBe(true);
    expect(box.getAttribute("aria-pressed")).toBe("true");
    expect(box.querySelector(".options__box")?.textContent).toBe("☑");
    expect(el.querySelector<HTMLButtonElement>("button[data-action=send-ticks]")!.disabled).toBe(false);
  });

  it("disables the boxes and Send inside the open delay, ticked or not", () => {
    const el = renderOptions(two, 1, { descriptions: false, enabled: false, ticked: [0] })!;
    expect([...el.querySelectorAll<HTMLButtonElement>("button")].every((b) => b.disabled)).toBe(true);
  });

  it("returns null when there is nothing to answer", () => {
    expect(renderOptions(two, 2, { descriptions: false, enabled: true })).toBeNull();
    expect(renderOptions({ ...two, state: "working", awaiting: null }, 0, { descriptions: false, enabled: true })).toBeNull();
    expect(renderOptions({ ...two, awaiting: { kind: "permission", detail: "Bash", questions: [] } }, 0, { descriptions: false, enabled: true })).toBeNull();
  });

  it("shows any ask with questions, not only AskUserQuestion", () => {
    const plan: Card = { ...two, awaiting: { kind: "plan", detail: "Plan", questions: [two.awaiting!.questions[0]] } };
    expect(renderOptions(plan, 0, { descriptions: false, enabled: true })!.querySelectorAll("button[data-action=answer]")).toHaveLength(2);
  });
});

describe("makeTicks", () => {
  const card = (stateSince: number): Card => ({ ...two, stateSince });

  it("keeps each session's ticks, in option order, and unticks on a second toggle", () => {
    const ticks = makeTicks();
    expect(ticks.toggle(card(5), 1, 2)).toEqual([2]);
    expect(ticks.toggle(card(5), 1, 0)).toEqual([0, 2]);
    expect(ticks.ticked(card(5), 1)).toEqual([0, 2]);
    expect(ticks.toggle(card(5), 1, 2)).toEqual([0]);
    expect(ticks.ticked({ ...card(5), sessionId: "other" }, 1)).toEqual([]);
  });

  it("forgets the ticks once the ask or the question moves on, or they are cleared", () => {
    const ticks = makeTicks();
    ticks.toggle(card(5), 1, 0);
    expect(ticks.ticked(card(6), 1)).toEqual([]);
    expect(ticks.ticked(card(5), 2)).toEqual([]);
    expect(ticks.toggle(card(6), 1, 1)).toEqual([1]);
    ticks.clear(card(6));
    expect(ticks.ticked(card(6), 1)).toEqual([]);
  });
});

describe("tickOnCard", () => {
  it("ticks and unticks the clicked box at once, readying Send only while one is ticked", () => {
    const card: Card = { ...two, sessionId: "tick-on-card" };
    const el = renderOptions(card, 1, { descriptions: false, enabled: true, numbered: true })!;
    const box = el.querySelector<HTMLButtonElement>("button[data-action=tick]")!;
    const send = el.querySelector<HTMLButtonElement>("button[data-action=send-ticks]")!;
    tickOnCard(card, 1, 0, box.querySelector(".options__text")!);
    expect([box.classList.contains("options__btn--on"), box.getAttribute("aria-pressed"), box.querySelector(".options__box")?.textContent]).toEqual([true, "true", "☑"]);
    expect(send.disabled).toBe(false);
    expect(sessionTicks.ticked(card, 1)).toEqual([0]);
    tickOnCard(card, 1, 0, box);
    expect([box.classList.contains("options__btn--on"), box.getAttribute("aria-pressed"), box.querySelector(".options__box")?.textContent]).toEqual([false, "false", "☐"]);
    expect(send.disabled).toBe(true);
    expect(sessionTicks.ticked(card, 1)).toEqual([]);
  });
});
