# Auto-update — design

Date: 2026-10-13
Status: approved

## Purpose

Maya notices a new release and, on the desktop, updates itself after one
click, so the user stops re-running the installer on every machine. Where
Maya cannot replace itself (the phone, a `.deb` install) it says that a new
version is out and links to it.

Success: with 0.18.0 installed by hand, a later release makes a bar appear
on the Mac's and the laptop's boards within six hours (or at the next
start); pressing Update downloads, installs and relaunches Maya on the new
version, and the sessions on the board are still there. The phone shows
the same bar with Download, which opens the new APK.

## Findings

- Releases are GitHub releases on the public repo `dosaki/maya`, published
  by `release.yml` on every push to `main` whose version has no release
  yet. Each one already carries the macOS `.app.tar.gz` and `.dmg`, the
  Windows NSIS `-setup.exe` and `.msi`, the Linux AppImage and `.deb`, the
  APK and the CLI binaries.
- Tauri's updater plugin (`tauri-plugin-updater`) replaces the `.app` in
  place on macOS, runs the NSIS installer on Windows and replaces the
  AppImage file on Linux. It cannot update a `.deb`, an MSI-only install
  or an Android app.
- The plugin checks a minisign signature on every download against a
  public key built into the app. Tauri writes the `.sig` files when
  `bundle.createUpdaterArtifacts` is true and `TAURI_SIGNING_PRIVATE_KEY`
  is in the environment; with the flag on and no key, the build fails.
- macOS release builds are signed with the stable "Maya Development"
  identity, so a replaced `.app` keeps its microphone and speech grants.
  A file the updater downloads has no quarantine flag.
- Sessions run in Terminal, Windows Terminal or tmux, not inside Maya, so
  relaunching Maya does not end them.
- The AppImage runtime sets `$APPIMAGE` to the AppImage's path; a `.deb`
  install has no such variable.
- `maya_core` already depends on `ureq` (for OpenCode), so the core can
  fetch a JSON file without a new dependency.

## Behaviour

### Desktop: macOS, Windows, the AppImage

- Maya checks at start and then every 6 hours. A debug build
  (`pnpm tauri dev`) never checks.
- When the feed's version is newer than the running one, a slim bar shows
  at the top of the board: *Maya 0.19.0 is ready — **Update** · Later*.
- **Update** downloads the update, showing the percentage in the bar,
  installs it and relaunches Maya.
- **Later** hides the bar until Maya restarts or a newer version appears.
  It is kept in memory only.
- Settings, beside "Maya 0.18.0", has **Check for updates**. It shows "Up
  to date", the same Update action, or the error.
- A failed check (offline, GitHub down) is logged and shows nothing. A
  failed download or a bad signature leaves the running app as it is and
  shows the error in the bar, and in `maya.log`.

### The phone and a `.deb` install

The same checks and the same bar, but the action is **Download**: on the
phone it opens the release's APK (`Maya_<version>.apk`), on a `.deb`
install the release page. Nothing installs by itself.

### Assistants

Every desktop Maya checks on its own, main or assistant. The main pushes
nothing to its assistants.

### Out of scope

- The CLI (`maya-cli`).
- MSI installs: the update arrives as the NSIS installer, which would
  install a second copy beside an MSI one. `install.ps1` installs with
  NSIS; the README says that an MSI install updates by hand.
- 0.17.0 and earlier have no updater: the first version with it is
  installed by hand once.

## The feed

One file, `latest.json`, uploaded with every release's assets and read at

    https://github.com/dosaki/maya/releases/latest/download/latest.json

in Tauri's static-manifest format:

```json
{
  "version": "0.19.0",
  "notes": "https://github.com/dosaki/maya/releases/tag/v0.19.0",
  "pub_date": "2026-10-20T10:00:00Z",
  "platforms": {
    "darwin-aarch64":  { "signature": "<.sig contents>", "url": ".../v0.19.0/Maya_0.19.0_aarch64.app.tar.gz" },
    "darwin-x86_64":   { "signature": "…", "url": ".../Maya_0.19.0_x86_64.app.tar.gz" },
    "windows-x86_64":  { "signature": "…", "url": ".../Maya_0.19.0_x64-setup.exe" },
    "linux-x86_64":    { "signature": "…", "url": ".../Maya_0.19.0_amd64.AppImage" },
    "linux-aarch64":   { "signature": "…", "url": ".../Maya_0.19.0_arm64.AppImage" }
  }
}
```

The phone and a `.deb` install read only `version` from the same file, so
there is one source and no GitHub API rate limit.

## The signing key

A one-off step for the user, before this pull request merges:

    pnpm tauri signer generate -w ~/.tauri/maya.key

- The public key goes in `src-tauri/tauri.conf.json` under
  `plugins.updater.pubkey` (committed).
- The private key and its password become the repository secrets
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- Losing the private key means the installed apps can verify no new
  release: every machine would need one reinstall by hand with a new key.
  Keep it with the Apple `.p12`.

## Release pipeline

### `build.yml`

When `inputs.sign` is true, the macOS, Windows and Linux app jobs:

- put `TAURI_SIGNING_PRIVATE_KEY` and its password in the build's
  environment and add `--config '{"bundle":{"createUpdaterArtifacts":true}}'`
  to `pnpm tauri build`;
- copy each `.sig` beside its renamed asset, with the asset's name plus
  `.sig` (for example `Maya_0.19.0_x64-setup.exe.sig`).

On macOS the released `Maya_<version>_<arch>.app.tar.gz` becomes Tauri's
own `bundle/macos/Maya.app.tar.gz`, because the signature covers its exact
bytes; it still holds `Maya.app` at its top, which is what `install.sh`
unpacks. Pull request builds (`inputs.sign` false) are unchanged: no key,
no `.sig`.

### `release.yml`

Before `gh release create`, the release job runs
`sh scripts/updater-manifest.sh <version> dist > dist/latest.json`. The
script fails when any of the five platforms has no asset or no `.sig`, so
a release never goes out that breaks updates. The `.sig` files are
uploaded with the other assets; the installers ignore them.

## Code

### `core/src/update.rs` (new)

- `is_newer(current: &str, latest: &str) -> bool`: compares
  `major.minor.patch`, tolerating a leading `v`; anything unparsable is
  not newer. About ten lines, so no `semver` crate.
- `latest_version(url: &str) -> Result<String, String>`: fetches the feed
  with `ureq` and returns its `version`.
- `FEED_URL`, the feed address above.

### `src-tauri/src/updater.rs` (new)

Adds `tauri-plugin-updater` and `tauri-plugin-process`. All the work is in
Rust behind three commands registered in `lib.rs`, so the page needs no JS
plugin packages and no new capability permissions:

- `update_kind() -> "self" | "notice" | "off"`: `off` in a debug build;
  `notice` on Linux when `$APPIMAGE` is unset; `self` otherwise.
- `update_check() -> Option<{ version, notes }>`: through the updater
  plugin for `self`, through `maya_core::update` for `notice`.
- `update_install()`: downloads and installs the update the last check
  found, emits `update-progress` (`{ received, total }`), then calls
  `app.restart()`.

`tauri.conf.json` gets `plugins.updater` with the endpoint, the public key
and `windows.installMode: "passive"`.

### `mobile/src/commands.rs`

`update_kind` is always `notice`; `update_check` goes through
`maya_core::update`.

### `src/update.ts` (new, shared by both pages)

- The bar's state: idle, available (version), downloading (percent),
  error (message), dismissed (version).
- Paints the bar into `<div id="update-bar">`, added to `index.html` and
  `mobile/web/index.html`.
- Runs the check at start and every 6 hours, unless the kind is `off`.
- The action is **Update** (`update_install`) for `self` and **Download**
  for `notice`: the phone opens the APK URL through the opener plugin, the
  desktop opens the release page through the existing `open_url` command.

### `src/settings.ts`

**Check for updates** beside the version line, using the same check and
showing its outcome.

## Testing

- `core`: `is_newer` (equal, older, newer, `0.9` against `0.10`, a leading
  `v`, junk) and parsing a sample feed.
- vitest, `src/update.test.ts`: the state transitions, Update against
  Download by kind, Later hiding the bar until a newer version, the
  download's percentage, the error in the bar; and the Settings button's
  outcomes.
- `scripts/updater-manifest.sh`: a test run in CI against fake assets and
  `.sig` files, which checks the JSON and that a missing `.sig` fails it.
- `install.sh` unpacking Tauri's `Maya.app.tar.gz` (checked once by hand on
  a release build's artifact).
- By hand, after the first release with the updater is installed: merge a
  later version bump, then see the bar and the update apply on the Mac and
  on the laptop, and the Download notice on the phone.

## Version

A `feat`: 0.17.0 → 0.18.0, in its own `chore(release): 0.18.0` commit.
