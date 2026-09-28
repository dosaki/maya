import { describe, expect, it } from "vitest";
import { renderOptions } from "./options";
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

  it("shows descriptions when asked and disables while not yet enabled", () => {
    const el = renderOptions(two, 0, { descriptions: true, enabled: false })!;
    expect([...el.querySelectorAll(".options__desc")].map((d) => d.textContent)).toEqual(["small", "large"]);
    expect([...el.querySelectorAll<HTMLButtonElement>("button")].every((b) => b.disabled)).toBe(true);
  });

  it("has no mixed-answer warning for a single question", () => {
    const one: Card = { ...two, awaiting: { kind: "question", detail: "Q", questions: [two.awaiting!.questions[0]] } };
    expect(renderOptions(one, 0, { descriptions: false, enabled: true })!.querySelector(".options__warn")).toBeNull();
  });

  it("disables multi-select questions with a note", () => {
    const el = renderOptions(two, 1, { descriptions: false, enabled: true })!;
    expect(el.querySelector(".options__note")?.textContent).toContain("Multi-select");
    expect(el.querySelector<HTMLButtonElement>("button[data-action=answer]")!.disabled).toBe(true);
  });

  it("returns null when there is nothing to answer", () => {
    expect(renderOptions(two, 2, { descriptions: false, enabled: true })).toBeNull();
    expect(renderOptions({ ...two, state: "working", awaiting: null }, 0, { descriptions: false, enabled: true })).toBeNull();
    expect(renderOptions({ ...two, awaiting: { kind: "permission", detail: "Bash", questions: [] } }, 0, { descriptions: false, enabled: true })).toBeNull();
  });
});
