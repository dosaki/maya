# What's new after an update

## Intent

The first time Maya opens on a newer version, it shows what changed since
the version that device last showed, with Maya's full-body art
(`src/assets/maya-full-body.png`) presenting the notes. Today the notes are
only in Settings › Release Notes, where nobody looks after an update.

## Goals

- On the computer and on the phone, after an update, show every version's
  notes newer than the last version this device showed, newest first.
- Show it once per version: closing it counts as seen.

## Non-goals

- A brand-new install shows nothing. (The user has plans for a first-run
  experience later.)
- The `maya` command shows nothing: it has no window, and a line on every
  run after an update would be noise.
- No change to `RELEASE_NOTES.md` or to Settings › Release Notes.

## Anti-goals

- It must not show on a brand-new install, nor again for a version already
  seen, nor after going back to an older version.

## When it shows

Each device keeps, in its settings file, the last version it showed
(`last_shown_version` or similar).

| Saved version | Settings file existed at start | Shows |
|---|---|---|
| none | no (brand-new install) | nothing; saves the running version |
| none | yes (an existing user's first update to the version that ships this) | only the running version's notes, titled "What's new in Maya \<version>", no "Updated from" |
| older than running | yes | every version newer than the saved one, titled "What's new in Maya \<version>" with "Updated from \<saved>" and how many versions that is |
| equal or newer | yes | nothing (a downgrade saves nothing) |

- "Got it", the ×, Escape and a click outside all close it and save the
  running version. If Maya quits before it is closed, it shows again next
  time.
- The notes come from the bundled `RELEASE_NOTES.md` (`RELEASE_NOTES` in
  `src/release-notes.ts`), compared by version number, not by string.
  Versions the file no longer has (across a major release: the file starts
  afresh at 1.0.0) are simply not shown.

## Looks (picked from the mockups)

- **Computer: look B, "Maya steps out".** A modal over the board, its
  backdrop dimmed. Maya stands outside the modal's left edge, full height,
  her open hand toward the notes, with a soft gold glow; no panel behind her.
  The modal: a small gold "Updated from 0.46.0" line, the title "What's new
  in Maya 0.48.0", "2 versions since you last opened Maya.", then each
  version under "Maya \<version>" with its New and Fixed lists (the running
  one marked "This version"), scrolling; a footer with "All release notes
  are in Settings › Release Notes." and a gold "Got it" button.
- **Phone: look A, a page like its other dialogs.** It opens under the top
  bar as a session or Settings does, with the × in the header. A banner at
  the top shows Maya's head and raised hand, fading into the page, with the
  kicker and "What's new" over it; the notes scroll below; a full-width
  "Got it" sits at the bottom with "All notes: Settings › Release Notes"
  under it.
- Both use the app's black-and-gold tokens from `src/styles.css` and the
  existing release-notes markup (`renderReleaseNotes`), so the notes read as
  they do in Settings.

The mockups are in `.superpowers/brainstorm/` (not committed).

## Approaches

1. **The settings file (chosen).** A new field in `Config`
   (`core/src/config.rs`), read and saved through the existing
   `update_config` on the computer (`src-tauri`) and the phone
   (`mobile/src/hub.rs`). At start the front end asks the backend once
   whether to show, and from which version; the backend answers from the
   field, the running version, and whether the settings file existed when
   it was loaded. "Got it" asks the backend to save the running version.
   Rules get Rust tests like the rest of the settings.
2. **The web view's `localStorage`.** Least code, but its storage can be
   cleared apart from Maya's settings, so the window would wrongly stay
   quiet or come back; and it still needs the backend for "did the settings
   exist?".
3. **Record the old version before the updater installs.** Ruled out: it
   misses the phone's APK, a manual download, and Linux builds that only
   link to the release.

The user chose 1.

## Constraints

- "The settings file existed" is decided when the settings are first
  loaded, before anything at start can save them (a migration, the phone's
  role fix in `load_settings`), or every brand-new install would look like
  an existing user.
- The modal is shared front-end code for the computer and the phone, as
  `src/devices.ts` is; nothing in it may import desktop-only code.
- Repaint in place if it ever repaints (CLAUDE.md lore on `:hover`); it is
  static once shown, so it should not need to.
- Ships as a `feat`: bump from `main`'s version (0.49.3 → 0.50.0) with a
  release note, per CLAUDE.md.

## Left to the builder

- Field and command names.
- The exact art placement and glow on the computer, and the banner crop on
  the phone, within the looks above; and how the computer look behaves on a
  narrow window (the art may hide below a width).
- Whether it waits for other start-up dialogs (a first-run setup on the
  phone never coincides with it, since that is a brand-new install).
