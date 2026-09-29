import { describe, expect, it, vi } from "vitest";
import { renderSettings } from "./settings";

const voiceBase = { hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null };
const handlers = () => ({ onInstall: vi.fn(), onRemove: vi.fn(), onTimeout: vi.fn(), onProjectsDir: vi.fn(), onNotify: vi.fn(), onClonesDir: vi.fn(), onSpeak: vi.fn(), onVoiceProvider: vi.fn(), onElevenKey: vi.fn(), onElevenVoice: vi.fn(), onTryVoice: vi.fn(), onListen: vi.fn(), onMicrophone: vi.fn(), onInterpreter: vi.fn() });

describe("renderSettings", () => {
  it("offers install when the hook is missing", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: false, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("not installed");
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=install]")!;
    btn.click();
    expect(h.onInstall).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=remove]")).toBeNull();
  });

  it("offers remove when the hook is installed", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("installed");
    el.querySelector<HTMLButtonElement>("button[data-action=remove]")!.click();
    expect(h.onRemove).toHaveBeenCalled();
  });

  it("reports timeout changes and shows errors", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: null, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: "boom", listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=timeout]")!;
    expect(input.value).toBe("30");
    input.value = "45";
    input.dispatchEvent(new Event("change"));
    expect(h.onTimeout).toHaveBeenCalledWith(45);
    expect(el.querySelector(".settings__error")?.textContent).toBe("boom");
  });

  it("has a notification toggle that reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=notify]")!;
    expect(box.type).toBe("checkbox");
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("awaits a decision");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onNotify).toHaveBeenCalledWith(false);
    expect(renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: false, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h).querySelector<HTMLInputElement>("input[name=notify]")!.checked).toBe(false);
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
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
    const box = el.querySelector<HTMLInputElement>("input[name=speak]")!;
    expect(box.checked).toBe(true);
    expect(box.closest("label")?.textContent).toContain("Speak");
    box.checked = false;
    box.dispatchEvent(new Event("change"));
    expect(h.onSpeak).toHaveBeenCalledWith(false);
  });

  it("shows the clones directory with the default as placeholder and reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=clonesDir]")!;
    expect(input.placeholder).toBe("~/dev/reviews");
    expect(input.value).toBe("");
    input.value = " ~/tmp/reviews ";
    input.dispatchEvent(new Event("change"));
    expect(h.onClonesDir).toHaveBeenCalledWith("~/tmp/reviews");
  });

  it("shows the projects directory and reports changes", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, projectsDir: "~/dev", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin" as const, elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null }, h);
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
    expect([...el.querySelectorAll(".settings__heading")].map((x) => x.textContent)).toEqual(["Sessions", "Notifications", "Voice assistant"]);
    expect(el.querySelector(".settings__section input[name=listen]")).not.toBeNull();
  });

  it("has the listen toggle, a microphone picker and an interpreter model", () => {
    const h = handlers();
    const el = renderSettings({ ...voiceBase, listen: true, microphone: "USB Mic", microphones: ["MacBook Pro Microphone", "USB Mic"], interpreterModel: "haiku", listenError: null }, h);
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
    const model = el.querySelector<HTMLSelectElement>("select[name=interpreterModel]")!;
    expect([...model.options].map((o) => o.value)).toEqual(["haiku", "sonnet", "opus"]);
    model.value = "sonnet";
    model.dispatchEvent(new Event("change"));
    expect(h.onInterpreter).toHaveBeenCalledWith("sonnet");
  });
});
