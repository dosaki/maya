import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { VoiceStatus } from "./voice";

export interface SettingsModel {
  hookInstalled: boolean | null;
  completedTimeoutMinutes: number;
  projectsDir: string;
  clonesDir: string;
  notifyOnAwaiting: boolean;
  speakNotifications: boolean;
  voiceProvider: VoiceProvider;
  elevenKeySet: boolean;
  elevenVoices: ElevenVoice[];
  elevenVoiceId: string;
  error: string | null;
  listen: boolean;
  microphone: string;
  microphones: string[];
  interpreterModel: string;
  /** Why the listener stopped (Dictation off, no microphone…), shown under the toggle. */
  listenError: string | null;
}

export type VoiceProvider = "builtin" | "elevenlabs";
export interface ElevenVoice {
  voiceId: string;
  name: string;
}

export interface SettingsHandlers {
  onInstall(): void;
  onRemove(): void;
  onTimeout(minutes: number): void;
  onProjectsDir(path: string): void;
  onNotify(enabled: boolean): void;
  onClonesDir(path: string): void;
  onSpeak(enabled: boolean): void;
  onVoiceProvider(provider: VoiceProvider): void;
  onElevenKey(key: string): void;
  onElevenVoice(voiceId: string): void;
  onTryVoice(): void;
  onListen(on: boolean): void;
  onMicrophone(name: string): void;
  onInterpreter(model: string): void;
}

interface ConfigJson {
  completedTimeoutMinutes: number;
  projectsDir?: string | null;
  clonesDir?: string | null;
  notifyOnAwaiting: boolean;
  speakNotifications: boolean;
  voiceProvider: VoiceProvider;
  elevenlabsVoiceId?: string | null;
  listen: boolean;
  microphone?: string | null;
  interpreterModel: string;
}

/** A titled card in the settings grid. */
function section(title: string): HTMLElement {
  const s = document.createElement("section");
  s.className = "settings__section";
  const heading = document.createElement("h2");
  heading.className = "settings__heading";
  heading.textContent = title;
  s.append(heading);
  return s;
}

export function renderSettings(model: SettingsModel, h: SettingsHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "settings__body";
  const sessions = section("Sessions");
  const notifications = section("Notifications");
  const assistant = section("Voice assistant");
  root.append(sessions, notifications, assistant);

  const status = document.createElement("div");
  status.className = "settings__status";
  status.textContent =
    model.hookInstalled === null
      ? "Checking hook…"
      : model.hookInstalled
        ? "Claude Code hook is installed. Awaiting Decision and Completed are precise."
        : "Claude Code hook is not installed. Permission prompts will show as Working.";
  sessions.append(status);

  const btn = document.createElement("button");
  btn.type = "button";
  if (model.hookInstalled) {
    btn.dataset.action = "remove";
    btn.textContent = "Remove hook";
    btn.addEventListener("click", () => h.onRemove());
  } else {
    btn.dataset.action = "install";
    btn.textContent = "Install hook";
    btn.disabled = model.hookInstalled === null;
    btn.addEventListener("click", () => h.onInstall());
  }
  sessions.append(btn);

  const label = document.createElement("label");
  label.textContent = "Completed decays to Idle after (minutes)";
  const input = document.createElement("input");
  input.type = "number";
  input.name = "timeout";
  input.min = "1";
  input.value = String(model.completedTimeoutMinutes);
  input.addEventListener("change", () => {
    const n = Number(input.value);
    if (Number.isFinite(n) && n >= 1) h.onTimeout(Math.floor(n));
  });
  label.append(input);
  sessions.append(label);

  const dirLabel = document.createElement("label");
  dirLabel.textContent = "Projects directory";
  const dirInput = document.createElement("input");
  dirInput.type = "text";
  dirInput.name = "projectsDir";
  dirInput.placeholder = "~/dev";
  dirInput.value = model.projectsDir;
  dirInput.addEventListener("change", () => h.onProjectsDir(dirInput.value.trim()));
  dirLabel.append(dirInput);
  sessions.append(dirLabel);

  const clonesLabel = document.createElement("label");
  clonesLabel.textContent = "Clones directory (for PR reviews)";
  const clonesInput = document.createElement("input");
  clonesInput.type = "text";
  clonesInput.name = "clonesDir";
  clonesInput.placeholder = "~/dev/reviews";
  clonesInput.value = model.clonesDir;
  clonesInput.addEventListener("change", () => h.onClonesDir(clonesInput.value.trim()));
  clonesLabel.append(clonesInput);
  sessions.append(clonesLabel);

  const notifyLabel = document.createElement("label");
  notifyLabel.className = "settings__check";
  const notifyBox = document.createElement("input");
  notifyBox.type = "checkbox";
  notifyBox.name = "notify";
  notifyBox.checked = model.notifyOnAwaiting;
  notifyBox.addEventListener("change", () => h.onNotify(notifyBox.checked));
  notifyLabel.append(notifyBox, document.createTextNode(" Notify me when a session awaits a decision"));
  notifications.append(notifyLabel);

  const speakLabel = document.createElement("label");
  speakLabel.className = "settings__check";
  const speakBox = document.createElement("input");
  speakBox.type = "checkbox";
  speakBox.name = "speak";
  speakBox.checked = model.speakNotifications;
  speakBox.addEventListener("change", () => h.onSpeak(speakBox.checked));
  speakLabel.append(speakBox, document.createTextNode(" Speak instead of a sound (\"needs a decision\", \"is finished\")"));
  notifications.append(speakLabel);

  const listenLabel = document.createElement("label");
  listenLabel.className = "settings__check";
  const listenBox = document.createElement("input");
  listenBox.type = "checkbox";
  listenBox.name = "listen";
  listenBox.checked = model.listen;
  listenBox.addEventListener("change", () => h.onListen(listenBox.checked));
  listenLabel.append(listenBox, document.createTextNode(' Listen for "Maya" (on-device speech recognition)'));
  assistant.append(listenLabel);
  if (model.listenError) {
    // The listener stopped on its own (Dictation off, no microphone…): say
    // why right under the toggle, or an unticked box looks like a glitch.
    const err = document.createElement("div");
    err.className = "settings__error";
    err.dataset.for = "listen";
    err.textContent = model.listenError;
    assistant.append(err);
  }

  const micLabel = document.createElement("label");
  micLabel.textContent = "Microphone";
  const mic = document.createElement("select");
  mic.name = "microphone";
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Built-in (recommended)";
  mic.append(auto);
  for (const name of model.microphones) {
    const o = document.createElement("option");
    o.value = name;
    o.textContent = name;
    mic.append(o);
  }
  mic.value = model.microphones.includes(model.microphone) ? model.microphone : "";
  mic.addEventListener("change", () => h.onMicrophone(mic.value));
  micLabel.append(mic);
  assistant.append(micLabel);

  const modelLabel = document.createElement("label");
  modelLabel.textContent = "Voice interpreter";
  const modelSel = document.createElement("select");
  modelSel.name = "interpreterModel";
  for (const [v, text] of [["haiku", "Haiku (fast)"], ["sonnet", "Sonnet"], ["opus", "Opus"]] as const) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    modelSel.append(o);
  }
  modelSel.value = model.interpreterModel;
  modelSel.addEventListener("change", () => h.onInterpreter(modelSel.value));
  modelLabel.append(modelSel);
  assistant.append(modelLabel);

  const providerLabel = document.createElement("label");
  providerLabel.textContent = "Voice";
  const provider = document.createElement("select");
  provider.name = "voiceProvider";
  for (const [v, text] of [["builtin", "Samantha (built in)"], ["elevenlabs", "ElevenLabs"]] as const) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = text;
    provider.append(o);
  }
  provider.value = model.voiceProvider;
  provider.addEventListener("change", () => h.onVoiceProvider(provider.value === "elevenlabs" ? "elevenlabs" : "builtin"));
  providerLabel.append(provider);
  notifications.append(providerLabel);

  if (model.voiceProvider === "elevenlabs") {
    const keyLabel = document.createElement("label");
    keyLabel.textContent = "ElevenLabs API key";
    const key = document.createElement("input");
    key.type = "password";
    key.name = "elevenKey";
    key.placeholder = model.elevenKeySet ? "saved in Keychain; paste to replace" : "paste your key";
    key.autocomplete = "off";
    key.addEventListener("change", () => {
      const k = key.value.trim();
      if (k) h.onElevenKey(k);
      key.value = "";
    });
    keyLabel.append(key);
    notifications.append(keyLabel);

    const voiceLabel = document.createElement("label");
    voiceLabel.textContent = "ElevenLabs voice";
    const voice = document.createElement("select");
    voice.name = "elevenVoice";
    for (const v of model.elevenVoices) {
      const o = document.createElement("option");
      o.value = v.voiceId;
      o.textContent = v.name;
      voice.append(o);
    }
    voice.value = model.elevenVoices.some((v) => v.voiceId === model.elevenVoiceId) ? model.elevenVoiceId : (model.elevenVoices[0]?.voiceId ?? "");
    voice.disabled = model.elevenVoices.length === 0;
    voice.addEventListener("change", () => h.onElevenVoice(voice.value));
    voiceLabel.append(voice);
    notifications.append(voiceLabel);
  }

  const tryBtn = document.createElement("button");
  tryBtn.type = "button";
  tryBtn.dataset.action = "try-voice";
  tryBtn.textContent = "Try the voice";
  tryBtn.addEventListener("click", () => h.onTryVoice());
  notifications.append(tryBtn);

  if (model.error) {
    const err = document.createElement("div");
    err.className = "settings__error";
    err.textContent = model.error;
    root.append(err);
  }
  return root;
}

export async function initSettings(): Promise<void> {
  const panel = document.getElementById("settings");
  if (!panel) return;

  const model: SettingsModel = { hookInstalled: null, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenKeySet: false, elevenVoices: [], elevenVoiceId: "", error: null, listen: false, microphone: "", microphones: [], interpreterModel: "haiku", listenError: null };

  const loadVoices = async () => {
    if (model.voiceProvider !== "elevenlabs") return;
    model.elevenKeySet = await invoke<boolean>("has_elevenlabs_key");
    if (!model.elevenKeySet) return;
    model.elevenVoices = await invoke<ElevenVoice[]>("list_elevenlabs_voices");
  };

  const saveConfig = async (patch: Partial<ConfigJson>) => {
    // The local model can be stale for fields it doesn't own (e.g. `listen`,
    // which the voice panel can flip independently), so fetch the freshest
    // config and merge the patch onto that rather than onto `model`.
    const fresh = await invoke<ConfigJson>("get_config");
    const c = await invoke<ConfigJson>("set_config", {
      config: { ...fresh, ...patch },
    });
    model.completedTimeoutMinutes = c.completedTimeoutMinutes;
    model.projectsDir = c.projectsDir ?? "";
    model.clonesDir = c.clonesDir ?? "";
    model.notifyOnAwaiting = c.notifyOnAwaiting;
    model.speakNotifications = c.speakNotifications;
    model.voiceProvider = c.voiceProvider;
    model.elevenVoiceId = c.elevenlabsVoiceId ?? "";
    model.listen = c.listen;
    model.microphone = c.microphone ?? "";
    model.interpreterModel = c.interpreterModel;
  };

  const paint = () => panel.replaceChildren(renderSettings(model, handlers));

  const run = async (action: () => Promise<void>) => {
    model.error = null;
    try {
      await action();
    } catch (e) {
      model.error = String(e);
    }
    paint();
  };

  const handlers: SettingsHandlers = {
    onInstall: () => void run(async () => { model.hookInstalled = await invoke<boolean>("install_hook"); }),
    onRemove: () => void run(async () => { model.hookInstalled = await invoke<boolean>("remove_hook"); }),
    onTimeout: (minutes) => void run(() => saveConfig({ completedTimeoutMinutes: minutes })),
    onProjectsDir: (path) => void run(() => saveConfig({ projectsDir: path || null })),
    onNotify: (enabled) => void run(() => saveConfig({ notifyOnAwaiting: enabled })),
    onClonesDir: (path) => void run(() => saveConfig({ clonesDir: path || null })),
    onSpeak: (enabled) => void run(() => saveConfig({ speakNotifications: enabled })),
    onVoiceProvider: (provider) => void run(async () => { await saveConfig({ voiceProvider: provider }); await loadVoices(); }),
    onElevenKey: (key) => void run(async () => { await invoke("set_elevenlabs_key", { key }); await loadVoices(); }),
    onElevenVoice: (voiceId) => void run(() => saveConfig({ elevenlabsVoiceId: voiceId || null })),
    onTryVoice: () => void run(() => invoke("try_voice")),
    onListen: (on) => void run(async () => { await invoke("voice_listen", { on }); model.listen = on; }),
    onMicrophone: (name) => void run(() => saveConfig({ microphone: name || null })),
    onInterpreter: (m) => void run(() => saveConfig({ interpreterModel: m })),
  };

  // The voice panel can turn listening on or off on its own; mirror that
  // into the model so the Settings checkbox doesn't show a stale state.
  // Every partial transcript emits `voice`: repaint only when listening
  // actually changed, or a repaint would wipe a field being typed in.
  await listen<VoiceStatus>("voice", (e) => {
    const listenError = e.payload.state === "error" && e.payload.detail ? e.payload.detail : null;
    if (model.listen === e.payload.listening && model.listenError === listenError) return;
    model.listen = e.payload.listening;
    model.listenError = listenError;
    if (!panel.hidden) paint();
  });

  await run(async () => {
    const [installed, config] = await Promise.all([invoke<boolean>("hook_status"), invoke<ConfigJson>("get_config")]);
    model.hookInstalled = installed;
    model.completedTimeoutMinutes = config.completedTimeoutMinutes;
    model.projectsDir = config.projectsDir ?? "";
    model.clonesDir = config.clonesDir ?? "";
    model.notifyOnAwaiting = config.notifyOnAwaiting;
    model.speakNotifications = config.speakNotifications;
    model.voiceProvider = config.voiceProvider;
    model.elevenVoiceId = config.elevenlabsVoiceId ?? "";
    model.listen = config.listen;
    model.microphone = config.microphone ?? "";
    model.interpreterModel = config.interpreterModel;
    await loadVoices();
    try {
      const raw = await invoke<string>("voice_selftest");
      const parsed = JSON.parse(raw) as { devices?: string[] };
      model.microphones = parsed.devices ?? [];
    } catch {
      // ignore: the microphone list is a nicety, not required to use settings
    }
  });
}
