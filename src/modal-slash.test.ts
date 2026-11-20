import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { closeModal, openModal, refreshModal, renderModal } from "./modal";
import { commandsKey, rememberCommands } from "./slash-cache";
import type { Card, CommandInfo } from "./types";

const flush = () => new Promise((r) => setTimeout(r, 0));
const cmd = (name: string): CommandInfo => ({ name, description: `${name} it`, source: "builtin" });
const card: Card = { sessionId: "s1", pid: 1, name: "eye", cwd: "/x", state: "idle", stateSince: 0, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null, context: null };
const other: Card = { ...card, sessionId: "s2", cwd: "/y" };

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

function answer(map: Record<string, unknown>) {
  invoke.mockImplementation((cmdName: string) => {
    if (cmdName in map) {
      const v = map[cmdName];
      return typeof v === "function" ? (v as () => unknown)() : Promise.resolve(v);
    }
    return cmdName === "session_history" ? Promise.resolve([]) : Promise.reject(new Error(cmdName));
  });
}

const box = () => document.querySelector<HTMLTextAreaElement>(".modal__input")!;
const popup = () => document.querySelector<HTMLElement>(".modal__composer .slash");
const type = (text: string) => {
  box().focus();
  box().value = text;
  box().setSelectionRange(text.length, text.length);
  box().dispatchEvent(new Event("input", { bubbles: true }));
};

describe("the slash list in a session window", () => {
  beforeEach(() => {
    closeModal();
    document.body.innerHTML = '<div id="modal-host"></div>';
    localStorage.clear();
    invoke.mockReset();
  });

  it("shows the cached list, then the session's own list", async () => {
    rememberCommands(commandsKey(card), [cmd("cached")]);
    const own = deferred<CommandInfo[]>();
    answer({ session_commands: () => own.promise, list_commands: [{ harness: "claude-code", commands: [cmd("shared")] }] });
    await openModal(card);
    type("/");
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/cached"]);
    own.resolve([cmd("mine")]);
    await flush();
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/mine"]);
    expect(invoke).not.toHaveBeenCalledWith("list_commands", expect.anything());
    // Remembered for the next window in this folder.
    closeModal();
    answer({ session_commands: () => new Promise(() => undefined) });
    await openModal(card);
    type("/");
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/mine"]);
  });

  it("falls back to the machine's shared list when nothing is cached", async () => {
    const own = deferred<CommandInfo[]>();
    answer({ session_commands: () => own.promise, list_commands: [{ harness: "codex", commands: [cmd("other")] }, { harness: "claude-code", commands: [cmd("shared")] }] });
    await openModal({ ...card, machine: "box" });
    await flush();
    expect(invoke).toHaveBeenCalledWith("list_commands", { machine: "box" });
    type("/");
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/shared"]);
    own.resolve([cmd("mine")]);
    await flush();
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/mine"]);
  });

  it("the shared list does not replace the session's own list that came first", async () => {
    const shared = deferred<unknown>();
    answer({ session_commands: [cmd("mine")], list_commands: () => shared.promise });
    await openModal(card);
    await flush();
    shared.resolve([{ harness: "claude-code", commands: [cmd("shared")] }]);
    await flush();
    type("/");
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/mine"]);
  });

  it("a list that lands after the window closed changes nothing on screen", async () => {
    const late = deferred<CommandInfo[]>();
    answer({ session_commands: () => late.promise, list_commands: [] });
    await openModal(card);
    closeModal();
    answer({ session_commands: [cmd("second")], list_commands: [] });
    await openModal(other);
    await flush();
    late.resolve([cmd("stale")]);
    await flush();
    type("/");
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/second"]);
    // It is still the right list for its own folder.
    closeModal();
    answer({ session_commands: () => new Promise(() => undefined) });
    await openModal(card);
    type("/");
    expect([...document.querySelectorAll(".slash__name")].map((n) => n.textContent)).toEqual(["/stale"]);
  });

  it("Escape with the list open keeps the window open", async () => {
    answer({ session_commands: [cmd("mine")], list_commands: [] });
    await openModal(card);
    await flush();
    type("/");
    expect(popup()!.hidden).toBe(false);
    box().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(popup()!.hidden).toBe(true);
    expect(document.querySelector(".modal")).not.toBeNull();
    box().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(document.querySelector(".modal")).toBeNull();
  });

  it("the list survives a repaint of the window", async () => {
    answer({ session_commands: [cmd("alpha"), cmd("beta")], list_commands: [] });
    await openModal(card);
    await flush();
    type("/");
    box().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    const before = popup()!;
    expect(before.querySelector("[aria-selected=true]")!.textContent).toContain("/beta");
    refreshModal([{ ...card, state: "working", stateSince: 9 }]);
    expect(popup()).toBe(before);
    expect(popup()!.hidden).toBe(false);
    expect(popup()!.querySelector("[aria-selected=true]")!.textContent).toContain("/beta");
    // The kept popup still answers keys after the repaint.
    box().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true }));
    expect(popup()!.querySelector("[aria-selected=true]")!.textContent).toContain("/alpha");
  });

  it("closes after a command is sent, over the emptied box", async () => {
    answer({ session_commands: [cmd("compact")], list_commands: [], send_slash_command: undefined });
    await openModal(card);
    await flush();
    type("/compact");
    expect(popup()!.hidden).toBe(false);
    document.querySelector<HTMLButtonElement>("[data-action=send]")!.click();
    await flush();
    await flush();
    expect(box().value).toBe("");
    expect(popup()!.hidden).toBe(true);
  });

  it("no list for an agent without slash lines, nor while the question menu shows", () => {
    const h = {} as never;
    const plain = renderModal({ card: { ...card, harness: "other" }, turns: [], status: null, draft: "" }, h);
    expect(plain.querySelector(".slash")).toBeNull();
    const asking: Card = { ...card, state: "awaiting", stateSince: 5000, awaiting: { kind: "question", detail: "Q?", questions: [{ question: "Q?", header: "H", multiSelect: false, options: [{ label: "A", description: "a" }], own: { label: "Type something", row: 1 }, chatRow: 2 }] } };
    const ask = renderModal({ card: asking, turns: [], status: null, draft: "", commands: [cmd("x")] }, h);
    expect(ask.querySelector(".slash")).toBeNull();
    const ok = renderModal({ card, turns: [], status: null, draft: "", commands: [cmd("x")] }, h);
    expect(ok.querySelector(".modal__composer .slash")).not.toBeNull();
  });
});
