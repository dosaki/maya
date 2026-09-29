import { beforeEach, describe, expect, it, vi } from "vitest";
import type { VoiceStatus } from "./voice";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const eventListen = vi.fn((..._args: unknown[]) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => eventListen(...args) }));

import { initVoice } from "./voice";

const flush = () => new Promise((r) => setTimeout(r, 0));
const status = (over: Partial<VoiceStatus> = {}): VoiceStatus => ({ listening: true, state: "idle", detail: "", level: 0, heard: "", said: "", pending: null, turns: 0, ...over });

describe("voice flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<span id="voice-host"></span><div id="voice-panel" hidden></div>';
    invoke.mockReset();
    eventListen.mockClear();
  });

  it("refetches the history only when a turn is added, not on every partial", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "voice_status") return Promise.resolve(status());
      if (cmd === "voice_history") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initVoice();
    const onVoice = eventListen.mock.calls.find((c) => c[0] === "voice")![1] as (e: { payload: VoiceStatus }) => void;
    const fetches = () => invoke.mock.calls.filter((c) => c[0] === "voice_history").length;

    onVoice({ payload: status({ heard: "so I" }) });
    onVoice({ payload: status({ heard: "so I told" }) });
    onVoice({ payload: status({ heard: "so I told her" }) });
    await flush();
    expect(fetches()).toBe(0);

    onVoice({ payload: status({ heard: "Maya what's waiting", state: "thinking", turns: 1 }) });
    await flush();
    expect(fetches()).toBe(1);
    onVoice({ payload: status({ said: "Nothing is waiting.", turns: 2 }) });
    await flush();
    expect(fetches()).toBe(2);
  });

  it("says that a click opens the panel when listening is off", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "voice_status" ? Promise.resolve(status({ listening: false, state: "off" })) : Promise.resolve([])));
    await initVoice();
    const button = document.querySelector<HTMLButtonElement>("#voice-host button")!;
    expect(button.title).toContain("Click to open");
    expect(button.title).not.toContain("Click to listen");
  });
});
