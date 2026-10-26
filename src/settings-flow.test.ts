import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

const eventListen = vi.fn((..._args: unknown[]) => Promise.resolve(() => {}));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => eventListen(...args) }));

const getVersion = vi.fn(() => Promise.resolve("0.13.0"));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: () => getVersion() }));

import { defaultKeys, keyBindings, setKeyBindings } from "./keys";
import { initSettings } from "./settings";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("settings flow", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="settings" class="settings pane" hidden></div>';
    invoke.mockReset();
    eventListen.mockClear();
  });

  it("shows the app version Tauri reports under Maya, and nothing when it cannot", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });

    await initSettings();
    await flush();
    expect(document.querySelector(".settings__section--general .settings__version")?.textContent).toBe("Maya 0.13.0");

    getVersion.mockImplementationOnce(() => Promise.reject(new Error("no tauri")));
    document.body.innerHTML = '<div id="settings" class="settings pane" hidden></div>';
    await initSettings();
    await flush();
    expect(document.querySelector("input[name=timeout]")).not.toBeNull();
    expect(document.querySelector(".settings__version")).toBeNull();
  });

  it("themes the page from the saved choice, and applies and saves a new one", async () => {
    localStorage.clear();
    delete document.documentElement.dataset.theme;
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "", colorTheme: "light" };
    let saved: { colorTheme?: string } | undefined;
    invoke.mockImplementation((cmd: string, args?: { config?: { colorTheme?: string } }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        saved = args?.config;
        return Promise.resolve({ ...config, ...args?.config });
      }
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });

    await initSettings();
    await flush();
    expect(document.documentElement.dataset.theme).toBe("light");
    const select = document.querySelector<HTMLSelectElement>("select[name=colorTheme]")!;
    expect(select.value).toBe("light");

    select.value = "dark";
    select.dispatchEvent(new Event("change"));
    expect(document.documentElement.dataset.theme).toBe("dark");
    await flush();
    expect(saved?.colorTheme).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("follows the saved message keys, and applies and saves new ones", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "", keys: { send: ["Meta+Enter"], newLine: ["Enter"] } };
    let saved: { keys?: unknown } | undefined;
    invoke.mockImplementation((cmd: string, args?: { config?: { keys?: unknown } }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        saved = args?.config;
        return Promise.resolve({ ...config, ...args?.config });
      }
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    try {
      await initSettings();
      await flush();
      expect(keyBindings()).toEqual({ send: ["Meta+Enter"], newLine: ["Enter"] });

      document.querySelector<HTMLButtonElement>("button[data-action=keys-reset]")!.click();
      expect(keyBindings()).toEqual(defaultKeys());
      await flush();
      expect(saved?.keys).toEqual(defaultKeys());
    } finally {
      setKeyBindings(defaultKeys());
    }
  });

  it("an old ElevenLabs config opens with ElevenLabs switched on", async () => {
    const open = async (voiceProvider: string) => {
      const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider, elevenlabsVoiceId: "abc", listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
      invoke.mockImplementation((cmd: string) => {
        if (cmd === "codex_hook_status") return Promise.resolve(false);
        if (cmd === "hook_status") return Promise.resolve(true);
        if (cmd === "get_config") return Promise.resolve({ ...config });
        if (cmd === "has_elevenlabs_key") return Promise.resolve(false);
        if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
        if (cmd === "list_whisper_models") return Promise.resolve([]);
        if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
        return Promise.reject(new Error("unexpected " + cmd));
      });
      document.body.innerHTML = '<div id="settings" class="settings pane" hidden></div>';
      await initSettings();
      await flush();
      return document.querySelector<HTMLInputElement>("input[name=elevenEnabled]")!.checked;
    };
    expect(await open("elevenlabs")).toBe(true);
    expect(await open("builtin")).toBe(false);
  });

  describe("voice chain loading", () => {
    const open = async (voices: Record<string, () => Promise<unknown>>) => {
      const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, elevenlabsEnabled: true, elevenlabsVoiceId: "abc", voiceServer: { enabled: true, url: "http://mac.local:8004", voice: "", model: "" }, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
      invoke.mockImplementation((cmd: string) => {
        if (cmd === "codex_hook_status") return Promise.resolve(false);
        if (cmd === "hook_status") return Promise.resolve(true);
        if (cmd === "get_config") return Promise.resolve({ ...config });
        if (cmd === "has_elevenlabs_key" || cmd === "has_voice_server_key") return Promise.resolve(true);
        if (cmd in voices) return voices[cmd]();
        if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
        if (cmd === "list_whisper_models") return Promise.resolve([]);
        if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
        return Promise.reject(new Error("unexpected " + cmd));
      });
      document.body.innerHTML = '<div id="settings" class="settings pane" hidden></div>';
      await initSettings();
      await flush();
    };

    it("shows a server address problem next to the voice, not as a page error", async () => {
      const text = "Not a server address: start it with http:// or https://.";
      await open({ list_elevenlabs_voices: () => Promise.resolve([]), list_server_voices: () => Promise.reject(text) });
      expect(document.querySelector("[data-for=server-voices]")?.textContent).toBe(`Could not list the server's voices: ${text}`);
      expect(document.querySelector(".settings__error")?.textContent ?? "").toBe("");
    });

    it("loads the server's voices even when ElevenLabs fails", async () => {
      await open({ list_elevenlabs_voices: () => Promise.reject("401"), list_server_voices: () => Promise.resolve(["default", "tiago.wav"]) });
      expect([...document.querySelectorAll<HTMLOptionElement>("select[name=serverVoice] option")].map((o) => o.value)).toEqual(["default", "tiago.wav"]);
      expect(document.querySelector<HTMLSelectElement>("select[name=serverVoice]")!.value).toBe("default");
    });

    it("labels the blank voice 'server default' when the list has no default", async () => {
      await open({ list_elevenlabs_voices: () => Promise.resolve([]), list_server_voices: () => Promise.resolve(["tiago.wav"]) });
      const options = [...document.querySelectorAll<HTMLOptionElement>("select[name=serverVoice] option")];
      expect(options.map((o) => [o.value, o.textContent])).toEqual([["", "server default"], ["tiago.wav", "tiago.wav"]]);
    });

    it("clears the try-voice report when the settings change", async () => {
      await open({ list_elevenlabs_voices: () => Promise.resolve([]), list_server_voices: () => Promise.resolve(["default"]), try_voice: () => Promise.resolve("Spoke with the server."), set_config: () => Promise.resolve({ completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, elevenlabsEnabled: true, elevenlabsVoiceId: "abc", voiceServer: { enabled: true, url: "http://mac.local:8004", voice: "default", model: "" }, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" }) });
      const select = () => document.querySelector<HTMLSelectElement>("select[name=serverVoice]")!;
      const tryButton = [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => /try/i.test(b.textContent ?? ""))!;
      tryButton.click();
      await flush();
      expect(document.body.textContent).toContain("Spoke with the server.");
      select().value = "default";
      select().dispatchEvent(new Event("change"));
      await flush();
      expect(document.body.textContent).not.toContain("Spoke with the server.");
    });
  });

  it("opens on the group chosen last time, and keeps it through a repaint", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    const visible = () => [...document.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group);

    localStorage.setItem("maya.settingsGroup", "voice");
    await initSettings();
    await flush();
    expect(visible()).toEqual(["voice"]);

    document.querySelector<HTMLButtonElement>(".settings__nav button[data-group=sessions]")!.click();
    document.querySelector<HTMLInputElement>("input[name=timeout]")!.dispatchEvent(new Event("change"));
    await flush();
    expect(visible()).toEqual(["sessions"]);
    localStorage.removeItem("maya.settingsGroup");
  });

  it("switches group on the press, so the save a blur starts cannot repaint the click away", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") return Promise.resolve({ ...config, ...args?.config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    const visible = () => [...document.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group);

    localStorage.setItem("maya.settingsGroup", "sessions");
    await initSettings();
    await flush();
    const dir = document.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    dir.value = "~/code";
    const pressed = document.querySelector<HTMLButtonElement>(".settings__nav button[data-group=voice]")!;
    pressed.dispatchEvent(new MouseEvent("pointerdown", { button: 0, bubbles: true }));
    // The blur's save lands and repaints before the button is let go.
    dir.dispatchEvent(new Event("change"));
    await flush();
    expect(pressed.isConnected).toBe(false);
    expect(visible()).toEqual(["voice"]);
    expect(document.querySelector(".settings__nav button[aria-current=true]")?.textContent).toBe("Voice");
    localStorage.removeItem("maya.settingsGroup");
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
      agent: "claude-code",
      agentModel: "",
      reviewPrompt: "",
    };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
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

  it("changing the agent clears its model and saves both", async () => {
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
      agent: "claude-code",
      agentModel: "",
      reviewPrompt: "",
    };
    const agents = [
      { harness: "claude-code" as const, models: [{ id: "opus", label: "Opus", efforts: [] }], efforts: [], modes: [] },
      { harness: "grok" as const, models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: [] },
    ];
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents, names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });

    // Settings starts hidden (the app opens on Sessions), and the agent
    // listing can take seconds, so it typically lands while the tab is
    // still hidden: the Agent select must pick it up once the tab is shown
    // rather than being stuck on Claude Code alone.
    await initSettings();
    await flush();
    await flush();
    document.getElementById("settings")!.hidden = false;
    await flush();
    const agent = document.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...agent.options].map((o) => o.value)).toEqual(["claude-code", "grok"]);
    agent.value = "grok";
    agent.dispatchEvent(new Event("change"));
    await flush();
    await flush();
    expect(invoke).toHaveBeenCalledWith("set_config", { config: expect.objectContaining({ agent: "grok", agentModel: "" }) });
  });

  it("picks up the agent the first-start modal saved once Settings is shown, and saves models with it", async () => {
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
      agent: null,
      agentModel: "",
      reviewPrompt: "",
    };
    const agents = [
      { harness: "claude-code" as const, models: [{ id: "opus", label: "Opus", efforts: [] }], efforts: [], modes: [] },
      { harness: "codex" as const, models: [{ id: "gpt-5.5", label: "gpt-5.5", efforts: [] }], efforts: [], modes: [] },
    ];
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") {
        config = { ...args!.config };
        return Promise.resolve({ ...config });
      }
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents, names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });

    await initSettings();
    await flush();
    // The first-start modal saves the agent through its own set_config.
    config = { ...config, agent: "codex" };
    document.getElementById("settings")!.hidden = false;
    await flush();
    const agent = document.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect(agent.value).toBe("codex");
    const model = document.querySelector<HTMLSelectElement>("select[name=agentModel]")!;
    model.value = "gpt-5.5";
    model.dispatchEvent(new Event("change"));
    await flush();
    await flush();
    expect(invoke).toHaveBeenCalledWith("set_config", { config: expect.objectContaining({ agent: "codex", agentModel: "gpt-5.5" }) });
  });

  it("follows a first-start agent choice while Settings is already the visible tab", async () => {
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
      agent: null,
      agentModel: "",
      reviewPrompt: "",
    };
    const agents = [
      { harness: "claude-code" as const, models: [{ id: "opus", label: "Opus", efforts: [] }], efforts: [], modes: [] },
      { harness: "codex" as const, models: [{ id: "gpt-5.5", label: "gpt-5.5", efforts: [] }], efforts: [], modes: [] },
    ];
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents, names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });

    // The remembered tab was Settings: it is visible from the start, so
    // `hidden` never toggles after the first-start modal saves.
    document.getElementById("settings")!.hidden = false;
    await initSettings();
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=agent]")!.value).toBe("claude-code");
    config = { ...config, agent: "codex" };
    window.dispatchEvent(new CustomEvent("maya:config-changed"));
    await flush();
    expect(document.querySelector<HTMLSelectElement>("select[name=agent]")!.value).toBe("codex");
  });

  it("loads the rest of Settings when the Codex hook status cannot be read", async () => {
    const config = { completedTimeoutMinutes: 42, projectsDir: "/p", clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.reject("hooks.json is not valid JSON, refusing to modify it");
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    expect(document.querySelector<HTMLInputElement>("input[name=timeout]")!.value).toBe("42");
    expect(document.querySelector<HTMLInputElement>("input[name=projectsDir]")!.value).toBe("/p");
    expect(document.querySelector("button[data-action=install-codex]")?.textContent).toBe("Install Codex hook");
    expect(document.body.textContent).toContain("Remove Claude hook");
  });

  it("repaints on a voice event only when listening changes, so typing survives", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: true, microphone: null, interpreterModel: "haiku" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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
      network: { role: "assistant", port: 0, mainHost: "desk.local", mainPort: 4127, name: "laptop", mainKey: "k1", assistants: [] },
    };
    const networkStatus = { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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
      network: { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [] },
    };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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
      network: { role: "off", port: 0, mainHost: "desk.local", mainPort: 4127, name: "laptop", mainKey: "k1", assistants: [] },
    };
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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
    expect((saved![1] as { config: { network: { role: string; mainKey: string } } }).config.network).toMatchObject({ role: "assistant", mainKey: "k1" });
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
      network: { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [] },
    };
    invoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve({ role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "network_pair") return Promise.reject("Wrong or expired pairing code.");
      if (cmd === "set_config") return Promise.resolve({ ...(args as { config: object }).config });
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

    // Leaving Assistant drops the failed attempt's message; coming back starts clean.
    const role = () => document.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    role().value = "off";
    role().dispatchEvent(new Event("change"));
    await flush();
    await flush();
    role().value = "assistant";
    role().dispatchEvent(new Event("change"));
    expect(document.querySelector(".settings__status--network")).toBeNull();
  });

  it("leaves the listen box alone while Assistant is only previewed before pairing", async () => {
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
      network: { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [] },
    };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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
    expect(document.querySelector<HTMLInputElement>("input[name=listen]")!.checked).toBe(true);

    const roleSel = document.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    roleSel.value = "assistant";
    roleSel.dispatchEvent(new Event("change"));
    await flush();
    expect(document.querySelector("input[name=networkCode]"), "the pairing form is previewed").not.toBeNull();
    const listen = document.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(listen.checked).toBe(true);
    expect(listen.disabled).toBe(false);
    expect(document.querySelector(".settings__hint[data-for=listen]")).toBeNull();
    expect(invoke.mock.calls.some((c) => c[0] === "set_config")).toBe(false);
  });

  it("sends one Pair while one is in flight, with the button disabled until it ends", async () => {
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
      network: { role: "off", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [] },
    };
    let finishPair: (v: unknown) => void = () => {};
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve({ role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "network_pair") return new Promise((_, reject) => (finishPair = reject));
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    const roleSel = document.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    roleSel.value = "assistant";
    roleSel.dispatchEvent(new Event("change"));
    const code = document.querySelector<HTMLInputElement>("input[name=networkCode]")!;
    code.value = "483921";
    code.dispatchEvent(new Event("input"));

    const pair = () => document.querySelector<HTMLButtonElement>("button[data-action=pair]")!;
    const first = pair();
    first.click();
    // The same element clicked again (a double click before the repaint), then the repainted one.
    first.click();
    await flush();
    expect(pair().disabled).toBe(true);
    pair().click();
    await flush();
    expect(invoke.mock.calls.filter((c) => c[0] === "network_pair")).toHaveLength(1);

    finishPair("Wrong or expired pairing code.");
    await flush();
    expect(pair().disabled).toBe(false);
    pair().click();
    await flush();
    expect(invoke.mock.calls.filter((c) => c[0] === "network_pair")).toHaveLength(2);
  });

  it("repaints a hidden Settings tab when it is shown after a network event", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku", recognizer: "system", whisperModel: "base.en-q5_1", network: { role: "main", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [{ id: "a1", name: "Gnowee", hostname: "TKC-0176", platform: "macos", token: "t" }] } };
    const before = { role: "main", code: null, assistants: [{ id: "a1", name: "Gnowee", hostname: "TKC-0176", platform: "macos", connected: false, lastSeen: null, note: null }], assistant: { connected: false, mainName: null, error: null }, mainError: null };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve(before);
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    const panel = document.getElementById("settings")!;
    expect(panel.hidden).toBe(true);
    // The assistant connects while the Sessions tab is showing.
    const onNetwork = eventListen.mock.calls.find((c) => c[0] === "network")![1] as (e: { payload: unknown }) => void;
    onNetwork({ payload: { ...before, assistants: [{ ...before.assistants[0], connected: true, lastSeen: 1 }] } });
    panel.hidden = false;
    await flush();
    expect(panel.textContent).toContain("Gnowee (macos) — Connected");
    expect(panel.textContent).not.toContain("Never connected");
  });

  it("keeps focus and the caret in a field through a repaint", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: "~/dev/maya", clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku", recognizer: "system", whisperModel: "base.en-q5_1", network: { role: "main", port: 0, mainHost: "", mainPort: 0, name: "", mainKey: "", assistants: [{ id: "a1", name: "Gnowee", hostname: "TKC-0176", platform: "macos", token: "t" }] } };
    const before = { role: "main", code: null, assistants: [{ id: "a1", name: "Gnowee", hostname: "TKC-0176", platform: "macos", address: "192.168.55.70", connected: false, lastSeen: null, note: null }], assistant: { connected: false, mainName: null, error: null }, mainError: null };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve(before);
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    const typing = document.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    typing.focus();
    typing.setSelectionRange(3, 6);
    // The assistant connects: the Network section repaints the whole pane.
    const onNetwork = eventListen.mock.calls.find((c) => c[0] === "network")![1] as (e: { payload: unknown }) => void;
    onNetwork({ payload: { ...before, assistants: [{ ...before.assistants[0], connected: true, lastSeen: 1 }] } });
    const repainted = document.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    expect(repainted).not.toBe(typing);
    expect(document.activeElement).toBe(repainted);
    expect([repainted.selectionStart, repainted.selectionEnd]).toEqual([3, 6]);
    // A focused checkbox keeps its focus too (it has no caret).
    document.querySelector<HTMLInputElement>("input[name=notify]")!.focus();
    onNetwork({ payload: before });
    expect(document.activeElement).toBe(document.querySelector("input[name=notify]"));
  });

  it("ignores a voice-model event for another id when nothing is downloading", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, interpreterModel: "haiku", recognizer: "builtin", whisperModel: "tiny.en" };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
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

  it("Check for updates reports up to date, a newer version, or the error", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
    let check: () => Promise<unknown> = () => Promise.resolve(null);
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      if (cmd === "update_kind") return Promise.resolve("self");
      if (cmd === "update_check") return check();
      return Promise.reject(new Error("unexpected " + cmd));
    });
    document.body.innerHTML = '<div id="update-bar" hidden></div><div id="settings" class="settings pane"></div>';
    const { updates } = await import("./update");
    await updates.init();
    await initSettings();
    await flush();
    const press = () => document.querySelector<HTMLButtonElement>("[data-action=check-updates]")!.click();
    const status = () => document.querySelector(".settings__update-status")?.textContent;

    press();
    await flush();
    expect(status()).toBe("Up to date.");

    check = () => Promise.resolve({ version: "0.19.0", download: "https://github.com/dosaki/maya/releases/tag/v0.19.0" });
    press();
    await flush();
    expect(status()).toBe("Maya 0.19.0 is ready: see the bar above.");

    check = () => Promise.reject("could not check for updates: offline");
    press();
    await flush();
    expect(status()).toBe("could not check for updates: offline");
  });

  it("opens on Git when the Pull requests tab asks while Settings is still loading", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "", git: null };
    let release: () => void = () => {};
    const late = new Promise<void>((r) => (release = r));
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return late.then(() => ({ ...config }));
      if (cmd === "get_git_status") return Promise.resolve({ github: { hasToken: false }, gitlab: { hasToken: false } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    const init = initSettings();
    document.dispatchEvent(new CustomEvent("maya:settings-group", { detail: "git" }));
    release();
    await init;
    await flush();
    const visible = [...document.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group);
    expect(visible).toEqual(["git"]);
  });

  it("saves a GitLab URL into the git list and opens on Git when the Pull requests tab asks", async () => {
    let saved: any = null;
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "", git: null };
    invoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "set_config") { saved = args.config; return Promise.resolve(args.config); }
      if (cmd === "get_git_status") return Promise.resolve({ github: { hasToken: false }, gitlab: { hasToken: false } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.dispatchEvent(new CustomEvent("maya:settings-group", { detail: "git" }));
    await flush();
    const visible = [...document.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group);
    expect(visible).toEqual(["git"]);
    const enable = document.querySelector<HTMLInputElement>(".settings__git[data-provider=gitlab] input[name^=gitEnabled]")!;
    enable.checked = true;
    enable.dispatchEvent(new Event("change"));
    await flush();
    expect(saved.git).toEqual([
      { provider: "github", enabled: true, baseUrl: null, token: { kind: "env", name: "GH_TOKEN" }, clone: "ssh" },
      { provider: "gitlab", enabled: true, baseUrl: null, token: { kind: "env", name: "GITLAB_TOKEN" }, clone: "ssh" },
    ]);
    localStorage.removeItem("maya.settingsGroup");
  });

  it("keeps focus inside the same provider block through a repaint", async () => {
    const both = [
      { provider: "github", enabled: true, baseUrl: null, token: { kind: "env", name: "GH_TOKEN" } },
      { provider: "gitlab", enabled: true, baseUrl: null, token: { kind: "env", name: "GITLAB_TOKEN" } },
    ];
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "", git: both };
    const net = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "network_status") return Promise.resolve(net);
      if (cmd === "get_git_status") return Promise.resolve({ github: { hasToken: false }, gitlab: { hasToken: false } });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      return Promise.reject(new Error("unexpected " + cmd));
    });
    await initSettings();
    await flush();
    document.getElementById("settings")!.hidden = false;
    document.dispatchEvent(new CustomEvent("maya:settings-group", { detail: "git" }));
    await flush();
    const gl = () => document.querySelector<HTMLElement>(".settings__git[data-provider=gitlab]")!;
    const onNetwork = eventListen.mock.calls.find((c) => c[0] === "network")![1] as (e: { payload: unknown }) => void;
    let tick = 0;
    for (const sel of ["input[name^=gitEnabled]", "input[name^=gitEnvName]", "input[name=gitTokenKind-gitlab][value=stored]"]) {
      const before = gl().querySelector<HTMLInputElement>(sel)!;
      before.focus();
      onNetwork({ payload: { ...net, mainError: `n${++tick}` } });
      expect(gl().querySelector(sel)).not.toBe(before);
      expect(gl().contains(document.activeElement)).toBe(true);
      expect(document.activeElement).toBe(gl().querySelector(sel));
    }
    localStorage.removeItem("maya.settingsGroup");
  });
});
