import { invoke } from "@tauri-apps/api/core";
import { showToast } from "./toast";

const SPEAKER = '<path d="M2.5 6 H5 L8.5 3 V13 L5 10 H2.5 Z" fill="currentColor"/>';
const WAVES = '<path d="M10.5 5.5 a3.5 3.5 0 0 1 0 5 M12.5 3.5 a6.5 6.5 0 0 1 0 9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>';
const CROSS = '<path d="M10.5 6 L14 9.5 M14 6 L10.5 9.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>';

/**
 * The top-bar speaker button, pressed while Maya is muted: she then makes
 * no sound at all but still notifies.
 */
export function renderMuteButton(muted: boolean): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.dataset.action = "mute";
  b.className = muted ? "voice mute mute--on" : "voice mute";
  b.setAttribute("aria-pressed", String(muted));
  b.setAttribute("aria-label", muted ? "Unmute Maya" : "Mute Maya");
  b.title = muted ? "Muted: Maya makes no sound but still notifies you. Click to unmute." : "Mute Maya";
  b.innerHTML = `<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">${SPEAKER}${muted ? CROSS : WAVES}</svg>`;
  return b;
}

/** Mounts the mute button and saves each toggle. */
export async function initMute(): Promise<void> {
  const host = document.getElementById("mute-host");
  if (!host) return;
  let muted = (await invoke<{ muted?: boolean }>("get_config")).muted ?? false;
  const paint = () => host.replaceChildren(renderMuteButton(muted));
  host.addEventListener("click", async () => {
    try {
      muted = (await invoke<{ muted?: boolean }>("set_muted", { muted: !muted })).muted ?? !muted;
    } catch (e) {
      showToast(String(e));
    }
    paint();
  });
  paint();
}
