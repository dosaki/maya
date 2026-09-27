import { invoke } from "@tauri-apps/api/core";

export interface SettingsModel {
  hookInstalled: boolean | null;
  completedTimeoutMinutes: number;
  error: string | null;
}

export interface SettingsHandlers {
  onInstall(): void;
  onRemove(): void;
  onTimeout(minutes: number): void;
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

  const model: SettingsModel = { hookInstalled: null, completedTimeoutMinutes: 30, error: null };

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
    onTimeout: (minutes) =>
      void run(async () => {
        const c = await invoke<{ completedTimeoutMinutes: number }>("set_config", { config: { completedTimeoutMinutes: minutes } });
        model.completedTimeoutMinutes = c.completedTimeoutMinutes;
      }),
  };

  toggle.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
  });

  await run(async () => {
    const [installed, config] = await Promise.all([
      invoke<boolean>("hook_status"),
      invoke<{ completedTimeoutMinutes: number }>("get_config"),
    ]);
    model.hookInstalled = installed;
    model.completedTimeoutMinutes = config.completedTimeoutMinutes;
  });
}
