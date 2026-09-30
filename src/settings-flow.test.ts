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
      if (cmd === "list_whisper_models") return Promise.resolve([]);
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
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    const onVoice = eventListen.mock.calls.find((c) => c[0] === "voice")![1] as (e: { payload: { listening: boolean; state?: string; detail?: string } }) => void;

    const typing = document.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    typing.value = "/Users/me/de";
    // Partials arrive several times a second while someone talks nearby.
    onVoice({ payload: { listening: true } });
    onVoice({ payload: { listening: true } });
    expect(document.querySelector("input[name=projectsDir]")).toBe(typing);
    expect(typing.value).toBe("/Users/me/de");

    // The listener giving up is shown next to the toggle, not swallowed.
    onVoice({ payload: { listening: false, state: "error", detail: "Dictation is off. Turn it on in System Settings." } });
    expect(document.querySelector<HTMLInputElement>("input[name=listen]")!.checked).toBe(false);
    expect(document.querySelector(".settings__error[data-for=listen]")?.textContent).toContain("Dictation is off");
    // Listening again clears it.
    onVoice({ payload: { listening: true, state: "idle", detail: "" } });
    expect(document.querySelector(".settings__error[data-for=listen]")).toBeNull();

    onVoice({ payload: { listening: false } });
    expect(document.querySelector<HTMLInputElement>("input[name=listen]")!.checked).toBe(false);
  });

  it("applies voice-model progress events to the picker, scoped to the model in flight", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku", recognizer: "builtin", whisperModel: "tiny.en" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([{ id: "tiny.en", label: "Tiny", bytes: 100, downloaded: false }]);
      // Never resolves in this test: we only need the download to be "in flight".
      if (cmd === "download_whisper_model") return new Promise(() => {});
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    // Start the download the way the UI does, so `model.downloading` is set
    // before any event arrives (mirrors clicking Download in the picker).
    document.querySelector<HTMLButtonElement>("button[data-action=download-model]")!.click();
    await flush();
    const onProgress = eventListen.mock.calls.find((c) => c[0] === "voice-model")![1] as (e: { payload: { id: string; received: number; total: number } }) => void;
    onProgress({ payload: { id: "tiny.en", received: 25, total: 100 } });
    expect(document.querySelector<HTMLProgressElement>("progress[name=modelDownload]")!.value).toBe(25);

    // (a) an event for a different id is ignored: the bar in flight doesn't move.
    onProgress({ payload: { id: "base.en-q5_1", received: 99, total: 100 } });
    expect(document.querySelector<HTMLProgressElement>("progress[name=modelDownload]")!.value).toBe(25);

    // (b) once the download in flight completes, the progress bar disappears.
    onProgress({ payload: { id: "tiny.en", received: 100, total: 100 } });
    expect(document.querySelector("progress[name=modelDownload]")).toBeNull();
  });

  it("repaints the Network section on a network event only when the status changed", async () => {
    const config = {
      completedTimeoutMinutes: 30,
      projectsDir: null,
      clonesDir: null,
      notifyOnAwaiting: true,
      speakNotifications: true,
      voiceProvider: "builtin",
      elevenlabsVoiceId: null,
      listen: false,
      microphone: null,
      interpreterModel: "haiku",
      network: { role: "assistant", port: 0, mainHost: "desk.local", mainPort: 4127, name: "laptop", assistantId: "id1", token: "t", assistants: [] },
    };
    const networkStatus = { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve({ ...networkStatus });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    expect(document.querySelector(".settings__status--network")?.textContent).toBe("Reconnecting…");

    const onNetwork = eventListen.mock.calls.find((c) => c[0] === "network")![1] as (e: {
      payload: { role: string; code: null; assistants: unknown[]; assistant: { connected: boolean; mainName: string | null; error: string | null } };
    }) => void;
    // The same status arrives again (e.g. an unrelated repaint upstream): no visible change.
    onNetwork({ payload: { ...networkStatus } });
    expect(document.querySelector(".settings__status--network")?.textContent).toBe("Reconnecting…");

    onNetwork({ payload: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "desk", error: null } } });
    expect(document.querySelector(".settings__status--network")?.textContent).toBe("Connected to desk");
  });

  it("keeps a half-typed pairing code across an unrelated repaint (a voice event)", async () => {
    const config = {
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
      network: { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", assistantId: "", token: "", assistants: [] },
    };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve({ role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;

    // Preview "Assistant" (unsaved) to reveal the pairing form.
    const roleSel = document.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    roleSel.value = "assistant";
    roleSel.dispatchEvent(new Event("change"));

    const code = document.querySelector<HTMLInputElement>("input[name=networkCode]")!;
    code.value = "48392";
    code.dispatchEvent(new Event("input"));

    // A `voice` event with `listening` changed repaints the whole panel.
    const onVoice = eventListen.mock.calls.find((c) => c[0] === "voice")![1] as (e: { payload: { listening: boolean } }) => void;
    onVoice({ payload: { listening: false } });

    expect(document.querySelector<HTMLInputElement>("input[name=networkCode]")!.value).toBe("48392");
  });

  it("Off → Assistant with stored credentials saves the role at once instead of asking to pair", async () => {
    let config: Record<string, unknown> = {
      completedTimeoutMinutes: 30,
      projectsDir: null,
      clonesDir: null,
      notifyOnAwaiting: true,
      speakNotifications: true,
      voiceProvider: "builtin",
      elevenlabsVoiceId: null,
      listen: false,
      microphone: null,
      interpreterModel: "haiku",
      network: { role: "off", port: 0, mainHost: "desk.local", mainPort: 4127, name: "laptop", assistantId: "id1", token: "t", assistants: [] },
    };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      if (cmd === "network_status") return Promise.resolve({ role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;

    const roleSel = document.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    roleSel.value = "assistant";
    roleSel.dispatchEvent(new Event("change"));
    await flush();
    await flush();
    const saved = invoke.mock.calls.find((c) => c[0] === "set_config");
    expect(saved, "the role is saved").toBeDefined();
    expect((saved![1] as { config: { network: { role: string; assistantId: string } } }).config.network).toMatchObject({ role: "assistant", assistantId: "id1" });
    expect(document.querySelector("input[name=networkCode]")).toBeNull();
    expect(document.querySelector("button[data-action=pair-again]")).not.toBeNull();
  });

  it("shows a failed Pair's message in the status line", async () => {
    const config = {
      completedTimeoutMinutes: 30,
      projectsDir: null,
      clonesDir: null,
      notifyOnAwaiting: true,
      speakNotifications: true,
      voiceProvider: "builtin",
      elevenlabsVoiceId: null,
      listen: false,
      microphone: null,
      interpreterModel: "haiku",
      network: { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", assistantId: "", token: "", assistants: [] },
    };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve({ role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "network_pair") return Promise.reject("Wrong or expired pairing code.");
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;

    const roleSel = document.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    roleSel.value = "assistant";
    roleSel.dispatchEvent(new Event("change"));
    // A never-paired machine has no status line until an attempt fails.
    expect(document.querySelector(".settings__status--network")).toBeNull();

    const host = document.querySelector<HTMLInputElement>("input[name=networkHost]")!;
    host.value = "desk.local";
    host.dispatchEvent(new Event("input"));
    const code = document.querySelector<HTMLInputElement>("input[name=networkCode]")!;
    code.value = "000000";
    code.dispatchEvent(new Event("input"));

    document.querySelector<HTMLButtonElement>("button[data-action=pair]")!.click();
    await flush();
    expect(document.querySelector(".settings__status--network")?.textContent).toBe("Wrong or expired pairing code.");
  });

  it("ignores a voice-model event for another id when nothing is downloading", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku", recognizer: "builtin", whisperModel: "tiny.en" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([{ id: "tiny.en", label: "Tiny", bytes: 100, downloaded: false }]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    const onProgress = eventListen.mock.calls.find((c) => c[0] === "voice-model")![1] as (e: { payload: { id: string; received: number; total: number } }) => void;
    onProgress({ payload: { id: "some-other-model", received: 10, total: 100 } });
    expect(document.querySelector("progress[name=modelDownload]")).toBeNull();
  });
});
