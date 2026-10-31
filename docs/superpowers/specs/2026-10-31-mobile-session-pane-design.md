# The phone's session window as a pane under the top bar

## Intent

On the phone, tapping a session's reply box does not bring up the keyboard.
Chat's box, which sits in the page under the top bar, does. The session
window, the new-session dialog and the resume dialog all open in a layer
pinned over the whole screen (`#modal-host` > `.modal`, `position: fixed`),
and three earlier fixes (#63, #65, #70) bent that layer around the keyboard.
On the phone, show that layer as a pane in the page instead, like Chat, and
let Chat's keyboard handling cover it.

The root cause is not proven: no phone was attached to inspect the web view.
The pinned layer is the main difference between the box that works and the
box that does not, and this change removes it. A try on the phone is the
check.

## Goals

- On the phone, the session window opens under the top bar, which stays on
  screen, and fills the rest of the page.
- Tapping its reply box brings up the keyboard, and the box stays above it.
- The new-session and resume dialogs open the same way, since they share
  the layer.

## Non-goals

- No change on the desktop: `.modal` keeps its window, drag-to-resize and
  backdrop there.
- No change to what the session window shows or does.

## Approaches

- **A. On the phone, show the layer as a pane, with CSS only (picked).** In
  `mobile.css`, `#modal-host` becomes a flex child of `#app`'s column that
  takes the space left under the top bar, and `.modal` inside it stops being
  pinned. While `#modal-host` holds something, the board and the rows above
  it (strip, mic bar, remembered row) are hidden. The smallest change.
- **B. A real "session" tab managed by `makeTabs`.** `tabs.ts` is shared
  with the desktop, so it would need a tab that only the phone has, and Back
  would work differently. More change for the same result.
- **C. Keep the pinned layer, starting below the top bar.** Keeps the
  pinned layer, the suspected cause. Dropped.

## Decided

- The top bar and its tabs stay on screen over an open session.
- Tapping a tab closes an open session, as × does (so the card's history
  entry is popped and Back keeps working), and closes an open new-session or
  resume dialog.
- A notification tap still switches to Sessions and then opens the card.
- The pinned layer's keyboard workaround from #70 goes: `trackVisibleArea`,
  the `--visible-top` / `--visible-height` rule on `.modal`, and their tests.
  `keepFocusedAboveKeyboard` stays: it is what keeps Chat's box clear.

## Left to the builder

- The exact CSS for hiding the board while the layer holds something
  (`:has()` works in Android's web view, or a class set from `main.ts`).
- Whether the desktop's edge-resize handler needs to stay off on the phone
  (its band is 4 px, so likely not).
