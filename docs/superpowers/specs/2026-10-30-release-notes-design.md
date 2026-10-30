# Release notes

Date: 2026-10-30. Status: reviewed by a builder check, awaiting approval.

## Intent

Every release of Maya carries notes written for the people who use it:
what is new and what is fixed, in plain words. The pull request that
bumps the version writes them, in `RELEASE_NOTES.md`. The release on
GitHub shows them under a "Release Notes" heading, and the app shows them
in Settings → Release Notes, on the desktop and on the phone.

### Why

Today a release's only notes are GitHub's generated list of pull request
titles, which say what changed in the code, not what changed for the
user, and the app shows no notes at all. Someone updating Maya cannot
see what the update gave them without reading commit subjects on GitHub.

### Who

The people who use Maya, on the desktop and on the phone. The notes are
written by whoever opens the version-bumping pull request (often Claude,
under the rule this adds to `CLAUDE.md`).

## Goals

- `RELEASE_NOTES.md` at the repository root holds the notes for the
  version in the version files, and only that version.
- Each pull request that bumps the version rewrites it, in the same
  pull request.
- The GitHub release for `v<version>` shows the notes under
  `## Release Notes`, above the install text and GitHub's generated
  "What's Changed" list.
- The desktop app's Settings has a Release Notes group, the last entry
  of its sidebar.
- The phone's tab "Network" becomes "Settings", laid out the way
  Android's own Settings is: one list of groups, tap into a group to see
  its contents. Release Notes is one of the groups.
- CI fails a pull request whose `RELEASE_NOTES.md` is for a different
  version than the version files.
- `CLAUDE.md` tells whoever opens a pull request to write the notes with
  the bump and keep them current until the push.

## Non-goals

- No history of past notes, in the repository or in the app. The file
  holds one version; older notes live on their GitHub releases.
- No notes for an update that is offered but not yet installed. The app
  shows the notes of the build it is.
- No change to the desktop Settings beyond the new group.
- No change to what the phone's settings do: every field, switch and
  button keeps its behaviour, only where it sits changes.

## Anti-goals

- A release published with notes for another version.
- Notes that read like a commit log. They are for users.
- A Release Notes page that needs the network, or can fail to load.

## Constraints

- The version lives in `src-tauri/tauri.conf.json`, kept in step with
  `mobile/tauri.conf.json`, `package.json`, `Cargo.toml` and
  `Cargo.lock` by `scripts/set-version.sh`. `scripts/release-version.sh`
  checks they agree, in CI on every pull request (`ci.yml`, job Version)
  and on every push to `main` (`release.yml`).
- `CLAUDE.md` already says a pull request that changes what ships bumps
  the version once, in a commit of its own named
  `chore(release): <version>`, redone if `main` moves on first.
- The desktop and phone pages are both built by Vite from `src/`
  (`vite.config.ts`, `vite.mobile.config.ts`, whose build root is
  `mobile/web`).
- `src/markdown.ts`'s `renderMarkdown` turns Markdown into a sanitised
  element.
- The phone's settings pane is `#settings`, mounted by `initNetwork` in
  `src/mobile/network.ts`, which repaints it whole
  (`pane.replaceChildren(renderNetwork(...))`) and carries typed values
  across repaints with `carryEdits`/`restoreEdits`.
- Android's back gesture reaches the page as `popstate`;
  `src/mobile/card-history.ts` already uses a history entry so Back
  closes an open card, and `src/mobile/main.ts` closes the card on
  `popstate`.

## Approaches

How the app gets the notes:

1. **Built into the page (chosen).** The Release Notes page imports
   `RELEASE_NOTES.md` as text at build time (Vite's `?raw`) and renders
   it with `renderMarkdown`. No Rust change, works offline, always the
   notes of the running build.
2. **Built into the Rust side.** `include_str!` in both apps and a
   `release_notes` command in `src-tauri` and `mobile`. Same result, two
   commands to write and test, and an async call for text that never
   changes.
3. **Fetched from the GitHub release.** Could show an offered update's
   notes, but needs the network, can show a version other than the one
   running, and adds a failure case to a static page.

The user chose 1: the least code, and nothing to fail at runtime.

## Design

### The file

`RELEASE_NOTES.md`, at the repository root:

```markdown
# 0.42.0

## New

- Settings has a Release Notes page, on the desktop and on the phone.
- The phone's Network tab is now Settings, with its options in groups.

## Fixed

- ...
```

- The first line is `# <version>`, exactly the version in the version
  files.
- Then `## New` and `## Fixed`, each a list of short bullets; a group
  with nothing in it is left out. Each bullet says what changed for the
  user, in plain words.
- A pull request that bumps the version rewrites the whole file for the
  new version, in the same commit as the bump
  (`chore(release): <version>`). A pull request that does not bump
  leaves it alone.
- When `main` moves on and the bump is redone, the notes are redone with
  it, covering everything that ships in the new version.

### The check

`scripts/release-version.sh` also reads the version on the first line of
`RELEASE_NOTES.md` (`# <version>`) and fails, the same way as a version
mismatch, when it is missing or differs from `tauri.conf.json`'s,
naming both. Because the script runs in CI on every pull request and on
every push to `main` before anything is released, a bump without notes
cannot merge, and a release never goes out with another version's notes.

### The GitHub release

In `release.yml`'s Release job, the body passed to `gh release create`
becomes, in this order:

1. `## Release Notes`
2. `RELEASE_NOTES.md` without its first line (the `# <version>` line;
   the release's title already says the version). Its `## New` and
   `## Fixed` stay as they are.
3. The install text that is the whole body today ("Built from ... on
   main. macOS builds are not notarised: ...").
4. GitHub's generated "What's Changed" list (`--generate-notes`, as
   today).

The text is assembled into a file and passed with `--notes-file`, so
Markdown and quotes in the notes need no shell escaping.

### Desktop: Settings → Release Notes

- `SETTINGS_GROUPS` in `src/settings.ts` gains `["releaseNotes",
  "Release Notes"]` as its last entry, after Debug.
- The group's card is titled "Release Notes" like every other group. It
  shows "Maya <version>" as a hint line, then the file's body (without
  its first line) rendered with `renderMarkdown`.
- Where the version comes from is the builder's call: the file's first
  line or the app's own version. CI keeps them equal.

### Phone: Settings, Android style

The tab button in `mobile/web/index.html` reads "Settings" instead of
"Network".

**The list.** The pane opens on a list of groups, one row each: the
group's name, a one-line summary of what is set under it, and a chevron.
Tapping a row opens the group's page.

| Group | Its page holds (today's cards) | Summary line, for example |
|---|---|---|
| Network | This phone, then Join a main (or Key, once joined) | "Connected to studio-mac", "Reconnecting…", "Not joined" |
| Notifications | Notifications, then Battery | "Awaiting decision, Completed", "Off" |
| Voice | Voice | "Speak on, wake word off" |
| Session viewer | Session viewer | "Chat" or "Terminal" |
| Release Notes | the notes, as on the desktop | "Maya 0.42.0" |

Battery ("Keep Maya awake") goes under Notifications because it mostly
keeps notifications coming with the screen off. Exact summary wording is
the builder's call; it reflects the model the page already holds.

**A group's page.** A bar with a back arrow (←) and the group's name,
then that group's cards exactly as the pane shows them today, with the
same behaviour and busy state.

**Errors.** Today one shared error (`model.error`, set by any action that
goes through `busy()`) shows only inside the "This phone" card, which
moves to the Network page. A failed Voice, Notifications or Session
viewer save would then be invisible. Instead, the error shows at the top
of whichever group's page is open (on the Network page, where it sits
today). Our call: the user sees a failure where they caused it.

**Back.** Opening a group pushes a history entry, as an open card does.
Android's back gesture (`popstate`) and the ← both return to the list;
the ← pops the entry so Back is never needed twice to leave the app. The
phone does not remember the open group: switching to another tab and
back lands on the list, and leaving the tab drops the history entry.

**Repaints.** The pane repaints whole, as today. The open group is part
of the model, so a repaint (a status change, a save) keeps the user on
the page they are on, with their typed values carried as today.

### Docs

- `CLAUDE.md`, in "Pull requests bump the version": rewrite
  `RELEASE_NOTES.md` for the new version, for users and in plain words,
  committed with the bump in `chore(release): <version>`; redo it
  whenever the bump is redone; and before every push, check it still
  describes everything the branch ships, since later commits change
  that.
- `docs/DEVELOPING.md`, "Releasing": the release body now leads with the
  notes from `RELEASE_NOTES.md`, and the version check covers the file.
  Its example commit message (`chore: release 0.2.0`) changes to
  `chore(release): 0.2.0` with the notes in it, to match `CLAUDE.md`.
- `README.md`: mention Settings → Release Notes where the README
  describes Settings or updates, if it does.

### This pull request's own release

This work is a `feat`, so the pull request bumps `0.41.0` (on `main`
today) to `0.42.0` and writes the first `RELEASE_NOTES.md` for it.

## Tests

- `scripts/release-version.sh`: passes with matching notes; fails, naming
  both versions, when the first line differs or the file is missing.
  (A small shell test beside `scripts/test-updater-manifest.sh`, run in
  CI's Version job, is the builder's call.)
- Desktop: the Release Notes group is in the sidebar, last, and shows the
  version and the rendered notes.
- Phone (Vitest, jsdom, as `src/mobile/network.test.ts`): the pane opens
  on the list with the five groups and their summaries; a tap opens the
  group's page with today's cards; ← and `popstate` return to the list;
  a repaint keeps the open group; switching tabs lands on the list;
  existing network tests still pass against the new structure.

## Left to the builder

- Where the version shown in the app comes from (file or app version).
- Summary line wording, and the list's styling within the existing
  theme.
- How the phone's settings model holds the open group, and how its
  history entry coexists with the card's (`card-history.ts`).
- Whether the release body is assembled inline in `release.yml` or by a
  small script.
