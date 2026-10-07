// Opening the card a tapped notification names. The Rust side puts the
// session id in the notification's `extra`; the plugin's action event has
// carried it both at the top level and under `notification`.

import { onAction } from "@tauri-apps/plugin-notification";

export function tappedSession(payload: unknown): string | null {
  if (!payload || typeof payload !== "object") return null;
  const p = payload as { extra?: Record<string, unknown>; notification?: { extra?: Record<string, unknown> } };
  const id = (p.extra ?? p.notification?.extra)?.sessionId;
  return typeof id === "string" && id !== "" ? id : null;
}

/** Calls `open` with the session of every tapped notification; quiet where the plugin has no actions (a desktop run). */
export async function watchTaps(open: (sessionId: string) => void): Promise<void> {
  try {
    await onAction((n) => {
      const id = tappedSession(n);
      if (id) open(id);
    });
  } catch {
    // no mobile plugin here
  }
}
