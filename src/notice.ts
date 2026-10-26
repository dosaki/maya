// A one-time notice after the update that made Enter send everywhere: the
// reply box and the new-session prompt sent on ⌘↵ before, and the keys are
// now in Settings › Keyboard.

import { invoke } from "@tauri-apps/api/core";
import { needsFirstRun } from "./firstrun";
import { comboLabel, defaultKeys } from "./keys";
import type { Harness } from "./types";

/** Set once the notice was shown, or once a fresh install made it moot. */
export const KEYS_NOTICE_KEY = "maya.notice.keys";

/** Marks the notice as shown; false when the page cannot remember it. */
function markShown(): boolean {
  try {
    localStorage.setItem(KEYS_NOTICE_KEY, "shown");
    return true;
  } catch {
    return false;
  }
}

function shownBefore(): boolean {
  try {
    return localStorage.getItem(KEYS_NOTICE_KEY) !== null;
  } catch {
    return true;
  }
}

/**
 * On the first start after the update, tells someone who used Maya before
 * that Enter now sends. A fresh install, someone who already chose keys, or
 * a page that could not remember showing it (and would show it every start)
 * gets nothing.
 */
export async function maybeShowKeysNotice(openKeyboard: () => void): Promise<void> {
  if (shownBefore()) return;
  let config: { agent?: Harness | null; keys?: unknown };
  try {
    config = await invoke("get_config");
  } catch {
    return;
  }
  // A fresh install starts with the new keys; it has no old habit to warn about.
  if (needsFirstRun(config) || config.keys) {
    markShown();
    return;
  }
  if (!markShown()) return;

  const box = document.createElement("div");
  box.className = "notice";
  box.setAttribute("role", "status");
  const text = document.createElement("p");
  text.textContent = `Message keys have changed: Enter now sends in a session's reply box and the new-session prompt, and ${comboLabel(defaultKeys().newLine[1])} makes a new line there. You can choose your own keys in Settings › Keyboard.`;
  const open = document.createElement("button");
  open.type = "button";
  open.dataset.action = "notice-open";
  open.textContent = "Choose keys";
  open.addEventListener("click", () => {
    box.remove();
    openKeyboard();
  });
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.dataset.action = "notice-dismiss";
  dismiss.textContent = "Got it";
  dismiss.addEventListener("click", () => box.remove());
  const actions = document.createElement("div");
  actions.className = "notice__actions";
  actions.append(open, dismiss);
  box.append(text, actions);
  document.body.append(box);
}
