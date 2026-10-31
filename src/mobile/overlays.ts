// On the phone, a session's window and the new-session and resume dialogs
// are a pane under the top bar (see mobile.css). Tapping a tab leaves none
// of them open over the pane it shows: the session closes as its × does, so
// the history entry it holds is popped and Back still leaves the app.

import { dismissModal } from "../modal";
import { closeNewSession } from "../newsession";
import { closeResume } from "../resume";

/** Closes the session window, the new-session form and the resume list, whichever is open. */
export function closeOverlays(): void {
  dismissModal();
  closeNewSession();
  closeResume();
}

/**
 * Closes whatever is open when the person taps a tab, Sessions included.
 * Only taps: the page switching tab itself (a notification tap, which opens
 * its card first, or Chat going away) must not close the card it shows.
 */
export function closeOverlaysOnTabTaps(root: ParentNode = document): void {
  for (const btn of root.querySelectorAll<HTMLElement>("[data-tab]")) btn.addEventListener("click", closeOverlays);
}
