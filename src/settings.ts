import { invoke } from "@tauri-apps/api/core";

export interface SettingsModel {
  hookInstalled: boolean | null;
  completedTimeoutMinutes: number;
  projectsDir: string;
  clonesDir: string;
  notifyOnAwaiting: boolean;
  speakNotifications: boolean;
  error: string | null;
}

export interface SettingsHandlers {
  onInstall(): void;
  onRemove(): void;
  onTimeout(minutes: number): void;
  onProjectsDir(path: string): void;
  onNotify(enabled: boolean): void;
  onClonesDir(path: string): void;
  onSpeak(enabled: boolean): void;
}

interface ConfigJson {
  completedTimeoutMinutes: number;
  projectsDir?: string | null;
  clonesDir?: string | null;
  notifyOnAwaiting: boolean;
  speakNotifications: boolean;
}

export function renderSettings(model: SettingsModel, h: SettingsHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "settings__body";

  const status = document.createElement("div");
  status.className = "settings__status";
  status.textContent =
    model.hookInstalled === null
      ? "Checking hook…"
      : model.hookInstalled
        ? "Claude Code hook is installed. Awaiting Decision and Completed are precise."
        : "Claude Code hook is not installed. Permission prompts will show as Working.";
  root.append(status);

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
  root.append(btn);

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
  root.append(label);

  const dirLabel = document.createElement("label");
  dirLabel.textContent = "Projects directory";
  const dirInput = document.createElement("input");
  dirInput.type = "text";
  dirInput.name = "projectsDir";
  dirInput.placeholder = "~/dev";
  dirInput.value = model.projectsDir;
  dirInput.addEventListener("change", () => h.onProjectsDir(dirInput.value.trim()));
  dirLabel.append(dirInput);
  root.append(dirLabel);

  const clonesLabel = document.createElement("label");
  clonesLabel.textContent = "Clones directory (for PR reviews)";
  const clonesInput = document.createElement("input");
  clonesInput.type = "text";
  clonesInput.name = "clonesDir";
  clonesInput.placeholder = "~/dev/reviews";
  clonesInput.value = model.clonesDir;
  clonesInput.addEventListener("change", () => h.onClonesDir(clonesInput.value.trim()));
  clonesLabel.append(clonesInput);
  root.append(clonesLabel);

  const notifyLabel = document.createElement("label");
  notifyLabel.className = "settings__check";
  const notifyBox = document.createElement("input");
  notifyBox.type = "checkbox";
  notifyBox.name = "notify";
  notifyBox.checked = model.notifyOnAwaiting;
  notifyBox.addEventListener("change", () => h.onNotify(notifyBox.checked));
  notifyLabel.append(notifyBox, document.createTextNode(" Notify me when a session awaits a decision"));
  root.append(notifyLabel);

  const speakLabel = document.createElement("label");
  speakLabel.className = "settings__check";
  const speakBox = document.createElement("input");
  speakBox.type = "checkbox";
  speakBox.name = "speak";
  speakBox.checked = model.speakNotifications;
  speakBox.addEventListener("change", () => h.onSpeak(speakBox.checked));
  speakLabel.append(speakBox, document.createTextNode(" Speak instead of a sound (\"needs a decision\", \"is finished\")"));
  root.append(speakLabel);

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
  const toggle = document.getElementById("settings-toggle");
  if (!panel || !toggle) return;

  const model: SettingsModel = { hookInstalled: null, completedTimeoutMinutes: 30, projectsDir: "", clonesDir: "", notifyOnAwaiting: true, speakNotifications: true, error: null };

  const saveConfig = async (patch: Partial<ConfigJson>) => {
    const c = await invoke<ConfigJson>("set_config", {
      config: { completedTimeoutMinutes: model.completedTimeoutMinutes, projectsDir: model.projectsDir || null, clonesDir: model.clonesDir || null, notifyOnAwaiting: model.notifyOnAwaiting, speakNotifications: model.speakNotifications, ...patch },
    });
    model.completedTimeoutMinutes = c.completedTimeoutMinutes;
    model.projectsDir = c.projectsDir ?? "";
    model.clonesDir = c.clonesDir ?? "";
    model.notifyOnAwaiting = c.notifyOnAwaiting;
    model.speakNotifications = c.speakNotifications;
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
  };

  toggle.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
  });

  await run(async () => {
    const [installed, config] = await Promise.all([invoke<boolean>("hook_status"), invoke<ConfigJson>("get_config")]);
    model.hookInstalled = installed;
    model.completedTimeoutMinutes = config.completedTimeoutMinutes;
    model.projectsDir = config.projectsDir ?? "";
    model.clonesDir = config.clonesDir ?? "";
    model.notifyOnAwaiting = config.notifyOnAwaiting;
    model.speakNotifications = config.speakNotifications;
  });
}
