import { afterEach, describe, expect, it, vi } from "vitest";
import { comboLabel, defaultKeys } from "./keys";
import { RUNNING_VERSION } from "./release-notes";
import { gitConnections, nextNetworkRole, pairingRepaintDue, renderSettings, SETTINGS_GROUP_KEY, storedSettingsGroup, tickAddresses, tickPairingCode, type SettingsModel } from "./settings";

const voiceBase = {
  hookInstalled: true,
  completedTimeoutMinutes: 30,
  projectsDir: "",
  clonesDir: "",
  notifyOnAwaiting: true,
  speakNotifications: true,
  elevenEnabled: false,
  elevenKeySet: false,
  elevenVoices: [],
  elevenVoiceId: "",
  serverEnabled: false,
  serverUrl: "",
  serverVoice: "",
  serverModel: "",
  serverKeySet: false,
  serverVoices: [] as string[],
  serverVoicesError: null as string | null,
  voiceReport: null as string | null,
  error: null,
  listen: false,
  microphone: "",
  microphones: [],
  agent: null,
  agentModel: "",
  agents: [],
  reviewPrompt: "",
  listenError: null,
  recognizer: "system" as const,
  whisperModel: "base.en-q5_1",
  models: [],
  downloading: null,
  network: { role: "off" as const, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } },
  networkRole: "off" as const,
  networkPort: 0,
  networkMainHost: "",
  networkMainPort: 0,
  networkName: "",
};
const handlers = () => ({
  onMirror: vi.fn(),
  onInstall: vi.fn(),
  onRemove: vi.fn(),
  onTimeout: vi.fn(),
  onProjectsDir: vi.fn(),
  onNotify: vi.fn(),
  onClonesDir: vi.fn(),
  onSpeak: vi.fn(),
  onElevenEnabled: vi.fn(),
  onElevenKey: vi.fn(),
  onElevenVoice: vi.fn(),
  onServerEnabled: vi.fn(),
  onServerUrl: vi.fn(),
  onServerVoice: vi.fn(),
  onServerModel: vi.fn(),
  onServerKey: vi.fn(),
  onTryVoice: vi.fn(),
  onVoiceVolume: vi.fn(),
  onListen: vi.fn(),
  onMicrophone: vi.fn(),
  onAgent: vi.fn(),
  onAgentModel: vi.fn(),
  onReviewPrompt: vi.fn(),
  onRecognizer: vi.fn(),
  onWhisperModel: vi.fn(),
  onDownloadModel: vi.fn(),
  onRemoveModel: vi.fn(),
  onRole: vi.fn(),
  onPairAgain: vi.fn(),
  onPort: vi.fn(),
  onMainHost: vi.fn(),
  onMainPort: vi.fn(),
  onName: vi.fn(),
  onRegenerate: vi.fn(),
  onRemoveAssistant: vi.fn(),
  onMakeMain: vi.fn(),
  onPromote: vi.fn(),
  onPromoteCancel: vi.fn(),
  onNewKeyAsk: vi.fn(),
  onNewKey: vi.fn(),
  onNewKeyCancel: vi.fn(),
  onForgetMain: vi.fn(),
  onPair: vi.fn(),
});

const agents = [
  { harness: "claude-code" as const, models: [{ id: "opus", label: "Opus", efforts: [] }], efforts: [], modes: [] },
  { harness: "grok" as const, models: [{ id: "grok-4.7", label: "grok-4.7", efforts: [] }], efforts: [], modes: [] },
];

describe("renderSettings", () => {
  it("puts Maya's agent first, with that agent's models under it", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, agents, agent: "grok", agentModel: "grok-4.7" }, h);
    expect(el.querySelector(".settings__section .settings__heading")?.textContent).toBe("General");
    const agent = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...agent.options].map((o) => o.textContent)).toEqual(["Claude Code", "Grok Build"]);
    expect(agent.value).toBe("grok");
    const model = el.querySelector<HTMLSelectElement>("select[name=agentModel]")!;
    expect([...model.options].map((o) => o.value)).toEqual(["", "grok-4.7"]);
    expect(model.value).toBe("grok-4.7");
    expect(el.querySelector("[data-for=agent]")?.textContent).toContain("voice commands");
    agent.value = "claude-code";
    agent.dispatchEvent(new Event("change"));
    expect(h.onAgent).toHaveBeenCalledWith("claude-code");
    model.value = "";
    model.dispatchEvent(new Event("change"));
    // The model is saved with the agent the Agent select shows, never alone.
    expect(h.onAgentModel).toHaveBeenCalledWith("claude-code", "");
  });

  it("keeps a chosen agent that is not installed selected, with only the Default model", () => {
    const el = renderSettings({ ...voiceBase, agents, agent: "codex", agentModel: "gpt-5" }, handlers());
    const agent = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    expect([...agent.options].map((o) => o.textContent)).toEqual(["Claude Code", "Grok Build", "Codex (not installed)"]);
    expect(agent.value).toBe("codex");
    const model = el.querySelector<HTMLSelectElement>("select[name=agentModel]")!;
    expect([...model.options].map((o) => o.value)).toEqual([""]);
  });

  it("disables the Model select the moment the Agent changes, until the repaint brings the new list", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, agents, agent: "grok", agentModel: "grok-4.7" }, h);
    const agent = el.querySelector<HTMLSelectElement>("select[name=agent]")!;
    const model = el.querySelector<HTMLSelectElement>("select[name=agentModel]")!;
    expect(model.disabled).toBe(false);
    agent.value = "claude-code";
    agent.dispatchEvent(new Event("change"));
    expect(model.disabled).toBe(true);
  });

  it("falls back to Default for a model the agent no longer lists, and to Claude Code before the listing", () => {
    const el = renderSettings({ ...voiceBase, agents, agent: "grok", agentModel: "grok-9" }, handlers());
    expect(el.querySelector<HTMLSelectElement>("select[name=agentModel]")!.value).toBe("");
    const early = renderSettings({ ...voiceBase, agents: [], agent: null }, handlers());
    expect(el.querySelector("select[name=interpreterModel]")).toBeNull();
    expect([...early.querySelector<HTMLSelectElement>("select[name=agent]")!.options].map((o) => o.value)).toEqual(["claude-code"]);
  });

  it("offers the review prompt with the built-in one as placeholder", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, reviewPrompt: "" }, h);
    const ta = el.querySelector<HTMLTextAreaElement>("textarea[name=reviewPrompt]")!;
    expect(ta.placeholder).toContain("Review pull request #{number}");
    expect(ta.value).toBe("");
    ta.value = "/should-i-approve PR #{number}";
    ta.dispatchEvent(new Event("change"));
    expect(h.onReviewPrompt).toHaveBeenCalledWith("/should-i-approve PR #{number}");
  });

  it("asks for Full Disk Access only when Maya cannot see the Focus state", () => {
    const h = { ...handlers(), onFullDiskAccess: vi.fn() };
    expect(renderSettings(voiceBase, h).querySelector("[data-for=focus]")).toBeNull();
    const el = renderSettings({ ...voiceBase, focusVisible: false }, h);
    const hint = el.querySelector("[data-for=focus]");
    expect(hint?.textContent).toContain("Full Disk Access");
    expect(hint?.textContent).toContain("Focus");
    el.querySelector<HTMLButtonElement>("button[data-action=full-disk-access]")!.click();
    expect(h.onFullDiskAccess).toHaveBeenCalled();
  });

  it("names which agent each hook button is for", () => {
    const labels = (claude: boolean, codex: boolean) =>
      [...renderSettings({ ...voiceBase, hookInstalled: claude, codexHookInstalled: codex }, handlers()).querySelectorAll("button")]
        .map((b) => b.textContent)
        .filter((t) => t?.includes("hook"));
    expect(labels(false, false)).toEqual(["Install Claude hook", "Install Codex hook"]);
    expect(labels(true, true)).toEqual(["Remove Claude hook", "Remove Codex hook"]);
  });

  it("the status line block offers install, remove or says what blocks it", () => {
    const h = { ...handlers(), onStatuslineInstall: vi.fn(), onStatuslineRemove: vi.fn() };
    const render = (statusline: SettingsModel["statusline"]) => renderSettings({ ...voiceBase, statusline }, h);
    const texts = (el: HTMLElement) => [...el.querySelectorAll(".settings__status")].map((s) => s.textContent);

    const checking = render(null);
    expect(texts(checking)).toContain("Checking status line…");

    const missing = render({ installed: false });
    expect(texts(missing)).toContain("Usage and limits are not recorded. Install the status line wrapper to see them in session windows.");
    const install = missing.querySelector<HTMLButtonElement>("button[data-action=install-statusline]")!;
    expect(install.textContent).toBe("Install usage status line");
    install.click();
    expect(h.onStatuslineInstall).toHaveBeenCalledOnce();

    const installed = render({ installed: true });
    expect(texts(installed)).toContain("Usage and limits are recorded from Claude Code's status line.");
    const remove = installed.querySelector<HTMLButtonElement>("button[data-action=remove-statusline]")!;
    expect(remove.textContent).toBe("Remove usage status line");
    remove.click();
    expect(h.onStatuslineRemove).toHaveBeenCalledOnce();

    const blocked = render({ installed: false, blockedBy: "/etc/claude/managed-settings.json" });
    expect(texts(blocked)).toContain("Usage and limits cannot be recorded: the status line is set in /etc/claude/managed-settings.json.");
    expect(blocked.querySelector<HTMLButtonElement>("button[data-action=install-statusline]")!.disabled).toBe(true);
  });

  it("installs and removes Codex hooks independently of Claude", () => {
    const h = { ...handlers(), onCodexInstall: vi.fn(), onCodexRemove: vi.fn() };
    const missing = renderSettings({ ...voiceBase, codexHookInstalled: false }, h);
    missing.querySelector<HTMLButtonElement>("button[data-action=install-codex]")!.click();
    expect(h.onCodexInstall).toHaveBeenCalledOnce();
    expect(h.onInstall).not.toHaveBeenCalled();
    const installed = renderSettings({ ...voiceBase, codexHookInstalled: true }, h);
    expect(installed.textContent).toContain("/hooks");
    installed.querySelector<HTMLButtonElement>("button[data-action=remove-codex]")!.click();
    expect(h.onCodexRemove).toHaveBeenCalledOnce();
    expect(h.onRemove).not.toHaveBeenCalled();
  });
  it("offers install when the hook is missing", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: false, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("not installed");
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=install]")!;
    btn.click();
    expect(h.onInstall).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=remove]")).toBeNull();
  });

  it("offers remove when the hook is installed", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("installed");
    el.querySelector<HTMLButtonElement>("button[data-action=remove]")!.click();
    expect(h.onRemove).toHaveBeenCalled();
  });

  it("reports timeout changes and shows errors", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: null, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: "boom", listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=timeout]")!;
    expect(input.value).toBe("30");
    input.value = "45";
    input.dispatchEvent(new Event("change"));
    expect(h.onTimeout).toHaveBeenCalledWith(45);
    expect(el.querySelector(".settings__error")?.textContent).toBe("boom");
  });

  it("has a notification toggle that reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=notify]")!;
    expect(box.type).toBe("checkbox");
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("awaits a decision");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onNotify).toHaveBeenCalledWith(false);
    expect(renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: false, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h).querySelector<HTMLInputElement>("input[name=notify]")!.checked).toBe(false);
  });

  it("has a Debug group with a debug mode switch that saves", () => {
    const h = { ...handlers(), onDebugMode: vi.fn() };
    const el = renderSettings({ ...voiceBase, debugMode: false }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=debug-mode]")!;
    expect(box.checked).toBe(false);
    box.checked = true;
    box.dispatchEvent(new Event("change"));
    expect(h.onDebugMode).toHaveBeenCalledWith(true);
  });

  it("has a PR review notification toggle, on unless the config says off", () => {
    const h = { ...handlers(), onNotifyPrReview: vi.fn() };
    const model = { hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null };
    const box = renderSettings(model, h).querySelector<HTMLInputElement>("input[name=notifyPrReview]")!;
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("assigned to me is approved or needs changes");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onNotifyPrReview).toHaveBeenCalledWith(false);
    expect(renderSettings({ ...model, notifyOnPrReview: false }, h).querySelector<HTMLInputElement>("input[name=notifyPrReview]")!.checked).toBe(false);
  });

  it("shows the voice order and a switch per remote voice", () => {
    const h = handlers();
    const el = renderSettings(voiceBase, h);
    expect(el.querySelector("[data-for=voice-order]")!.textContent).toMatch(/^Maya speaks with the first voice that works: ElevenLabs, then the voice server, then .+ \(built in\)\.$/);
    expect(el.querySelector("select[name=voiceProvider]")).toBeNull();
    const eleven = el.querySelector<HTMLInputElement>("input[name=elevenEnabled]")!;
    const server = el.querySelector<HTMLInputElement>("input[name=serverEnabled]")!;
    expect(eleven.checked).toBe(false);
    expect(server.checked).toBe(false);
    expect(eleven.closest("label")?.textContent).toContain("Use ElevenLabs");
    expect(server.closest("label")?.textContent).toContain("Use a voice server (OpenAI-compatible)");
    expect(el.querySelector("input[name=elevenKey]")).toBeNull();
    expect(el.querySelector("input[name=serverUrl]")).toBeNull();
    eleven.checked = true;
    eleven.dispatchEvent(new Event("change"));
    expect(h.onElevenEnabled).toHaveBeenCalledWith(true);
    server.checked = true;
    server.dispatchEvent(new Event("change"));
    expect(h.onServerEnabled).toHaveBeenCalledWith(true);
  });

  it("the ElevenLabs section shows its key and voice when on", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, elevenEnabled: true, elevenKeySet: false, elevenVoices: [{ voiceId: "abc", name: "Rachel" }, { voiceId: "def", name: "Bella" }], elevenVoiceId: "abc" }, h);
    const key = el.querySelector<HTMLInputElement>("input[name=elevenKey]")!;
    expect(key.type).toBe("password");
    key.value = " sk-123 ";
    key.dispatchEvent(new Event("change"));
    expect(h.onElevenKey).toHaveBeenCalledWith("sk-123");
    const voice = el.querySelector<HTMLSelectElement>("select[name=elevenVoice]")!;
    expect([...voice.options].map((o) => o.textContent)).toEqual(["Rachel", "Bella"]);
    expect(voice.value).toBe("abc");
    voice.value = "def";
    voice.dispatchEvent(new Event("change"));
    expect(h.onElevenVoice).toHaveBeenCalledWith("def");
    el.querySelector<HTMLButtonElement>("button[data-action=try-voice]")!.click();
    expect(h.onTryVoice).toHaveBeenCalled();
    const saved = renderSettings({ ...voiceBase, elevenEnabled: true, elevenKeySet: true }, h);
    expect(saved.querySelector<HTMLInputElement>("input[name=elevenKey]")!.placeholder).toContain("saved");
  });

  it("the server section offers a list when the server has one and a text box when not", () => {
    const h = handlers();
    const listed = renderSettings({ ...voiceBase, serverEnabled: true, serverUrl: "http://mac.local:8004", serverVoices: ["default", "tiago.wav"], serverVoice: "tiago.wav" }, h);
    const select = listed.querySelector<HTMLSelectElement>("select[name=serverVoice]")!;
    expect(select.value).toBe("tiago.wav");
    expect(listed.querySelector("input[name=serverVoice]")).toBeNull();
    select.value = "default";
    select.dispatchEvent(new Event("change"));
    expect(h.onServerVoice).toHaveBeenCalledWith("default");

    const el = renderSettings({ ...voiceBase, serverEnabled: true, serverUrl: "http://mac.local:8004", serverVoices: [], serverVoicesError: "404" }, h);
    const text = el.querySelector<HTMLInputElement>("input[name=serverVoice]")!;
    expect(text.placeholder).toBe("default");
    expect(el.querySelector("select[name=serverVoice]")).toBeNull();
    expect(el.querySelector("[data-for=server-voices]")!.textContent).toBe("Could not list the server's voices: 404");
    text.value = "";
    text.dispatchEvent(new Event("change"));
    expect(h.onServerVoice).toHaveBeenCalledWith("");

    const url = el.querySelector<HTMLInputElement>("input[name=serverUrl]")!;
    expect(url.placeholder).toBe("http://localhost:8004");
    url.value = "http://mac.local:8004";
    url.dispatchEvent(new Event("change"));
    expect(h.onServerUrl).toHaveBeenCalledWith("http://mac.local:8004");
    const model = el.querySelector<HTMLInputElement>("input[name=serverModel]")!;
    expect(model.placeholder).toBe("optional");
    model.value = "tts-1";
    model.dispatchEvent(new Event("change"));
    expect(h.onServerModel).toHaveBeenCalledWith("tts-1");

    const key = el.querySelector<HTMLInputElement>("input[name=serverKey]")!;
    expect(key.type).toBe("password");
    expect(key.placeholder).toBe("optional; paste a key");
    key.value = " sk-abc ";
    key.dispatchEvent(new Event("change"));
    expect(h.onServerKey).toHaveBeenCalledWith("sk-abc");
    expect(key.value).toBe("");
    const saved = renderSettings({ ...voiceBase, serverEnabled: true, serverKeySet: true }, h);
    expect(saved.querySelector<HTMLInputElement>("input[name=serverKey]")!.placeholder).toContain("saved in");
  });

  it("the voice test's report is shown", () => {
    const el = renderSettings({ ...voiceBase, voiceReport: "ElevenLabs failed: 402. Spoke with the voice server." }, handlers());
    expect(el.querySelector("[data-for=voice-report]")!.textContent).toBe("ElevenLabs failed: 402. Spoke with the voice server.");
    expect(renderSettings(voiceBase, handlers()).querySelector("[data-for=voice-report]")).toBeNull();
  });

  it("has a speak toggle that reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=speak]")!;
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("Speak");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onSpeak).toHaveBeenCalledWith(false);
  });

  it("shows the clones directory with the default as placeholder and reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=clonesDir]")!;
    expect(input.placeholder).toBe("~/dev/reviews");
    expect(input.value).toBe("");
    input.value = " ~/tmp/reviews ";
    input.dispatchEvent(new Event("change"));
    expect(h.onClonesDir).toHaveBeenCalledWith("~/tmp/reviews");
  });

  it("takes a wake word, names it on the listen switch, and shows Maya when blank", () => {
    const h = { ...handlers(), onListenWord: vi.fn() };
    const blank = renderSettings({ ...voiceBase }, h);
    const input = blank.querySelector<HTMLInputElement>("input[name=listenWord]")!;
    expect(input.placeholder).toBe("Maya");
    expect(input.value).toBe("");
    expect(blank.textContent).toContain('Listen for "Maya"');
    input.value = "  Jarvis ";
    input.dispatchEvent(new Event("change"));
    expect(h.onListenWord).toHaveBeenCalledWith("Jarvis");
    const named = renderSettings({ ...voiceBase, listenWord: "Jarvis" }, h);
    expect(named.querySelector<HTMLInputElement>("input[name=listenWord]")!.value).toBe("Jarvis");
    expect(named.textContent).toContain('Listen for "Jarvis"');
    const assistant = renderSettings({ ...voiceBase, networkRole: "assistant", network: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "desk", error: null } } }, h);
    expect(assistant.querySelector<HTMLInputElement>("input[name=listenWord]")!.disabled).toBe(true);
  });

  it("shows the projects directory and reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    expect(input.value).toBe("~/dev");
    input.value = "~/code";
    input.dispatchEvent(new Event("change"));
    expect(h.onProjectsDir).toHaveBeenCalledWith("~/code");
  });

  it("explains under the listen toggle why listening stopped", () => {
    const h = handlers();
    const failed = renderSettings({ ...voiceBase, listen: false, listenError: "Dictation is off. Turn it on in System Settings." }, h);
    const err = failed.querySelector<HTMLElement>(".settings__error[data-for=listen]")!;
    expect(err.textContent).toContain("Dictation is off");
    expect(err.previousElementSibling?.querySelector("input[name=listen]")).not.toBeNull();
    expect(renderSettings({ ...voiceBase, listen: true, listenError: null }, h).querySelector(".settings__error[data-for=listen]")).toBeNull();
  });

  it("lists the groups in a sidebar, one titled section each, and shows only General at first", () => {
    const el = renderSettings(voiceBase, handlers());
    const nav = [...el.querySelectorAll<HTMLButtonElement>(".settings__nav button")];
    expect(nav.map((b) => b.textContent)).toEqual(["General", "Keyboard", "Sessions", "Git", "Pull requests", "Notifications", "Voice", "Network", "Debug", "Release Notes"]);
    expect([...el.querySelectorAll(".settings__heading")].map((x) => x.textContent)).toEqual(["General", "Keyboard", "Sessions", "Git", "Pull requests", "Notifications", "Voice", "Network", "Debug", "Release Notes"]);
    const sections = [...el.querySelectorAll<HTMLElement>(".settings__section")];
    expect(sections.map((s) => s.dataset.group)).toEqual(["general", "keyboard", "sessions", "git", "reviews", "notifications", "voice", "network", "debug", "releaseNotes"]);
    expect(sections.filter((s) => !s.hidden).map((s) => s.dataset.group)).toEqual(["general"]);
    expect(nav.map((b) => b.getAttribute("aria-current"))).toEqual(["true", null, null, null, null, null, null, null, null, null]);
  });

  it("shows the release notes, last in the sidebar", () => {
    const el = renderSettings({ ...voiceBase, settingsGroup: "releaseNotes" }, handlers());
    const s = el.querySelector<HTMLElement>(".settings__section[data-group=releaseNotes]")!;
    expect(s.hidden).toBe(false);
    expect(s.querySelector(".settings__heading")!.textContent).toBe("Release Notes");
    expect(s.querySelector(".release-notes__version")!.textContent).toBe(`Maya ${RUNNING_VERSION}`);
  });

  it("puts each setting in its group", () => {
    const el = renderSettings({ ...voiceBase, elevenEnabled: true, serverEnabled: true, serverUrl: "http://x", recognizer: "builtin", focusVisible: false, networkRole: "main", network: { ...voiceBase.network, role: "main" } }, handlers());
    const groupOf = (selector: string) => el.querySelector(selector)?.closest<HTMLElement>(".settings__section")?.dataset.group;
    const expected: Record<string, string> = {
      "select[name=agent]": "general",
      "select[name=agentModel]": "general",
      "button[data-action=remove]": "sessions",
      "button[data-action=install-codex]": "sessions",
      "input[name=timeout]": "sessions",
      "input[name=projectsDir]": "sessions",
      "input[name=clonesDir]": "reviews",
      "textarea[name=reviewPrompt]": "reviews",
      "input[name=notifyPrReview]": "reviews",
      "input[name=notify]": "notifications",
      "input[name=speak]": "notifications",
      "[data-for=focus]": "notifications",
      "input[name=elevenEnabled]": "voice",
      "input[name=serverEnabled]": "voice",
      "input[name=serverUrl]": "voice",
      "input[name=serverKey]": "voice",
      "input[name=elevenKey]": "voice",
      "select[name=elevenVoice]": "voice",
      "input[name=voiceVolume]": "voice",
      "button[data-action=try-voice]": "voice",
      "input[name=listen]": "voice",
      "select[name=recognizer]": "voice",
      "select[name=whisperModel]": "voice",
      "select[name=microphone]": "voice",
      "select[name=networkRole]": "network",
      "input[name=networkPort]": "network",
      "input[name=networkName]": "network",
    };
    expect(Object.fromEntries(Object.keys(expected).map((s) => [s, groupOf(s)]))).toEqual(expected);
  });

  it("splits Voice into Speaking, then Listening", () => {
    const voice = renderSettings(voiceBase, handlers()).querySelector(".settings__section--voice")!;
    expect([...voice.querySelectorAll(".settings__subheading")].map((x) => x.textContent)).toEqual(["Speaking", "Listening"]);
    const [speaking, listening] = [...voice.querySelectorAll(".settings__subheading")];
    const follows = (a: Element, b: Element) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
    expect(follows(speaking, voice.querySelector("input[name=elevenEnabled]")!)).toBe(true);
    expect(follows(voice.querySelector("button[data-action=try-voice]")!, listening)).toBe(true);
    expect(follows(listening, voice.querySelector("input[name=listen]")!)).toBe(true);
  });

  it("opens the group the model names", () => {
    const el = renderSettings({ ...voiceBase, settingsGroup: "network" }, handlers());
    expect([...el.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group)).toEqual(["network"]);
    expect(el.querySelector(".settings__nav button[aria-current=true]")?.textContent).toBe("Network");
  });

  it("switches group in place from the sidebar, keeps the choice on the model and remembers it", () => {
    localStorage.removeItem(SETTINGS_GROUP_KEY);
    const model: SettingsModel = { ...voiceBase };
    const el = renderSettings(model, handlers());
    const typed = el.querySelector<HTMLInputElement>("input[name=projectsDir]")!;
    el.querySelector<HTMLButtonElement>(".settings__nav button[data-group=voice]")!.click();
    expect([...el.querySelectorAll<HTMLElement>(".settings__section")].filter((s) => !s.hidden).map((s) => s.dataset.group)).toEqual(["voice"]);
    expect(el.querySelector(".settings__nav button[aria-current=true]")?.textContent).toBe("Voice");
    expect(el.querySelector("input[name=projectsDir]")).toBe(typed);
    expect(model.settingsGroup).toBe("voice");
    expect(storedSettingsGroup()).toBe("voice");
  });

  it("falls back to General for a missing or unknown remembered group", () => {
    localStorage.removeItem(SETTINGS_GROUP_KEY);
    expect(storedSettingsGroup()).toBe("general");
    localStorage.setItem(SETTINGS_GROUP_KEY, "assistant");
    expect(storedSettingsGroup()).toBe("general");
    localStorage.setItem(SETTINGS_GROUP_KEY, "reviews");
    expect(storedSettingsGroup()).toBe("reviews");
    localStorage.removeItem(SETTINGS_GROUP_KEY);
  });

  it("keeps the page error visible whichever group is open", () => {
    const el = renderSettings({ ...voiceBase, error: "boom", settingsGroup: "network" }, handlers());
    const err = el.querySelector<HTMLElement>(".settings__error")!;
    expect(err.textContent).toBe("boom");
    expect(err.closest(".settings__section")).toBeNull();
  });

  it("puts a device's buttons on their own row under its name and status", () => {
    const assistants = [{ id: "a1", name: "Iris", hostname: "iris", platform: "android", address: "192.168.55.99", connected: true, lastSeen: null, canLead: true, wantsMirror: true, mirror: true }];
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: { ...voiceBase.network, role: "main", assistants } }, handlers());
    const row = el.querySelector(".settings__assistant")!;
    expect(row.firstElementChild?.textContent).toContain("Iris (Android, 192.168.55.99) — Connected");
    const actions = row.querySelector(".settings__assistant-actions")!;
    expect([...actions.children].map((c) => (c as HTMLElement).dataset.action ?? c.querySelector<HTMLElement>("input")?.dataset.action)).toEqual(["remove-assistant", "mirror"]);
  });

  it("a row that mirrors gets the switch but never Make main", () => {
    const assistants = [{ id: "a1", name: "Iris", hostname: "iris", platform: "android", address: "192.168.55.99", connected: true, lastSeen: null, canLead: true, wantsMirror: true, mirror: false }];
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: { ...voiceBase.network, role: "main", assistants } }, handlers());
    expect(el.querySelector("[data-action=make-main]")).toBeNull();
    expect(el.querySelector("[data-action=mirror]")).not.toBeNull();
  });

  it("shows the app version at the end of the General section once it is known", () => {
    const el = renderSettings({ ...voiceBase, version: "0.13.0" }, handlers());
    const general = el.querySelector(".settings__section--general")!;
    expect(general.lastElementChild?.className).toBe("settings__hint settings__version");
    expect(general.lastElementChild?.textContent).toBe("Maya 0.13.0");
    expect(renderSettings(voiceBase, handlers()).querySelector(".settings__version")).toBeNull();
  });

  it("has the listen toggle and a microphone picker", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, listen: true, microphone: "USB Mic", microphones: ["MacBook Pro Microphone", "USB Mic"], listenError: null }, h);
    const listen = el.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(listen.checked).toBe(true);
    expect(listen.closest("label")?.textContent).toContain("Listen for");
    listen.checked = false;
    listen.dispatchEvent(new Event("change"));
    expect(h.onListen).toHaveBeenCalledWith(false);
    const mic = el.querySelector<HTMLSelectElement>("select[name=microphone]")!;
    expect([...mic.options].map((o) => o.textContent)).toEqual(["Built-in (recommended)", "MacBook Pro Microphone", "USB Mic"]);
    expect(mic.value).toBe("USB Mic");
    mic.value = "";
    mic.dispatchEvent(new Event("change"));
    expect(h.onMicrophone).toHaveBeenCalledWith("");
  });

  const models = [
    { id: "tiny.en", label: "Tiny (78 MB, fastest)", bytes: 77704715, downloaded: false },
    { id: "base.en-q5_1", label: "Base, quantised (60 MB, recommended)", bytes: 59721011, downloaded: true },
  ];

  it("offers System and Built-in recognition, System first", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, models }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=recognizer]")!;
    expect([...sel.options].map((o) => o.value)).toEqual(["system", "builtin"]);
    expect(sel.value).toBe("system");
    expect(el.querySelector("select[name=whisperModel]")).toBeNull();
    sel.value = "builtin";
    sel.dispatchEvent(new Event("change"));
    expect(h.onRecognizer).toHaveBeenCalledWith("builtin");
  });

  it("with Built-in, lists the models, shows which are downloaded, and downloads or removes", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, recognizer: "builtin", models }, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=whisperModel]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Tiny (78 MB, fastest)", "Base, quantised (60 MB, recommended) ✓"]);
    expect(sel.value).toBe("base.en-q5_1");
    expect(el.querySelector("button[data-action=download-model]")).toBeNull();
    const remove = el.querySelector<HTMLButtonElement>("button[data-action=remove-model]")!;
    expect(remove.disabled).toBe(true);
    sel.value = "tiny.en";
    sel.dispatchEvent(new Event("change"));
    expect(h.onWhisperModel).toHaveBeenCalledWith("tiny.en");

    const tiny = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models }, h);
    const dl = tiny.querySelector<HTMLButtonElement>("button[data-action=download-model]")!;
    expect(dl.textContent).toContain("Download");
    expect(dl.textContent).toContain("78 MB");
    dl.click();
    expect(h.onDownloadModel).toHaveBeenCalledWith("tiny.en");
    expect(tiny.querySelector(".settings__row .settings__hint")?.textContent).toContain("Download");
  });

  it("says the model stays on this computer, under the Linux user agent", () => {
    const original = navigator.userAgent;
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15", configurable: true });
    try {
      const tiny = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models }, handlers());
      expect(tiny.querySelector(".settings__row .settings__hint")?.textContent).toBe("Download the model once; it stays on this computer.");
    } finally {
      Object.defineProperty(navigator, "userAgent", { value: original, configurable: true });
    }
  });

  it("shows download progress and disables the button meanwhile", () => {
    const el = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models, downloading: { id: "tiny.en", received: 38852357, total: 77704715 } }, handlers());
    const bar = el.querySelector<HTMLProgressElement>("progress[name=modelDownload]")!;
    expect(bar.value).toBe(38852357);
    expect(bar.max).toBe(77704715);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=download-model]")!.disabled).toBe(true);
    // The progress text names the model in flight, so it's never ambiguous which one.
    expect(el.querySelector(".settings__progress")?.textContent).toContain("Tiny");
    expect(el.querySelector(".settings__progress")?.textContent).toContain("50%");
  });

  it("says to wait when a different model is the one downloading", () => {
    const el = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models, downloading: { id: "base.en-q5_1", received: 1, total: 2 } }, handlers());
    const dl = el.querySelector<HTMLButtonElement>("button[data-action=download-model]")!;
    expect(dl.disabled).toBe(true);
    expect(dl.textContent).toBe("Wait for the current download");
  });

  it("removes a downloaded model that isn't the one in use, and disables the one in use", () => {
    const h = handlers();
    const twoDownloaded = [
      { id: "tiny.en", label: "Tiny (78 MB, fastest)", bytes: 77704715, downloaded: true },
      { id: "base.en-q5_1", label: "Base, quantised (60 MB, recommended)", bytes: 59721011, downloaded: true },
    ];
    const el = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "base.en-q5_1", models: twoDownloaded }, h);
    const removeButtons = [...el.querySelectorAll<HTMLButtonElement>("button[data-action=remove-model]")];
    expect(removeButtons).toHaveLength(2);

    const inUse = removeButtons.find((b) => b.dataset.model === "base.en-q5_1")!;
    expect(inUse.disabled).toBe(true);

    const other = removeButtons.filter((b) => !b.disabled);
    expect(other).toHaveLength(1);
    expect(other[0].dataset.model).toBe("tiny.en");
    other[0].click();
    expect(h.onRemoveModel).toHaveBeenCalledWith("tiny.en");
  });
});

describe("Voice volume", () => {
  it("shows the saved volume and saves a new one when the slider is let go", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, voiceVolume: 80 }, h);
    const slider = el.querySelector<HTMLInputElement>("input[name=voiceVolume]")!;
    expect(slider.type).toBe("range");
    expect([slider.min, slider.max, slider.step, slider.value]).toEqual(["10", "100", "5", "80"]);
    const shown = el.querySelector("[data-for=voice-volume]")!;
    expect(shown.textContent).toBe("80%");
    slider.value = "55";
    slider.dispatchEvent(new Event("input"));
    expect(shown.textContent).toBe("55%");
    expect(h.onVoiceVolume).not.toHaveBeenCalled();
    slider.dispatchEvent(new Event("change"));
    expect(h.onVoiceVolume).toHaveBeenCalledWith(55);
  });

  it("is at full volume when the config has none", () => {
    const el = renderSettings(voiceBase, handlers());
    expect(el.querySelector<HTMLInputElement>("input[name=voiceVolume]")!.value).toBe("100");
  });
});

describe("Network settings", () => {
  it("offers Off, Main and Assistant, off by default, and reports Off/Main changes", () => {
    const h = handlers();
    const el = renderSettings(voiceBase, h);
    const sel = el.querySelector<HTMLSelectElement>("select[name=networkRole]")!;
    expect([...sel.options].map((o) => o.value)).toEqual(["off", "main", "assistant"]);
    expect(sel.value).toBe("off");
    sel.value = "main";
    sel.dispatchEvent(new Event("change"));
    expect(h.onRole).toHaveBeenCalledWith("main");
  });

  it("role main lists this machine's IP addresses, right after the port", () => {
    const el = renderSettings(
      { ...voiceBase, networkRole: "main", network: { role: "main", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, addresses: ["192.168.1.20", "10.0.0.5"] } },
      handlers(),
    );
    const addr = el.querySelector(".settings__addresses")!;
    expect(addr.textContent).toBe("This machine's addresses: 192.168.1.20, 10.0.0.5");
    expect(addr.previousElementSibling?.querySelector("input[name=networkPort]")).not.toBeNull();
  });

  it("tickAddresses updates, adds and removes the addresses line in place without a repaint", () => {
    const network = { role: "main" as const, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, addresses: ["192.168.1.20"] };
    const model = { ...voiceBase, networkRole: "main" as const, network };
    const el = renderSettings(model, handlers());
    const port = el.querySelector<HTMLInputElement>("input[name=networkPort]")!;
    port.value = "5000"; // an unsaved edit the refresh must not wipe

    expect(tickAddresses(el, model, ["192.168.1.20"])).toBe(false);
    expect(tickAddresses(el, model, ["10.0.0.5", "192.168.1.20"])).toBe(true);
    expect(el.querySelector(".settings__addresses")!.textContent).toBe("This machine's addresses: 10.0.0.5, 192.168.1.20");
    expect(model.network.addresses).toEqual(["10.0.0.5", "192.168.1.20"]);

    expect(tickAddresses(el, model, [])).toBe(true);
    expect(el.querySelector(".settings__addresses")).toBeNull();

    expect(tickAddresses(el, model, ["10.0.0.5"])).toBe(true);
    const line = el.querySelector(".settings__addresses")!;
    expect(line.textContent).toBe("This machine's addresses: 10.0.0.5");
    expect(line.previousElementSibling?.querySelector("input[name=networkPort]")).not.toBeNull();
    expect(el.contains(port) && port.value).toBe("5000");
  });

  it("roles off and assistant do not list the addresses", () => {
    const network = { role: "main" as const, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, addresses: ["192.168.1.20"] };
    expect(renderSettings({ ...voiceBase, networkRole: "off", network }, handlers()).querySelector(".settings__addresses")).toBeNull();
    expect(renderSettings({ ...voiceBase, networkRole: "assistant", network }, handlers()).querySelector(".settings__addresses")).toBeNull();
  });

  it("role main shows the port, the pairing code, Regenerate and the assistants list", () => {
    const h = handlers();
    const el = renderSettings(
      {
        ...voiceBase,
        networkRole: "main",
        networkPort: 4127,
        network: {
          role: "main",
          code: { code: "483921", expiresAt: 180_000 },
          assistants: [{ id: "a1", name: "laptop", hostname: "laptop.local", platform: "macos", connected: true, lastSeen: null }],
          assistant: { connected: false, mainName: null, error: null },
        },
      },
      h,
      0,
    );
    expect(el.querySelector<HTMLSelectElement>("select[name=networkRole]")!.value).toBe("main");
    const port = el.querySelector<HTMLInputElement>("input[name=networkPort]")!;
    expect(port.value).toBe("4127");
    expect(el.querySelector(".settings__addresses")).toBeNull();
    port.value = "5000";
    port.dispatchEvent(new Event("change"));
    expect(h.onPort).toHaveBeenCalledWith(5000);
    expect(el.querySelector(".settings__code")?.textContent).toBe("483 921");
    expect(el.querySelector(".settings__code-expiry")?.textContent).toContain("3 min");
    const regen = el.querySelector<HTMLButtonElement>("button[data-action=regenerate-code]")!;
    expect(regen.textContent).toBe("Regenerate");
    regen.click();
    expect(h.onRegenerate).toHaveBeenCalled();
    const remove = el.querySelector<HTMLButtonElement>("button[data-action=remove-assistant]")!;
    expect(remove.dataset.id).toBe("a1");
    remove.click();
    expect(h.onRemoveAssistant).toHaveBeenCalledWith("a1");
    expect(el.querySelector("input[name=networkHost]")).toBeNull();
  });

  it("an expired pairing code is not shown, and the timer repaints until just after it expires", () => {
    const withCode: SettingsModel = {
      ...voiceBase,
      networkRole: "main",
      network: { role: "main", code: { code: "483921", expiresAt: 300_000 }, assistants: [], assistant: { connected: false, mainName: null, error: null } },
    };
    expect(renderSettings(withCode, handlers(), 100_000).querySelector("input[name=networkName]")).not.toBeNull();
    expect(renderSettings(withCode, handlers(), 100_000).querySelector(".settings__code-expiry")?.textContent).toBe("expires in 4 min");
    expect(renderSettings(withCode, handlers(), 250_000).querySelector(".settings__code-expiry")?.textContent).toBe("expires in 1 min");
    const expired = renderSettings(withCode, handlers(), 300_001);
    expect(expired.querySelector(".settings__code")).toBeNull();
    expect(expired.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Show pairing code");
    // The tick edits in place, so unsaved text elsewhere in Settings survives.
    const host = document.createElement("div");
    host.append(renderSettings(withCode, handlers(), 100_000));
    const marker = document.createElement("input");
    marker.value = "typing";
    host.append(marker);
    tickPairingCode(host, withCode, 250_000);
    expect(host.querySelector(".settings__code-expiry")?.textContent).toBe("expires in 1 min");
    expect(host.querySelector(".settings__code")).not.toBeNull();
    expect(host.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Regenerate");
    tickPairingCode(host, withCode, 320_000);
    expect(host.querySelector(".settings__code")).toBeNull();
    expect(host.querySelector(".settings__code-expiry")).toBeNull();
    // The button no longer offers to regenerate a code that is gone.
    expect(host.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Show pairing code");
    expect(marker.value).toBe("typing");
    expect(pairingRepaintDue(withCode, 100_000)).toBe(true);
    expect(pairingRepaintDue(withCode, 320_000)).toBe(true);
    expect(pairingRepaintDue(withCode, 340_000)).toBe(false);
    expect(pairingRepaintDue({ ...voiceBase, networkRole: "main" }, 0)).toBe(false);
  });

  it("a main that cannot listen shows why under the role and keeps its paired list", () => {
    const el = renderSettings(
      {
        ...voiceBase,
        networkRole: "main",
        network: {
          role: "main",
          code: null,
          assistants: [{ id: "a1", name: "laptop", hostname: "h", platform: "macos", connected: false, lastSeen: null }],
          assistant: { connected: false, mainName: null, error: null },
          mainError: "Could not listen on port 4127: Address already in use (os error 48). Choose another port.",
        },
      },
      handlers(),
    );
    const err = el.querySelector(".settings__error[data-for=network]");
    expect(err?.textContent).toBe("Could not listen on port 4127: Address already in use (os error 48). Choose another port.");
    expect(el.querySelector("select[name=networkRole]")!.closest("label")!.nextElementSibling).toBe(err);
    expect(el.querySelector(".settings__assistant")?.textContent).toContain("laptop");
    const plain = renderSettings({ ...voiceBase, networkRole: "main" }, handlers());
    expect(plain.querySelector(".settings__error[data-for=network]")).toBeNull();
    expect(plain.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Show pairing code");
  });

  it("shows an assistant's note (a different version, an unreadable board) in the list", () => {
    const el = renderSettings(
      {
        ...voiceBase,
        networkRole: "main",
        network: {
          role: "main",
          code: null,
          assistants: [
            { id: "a1", name: "laptop", hostname: "h", platform: "macos", connected: true, lastSeen: null, note: "runs Maya 0.3.0; this Mac runs 0.2.0" },
            { id: "b2", name: "desk", hostname: "h2", platform: "macos", connected: true, lastSeen: null, note: null },
          ],
          assistant: { connected: false, mainName: null, error: null },
        },
      },
      handlers(),
    );
    const notes = [...el.querySelectorAll("[data-for=assistant-note]")].map((n) => n.textContent);
    expect(notes).toEqual(["runs Maya 0.3.0; this Mac runs 0.2.0"]);
  });

  it("lists each assistant with its platform and address", () => {
    const el = renderSettings(
      {
        ...voiceBase,
        networkRole: "main",
        network: {
          role: "main",
          code: null,
          assistants: [
            { id: "a1", name: "Gnowee", hostname: "TKC-0176", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, note: null },
            { id: "b2", name: "desk", hostname: "h2", platform: "macos", address: "", connected: false, lastSeen: null, note: null },
          ],
          assistant: { connected: false, mainName: null, error: null },
        },
      },
      handlers(),
    );
    const rows = [...el.querySelectorAll(".settings__assistant span")].map((n) => n.textContent);
    expect(rows).toEqual(["Gnowee (macos, 192.168.55.70) — Connected", "desk (macos) — Never connected"]);
  });

  it("role assistant shows host, port, name, code, Pair and the status line", () => {
    const h = handlers();
    const el = renderSettings(
      {
        ...voiceBase,
        networkRole: "assistant",
        networkMainHost: "maya-mini.local",
        networkMainPort: 4127,
        networkName: "laptop",
        network: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "desk", error: null } },
      },
      h,
    );
    expect(el.querySelector<HTMLInputElement>("input[name=networkHost]")!.value).toBe("maya-mini.local");
    expect(el.querySelector<HTMLInputElement>("input[name=networkMainPort]")!.value).toBe("4127");
    expect(el.querySelector<HTMLInputElement>("input[name=networkName]")!.value).toBe("laptop");
    expect(el.querySelector("input[name=networkCode]")).not.toBeNull();
    expect(el.querySelector(".settings__status--network")?.textContent).toBe("Connected to desk");
    el.querySelector<HTMLInputElement>("input[name=networkHost]")!.value = "office.local";
    el.querySelector<HTMLInputElement>("input[name=networkHost]")!.dispatchEvent(new Event("change"));
    expect(h.onMainHost).toHaveBeenCalledWith("office.local");
    el.querySelector<HTMLButtonElement>("button[data-action=pair]")!.click();
    expect(h.onPair).toHaveBeenCalled();
    expect(el.querySelector("select[name=networkRole]")).not.toBeNull();
  });

  it("an assistant with stored credentials shows no code input, only a Pair again link that reveals it", () => {
    const h = handlers();
    const model: SettingsModel = { ...voiceBase, networkRole: "assistant", networkPaired: true, networkMainHost: "desk.local" };
    const el = renderSettings(model, h);
    expect(el.querySelector("input[name=networkCode]")).toBeNull();
    expect(el.querySelector("button[data-action=pair]")).toBeNull();
    expect(el.querySelector<HTMLInputElement>("input[name=networkHost]")!.value).toBe("desk.local");
    el.querySelector<HTMLButtonElement>("button[data-action=pair-again]")!.click();
    expect(h.onPairAgain).toHaveBeenCalled();
    const again = renderSettings({ ...model, networkRepair: true }, h);
    expect(again.querySelector("input[name=networkCode]")).not.toBeNull();
    expect(again.querySelector("button[data-action=pair]")).not.toBeNull();
    expect(again.querySelector("button[data-action=pair-again]")).toBeNull();
    // Never paired: the pairing form as before, no link.
    const fresh = renderSettings({ ...voiceBase, networkRole: "assistant" }, h);
    expect(fresh.querySelector("input[name=networkCode]")).not.toBeNull();
    expect(fresh.querySelector("button[data-action=pair-again]")).toBeNull();
  });

  it("shows Reconnecting… when paired but not connected, and the removal error otherwise", () => {
    const reconnecting = renderSettings(
      { ...voiceBase, networkRole: "assistant", network: { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } } },
      handlers(),
    );
    expect(reconnecting.querySelector(".settings__status--network")?.textContent).toBe("Reconnecting…");
    const removed = renderSettings(
      {
        ...voiceBase,
        networkRole: "assistant",
        network: { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: "Removed by the main Maya; pair again." } },
      },
      handlers(),
    );
    expect(removed.querySelector(".settings__status--network")?.textContent).toBe("Removed by the main Maya; pair again.");
  });

  it("says Reconnecting… while the client retries, with the last error under it", () => {
    const retrying = renderSettings(
      {
        ...voiceBase,
        networkRole: "assistant",
        networkPaired: true,
        network: { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: "The main Maya closed the connection.", retrying: true } },
      },
      handlers(),
    );
    expect(retrying.querySelector(".settings__status--network")?.textContent).toBe("Reconnecting…");
    expect(retrying.querySelector("[data-for=network-last-error]")?.textContent).toBe("Last error: The main Maya closed the connection.");
    const back = renderSettings(
      { ...voiceBase, networkRole: "assistant", networkPaired: true, network: { role: "assistant", code: null, assistants: [], assistant: { connected: true, mainName: "desk", error: null, retrying: false } } },
      handlers(),
    );
    expect(back.querySelector(".settings__status--network")?.textContent).toBe("Connected to desk");
    expect(back.querySelector("[data-for=network-last-error]")).toBeNull();
  });

  it("shows a failed Pair's message in the status line even on a never-paired machine", () => {
    const neverPaired = renderSettings({ ...voiceBase, networkRole: "assistant", networkError: "Wrong or expired pairing code." }, handlers());
    expect(neverPaired.querySelector(".settings__status--network")?.textContent).toBe("Wrong or expired pairing code.");
    // Previewing Assistant with no error yet and never paired: nothing to show.
    const previewing = renderSettings({ ...voiceBase, networkRole: "assistant" }, handlers());
    expect(previewing.querySelector(".settings__status--network")).toBeNull();
  });

  it("holds the pairing form's typed values on the model so a repaint never wipes them", () => {
    // renderSettings mutates the model object it was given, live, on every
    // keystroke, so a caller that re-renders from the same model (as
    // initSettings' `paint` does) never loses a half-typed field.
    const h = handlers();
    const model: SettingsModel = { ...voiceBase, networkRole: "assistant" };
    const el = renderSettings(model, h);
    const code = el.querySelector<HTMLInputElement>("input[name=networkCode]")!;
    code.value = "48392";
    code.dispatchEvent(new Event("input"));
    expect(model.networkCode).toBe("48392");
    const host = el.querySelector<HTMLInputElement>("input[name=networkHost]")!;
    host.value = "office.local";
    host.dispatchEvent(new Event("input"));
    expect(model.networkMainHost).toBe("office.local");
    const port = el.querySelector<HTMLInputElement>("input[name=networkMainPort]")!;
    port.value = "5000";
    port.dispatchEvent(new Event("input"));
    expect(model.networkMainPort).toBe(5000);
    const name = el.querySelector<HTMLInputElement>("input[name=networkName]")!;
    name.value = "laptop";
    name.dispatchEvent(new Event("input"));
    expect(model.networkName).toBe("laptop");
    // Pair reads the model, not stale closures over the DOM elements.
    el.querySelector<HTMLButtonElement>("button[data-action=pair]")!.click();
    expect(h.onPair).toHaveBeenCalledWith("office.local", 5000, "laptop", "48392");
  });

  it("disables the listen toggle with a note when the role is assistant", () => {
    const live = { role: "assistant" as const, code: null, assistants: [], assistant: { connected: true, mainName: "desk", error: null } };
    const el = renderSettings({ ...voiceBase, listen: true, networkRole: "assistant", network: live }, handlers());
    const listen = el.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(listen.disabled).toBe(true);
    expect(listen.checked).toBe(false);
    expect(el.textContent).toContain("The main Maya notifies and listens for this machine.");
    const off = renderSettings({ ...voiceBase, listen: true, networkRole: "off" }, handlers());
    expect(off.querySelector<HTMLInputElement>("input[name=listen]")!.disabled).toBe(false);
    expect(off.textContent).not.toContain("The main Maya notifies and listens for this machine.");
    // Previewing Assistant before pairing changes nothing yet: the live role is still off.
    const preview = renderSettings({ ...voiceBase, listen: true, networkRole: "assistant", network: { ...live, role: "off" } }, handlers());
    expect(preview.querySelector<HTMLInputElement>("input[name=listen]")!.disabled).toBe(false);
    expect(preview.querySelector<HTMLInputElement>("input[name=listen]")!.checked).toBe(true);
    expect(preview.textContent).not.toContain("The main Maya notifies and listens for this machine.");
  });
});

describe("Make main", () => {
  const laptop = { id: "g", name: "laptop", hostname: "laptop", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, canLead: true, canFollow: true };
  const cli = { ...laptop, id: "c", name: "cli", canLead: false };
  const nas = { ...laptop, id: "n", name: "nas", connected: false, canLead: false, canFollow: false };
  const network = { role: "main" as const, code: null, assistants: [laptop, cli, nas], assistant: { connected: false, mainName: null, error: null } };

  it("shows only for a connected assistant that can lead", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network }, h);
    const buttons = [...el.querySelectorAll<HTMLButtonElement>("button[data-action=make-main]")];
    expect(buttons.map((b) => b.dataset.id)).toEqual(["g"]);
    buttons[0].click();
    expect(h.onMakeMain).toHaveBeenCalledWith("g");
    expect(el.querySelector("[data-for=promote-confirm]")).toBeNull();
  });

  it("confirms with who stays behind before handing over", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network, networkPromote: "g" }, h);
    const confirm = el.querySelector<HTMLElement>("[data-for=promote-confirm]")!;
    expect(confirm.textContent).toContain("Make laptop the main Maya? Every connected Maya will assist laptop");
    expect(confirm.textContent).toContain("nas is offline; when it is back, pair it with laptop.");
    confirm.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.click();
    expect(h.onPromote).toHaveBeenCalledWith("g");
    confirm.querySelector<HTMLButtonElement>("button[data-action=promote-cancel]")!.click();
    expect(h.onPromoteCancel).toHaveBeenCalled();
  });

  it("disables the buttons while the handover runs", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network, networkPromote: "g", networkPromoting: true }, handlers());
    expect(el.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.disabled).toBe(true);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=promote-confirm]")!.textContent).toBe("Handing over…");
    expect([...el.querySelectorAll<HTMLButtonElement>("button[data-action=remove-assistant]")].every((b) => b.disabled)).toBe(true);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=regenerate-code]")!.disabled).toBe(true);
    expect(el.querySelector<HTMLSelectElement>("select[name=networkRole]")!.disabled).toBe(true);
    expect(el.querySelector<HTMLInputElement>("input[name=networkPort]")!.disabled).toBe(true);
    expect(el.querySelector<HTMLInputElement>("input[name=networkName]")!.disabled).toBe(true);
  });

  it("disables the assistant's fields while a handover runs", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "assistant", network: { ...network, role: "assistant" }, networkPaired: true, networkPromoting: true }, handlers());
    expect(el.querySelector<HTMLInputElement>("input[name=networkHost]")!.disabled).toBe(true);
    expect(el.querySelector<HTMLInputElement>("input[name=networkMainPort]")!.disabled).toBe(true);
  });
});

describe("nextNetworkRole", () => {
  const live = (role: "off" | "main" | "assistant") => ({ role, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } });
  it("keeps an Assistant preview on a main", () => {
    expect(nextNetworkRole(live("main"), "assistant", live("main"))).toBe("assistant");
  });
  it("follows a live assistant that became the main", () => {
    expect(nextNetworkRole(live("assistant"), "assistant", live("main"))).toBe("main");
  });
  it("follows a main that became an assistant", () => {
    expect(nextNetworkRole(live("main"), "main", live("assistant"))).toBe("assistant");
  });
});

describe("check for updates", () => {
  it("offers the button beside the version only where Maya can update", () => {
    const h = { ...handlers(), onCheckUpdates: vi.fn() };
    const off = renderSettings({ ...voiceBase, version: "0.18.0" }, h);
    expect(off.querySelector("[data-action=check-updates]")).toBeNull();
    const on = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: true }, h);
    on.querySelector<HTMLButtonElement>("[data-action=check-updates]")!.click();
    expect(h.onCheckUpdates).toHaveBeenCalled();
  });

  it("shows the outcome of the last check", () => {
    const el = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: true, updateStatus: "Up to date." }, { ...handlers(), onCheckUpdates: vi.fn() });
    expect(el.querySelector(".settings__update-status")?.textContent).toBe("Up to date.");
  });
});

describe("the mirror switch", () => {
  const phone = { id: "p", name: "Pixel", hostname: "localhost", platform: "android", address: "192.168.1.30", connected: true, lastSeen: null, canLead: true, canFollow: true, mirror: true, wantsMirror: true };
  const laptop = { id: "g", name: "laptop", hostname: "laptop", platform: "macos", address: "192.168.55.70", connected: true, lastSeen: null, mirror: false, wantsMirror: false };
  const network = { role: "main" as const, code: null, assistants: [phone, laptop], assistant: { connected: false, mainName: null, error: null } };

  it("shows for a Maya that asked to mirror, says Android, and saves on change", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network }, h);
    const boxes = [...el.querySelectorAll<HTMLInputElement>("input[data-action=mirror]")];
    expect(boxes.map((b) => b.dataset.id)).toEqual(["p"]);
    expect(boxes[0].checked).toBe(true);
    expect(boxes[0].parentElement!.textContent).toContain("Sees and controls all sessions");
    expect(el.textContent).toContain("Pixel (Android, 192.168.1.30)");
    boxes[0].checked = false;
    boxes[0].dispatchEvent(new Event("change"));
    expect(h.onMirror).toHaveBeenCalledWith("p", false);
  });
});

describe("secure pairing in Settings › Network", () => {
  const main = (extra: object = {}) => ({ role: "main" as const, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, ...extra });

  it("says the code is used up on a main", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: main({ codeUsedUp: true }) }, handlers());
    expect(el.textContent).toContain("Code used up after 3 wrong tries — Regenerate.");
    const fresh = renderSettings({ ...voiceBase, networkRole: "main", network: main() }, handlers());
    expect(fresh.textContent).not.toContain("Code used up");
  });

  it("tells an updated Maya to pair again", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: main({ pairAgain: true }) }, handlers());
    expect(el.querySelector("[data-for=pair-again-note]")?.textContent).toBe("This Maya was updated to secure pairing — pair again.");
  });

  it("offers New key, which asks before it drops every pairing", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: main() }, h);
    el.querySelector<HTMLButtonElement>("button[data-action=new-key]")!.click();
    expect(h.onNewKeyAsk).toHaveBeenCalled();
    expect(h.onNewKey).not.toHaveBeenCalled();
    const asking = renderSettings({ ...voiceBase, networkRole: "main", network: main(), networkNewKeyConfirm: true }, h);
    expect(asking.textContent).toContain("Drop every pairing and make a new key?");
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-confirm]")!.click();
    expect(h.onNewKey).toHaveBeenCalled();
    asking.querySelector<HTMLButtonElement>("button[data-action=new-key-cancel]")!.click();
    expect(h.onNewKeyCancel).toHaveBeenCalled();
    const off = renderSettings({ ...voiceBase }, h);
    expect(off.querySelector("button[data-action=new-key]")).toBeNull();
  });

  it("offers Forget main on a paired assistant only", () => {
    const h = handlers();
    const paired = renderSettings({ ...voiceBase, networkRole: "assistant", networkPaired: true, networkMainHost: "desk.local" }, h);
    paired.querySelector<HTMLButtonElement>("button[data-action=forget-main]")!.click();
    expect(h.onForgetMain).toHaveBeenCalled();
    const fresh = renderSettings({ ...voiceBase, networkRole: "assistant" }, h);
    expect(fresh.querySelector("button[data-action=forget-main]")).toBeNull();
  });
});

describe("Settings → Git", () => {
  const gitModel = (over: Partial<SettingsModel> = {}): SettingsModel => ({
    ...voiceBase,
    settingsGroup: "git",
    git: gitConnections(null),
    gitStatus: { github: { hasToken: false }, gitlab: { hasToken: false } },
    ...over,
  });
  const block = (root: HTMLElement, p: string) => root.querySelector<HTMLElement>(`.settings__git[data-provider=${p}]`)!;

  it("defaults to GitHub on reading GH_TOKEN and GitLab off reading GITLAB_TOKEN", () => {
    expect(gitConnections(undefined)).toEqual([
      { provider: "github", enabled: true, baseUrl: null, token: { kind: "env", name: "GH_TOKEN" }, clone: "ssh" },
      { provider: "gitlab", enabled: false, baseUrl: null, token: { kind: "env", name: "GITLAB_TOKEN" }, clone: "ssh" },
    ]);
    expect(gitConnections([{ provider: "gitlab", enabled: true, token: { kind: "stored" } }])[1]).toEqual({ provider: "gitlab", enabled: true, baseUrl: null, token: { kind: "stored" }, clone: "ssh" });
    expect(gitConnections([{ provider: "github", enabled: true, token: { kind: "stored" }, clone: "https" }])[0].clone).toBe("https");
  });

  it("shows a block per provider, its fields disabled while it is off", () => {
    const root = renderSettings(gitModel(), handlers());
    expect(block(root, "github").querySelector<HTMLInputElement>("input[name^=gitEnabled]")!.checked).toBe(true);
    expect(block(root, "gitlab").querySelector<HTMLInputElement>("input[name^=gitEnabled]")!.checked).toBe(false);
    expect(block(root, "gitlab").querySelector<HTMLInputElement>("input[name^=gitBaseUrl]")!.disabled).toBe(true);
    expect(block(root, "gitlab").querySelector<HTMLInputElement>("input[name^=gitBaseUrl]")!.placeholder).toBe("https://gitlab.com");
    expect(block(root, "github").querySelector("input[name^=gitBaseUrl]")).toBeNull();
  });

  it("offers SSH or HTTPS to clone with, SSH by default, off with its provider", () => {
    const h = { ...handlers(), onGitClone: vi.fn() };
    const root = renderSettings(gitModel(), h);
    const radio = (p: string, v: string) => block(root, p).querySelector<HTMLInputElement>(`input[name=gitClone-${p}][value=${v}]`)!;
    expect([radio("github", "ssh").checked, radio("github", "https").checked]).toEqual([true, false]);
    expect(radio("github", "ssh").disabled).toBe(false);
    expect(radio("gitlab", "ssh").disabled && radio("gitlab", "https").disabled).toBe(true);
    expect(block(root, "github").textContent).toContain("Clone with");
    expect(block(root, "github").textContent).toContain("HTTPS");
    const on = gitModel({ git: gitConnections([{ provider: "gitlab", enabled: true, token: { kind: "stored" }, clone: "https" }]) });
    const root2 = renderSettings(on, h);
    expect(block(root2, "gitlab").querySelector<HTMLInputElement>("input[name=gitClone-gitlab][value=https]")!.checked).toBe(true);
    const https = block(root2, "gitlab").querySelector<HTMLInputElement>("input[name=gitClone-gitlab][value=https]")!;
    https.dispatchEvent(new Event("change"));
    expect(h.onGitClone).toHaveBeenCalledWith("gitlab", "https");
  });

  it("shows the variable field for env and the token field for stored, never filled", () => {
    const stored = gitConnections([{ provider: "github", enabled: true, token: { kind: "stored" } }]);
    const root = renderSettings(gitModel({ git: stored, gitStatus: { github: { hasToken: true }, gitlab: { hasToken: false } } }), handlers());
    const gh = block(root, "github");
    const tokenField = gh.querySelector<HTMLInputElement>("input[name=gitToken-github]")!;
    expect(tokenField.type).toBe("password");
    expect(tokenField.value).toBe("");
    expect(gh.querySelector("input[name^=gitEnvName]")).toBeNull();
    expect(gh.querySelector("[data-action=git-clear]")).not.toBeNull();
    const gl = block(root, "gitlab");
    expect(gl.querySelector<HTMLInputElement>("input[name^=gitEnvName]")!.value).toBe("GITLAB_TOKEN");
    expect(gl.querySelector("input[name=gitToken-gitlab]")).toBeNull();
  });

  it("calls a handler for each change", () => {
    const h = { ...handlers(), onGitEnabled: vi.fn(), onGitTokenKind: vi.fn(), onGitEnvName: vi.fn(), onGitBaseUrl: vi.fn(), onGitToken: vi.fn(), onGitClearToken: vi.fn(), onGitTest: vi.fn() };
    const stored = gitConnections([{ provider: "github", enabled: true, token: { kind: "stored" } }, { provider: "gitlab", enabled: true, token: { kind: "env", name: "GITLAB_TOKEN" } }]);
    const root = renderSettings(gitModel({ git: stored, gitStatus: { github: { hasToken: true }, gitlab: { hasToken: false } } }), h);
    const gl = block(root, "gitlab");
    const on = gl.querySelector<HTMLInputElement>("input[name^=gitEnabled]")!;
    on.checked = false;
    on.dispatchEvent(new Event("change"));
    expect(h.onGitEnabled).toHaveBeenCalledWith("gitlab", false);
    const url = gl.querySelector<HTMLInputElement>("input[name^=gitBaseUrl]")!;
    url.value = "  https://gitlab.acme.io/ ";
    url.dispatchEvent(new Event("change"));
    expect(h.onGitBaseUrl).toHaveBeenCalledWith("https://gitlab.acme.io/");
    const env = gl.querySelector<HTMLInputElement>("input[name^=gitEnvName]")!;
    env.value = " MY_TOKEN ";
    env.dispatchEvent(new Event("change"));
    expect(h.onGitEnvName).toHaveBeenCalledWith("gitlab", "MY_TOKEN");
    gl.querySelector<HTMLInputElement>("input[name=gitTokenKind-gitlab][value=stored]")!.dispatchEvent(new Event("change"));
    expect(h.onGitTokenKind).toHaveBeenCalledWith("gitlab", "stored");
    const gh = block(root, "github");
    const tok = gh.querySelector<HTMLInputElement>("input[name=gitToken-github]")!;
    tok.value = " ghp_new ";
    tok.dispatchEvent(new Event("change"));
    expect(h.onGitToken).toHaveBeenCalledWith("github", "ghp_new");
    expect(tok.value).toBe(""); // emptied at once
    gh.querySelector<HTMLButtonElement>("[data-action=git-clear]")!.click();
    expect(h.onGitClearToken).toHaveBeenCalledWith("github");
    gh.querySelector<HTMLButtonElement>("[data-action=git-test]")!.click();
    expect(h.onGitTest).toHaveBeenCalledWith("github");
  });

  it("shows the last test's outcome", () => {
    const root = renderSettings(gitModel({ gitStatus: { github: { hasToken: false, test: "Connected as @tiago" }, gitlab: { hasToken: false } } }), handlers());
    expect(block(root, "github").querySelector(".settings__git-test")?.textContent).toBe("Connected as @tiago");
  });
});

describe("colour theme", () => {
  it("offers Dark and Light under General and reports the choice", () => {
    const h = { ...handlers(), onColorTheme: vi.fn() };
    const el = renderSettings({ ...voiceBase, colorTheme: "dark" }, h);
    const general = el.querySelector(".settings__section--general")!;
    expect(general.textContent).toContain("Theme");
    const sel = general.querySelector<HTMLSelectElement>("select[name=colorTheme]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Dark", "Light"]);
    expect(sel.value).toBe("dark");
    sel.value = "light";
    sel.dispatchEvent(new Event("change"));
    expect(h.onColorTheme).toHaveBeenCalledWith("light");
  });

  it("shows the saved theme, and Dark when none was saved", () => {
    const pick = (model: object) => renderSettings({ ...voiceBase, ...model }, handlers()).querySelector<HTMLSelectElement>("select[name=colorTheme]")!.value;
    expect(pick({ colorTheme: "light" })).toBe("light");
    expect(pick({})).toBe("dark");
  });
});

describe("keyboard", () => {
  const keys = { send: ["Enter"], newLine: ["Alt+Enter", "Ctrl+Enter"] };
  const open = (model: object = { keys }) => {
    const h = { ...handlers(), onKeys: vi.fn() };
    const el = renderSettings({ ...voiceBase, ...model }, h);
    const row = (action: string) => el.querySelector<HTMLElement>(`.settings__keys[data-keys=${action}]`)!;
    const chips = (action: string) => [...row(action).querySelectorAll(".chip")].map((c) => c.firstChild?.textContent);
    return { h, el, row, chips };
  };
  const press = (target: HTMLElement, init: KeyboardEventInit) => {
    const ev = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(ev);
    return ev;
  };

  it("lists the combinations of Send and New line under Keyboard", () => {
    const { el, chips } = open();
    const section = el.querySelector(".settings__section--keyboard")!;
    expect(section.textContent).toContain("Send");
    expect(section.textContent).toContain("New line");
    expect(chips("send")).toEqual([comboLabel("Enter")]);
    expect(chips("newLine")).toEqual([comboLabel("Alt+Enter"), comboLabel("Ctrl+Enter")]);
  });

  it("shows the defaults when no keys were saved", () => {
    const { chips } = open({});
    expect(chips("send")).toEqual(defaultKeys().send.map(comboLabel));
    expect(chips("newLine")).toEqual(defaultKeys().newLine.map(comboLabel));
  });

  it("removes a combination, and offers no remove on Send's last one", () => {
    const { h, row } = open();
    expect(row("send").querySelector(".chip__remove")).toBeNull();
    row("newLine").querySelectorAll<HTMLButtonElement>(".chip__remove")[0].click();
    expect(h.onKeys).toHaveBeenCalledWith({ send: ["Enter"], newLine: ["Ctrl+Enter"] });
  });

  it("records the next Enter press for an action, and ignores other keys while it waits", () => {
    const { h, row } = open();
    const add = row("send").querySelector<HTMLButtonElement>("button[data-action=keys-add]")!;
    add.click();
    expect(add.textContent).toBe("Press keys… (Esc to cancel)");
    press(add, { key: "Meta", metaKey: true });
    press(add, { key: "k", metaKey: true });
    expect(h.onKeys).not.toHaveBeenCalled();
    const ev = press(add, { key: "Enter", metaKey: true });
    expect(ev.defaultPrevented).toBe(true);
    expect(h.onKeys).toHaveBeenCalledWith({ send: ["Enter", "Meta+Enter"], newLine: ["Alt+Enter", "Ctrl+Enter"] });
  });

  it("does nothing with an Enter press on Add before it was clicked", () => {
    const { h, row } = open();
    const add = row("send").querySelector<HTMLButtonElement>("button[data-action=keys-add]")!;
    press(add, { key: "Enter", metaKey: true });
    expect(h.onKeys).not.toHaveBeenCalled();
  });

  it("stops recording on Esc, and when Add loses focus", () => {
    const { h, row } = open();
    const add = row("newLine").querySelector<HTMLButtonElement>("button[data-action=keys-add]")!;
    add.click();
    press(add, { key: "Escape" });
    expect(add.textContent).toBe("Add");
    press(add, { key: "Enter", shiftKey: true });
    add.click();
    add.dispatchEvent(new FocusEvent("blur"));
    expect(add.textContent).toBe("Add");
    press(add, { key: "Enter", shiftKey: true });
    expect(h.onKeys).not.toHaveBeenCalled();
  });

  it("refuses a combination the other action has, and says which", () => {
    const { h, row } = open();
    const add = row("send").querySelector<HTMLButtonElement>("button[data-action=keys-add]")!;
    add.click();
    press(add, { key: "Enter", altKey: true });
    expect(h.onKeys).not.toHaveBeenCalled();
    expect(row("send").querySelector(".settings__error")?.textContent).toBe(`New line already uses ${comboLabel("Alt+Enter")}.`);
  });

  it("resets to the defaults", () => {
    const { h, el } = open({ keys: { send: ["Meta+Enter"], newLine: [] } });
    el.querySelector<HTMLButtonElement>("button[data-action=keys-reset]")!.click();
    expect(h.onKeys).toHaveBeenCalledWith(defaultKeys());
  });
});

describe("where new sessions open", () => {
  const original = navigator.userAgent;
  const pretend = (ua: string) => Object.defineProperty(navigator, "userAgent", { value: ua, configurable: true });
  const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";

  afterEach(() => pretend(original));

  it("offers Terminal or iTerm2 under Sessions on a Mac and saves the choice", () => {
    pretend(MAC);
    const h = { ...handlers(), onNewSessionTerminal: vi.fn() };
    const el = renderSettings({ ...voiceBase, newSessionTerminal: "terminal" }, h);
    const sessions = el.querySelector(".settings__section--sessions")!;
    const sel = sessions.querySelector<HTMLSelectElement>("select[name=newSessionTerminal]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Terminal", "iTerm2"]);
    expect(sel.value).toBe("terminal");
    sel.value = "iterm";
    sel.dispatchEvent(new Event("change"));
    expect(h.onNewSessionTerminal).toHaveBeenCalledWith("iterm");
  });

  it("shows the saved choice and defaults to Terminal", () => {
    pretend(MAC);
    const chosen = renderSettings({ ...voiceBase, newSessionTerminal: "iterm" }, handlers());
    expect(chosen.querySelector<HTMLSelectElement>("select[name=newSessionTerminal]")!.value).toBe("iterm");
    const fresh = renderSettings({ ...voiceBase }, handlers());
    expect(fresh.querySelector<HTMLSelectElement>("select[name=newSessionTerminal]")!.value).toBe("terminal");
  });

  it("is not offered on Windows or Linux, where there is no iTerm2", () => {
    pretend("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0");
    expect(renderSettings({ ...voiceBase }, handlers()).querySelector("select[name=newSessionTerminal]")).toBeNull();
    pretend("Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko)");
    expect(renderSettings({ ...voiceBase }, handlers()).querySelector("select[name=newSessionTerminal]")).toBeNull();
  });
});

describe("session viewer", () => {
  it("offers the viewer's look under Sessions and saves the choice", () => {
    const h = { ...handlers(), onViewerTheme: vi.fn() };
    const el = renderSettings({ ...voiceBase, viewerTheme: "chat" }, h);
    const sessions = el.querySelector(".settings__section--sessions")!;
    expect(sessions.textContent).toContain("Session viewer");
    const sel = sessions.querySelector<HTMLSelectElement>("select[name=viewerTheme]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Chat", "Terminal"]);
    expect(sel.value).toBe("chat");
    sel.value = "terminal";
    sel.dispatchEvent(new Event("change"));
    expect(h.onViewerTheme).toHaveBeenCalledWith("terminal");
  });
});
