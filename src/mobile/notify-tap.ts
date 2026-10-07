// Opening the card a tapped notification names. The Rust side puts the
// session id in the notification's `extra`, but on Android the plugin's
// action event leaves the notification out, and a tap that launched the app
// after Android killed it fires before the page listens. So the app keeps
// every tap, and `pending_tap` hands over its session once: right after the
// listener is up, and on each action event that does not name a session.

import { invoke } from "@tauri-apps/api/core";
import { onAction } from "@tauri-apps/plugin-notification";

export function tappedSession(payload: unknown): string | null {
  if (!payload || typeof payload !== "object") return null;
  const p = payload as { extra?: Record<string, unknown>; notification?: { extra?: Record<string, unknown> } };
  const id = (p.extra ?? p.notification?.extra)?.sessionId;
  return typeof id === "string" && id !== "" ? id : null;
}

/** The session of a notification tapped before the page listened, taken once. */
async function takePending(): Promise<string | null> {
  return invoke<string | null>("pending_tap");
}

/**
 * Calls `open` with the session of every tapped notification, then once for
 * a tap that came before the listener; quiet where the plugin has no actions
 * or the command is missing (a desktop run).
 */
export async function watchTaps(open: (sessionId: string) => void, pending: () => Promise<string | null> = takePending): Promise<void> {
  try {
    await onAction((n) => {
      const id = tappedSession(n);
      if (id) {
        open(id);
        // The app kept this tap too: drop that copy.
        pending().catch(() => undefined);
      } else {
        pending()
          .then((kept) => {
            if (kept) open(kept);
          })
          .catch(() => undefined);
      }
    });
  } catch {
    // no mobile plugin here
  }
  try {
    const id = await pending();
    if (id) open(id);
  } catch {
    // no pending-tap command here
  }
}
