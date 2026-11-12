import { afterEach, describe, expect, it, vi } from "vitest";
import { comboLabel, defaultKeys } from "./keys";
import { RUNNING_VERSION } from "./release-notes";
import { SETTINGS_GROUPS, SETTINGS_LEDES, SETTINGS_NAV, SOURCES_OPEN_KEY, forgeSummary, gitConnections, ruleSummary, storedSourcesOpen, workItemsConfig, nextNetworkRole, pairingRepaintDue, renderSettings, SETTINGS_GROUP_KEY, storedSettingsGroup, terminalChoices, tickAddresses, tickPairingCode, type SettingsModel } from "./settings";
import { builtinVoiceName, secretStore } from "./platform";
import styles from "./styles.css?raw";
import mobileStyles from "./mobile/mobile.css?raw";
import popoverMain from "./fixtures/popover-rules-main.json";
import { renderVoicePanel } from "./voice";
import artSrc from "./assets/maya-full-body.png";

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
  onRemoveMember: vi.fn(),
  onJoinAsk: vi.fn(),
  onJoin: vi.fn(),
  onJoinCancel: vi.fn(),
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
    expect(el.querySelector(".settings__section .settings__title")?.textContent).toBe("General");
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
        .map((b) => b.getAttribute("aria-label"))
        .filter((t) => t?.includes("hook"));
    expect(labels(false, false)).toEqual(["Install Claude hook", "Install Codex hook"]);
    expect(labels(true, true)).toEqual(["Remove Claude hook", "Remove Codex hook"]);
  });

  it("the status line block offers install, remove or says what blocks it", () => {
    const h = { ...handlers(), onStatuslineInstall: vi.fn(), onStatuslineRemove: vi.fn() };
    const render = (statusline: SettingsModel["statusline"]) => renderSettings({ ...voiceBase, statusline }, h);
    const texts = (el: HTMLElement) => [...el.querySelectorAll(".settings__help")].map((s) => s.textContent);

    const checking = render(null);
    expect(texts(checking)).toContain("Checking status line…");

    const missing = render({ installed: false });
    expect(texts(missing)).toContain("Usage and limits are not recorded. Install the status line wrapper to see them in session windows.");
    const install = missing.querySelector<HTMLButtonElement>("button[data-action=install-statusline]")!;
    expect(install.getAttribute("aria-label")).toBe("Install usage status line");
    expect(install.textContent).toBe("Install");
    install.click();
    expect(h.onStatuslineInstall).toHaveBeenCalledOnce();

    const installed = render({ installed: true });
    expect(texts(installed)).toContain("Usage and limits are recorded from Claude Code's status line.");
    const remove = installed.querySelector<HTMLButtonElement>("button[data-action=remove-statusline]")!;
    expect(remove.getAttribute("aria-label")).toBe("Remove usage status line");
    expect(remove.textContent).toBe("Remove");
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
    expect([...el.querySelectorAll(".settings__help")].find((x) => x.textContent?.startsWith("Claude Code hook"))?.textContent).toContain("not installed");
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=install]")!;
    btn.click();
    expect(h.onInstall).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=remove]")).toBeNull();
  });

  it("offers remove when the hook is installed", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, elevenEnabled: false, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", serverEnabled: false, serverUrl: "", serverVoice: "", serverModel: "", serverKeySet: false, serverVoices: [], serverVoicesError: null, voiceReport: null, error: null, listen: false, microphone: "", microphones: [], agent: null, agentModel: "", agents: [], reviewPrompt: "", listenError: null, recognizer: "system" as const, whisperModel: "base.en-q5_1", models: [], downloading: null }, h);
    expect([...el.querySelectorAll(".settings__help")].find((x) => x.textContent?.startsWith("Claude Code hook"))?.textContent).toContain("installed");
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

  describe("Sessions and Pull requests screens", () => {
    const rowOf = (root: ParentNode, label: string) =>
      [...root.querySelectorAll<HTMLElement>(".settings__row")].find((r) => r.querySelector(".settings__label")?.textContent === label)!;
    const titles = (sec: Element) => [...sec.querySelectorAll(".settings__group-title")].map((t) => t.textContent);

    it("Sessions has Session tracking, Board and New sessions groups", () => {
      const sec = renderSettings(voiceBase, handlers()).querySelector(".settings__section--sessions")!;
      expect(titles(sec)).toEqual(["Session tracking", "Board", "New sessions"]);
      expect(sec.querySelector(".settings__group-note")?.textContent).toBe("Hooks let the agents tell Maya what they are doing, rather than her guessing from transcripts.");
      const tracking = sec.querySelector(".settings__group")!;
      expect([...tracking.querySelectorAll(".settings__row .settings__label")].map((l) => l.textContent)).toEqual(["Claude Code hook", "Usage status line", "Codex hook"]);
    });

    it("each hook row shows Installed or Not installed beside its button", () => {
      const render = (hookInstalled: boolean, codexHookInstalled: boolean, statusline: SettingsModel["statusline"]) =>
        renderSettings({ ...voiceBase, hookInstalled, codexHookInstalled, statusline }, handlers());
      const on = render(true, true, { installed: true });
      for (const [label, action] of [["Claude Code hook", "remove"], ["Usage status line", "remove-statusline"], ["Codex hook", "remove-codex"]] as const) {
        const r = rowOf(on, label);
        expect(r.querySelector(".settings__state")?.className).toContain("settings__state--on");
        expect(r.querySelector(".settings__state")?.textContent).toBe("Installed");
        expect(r.querySelector<HTMLButtonElement>(`button[data-action=${action}]`)?.textContent).toBe("Remove");
      }
      const off = render(false, false, { installed: false });
      for (const [label, action] of [["Claude Code hook", "install"], ["Usage status line", "install-statusline"], ["Codex hook", "install-codex"]] as const) {
        const r = rowOf(off, label);
        expect(r.querySelector(".settings__state")?.className).toContain("settings__state--off");
        expect(r.querySelector(".settings__state")?.textContent).toBe("Not installed");
        expect(r.querySelector<HTMLButtonElement>(`button[data-action=${action}]`)?.textContent).toBe("Install");
      }
    });

    it("keeps the full button text as the accessible name and the status sentence as help", () => {
      const el = renderSettings({ ...voiceBase, hookInstalled: false, codexHookInstalled: true }, handlers());
      expect(rowOf(el, "Claude Code hook").querySelector("button")!.getAttribute("aria-label")).toBe("Install Claude hook");
      expect(rowOf(el, "Claude Code hook").querySelector(".settings__help")?.textContent).toBe("Claude Code hook is not installed. Permission prompts will show as Working.");
      expect(rowOf(el, "Codex hook").querySelector("button")!.getAttribute("aria-label")).toBe("Remove Codex hook");
    });

    it("a blocked status line shows the blocked-by sentence as its help and no Installed state", () => {
      const el = renderSettings({ ...voiceBase, statusline: { installed: false, blockedBy: "/etc/x.json" } }, handlers());
      const r = rowOf(el, "Usage status line");
      expect(r.querySelector(".settings__help")?.textContent).toBe("Usage and limits cannot be recorded: the status line is set in /etc/x.json.");
      expect(r.querySelector(".settings__state")?.textContent).toBe("Not installed");
    });

    it("Board puts the unit 'minutes' after the timeout box", () => {
      const el = renderSettings(voiceBase, handlers());
      const input = el.querySelector<HTMLInputElement>("input[name=timeout]")!;
      expect(input.nextElementSibling?.tagName).toBe("SPAN");
      expect(input.nextElementSibling?.textContent).toBe("minutes");
      expect(input.closest(".settings__row")?.querySelector(".settings__label")?.textContent).toBe("Completed decays to Idle after");
      expect(rowOf(el, "Session viewer").querySelector("select[name=viewerTheme]")).not.toBeNull();
    });

    it("New sessions has Projects directory and Open in rows", () => {
      const el = renderSettings(voiceBase, handlers());
      expect(rowOf(el, "Projects directory").querySelector("input[name=projectsDir]")).not.toBeNull();
      expect(rowOf(el, "Open in").querySelector("select[name=newSessionTerminal]")).not.toBeNull();
    });

    it("Pull requests has Reviews and Your pull requests groups", () => {
      const sec = renderSettings(voiceBase, handlers()).querySelector(".settings__section--reviews")!;
      expect(titles(sec)).toEqual(["Reviews", "Your pull requests"]);
      const stack = sec.querySelector("textarea[name=reviewPrompt]")!.closest(".settings__row--stack")!;
      expect(stack).not.toBeNull();
      expect(sec.querySelector<HTMLTextAreaElement>("textarea[name=reviewPrompt]")!.rows).toBe(7);
      expect(stack.querySelector(".settings__help")?.textContent).toContain("Blank uses the built-in prompt.");
      expect(rowOf(sec, "Clones directory").querySelector(".settings__help")?.textContent).toBe("For reviews and worktrees.");
      expect(rowOf(sec, "Clones directory").querySelector("input[name=clonesDir]")).not.toBeNull();
      expect(sec.querySelector("input[name=notifyPrReview]")?.classList.contains("settings__toggle")).toBe(true);
    });
  });

  it("shows the voice order and a switch per remote voice", () => {
    const h = handlers();
    const el = renderSettings(voiceBase, h);
    expect(el.querySelector("[data-for=voice-order]")!.textContent).toBe("Maya speaks with the first voice that works, in this order.");
    expect(el.querySelector("[data-for=voice-order]")!.classList.contains("settings__group-note")).toBe(true);
    expect(el.querySelector("select[name=voiceProvider]")).toBeNull();
    const eleven = el.querySelector<HTMLInputElement>("input[name=elevenEnabled]")!;
    const server = el.querySelector<HTMLInputElement>("input[name=serverEnabled]")!;
    expect(eleven.checked).toBe(false);
    expect(server.checked).toBe(false);
    expect(eleven.classList.contains("settings__toggle")).toBe(true);
    expect(server.classList.contains("settings__toggle")).toBe(true);
    expect(eleven.closest(".settings__row--head")?.textContent).toContain("ElevenLabs");
    expect(server.closest(".settings__row--head")?.textContent).toContain("Voice server");
    expect(el.querySelector("input[name=elevenKey]")).toBeNull();
    expect(el.querySelector("input[name=serverUrl]")).toBeNull();
    eleven.checked = true;
    eleven.dispatchEvent(new Event("change"));
    expect(h.onElevenEnabled).toHaveBeenCalledWith(true);
    server.checked = true;
    server.dispatchEvent(new Event("change"));
    expect(h.onServerEnabled).toHaveBeenCalledWith(true);
  });

  describe("Voice screen", () => {
    const voice = (m: object = {}) => renderSettings({ ...voiceBase, ...m }, handlers()).querySelector<HTMLElement>(".settings__section--voice")!;
    const rowOf = (root: ParentNode, label: string) =>
      [...root.querySelectorAll<HTMLElement>(".settings__row")].find((r) => r.querySelector(".settings__label")?.textContent === label)!;

    it("has Speaking and Listening groups", () => {
      const sec = voice();
      expect([...sec.querySelectorAll(".settings__group-title")].map((x) => x.textContent)).toEqual(["Speaking", "Listening"]);
      const [speaking, listening] = [...sec.querySelectorAll(".settings__group-title")];
      const follows = (a: Element, b: Element) => !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
      expect(follows(speaking, sec.querySelector("input[name=elevenEnabled]")!)).toBe(true);
      expect(follows(sec.querySelector("button[data-action=try-voice]")!, listening)).toBe(true);
      expect(follows(listening, sec.querySelector("input[name=listen]")!)).toBe(true);
    });

    it("has three numbered voice cards with their heads", () => {
      const sec = voice();
      const cards = [...sec.querySelectorAll<HTMLElement>(".settings__group")[0]!.querySelectorAll<HTMLElement>(".settings__card")];
      expect(cards).toHaveLength(4);
      const heads = cards.slice(0, 3).map((c) => c.querySelector<HTMLElement>(".settings__row--head")!);
      expect(heads.map((h) => h.querySelector(".settings__order")?.textContent)).toEqual(["1", "2", "3"]);
      expect(heads.map((h) => h.querySelector(".settings__label")?.textContent)).toEqual(["ElevenLabs", "Voice server", builtinVoiceName()]);
      expect(heads.map((h) => h.querySelector(".settings__help")?.textContent ?? null)).toEqual([null, "Any OpenAI-compatible speech server.", "Built in; always there."]);
      expect(heads[0]!.querySelector("input[name=elevenEnabled]")).not.toBeNull();
      expect(heads[1]!.querySelector("input[name=serverEnabled]")).not.toBeNull();
      expect(heads[2]!.querySelector("input")).toBeNull();
    });

    it("marks a card whose switch is off, and keeps the fields of one that is on", () => {
      const off = voice();
      const offCards = [...off.querySelectorAll(".settings__card")];
      expect(offCards.map((c) => c.classList.contains("settings__card--off"))).toEqual([true, true, false, false, false]);
      const on = voice({ elevenEnabled: true, serverEnabled: true, elevenVoices: [{ voiceId: "a", name: "Rachel" }] });
      const onCards = [...on.querySelectorAll(".settings__card")];
      expect(onCards.map((c) => c.classList.contains("settings__card--off"))).toEqual([false, false, false, false, false]);
      expect(rowOf(on, "API key").querySelector("input[name=elevenKey]")).not.toBeNull();
      expect(rowOf(on, "Voice").querySelector("select[name=elevenVoice]")).not.toBeNull();
      expect(rowOf(on, "Server address").querySelector("input[name=serverUrl]")?.classList.contains("mono")).toBe(true);
      expect(rowOf(on, "Model").querySelector("input[name=serverModel]")).not.toBeNull();
    });

    it("says where a saved key is kept in the key row's help", () => {
      const sec = voice({ elevenEnabled: true, elevenKeySet: true, serverEnabled: true, serverKeySet: true });
      const helps = [...sec.querySelectorAll(".settings__row")].filter((r) => r.querySelector("input[type=password]")).map((r) => r.querySelector(".settings__help")?.textContent);
      expect(helps).toEqual([`Saved in ${secretStore()}.`, `Saved in ${secretStore()}.`]);
      expect(rowOf(voice({ elevenEnabled: true }), "API key").querySelector(".settings__help")).toBeNull();
    });

    it("puts the volume and the voice test in one card, the report in its foot", () => {
      const sec = voice({ voiceReport: "Spoke with the voice server." });
      const vol = sec.querySelector("input[name=voiceVolume]")!.closest<HTMLElement>(".settings__card")!;
      expect(rowOf(vol, "Volume")).toBeTruthy();
      const foot = vol.querySelector<HTMLElement>(".settings__row--foot")!;
      expect(foot.querySelector("button[data-action=try-voice]")).not.toBeNull();
      expect(foot.querySelector("[data-for=voice-report]")!.textContent).toBe("Spoke with the voice server.");
      expect(voice().querySelector("[data-for=voice-report]")).toBeNull();
      expect(voice().querySelector(".settings__card .settings__row--foot button[data-action=try-voice]")).not.toBeNull();
    });

    it("has Listening as one card of rows", () => {
      const sec = voice({ recognizer: "builtin", models: [{ id: "base.en-q5_1", label: "Base", bytes: 1, downloaded: true }] });
      const listen = rowOf(sec, "Listen for the wake word");
      expect(listen.querySelector("input[name=listen]")?.classList.contains("settings__toggle")).toBe(true);
      expect(listen.querySelector(".settings__help")?.textContent).toBe("On-device speech recognition.");
      const card = listen.closest(".settings__card")!;
      expect(rowOf(card, "Wake word").querySelector("input[name=listenWord]")).not.toBeNull();
      expect(rowOf(card, "Wake word").querySelector(".settings__help")?.textContent).toBe("Blank is \"Maya\". A phone joined to this Maya listens for the same word. Pick a word the speech recognition spells the way you type it.");
      expect(rowOf(card, "Speech recognition").querySelector("select[name=recognizer]")).not.toBeNull();
      expect(rowOf(card, "Model").querySelector("select[name=whisperModel]")).not.toBeNull();
      expect(rowOf(card, "Microphone").querySelector("select[name=microphone]")).not.toBeNull();
    });

    it("puts the listen error and note in the Listening card's foot", () => {
      const failed = voice({ listenError: "Dictation is off." });
      const err = failed.querySelector<HTMLElement>(".settings__error[data-for=listen]")!;
      expect(err.closest(".settings__row--foot")).not.toBeNull();
      expect(err.closest(".settings__card")).toBe(failed.querySelector("input[name=listen]")!.closest(".settings__card"));
      const note = voice({ network: { role: "assistant" } as never }).querySelector<HTMLElement>(".settings__hint[data-for=listen]")!;
      expect(note.textContent).toBe("The main Maya notifies and listens for this machine.");
      expect(note.closest(".settings__row--foot")).not.toBeNull();
      expect(voice().querySelector(".settings__row--foot [data-for=listen]")).toBeNull();
    });

    it("styles the foot error in the danger colour", () => {
      expect(styles).toMatch(/\.settings\.pane \.settings__row--foot \.settings__error\s*\{[^}]*var\(--danger\)/);
    });
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
    expect(saved.querySelector<HTMLInputElement>("input[name=elevenKey]")!.placeholder).toBe("paste to replace");
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
    expect(key.placeholder).toBe("optional");
    key.value = " sk-abc ";
    key.dispatchEvent(new Event("change"));
    expect(h.onServerKey).toHaveBeenCalledWith("sk-abc");
    expect(key.value).toBe("");
    const saved = renderSettings({ ...voiceBase, serverEnabled: true, serverKeySet: true }, h);
    expect(saved.querySelector<HTMLInputElement>("input[name=serverKey]")!.placeholder).toBe("paste to replace");
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

  it("takes a wake word, and the listen switch keeps its fixed label", () => {
    const h = { ...handlers(), onListenWord: vi.fn() };
    const blank = renderSettings({ ...voiceBase }, h);
    const input = blank.querySelector<HTMLInputElement>("input[name=listenWord]")!;
    expect(input.placeholder).toBe("Maya");
    expect(input.value).toBe("");
    expect(blank.textContent).toContain("Listen for the wake word");
    input.value = "  Jarvis ";
    input.dispatchEvent(new Event("change"));
    expect(h.onListenWord).toHaveBeenCalledWith("Jarvis");
    const named = renderSettings({ ...voiceBase, listenWord: "Jarvis" }, h);
    expect(named.querySelector<HTMLInputElement>("input[name=listenWord]")!.value).toBe("Jarvis");
    expect(named.textContent).toContain("Listen for the wake word");
    expect(named.textContent).not.toContain("Jarvis\"");
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
    expect(err.closest(".settings__card")).toBe(failed.querySelector("input[name=listen]")!.closest(".settings__card"));
    expect(renderSettings({ ...voiceBase, listen: true, listenError: null }, h).querySelector(".settings__error[data-for=listen]")).toBeNull();
  });

  it("lists the groups in a sidebar, one titled section each, and shows only General at first", () => {
    const el = renderSettings(voiceBase, handlers());
    const nav = [...el.querySelectorAll<HTMLButtonElement>(".settings__nav button")];
    expect(nav.map((b) => b.textContent)).toEqual(["General", "Keyboard", "Sessions", "Git", "Pull requests", "Work Items", "Notifications", "Voice", "Network", "Devices", "Debug", "Release Notes"]);
    expect([...el.querySelectorAll(".settings__title")].map((x) => x.textContent)).toEqual(["General", "Keyboard", "Sessions", "Git", "Pull requests", "Work Items", "Notifications", "Voice", "Network", "Devices", "Debug", "Release Notes"]);
    const sections = [...el.querySelectorAll<HTMLElement>(".settings__section")];
    expect(sections.map((s) => s.dataset.group)).toEqual(["general", "keyboard", "sessions", "git", "reviews", "workItems", "notifications", "voice", "network", "devices", "debug", "releaseNotes"]);
    expect(sections.filter((s) => !s.hidden).map((s) => s.dataset.group)).toEqual(["general"]);
    expect(nav.map((b) => b.getAttribute("aria-current"))).toEqual(["true", null, null, null, null, null, null, null, null, null, null, null]);
  });

  it("gives every section its group's title and lede", () => {
    const el = renderSettings(voiceBase, handlers());
    for (const [key, title] of SETTINGS_GROUPS) {
      const s = el.querySelector<HTMLElement>(`.settings__section[data-group=${key}]`)!;
      expect(s.querySelector(".settings__title")!.textContent).toBe(title);
      expect(s.querySelector(".settings__lede")!.textContent).toBe(SETTINGS_LEDES[key]);
    }
    expect(SETTINGS_GROUPS).toHaveLength(12);
  });

  it("clusters the sidebar into five groups", () => {
    const el = renderSettings(voiceBase, handlers());
    const clusters = [...el.querySelectorAll(".settings__nav-group")];
    expect(clusters.map((c) => c.children.length)).toEqual([2, 4, 2, 2, 2]);
    expect(clusters.map((c) => [...c.querySelectorAll<HTMLElement>("button")].map((b) => b.dataset.group))).toEqual(SETTINGS_NAV.map((g) => [...g]));
  });

  it("shows a failure as a notice above every section", () => {
    const el = renderSettings({ ...voiceBase, error: "boom" }, handlers());
    const first = el.querySelector(".settings__content")!.firstElementChild!;
    expect(first.classList.contains("settings__notice")).toBe(true);
    expect(first.classList.contains("settings__error")).toBe(true);
    expect(first.textContent).toBe("boom");
    expect(el.querySelectorAll(".settings__notice")).toHaveLength(1);
  });

  it("shows the release notes, last in the sidebar", () => {
    const el = renderSettings({ ...voiceBase, settingsGroup: "releaseNotes" }, handlers());
    const s = el.querySelector<HTMLElement>(".settings__section[data-group=releaseNotes]")!;
    expect(s.hidden).toBe(false);
    expect(s.querySelector(".settings__title")!.textContent).toBe("Release Notes");
    expect(s.querySelector(".release-notes__version")!.firstChild!.textContent).toBe(`Maya ${RUNNING_VERSION}`);
  });

  it("has Maya present the release notes, with the running version marked", () => {
    const el = renderSettings({ ...voiceBase, settingsGroup: "releaseNotes" }, handlers());
    const s = el.querySelector<HTMLElement>(".settings__section[data-group=releaseNotes]")!;
    const rn = s.querySelector<HTMLElement>(".settings__rn")!;
    const art = rn.querySelector<HTMLImageElement>("img.settings__rn-art")!;
    expect(art.getAttribute("alt")).toBe("");
    expect(art.getAttribute("src")).toBe(artSrc);
    expect(art.nextElementSibling!.classList.contains("release-notes")).toBe(true);
    const running = [...rn.querySelectorAll(".release-notes__version")].find((x) => x.textContent!.startsWith(`Maya ${RUNNING_VERSION}`))!;
    expect(running.querySelector(".settings__running")!.textContent).toBe("Running");
    expect(rn.querySelectorAll(".settings__running")).toHaveLength(1);
  });

  it("styles the notes only inside Settings, and drops the art under 1100px", () => {
    expect(styles).toMatch(/\.settings\.pane \.settings__rn\s*\{[^}]*grid-template-columns:\s*290px minmax\(0, 1fr\)/);
    expect(styles).toMatch(/\.settings\.pane \.settings__rn-art\s*\{[^}]*position:\s*sticky[^}]*top:\s*24px[^}]*min\(76vh, 600px\)[^}]*margin-left:\s*-64px[^}]*drop-shadow/);
    expect(styles).toMatch(/\.settings\.pane \.release-notes\s*\{[^}]*max-width:\s*68ch/);
    const media = styles.match(/@media \(max-width: 1100px\)\s*\{([^@]*?\.settings\.pane \.settings__rn-art[^\n]*)/)![1];
    expect(media).toMatch(/\.settings\.pane \.settings__rn\s*\{\s*grid-template-columns:\s*minmax\(0, 1fr\)/);
    expect(media).toMatch(/\.settings\.pane \.settings__rn-art\s*\{\s*display:\s*none/);
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

  it("puts a device's buttons in their own group after its name, state and mirror switch", () => {
    const assistants = [{ id: "a1", name: "Iris", hostname: "iris", platform: "android", address: "192.168.55.99", connected: true, lastSeen: null, canLead: true, wantsMirror: true, mirror: true }];
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: { ...voiceBase.network, role: "main", assistants } }, handlers());
    const row = el.querySelector(".settings__assistant")!;
    expect(row.querySelector(".settings__device-name")?.textContent).toBe("Iris");
    expect(row.querySelector(".settings__device-meta")?.textContent).toBe("Android, 192.168.55.99");
    expect(row.querySelector(".settings__state--on")?.textContent).toBe("Connected");
    const actions = row.querySelector(".settings__assistant-actions")!;
    expect([...actions.children].map((c) => (c as HTMLElement).dataset.action)).toEqual(["remove-assistant"]);
    expect(row.querySelector(".settings__device-text input[data-action=mirror]")).not.toBeNull();
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
    expect(general.lastElementChild?.className).toBe("settings__group");
    expect(general.lastElementChild?.querySelector(".settings__version")?.textContent).toBe("Maya 0.13.0");
    expect(renderSettings(voiceBase, handlers()).querySelector(".settings__version")).toBeNull();
  });

  it("has the listen toggle and a microphone picker", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, listen: true, microphone: "USB Mic", microphones: ["MacBook Pro Microphone", "USB Mic"], listenError: null }, h);
    const listen = el.querySelector<HTMLInputElement>("input[name=listen]")!;
    expect(listen.checked).toBe(true);
    expect(listen.closest("label")?.textContent).toContain("Listen for the wake word");
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
    expect(dl.closest(".settings__row")!.querySelector(".settings__help")?.textContent).toContain("Download");
  });

  it("says the model stays on this computer, under the Linux user agent", () => {
    const original = navigator.userAgent;
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15", configurable: true });
    try {
      const tiny = renderSettings({ ...voiceBase, recognizer: "builtin", whisperModel: "tiny.en", models }, handlers());
      expect(tiny.querySelector("button[data-action=download-model]")!.closest(".settings__row")!.querySelector(".settings__help")?.textContent).toBe("Download the model once; it stays on this computer.");
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
    expect(addr.closest(".settings__row")?.querySelector("input[name=networkPort]")).not.toBeNull();
    expect(addr.classList.contains("settings__help")).toBe(true);
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
    expect(line.closest(".settings__row")?.querySelector("input[name=networkPort]")).not.toBeNull();
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
    expect(expired.querySelector(".settings__code:not(.settings__code--empty)")).toBeNull();
    expect(expired.querySelector(".settings__code--empty")?.textContent).toBe("No code yet");
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
    expect(host.querySelector(".settings__code:not(.settings__code--empty)")).toBeNull();
    expect(host.querySelector(".settings__code--empty")?.textContent).toBe("No code yet");
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
    expect(el.querySelector("select[name=networkRole]")!.closest(".settings__card")!.contains(err)).toBe(true);
    expect(err!.closest(".settings__row--foot")).not.toBeNull();
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
    const rows = [...el.querySelectorAll(".settings__assistant")].map((r) => [r.querySelector(".settings__device-name")?.textContent, r.querySelector(".settings__device-meta")?.textContent, r.querySelector(".settings__state")?.textContent]);
    expect(rows).toEqual([["Gnowee", "macOS, 192.168.55.70", "Connected"], ["desk", "macOS", "Never connected"]]);
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

describe("Settings › Devices", () => {
  it("the Devices section follows Network in the sidebar", () => {
    const el = renderSettings(voiceBase, handlers());
    const items = [...el.querySelectorAll<HTMLElement>(".settings__nav-item")].map((b) => b.dataset.group);
    expect(items[items.indexOf("network") + 1]).toBe("devices");
    expect(el.querySelector(".settings__section--devices")).not.toBeNull();
  });

  it("a main's Network section points to Devices instead of listing them", () => {
    const assistants = [{ id: "a1", name: "Iris", hostname: "iris", platform: "macos", connected: true, lastSeen: null }];
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: { ...voiceBase.network, role: "main", assistants } }, handlers());
    expect(el.querySelector(".settings__section--network")!.textContent).toContain("Paired devices are in Settings › Devices.");
    expect(el.querySelector(".settings__section--network .settings__assistant")).toBeNull();
    expect(el.querySelector(".settings__section--devices .settings__assistant")).not.toBeNull();
  });
});

describe("Settings → Network in the card-and-row design", () => {
  const mainNet = (over: Record<string, unknown> = {}) => ({ role: "main" as const, code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, ...over });
  const label = (r: Element) => r.querySelector(".settings__label")?.textContent;
  const rowsOf = (card: Element) => [...card.querySelectorAll(":scope > .settings__row")];

  it("a main's first card holds This Maya, Port with the addresses as its help, and the machine's name", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", networkPort: 4127, network: mainNet({ addresses: ["192.168.1.9", "10.8.0.4"] }) }, handlers());
    const card = el.querySelector("select[name=networkRole]")!.closest(".settings__card")!;
    expect(rowsOf(card).map(label)).toEqual(["This Maya", "Port", "This machine's name"]);
    const portRow = rowsOf(card)[1]!;
    expect(portRow.querySelector("input[name=networkPort]")).not.toBeNull();
    expect(portRow.querySelector(".settings__help.settings__addresses")?.textContent).toBe("This machine's addresses: 192.168.1.9, 10.8.0.4");
    expect(rowsOf(card)[2]!.querySelector("input[name=networkName]")).not.toBeNull();
  });

  it("tickAddresses puts a missing addresses line in as the Port row's help", () => {
    const model = { ...voiceBase, networkRole: "main" as const, network: mainNet() };
    const el = renderSettings(model, handlers());
    expect(el.querySelector(".settings__addresses")).toBeNull();
    tickAddresses(el, model, ["10.0.0.5"]);
    const help = el.querySelector(".settings__addresses")!;
    expect(help.classList.contains("settings__help")).toBe(true);
    const portRow = el.querySelector("input[name=networkPort]")!.closest(".settings__row")!;
    expect(portRow.contains(help)).toBe(true);
    expect(el.querySelector("input[name=networkPort]")!.getAttribute("aria-describedby")).toBe(help.id);
  });

  it("the pairing card shows the code, the instruction, the expiry and Regenerate", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: mainNet({ code: { code: "482199", expiresAt: 300_000 } }) }, handlers(), 0);
    const pair = el.querySelector(".settings__pair")!;
    expect(pair.querySelector(".settings__code")?.textContent).toBe("482 199");
    expect(pair.querySelector(".settings__code")?.classList.contains("settings__code--empty")).toBe(false);
    expect(pair.textContent).toContain("Type this on the machine you are pairing.");
    expect(pair.querySelector(".settings__code-expiry")?.textContent).toBe("expires in 5 min");
    expect(pair.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Regenerate");
    expect(pair.closest(".settings__group")?.querySelector(".settings__group-title")?.textContent).toBe("Pairing code");
    expect(pair.closest(".settings__group")?.querySelector(".settings__group-foot")?.textContent).toBe("Paired devices are in Settings › Devices.");
  });

  it("with no live code the pairing card shows 'No code yet' and Show pairing code", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: mainNet() }, handlers());
    const pair = el.querySelector(".settings__pair")!;
    const code = pair.querySelector(".settings__code")!;
    expect(code.classList.contains("settings__code--empty")).toBe(true);
    expect(code.textContent).toBe("No code yet");
    expect(pair.querySelector(".settings__code-expiry")).toBeNull();
    expect(pair.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Show pairing code");
  });

  it("tickPairingCode turns an expired code into the empty state in place", () => {
    const model: SettingsModel = { ...voiceBase, networkRole: "main", network: mainNet({ code: { code: "482199", expiresAt: 300_000 } }) };
    const host = document.createElement("div");
    host.append(renderSettings(model, handlers(), 100_000));
    tickPairingCode(host, model, 320_000);
    const code = host.querySelector(".settings__pair .settings__code")!;
    expect(code.classList.contains("settings__code--empty")).toBe(true);
    expect(code.textContent).toBe("No code yet");
    expect(host.querySelector(".settings__pair")!.textContent).not.toContain("Type this on the machine");
    expect(host.querySelector("button[data-action=regenerate-code]")?.textContent).toBe("Show pairing code");
  });

  it("the used-up state keeps today's wording inside the pairing card", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: mainNet({ codeUsedUp: true }) }, handlers());
    expect(el.querySelector(".settings__pair")?.textContent).toContain("Code used up after 3 wrong tries — Regenerate.");
  });

  it("only a main has the pairing card", () => {
    expect(renderSettings({ ...voiceBase, networkRole: "assistant" }, handlers()).querySelector(".settings__pair")).toBeNull();
    expect(renderSettings({ ...voiceBase }, handlers()).querySelector(".settings__pair")).toBeNull();
  });

  it("New key is a row with its help and a danger button; the confirmation opens in that card", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, networkRole: "main", network: mainNet() }, h);
    const key = el.querySelector<HTMLButtonElement>("button[data-action=new-key]")!;
    expect(key.classList.contains("settings__btn--danger")).toBe(true);
    const keyRow = key.closest(".settings__row")!;
    expect(label(keyRow)).toBe("New key");
    expect(keyRow.querySelector(".settings__help")?.textContent).toBe("Drops every pairing, so each device must pair again.");
    const asking = renderSettings({ ...voiceBase, networkRole: "main", network: mainNet(), networkNewKeyConfirm: true }, h);
    const confirm = asking.querySelector("[data-for=new-key-confirm]")!;
    expect(confirm.closest(".settings__card")?.textContent).toContain("New key");
    expect(confirm.textContent).toContain("Drop every pairing and make a new key?");
    expect(confirm.querySelector("button[data-action=new-key-confirm]")?.textContent).toBe("Drop and make a new key");
    expect(confirm.querySelector("button[data-action=new-key-cancel]")?.textContent).toBe("Cancel");
    expect(asking.querySelector("button[data-action=new-key]")).toBeNull();
  });

  it("an assistant's card has the fields as rows, Pair as its gold action and the status in the foot", () => {
    const el = renderSettings(
      { ...voiceBase, networkRole: "assistant", networkMainHost: "desk.local", networkMainPort: 4127, network: { role: "assistant", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null } } },
      handlers(),
    );
    const card = el.querySelector("select[name=networkRole]")!.closest(".settings__card")!;
    expect(rowsOf(card).filter((r) => !r.classList.contains("settings__row--foot")).map(label)).toEqual(["This Maya", "Main's host or address", "Port", "This machine's name", "Pairing code"]);
    for (const n of ["networkHost", "networkMainPort", "networkName", "networkCode"]) expect(card.querySelector(`input[name=${n}]`), n).not.toBeNull();
    const pair = card.querySelector("button[data-action=pair]")!;
    expect(pair.classList.contains("settings__btn--gold")).toBe(true);
    expect(pair.closest(".settings__row--foot")).not.toBeNull();
    const status = card.querySelector(".settings__status--network")!;
    expect(status.closest(".settings__row--foot")).not.toBeNull();
    expect(status.textContent).toBe("Reconnecting…");
  });

  it("a paired assistant's foot has Pair again and Forget main", () => {
    const el = renderSettings({ ...voiceBase, networkRole: "assistant", networkPaired: true, networkMainHost: "desk.local" }, handlers());
    for (const a of ["pair-again", "forget-main"]) expect(el.querySelector(`button[data-action=${a}]`)?.closest(".settings__row--foot"), a).not.toBeNull();
  });

  it("styles the pairing card with a gold edge, a 40px gold code and a 20px muted empty state", () => {
    expect(styles).toMatch(/\.settings\.pane \.settings__pair\s*\{[^}]*border:\s*1px solid var\(--gold-deep\)[^}]*border-radius:\s*14px[^}]*radial-gradient/);
    expect(styles).toMatch(/\.settings\.pane \.settings__pair \.settings__code\s*\{[^}]*font:[^}]*40px[^}]*color:\s*var\(--gold-bright\)/);
    expect(styles).toMatch(/\.settings\.pane \.settings__pair \.settings__code--empty\s*\{[^}]*font-size:\s*20px[^}]*color:\s*var\(--muted\)/);
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

describe("General, Keyboard, Notifications and Debug in the card design", () => {
  const titles = (el: HTMLElement, key: string) => [...el.querySelectorAll(`.settings__section--${key} .settings__group-title`)].map((t) => t.textContent);

  it("groups General as Agent, Appearance and Updates, with the agent hint as the Agent row's help", () => {
    const el = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: true, updateStatus: "Up to date." }, { ...handlers(), onCheckUpdates: vi.fn() });
    expect(titles(el, "general")).toEqual(["Agent", "Appearance", "Updates"]);
    const agentRow = el.querySelector("select[name=agent]")!.closest(".settings__row")!;
    expect(agentRow.querySelector(".settings__help")?.textContent).toContain("voice commands");
    const version = el.querySelector(".settings__version")!;
    expect(version.classList.contains("settings__label")).toBe(true);
    expect(version.textContent).toBe("Maya 0.18.0");
    expect(version.closest(".settings__row")!.querySelector(".settings__help.settings__update-status")?.textContent).toBe("Up to date.");
  });

  it("leaves the Updates help out without a status, and the button out when updates are off", () => {
    const h = { ...handlers(), onCheckUpdates: vi.fn() };
    const on = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: true }, h);
    expect(on.querySelector(".settings__version")!.closest(".settings__row")!.querySelector(".settings__help")).toBeNull();
    const off = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: false, updateStatus: "Up to date." }, h);
    expect(off.querySelector("[data-action=check-updates]")).toBeNull();
    expect(off.querySelector(".settings__version")?.textContent).toBe("Maya 0.18.0");
  });

  it("shows Keyboard as one card of rows with a foot note, then Reset to defaults", () => {
    const el = renderSettings({ ...voiceBase, keys: defaultKeys() }, { ...handlers(), onKeys: vi.fn() });
    const rows = el.querySelectorAll(".settings__section--keyboard [data-keys]");
    expect(rows.length).toBeGreaterThan(0);
    for (const r of rows) expect(r.classList.contains("settings__row") && r.classList.contains("settings__keys")).toBe(true);
    const group = el.querySelector(".settings__section--keyboard .settings__group")!;
    expect(group.querySelector(".settings__group-foot")?.textContent).toBe("An Enter press in neither list does what the box does by itself.");
    expect(group.nextElementSibling?.matches("button[data-action=keys-reset]")).toBe(true);
  });

  it("makes the notification switches toggles, with the Focus hint as the foot", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, focusVisible: false }, h);
    for (const name of ["notify", "speak"]) expect(el.querySelector(`input[name=${name}]`)!.classList.contains("settings__toggle")).toBe(true);
    expect(el.querySelector("input[name=speak]")!.closest(".settings__row")!.querySelector(".settings__help")?.textContent).toBe('"needs a decision", "is finished"');
    expect(el.querySelector(".settings__section--notifications .settings__group-foot [data-for=focus]")).not.toBeNull();
    expect(renderSettings(voiceBase, h).querySelector(".settings__section--notifications .settings__group-foot")).toBeNull();
  });

  it("shows Debug mode as a toggle row with its help", () => {
    const box = renderSettings(voiceBase, handlers()).querySelector("input[name=debug-mode]")!;
    expect(box.classList.contains("settings__toggle")).toBe(true);
    const row = box.closest(".settings__row")!;
    expect(row.querySelector(".settings__label")?.textContent).toBe("Debug mode");
    expect(row.querySelector(".settings__help")?.textContent).toBe("Shows the Debug tab and Maya's own card.");
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
    expect(el.querySelector("[data-row=p] .settings__device-name")?.textContent).toBe("Pixel");
    expect(el.querySelector("[data-row=p] .settings__device-meta")?.textContent).toBe("Android, 192.168.1.30");
    boxes[0].checked = false;
    boxes[0].dispatchEvent(new Event("change", { bubbles: true }));
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
  const block = (root: HTMLElement, p: string) => root.querySelector<HTMLElement>(`.settings__card.settings__git[data-provider=${p}]`)!;

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
    const storedKind = gl.querySelector<HTMLInputElement>("input[name=gitTokenKind-gitlab][value=stored]")!;
    storedKind.checked = true;
    storedKind.dispatchEvent(new Event("change"));
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
  it("heads each forge's card with its name and switch, and dims a card that is off", () => {
    const root = renderSettings(gitModel(), handlers());
    expect(root.querySelectorAll(".settings__card.settings__git[data-provider]")).toHaveLength(2);
    const gh = block(root, "github");
    expect(gh.querySelector(".settings__row--head .settings__label")?.textContent).toBe("GitHub");
    expect(gh.querySelector(".settings__row--head input[name=gitEnabled-github].settings__toggle")).not.toBeNull();
    expect(block(root, "gitlab").querySelector(".settings__row--head .settings__label")?.textContent).toBe("GitLab");
    expect(gh.classList.contains("settings__card--off")).toBe(false);
    expect(block(root, "gitlab").classList.contains("settings__card--off")).toBe(true);
    // jsdom does not apply styles.css, so check the rule's source and that the
    // off card's other rows are non-head rows inside it.
    expect(styles).toMatch(/\.settings\.pane \.settings__card--off \.settings__row:not\(\.settings__row--head\)\s*\{[^}]*opacity:\s*\.45/);
    const gl = block(root, "gitlab");
    expect(gl.querySelector(".settings__row--head")!.matches(".settings__card--off .settings__row:not(.settings__row--head)")).toBe(false);
    expect(gl.querySelector("input[name=gitClone-gitlab]")!.closest(".settings__row")!.matches(".settings__card--off .settings__row:not(.settings__row--head)")).toBe(true);
  });

  it("offers the token kind and the clone protocol as segmented choices", () => {
    const h = { ...handlers(), onGitTokenKind: vi.fn(), onGitClone: vi.fn() };
    const root = renderSettings(gitModel(), h);
    const gh = block(root, "github");
    for (const name of ["gitTokenKind-github", "gitClone-github"]) {
      const seg = gh.querySelector(`.settings__seg:has(input[name=${name}])`);
      expect(seg).not.toBeNull();
    }
    expect([...gh.querySelectorAll<HTMLInputElement>("input[name=gitTokenKind-github]")].map((r) => r.value)).toEqual(["stored", "env"]);
    expect([...gh.querySelectorAll<HTMLInputElement>("input[name=gitClone-github]")].map((r) => r.value)).toEqual(["ssh", "https"]);
    const stored = gh.querySelector<HTMLInputElement>("input[name=gitTokenKind-github][value=stored]")!;
    stored.checked = true;
    stored.dispatchEvent(new Event("change"));
    expect(h.onGitTokenKind).toHaveBeenCalledWith("github", "stored");
    const https = gh.querySelector<HTMLInputElement>("input[name=gitClone-github][value=https]")!;
    https.checked = true;
    https.dispatchEvent(new Event("change"));
    expect(h.onGitClone).toHaveBeenCalledWith("github", "https");
  });

  it("says which scope each Token needs, and drops the old hint paragraph", () => {
    const root = renderSettings(gitModel(), handlers());
    const help = (p: string) => [...block(root, p).querySelectorAll(".settings__row")].find((r) => r.querySelector(".settings__label")?.textContent === "Token")?.querySelector(".settings__help")?.textContent;
    expect(help("github")).toBe("Needs the repo scope.");
    expect(help("gitlab")).toBe("Needs read_api.");
    const section = root.querySelector(".settings__section--git")!;
    expect(section.textContent).not.toContain("GitHub needs the repo scope; GitLab");
    expect(section.querySelector(":scope > .settings__hint")).toBeNull();
  });

  it("names the secret store in the stored token's help, with a quiet Clear", () => {
    const stored = gitConnections([{ provider: "github", enabled: true, token: { kind: "stored" } }]);
    const root = renderSettings(gitModel({ git: stored, gitStatus: { github: { hasToken: true }, gitlab: { hasToken: false } } }), handlers());
    const tokenRow = block(root, "github").querySelector("input[name=gitToken-github]")!.closest(".settings__row")!;
    expect(tokenRow.querySelector(".settings__help")?.textContent).toBe(`Saved in ${secretStore()}.`);
    expect(tokenRow.querySelector("button[data-action=git-clear].settings__btn--quiet")).not.toBeNull();
  });

  it("puts the Test button and its outcome in the foot row", () => {
    const root = renderSettings(gitModel({ gitStatus: { github: { hasToken: false, test: "Connected as @tiago" }, gitlab: { hasToken: false } } }), handlers());
    const foot = block(root, "github").querySelector(".settings__row--foot")!;
    expect(foot.querySelector("button[data-action=git-test]")).not.toBeNull();
    expect(foot.querySelector(".settings__git-test")?.textContent).toBe("Connected as @tiago");
    expect(block(root, "gitlab").querySelector(".settings__row--foot .settings__git-test")).toBeNull();
    // No result: the Test button still sits at the end of the row.
    const testBtn = block(root, "gitlab").querySelector(".settings__row--foot button[data-action=git-test]")!;
    expect(testBtn.classList.contains("settings__foot-end")).toBe(true);
    expect(styles).toMatch(/\.settings\.pane \.settings__foot-end\s*\{[^}]*margin-left:\s*auto/);
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
    const chips = (action: string) => [...row(action).querySelectorAll(".settings__kbd")].map((c) => c.firstChild?.textContent);
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
    expect(row("send").querySelector(".settings__kbd button")).toBeNull();
    row("newLine").querySelectorAll<HTMLButtonElement>(".settings__kbd button")[0].click();
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
    expect(add.textContent).toBe("+ Add");
    press(add, { key: "Enter", shiftKey: true });
    add.click();
    add.dispatchEvent(new FocusEvent("blur"));
    expect(add.textContent).toBe("+ Add");
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

  it("offers Terminal, iTerm2 or Native under Sessions on a Mac and saves the choice", () => {
    pretend(MAC);
    const h = { ...handlers(), onNewSessionTerminal: vi.fn() };
    const el = renderSettings({ ...voiceBase, newSessionTerminal: "terminal" }, h);
    const sessions = el.querySelector(".settings__section--sessions")!;
    const sel = sessions.querySelector<HTMLSelectElement>("select[name=newSessionTerminal]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Terminal", "iTerm2", "Native"]);
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

  const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0";
  const LINUX = "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko)";

  it("offers Terminal, iTerm2 and Native on macOS", () => {
    pretend(MAC);
    expect(terminalChoices()).toEqual([["terminal", "Terminal"], ["iterm", "iTerm2"], ["native", "Native"]]);
  });
  it("offers Windows Terminal and Native on Windows", () => {
    pretend(WINDOWS);
    expect(terminalChoices()).toEqual([["terminal", "Windows Terminal"], ["native", "Native"]]);
  });
  it("offers Terminal (tmux) and Native on Linux", () => {
    pretend(LINUX);
    expect(terminalChoices()).toEqual([["terminal", "Terminal (tmux)"], ["native", "Native"]]);
  });
  it("takes any page that is not on Windows or Linux for a Mac's, as the rest of the page does", () => {
    pretend("Mozilla/5.0 (jsdom)");
    expect(terminalChoices()).toEqual([["terminal", "Terminal"], ["iterm", "iTerm2"], ["native", "Native"]]);
  });
  it("shows New sessions open in on every desktop platform and saves native", () => {
    for (const ua of [MAC, WINDOWS, LINUX]) {
      pretend(ua);
      const h = { ...handlers(), onNewSessionTerminal: vi.fn() };
      const sel = renderSettings({ ...voiceBase }, h).querySelector<HTMLSelectElement>("select[name=newSessionTerminal]")!;
      sel.value = "native";
      sel.dispatchEvent(new Event("change"));
      expect(h.onNewSessionTerminal).toHaveBeenCalledWith("native");
    }
  });
  it("shows Terminal on Windows when the saved value is iterm", () => {
    pretend(WINDOWS);
    const sel = renderSettings({ ...voiceBase, newSessionTerminal: "iterm" }, handlers()).querySelector<HTMLSelectElement>("select[name=newSessionTerminal]")!;
    expect(sel.value).toBe("terminal");
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

describe("Settings → Work Items", () => {
  const wiModel = (over: Partial<SettingsModel> = {}): SettingsModel => ({
    ...voiceBase,
    settingsGroup: "workItems",
    git: gitConnections([{ provider: "gitlab", enabled: true, token: { kind: "env", name: "GITLAB_TOKEN" } }, { provider: "github", enabled: true, token: { kind: "env", name: "GH_TOKEN" } }]),
    ...over,
  });
  const box = (root: HTMLElement, f: string) => root.querySelector<HTMLElement>(`.settings__workitems[data-forge=${f}]`)!;
  const type = (el: HTMLInputElement | HTMLSelectElement, value: string) => {
    el.value = value;
    el.dispatchEvent(new Event("change"));
  };

  it("lists Work Items after Pull requests", () => {
    const keys = SETTINGS_GROUPS.map(([k]) => k) as string[];
    expect(keys.indexOf("workItems")).toBe(keys.indexOf("reviews") + 1);
  });

  it("gives no two elements the same id, and each help is linked to its own forge's field", () => {
    const root = renderSettings(wiModel(), handlers());
    document.body.append(root);
    const ids = [...root.querySelectorAll("[id]")].map((e) => e.id);
    expect(ids.filter((id, i) => ids.indexOf(id) !== i)).toEqual([]);
    for (const f of ["github", "gitlab"]) {
      const field = box(root, f).querySelector("input[name=wi-statusLabels]")!;
      const help = document.getElementById(field.getAttribute("aria-describedby")!)!;
      expect(box(root, f).contains(help)).toBe(true);
    }
    root.remove();
  });

  it("saves a rule typed as comma lists", () => {
    const h = { ...handlers(), onWorkItems: vi.fn() };
    const root = renderSettings(wiModel(), h);
    const gh = box(root, "github");
    expect(gh.querySelector<HTMLInputElement>("input[name=wi-github-enabled]")!.checked).toBe(true);
    const rule = gh.querySelector<HTMLElement>(".settings__rule")!;
    type(rule.querySelector<HTMLSelectElement>("select[name=wi-assignee]")!, "anyone");
    type(rule.querySelector<HTMLInputElement>("input[name=wi-labels]")!, " urgent, bug ,,");
    type(rule.querySelector<HTMLInputElement>("input[name=wi-repos]")!, "acme/api");
    type(gh.querySelector<HTMLInputElement>("input[name=wi-statusLabels]")!, "todo, in progress");
    h.onWorkItems.mockClear();
    type(gh.querySelector<HTMLSelectElement>("select[name=wi-startStatus]")!, "in progress");
    expect(h.onWorkItems).toHaveBeenLastCalledWith("github", {
      enabled: true,
      rules: [{ assignee: { kind: "anyone" }, labels: ["urgent", "bug"], repos: ["acme/api"] }],
      statusLabels: ["todo", "in progress"],
      startStatus: "in progress",
      trackerRepos: [],
    });
  });

  it("saves a rule for someone with their login, and adds and removes rules", () => {
    const h = { ...handlers(), onWorkItems: vi.fn() };
    const root = renderSettings(wiModel(), h);
    const gh = box(root, "github");
    const rule = gh.querySelector<HTMLElement>(".settings__rule")!;
    const login = rule.querySelector<HTMLInputElement>("input[name=wi-login]")!;
    expect(login.hidden).toBe(true);
    type(rule.querySelector<HTMLSelectElement>("select[name=wi-assignee]")!, "user");
    expect(login.hidden).toBe(false);
    type(login, " alice ");
    expect(h.onWorkItems.mock.lastCall![1].rules[0].assignee).toEqual({ kind: "user", login: "alice" });
    gh.querySelector<HTMLButtonElement>("[data-action=wi-add-rule]")!.click();
    expect(h.onWorkItems.mock.lastCall![1].rules).toEqual([
      { assignee: { kind: "user", login: "alice" }, labels: [], repos: [] },
      { assignee: { kind: "me" }, labels: [], repos: [] },
    ]);
    gh.querySelector<HTMLButtonElement>("[data-action=wi-remove-rule]")!.click();
    expect(h.onWorkItems.mock.lastCall![1].rules).toEqual([]);
  });

  it("saves nothing while a rule for someone has no login", () => {
    const h = { ...handlers(), onWorkItems: vi.fn() };
    const root = renderSettings(wiModel(), h);
    const gh = box(root, "github");
    const rule = gh.querySelector<HTMLElement>(".settings__rule")!;
    type(rule.querySelector<HTMLSelectElement>("select[name=wi-assignee]")!, "user");
    type(rule.querySelector<HTMLInputElement>("input[name=wi-labels]")!, "bug");
    gh.querySelector<HTMLButtonElement>("[data-action=wi-add-rule]")!.click();
    type(gh.querySelector<HTMLInputElement>("input[name=wi-trackerRepos]")!, "acme/x");
    expect(h.onWorkItems).not.toHaveBeenCalled();
    type(rule.querySelector<HTMLInputElement>("input[name=wi-login]")!, "alice");
    expect(h.onWorkItems).toHaveBeenCalledTimes(1);
    expect(h.onWorkItems.mock.lastCall![1].rules[0]).toEqual({ assignee: { kind: "user", login: "alice" }, labels: ["bug"], repos: [] });
  });

  it("disables a service that is off in Settings → Git and links there", () => {
    const h = { ...handlers(), onOpenGroup: vi.fn() };
    const root = renderSettings(wiModel({ git: gitConnections(null) }), h);
    const gl = box(root, "gitlab");
    // The open body starts with the sentence and the link.
    const first = gl.querySelector(".settings__source-body")!.firstElementChild!;
    expect(first.textContent).toContain("GitLab is off in Settings → Git.");
    expect(first.querySelector("[data-action=wi-open-git]")).not.toBeNull();
    expect(gl.querySelector<HTMLButtonElement>(".settings__source-toggle")!.disabled).toBe(false);
    const fields = [...gl.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLButtonElement>("input, select")].filter((e) => e.name !== "wi-gitlab-enabled");
    expect(fields.length).toBeGreaterThan(0);
    expect(fields.every((e) => e.disabled)).toBe(true);
    gl.querySelector<HTMLButtonElement>("[data-action=wi-open-git]")!.click();
    expect(h.onOpenGroup).toHaveBeenCalledWith("git");
    expect(box(root, "github").textContent).not.toContain("is off in Settings");
  });

  it("offers only the status labels for On Start", () => {
    const items = { enabled: true, rules: [], statusLabels: ["todo", "doing"], startStatus: "doing", trackerRepos: [] };
    const root = renderSettings(wiModel({ workItems: { github: items, gitlab: items, coral: workItemsConfig(null).coral, prompt: "", custom: [] } }), handlers());
    const sel = box(root, "github").querySelector<HTMLSelectElement>("select[name=wi-startStatus]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Leave as it is", "todo", "doing"]);
    expect(sel.value).toBe("doing");
  });

  it("saves the work prompt and shows the token hints", () => {
    const h = { ...handlers(), onWorkPrompt: vi.fn() };
    const root = renderSettings(wiModel(), h);
    const prompt = root.querySelector<HTMLTextAreaElement>("textarea[name=workPrompt]")!;
    prompt.value = "  Do {id}  ";
    prompt.dispatchEvent(new Event("change"));
    expect(h.onWorkPrompt).toHaveBeenCalledWith("Do {id}");
    expect(box(root, "github").textContent).toContain("Issues: read and write");
    expect(box(root, "gitlab").textContent).toContain("the api scope");
  });
});

describe("ruleSummary", () => {
  it("names who the rule is for", () => {
    expect(ruleSummary({ assignee: { kind: "me" }, labels: [], repos: [] })).toBe("Assigned to me");
    expect(ruleSummary({ assignee: { kind: "user", login: "alice" }, labels: [], repos: [] })).toBe("Assigned to alice");
    expect(ruleSummary({ assignee: { kind: "user", login: "" }, labels: [], repos: [] })).toBe("Assigned to someone");
    expect(ruleSummary({ assignee: { kind: "anyone" }, labels: [], repos: [] })).toBe("Anyone's issues");
  });
  it("adds its labels and repositories", () => {
    expect(ruleSummary({ assignee: { kind: "me" }, labels: ["a", "b", "c"], repos: [] })).toBe("Assigned to me, labelled a, b or c");
    expect(ruleSummary({ assignee: { kind: "me" }, labels: [], repos: ["x/y", "x/z"] })).toBe("Assigned to me, in x/y or x/z");
    expect(ruleSummary({ assignee: { kind: "me" }, labels: ["bug"], repos: ["dosaki/maya"] })).toBe("Assigned to me, labelled bug, in dosaki/maya");
  });
});

describe("forgeSummary", () => {
  const rule = { assignee: { kind: "me" as const }, labels: [], repos: [] };
  it("counts the rules and names the start status when set", () => {
    expect(forgeSummary({ enabled: true, rules: [rule], statusLabels: [], startStatus: null, trackerRepos: [] })).toBe("1 rule");
    expect(forgeSummary({ enabled: true, rules: [rule, rule], statusLabels: ["in progress"], startStatus: "in progress", trackerRepos: [] })).toBe("2 rules; Start sets status to in progress");
  });
});

describe("storedSourcesOpen", () => {
  afterEach(() => localStorage.removeItem(SOURCES_OPEN_KEY));
  it("reads the open sources, and nothing when missing or unreadable", () => {
    expect(SOURCES_OPEN_KEY).toBe("maya.settingsSources");
    localStorage.removeItem(SOURCES_OPEN_KEY);
    expect(storedSourcesOpen()).toEqual({});
    localStorage.setItem(SOURCES_OPEN_KEY, "{not json");
    expect(storedSourcesOpen()).toEqual({});
    localStorage.setItem(SOURCES_OPEN_KEY, "[true]");
    expect(storedSourcesOpen()).toEqual({});
    localStorage.setItem(SOURCES_OPEN_KEY, "7");
    expect(storedSourcesOpen()).toEqual({});
    localStorage.setItem(SOURCES_OPEN_KEY, '{"github":true,"custom-1":false}');
    expect(storedSourcesOpen()).toEqual({ github: true, "custom-1": false });
  });
});

describe("Settings → Work Items sources", () => {
  afterEach(() => localStorage.removeItem(SOURCES_OPEN_KEY));
  const rule = (over: { labels?: string[]; repos?: string[] } = {}) => ({ assignee: { kind: "me" as const }, labels: [] as string[], repos: [] as string[], ...over });
  const forge = { enabled: true, rules: [rule(), rule({ labels: ["bug"] })], statusLabels: ["todo", "in progress"], startStatus: "in progress", trackerRepos: [] };
  const gitlabItems = { enabled: false, rules: [rule()], statusLabels: [], startStatus: null, trackerRepos: [] };
  const tool = { id: "custom-1", name: "Linear", enabled: true, listCommand: "~/bin/linear-items.sh", startCommand: "", me: "" };
  const offTool = { id: "custom-2", name: "Jira", enabled: false, listCommand: "jira-items", startCommand: "", me: "" };
  const both = gitConnections([{ provider: "github", enabled: true, token: { kind: "env", name: "GH_TOKEN" } }, { provider: "gitlab", enabled: true, token: { kind: "env", name: "GITLAB_TOKEN" } }]);
  const model = (over: Partial<SettingsModel> = {}): SettingsModel => ({
    ...voiceBase,
    settingsGroup: "workItems",
    git: both,
    workItems: { github: forge, gitlab: gitlabItems, coral: workItemsConfig(null).coral, prompt: "", custom: [tool, offTool] },
    ...over,
  });
  const src = (root: HTMLElement, key: string) => root.querySelector<HTMLElement>(`[data-source="${key}"]`)!;

  it("the Work Items lede names the Work Items tab", () => {
    expect(SETTINGS_LEDES.workItems).toBe("Where the Work Items tab finds your work, and what Start does with an item.");
  });

  it("lists GitHub, GitLab and each custom tool under Sources, then Add a custom tool", () => {
    const root = renderSettings(model(), handlers());
    const section = root.querySelector(".settings__section--workItems")!;
    const sources = [...section.querySelectorAll<HTMLElement>("[data-source]")];
    const group = sources[0].closest(".settings__group")!;
    expect(group.querySelector(".settings__group-title")!.textContent).toBe("Sources");
    expect(group.querySelector(".settings__group-note")!.textContent).toBe("Open one to change what the Work Items tab picks up from it.");
    expect(sources.map((s) => s.dataset.source)).toEqual(["github", "gitlab", "custom-1", "custom-2"]);
    expect(sources.every((s) => s.parentElement === group)).toBe(true);
    const add = group.lastElementChild as HTMLElement;
    expect(add.classList.contains("settings__add")).toBe(true);
    expect(add.textContent).toBe("+ Add a custom tool");
    expect(add.dataset.action).toBe("ct-add");
  });

  it("starts folded, with a summary and whether each is on", () => {
    const root = renderSettings(model(), handlers());
    for (const key of ["github", "gitlab", "custom-1", "custom-2"]) expect(src(root, key).hasAttribute("data-open")).toBe(false);
    const sum = (key: string) => src(root, key).querySelector(".settings__source-sum")!;
    expect(sum("github").textContent).toBe(forgeSummary(forge));
    expect(sum("gitlab").textContent).toBe("1 rule");
    expect(sum("custom-1").textContent).toBe("~/bin/linear-items.sh");
    expect(sum("custom-1").classList.contains("mono")).toBe(true);
    expect(src(root, "custom-1").querySelector(".settings__source-tag")!.textContent).toBe("Custom tool");
    expect(src(root, "github").querySelector(".settings__source-tag")).toBeNull();
    const stateOf = (key: string) => src(root, key).querySelector(".settings__source-state")!.textContent;
    expect(src(root, "github").querySelector<HTMLInputElement>("input[name=wi-github-enabled]")!.checked).toBe(true);
    expect(stateOf("github")).toBe("On");
    expect(stateOf("gitlab")).toBe("Off");
    expect(src(root, "gitlab").classList.contains("settings__source--off")).toBe(true);
    expect(src(root, "custom-1").querySelector<HTMLInputElement>("input[name=ct-0-enabled]")!.checked).toBe(true);
    expect(stateOf("custom-1")).toBe("On");
    expect(stateOf("custom-2")).toBe("Off");
    // The switch sits in the head, beside the state word.
    expect(src(root, "github").querySelector(".settings__source-head input[name=wi-github-enabled]")).not.toBeNull();
    expect(src(root, "custom-2").querySelector(".settings__source-head input[name=ct-1-enabled]")).not.toBeNull();
  });

  it("reads Git off, with the sentence in place of the summary, when the Git account is off", () => {
    const git = gitConnections([{ provider: "github", enabled: false, token: { kind: "env", name: "GH_TOKEN" } }, { provider: "gitlab", enabled: true, token: { kind: "env", name: "GITLAB_TOKEN" } }]);
    const root = renderSettings(model({ git }), handlers());
    const gh = src(root, "github");
    expect(gh.querySelector(".settings__source-state")!.textContent).toBe("Git off");
    expect(gh.querySelector(".settings__source-sum")!.textContent).toBe("GitHub is off in Settings → Git.");
    expect(gh.classList.contains("settings__source--off")).toBe(true);
    const offGl = renderSettings(model({ git: gitConnections(null), workItems: { github: forge, gitlab: { ...gitlabItems, enabled: true }, coral: workItemsConfig(null).coral, prompt: "", custom: [] } }), handlers());
    expect(src(offGl, "gitlab").querySelector(".settings__source-sum")!.textContent).toBe("GitLab is off in Settings → Git.");
    expect(src(offGl, "gitlab").querySelector(".settings__source-state")!.textContent).toBe("Git off");
  });

  it("opens a source in place and remembers it, without saving", () => {
    const h = { ...handlers(), onWorkItems: vi.fn(), onCustomTools: vi.fn(), onWorkPrompt: vi.fn() };
    const m = model();
    const root = renderSettings(m, h);
    const gh = src(root, "github");
    const head = gh.querySelector<HTMLButtonElement>(".settings__source-toggle")!;
    head.click();
    expect(gh.hasAttribute("data-open")).toBe(true);
    expect(src(root, "github")).toBe(gh);
    expect(m.sourcesOpen?.github).toBe(true);
    expect(localStorage.getItem("maya.settingsSources")).toBe('{"github":true}');
    expect(h.onWorkItems).not.toHaveBeenCalled();
    expect(h.onCustomTools).not.toHaveBeenCalled();
    expect(h.onWorkPrompt).not.toHaveBeenCalled();
    expect(src(renderSettings(m, h), "github").hasAttribute("data-open")).toBe(true);
    // Folding it again is remembered too.
    head.click();
    expect(m.sourcesOpen?.github).toBe(false);
    expect(localStorage.getItem("maya.settingsSources")).toBe('{"github":false}');
    // A click on the switch neither opens nor folds.
    src(root, "custom-1").querySelector<HTMLInputElement>("input[name=ct-0-enabled]")!.click();
    expect(src(root, "custom-1").hasAttribute("data-open")).toBe(false);
  });

  it("draws each rule as a box whose head sums it up, with a quiet Remove", () => {
    const h = { ...handlers(), onWorkItems: vi.fn() };
    const root = renderSettings(model({ sourcesOpen: { github: true } }), h);
    const rules = [...src(root, "github").querySelectorAll<HTMLElement>(".settings__rule")];
    expect(rules).toHaveLength(2);
    const head = (r: HTMLElement) => r.querySelector<HTMLElement>(".settings__rule-head")!;
    const summary = (r: HTMLElement) => head(r).querySelector(".settings__rule-sum")!.textContent;
    expect(summary(rules[0])).toBe(ruleSummary(forge.rules[0]));
    expect(summary(rules[1])).toBe("Assigned to me, labelled bug");
    const remove = head(rules[0]).querySelector<HTMLButtonElement>("button[data-action=wi-remove-rule]")!;
    expect(remove.classList.contains("settings__btn--quiet")).toBe(true);
    expect(remove.textContent).toBe("Remove");
    const labels = rules[0].querySelector<HTMLInputElement>("input[name=wi-labels]")!;
    labels.value = "bug, urgent";
    labels.dispatchEvent(new Event("input", { bubbles: true }));
    expect(summary(rules[0])).toBe("Assigned to me, labelled bug or urgent");
    expect(h.onWorkItems).not.toHaveBeenCalled();
    const repos = rules[0].querySelector<HTMLInputElement>("input[name=wi-repos]")!;
    repos.value = "dosaki/maya";
    repos.dispatchEvent(new Event("input", { bubbles: true }));
    expect(summary(rules[0])).toBe("Assigned to me, labelled bug or urgent, in dosaki/maya");
    // The rows of an open forge follow the rules.
    const gh = src(root, "github");
    const stack = rules[0].closest(".settings__row")!;
    expect(stack.classList.contains("settings__row--stack")).toBe(true);
    expect(stack.querySelector(".settings__label")!.textContent).toBe("Work items to pick up");
    expect(stack.querySelector(".settings__help")!.textContent).toBe("A work item shows when it matches any one of these rules.");
    expect(stack.querySelector(".settings__add[data-action=wi-add-rule]")!.textContent).toBe("+ Add rule");
    const helpOf = (n: string) => gh.querySelector(`[name=${n}]`)!.closest(".settings__row")!.querySelector(".settings__help")?.textContent;
    expect(helpOf("wi-statusLabels")).toBe("In order, comma-separated.");
    expect(helpOf("wi-trackerRepos")).toBe("Comma-separated.");
    expect(gh.querySelector("[name=wi-startStatus]")!.closest(".settings__row")!.querySelector(".settings__label")!.textContent).toBe("On Start, set status to");
    const foot = gh.querySelector(".settings__source-body")!.lastElementChild!;
    expect(foot.classList.contains("settings__row--foot")).toBe(true);
    expect(foot.textContent).toContain("Issues: read and write");
  });

  it("lays a custom tool out with its name row and Remove this tool at the body's foot", () => {
    const root = renderSettings(model(), handlers());
    const t = src(root, "custom-1");
    const name = t.querySelector<HTMLInputElement>("input[name=ct-0-name]")!;
    const nameRow = name.closest(".settings__row")!;
    expect(nameRow.querySelector(".settings__label")!.textContent).toBe("Name");
    const remove = t.querySelector<HTMLButtonElement>("[data-action=ct-remove]")!;
    expect(remove.textContent).toBe("Remove this tool");
    expect(t.querySelector(".settings__source-head [data-action=ct-remove]")).toBeNull();
    const body = t.querySelector(".settings__source-body")!;
    expect(body.lastElementChild!.classList.contains("settings__row--foot")).toBe(true);
    expect(body.lastElementChild!.contains(remove)).toBe(true);
  });

  it("keeps a custom tool's details open as the model says, and records a toggle", () => {
    const m = model({ customDetailsOpen: { "custom-2": true } });
    const root = renderSettings(m, handlers());
    expect(src(root, "custom-1").querySelector("details")!.open).toBe(false);
    expect(src(root, "custom-2").querySelector("details")!.open).toBe(true);
    const d = src(root, "custom-1").querySelector("details")!;
    d.open = true;
    d.dispatchEvent(new Event("toggle"));
    expect(m.customDetailsOpen?.["custom-1"]).toBe(true);
  });

  it("puts the work prompt in a stacked row", () => {
    const root = renderSettings(model(), handlers());
    const prompt = root.querySelector<HTMLTextAreaElement>("textarea[name=workPrompt]")!;
    const r = prompt.closest(".settings__row")!;
    expect(r.classList.contains("settings__row--stack")).toBe(true);
    expect(r.querySelector(".settings__help")!.textContent).toBe("What Start tells the agent. Blank uses the built-in prompt. {id}, {title}, {url}, {description} and {repo} are filled in.");
    expect(r.closest(".settings__group")!.querySelector(".settings__group-title")!.textContent).toBe("Work prompt");
  });

  it("cuts a long list command short rather than widen the head", () => {
    const long = { ...tool, listCommand: "/a/very/long/path/to/a/script/that/lists/items --with --many --flags ".repeat(4) };
    const root = renderSettings(model({ workItems: { github: forge, gitlab: gitlabItems, coral: workItemsConfig(null).coral, prompt: "", custom: [long] } }), handlers());
    const sum = src(root, "custom-1").querySelector(".settings__source-sum")!;
    // jsdom does not apply styles.css: check the rule and that the summary matches it.
    const m = styles.match(/\.settings\.pane \.settings__source-sum\s*\{([^}]*)\}/);
    expect(m).not.toBeNull();
    expect(m![1]).toMatch(/text-overflow:\s*ellipsis/);
    expect(m![1]).toMatch(/white-space:\s*nowrap/);
    expect(m![1]).toMatch(/overflow:\s*hidden/);
    expect(m![1]).toMatch(/min-width:\s*0/);
    expect(sum.matches(".settings__source-sum")).toBe(true);
  });

  it("gives a rule's fields the full width of their column", () => {
    const root = renderSettings(model({ sourcesOpen: { github: true } }), handlers());
    const r = src(root, "github").querySelector<HTMLElement>(".settings__rule")!;
    expect(styles).toMatch(/\.settings\.pane \.settings__rule-body\s*\{[^}]*grid-template-columns:\s*120px minmax\(0,\s*1fr\)/);
    const widthRule = styles.match(/\.settings\.pane \.settings__rule input, \.settings\.pane \.settings__rule select\s*\{([^}]*)\}/);
    expect(widthRule).not.toBeNull();
    expect(widthRule![1]).toMatch(/width:\s*100%/);
    for (const n of ["wi-labels", "wi-repos", "wi-login"]) {
      expect(r.querySelector(`input[name=${n}]`)!.matches(".settings__rule input")).toBe(true);
    }
    expect(r.querySelector("select[name=wi-assignee]")!.matches(".settings__rule select")).toBe(true);
    expect(r.querySelector("input[name=wi-labels]")!.parentElement!.classList.contains("settings__rule-body")).toBe(true);
    expect(styles).not.toMatch(/\n\.settings__rule-field\s*\{/);
    expect(styles).not.toMatch(/\n\.settings__custom pre\s*\{/);
  });
});

describe("Settings → Work Items custom tools", () => {
  const items = { enabled: false, rules: [], statusLabels: [], startStatus: "", trackerRepos: [] };
  const t1 = { id: "custom-1", name: "Linear", enabled: true, listCommand: "linear-items", startCommand: "linear-start", me: "tiago" };
  const t2 = { id: "custom-2", name: "Jira", enabled: false, listCommand: "jira-items", startCommand: "", me: "" };
  const model = (): SettingsModel => ({ ...voiceBase, settingsGroup: "workItems", workItems: { github: items, gitlab: items, coral: workItemsConfig(null).coral, prompt: "", custom: [t1, t2] } });
  const boxes = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>(".settings__custom")];
  const input = (root: HTMLElement, n: string) => root.querySelector<HTMLInputElement>(`input[name=${n}]`)!;

  it("shows each custom tool with its commands and an example", () => {
    const root = renderSettings(model(), handlers());
    const b = boxes(root);
    expect(b.map((x) => x.dataset.tool)).toEqual(["custom-1", "custom-2"]);
    expect(input(root, "ct-0-enabled").checked).toBe(true);
    expect(input(root, "ct-1-enabled").checked).toBe(false);
    expect(input(root, "ct-0-name").value).toBe("Linear");
    expect(input(root, "ct-0-name").placeholder).toBe("Linear (mine-only)");
    expect(input(root, "ct-0-list").value).toBe("linear-items");
    expect(input(root, "ct-0-start").value).toBe("linear-start");
    expect(input(root, "ct-0-me").value).toBe("tiago");
    expect(input(root, "ct-1-list").value).toBe("jira-items");
    const help = (n: string) => input(root, n).closest(".settings__row")!.querySelector(".settings__help")!.textContent;
    expect(help("ct-0-name")).toBe("How its items are labelled on the Work Items tab.");
    expect(help("ct-0-list")).toBe("Runs every 2 minutes through your shell, and must finish within 30 seconds.");
    expect(help("ct-0-start")).toBe("Runs when you press Start, with the item's JSON on its input, to assign or move it in your tool.");
    expect(help("ct-0-me")).toBe("Start asks before taking over an item assigned to someone else. Blank: it never asks.");
    expect(input(root, "ct-0-list").closest(".settings__row")!.classList.contains("settings__row--stack")).toBe(true);
    expect(input(root, "ct-0-start").closest(".settings__row")!.classList.contains("settings__row--stack")).toBe(true);
    expect(input(root, "ct-0-list").classList.contains("mono")).toBe(true);
    expect(input(root, "ct-0-start").classList.contains("mono")).toBe(true);
    expect(b[0].querySelector("details > summary")!.textContent).toBe("Output format and an example script");
    expect(b[0].querySelectorAll("details pre").length).toBe(2);
    expect(b[0].querySelector("details")!.textContent).toContain("id, title and url are required");
    expect(b[0].querySelector("details")!.textContent).toContain("On macOS and Linux, anything a command leaves running is stopped when it ends.");
    expect(input(root, "ct-0-enabled").getAttribute("aria-label")).toBe("Use this tool");
    expect(input(root, "ct-0-name").getAttribute("aria-label")).toBe("Tool name");
  });

  it("adds, edits and removes custom tools", () => {
    const h = { ...handlers(), onCustomTools: vi.fn() };
    const root = renderSettings(model(), h);
    root.querySelector<HTMLButtonElement>("[data-action=ct-add]")!.click();
    expect(h.onCustomTools).toHaveBeenLastCalledWith([t1, t2, { id: "", name: "", enabled: true, listCommand: "", startCommand: "", me: "" }]);
    const list = input(root, "ct-0-list");
    list.value = " linear-items2 ";
    list.dispatchEvent(new Event("change"));
    expect(h.onCustomTools).toHaveBeenLastCalledWith([{ ...t1, listCommand: "linear-items2" }, t2]);
    expect(list.value).toBe("linear-items2");
    const on = input(root, "ct-1-enabled");
    on.checked = true;
    on.dispatchEvent(new Event("change"));
    expect(h.onCustomTools).toHaveBeenLastCalledWith([{ ...t1, listCommand: "linear-items2" }, { ...t2, enabled: true }]);
    boxes(root)[0].querySelector<HTMLButtonElement>("[data-action=ct-remove]")!.click();
    expect(h.onCustomTools).toHaveBeenLastCalledWith([{ ...t2, enabled: true }]);
  });

  it("builds each save from what the page shows, not from the last drawing", () => {
    const h = { ...handlers(), onCustomTools: vi.fn() };
    const root = renderSettings(model(), h);
    const list = input(root, "ct-1-list");
    list.value = " jira-2 ";
    list.dispatchEvent(new Event("change"));
    const on = input(root, "ct-1-enabled");
    on.checked = true;
    on.dispatchEvent(new Event("change"));
    expect(h.onCustomTools).toHaveBeenLastCalledWith([t1, { ...t2, listCommand: "jira-2", enabled: true }]);
    boxes(root)[0].querySelector<HTMLButtonElement>("[data-action=ct-remove]")!.click();
    expect(h.onCustomTools).toHaveBeenLastCalledWith([{ ...t2, listCommand: "jira-2", enabled: true }]);
    root.querySelector<HTMLButtonElement>("[data-action=ct-add]")!.click();
    expect(h.onCustomTools.mock.lastCall![0]).toEqual([t1, { ...t2, listCommand: "jira-2", enabled: true }, { id: "", name: "", enabled: true, listCommand: "", startCommand: "", me: "" }]);
  });

  it("keeps the custom boxes enabled when Git is off", () => {
    const root = renderSettings({ ...model(), git: gitConnections(null) }, handlers());
    const fields = [...root.querySelectorAll<HTMLInputElement | HTMLButtonElement>(".settings__custom input, .settings__custom button, [data-action=ct-add]")];
    expect(fields.length).toBeGreaterThan(10);
    expect(fields.every((f) => !f.disabled)).toBe(true);
  });
});

describe("workItemsConfig coral", () => {
  it("is off with one rule for my items when the file lacks it", () => {
    expect(workItemsConfig(null).coral).toEqual({ enabled: false, baseUrl: "", token: { kind: "stored" }, rules: [{ assignee: { kind: "me" }, labels: [], repos: [] }], startStatus: null });
    const coral = { enabled: true, baseUrl: "https://coral.test", token: { kind: "env" as const, name: "CORAL_TOKEN" }, rules: [], startStatus: "s1" };
    expect(workItemsConfig({ coral }).coral).toEqual(coral);
  });
});

describe("workItemsConfig custom tools", () => {
  it("defaults to none and keeps the ones given", () => {
    expect(workItemsConfig(null).custom).toEqual([]);
    const t = { id: "custom-1", name: "Jira", enabled: true, listCommand: "ls", startCommand: "", me: "" };
    expect(workItemsConfig({ custom: [t] }).custom).toEqual([t]);
  });
});

describe("the Voice popover keeps its look", () => {
  // jsdom does not apply styles.css to computed style, so this compares the
  // rules instead: every rule (and @media) that matches an element inside the
  // popover, per element, against the same list taken from main's stylesheets
  // (src/fixtures/popover-rules-main.json) before the Settings redesign.
  function splitSelectors(text: string): string[] {
    const out: string[] = [];
    let depth = 0;
    let cur = "";
    for (const ch of text) {
      if (ch === "(" || ch === "[") depth++;
      if (ch === ")" || ch === "]") depth--;
      if (ch === "," && depth === 0) { out.push(cur.trim()); cur = ""; } else cur += ch;
    }
    out.push(cur.trim());
    return out;
  }

  function popover(): HTMLElement {
    const aside = document.createElement("aside");
    aside.id = "voice-panel";
    aside.className = "settings popover";
    aside.append(renderVoicePanel(
      { listening: true, state: "error", detail: "Mic gone", level: 0, heard: "", said: "", pending: "Archive three sessions.", until: Date.now() + 5000, turns: 1 },
      [{ who: "user", text: "what is waiting", at: 1 }, { who: "maya", text: "Two sessions.", at: 2 }],
      { onCancel() {} },
    ));
    aside.append(renderVoicePanel({ listening: true, state: "idle", detail: "", level: 0, heard: "", said: "", pending: null, turns: 0 }, [], { onCancel() {} }));
    document.body.append(aside);
    return aside;
  }

  /** For each element in the popover, the "media | selector { declarations }" entries that match it. */
  function popoverRules(sheets: string[]): Record<string, string[]> {
    const aside = popover();
    const out: Record<string, string[]> = {};
    const elements = [aside, ...Array.from(aside.querySelectorAll("*"))];
    const keyOf = (el: Element, i: number) => `${i}:${el.tagName.toLowerCase()}${el.id ? "#" + el.id : ""}.${[...el.classList].join(".")}`;
    const visit = (rules: CSSRuleList, media: string) => {
      for (const rule of Array.from(rules)) {
        if ((rule as CSSMediaRule).media) {
          visit((rule as CSSMediaRule).cssRules, `${media}@media ${(rule as CSSMediaRule).media.mediaText} `);
          continue;
        }
        const style = rule as CSSStyleRule;
        if (!style.selectorText) continue;
        for (const sel of splitSelectors(style.selectorText)) {
          elements.forEach((el, i) => {
            let hit = false;
            try { hit = el.matches(sel); } catch { hit = false; }
            if (hit) (out[keyOf(el, i)] ??= []).push(`${media}${sel} { ${style.style.cssText} }`);
          });
        }
      }
    };
    for (const css of sheets) {
      const style = document.createElement("style");
      style.textContent = css;
      document.head.append(style);
      visit(style.sheet!.cssRules, "");
      style.remove();
    }
    aside.remove();
    return out;
  }

  it("is styled by the same rules as before the redesign", () => {
    const now = popoverRules([styles, mobileStyles]);
    expect(Object.keys(now).length).toBeGreaterThan(6);
    expect(now).toEqual(popoverMain);
  });

  it("is reached by no rule scoped to the Settings pane", () => {
    const hits = Object.values(popoverRules([styles, mobileStyles])).flat().filter((r) => r.includes(".settings.pane"));
    expect(hits).toEqual([]);
  });
});

describe("the Settings stylesheet after the redesign", () => {
  const text = styles.replace(/\/\*[\s\S]*?\*\//g, "");
  const rules = (css: string) => {
    const style = document.createElement("style");
    style.textContent = css;
    document.head.append(style);
    const list = Array.from(style.sheet!.cssRules);
    style.remove();
    return list;
  };

  it("no longer carries the old label grid, the 559px row rule, the 22px code or the unused check and switch rules", () => {
    expect(text).not.toMatch(/\.settings__section label/);
    expect(text).not.toMatch(/max-width:\s*559px/);
    expect(text).not.toMatch(/\n\.settings__code\s*\{/);
    expect(text).not.toMatch(/\.settings__(check|switch|git-kinds)\b/);
    expect(text).not.toMatch(/\n\.settings__row\s*\{/);
  });

  it("leaves no rule matching a Work Items rule-body label or a segmented label that makes it a grid", () => {
    const root = renderSettings({ ...voiceBase, settingsGroup: "workItems", git: gitConnections([{ provider: "github", enabled: true, token: { kind: "env", name: "GH_TOKEN" } }]) }, handlers());
    document.body.append(root);
    const labels = [root.querySelector(".settings__rule-body label")!, root.querySelector(".settings__seg label") ?? root.querySelector(".settings__rule-body label")!];
    expect(labels[0]).not.toBeNull();
    for (const label of labels) {
      for (const rule of rules(styles)) {
        const sel = rule as CSSStyleRule;
        if (!sel.selectorText) continue;
        let hit = false;
        try { hit = label.matches(sel.selectorText); } catch { hit = false; }
        if (hit) expect(sel.style.getPropertyValue("display"), sel.selectorText).not.toBe("grid");
      }
    }
    root.remove();
  });

  it("keeps the key chips under their own comment, apart from the state dot", () => {
    const kbd = styles.indexOf(".settings.pane .settings__kbd {");
    const state = styles.indexOf(".settings.pane .settings__state {");
    const stateComment = styles.indexOf("/* State, as a dot and a word. */");
    const kbdComment = styles.indexOf("/* Key chips. */");
    expect(kbdComment).toBeGreaterThan(-1);
    expect(kbdComment).toBeLessThan(kbd);
    expect(stateComment).toBeGreaterThan(kbd);
    expect(stateComment).toBeLessThan(state);
  });

  it("keeps Work Items, details and pre padding on the spacing scale", () => {
    expect(styles).not.toMatch(/\.settings__rule-body \{[^}]*padding: 14px/);
    expect(styles).not.toMatch(/\.settings__details > summary \{[^}]*padding: 14px/);
    expect(styles).not.toMatch(/\.settings__details-body pre \{[^}]*padding: 14px/);
  });

  it("drops the top margin of a help that is the first thing in its row", () => {
    expect(styles).toContain(".settings.pane .settings__row-text > .settings__help:first-child { margin-top: 0; }");
  });
});

describe("Devices with the network off", () => {
  it("holds today's line in a card, keeping its data-for and data-row", () => {
    const el = renderSettings({ ...voiceBase, settingsGroup: "devices" }, handlers());
    const line = el.querySelector<HTMLElement>(".settings__section--devices [data-for=devices-off]")!;
    expect(line.dataset.row).toBe("devices-off");
    expect(line.textContent).toBe("Turn the network on in Settings › Network to see your devices.");
    expect(line.parentElement!.classList.contains("settings__card")).toBe(true);
  });
});
