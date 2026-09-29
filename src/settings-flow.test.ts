import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const eventListen = vi.fn((..._args: unknown[]) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => eventListen(...args) }));

import { initSettings } from "./settings";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("settings flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="settings" class="settings pane" hidden></div>';
    invoke.mockReset();
    eventListen.mockClear();
  });

  it("saves against the freshest config rather than a stale local model", async () => {
    let config: Record<string, unknown> = {
      completedTimeoutMinutes: 30,
      projectsDir: null,
      clonesDir: null,
      notifyOnAwaiting: true,
      speakNotifications: true,
      voiceProvider: "builtin",
      elevenlabsVoiceId: null,
      listen: true,
      microphone: null,
      interpreterModel: "haiku",
    };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      return Promise.reject(new Error("unexpected " + cmd));
    });

    await initSettings();
    await flush();

    // The voice panel turns listening off on its own, independently of Settings.
    config = { ...config, listen: false };

    const input = document.querySelector<HTMLInputElement>("input[name=timeout]")!;
    input.value = "45";
    input.dispatchEvent(new Event("change"));
    await flush();
    await flush();

    expect(invoke).toHaveBeenCalledWith("set_config", { config: expect.objectContaining({ completedTimeoutMinutes: 45, listen: false }) });
    expect(document.querySelector<HTMLInputElement>("input[name=timeout]")!.value).toBe("45");
  });

  it("repaints on a voice event only when listening changes, so typing survives", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: true, microphone: null, interpreterModel: "haiku" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    const onVoice = eventListen.mock.calls.find((c) => c[0] === "voice")![1] as (e: { payload: { listening: boolean } }) => void;

    const typing = document.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    typing.value = "/Users/me/de";
    // Partials arrive several times a second while someone talks nearby.
    onVoice({ payload: { listening: true } });
    onVoice({ payload: { listening: true } });
    expect(document.querySelector("input[name=projectsDir]")).toBe(typing);
    expect(typing.value).toBe("/Users/me/de");

    onVoice({ payload: { listening: false } });
    expect(document.querySelector<HTMLInputElement>("input[name=listen]")!.checked).toBe(false);
  });
});
