import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const eventListen = vi.fn((..._args: unknown[]) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => eventListen(...args) }));

import { initSettings } from "./settings";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("settings flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<aside id="settings" class="settings" hidden></aside><button id="settings-toggle"></button>';
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
});
