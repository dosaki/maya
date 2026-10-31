// On the phone, a session's window and the new-session and resume dialogs
// are a pane under the top bar (see mobile.css). Switching tabs leaves none
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
