import { beforeEach, describe, expect, it, vi } from "vitest";
import type { BrainStatus, ChatTurn } from "./chat";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const eventListen = vi.fn((..._args: unknown[]) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => eventListen(...args) }));

import { initChat } from "./chat";
import { makeTabs } from "./tabs";

const flush = () => new Promise((r) => setTimeout(r, 0));
const status = (over: Partial<BrainStatus> = {}): BrainStatus => ({ dot: "idle", pending: null, job: null, ready: true, ...over });
const on = <T,>(name: string) => eventListen.mock.calls.find((c) => c[0] === name)![1] as (e: { payload: T }) => void;

describe("chat flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<nav><button data-tab="sessions">S</button><button data-tab="chat">Chat <span class="tab__dot"></span></button></nav><div id="board"></div><div id="chat" hidden></div>';
    localStorage.clear();
    invoke.mockReset();
    eventListen.mockClear();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "chat_history") return Promise.resolve([]);
      if (cmd === "brain_status") return Promise.resolve(status());
      return Promise.resolve(undefined);
    });
  });

  it("colours the tab dot by status and marks the chat seen when opened", async () => {
    const tabs = makeTabs();
    initChat();
    await flush();
    on<BrainStatus>("brain")({ payload: status({ dot: "unread" }) });
    expect(document.querySelector(".tab__dot")!.className).toContain("tab__dot--unread");
    expect(invoke).not.toHaveBeenCalledWith("chat_seen");
    tabs.show("chat");
    await flush();
    expect(invoke).toHaveBeenCalledWith("chat_seen");
  });

  it("shows pushed turns, sends typed text and wires Cancel and Stop to their commands", async () => {
    initChat();
    await flush();
    const turn: ChatTurn = { who: "maya", text: "Nothing waits.", at: 2, via: "chat" };
    on<ChatTurn>("chat")({ payload: turn });
    on<BrainStatus>("brain")({ payload: status({ dot: "working", job: "j", pending: { say: "Focusing x.", until: Date.now() + 5000 } }) });
    expect(document.querySelector(".chat__turn")!.textContent).toBe("Nothing waits.");
    document.querySelector<HTMLButtonElement>("button[data-action=window-cancel]")!.click();
    document.querySelector<HTMLButtonElement>("button[data-action=brain-stop]")!.click();
    expect(invoke).toHaveBeenCalledWith("window_cancel");
    expect(invoke).toHaveBeenCalledWith("brain_stop");
    const box = document.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
    box.value = "what's waiting?";
    box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(invoke).toHaveBeenCalledWith("chat_send", { text: "what's waiting?" });
  });

  it("ticks the countdown in place, leaving the input and its caret alone", async () => {
    initChat();
    await flush();
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    try {
      on<BrainStatus>("brain")({ payload: status({ pending: { say: "Telling api-fix: open the PR.", until: Date.now() + 5000 } }) });
      const box = document.querySelector<HTMLTextAreaElement>("textarea[name=chat]")!;
      const pending = document.querySelector(".chat__pending")!;
      box.value = "no, the oth";
      box.setSelectionRange(3, 3);
      expect(document.querySelector(".chat__pending-left")!.textContent).toBe("5s");
      vi.advanceTimersByTime(1000);
      expect(document.querySelector(".chat__pending-left")!.textContent).toBe("4s");
      expect(document.querySelector("textarea[name=chat]")).toBe(box);
      expect(document.querySelector(".chat__pending")).toBe(pending);
      expect(box.selectionStart).toBe(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("follows the role at runtime: read-only while dormant, live again, the tab always there", async () => {
    let current = status({ dormant: "Dormant: desk's Maya is awake.", ready: false });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "chat_history") return Promise.resolve([{ who: "user", text: "hi", at: 1, via: "chat" }]);
      if (cmd === "brain_status") return Promise.resolve(current);
      return Promise.resolve(undefined);
    });
    makeTabs();
    initChat();
    await flush();
    const tab = document.querySelector<HTMLElement>("[data-tab=chat]")!;
    expect(tab.hidden).toBe(false);
    expect(document.querySelector(".chat__dormant")!.textContent).toBe("Dormant: desk's Maya is awake.");
    expect(document.querySelector("textarea[name=chat]")).toBeNull();
    expect(document.querySelector(".chat__turn")!.textContent).toBe("hi");
    // Made main: the next status (re-read when the network changes) is live.
    current = status();
    on<unknown>("network")({ payload: {} });
    await flush();
    expect(document.querySelector(".chat__dormant")).toBeNull();
    expect(document.querySelector("textarea[name=chat]")).not.toBeNull();
    // Settings made it an assistant again: re-read on config-changed.
    current = status({ dormant: "Dormant: the main Maya is awake." });
    window.dispatchEvent(new Event("maya:config-changed"));
    await flush();
    expect(document.querySelector(".chat__dormant")!.textContent).toBe("Dormant: the main Maya is awake.");
    expect(document.querySelector("textarea[name=chat]")).toBeNull();
    expect(tab.hidden).toBe(false);
  });
});
