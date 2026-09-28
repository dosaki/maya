import { beforeEach, describe, expect, it, vi } from "vitest";
import { makeAnswerGuard } from "./answer";
import type { Card } from "./types";

const card: Card = { sessionId: "s", pid: 1, name: "n", cwd: "/x", state: "awaiting", stateSince: 500, snippet: "", hasInbox: true, harness: "claude-code", pr: null, context: null,
  awaiting: { kind: "question", detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "" }, { label: "B", description: "" }] }] } };

describe("makeAnswerGuard", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div class="options"><button data-action="answer" data-q="0" data-opt="0">A</button><button data-action="answer" data-q="0" data-opt="1">B</button></div>';
  });

  it("invokes once with the ask id, disables the buttons synchronously and drops a second click while pending", async () => {
    let release: () => void = () => {};
    const send = vi.fn(() => new Promise<void>((r) => { release = r; }));
    const guard = makeAnswerGuard(send);
    const btn = document.querySelector<HTMLButtonElement>('button[data-opt="1"]')!;
    const p1 = guard.answer(card, 0, 1, btn);
    const p2 = guard.answer(card, 0, 0, btn);
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith({ sessionId: "s", askId: 500, questionIndex: 0, optionIndex: 1 });
    expect([...document.querySelectorAll<HTMLButtonElement>("button")].every((b) => b.disabled)).toBe(true);
    release();
    await Promise.all([p1, p2]);
    expect(await p2).toBe("dropped");
    expect(await p1).toBe("sent");
  });

  it("allows a new click after the previous one settled, and per session", async () => {
    const send = vi.fn(() => Promise.resolve());
    const guard = makeAnswerGuard(send);
    const btn = document.querySelector<HTMLButtonElement>("button")!;
    await guard.answer(card, 0, 0, btn);
    await guard.answer({ ...card, sessionId: "other" }, 0, 0, btn);
    await guard.answer(card, 0, 1, btn);
    expect(send).toHaveBeenCalledTimes(3);
  });
});
