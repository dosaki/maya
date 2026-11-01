# Maya

Building, testing, the layout and the release workflow are in
[docs/DEVELOPING.md](docs/DEVELOPING.md).

## Pull requests bump the version

Every pull request that changes what ships bumps the version, in the same
pull request, so that merging it publishes a release. Merging to `main`
publishes `v<version>` whenever no release with that version exists yet;
a pull request that leaves the version alone releases nothing.

- Bump from the version on `main` (it may have moved since the branch was
  made), following semver and the pull request's commits:
  - a breaking change: the major version (minor before 1.0);
  - any `feat`: the minor version;
  - only `fix` or `perf`: the patch version;
  - only `docs`, `ci`, `test` or `chore`: no bump, since nothing ships.
- Set it with `sh scripts/set-version.sh <major.minor.patch>`, which keeps
  `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml` and
  `Cargo.lock` in step (CI fails when they disagree), and commit it on its
  own as `chore(release): <version>`.
- Bump once per pull request. If `main` moves on before the merge, redo
  the bump against `main`'s new version.

## Release notes

`RELEASE_NOTES.md` holds the notes for every version of the current major
release, newest first, each under a `# <version>` line. The release on
GitHub leads with the top version's notes, and the app shows the whole
file in Settings → Release Notes.

- With the bump, add a section at the top of `RELEASE_NOTES.md` for the
  new version: a line `# <version>`, then `## New` and `## Fixed` (leave
  out a group with nothing in it). Each bullet says what changed for the
  people who use Maya, in plain words, not commit subjects, and together
  they cover everything the version ships. Leave the older sections as
  they are.
- A new major version starts the file afresh with its own section; the
  older major's notes stay on its GitHub releases. (0.x counts as one
  major, so the file starts afresh at 1.0.0.)
- Commit it in the same `chore(release): <version>` commit as the bump.
  When the bump is redone, redo the top section with it, for the new
  version and everything it now ships.
- Before every push, check the top section still names the version and
  describes everything the branch ships: later commits change that. CI
  fails when its version differs from the version files.
- Write `\<word>`, not `<word>`, for a placeholder: Markdown drops an
  unescaped one as a tag.

## Lore

- **Pitfall:** Maya's WKWebView drops `:hover` (and a tooltip, a selection) from a node that is replaced, until the pointer moves; repaint with `morph` or `keepEqualChildren` from `src/morph.ts` instead of `replaceChildren`. (2026-11-01)
- **Tip:** Playwright's WebKit with the real `src/styles.css` reproduces WebKit layout bugs, but not the hover loss above: headless WebKit re-applies `:hover` after a node swap, so check hover in the app itself. (2026-10-30, 2026-11-01)
