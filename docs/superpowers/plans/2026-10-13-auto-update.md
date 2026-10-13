# Auto-update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The desktop app updates itself after one click from a bar on the board; the phone and a `.deb` install show the same bar with a Download link.

**Architecture:** Every release publishes a signed `latest.json` beside its assets. The desktop app reads it through `tauri-plugin-updater` (behind three Rust commands in a new `src-tauri/src/updater.rs`); the phone and a `.deb` install read only its `version` through a new `maya_core::update`. A new shared page module, `src/update.ts`, owns the bar's state, paints it, schedules the checks and backs a Check for updates button in Settings.

**Tech Stack:** Rust (Tauri 2, `tauri-plugin-updater` 2, `ureq` 3, `serde_json`), vanilla TypeScript (vitest, jsdom), POSIX `sh` + `jq`, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-13-auto-update-design.md`

## Global Constraints

- Feed URL: `https://github.com/dosaki/maya/releases/latest/download/latest.json`.
- Feed platforms, exactly: `darwin-aarch64`, `darwin-x86_64`, `windows-x86_64`, `linux-x86_64`, `linux-aarch64`.
- Feed assets: `Maya_<v>_aarch64.app.tar.gz`, `Maya_<v>_x86_64.app.tar.gz`, `Maya_<v>_x64-setup.exe`, `Maya_<v>_amd64.AppImage`, `Maya_<v>_arm64.AppImage`, each with `<asset>.sig`.
- Secrets: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- Checks: at start, then every 6 hours. A debug build never checks (`update_kind` is `off`).
- Bar copy: `Maya <v> is ready.` + **Update** · **Later** (self); `Maya <v> is out.` + **Download** · **Later** (notice).
- Later is in memory only; it hides the bar for that version until restart.
- Windows `installMode`: `passive`.
- Pull request builds (`inputs.sign` false) must not change: no key, no `.sig`, no `createUpdaterArtifacts`.
- The release fails when any platform's asset or `.sig` is missing.
- Version bump: 0.17.0 → 0.18.0, its own commit `chore(release): 0.18.0`, via `sh scripts/set-version.sh 0.18.0`; check `Cargo.lock` changed.
- Never push or commit to `main`; work on `feat/auto-update`. Ask before each push.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

Deviation from the spec, by design: the spec names `tauri-plugin-process` for the relaunch. Tauri 2's own `AppHandle::restart()` does it from Rust, so that plugin is not added.

## Review Focus

1. The feed is unreachable (offline at start, GitHub down): no bar, no toast, the failure logged once per check — Task 2 (`notice` logs) and Task 5 (a rejected periodic check paints nothing).
2. Update pressed twice, or Later pressed while downloading: the second press does nothing — Task 5 (the downloading bar has no buttons; `act` ignores the downloading phase).
3. Later on 0.19.0, then 0.20.0 appears: the bar comes back — Task 5.
4. The download reports no content length: the bar says "Downloading…" with no percent, never `NaN%` or `Infinity%` — Task 5.
5. An install fails, then Try again: the backend dropped the update it took, so the retry checks again before installing — Task 5 (Try again calls `update_check` then `update_install`).

---

### Task 1: The updater signing key (the user)

This task is the user's. It must be done before Task 3 (which needs the public key) and before the pull request merges (the release needs the secrets). The implementer asks the user to do it and waits.

- [ ] **Step 1: Ask the user to generate the key**

Ask the user to run, on their Mac:

```bash
pnpm tauri signer generate -w ~/.tauri/maya.key
```

It asks for a password and writes `~/.tauri/maya.key` (private) and `~/.tauri/maya.key.pub` (public).

- [ ] **Step 2: Ask the user to add the two secrets**

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/maya.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

(The second prompts for the password.) Remind them to keep `maya.key` and its password with the Apple `.p12`: losing them means every machine reinstalls by hand once.

- [ ] **Step 3: Get the public key**

Ask the user for the contents of `~/.tauri/maya.key.pub` (one base64 line; safe to share and to commit). Task 3 puts it in `src-tauri/tauri.conf.json`. Confirm with `gh secret list` that both secrets are listed.

---

### Task 2: `maya_core::update`

**Files:**
- Create: `core/src/update.rs`
- Modify: `core/src/lib.rs` (`pub mod update;`, Step 2)

**Interfaces:**
- Produces:
  - `pub const FEED_URL: &str`
  - `pub fn is_newer(current: &str, latest: &str) -> bool`
  - `pub fn feed_version(json: &str) -> Result<String, String>`
  - `pub fn release_page(version: &str) -> String` → `https://github.com/dosaki/maya/releases/tag/v<version>`
  - `pub fn apk_url(version: &str) -> String` → `https://github.com/dosaki/maya/releases/download/v<version>/Maya_<version>.apk`
  - `pub struct UpdateInfo { pub version: String, pub download: String }` (`Serialize`, `Clone`, `Debug`, `PartialEq`)
  - `pub fn notice(current: &str, download: fn(&str) -> String) -> Result<Option<UpdateInfo>, String>`: fetches the feed; `Some` only when its version is newer. Logs a failure under the `update` source and returns the error.

- [ ] **Step 1: Write the failing tests**

Create `core/src/update.rs` with only the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_version_is_newer_and_an_equal_or_lower_one_is_not() {
        assert!(is_newer("0.17.0", "0.18.0"));
        assert!(is_newer("0.17.0", "1.0.0"));
        assert!(is_newer("0.17.0", "0.17.1"));
        assert!(!is_newer("0.17.0", "0.17.0"));
        assert!(!is_newer("0.18.0", "0.17.9"));
    }

    #[test]
    fn parts_compare_as_numbers_not_text() {
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(!is_newer("0.10.0", "0.9.0"));
    }

    #[test]
    fn a_leading_v_and_surrounding_space_are_tolerated() {
        assert!(is_newer("v0.17.0", " v0.18.0\n"));
    }

    #[test]
    fn anything_unparsable_is_never_newer() {
        for junk in ["", "latest", "0.18", "0.18.0.1", "0.18.x", "0.18.0-beta"] {
            assert!(!is_newer("0.17.0", junk), "{junk:?}");
            assert!(!is_newer(junk, "0.18.0"), "{junk:?}");
        }
    }

    #[test]
    fn the_feed_version_is_read_from_a_tauri_manifest() {
        let json = r#"{"version":"0.19.0","notes":"https://github.com/dosaki/maya/releases/tag/v0.19.0","platforms":{}}"#;
        assert_eq!(feed_version(json), Ok("0.19.0".to_string()));
    }

    #[test]
    fn a_feed_without_a_version_or_not_json_is_an_error() {
        assert!(feed_version(r#"{"platforms":{}}"#).unwrap_err().contains("no version"));
        assert!(feed_version("<html>").unwrap_err().contains("not JSON"));
    }

    #[test]
    fn the_links_point_at_the_release() {
        assert_eq!(release_page("0.19.0"), "https://github.com/dosaki/maya/releases/tag/v0.19.0");
        assert_eq!(apk_url("0.19.0"), "https://github.com/dosaki/maya/releases/download/v0.19.0/Maya_0.19.0.apk");
    }
}
```

- [ ] **Step 2: Add the module to the core**

In `core/src/lib.rs`, add `pub mod update;` after `pub mod tty;` (the list is alphabetical except for the platform-gated tail):

```rust
pub mod tty;
pub mod update;
pub mod watcher;
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p maya-core update::`
Expected: compile errors, `cannot find function is_newer` (and the others).

- [ ] **Step 4: Write the implementation**

Put this above the tests module in `core/src/update.rs`:

```rust
//! Whether a newer Maya is out: the release feed's version against this
//! one. The desktop app installs through Tauri's updater, which reads the
//! same feed; this is for the installs it cannot update (the phone, a
//! `.deb`), which only say that a new version is out.

use serde::Serialize;
use std::time::Duration;

use crate::log;

/// The feed every release publishes: Tauri's static updater manifest.
pub const FEED_URL: &str = "https://github.com/dosaki/maya/releases/latest/download/latest.json";

const RELEASES: &str = "https://github.com/dosaki/maya/releases";
const TIMEOUT: Duration = Duration::from_secs(20);

/// A newer release, and the link a page that cannot install it opens.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub download: String,
}

fn parts(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let parsed = (it.next()??, it.next()??, it.next()??);
    if it.next().is_some() {
        return None;
    }
    Some(parsed)
}

/// True when `latest` is a higher `major.minor.patch` than `current`;
/// anything either side that is not one is never newer.
pub fn is_newer(current: &str, latest: &str) -> bool {
    matches!((parts(current), parts(latest)), (Some(c), Some(l)) if l > c)
}

/// The `version` of a feed.
pub fn feed_version(json: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("the update feed is not JSON: {e}"))?;
    v["version"].as_str().map(str::to_string).ok_or_else(|| "the update feed has no version".to_string())
}

/// The release's page.
pub fn release_page(version: &str) -> String {
    format!("{RELEASES}/tag/v{version}")
}

/// The release's APK, by the name `release.yml` gives it.
pub fn apk_url(version: &str) -> String {
    format!("{RELEASES}/download/v{version}/Maya_{version}.apk")
}

fn fetch_feed_version() -> Result<String, String> {
    let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(true).build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent.get(FEED_URL).call().map_err(|e| format!("could not check for updates: {e}"))?;
    let body = resp.body_mut().read_to_string().map_err(|e| format!("could not read the update feed: {e}"))?;
    feed_version(&body)
}

/// The release the feed names when it is newer than `current`, with
/// `download(version)` as its link. A failure is logged and returned.
pub fn notice(current: &str, download: fn(&str) -> String) -> Result<Option<UpdateInfo>, String> {
    match fetch_feed_version() {
        Ok(v) if is_newer(current, &v) => Ok(Some(UpdateInfo { download: download(&v), version: v })),
        Ok(_) => Ok(None),
        Err(e) => {
            log::line("update", e.clone());
            Err(e)
        }
    }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p maya-core update::`
Expected: 7 passed.

- [ ] **Step 6: Commit**

```bash
git add core/src/update.rs core/src/lib.rs
git commit -m "feat: the core reads the release feed's version

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The desktop updater

**Files:**
- Create: `src-tauri/src/updater.rs`
- Modify: `src-tauri/Cargo.toml` (dependency), `src-tauri/src/lib.rs` (module, plugin, state, commands), `src-tauri/tauri.conf.json` (plugin config)

**Interfaces:**
- Consumes: `maya_core::update::{notice, release_page, UpdateInfo}` (Task 2).
- Produces (Tauri commands, called by `src/update.ts` in Task 5):
  - `update_kind` → `"self" | "notice" | "off"` (a string)
  - `update_check` → `UpdateInfo | null`, serialized `{ "version": "0.19.0", "download": "https://…/tag/v0.19.0" }`; rejects with a message string on failure
  - `update_install` → never resolves on success (the app restarts); rejects with a message string
  - event `update-progress` with payload `{ "received": number, "total": number }` (`total` is 0 when unknown)
- `pub fn kind_for(debug: bool, linux: bool, appimage: Option<&std::ffi::OsStr>) -> &'static str`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/updater.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::kind_for;
    use std::ffi::OsStr;

    #[test]
    fn a_debug_build_never_updates() {
        assert_eq!(kind_for(true, false, None), "off");
        assert_eq!(kind_for(true, true, Some(OsStr::new("/home/u/.local/bin/maya-app"))), "off");
    }

    #[test]
    fn linux_updates_itself_only_from_an_appimage() {
        assert_eq!(kind_for(false, true, Some(OsStr::new("/home/u/.local/bin/maya-app"))), "self");
        assert_eq!(kind_for(false, true, None), "notice");
        assert_eq!(kind_for(false, true, Some(OsStr::new(""))), "notice");
    }

    #[test]
    fn macos_and_windows_update_themselves() {
        assert_eq!(kind_for(false, false, None), "self");
    }
}
```

and add `pub mod updater;` to `src-tauri/src/lib.rs` after `pub mod terminal_win;`'s line and before `pub mod voice;`:

```rust
#[cfg(windows)]
pub mod terminal_win;
pub mod updater;
pub mod voice;
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p maya updater::`
Expected: compile error, `unresolved import super::kind_for`.

- [ ] **Step 3: Add the dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]`, after `tauri = …`:

```toml
tauri-plugin-updater = "2"
```

- [ ] **Step 4: Write the module**

Put this above the tests module in `src-tauri/src/updater.rs`:

```rust
//! Updating Maya from the release feed: Tauri's updater where it can
//! replace the app (macOS, Windows, the AppImage), the core's notice where
//! it cannot (a `.deb`). The page asks `update_kind` once, then
//! `update_check` at start and every 6 hours.

use std::ffi::OsStr;
use std::sync::Mutex;

use maya_core::log;
use maya_core::update::{self, UpdateInfo};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The update the last check found, for `update_install` to take.
#[derive(Default)]
pub struct Pending(pub Mutex<Option<Update>>);

#[derive(Clone, Serialize)]
struct Progress {
    received: u64,
    total: u64,
}

/// How this install updates: `off` in a debug build, `notice` on Linux
/// outside an AppImage (the AppImage runtime sets `$APPIMAGE`), `self`
/// otherwise.
pub fn kind_for(debug: bool, linux: bool, appimage: Option<&OsStr>) -> &'static str {
    if debug {
        "off"
    } else if linux && appimage.is_none_or(|p| p.is_empty()) {
        "notice"
    } else {
        "self"
    }
}

fn kind() -> &'static str {
    kind_for(cfg!(debug_assertions), cfg!(target_os = "linux"), std::env::var_os("APPIMAGE").as_deref())
}

#[tauri::command]
pub fn update_kind() -> String {
    kind().to_string()
}

/// The newer release, if any. For `self` installs it is also kept for
/// `update_install`.
#[tauri::command]
pub async fn update_check(app: AppHandle, pending: State<'_, Pending>) -> Result<Option<UpdateInfo>, String> {
    match kind() {
        "off" => Ok(None),
        "notice" => tauri::async_runtime::spawn_blocking(|| update::notice(env!("CARGO_PKG_VERSION"), update::release_page))
            .await
            .map_err(|e| e.to_string())?,
        _ => {
            let checked = match app.updater() {
                Ok(u) => u.check().await.map_err(|e| format!("could not check for updates: {e}")),
                Err(e) => Err(format!("the updater is not set up: {e}")),
            };
            let found = checked.inspect_err(|e| log::line("update", e.clone()))?;
            let info = found.as_ref().map(|u| UpdateInfo { version: u.version.clone(), download: update::release_page(&u.version) });
            *pending.0.lock().unwrap() = found;
            Ok(info)
        }
    }
}

/// Downloads and installs the update the last check found, reporting
/// `update-progress`, then relaunches Maya. Sessions live in their
/// terminals, so the relaunch ends none of them.
#[tauri::command]
pub async fn update_install(app: AppHandle, pending: State<'_, Pending>) -> Result<(), String> {
    let Some(found) = pending.0.lock().unwrap().take() else {
        return Err("No update is waiting; check again.".into());
    };
    log::line("update", format!("installing Maya {}", found.version));
    let emitter = app.clone();
    let mut received = 0u64;
    found
        .download_and_install(
            move |chunk, total| {
                received += chunk as u64;
                let _ = emitter.emit("update-progress", Progress { received, total: total.unwrap_or(0) });
            },
            || {},
        )
        .await
        .map_err(|e| {
            let m = format!("the update failed: {e}");
            log::line("update", m.clone());
            m
        })?;
    log::line("update", "installed; relaunching");
    app.restart();
}
```

- [ ] **Step 5: Run the test to see it pass**

Run: `cargo test -p maya updater::`
Expected: 3 passed.

- [ ] **Step 6: Register the plugin, the state and the commands**

In `src-tauri/src/lib.rs`'s `run()`, change the builder's head to:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState { store: Mutex::new(store), notifier: Mutex::new(notify::Notifier::default()), reviews: Mutex::new(ReviewState::default()), voice: Mutex::new(VoiceState::default()), network: Mutex::new(net::NetworkState::default()) })
        .manage(updater::Pending::default())
```

and append to the end of `generate_handler![…]`, after `list_machines`:

```rust
            list_machines,
            updater::update_kind,
            updater::update_check,
            updater::update_install
```

- [ ] **Step 7: Configure the plugin**

In `src-tauri/tauri.conf.json`, add a top-level `plugins` key after `bundle` (replace `<PUBLIC KEY>` with the one line from Task 1, Step 3):

```json
  "plugins": {
    "updater": {
      "pubkey": "<PUBLIC KEY>",
      "endpoints": ["https://github.com/dosaki/maya/releases/latest/download/latest.json"],
      "windows": { "installMode": "passive" }
    }
  }
```

- [ ] **Step 8: Build and run the app's tests**

Run: `cargo test -p maya`
Expected: all pass. Then `pnpm tauri build --bundles app` must still succeed without `TAURI_SIGNING_PRIVATE_KEY` set (no `createUpdaterArtifacts` in the committed config).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/Cargo.toml Cargo.lock src-tauri/src/updater.rs src-tauri/src/lib.rs src-tauri/tauri.conf.json
git commit -m "feat: the desktop app checks for and installs updates

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The phone's commands

**Files:**
- Modify: `mobile/src/commands.rs`, `mobile/src/lib.rs` (`generate_handler!`)

**Interfaces:**
- Consumes: `maya_core::update::{notice, apk_url, UpdateInfo}` (Task 2).
- Produces: `update_kind` (`"notice"`, or `"off"` in a debug build) and `update_check` (`UpdateInfo | null`), the same names and shapes as Task 3, for `src/update.ts`.

- [ ] **Step 1: Write the failing test**

At the end of `mobile/src/commands.rs`'s existing `#[cfg(test)] mod tests` (or a new one if there is none), add:

```rust
    #[test]
    fn the_phone_only_ever_shows_a_notice() {
        assert_eq!(super::update_kind(), if cfg!(debug_assertions) { "off" } else { "notice" });
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya-mobile the_phone_only_ever_shows_a_notice`
Expected: compile error, `cannot find function update_kind`.

- [ ] **Step 3: Add the commands**

In `mobile/src/commands.rs`, after `open_url`:

```rust
/// The phone cannot install an APK by itself, so it only ever says that a
/// new version is out. A debug build never checks.
#[tauri::command]
pub fn update_kind() -> String {
    if cfg!(debug_assertions) { "off" } else { "notice" }.to_string()
}

/// The newer release, if any, linking to its APK.
#[tauri::command(async)]
pub fn update_check() -> Result<Option<maya_core::update::UpdateInfo>, String> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    maya_core::update::notice(env!("CARGO_PKG_VERSION"), maya_core::update::apk_url)
}
```

and in `mobile/src/lib.rs`, append `commands::update_kind, commands::update_check` to `generate_handler![…]`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-mobile`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add mobile/src/commands.rs mobile/src/lib.rs
git commit -m "feat: the phone checks for a newer release

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The update bar (`src/update.ts`)

**Files:**
- Create: `src/update.ts`, `src/update.test.ts`
- Modify: `index.html`, `mobile/web/index.html` (the host), `src/styles.css` (the bar), `src/main.ts`, `src/mobile/main.ts` (start it)

**Interfaces:**
- Consumes: the commands and event from Tasks 3 and 4; `open_url` (desktop `src-tauri/src/lib.rs:779`, phone `mobile/src/commands.rs:103`), which both pages already have.
- Produces, for Task 6:
  - `export type UpdateKind = "self" | "notice" | "off"`
  - `export interface UpdateInfo { version: string; download: string }`
  - `export type UpdateState = { phase: "idle" } | { phase: "available"; info: UpdateInfo } | { phase: "downloading"; info: UpdateInfo; percent: number | null } | { phase: "error"; info: UpdateInfo; message: string }`
  - `export function renderUpdateBar(state: UpdateState, kind: UpdateKind, dismissed: string | null): HTMLElement | null`
  - `export function makeUpdates(): { init(): Promise<void>; ready(): Promise<void>; check(manual?: boolean): Promise<UpdateInfo | null>; kind(): UpdateKind }` — `ready()` settles once `init()` has read the kind (immediately before `init()` is called)
  - `export const updates` (the page's one instance) and `export const CHECK_EVERY_MS = 21_600_000`

- [ ] **Step 1: Write the failing tests**

Create `src/update.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type Handler = (e: { payload: unknown }) => void;
const handlers: Record<string, Handler> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, h: Handler) => {
    handlers[name] = h;
    return Promise.resolve(() => {});
  },
}));

import { CHECK_EVERY_MS, makeUpdates, renderUpdateBar, type UpdateInfo } from "./update";

const info: UpdateInfo = { version: "0.19.0", download: "https://github.com/dosaki/maya/releases/tag/v0.19.0" };
const flush = () => new Promise((r) => setTimeout(r, 0));
const bar = () => document.querySelector("#update-bar .update-bar");
const text = () => document.querySelector("#update-bar .update-bar__text")?.textContent;
const labels = () => [...document.querySelectorAll<HTMLButtonElement>("#update-bar button")].map((b) => b.textContent);
const press = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("#update-bar button")].find((b) => b.textContent === label)!.click();

function backend(kind: string, check: () => Promise<UpdateInfo | null>, install = () => new Promise<void>(() => {})) {
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "update_kind") return Promise.resolve(kind);
    if (cmd === "update_check") return check();
    if (cmd === "update_install") return install();
    if (cmd === "open_url") return Promise.resolve();
    return Promise.reject(new Error("unexpected " + cmd));
  });
}

describe("renderUpdateBar", () => {
  it("shows nothing when idle, off, or dismissed for this version", () => {
    expect(renderUpdateBar({ phase: "idle" }, "self", null)).toBeNull();
    expect(renderUpdateBar({ phase: "available", info }, "off", null)).toBeNull();
    expect(renderUpdateBar({ phase: "available", info }, "self", "0.19.0")).toBeNull();
  });

  it("offers Update where Maya installs itself and Download where it cannot", () => {
    const self = renderUpdateBar({ phase: "available", info }, "self", null)!;
    expect(self.querySelector(".update-bar__text")?.textContent).toBe("Maya 0.19.0 is ready.");
    expect([...self.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Update", "Later"]);
    const notice = renderUpdateBar({ phase: "available", info }, "notice", null)!;
    expect(notice.querySelector(".update-bar__text")?.textContent).toBe("Maya 0.19.0 is out.");
    expect([...notice.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Download", "Later"]);
  });

  it("shows the percentage while downloading, none when the size is unknown, and no buttons", () => {
    const some = renderUpdateBar({ phase: "downloading", info, percent: 42 }, "self", null)!;
    expect(some.textContent).toBe("Downloading Maya 0.19.0… 42%");
    expect(some.querySelectorAll("button").length).toBe(0);
    expect(renderUpdateBar({ phase: "downloading", info, percent: null }, "self", null)!.textContent).toBe("Downloading Maya 0.19.0…");
  });

  it("shows a failed install with Try again", () => {
    const el = renderUpdateBar({ phase: "error", info, message: "the update failed: signature" }, "self", null)!;
    expect(el.querySelector(".update-bar__text")?.textContent).toBe("Maya 0.19.0 could not be installed: the update failed: signature");
    expect([...el.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Try again", "Later"]);
  });
});

describe("makeUpdates", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="update-bar" hidden></div>';
    invoke.mockReset();
    vi.useRealTimers();
  });

  it("never checks when the kind is off", async () => {
    backend("off", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    expect(invoke).not.toHaveBeenCalledWith("update_check");
    expect(bar()).toBeNull();
  });

  it("checks at start and shows the bar for a newer release", async () => {
    backend("self", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    expect(text()).toBe("Maya 0.19.0 is ready.");
    expect(document.getElementById("update-bar")!.hidden).toBe(false);
  });

  it("checks again every six hours", async () => {
    vi.useFakeTimers();
    backend("self", () => Promise.resolve(null));
    await makeUpdates().init();
    await vi.advanceTimersByTimeAsync(0);
    const calls = () => invoke.mock.calls.filter((c) => c[0] === "update_check").length;
    expect(calls()).toBe(1);
    await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS);
    expect(calls()).toBe(2);
  });

  it("a failed background check shows nothing", async () => {
    backend("self", () => Promise.reject("could not check for updates: offline"));
    await makeUpdates().init();
    await flush();
    expect(bar()).toBeNull();
  });

  it("Later hides the bar until a newer version appears", async () => {
    let latest: UpdateInfo = info;
    backend("self", () => Promise.resolve(latest));
    const u = makeUpdates();
    await u.init();
    await flush();
    press("Later");
    expect(bar()).toBeNull();
    await u.check();
    expect(bar()).toBeNull();
    latest = { version: "0.20.0", download: "https://github.com/dosaki/maya/releases/tag/v0.20.0" };
    await u.check();
    expect(text()).toBe("Maya 0.20.0 is ready.");
  });

  it("a check from Settings brings a dismissed bar back", async () => {
    backend("self", () => Promise.resolve(info));
    const u = makeUpdates();
    await u.init();
    await flush();
    press("Later");
    await u.check(true);
    expect(text()).toBe("Maya 0.19.0 is ready.");
  });

  it("Update installs, shows progress, and ignores further presses", async () => {
    backend("self", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(invoke).toHaveBeenCalledWith("update_install");
    expect(text()).toBe("Downloading Maya 0.19.0…");
    expect(labels()).toEqual([]);
    handlers["update-progress"]({ payload: { received: 50, total: 200 } });
    expect(text()).toBe("Downloading Maya 0.19.0… 25%");
    handlers["update-progress"]({ payload: { received: 50, total: 0 } });
    expect(text()).toBe("Downloading Maya 0.19.0…");
  });

  it("a background check during the download leaves the bar downloading", async () => {
    backend("self", () => Promise.resolve(info));
    const u = makeUpdates();
    await u.init();
    await flush();
    press("Update");
    await flush();
    await u.check();
    expect(text()).toBe("Downloading Maya 0.19.0…");
  });

  it("a failed install shows the error, and Try again checks before installing", async () => {
    let fail = true;
    backend("self", () => Promise.resolve(info), () => (fail ? Promise.reject("the update failed: signature") : new Promise<void>(() => {})));
    await makeUpdates().init();
    await flush();
    press("Update");
    await flush();
    expect(text()).toBe("Maya 0.19.0 could not be installed: the update failed: signature");
    fail = false;
    invoke.mockClear();
    press("Try again");
    await flush();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(["update_check", "update_install"]);
  });

  it("Download opens the release's link and installs nothing", async () => {
    backend("notice", () => Promise.resolve(info));
    await makeUpdates().init();
    await flush();
    press("Download");
    await flush();
    expect(invoke).toHaveBeenCalledWith("open_url", { url: info.download });
    expect(invoke).not.toHaveBeenCalledWith("update_install");
  });
});
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm vitest run src/update.test.ts`
Expected: FAIL, `Failed to resolve import "./update"`.

- [ ] **Step 3: Write the module**

Create `src/update.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** How this install updates: by itself, by a link to the release, or never (a debug build). */
export type UpdateKind = "self" | "notice" | "off";

/** A newer release, and the link a page that cannot install it opens. */
export interface UpdateInfo {
  version: string;
  download: string;
}

export type UpdateState =
  | { phase: "idle" }
  | { phase: "available"; info: UpdateInfo }
  | { phase: "downloading"; info: UpdateInfo; percent: number | null }
  | { phase: "error"; info: UpdateInfo; message: string };

/** Six hours between background checks. */
export const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

function button(label: string, action: string, primary: boolean): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = primary ? "update-bar__action update-bar__action--primary" : "update-bar__action";
  b.dataset.update = action;
  b.textContent = label;
  return b;
}

/** The bar for `state`, or null when nothing should show. */
export function renderUpdateBar(state: UpdateState, kind: UpdateKind, dismissed: string | null): HTMLElement | null {
  if (state.phase === "idle" || kind === "off") return null;
  if (state.phase === "available" && dismissed === state.info.version) return null;
  const bar = document.createElement("div");
  bar.className = "update-bar";
  bar.setAttribute("role", "status");
  const text = document.createElement("span");
  text.className = "update-bar__text";
  const v = state.info.version;
  if (state.phase === "available") text.textContent = kind === "self" ? `Maya ${v} is ready.` : `Maya ${v} is out.`;
  else if (state.phase === "downloading") text.textContent = state.percent === null ? `Downloading Maya ${v}…` : `Downloading Maya ${v}… ${state.percent}%`;
  else text.textContent = `Maya ${v} could not be installed: ${state.message}`;
  bar.append(text);
  if (state.phase === "downloading") return bar;
  const main = kind === "notice" ? "Download" : state.phase === "error" ? "Try again" : "Update";
  bar.append(button(main, "go", true), button("Later", "later", false));
  return bar;
}

/** The page's updates: the bar's state, its checks and its buttons. */
export function makeUpdates() {
  let kind: UpdateKind = "off";
  let state: UpdateState = { phase: "idle" };
  let dismissed: string | null = null;

  function paint(): void {
    const host = document.getElementById("update-bar");
    if (!host) return;
    const bar = renderUpdateBar(state, kind, dismissed);
    host.replaceChildren(...(bar ? [bar] : []));
    host.hidden = !bar;
  }

  /** Asks for a newer release and repaints. `manual` (Settings) brings a dismissed bar back. Rejects when the check fails. */
  async function check(manual = false): Promise<UpdateInfo | null> {
    if (kind === "off") return null;
    if (manual) dismissed = null;
    const info = await invoke<UpdateInfo | null>("update_check");
    if (state.phase !== "downloading") state = info ? { phase: "available", info } : { phase: "idle" };
    paint();
    return info;
  }

  async function act(action: string): Promise<void> {
    if (state.phase === "idle" || state.phase === "downloading") return;
    const info = state.info;
    if (action === "later") {
      dismissed = info.version;
      state = { phase: "available", info };
      paint();
      return;
    }
    if (kind === "notice") {
      try {
        await invoke("open_url", { url: info.download });
      } catch (e) {
        state = { phase: "error", info, message: String(e) };
        paint();
      }
      return;
    }
    // A failed install took the update it found, so a retry finds it again.
    const retry = state.phase === "error";
    state = { phase: "downloading", info, percent: null };
    paint();
    try {
      if (retry) await invoke("update_check");
      await invoke("update_install");
    } catch (e) {
      state = { phase: "error", info, message: String(e) };
      paint();
    }
  }

  async function start(): Promise<void> {
    try {
      kind = await invoke<UpdateKind>("update_kind");
    } catch {
      kind = "off";
    }
    if (kind === "off") return;
    document.getElementById("update-bar")?.addEventListener("click", (ev) => {
      const action = (ev.target as Element).closest<HTMLElement>("[data-update]")?.dataset.update;
      if (action) void act(action);
    });
    await listen<{ received: number; total: number }>("update-progress", (e) => {
      if (state.phase !== "downloading") return;
      const { received, total } = e.payload;
      state = { ...state, percent: total > 0 ? Math.min(100, Math.floor((received * 100) / total)) : null };
      paint();
    });
    // The backend logs a failed check; in the background it shows nothing.
    const quiet = () => void check().catch(() => undefined);
    quiet();
    setInterval(quiet, CHECK_EVERY_MS);
  }

  // Settings reads `kind()` once this settles, since `main.ts` starts
  // both without waiting for either.
  let ready: Promise<void> = Promise.resolve();
  const init = (): Promise<void> => (ready = start());

  return { init, ready: () => ready, check, kind: () => kind };
}

/** The page's one instance, shared with Settings. */
export const updates = makeUpdates();
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `pnpm vitest run src/update.test.ts`
Expected: all pass.

- [ ] **Step 5: Add the host to both pages**

In `index.html`, between `</header>` and `<div id="board"></div>`:

```html
      </header>
      <div id="update-bar" hidden></div>
      <div id="board"></div>
```

In `mobile/web/index.html`, between `</header>` and `<div id="strip-host"></div>`:

```html
      </header>
      <div id="update-bar" hidden></div>
      <div id="strip-host"></div>
```

- [ ] **Step 6: Style the bar**

Append to `src/styles.css`, next to the `.toast` rule (line 88), in the file's one-line-rule style:

```css
.update-bar { display: flex; align-items: center; gap: 10px; padding: 6px 14px; background: var(--panel); border-bottom: 1px solid var(--border); color: var(--text); font-size: 13px; }
.update-bar__text { flex: 1; min-width: 0; }
.update-bar__action { background: none; border: 1px solid var(--border); color: var(--text); border-radius: 6px; padding: 3px 10px; font-size: 12px; cursor: pointer; }
.update-bar__action--primary { background: var(--gold); border-color: var(--gold); color: var(--on-gold); }
.update-bar__action--primary:hover { background: var(--gold-bright); }
```

- [ ] **Step 7: Start it on both pages**

In `src/main.ts`, add the import with the others and the call at the top of `start()`:

```ts
import { updates } from "./update";
```

```ts
async function start(): Promise<void> {
  void initSettings();
  void updates.init();
```

In `src/mobile/main.ts`, add `import { updates } from "../update";` with the imports, and `void updates.init();` as the first line of its `start()`.

- [ ] **Step 8: Run the whole page suite**

Run: `pnpm test`
Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add src/update.ts src/update.test.ts index.html mobile/web/index.html src/styles.css src/main.ts src/mobile/main.ts
git commit -m "feat: a bar on the board offers a newer Maya

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Check for updates in Settings

**Files:**
- Modify: `src/settings.ts` (model, handler, render, wiring), `src/settings.test.ts`, `src/settings-flow.test.ts`

**Interfaces:**
- Consumes: `updates.check(manual: boolean): Promise<UpdateInfo | null>`, `updates.ready(): Promise<void>` and `updates.kind(): UpdateKind` from `src/update.ts` (Task 5).
- Produces: `SettingsModel.updatesOn?: boolean`, `SettingsModel.updateStatus?: string`, `SettingsHandlers.onCheckUpdates?(): void`.

- [ ] **Step 1: Write the failing render tests**

In `src/settings.test.ts`, add a `describe` block:

```ts
describe("check for updates", () => {
  it("offers the button beside the version only where Maya can update", () => {
    const h = { ...handlers(), onCheckUpdates: vi.fn() };
    const off = renderSettings({ ...voiceBase, version: "0.18.0" }, h);
    expect(off.querySelector("[data-action=check-updates]")).toBeNull();
    const on = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: true }, h);
    on.querySelector<HTMLButtonElement>("[data-action=check-updates]")!.click();
    expect(h.onCheckUpdates).toHaveBeenCalled();
  });

  it("shows the outcome of the last check", () => {
    const el = renderSettings({ ...voiceBase, version: "0.18.0", updatesOn: true, updateStatus: "Up to date." }, handlers());
    expect(el.querySelector(".settings__update-status")?.textContent).toBe("Up to date.");
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm vitest run src/settings.test.ts -t "check for updates"`
Expected: FAIL (no `[data-action=check-updates]`; TypeScript may also flag `updatesOn` as unknown).

- [ ] **Step 3: Add the model fields and the handler**

In `src/settings.ts`'s `SettingsModel`, after `version?: string;`:

```ts
  /** This install can check for updates (`update_kind` is not `off`). */
  updatesOn?: boolean;
  /** The outcome of the last Check for updates; unset before one. */
  updateStatus?: string;
```

In `SettingsHandlers`, after `onFullDiskAccess?(): void;`:

```ts
  onCheckUpdates?(): void;
```

- [ ] **Step 4: Render the button**

Replace the version block in `renderSettings` (currently `if (model.version) { … maya.append(version); }`) with:

```ts
  if (model.version) {
    const version = document.createElement("div");
    version.className = "settings__hint settings__version";
    version.textContent = `Maya ${model.version}`;
    if (model.updatesOn && h.onCheckUpdates) {
      const check = document.createElement("button");
      check.type = "button";
      check.dataset.action = "check-updates";
      check.textContent = "Check for updates";
      check.addEventListener("click", () => h.onCheckUpdates?.());
      version.append(" ", check);
      if (model.updateStatus) {
        const status = document.createElement("span");
        status.className = "settings__update-status";
        status.textContent = model.updateStatus;
        version.append(" ", status);
      }
    }
    maya.append(version);
  }
```

Note the version line's text is no longer only `Maya 0.18.0` when the button shows; the existing flow test (`settings-flow.test.ts`, "shows the app version…") mocks no `update_kind`, so `updatesOn` stays false there and its `toBe("Maya 0.13.0")` still holds.

- [ ] **Step 5: Run the render tests to see them pass**

Run: `pnpm vitest run src/settings.test.ts`
Expected: all pass.

- [ ] **Step 6: Write the failing flow test**

In `src/settings-flow.test.ts`, add `update_kind` and `update_check` answers to a copy of the first test's mock and assert the outcome texts:

```ts
  it("Check for updates reports up to date, a newer version, or the error", async () => {
    const config = { completedTimeoutMinutes: 30, projectsDir: null, clonesDir: null, notifyOnAwaiting: true, speakNotifications: true, voiceProvider: "builtin", elevenlabsVoiceId: null, listen: false, microphone: null, agent: "claude-code", agentModel: "", reviewPrompt: "" };
    let check: () => Promise<unknown> = () => Promise.resolve(null);
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "codex_hook_status") return Promise.resolve(false);
      if (cmd === "hook_status") return Promise.resolve(true);
      if (cmd === "get_config") return Promise.resolve({ ...config });
      if (cmd === "voice_selftest") return Promise.resolve(JSON.stringify({ devices: [] }));
      if (cmd === "list_whisper_models") return Promise.resolve([]);
      if (cmd === "list_agents") return Promise.resolve({ agents: [], names: true });
      if (cmd === "update_kind") return Promise.resolve("self");
      if (cmd === "update_check") return check();
      return Promise.reject(new Error("unexpected " + cmd));
    });
    document.body.innerHTML = '<div id="update-bar" hidden></div><div id="settings" class="settings pane"></div>';
    const { updates } = await import("./update");
    await updates.init();
    await initSettings();
    await flush();
    const press = () => document.querySelector<HTMLButtonElement>("[data-action=check-updates]")!.click();
    const status = () => document.querySelector(".settings__update-status")?.textContent;

    press();
    await flush();
    expect(status()).toBe("Up to date.");

    check = () => Promise.resolve({ version: "0.19.0", download: "https://github.com/dosaki/maya/releases/tag/v0.19.0" });
    press();
    await flush();
    expect(status()).toBe("Maya 0.19.0 is ready: see the bar above.");

    check = () => Promise.reject("could not check for updates: offline");
    press();
    await flush();
    expect(status()).toBe("could not check for updates: offline");
  });
```

- [ ] **Step 7: Run it to see it fail**

Run: `pnpm vitest run src/settings-flow.test.ts -t "Check for updates"`
Expected: FAIL (the button is not rendered: `updatesOn` is never set).

- [ ] **Step 8: Wire it in `initSettings`**

In `src/settings.ts`, import the instance at the top:

```ts
import { updates } from "./update";
```

In the `handlers` object in `initSettings`, add:

```ts
    onCheckUpdates: () => {
      model.updateStatus = "Checking…";
      paint();
      updates
        .check(true)
        .then((info) => {
          model.updateStatus = info ? `Maya ${info.version} is ${updates.kind() === "self" ? "ready" : "out"}: see the bar above.` : "Up to date.";
        })
        .catch((e) => {
          model.updateStatus = String(e);
        })
        .finally(() => {
          if (!panel.hidden) paint();
        });
    },
```

and in the `run(async () => { … })` block, right after `model.version = await version;`:

```ts
    await updates.ready();
    model.updatesOn = updates.kind() !== "off";
```

- [ ] **Step 9: Run the page suite**

Run: `pnpm test`
Expected: all pass, Task 5's tests included.

- [ ] **Step 10: Commit**

```bash
git add src/settings.ts src/settings.test.ts src/settings-flow.test.ts
git commit -m "feat: Check for updates in Settings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The feed manifest script

**Files:**
- Create: `scripts/updater-manifest.sh`, `scripts/test-updater-manifest.sh`
- Modify: `.github/workflows/ci.yml` (run the test in the `version` job)

**Interfaces:**
- Produces: `sh scripts/updater-manifest.sh <version> <dir>` prints `latest.json` to stdout; exits non-zero naming the first missing asset or `.sig`. `REPO` (default `dosaki/maya`) and `PUB_DATE` (default now, UTC) override the URLs and date. Used by Task 8.

- [ ] **Step 1: Write the failing test**

Create `scripts/test-updater-manifest.sh`:

```sh
#!/bin/sh
# Checks scripts/updater-manifest.sh against fake release assets: the JSON
# it prints for a complete set, and that a missing .sig fails it.
# Needs jq. Usage: sh scripts/test-updater-manifest.sh
set -eu
cd "$(dirname "$0")/.."
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
v=0.19.0
for a in "Maya_${v}_aarch64.app.tar.gz" "Maya_${v}_x86_64.app.tar.gz" "Maya_${v}_x64-setup.exe" "Maya_${v}_amd64.AppImage" "Maya_${v}_arm64.AppImage"; do
  : > "$dir/$a"
  printf 'sig-of-%s\n' "$a" > "$dir/$a.sig"
done

out="$(PUB_DATE=2026-10-20T10:00:00Z sh scripts/updater-manifest.sh "$v" "$dir")"
check() {
  got="$(printf '%s' "$out" | jq -r "$1")"
  [ "$got" = "$2" ] || { echo "FAIL: $1 is '$got', expected '$2'" >&2; exit 1; }
}
check .version "$v"
check .notes "https://github.com/dosaki/maya/releases/tag/v$v"
check .pub_date 2026-10-20T10:00:00Z
check '.platforms | keys | join(",")' "darwin-aarch64,darwin-x86_64,linux-aarch64,linux-x86_64,windows-x86_64"
check '.platforms["windows-x86_64"].url' "https://github.com/dosaki/maya/releases/download/v$v/Maya_${v}_x64-setup.exe"
check '.platforms["linux-aarch64"].signature' "sig-of-Maya_${v}_arm64.AppImage"
check '.platforms["darwin-x86_64"].url' "https://github.com/dosaki/maya/releases/download/v$v/Maya_${v}_x86_64.app.tar.gz"

rm "$dir/Maya_${v}_amd64.AppImage.sig"
if sh scripts/updater-manifest.sh "$v" "$dir" > /dev/null 2> "$dir/err"; then
  echo "FAIL: a missing .sig did not fail the manifest" >&2
  exit 1
fi
grep -q "Maya_${v}_amd64.AppImage.sig" "$dir/err" || { echo "FAIL: the error does not name the missing .sig" >&2; cat "$dir/err" >&2; exit 1; }
echo "updater-manifest: ok"
```

- [ ] **Step 2: Run it to see it fail**

Run: `sh scripts/test-updater-manifest.sh`
Expected: FAIL, `scripts/updater-manifest.sh: No such file or directory`.

- [ ] **Step 3: Write the script**

Create `scripts/updater-manifest.sh`:

```sh
#!/bin/sh
# Prints the release's updater feed, latest.json, in Tauri's static
# manifest format: for each desktop platform the asset's download URL and
# the contents of its .sig. Fails, naming it, when any asset or .sig is
# missing, so no release goes out that would break updates.
# Needs jq. REPO (default dosaki/maya) and PUB_DATE (default now) override.
# Usage: sh scripts/updater-manifest.sh <version> <dir> > <dir>/latest.json
set -eu
v="$1"
dir="$2"
repo="${REPO:-dosaki/maya}"
pub_date="${PUB_DATE:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
base="https://github.com/$repo/releases/download/v$v"
platforms='{}'
for pair in \
  "darwin-aarch64 Maya_${v}_aarch64.app.tar.gz" \
  "darwin-x86_64 Maya_${v}_x86_64.app.tar.gz" \
  "windows-x86_64 Maya_${v}_x64-setup.exe" \
  "linux-x86_64 Maya_${v}_amd64.AppImage" \
  "linux-aarch64 Maya_${v}_arm64.AppImage"; do
  platform="${pair%% *}"
  asset="${pair#* }"
  for f in "$asset" "$asset.sig"; do
    [ -f "$dir/$f" ] || { echo "updater-manifest: $f is missing from $dir" >&2; exit 1; }
  done
  platforms="$(printf '%s' "$platforms" | jq --arg p "$platform" --arg url "$base/$asset" --rawfile sig "$dir/$asset.sig" \
    '.[$p] = { signature: ($sig | rtrimstr("\n")), url: $url }')"
done
jq -n --arg v "$v" --arg notes "https://github.com/$repo/releases/tag/v$v" --arg date "$pub_date" --argjson platforms "$platforms" \
  '{ version: $v, notes: $notes, pub_date: $date, platforms: $platforms }'
```

- [ ] **Step 4: Run the test to see it pass**

Run: `sh scripts/test-updater-manifest.sh`
Expected: `updater-manifest: ok`.

- [ ] **Step 5: Run it in CI**

In `.github/workflows/ci.yml`'s `version` job, add a step after "Check the version":

```yaml
      - name: Check the updater manifest script
        run: sh scripts/test-updater-manifest.sh
```

- [ ] **Step 6: Commit**

```bash
git add scripts/updater-manifest.sh scripts/test-updater-manifest.sh .github/workflows/ci.yml
git commit -m "ci: a script that writes the updater feed, with its test

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Sign the updates and publish the feed

**Files:**
- Modify: `.github/workflows/build.yml` (secrets, the three desktop build steps), `.github/workflows/release.yml` (the manifest)

**Interfaces:**
- Consumes: `scripts/updater-manifest.sh` (Task 7); the secrets from Task 1.
- Produces: release assets `<asset>.sig` for the five feed assets, and `latest.json`.

- [ ] **Step 1: Declare the secrets**

In `.github/workflows/build.yml`'s `on.workflow_call.secrets`, after `ANDROID_KEY_PASSWORD`:

```yaml
      TAURI_SIGNING_PRIVATE_KEY:
        required: false
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD:
        required: false
```

- [ ] **Step 2: Linux: sign the AppImage on releases**

Replace the `linux-app` job's "Build the bundles" step with:

```yaml
      # A release also writes the AppImage's updater signature (the .deb
      # cannot update itself, so it has none); a pull request does not.
      - name: Build the bundles
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ inputs.sign && secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ inputs.sign && secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
        run: |
          set -euo pipefail
          version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
          updater=""
          if [ "${{ inputs.sign }}" = "true" ]; then updater='{"bundle":{"createUpdaterArtifacts":true}}'; fi
          pnpm tauri build --bundles deb,appimage ${updater:+--config "$updater"}
          mkdir -p release
          cp target/release/bundle/deb/*.deb "release/Maya_${version}_${{ matrix.deb }}.deb"
          cp target/release/bundle/appimage/*.AppImage "release/Maya_${version}_${{ matrix.deb }}.AppImage"
          if [ -n "$updater" ]; then
            cp target/release/bundle/appimage/*.AppImage.sig "release/Maya_${version}_${{ matrix.deb }}.AppImage.sig"
          fi
          chmod +x release/*.AppImage
          ls -la release
```

The smoke test copies `release/*.AppImage`, which does not match the `.sig`; leave it as it is.

- [ ] **Step 3: Windows: sign the NSIS installer on releases**

Replace the `windows` job's "Build the installers" step with (the job's `defaults.run.shell` is already `bash`):

```yaml
      - name: Build the installers
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ inputs.sign && secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ inputs.sign && secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
        run: |
          set -euo pipefail
          version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
          updater=""
          if [ "${{ inputs.sign }}" = "true" ]; then updater='{"bundle":{"createUpdaterArtifacts":true}}'; fi
          pnpm tauri build --bundles nsis,msi ${updater:+--config "$updater"}
          mkdir -p release
          cp target/release/bundle/nsis/*-setup.exe "release/Maya_${version}_x64-setup.exe"
          cp target/release/bundle/msi/*.msi "release/Maya_${version}_x64.msi"
          if [ -n "$updater" ]; then
            cp target/release/bundle/nsis/*-setup.exe.sig "release/Maya_${version}_x64-setup.exe.sig"
          fi
          ls -la release
```

(The MSI also gets a `.sig` from Tauri; it is not copied, since the feed points Windows at NSIS.)

- [ ] **Step 4: macOS: release Tauri's own tarball on releases**

Replace the `macos` job's "Build" step with:

```yaml
      - name: Build
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ inputs.sign && secrets.TAURI_SIGNING_PRIVATE_KEY || '' }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ inputs.sign && secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || '' }}
        run: |
          set -euo pipefail
          updater=""
          if [ "${{ inputs.sign }}" = "true" ]; then updater='{"bundle":{"createUpdaterArtifacts":true}}'; fi
          pnpm tauri build --target ${{ matrix.target }} --bundles app,dmg ${updater:+--config "$updater"}
```

and in its "Package" step replace the `tar -C "$bundle/macos" -czf …` line with:

```sh
          # A release ships Tauri's own tarball, since its updater
          # signature covers those exact bytes; it holds Maya.app at its
          # top, as install.sh expects. A pull request tars the app itself.
          if [ -f "$bundle/macos/Maya.app.tar.gz.sig" ]; then
            cp "$bundle/macos/Maya.app.tar.gz" "release/Maya_${version}_${{ matrix.arch }}.app.tar.gz"
            cp "$bundle/macos/Maya.app.tar.gz.sig" "release/Maya_${version}_${{ matrix.arch }}.app.tar.gz.sig"
          else
            tar -C "$bundle/macos" -czf "release/Maya_${version}_${{ matrix.arch }}.app.tar.gz" Maya.app
          fi
```

- [ ] **Step 5: Publish the feed**

In `.github/workflows/release.yml`'s `release` job, in the "Release" step's `run`, insert after `ls -la dist`:

```sh
          sh scripts/updater-manifest.sh "$VERSION" dist > latest.json
          mv latest.json dist/latest.json
```

The job has no checkout yet, so add one as its first step:

```yaml
      - uses: actions/checkout@v4
```

(`jq` is on `ubuntu-latest`.) Also add, to the release notes string, after the Android sentence: `From 0.18.0 the desktop app updates itself: a bar on the board offers each new release. An .msi or .deb install updates by hand.`

- [ ] **Step 6: Lint the workflows**

Run: `actionlint .github/workflows/*.yml` if `actionlint` is installed (`brew install actionlint`); otherwise `python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/*.yml`.
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/build.yml .github/workflows/release.yml
git commit -m "ci: sign each release's updates and publish latest.json

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Docs and the version

**Files:**
- Modify: `docs/DEVELOPING.md` (CI and a new "Updates" section), `README.md` (updating), `docs/superpowers/specs/2026-10-13-auto-update-design.md` (status)
- Version files via `scripts/set-version.sh`

- [ ] **Step 1: Document the updater for developers**

In `docs/DEVELOPING.md`, add a section `## Updates` before `## Signing`:

```markdown
## Updates

The desktop app updates itself from `latest.json`, which every release
publishes beside its assets (`scripts/updater-manifest.sh`, tested by
`scripts/test-updater-manifest.sh` in CI). `src-tauri/src/updater.rs`
checks it through `tauri-plugin-updater` on macOS, Windows and the
AppImage, and through `maya_core::update` on a `.deb` and the phone, which
only show a Download link. `src/update.ts` is the bar on the board and the
checks (at start, then every 6 hours); a debug build never checks.

Each update is signed with the key in the `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets, and the app checks it against
`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. A release build
turns on `createUpdaterArtifacts` and writes the `.sig` files; a pull
request's does not. A release with any platform's `.sig` missing fails.
Losing the private key means every installed Maya must be reinstalled by
hand once with a new one.
```

In its "CI" section's `release.yml` bullet, add after "publishes `v<version>` with all the artifacts": `, the `.sig` files and `latest.json``.

- [ ] **Step 2: Tell users**

In `README.md`, replace `Run the same command again to update Maya.` with:

```markdown
From 0.18.0, Maya updates itself: a bar on the board offers each new
release, and Settings has Check for updates. Running the same command
again also updates Maya, and is the only way for an `.msi` or `.deb`
install.
```

- [ ] **Step 3: Mark the spec implemented**

In the spec, change `Status: approved` to `Status: implemented`.

- [ ] **Step 4: Commit the docs**

```bash
git add docs/DEVELOPING.md README.md docs/superpowers/specs/2026-10-13-auto-update-design.md
git commit -m "docs: how Maya updates itself

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 5: Bump the version**

```bash
git fetch origin && git show origin/main:src-tauri/tauri.conf.json | grep '"version"'
```

Expected: `"version": "0.17.0"`. If `main` has moved on, bump its minor instead. Then:

```bash
PATH="$HOME/.cargo/bin:$PATH" sh scripts/set-version.sh 0.18.0
git diff --stat
```

Expected: `src-tauri/tauri.conf.json`, `mobile/tauri.conf.json`, `package.json`, `Cargo.toml` and `Cargo.lock` changed. If `Cargo.lock` did not, run `cargo update -w` and check again.

- [ ] **Step 6: Run everything**

Run: `pnpm test && cargo test --workspace && sh scripts/test-updater-manifest.sh && sh scripts/release-version.sh`
Expected: all pass; the last prints `version=0.18.0`.

- [ ] **Step 7: Commit the version**

```bash
git add -A src-tauri/tauri.conf.json mobile/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.18.0

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 8: Record the hand check**

Add a memory (`maya-update-hand-check.md`) that 0.18.0 must be installed by hand on the Mac, the laptop and the phone, and that the next release after it is the real test: the bar appears, Update applies and relaunches with sessions intact, the phone's Download opens the APK. Link it from `MEMORY.md`.

- [ ] **Step 9: Before the pull request**

Confirm with the user that the Task 1 secrets are set (`gh secret list` shows both), then ask before pushing `feat/auto-update`.
