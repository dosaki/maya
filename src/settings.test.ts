import { describe, expect, it, vi } from "vitest";
import { pairingRepaintDue, renderSettings, tickPairingCode, type SettingsModel } from "./settings";

const voiceBase = {
  hookInstalled: true,
  completedTimeoutMinutes: 30,
  projectsDir: "",
  clonesDir: "",
  notifyOnAwaiting: true,
  speakNotifications: true,
  voiceProvider: "builtin" as const,
  elevenKeySet: false,
  elevenVoices: [],
  elevenVoiceId: "",
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
  onInstall: vi.fn(),
  onRemove: vi.fn(),
  onTimeout: vi.fn(),
  onProjectsDir: vi.fn(),
  onNotify: vi.fn(),
  onClonesDir: vi.fn(),
  onSpeak: vi.fn(),
  onVoiceProvider: vi.fn(),
  onElevenKey: vi.fn(),
  onElevenVoice: vi.fn(),
  onTryVoice: vi.fn(),
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
    expect(el.querySelector(".settings__section .settings__heading")?.textContent).toBe("Maya");
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
    const el = renderSettings({ hookInstalled: false, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("not installed");
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=install]")!;
    btn.click();
    expect(h.onInstall).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=remove]")).toBeNull();
  });

  it("offers remove when the hook is installed", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("installed");
    el.querySelector<HTMLButtonElement>("button[data-action=remove]")!.click();
    expect(h.onRemove).toHaveBeenCalled();
  });

  it("reports timeout changes and shows errors", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: null, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: "boom", listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=timeout]")!;
    expect(input.value).toBe("30");
    input.value = "45";
    input.dispatchEvent(new Event("change"));
    expect(h.onTimeout).toHaveBeenCalledWith(45);
    expect(el.querySelector(".settings__error")?.textContent).toBe("boom");
  });

  it("has a notification toggle that reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=notify]")!;
    expect(box.type).toBe("checkbox");
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("awaits a decision");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onNotify).toHaveBeenCalledWith(false);
    expect(renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: false, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h).querySelector<HTMLInputElement>("input[name=notify]")!.checked).toBe(false);
  });

  it("offers ElevenLabs as a voice, revealing key, voice list and Try when chosen", () => {
    const h = handlers();
    const builtin = renderSettings({ ...voiceBase, voiceProvider: "builtin" }, h);
    const provider = builtin.querySelector<HTMLSelectElement>("select[name=voiceProvider]")!;
    expect([...provider.options].map((o) => o.value)).toEqual(["builtin", "elevenlabs"]);
    expect(builtin.querySelector("input[name=elevenKey]")).toBeNull();
    provider.value = "elevenlabs";
    provider.dispatchEvent(new Event("change"));
    expect(h.onVoiceProvider).toHaveBeenCalledWith("elevenlabs");
    const el = renderSettings({ ...voiceBase, voiceProvider: "elevenlabs", elevenKeySet: false, elevenVoices: [{ voiceId: "abc", name: "Rachel" }, { voiceId: "def", name: "Bella" }], elevenVoiceId: "def" }, h);
    const key = el.querySelector<HTMLInputElement>("input[name=elevenKey]")!;
    expect(key.type).toBe("password");
    key.value = " sk-123 ";
    key.dispatchEvent(new Event("change"));
    expect(h.onElevenKey).toHaveBeenCalledWith("sk-123");
    const voice = el.querySelector<HTMLSelectElement>("select[name=elevenVoice]")!;
    expect([...voice.options].map((o) => o.textContent)).toEqual(["Rachel", "Bella"]);
    expect(voice.value).toBe("def");
    voice.value = "abc";
    voice.dispatchEvent(new Event("change"));
    expect(h.onElevenVoice).toHaveBeenCalledWith("abc");
    el.querySelector<HTMLButtonElement>("button[data-action=try-voice]")!.click();
    expect(h.onTryVoice).toHaveBeenCalled();
    const saved = renderSettings({ ...voiceBase, voiceProvider: "elevenlabs", elevenKeySet: true, elevenVoices: [], elevenVoiceId: "" }, h);
    expect(saved.querySelector<HTMLInputElement>("input[name=elevenKey]")!.placeholder).toContain("saved");
  });

  it("has a speak toggle that reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=speak]")!;
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("Speak");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onSpeak).toHaveBeenCalledWith(false);
  });

  it("shows the clones directory with the default as placeholder and reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=clonesDir]")!;
    expect(input.placeholder).toBe("~/dev/reviews");
    expect(input.value).toBe("");
    input.value = " ~/tmp/reviews ";
    input.dispatchEvent(new Event("change"));
    expect(h.onClonesDir).toHaveBeenCalledWith("~/tmp/reviews");
  });

  it("shows the projects directory and reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
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

  it("groups the fields into titled sections", () => {
    const el = renderSettings(voiceBase, handlers());
    expect([...el.querySelectorAll(".settings__heading")].map((x) => x.textContent)).toEqual(["Maya", "Sessions", "Notifications", "Voice assistant", "Network"]);
    expect(el.querySelector(".settings__section input[name=listen]")).not.toBeNull();
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
