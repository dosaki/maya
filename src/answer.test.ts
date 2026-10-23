import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { makeAnswerGuard, makeStopGuard, sendPick, stopButton } from "./answer";
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
    expect(send).toHaveBeenCalledWith({ sessionId: "s", askId: 500, questionIndex: 0, pick: { kind: "option", index: 1 } });
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

  it("sends any pick with the ask id, and locks the menu's rows and own box while it is in flight", async () => {
    document.body.innerHTML = '<div class="menu__rows"><div data-action="pick" data-row="2"><textarea></textarea><button data-action="own">Send</button></div></div>';
    let fail: (e: unknown) => void = () => {};
    const send = vi.fn(() => new Promise<void>((_, reject) => { fail = reject; }));
    const guard = makeAnswerGuard(send);
    const row = document.querySelector<HTMLElement>("[data-row='2']")!;
    const p = guard.pick(card, 0, { kind: "own", text: "Redis" }, row);
    expect(send).toHaveBeenCalledWith({ sessionId: "s", askId: 500, questionIndex: 0, pick: { kind: "own", text: "Redis" } });
    const rows = document.querySelector(".menu__rows")!;
    expect(rows.getAttribute("aria-disabled")).toBe("true");
    expect(document.querySelector("textarea")!.disabled).toBe(true);
    expect(document.querySelector("button")!.disabled).toBe(true);
    fail(new Error("Maya could not type into the terminal."));
    await expect(p).rejects.toThrow("Maya could not type into the terminal.");
    expect(rows.getAttribute("aria-disabled")).toBeNull();
    expect(document.querySelector("textarea")!.disabled).toBe(false);
  });
});

describe("sendPick", () => {
  it("sends an option pick through answer_question, so an older Maya still answers it, and the rest through answer_pick", async () => {
    invoke.mockReset();
    invoke.mockResolvedValue(null);
    await sendPick({ sessionId: "s", askId: 500, questionIndex: 1, pick: { kind: "option", index: 2 } });
    await sendPick({ sessionId: "s", askId: 500, questionIndex: 0, pick: { kind: "own", text: "Redis" } });
    await sendPick({ sessionId: "s", askId: 500, questionIndex: 0, pick: { kind: "chat" } });
    expect(invoke.mock.calls).toEqual([
      ["answer_question", { sessionId: "s", askId: 500, questionIndex: 1, optionIndex: 2 }],
      ["answer_pick", { sessionId: "s", askId: 500, questionIndex: 0, pick: { kind: "own", text: "Redis" } }],
      ["answer_pick", { sessionId: "s", askId: 500, questionIndex: 0, pick: { kind: "chat" } }],
    ]);
  });
});

describe("makeStopGuard", () => {
  const working: Card = { ...card, state: "working", awaiting: null, stateSince: 900 };

  it("a second Stop is ignored until the card changes", async () => {
    const send = vi.fn(() => Promise.resolve());
    const guard = makeStopGuard(send);
    const button = document.createElement("button");
    expect(await guard.stop(working, button)).toBe("sent");
    expect(button.disabled).toBe(true);
    expect(guard.stopping(working)).toBe(true);
    expect(await guard.stop(working, document.createElement("button"))).toBe("dropped");
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith("s");
    const moved = { ...working, state: "idle" as const, stateSince: 2000 };
    expect(guard.stopping(moved)).toBe(false);
    expect(await guard.stop({ ...moved, state: "working" }, button)).toBe("sent");
    expect(send).toHaveBeenCalledTimes(2);
  });

  it("disables the button at once, and gives it back when the stop fails", async () => {
    let fail: (e: unknown) => void = () => {};
    const guard = makeStopGuard(() => new Promise<void>((_, reject) => { fail = reject; }));
    const button = document.createElement("button");
    const p = guard.stop(working, button);
    expect(button.disabled).toBe(true);
    expect(await guard.stop(working, button)).toBe("dropped");
    fail(new Error("There is nothing to stop in this session."));
    await expect(p).rejects.toThrow("There is nothing to stop in this session.");
    expect(button.disabled).toBe(false);
    expect(guard.stopping(working)).toBe(false);
  });
});

describe("stopButton", () => {
  const working: Card = { ...card, sessionId: "w", state: "working", awaiting: null, stateSince: 10_000 };

  it("Stop shows only when canStop", () => {
    expect(stopButton(working, 20_000)?.textContent).toBe("Stop");
    expect(stopButton(working, 20_000)?.dataset.action).toBe("stop");
    expect(stopButton({ ...working, shell: true }, 20_000)).toBeNull();
    expect(stopButton({ ...working, state: "idle" }, 20_000)).toBeNull();
    expect(stopButton({ ...working, harness: "grok" }, 20_000)).toBeNull();
    expect(stopButton({ ...working, brain: true }, 20_000)).toBeNull();
    expect(stopButton({ ...card, awaiting: { kind: "text", detail: "Ok?", questions: [] } }, 20_000)).toBeNull();
    expect(stopButton(card, 20_000)).not.toBeNull();
  });

  it("is disabled inside the open delay on every device", () => {
    expect(stopButton(working, 10_999)!.disabled).toBe(true);
    expect(stopButton(working, 11_000)!.disabled).toBe(false);
  });
});

