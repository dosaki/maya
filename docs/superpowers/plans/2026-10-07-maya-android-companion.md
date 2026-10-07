# Maya for Android Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An Android app that is a main Maya: it runs the existing server, shows every paired assistant's sessions on a phone-sized board, drives them, and notifies with the screen off.

**Architecture:** A new thin Tauri 2 crate `mobile/` depends on `maya_core` and implements the same Tauri command names the page already calls, every one routed to an assistant through the core's server. The frontend gets a second Vite entry that reuses the board, card, modal and dialog modules with a phone stylesheet. A small Kotlin foreground service keeps the process alive, and the core's notifier diff drives Android notifications.

**Tech Stack:** Rust (Tauri 2.12, `maya_core`, `tauri-plugin-notification`, `tauri-plugin-opener`), Kotlin (one service, one Tauri plugin class), TypeScript + Vite + vitest, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-07-maya-android-companion-design.md`

## Global Constraints

- The crate is `maya-mobile` in `mobile/`, Tauri identifier `com.dosaki.maya.mobile`, product name `Maya`; it carries the workspace version, kept in step by `scripts/set-version.sh` and checked by `scripts/release-version.sh`.
- The phone is only ever a main. There are no local sessions: every session command resolves its machine with `merge::route(&[], …)` and sends through `send_command_with`, timeout 30 s.
- Port default 4127; name default is the device model, "Android" when blank. Config lives at `<app data dir>/maya/config.json`, the log at `<app data dir>/maya/maya.log`, attachments under `<app data dir>/maya/attachments/`.
- Columns are at least 200 dp wide, as many as fit up to four, one column in portrait; the app opens on Awaiting Decision when it has cards, else Working.
- Stale greying after 30 s, disappearance after five minutes, the 20 MB attachment cap and the pairing rules are the core's and are not reimplemented.
- Notification channels: `decisions` (Importance High, vibration) and `finished` (Importance Default). One notification per session, id from a stable hash of the session id, replaced in place and cleared when the card leaves the column or the board.
- Exact copy: "Pair an assistant first.", "Pair an assistant to see its sessions.", "The server is stopped.", "Start the server first.", "Session is no longer running.", "Notifications are off for Maya in Android settings.", "Android may otherwise stop the server with the screen off.", "Set a projects directory in Settings on <label>.", "<name> on <machine> needs a decision", "<name> on <machine> is finished".
- Every pull request bumps the version (CLAUDE.md): this one is a `feat`, so the minor version, redone against `main` at merge time.
- Commit messages are conventional (`feat:`, `fix:`, `docs:`, `chore:`), one logical change each.

## Review Focus

1. **Two assistants with the same name.** The core labels them `name (address)`; cards, the machine picker and notification titles must all use that label, never the bare name. Pinned in Task 3 (`title_for` uses `card.machine` as merged) and Task 2 (`machines_of` returns the status's display names).
2. **A decision answered at the desk.** The phone's notification for it must go when the card leaves Awaiting without the phone acting. Pinned in Task 3 (`plan` clears a posted session whose state moved) and Task 4 (the round trip sees the clear).
3. **The port is taken at app start.** The server must not start, the foreground service must not start, and the Network screen must show the error under Port. Pinned in Task 4 (`start_server` records `main_error`, returns `Err`, starts no service, and the status carries it) and Task 7 (the Network screen renders `mainError`).
4. **Rotation mid-board.** The column on screen must stay on screen when the column count changes. Pinned in Task 6 (`scrollLeftFor` after a width change keeps the active column).
5. **A repaint while swiping.** The board repaints once a second while a session works; the horizontal scroll must survive it. Pinned in Task 6 (`swapBoard` carries the board's `scrollLeft`).

---

## File Structure

**Core (`core/`)**
- Modify `core/src/config.rs`: `notify_on_completed` on `Config`.
- Create `core/src/net/routing.rs`: the main-side helpers shared by the desktop and the phone (`remote_attachments`, `check_remote_agent`, `agents_reply`, `AgentsReply`, `MachineInfo`, `machines_of`, `ROUTE_TIMEOUT`), moved from `src-tauri/src/lib.rs`.
- Modify `core/src/net/mod.rs`: `pub mod routing;`.
- Modify `src-tauri/src/lib.rs`: use the moved helpers.

**Mobile crate (`mobile/`)**
- `mobile/Cargo.toml`, `mobile/build.rs`, `mobile/tauri.conf.json`, `mobile/capabilities/default.json`, `mobile/icons/` (copied from `src-tauri/icons/`).
- `mobile/src/lib.rs`: `AppState`, `Settings`, config load/save, log init, `run()`, the `Notify` adapter, `refresh_and_emit`, server start/stop.
- `mobile/src/alerts.rs`: the `Alerts` trait, `Post`, `Action`, `plan`, `notification_id`, `title_for`, `finished_body`, `service_line`, `default_name`. Pure, host-tested.
- `mobile/src/commands.rs`: every Tauri command.
- `mobile/src/android.rs`: the `keepalive` plugin bridge (Android) and its host stubs; `AndroidAlerts` and `HostAlerts`.
- `mobile/tests/round_trip.rs`: the localhost integration test.
- `mobile/gen/android/`: generated by `tauri android init`, plus `KeepAliveService.kt`, `KeepAlivePlugin.kt`, manifest entries, signing config.

**Frontend (`src/`)**
- Create `src/machines.ts`: the local-machine switch and `machineChoices`.
- Modify `src/newsession.ts`, `src/resume.ts`: use `machines.ts`; `noMachines` state.
- Modify `src/board.ts`: `swapBoard` carries the board's own `scrollLeft`.
- Modify `src/modal.ts`: `setComposerOptions({ attachButton })` adds an Attach button feeding `onPasteFiles`.
- Create `mobile/web/index.html`, `vite.mobile.config.ts`, `src/mobile/main.ts`, `src/mobile/strip.ts`, `src/mobile/network.ts`, `src/mobile/setup.ts`, `src/mobile/notify-tap.ts`, `src/mobile/mobile.css`, with tests beside them.
- Modify `package.json`: scripts and `@tauri-apps/plugin-notification`.

**Build and docs**
- Modify `scripts/set-version.sh`, `scripts/release-version.sh`, `.github/workflows/build.yml`, `.github/workflows/release.yml`, `docs/DEVELOPING.md`, `README.md`, `.gitignore`.

---

### Task 0: Toolchain on the development Mac (no commit)

This Mac has Homebrew's `rustc 1.87` (no `rustup`, so no Android targets), Java 21, an Android SDK at `/opt/homebrew/share/android-commandlinetools` with platform 35 and build-tools 35, an emulator image, but no NDK. Do these once before Task 3.

- [ ] **Step 1: Install rustup and the Android targets**

```bash
brew install rustup
rustup-init -y --no-modify-path
# Put ~/.cargo/bin before /opt/homebrew/bin so cargo is rustup's:
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
cargo --version   # must print a rustup-managed toolchain, not "(Homebrew)"
rustup target add aarch64-linux-android x86_64-linux-android
```

- [ ] **Step 2: Install the NDK and set the environment**

```bash
sdkmanager "ndk;27.2.12479018" "platforms;android-35" "build-tools;35.0.0"
cat >> ~/.zshrc <<'ZSH'
export ANDROID_HOME="/opt/homebrew/share/android-commandlinetools"
export NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
ZSH
source ~/.zshrc
ls "$NDK_HOME/toolchains/llvm/prebuilt"   # darwin-x86_64 (works on Apple Silicon)
```

- [ ] **Step 3: Check the workspace still builds with rustup's toolchain**

Run: `cargo test -p maya-core`
Expected: all tests pass (the Homebrew toolchain is no longer in play).

---

### Task 1: `notify_on_completed` on the core config

**Files:**
- Modify: `core/src/config.rs:58-61` (field) and `core/src/config.rs:143-162` (`Default`)

**Interfaces:**
- Produces: `Config.notify_on_completed: bool`, default `true`, serde default `true`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `core/src/config.rs`:

```rust
    #[test]
    fn notify_on_completed_defaults_on_and_reads_from_older_files() {
        assert!(Config::default().notify_on_completed);
        let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes": 30, "notifyOnAwaiting": false}"#).unwrap();
        assert!(older.notify_on_completed, "a file written before the field reads as on");
        assert!(!older.notify_on_awaiting);
        let text = serde_json::to_string(&Config { notify_on_completed: false, ..Config::default() }).unwrap();
        assert!(text.contains(r#""notifyOnCompleted":false"#));
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya-core notify_on_completed`
Expected: FAIL, "no field `notify_on_completed`".

- [ ] **Step 3: Add the field**

After `notify_on_awaiting` in the struct:

```rust
    /// Post a notification when a session's turn finishes. Read by the
    /// phone; the desktop announces finished turns under `notify_on_awaiting`.
    #[serde(default = "default_true")]
    pub notify_on_completed: bool,
```

and in `Default`, after `notify_on_awaiting: true,`:

```rust
            notify_on_completed: true,
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core config::`
Expected: PASS, including the new one.

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): a notify-on-completed switch in the config"
```

---

### Task 2: Move the main-side routing helpers into the core

The desktop's `lib.rs` holds helpers every main needs: encoding attachments for a remote reply, refusing an agent a machine cannot start, the machine list for the pickers. The phone needs the same ones; they move to `maya_core::net::routing` and the desktop uses them from there. Behaviour does not change; the four attachment tests move with the code.

**Files:**
- Create: `core/src/net/routing.rs`
- Modify: `core/src/net/mod.rs` (add `pub mod routing;`)
- Modify: `src-tauri/src/lib.rs:112-166` (remove `remote_attachments`, `MachineInfo`), `:900-936` (remove `AgentsReply`, `agents_reply`, `check_remote_agent`), `:1310-1360` (remove the four `remote_attachments_*` tests)

**Interfaces:**
- Produces (all `pub` in `maya_core::net::routing`):
  - `pub const ROUTE_TIMEOUT: Duration` (30 s)
  - `pub struct MachineInfo { pub name: String, pub hostname: String, pub platform: String, pub connected: bool }` (Serialize, camelCase)
  - `pub struct AgentsReply { pub agents: Vec<AgentInfo>, pub names: bool }` (Serialize, camelCase)
  - `pub fn remote_attachments(paths: &[String]) -> Result<Vec<Attachment>, String>`
  - `pub fn agents_reply(local: bool, remote: Option<Vec<AgentInfo>>, local_list: impl FnOnce() -> Vec<AgentInfo>) -> AgentsReply`
  - `pub fn check_remote_agent(agent: Harness, remote: &Option<Vec<AgentInfo>>, machine: &str) -> Result<(), String>`
  - `pub fn machines_of(status: &NetworkStatus) -> Vec<MachineInfo>`

- [ ] **Step 1: Write the failing tests in the new module**

Create `core/src/net/routing.rs` with only the tests first:

```rust
//! What a main does with a session command bound for an assistant: the
//! helpers the desktop app and the phone share. Sending itself is
//! `server::send_command_with`; this is what wraps the arguments.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentInfo;
    use crate::model::Harness;
    use crate::net::{AssistantStatus, NetworkStatus};
    use base64::Engine;

    #[test]
    fn remote_attachments_encodes_a_small_file() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");
        std::fs::write(&path, b"hello there").unwrap();
        let path_str = path.to_string_lossy().into_owned();
        let out = remote_attachments(&[path_str.clone()]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, path_str, "the name carries the main's full local path exactly");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&out[0].bytes).unwrap(), b"hello there");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remote_attachments_refuses_a_missing_file() {
        let err = remote_attachments(&["/no/such/file-for-maya-tests.txt".to_string()]).unwrap_err();
        assert!(err.contains("Attachment not found"), "{err}");
    }

    #[test]
    fn remote_attachments_refuses_a_file_over_20_mb() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        std::fs::File::create(&path).unwrap().set_len(20 * 1024 * 1024 + 1).unwrap();
        let err = remote_attachments(&[path.to_string_lossy().into_owned()]).unwrap_err();
        assert_eq!(err, "The file is too large (over 20 MB).");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remote_attachments_refuses_files_over_20_mb_together() {
        let dir = std::env::temp_dir().join(format!("maya-route-test-total-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = (0..2)
            .map(|i| {
                let path = dir.join(format!("part{i}.bin"));
                std::fs::File::create(&path).unwrap().set_len(11 * 1024 * 1024).unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        assert!(remote_attachments(&paths[..1]).is_ok(), "one alone fits");
        assert_eq!(remote_attachments(&paths).unwrap_err(), "Attachments total more than 20 MB.");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn agent(h: Harness) -> AgentInfo {
        AgentInfo { harness: h, models: vec![], efforts: vec![], modes: vec![] }
    }

    #[test]
    fn an_older_remote_starts_claude_code_only_and_ignores_names() {
        let reply = agents_reply(false, None, Vec::new);
        assert_eq!(reply.agents.len(), 1);
        assert_eq!(reply.agents[0].harness, Harness::ClaudeCode);
        assert!(!reply.names);
        let reply = agents_reply(false, Some(vec![agent(Harness::Codex)]), Vec::new);
        assert_eq!(reply.agents[0].harness, Harness::Codex);
        assert!(reply.names);
        let reply = agents_reply(true, None, || vec![agent(Harness::Kiro)]);
        assert_eq!(reply.agents[0].harness, Harness::Kiro);
        assert!(reply.names);
    }

    #[test]
    fn check_remote_agent_refuses_what_the_machine_cannot_start() {
        assert_eq!(check_remote_agent(Harness::Codex, &None, "laptop").unwrap_err(), "laptop runs an older Maya that can only start Claude Code.");
        assert!(check_remote_agent(Harness::ClaudeCode, &None, "laptop").is_ok());
        let list = Some(vec![agent(Harness::ClaudeCode)]);
        assert_eq!(check_remote_agent(Harness::Codex, &list, "laptop").unwrap_err(), "That agent is not installed on laptop.");
        assert!(check_remote_agent(Harness::ClaudeCode, &list, "laptop").is_ok());
    }

    #[test]
    fn machines_of_keeps_the_status_display_names() {
        let status = NetworkStatus {
            assistants: vec![
                AssistantStatus { id: "a".into(), name: "laptop (10.0.0.2)".into(), hostname: "laptop".into(), platform: "macos".into(), address: "10.0.0.2".into(), connected: true, last_seen: None, note: None },
                AssistantStatus { id: "b".into(), name: "laptop (10.0.0.3)".into(), hostname: "laptop".into(), platform: "linux".into(), address: "10.0.0.3".into(), connected: false, last_seen: Some(1), note: None },
            ],
            ..Default::default()
        };
        let m = machines_of(&status);
        assert_eq!(m.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["laptop (10.0.0.2)", "laptop (10.0.0.3)"]);
        assert_eq!((m[0].connected, m[1].connected), (true, false));
        assert_eq!(m[1].platform, "linux");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Add `pub mod routing;` to `core/src/net/mod.rs` (beside `pub mod merge;`), then:
Run: `cargo test -p maya-core routing::`
Expected: FAIL to compile, "cannot find function `remote_attachments`".

- [ ] **Step 3: Write the module body**

Above the tests in `core/src/net/routing.rs`:

```rust
use super::protocol::Attachment;
use super::NetworkStatus;
use crate::agents::{self, AgentInfo};
use crate::model::Harness;
use base64::Engine;
use serde::Serialize;
use std::time::Duration;

/// How long a main waits for a command's result from an assistant.
pub const ROUTE_TIMEOUT: Duration = Duration::from_secs(30);

/// The command must fit in one frame: one file, and all of them together.
const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// What the Machine pickers show: each paired assistant's display name,
/// host and platform, and whether it is connected now.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub name: String,
    pub hostname: String,
    pub platform: String,
    pub connected: bool,
}

/// The agents a machine can start, and whether its Maya takes a session
/// name (an older one ignores names).
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentsReply {
    pub agents: Vec<AgentInfo>,
    pub names: bool,
}

/// Reads and base64-encodes local files for a remote reply's attachments;
/// refuses a missing file, one over 20 MB, or files over 20 MB together.
/// `name` on each `Attachment` is the path exactly as given, since the
/// assistant matches on it.
pub fn remote_attachments(paths: &[String]) -> Result<Vec<Attachment>, String> {
    let mut total = 0u64;
    for p in paths {
        let meta = std::fs::metadata(p).map_err(|_| format!("Attachment not found: {p}"))?;
        if meta.len() > MAX_BYTES {
            return Err("The file is too large (over 20 MB).".into());
        }
        total += meta.len();
    }
    if total > MAX_BYTES {
        return Err("Attachments total more than 20 MB.".into());
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let bytes = std::fs::read(p).map_err(|_| format!("Attachment not found: {p}"))?;
        out.push(Attachment { name: p.clone(), bytes: base64::engine::general_purpose::STANDARD.encode(bytes) });
    }
    Ok(out)
}

/// A remote that reports no agents runs an older Maya: it starts only
/// Claude Code and ignores a name. Locally, `local_list` is asked.
pub fn agents_reply(local: bool, remote: Option<Vec<AgentInfo>>, local_list: impl FnOnce() -> Vec<AgentInfo>) -> AgentsReply {
    if local {
        return AgentsReply { agents: local_list(), names: true };
    }
    match remote {
        Some(agents) => AgentsReply { agents, names: true },
        None => AgentsReply { agents: vec![agents::claude()], names: false },
    }
}

/// An older Maya ignores the agent and starts Claude Code: refuse instead.
pub fn check_remote_agent(agent: Harness, remote: &Option<Vec<AgentInfo>>, machine: &str) -> Result<(), String> {
    match remote {
        None if agent != Harness::ClaudeCode => Err(format!("{machine} runs an older Maya that can only start Claude Code.")),
        Some(list) if !list.iter().any(|a| a.harness == agent) => Err(format!("That agent is not installed on {machine}.")),
        _ => Ok(()),
    }
}

/// The pickers' machine list from the network status: the display names
/// the cards use (`name (address)` for assistants sharing a name).
pub fn machines_of(status: &NetworkStatus) -> Vec<MachineInfo> {
    status.assistants.iter().map(|a| MachineInfo { name: a.name.clone(), hostname: a.hostname.clone(), platform: a.platform.clone(), connected: a.connected }).collect()
}
```

- [ ] **Step 4: Run the core tests**

Run: `cargo test -p maya-core routing::`
Expected: 7 tests PASS.

- [ ] **Step 5: Point the desktop at the moved helpers**

In `src-tauri/src/lib.rs`:
- Delete `const ROUTE_TIMEOUT`, `fn remote_attachments`, `struct MachineInfo`, `struct AgentsReply`, `fn agents_reply`, `fn check_remote_agent`, and the four `remote_attachments_*` tests.
- Add to the imports: `use maya_core::net::routing::{agents_reply, check_remote_agent, machines_of, remote_attachments, AgentsReply, MachineInfo, ROUTE_TIMEOUT};`
- Replace the body of `list_machines` with `machines_of(&network_status_of(&state))`.
- If `AgentsReply` or `MachineInfo` was referenced elsewhere in `src-tauri/src/` (`grep -rn "MachineInfo\|AgentsReply" src-tauri/src`), the import covers it.

- [ ] **Step 6: Build and test the desktop and the CLI**

Run: `cargo test -p maya-core && cargo build -p maya && cargo test -p maya-cli`
Expected: all green; `cargo test -p maya` needs the ear sidecar, so `cargo build -p maya` is the check here unless `src-tauri/binaries/maya-ear-*` exists, in which case run `cargo test -p maya` too.

- [ ] **Step 7: Commit**

```bash
git add core/src/net/routing.rs core/src/net/mod.rs src-tauri/src/lib.rs
git commit -m "refactor(core): share the main's routing helpers with other mains"
```

---

### Task 3: The `mobile/` crate skeleton and the alert planner

A Tauri crate that compiles and tests on the host (macOS, Linux), with no commands yet, plus `alerts.rs`: the pure decision of which Android notifications a board change posts and clears. The Android side is behind the `Alerts` trait, so the planner is tested here.

**Files:**
- Create: `mobile/Cargo.toml`, `mobile/build.rs`, `mobile/tauri.conf.json`, `mobile/capabilities/default.json`, `mobile/src/main.rs`, `mobile/src/lib.rs`, `mobile/src/alerts.rs`, `mobile/src/android.rs`
- Copy: `src-tauri/icons/*` to `mobile/icons/`
- Modify: `Cargo.toml` (workspace members), `.gitignore`, `scripts/set-version.sh`, `scripts/release-version.sh`

**Interfaces:**
- Produces (`mobile/src/alerts.rs`):
  - `pub enum Kind { Decision, Finished }` with `pub fn channel_id(self) -> &'static str` ("decisions", "finished")
  - `pub struct Post { pub id: i32, pub kind: Kind, pub title: String, pub body: String, pub session_id: String }`
  - `pub enum Action { Post(Post), Clear(i32) }`
  - `pub struct Switches { pub awaiting: bool, pub completed: bool }` (Copy, Default)
  - `pub struct Posted` (Default): the notifications on screen by session id
  - `pub trait Alerts: Send + Sync { fn post(&self, post: &Post); fn clear(&self, id: i32); fn service_line(&self, line: &str); }`
  - `pub fn plan(cards: &[Card], fresh: &[Card], finished: &[Card], posted: &mut Posted, switches: Switches) -> Vec<Action>`
  - `pub fn notification_id(session_id: &str) -> i32` (stable, ≥ 2; 1 is the service's)
  - `pub fn title_for(card: &Card) -> String`, `pub fn finished_body(card: &Card) -> String`
  - `pub fn service_line(paired: usize, connected: usize) -> String`
  - `pub fn default_name(model: &str) -> String`
- Produces (`mobile/src/android.rs`, generic over `R: Runtime`, Android bodies and host no-ops): `init() -> TauriPlugin<R>`, `service_start(&AppHandle<R>, &str)`, `service_update`, `service_stop`, `request_battery_exemption -> Result<(), String>`, `notifications_allowed -> bool`, `device_model -> String`, `local_addresses -> Vec<String>`, `create_channels -> Result<(), String>`, `alerts(&AppHandle<R>) -> Arc<dyn Alerts>`.

- [ ] **Step 1: Scaffold the crate**

`mobile/Cargo.toml`:

```toml
[package]
name = "maya-mobile"
version.workspace = true
description = "Maya for Android: a main Maya that coordinates the assistants paired with it"
authors.workspace = true
edition.workspace = true

[lib]
name = "maya_mobile_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
maya-core = { path = "../core" }
tauri = { version = "2", features = [] }
tauri-plugin-notification = "2"
tauri-plugin-opener = "2"
serde = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

`mobile/build.rs`:

```rust
fn main() {
    tauri_build::build()
}
```

`mobile/src/main.rs`:

```rust
// The desktop entry, for running the phone's app on a Mac or a Linux box
// while developing it; Android enters through `maya_mobile_lib::run`.
fn main() {
    maya_mobile_lib::run()
}
```

`mobile/tauri.conf.json` (the version is whatever `Cargo.toml` says today):

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Maya",
  "version": "0.12.0",
  "identifier": "com.dosaki.maya.mobile",
  "build": {
    "beforeDevCommand": "pnpm dev:mobile",
    "devUrl": "http://localhost:1430",
    "beforeBuildCommand": "pnpm build:mobile",
    "frontendDist": "../dist-mobile"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "title": "Maya",
        "label": "main",
        "width": 420,
        "height": 860
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "android": {
      "minSdkVersion": 26
    }
  }
}
```

`mobile/capabilities/default.json`:

```json
{
  "identifier": "default",
  "description": "The phone's one window",
  "windows": ["main"],
  "permissions": ["core:default", "notification:default", "opener:default"]
}
```

Then: `cp -R src-tauri/icons mobile/icons`, add `"mobile"` to `members` in the root `Cargo.toml`, and append to `.gitignore`:

```
mobile/gen/schemas
dist-mobile
```

- [ ] **Step 2: Version scripts**

In `scripts/set-version.sh`, change the first `sed` line to cover the second conf and the `rm` and `grep` lines to match:

```sh
sed -i.bak -E "s/^(  \"version\": \")[^\"]+(\",)$/\1$v\2/" package.json src-tauri/tauri.conf.json mobile/tauri.conf.json
```
```sh
rm -f package.json.bak src-tauri/tauri.conf.json.bak mobile/tauri.conf.json.bak Cargo.toml.bak
```
```sh
grep -H '"version"' package.json src-tauri/tauri.conf.json mobile/tauri.conf.json
```

In `scripts/release-version.sh`, after `cargo_version=…` add:

```sh
mobile_version="$(node -p "require('./mobile/tauri.conf.json').version")"
```
and extend the check:
```sh
if [ "$version" != "$pkg" ] || [ "$version" != "$cargo_version" ] || [ "$version" != "$mobile_version" ]; then
  echo "version mismatch: tauri.conf.json=$version package.json=$pkg Cargo.toml=$cargo_version mobile/tauri.conf.json=$mobile_version" >&2
```

Run: `sh scripts/set-version.sh 0.12.0 && git diff --stat`
Expected: the four files listed by the script, no content change beyond `Cargo.lock` picking up `maya-mobile`.

- [ ] **Step 3: Write the alert planner's failing tests**

`mobile/src/alerts.rs`, tests first:

```rust
//! Which Android notifications a board change posts and clears, and the
//! lines they and the foreground service show. Pure: the Android side is
//! behind `Alerts`, so this is tested on the host.

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::model::{AwaitKind, Awaiting, Harness};

    fn card(id: &str, state: State, since: u64) -> Card {
        Card {
            session_id: id.into(),
            pid: 0,
            name: "hexgrid".into(),
            cwd: "/home/u/dev/hexgrid".into(),
            state,
            state_since: since,
            snippet: "All done, the branch is pushed.".into(),
            awaiting: (state == State::Awaiting).then(|| Awaiting { kind: AwaitKind::Permission, detail: "Run `cargo test`?".into(), questions: vec![] }),
            has_inbox: true,
            harness: Harness::ClaudeCode,
            pr: None,
            context: None,
            machine: Some("maya-mini (10.0.0.2)".into()),
            machine_address: Some("10.0.0.2".into()),
            machine_platform: Some("macos".into()),
            terminal: None,
            stale: false,
            model: None,
        }
    }

    const ON: Switches = Switches { awaiting: true, completed: true };

    #[test]
    fn titles_carry_the_machine_label_as_merged() {
        assert_eq!(title_for(&card("a", State::Awaiting, 1)), "hexgrid on maya-mini (10.0.0.2) needs a decision");
        assert_eq!(title_for(&card("a", State::Completed, 1)), "hexgrid on maya-mini (10.0.0.2) is finished");
        let mut local = card("a", State::Awaiting, 1);
        local.machine = None;
        assert_eq!(title_for(&local), "hexgrid needs a decision");
    }

    #[test]
    fn a_new_ask_posts_once_on_the_decisions_channel_with_the_ask_as_body() {
        let c = card("a", State::Awaiting, 1);
        let mut posted = Posted::default();
        let out = plan(&[c.clone()], &[c.clone()], &[], &mut posted, ON);
        assert_eq!(out.len(), 1);
        let Action::Post(p) = &out[0] else { panic!("{out:?}") };
        assert_eq!((p.kind, p.id, p.session_id.as_str()), (Kind::Decision, notification_id("a"), "a"));
        assert_eq!(p.body, "Run `cargo test`?");
        // The same board again: nothing new, nothing cleared.
        assert!(plan(&[c], &[], &[], &mut posted, ON).is_empty());
    }

    #[test]
    fn a_decision_taken_elsewhere_clears_its_notification() {
        let asked = card("a", State::Awaiting, 1);
        let mut posted = Posted::default();
        plan(&[asked.clone()], &[asked], &[], &mut posted, ON);
        let working = card("a", State::Working, 2);
        assert_eq!(plan(&[working], &[], &[], &mut posted, ON), vec![Action::Clear(notification_id("a"))]);
        // Gone from the board entirely: also cleared, once.
        let asked = card("b", State::Awaiting, 1);
        plan(&[asked.clone()], &[asked], &[], &mut posted, ON);
        assert_eq!(plan(&[], &[], &[], &mut posted, ON), vec![Action::Clear(notification_id("b"))]);
        assert!(plan(&[], &[], &[], &mut posted, ON).is_empty());
    }

    #[test]
    fn awaiting_then_finished_swaps_channels_and_the_switches_gate_each() {
        let asked = card("a", State::Awaiting, 1);
        let mut posted = Posted::default();
        plan(&[asked.clone()], &[asked], &[], &mut posted, ON);
        let done = card("a", State::Completed, 2);
        let out = plan(&[done.clone()], &[], &[done.clone()], &mut posted, ON);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], Action::Clear(notification_id("a")));
        let Action::Post(p) = &out[1] else { panic!("{out:?}") };
        assert_eq!((p.kind, p.body.as_str()), (Kind::Finished, "All done, the branch is pushed."));
        // Finished off: the finish is neither posted nor remembered.
        let mut quiet = Posted::default();
        assert!(plan(&[done.clone()], &[], &[done], &mut quiet, Switches { awaiting: true, completed: false }).is_empty());
        let asked = card("c", State::Awaiting, 1);
        assert!(plan(&[asked.clone()], &[asked], &[], &mut quiet, Switches { awaiting: false, completed: true }).is_empty());
    }

    #[test]
    fn notification_ids_are_stable_positive_and_never_the_services() {
        assert_eq!(notification_id("abc"), notification_id("abc"));
        assert_ne!(notification_id("abc"), notification_id("abd"));
        assert!(notification_id("") >= 2);
        for id in ["1", "x", "session-9"] {
            assert!(notification_id(id) >= 2, "{id}");
        }
    }

    #[test]
    fn service_lines_count_assistants() {
        assert_eq!(service_line(0, 0), "No assistants paired yet");
        assert_eq!(service_line(1, 1), "Main for 1 assistant, connected");
        assert_eq!(service_line(1, 0), "Main for 1 assistant, not connected");
        assert_eq!(service_line(3, 2), "Main for 3 assistants, 2 connected");
    }

    #[test]
    fn the_default_name_is_the_model_or_android() {
        assert_eq!(default_name(" Pixel 8 "), "Pixel 8");
        assert_eq!(default_name(""), "Android");
        assert_eq!(default_name("   "), "Android");
    }

    #[test]
    fn a_finished_turn_with_no_snippet_still_has_a_body() {
        let mut c = card("a", State::Completed, 1);
        c.snippet = "  ".into();
        assert_eq!(finished_body(&c), "Finished its turn");
        c.snippet = "x".repeat(300);
        assert_eq!(finished_body(&c).chars().count(), 200);
    }
}
```

- [ ] **Step 4: Write `android.rs` and a minimal `lib.rs` so the crate compiles, then run the tests to see them fail**

`mobile/src/android.rs`:

```rust
//! The Android side: the `keepalive` plugin (the foreground service, the
//! battery exemption, the device's name and addresses) and the Android
//! notifications. On any other OS, which only happens when the crate is
//! run on a desktop for development, every call is a logged no-op.

use crate::alerts::{Alerts, Post};
use maya_core::log;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Runtime};

#[allow(dead_code)]
#[derive(Serialize)]
struct LineArgs<'a> {
    line: &'a str,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct BoolReply {
    value: bool,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TextReply {
    value: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ListReply {
    value: Vec<String>,
}

/// The Kotlin `KeepAlivePlugin`, once registered.
#[cfg(target_os = "android")]
struct KeepAlive<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// The app's own mobile plugin: on Android it loads `KeepAlivePlugin` from
/// the app's package; elsewhere it registers nothing.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("keepalive")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.dosaki.maya.mobile", "KeepAlivePlugin")?;
                app.manage(KeepAlive(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use tauri::Manager;
    use tauri_plugin_notification::{Channel, Importance, NotificationExt};

    fn call<R: Runtime, T: serde::de::DeserializeOwned>(app: &AppHandle<R>, command: &str, args: impl Serialize) -> Result<T, String> {
        app.state::<KeepAlive<R>>().0.run_mobile_plugin(command, args).map_err(|e| e.to_string())
    }

    fn unit<R: Runtime>(app: &AppHandle<R>, command: &str, args: impl Serialize) {
        if let Err(e) = call::<R, serde_json::Value>(app, command, args) {
            log::line("android", format!("{command}: {e}"));
        }
    }

    pub fn service_start<R: Runtime>(app: &AppHandle<R>, line: &str) {
        unit(app, "start", LineArgs { line })
    }

    pub fn service_update<R: Runtime>(app: &AppHandle<R>, line: &str) {
        unit(app, "update", LineArgs { line })
    }

    pub fn service_stop<R: Runtime>(app: &AppHandle<R>) {
        unit(app, "stop", ())
    }

    pub fn request_battery_exemption<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        call::<R, serde_json::Value>(app, "requestBatteryExemption", ()).map(|_| ())
    }

    pub fn notifications_allowed<R: Runtime>(app: &AppHandle<R>) -> bool {
        call::<R, BoolReply>(app, "notificationsAllowed", ()).map(|r| r.value).unwrap_or(true)
    }

    pub fn device_model<R: Runtime>(app: &AppHandle<R>) -> String {
        call::<R, TextReply>(app, "deviceModel", ()).map(|r| r.value).unwrap_or_default()
    }

    pub fn local_addresses<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
        call::<R, ListReply>(app, "localAddresses", ()).map(|r| r.value).unwrap_or_default()
    }

    /// The two channels the switches gate; creating an existing channel is a no-op on Android.
    pub fn create_channels<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        let n = app.notification();
        n.create_channel(Channel::builder(crate::alerts::Kind::Decision.channel_id(), "Decisions").description("A session needs a decision").importance(Importance::High).vibration(true).build()).map_err(|e| e.to_string())?;
        n.create_channel(Channel::builder(crate::alerts::Kind::Finished.channel_id(), "Finished").description("A session finished its turn").importance(Importance::Default).build()).map_err(|e| e.to_string())
    }

    struct AndroidAlerts<R: Runtime> {
        app: AppHandle<R>,
    }

    impl<R: Runtime> Alerts for AndroidAlerts<R> {
        fn post(&self, p: &Post) {
            let shown = self.app.notification().builder().id(p.id).channel_id(p.kind.channel_id()).title(&p.title).body(&p.body).extra("sessionId", &p.session_id).auto_cancel().show();
            if let Err(e) = shown {
                log::line("android", format!("notification: {e}"));
            }
        }

        fn clear(&self, id: i32) {
            if let Err(e) = self.app.notification().remove_active(vec![id]) {
                log::line("android", format!("clear notification {id}: {e}"));
            }
        }

        fn service_line(&self, line: &str) {
            service_update(&self.app, line)
        }
    }

    pub fn alerts<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn Alerts> {
        Arc::new(AndroidAlerts { app: app.clone() })
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;

    pub fn service_start<R: Runtime>(_: &AppHandle<R>, line: &str) {
        log::line("android", format!("service start: {line}"))
    }

    pub fn service_update<R: Runtime>(_: &AppHandle<R>, line: &str) {
        log::line("android", format!("service: {line}"))
    }

    pub fn service_stop<R: Runtime>(_: &AppHandle<R>) {
        log::line("android", "service stop")
    }

    pub fn request_battery_exemption<R: Runtime>(_: &AppHandle<R>) -> Result<(), String> {
        Ok(())
    }

    pub fn notifications_allowed<R: Runtime>(_: &AppHandle<R>) -> bool {
        true
    }

    pub fn device_model<R: Runtime>(_: &AppHandle<R>) -> String {
        String::new()
    }

    pub fn local_addresses<R: Runtime>(_: &AppHandle<R>) -> Vec<String> {
        vec![]
    }

    pub fn create_channels<R: Runtime>(_: &AppHandle<R>) -> Result<(), String> {
        Ok(())
    }

    struct HostAlerts;

    impl Alerts for HostAlerts {
        fn post(&self, p: &Post) {
            log::line("android", format!("would notify: {} — {}", p.title, p.body))
        }

        fn clear(&self, id: i32) {
            log::line("android", format!("would clear notification {id}"))
        }

        fn service_line(&self, line: &str) {
            log::line("android", format!("service: {line}"))
        }
    }

    pub fn alerts<R: Runtime>(_: &AppHandle<R>) -> Arc<dyn Alerts> {
        Arc::new(HostAlerts)
    }
}

pub use imp::*;
```

`mobile/src/lib.rs`, for now:

```rust
//! Maya for Android: a main Maya with no sessions of its own. The server,
//! pairing, merging and routing are `maya_core`'s; this crate is the Tauri
//! commands the page calls, each routed to an assistant, and the Android
//! glue: the foreground service and the notifications.

pub mod alerts;
pub mod android;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(android::init())
        .run(tauri::generate_context!())
        .expect("error while running Maya");
}
```

Run: `cargo test -p maya-mobile`
Expected: FAIL to compile, "cannot find function `plan`" and friends (the tests name what does not exist yet).

- [ ] **Step 5: Write the planner**

Above the tests in `mobile/src/alerts.rs`:

```rust
use maya_core::model::{Card, State};
use maya_core::notify::body_for;
use std::collections::HashMap;

/// Which channel a notification goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Decision,
    Finished,
}

impl Kind {
    pub fn channel_id(self) -> &'static str {
        match self {
            Kind::Decision => "decisions",
            Kind::Finished => "finished",
        }
    }
}

/// One notification to show. `id` is stable per session, so a newer one
/// replaces the older in place; `session_id` rides along so a tap opens
/// the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post {
    pub id: i32,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Post(Post),
    Clear(i32),
}

/// The two Network-screen switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Switches {
    pub awaiting: bool,
    pub completed: bool,
}

/// The notifications on screen, by session id: which kind each is.
#[derive(Default, Debug)]
pub struct Posted(HashMap<String, Kind>);

/// What shows and clears notifications and keeps the service's line: the
/// Android side, or a logger on the desktop.
pub trait Alerts: Send + Sync {
    fn post(&self, post: &Post);
    fn clear(&self, id: i32);
    /// The foreground service's text ("Main for 2 assistants, 1 connected").
    fn service_line(&self, line: &str);
}

/// FNV-1a over the session id, folded to a positive i32 and kept clear of
/// 1, the service's own id. The same id across runs, so a notification left
/// from an earlier run is replaced or cleared rather than doubled.
pub fn notification_id(session_id: &str) -> i32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in session_id.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    ((h & 0x7fff_ffff) as i32).max(2)
}

/// "hexgrid on maya-mini needs a decision" / "… is finished"; the machine is
/// the label `merge::merged` set, so two assistants of one name read apart.
pub fn title_for(card: &Card) -> String {
    let who = match &card.machine {
        Some(m) => format!("{} on {m}", card.name),
        None => card.name.clone(),
    };
    match card.state {
        State::Awaiting => format!("{who} needs a decision"),
        State::Completed => format!("{who} is finished"),
        _ => who,
    }
}

/// The last assistant text, or a stock line when there is none.
pub fn finished_body(card: &Card) -> String {
    let s = card.snippet.trim();
    if s.is_empty() {
        "Finished its turn".into()
    } else {
        s.chars().take(200).collect()
    }
}

/// The notifications to post and clear for this board. Clears come first,
/// for every posted session whose card moved on or left the board (a
/// decision taken at the desk); then one post per new ask and per finished
/// turn, each behind its switch.
pub fn plan(cards: &[Card], fresh: &[Card], finished: &[Card], posted: &mut Posted, switches: Switches) -> Vec<Action> {
    let mut out = vec![];
    let now: HashMap<&str, State> = cards.iter().map(|c| (c.session_id.as_str(), c.state)).collect();
    posted.0.retain(|id, kind| {
        let still = matches!((now.get(id.as_str()), *kind), (Some(State::Awaiting), Kind::Decision) | (Some(State::Completed), Kind::Finished));
        if !still {
            out.push(Action::Clear(notification_id(id)));
        }
        still
    });
    if switches.awaiting {
        for c in fresh {
            push(&mut out, posted, c, Kind::Decision, body_for(c));
        }
    }
    if switches.completed {
        for c in finished {
            push(&mut out, posted, c, Kind::Finished, finished_body(c));
        }
    }
    out
}

fn push(out: &mut Vec<Action>, posted: &mut Posted, c: &Card, kind: Kind, body: String) {
    posted.0.insert(c.session_id.clone(), kind);
    out.push(Action::Post(Post { id: notification_id(&c.session_id), kind, title: title_for(c), body, session_id: c.session_id.clone() }));
}

/// The foreground service's line.
pub fn service_line(paired: usize, connected: usize) -> String {
    match (paired, connected) {
        (0, _) => "No assistants paired yet".into(),
        (1, 1) => "Main for 1 assistant, connected".into(),
        (1, _) => "Main for 1 assistant, not connected".into(),
        (n, c) => format!("Main for {n} assistants, {c} connected"),
    }
}

/// The phone's name as assistants see it: the device model, or "Android" when that is blank.
pub fn default_name(model: &str) -> String {
    let m = model.trim();
    if m.is_empty() {
        "Android".to_string()
    } else {
        m.to_string()
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p maya-mobile`
Expected: 8 tests PASS. Also `cargo build -p maya-mobile` (the binary) succeeds.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore mobile scripts/set-version.sh scripts/release-version.sh
git commit -m "feat(mobile): the Android crate's skeleton and its alert planner"
```

---

### Task 4: The hub, the Tauri commands, and the localhost round trip

Everything the phone does that is not Android glue lives in `Hub`: the config, the server handle, the notifier and the alert state, with the page and the service behind a `Sink` trait. `lib.rs` wires the hub to Tauri; `commands.rs` is the command surface the page calls, every name and argument shape the desktop's. An integration test drives a real server and a real core client on localhost.

**Files:**
- Create: `mobile/src/hub.rs`, `mobile/src/commands.rs`, `mobile/tests/round_trip.rs`
- Modify: `mobile/src/lib.rs`

**Interfaces:**
- Consumes: Task 2's `maya_core::net::routing::*`; Task 3's `alerts` and `android`.
- Produces (`mobile/src/hub.rs`):
  - `pub trait Sink: Send + Sync { fn sessions(&self, cards: &[Card]); fn network(&self, status: &NetworkStatus); fn service_start(&self, line: &str); fn service_stop(&self); }`
  - `pub struct Settings { pub path: PathBuf, pub config: Config }`
  - `pub struct Hub { pub maya_dir: PathBuf, pub settings: Mutex<Settings>, pub server: Mutex<Option<ServerHandle>>, pub main_error: Mutex<Option<String>>, pub notifier: Mutex<Notifier>, pub posted: Mutex<Posted>, pub alerts: Arc<dyn Alerts>, pub sink: Arc<dyn Sink> }`
  - `pub fn settings_path(maya_dir: &Path) -> PathBuf`, `pub fn load_settings(maya_dir: &Path, model: &str) -> Settings`
  - `impl Hub`: `new(maya_dir, settings, alerts, sink) -> Arc<Hub>`, `boards(&self) -> Vec<RemoteBoard>`, `cards(&self) -> Vec<Card>`, `refresh(&self)`, `status(&self) -> NetworkStatus`, `update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config, String>`, `start_server(self: &Arc<Self>) -> Result<(), String>`, `stop_server(&self)`, `pairing_code(&self) -> Result<NetworkStatus, String>`, `remove_assistant(&self, id: &str) -> Result<NetworkStatus, String>`, `send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String>`, `send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String>`
- Produces (`mobile/src/commands.rs`): Tauri commands `list_sessions`, `session_history`, `send_reply`, `answer_question`, `set_session_option`, `cycle_session_mode`, `send_slash_command`, `rename_session`, `compact_session`, `close_session`, `save_attachment`, `open_url`, `open_pr`, `list_machines`, `list_project_dirs`, `list_agents`, `list_resumable_sessions`, `resume_session`, `start_session`, `network_status`, `network_pairing_code`, `network_remove_assistant`, `get_config`, `set_config`, `log_lines`, `log_clear`, `log_path`, `server_start`, `server_stop`, `local_addresses`, `request_battery_exemption`, `notifications_allowed`.
- Tauri state: `tauri::State<Arc<Hub>>`. Events: `sessions` (`Vec<Card>`), `network` (`NetworkStatus`), `log` (`log::Line`).

- [ ] **Step 1: Write the failing integration test**

`mobile/tests/round_trip.rs`:

```rust
//! The phone's hub against a real core client on localhost: pairing, a
//! board, a command round trip, and which boards post notifications.

use maya_core::config::{Config, NetworkConfig, NetworkRole};
use maya_core::model::{AwaitKind, Awaiting, Card, Harness, State};
use maya_core::net::client::{pair_with, run_once, ClientNotify, Executor};
use maya_core::net::protocol::CommandKind;
use maya_core::net::NetworkStatus;
use maya_mobile_lib::alerts::{Alerts, Post};
use maya_mobile_lib::hub::{Hub, Settings, Sink};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Recording {
    posts: Mutex<Vec<Post>>,
    cleared: Mutex<Vec<i32>>,
    service: Mutex<Vec<String>>,
}

impl Alerts for Recording {
    fn post(&self, post: &Post) {
        self.posts.lock().unwrap().push(post.clone());
    }
    fn clear(&self, id: i32) {
        self.cleared.lock().unwrap().push(id);
    }
    fn service_line(&self, line: &str) {
        self.service.lock().unwrap().push(line.into());
    }
}

#[derive(Default)]
struct Page {
    boards: Mutex<Vec<Vec<Card>>>,
    statuses: Mutex<Vec<NetworkStatus>>,
    service: Mutex<Vec<String>>,
}

impl Sink for Page {
    fn sessions(&self, cards: &[Card]) {
        self.boards.lock().unwrap().push(cards.to_vec());
    }
    fn network(&self, status: &NetworkStatus) {
        self.statuses.lock().unwrap().push(status.clone());
    }
    fn service_start(&self, line: &str) {
        self.service.lock().unwrap().push(format!("start: {line}"));
    }
    fn service_stop(&self) {
        self.service.lock().unwrap().push("stop".into());
    }
}

/// A pretend assistant: one board it can swap, and a log of the commands it ran.
struct FakeAssistant {
    cards: Mutex<Vec<Card>>,
    due: AtomicBool,
    ran: Mutex<Vec<CommandKind>>,
}

impl Executor for FakeAssistant {
    fn execute(&self, kind: CommandKind) -> Result<Option<Value>, String> {
        self.ran.lock().unwrap().push(kind);
        Ok(None)
    }
    fn board(&self) -> (Vec<Card>, Vec<String>) {
        (self.cards.lock().unwrap().clone(), vec!["hexgrid".into(), "maya".into()])
    }
    fn board_requested(&self) -> bool {
        self.due.swap(false, Ordering::SeqCst)
    }
}

struct Quiet;
impl ClientNotify for Quiet {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, _: &str) {}
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
}

fn card(id: &str, state: State, since: u64) -> Card {
    Card {
        session_id: id.into(),
        pid: 42,
        name: id.into(),
        cwd: "/home/u/dev/hexgrid".into(),
        state,
        state_since: since,
        snippet: "working on it".into(),
        awaiting: (state == State::Awaiting).then(|| Awaiting { kind: AwaitKind::Permission, detail: "Run the tests?".into(), questions: vec![] }),
        has_inbox: true,
        harness: Harness::ClaudeCode,
        pr: None,
        context: None,
        machine: None,
        machine_address: None,
        machine_platform: None,
        terminal: None,
        stale: false,
        model: None,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
}

#[test]
fn pairs_merges_a_board_routes_a_reply_and_notifies_only_what_is_new() {
    let dir = tempfile::tempdir().unwrap();
    let maya_dir = dir.path().join("maya");
    let port = free_port();
    let mut config = Config::default();
    config.network.port = port;
    config.network.name = "Pixel".into();
    let settings = Settings { path: maya_dir.join("config.json"), config };
    let alerts = Arc::new(Recording::default());
    let page = Arc::new(Page::default());
    let hub = Hub::new(maya_dir.clone(), settings, alerts.clone(), page.clone());

    hub.start_server().unwrap();
    assert_eq!(page.service.lock().unwrap().as_slice(), ["start: No assistants paired yet"]);
    let status = hub.pairing_code().unwrap();
    let code = status.code.expect("a pairing code").code;

    // The assistant pairs with its first board already holding an ask: not news.
    let assistant = Arc::new(FakeAssistant { cards: Mutex::new(vec![card("s1", State::Awaiting, 1_000)]), due: AtomicBool::new(false), ran: Mutex::new(vec![]) });
    let (main_name, id, token) = pair_with(assistant.clone(), "127.0.0.1", port, "laptop", &code).unwrap();
    assert_eq!(main_name, "Pixel");
    let saved = maya_core::config::load(&maya_dir.join("config.json"));
    assert_eq!(saved.network.assistants.len(), 1, "the pairing was saved");
    assert_eq!(saved.network.assistants[0].id, id);

    let link = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), assistant_id: id, token, ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let client = {
        let (assistant, stop) = (assistant.clone(), stop.clone());
        std::thread::spawn(move || run_once(&link, assistant, Arc::new(Quiet), &stop, None))
    };

    wait_until("the first board", || hub.cards().iter().any(|c| c.session_id == "s1"));
    let cards = hub.cards();
    assert_eq!(cards[0].machine.as_deref(), Some("laptop"), "merged cards carry the label");
    assert!(alerts.posts.lock().unwrap().is_empty(), "a first board after pairing is seeded, not announced");
    wait_until("the service line", || page.service.lock().unwrap().iter().any(|l| l.contains("1 assistant")) || alerts.service.lock().unwrap().iter().any(|l| l.contains("connected")));

    // A reply from the phone reaches the assistant through the server.
    hub.send("s1", |s| CommandKind::Reply { session: s, text: "go ahead".into(), attachments: vec![] }).unwrap();
    wait_until("the reply", || !assistant.ran.lock().unwrap().is_empty());
    assert!(matches!(&assistant.ran.lock().unwrap()[0], CommandKind::Reply { session, text, .. } if session == "s1" && text == "go ahead"));
    assert_eq!(hub.send("nope", |s| CommandKind::Compact { session: s }).unwrap_err(), "Session is no longer running.");

    // A later board with a new ask posts exactly one notification; s1 moving on clears its (never posted) slot silently.
    *assistant.cards.lock().unwrap() = vec![card("s1", State::Working, 2_000), card("s2", State::Awaiting, 2_000)];
    assistant.due.store(true, Ordering::SeqCst);
    wait_until("the decision notification", || !alerts.posts.lock().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    let posts = alerts.posts.lock().unwrap().clone();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "s2 on laptop needs a decision");
    assert_eq!(posts[0].body, "Run the tests?");
    assert!(alerts.cleared.lock().unwrap().is_empty());

    // Answered at the desk: the notification goes.
    *assistant.cards.lock().unwrap() = vec![card("s2", State::Working, 3_000)];
    assistant.due.store(true, Ordering::SeqCst);
    wait_until("the clear", || !alerts.cleared.lock().unwrap().is_empty());
    assert_eq!(alerts.cleared.lock().unwrap()[0], posts[0].id);

    hub.stop_server();
    stop.store(true, Ordering::SeqCst);
    let _ = client.join();
    assert!(page.service.lock().unwrap().iter().any(|l| l == "stop"));
    assert_eq!(hub.status().role, NetworkRole::Off);
}

#[test]
fn a_taken_port_is_reported_and_starts_no_service() {
    let dir = tempfile::tempdir().unwrap();
    let holder = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let port = holder.local_addr().unwrap().port();
    let mut config = Config::default();
    config.network.port = port;
    let settings = Settings { path: dir.path().join("config.json"), config };
    let page = Arc::new(Page::default());
    let hub = Hub::new(dir.path().to_path_buf(), settings, Arc::new(Recording::default()), page.clone());
    let err = hub.start_server().unwrap_err();
    assert!(err.contains(&format!("port {port}")), "{err}");
    assert_eq!(hub.status().main_error.as_deref(), Some(err.as_str()));
    assert!(page.service.lock().unwrap().is_empty(), "no foreground service for a server that is not running");
    assert_eq!(hub.pairing_code().unwrap_err(), err);
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p maya-mobile --test round_trip`
Expected: FAIL to compile, "could not find `hub` in `maya_mobile_lib`".

- [ ] **Step 3: Write `hub.rs`**

```rust
//! The phone's main, with the page and the Android service behind traits:
//! the config, the server, the notifier diff and the alert state. The Tauri
//! layer (`lib.rs`, `commands.rs`) is thin on purpose, so this is what the
//! integration test drives.

use crate::alerts::{self, Action, Alerts, Posted, Switches};
use maya_core::config::{self, Config, NetworkRole, PairedAssistant};
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::merge::{self, RemoteBoard};
use maya_core::net::protocol::CommandKind;
use maya_core::net::routing::ROUTE_TIMEOUT;
use maya_core::net::server::{send_command_with, start_with, Notify, ServerHandle};
use maya_core::net::{self, NetworkStatus};
use maya_core::notify::Notifier;
use maya_core::store::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// What the hub tells the outside: the page's events and the service.
pub trait Sink: Send + Sync {
    fn sessions(&self, cards: &[Card]);
    fn network(&self, status: &NetworkStatus);
    fn service_start(&self, line: &str);
    fn service_stop(&self);
}

/// The config and where it is saved.
pub struct Settings {
    pub path: PathBuf,
    pub config: Config,
}

/// Where the config lives under the app's data directory.
pub fn settings_path(maya_dir: &Path) -> PathBuf {
    maya_dir.join("config.json")
}

/// Loads the config, filling a blank name with the device's so the main's
/// `welcome` never says "localhost".
pub fn load_settings(maya_dir: &Path, model: &str) -> Settings {
    let path = settings_path(maya_dir);
    let mut config = config::load(&path);
    if config.network.name.trim().is_empty() {
        config.network.name = alerts::default_name(model);
    }
    Settings { path, config }
}

pub struct Hub {
    /// `<app data dir>/maya`: the config, the log and saved attachments.
    pub maya_dir: PathBuf,
    /// Lock order: `server`, then the server's own mutex, then `settings`;
    /// `notifier` and `posted` are taken alone.
    pub settings: Mutex<Settings>,
    pub server: Mutex<Option<ServerHandle>>,
    /// Why the server is not running (the port is taken…); `None` once it runs.
    pub main_error: Mutex<Option<String>>,
    pub notifier: Mutex<Notifier>,
    pub posted: Mutex<Posted>,
    pub alerts: Arc<dyn Alerts>,
    pub sink: Arc<dyn Sink>,
}

impl Hub {
    pub fn new(maya_dir: PathBuf, settings: Settings, alerts: Arc<dyn Alerts>, sink: Arc<dyn Sink>) -> Arc<Hub> {
        Arc::new(Hub { maya_dir, settings: Mutex::new(settings), server: Mutex::new(None), main_error: Mutex::new(None), notifier: Mutex::new(Notifier::default()), posted: Mutex::new(Posted::default()), alerts, sink })
    }

    fn handle(&self) -> Option<ServerHandle> {
        self.server.lock().unwrap().clone()
    }

    /// The assistants' last snapshots; none while the server is down.
    pub fn boards(&self) -> Vec<RemoteBoard> {
        self.handle().map(|s| s.boards()).unwrap_or_default()
    }

    /// The board: every connected assistant's cards, no local ones.
    pub fn cards(&self) -> Vec<Card> {
        merge::merged(vec![], &self.boards(), now_ms())
    }

    /// Repaints the page and posts or clears notifications for what changed.
    pub fn refresh(&self) {
        let cards = self.cards();
        let (fresh, finished) = {
            let mut n = self.notifier.lock().unwrap();
            (n.take_new(&cards), n.take_finished(&cards))
        };
        let switches = {
            let s = self.settings.lock().unwrap();
            Switches { awaiting: s.config.notify_on_awaiting, completed: s.config.notify_on_completed }
        };
        let actions = alerts::plan(&cards, &fresh, &finished, &mut self.posted.lock().unwrap(), switches);
        for a in actions {
            match a {
                Action::Post(p) => self.alerts.post(&p),
                Action::Clear(id) => self.alerts.clear(id),
            }
        }
        self.sink.sessions(&cards);
    }

    /// The status as the Network screen shows it: the server's, or the
    /// paired list offline with why the server is down.
    pub fn status(&self) -> NetworkStatus {
        match self.handle() {
            Some(s) => s.status(),
            None => {
                let paired = self.settings.lock().unwrap().config.network.assistants.clone();
                NetworkStatus { role: NetworkRole::Off, code: None, assistants: net::paired_offline(&paired), main_error: self.main_error.lock().unwrap().clone(), ..Default::default() }
            }
        }
    }

    /// Changes the config with `f` and saves it; the config in memory
    /// changes only once the save succeeded.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
        let mut s = self.settings.lock().unwrap();
        let mut next = s.config.clone();
        f(&mut next);
        config::save(&s.path, &next).map_err(|e| format!("Could not save the settings: {e}"))?;
        s.config = next.clone();
        Ok(next)
    }

    fn save_assistants(&self, f: impl FnOnce(&mut Vec<PairedAssistant>)) -> Result<(), String> {
        self.update_config(|c| f(&mut c.network.assistants)).map(|_| ()).map_err(|e| {
            log::line("network", format!("could not save the paired assistants: {e}"));
            format!("Could not save the paired assistants: {e}")
        })
    }

    /// Starts the server on the configured port, replacing a running one,
    /// and the foreground service with it. When the port cannot be bound
    /// the error is kept for the Network screen and no service starts.
    pub fn start_server(self: &Arc<Self>) -> Result<(), String> {
        self.stop_server();
        let (port, view) = {
            let s = self.settings.lock().unwrap();
            (s.config.listen_port(), s.config.network.clone())
        };
        let notify: Arc<dyn Notify> = Arc::new(HubNotify(Arc::downgrade(self)));
        let mut started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port);
        for _ in 0..5 {
            if started.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
            started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port);
        }
        match started {
            Ok(handle) => {
                let status = handle.status();
                *self.server.lock().unwrap() = Some(handle);
                *self.main_error.lock().unwrap() = None;
                log::line("network", format!("server listening on port {port}"));
                self.sink.service_start(&alerts::service_line(status.assistants.len(), 0));
                self.sink.network(&status);
                Ok(())
            }
            Err(e) => {
                *self.main_error.lock().unwrap() = Some(e.clone());
                log::line("network", format!("server not started: {e}"));
                self.sink.network(&self.status());
                Err(e)
            }
        }
    }

    /// Stops the server and the service; the board empties.
    pub fn stop_server(&self) {
        let running = self.server.lock().unwrap().take();
        if let Some(s) = running {
            s.stop();
            self.sink.service_stop();
        }
        self.refresh();
        self.sink.network(&self.status());
    }

    /// Opens pairing, or regenerates the code when it is already open.
    pub fn pairing_code(&self) -> Result<NetworkStatus, String> {
        let Some(server) = self.handle() else {
            return Err(self.main_error.lock().unwrap().clone().unwrap_or_else(|| "Start the server first.".into()));
        };
        server.open_pairing(now_ms());
        Ok(server.status())
    }

    /// Forgets a paired assistant; a connected one is told and closed.
    pub fn remove_assistant(&self, id: &str) -> Result<NetworkStatus, String> {
        match self.handle() {
            Some(s) => s.remove_assistant(id)?,
            None => {
                self.save_assistants(|list| list.retain(|a| a.id != id))?;
                log::line("network", format!("{id}: removed"));
            }
        }
        Ok(self.status())
    }

    /// Sends a session command to the machine that lists the session. There
    /// are no local sessions: a session nobody lists is no longer running.
    pub fn send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String> {
        let machine = merge::route(&[], &self.boards(), session_id, now_ms()).ok_or("Session is no longer running.")?;
        self.send_to(&machine, kind(session_id.to_string()))
    }

    /// Sends a command to a machine by its label (start, resume, listings).
    pub fn send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String> {
        let shared = self.handle().map(|s| s.shared.clone()).ok_or("The server is stopped.")?;
        send_command_with(&shared, machine, kind, ROUTE_TIMEOUT)
    }
}

/// The server's line to the hub. `paired` and `paired_list_changed` run
/// with the server's mutex held and take only `settings`; the others run
/// on the server's threads with no lock held.
struct HubNotify(Weak<Hub>);

impl HubNotify {
    fn with(&self, f: impl FnOnce(&Hub)) {
        if let Some(hub) = self.0.upgrade() {
            f(&hub);
        }
    }
}

impl Notify for HubNotify {
    fn board_changed(&self) {
        self.with(|h| h.refresh());
    }

    fn board_seeded(&self, cards: &[Card]) {
        self.with(|h| h.notifier.lock().unwrap().seed(cards));
    }

    fn status_changed(&self, status: NetworkStatus) {
        self.with(|h| {
            let connected = status.assistants.iter().filter(|a| a.connected).count();
            h.alerts.service_line(&alerts::service_line(status.assistants.len(), connected));
            h.sink.network(&status);
        });
    }

    fn paired(&self, assistant: &PairedAssistant) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(|list| match list.iter_mut().find(|a| a.id == assistant.id) {
            Some(a) => *a = assistant.clone(),
            None => list.push(assistant.clone()),
        })
    }

    fn paired_list_changed(&self, assistants: &[PairedAssistant]) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(|list| *list = assistants.to_vec())
    }
}
```

Add `pub mod hub;` to `lib.rs`.

- [ ] **Step 4: Run the integration test**

Run: `cargo test -p maya-mobile --test round_trip`
Expected: both tests PASS. If `pairs_merges_…` times out at "the first board", check the service line assertion first: `start_server` reports `status.assistants.len()` from the fresh server, which is 0 before pairing.

- [ ] **Step 5: Write `commands.rs`**

```rust
//! The commands the page calls, by the names the desktop's page uses, so
//! the board, card, modal and dialogs run unchanged. Every session command
//! goes to the assistant that lists the session; nothing runs here.

use crate::hub::Hub;
use crate::android;
use maya_core::actions::StartResult;
use maya_core::agents::AgentInfo;
use maya_core::config::{Config, NetworkRole};
use maya_core::launch::LaunchOptions;
use maya_core::model::{Card, Harness};
use maya_core::net::merge;
use maya_core::net::protocol::CommandKind;
use maya_core::net::routing::{agents_reply, check_remote_agent, machines_of, remote_attachments, AgentsReply, MachineInfo};
use maya_core::net::NetworkStatus;
use maya_core::resume::ResumableSession;
use maya_core::store::now_ms;
use maya_core::transcript::Turn;
use maya_core::{attachments, log};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

type H<'a> = State<'a, Arc<Hub>>;

fn data<T: DeserializeOwned>(value: Option<Value>) -> Result<T, String> {
    serde_json::from_value(value.ok_or("The assistant sent no result.")?).map_err(|e| e.to_string())
}

/// The pickers always name a machine on the phone; a blank one is a page bug.
fn machine_arg(machine: Option<String>) -> Result<String, String> {
    machine.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).ok_or_else(|| "Choose a machine first.".into())
}

#[tauri::command]
pub fn list_sessions(hub: H) -> Vec<Card> {
    hub.cards()
}

#[tauri::command(async)]
pub fn session_history(hub: H, session_id: String) -> Result<Vec<Turn>, String> {
    data(hub.send(&session_id, |session| CommandKind::History { session })?)
}

#[tauri::command(async)]
pub fn send_reply(hub: H, session_id: String, text: String, attachments: Vec<String>) -> Result<(), String> {
    let attachments = remote_attachments(&attachments)?;
    hub.send(&session_id, |session| CommandKind::Reply { session, text, attachments }).map(|_| ())
}

#[tauri::command(async)]
pub fn answer_question(hub: H, session_id: String, ask_id: u64, question_index: usize, option_index: usize) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Answer { session, ask_id, question: question_index, option: option_index }).map(|_| ())
}

#[tauri::command(async)]
pub fn set_session_option(hub: H, session_id: String, setting: String, value: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::SetOption { session, setting, value }).map(|_| ())
}

#[tauri::command(async)]
pub fn cycle_session_mode(hub: H, session_id: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::CycleMode { session }).map(|_| ())
}

#[tauri::command(async)]
pub fn send_slash_command(hub: H, session_id: String, text: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Slash { session, text }).map(|_| ())
}

#[tauri::command(async)]
pub fn rename_session(hub: H, session_id: String, name: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Rename { session, name }).map(|_| ())
}

#[tauri::command(async)]
pub fn compact_session(hub: H, session_id: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Compact { session }).map(|_| ())
}

#[tauri::command(async)]
pub fn close_session(hub: H, session_id: String) -> Result<(), String> {
    hub.send(&session_id, |session| CommandKind::Close { session }).map(|_| ())
}

/// Saves a picked file under the app's data so a reply can carry it.
#[tauri::command(async)]
pub fn save_attachment(hub: H, name: String, bytes: Vec<u8>) -> Result<String, String> {
    let path = attachments::save(&hub.maya_dir, &name, &bytes, now_ms())?;
    Ok(path.to_string_lossy().into_owned())
}

fn open_web(app: &AppHandle, url: &str) -> Result<(), String> {
    if !attachments::is_web_url(url) {
        return Err("Only web links can be opened.".into());
    }
    app.opener().open_url(url.trim(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    open_web(&app, &url)
}

/// The session's pull request, from the assistant's card, never from the page.
#[tauri::command(async)]
pub fn open_pr(app: AppHandle, hub: H, session_id: String) -> Result<(), String> {
    let card = hub.cards().into_iter().find(|c| c.session_id == session_id).ok_or("Session is no longer running.")?;
    let pr = card.pr.ok_or("No pull request is known for this session yet.")?;
    open_web(&app, &pr.url)
}

#[tauri::command]
pub fn list_machines(hub: H) -> Vec<MachineInfo> {
    machines_of(&hub.status())
}

#[tauri::command]
pub fn list_project_dirs(hub: H, machine: Option<String>) -> Result<Vec<String>, String> {
    let machine = machine_arg(machine)?;
    Ok(merge::dirs_of(&hub.boards(), &machine))
}

#[tauri::command]
pub fn list_agents(hub: H, machine: Option<String>) -> AgentsReply {
    let remote: Option<Vec<AgentInfo>> = machine.as_deref().filter(|m| !m.is_empty()).and_then(|m| merge::agents_of(&hub.boards(), m));
    agents_reply(false, remote, Vec::new)
}

#[tauri::command(async)]
pub fn list_resumable_sessions(hub: H, dir: String, machine: Option<String>, agent: Option<Harness>) -> Result<Vec<ResumableSession>, String> {
    let machine = machine_arg(machine)?;
    let agent = agent.unwrap_or_default();
    check_remote_agent(agent, &merge::agents_of(&hub.boards(), &machine), &machine)?;
    data(hub.send_to(&machine, CommandKind::ListResumable { dir, agent })?)
}

#[tauri::command(async)]
pub fn resume_session(hub: H, dir: String, session_id: String, machine: Option<String>, agent: Option<Harness>) -> Result<(), String> {
    let machine = machine_arg(machine)?;
    let agent = agent.unwrap_or_default();
    check_remote_agent(agent, &merge::agents_of(&hub.boards(), &machine), &machine)?;
    hub.send_to(&machine, CommandKind::Resume { dir, session: session_id, agent }).map(|_| ())
}

#[tauri::command(async)]
pub fn start_session(hub: H, dir: Option<String>, prompt: String, options: LaunchOptions, machine: Option<String>) -> Result<StartResult, String> {
    let machine = machine_arg(machine)?;
    if prompt.trim().is_empty() {
        return Err("Type a prompt first.".into());
    }
    options.validate_shape()?;
    check_remote_agent(options.agent, &merge::agents_of(&hub.boards(), &machine), &machine)?;
    data(hub.send_to(&machine, CommandKind::Start { dir, prompt, options })?)
}

#[tauri::command]
pub fn network_status(hub: H) -> NetworkStatus {
    hub.status()
}

#[tauri::command]
pub fn network_pairing_code(hub: H) -> Result<NetworkStatus, String> {
    hub.pairing_code()
}

#[tauri::command(async)]
pub fn network_remove_assistant(hub: H, id: String) -> Result<NetworkStatus, String> {
    hub.remove_assistant(&id)
}

#[tauri::command]
pub fn get_config(hub: H) -> Config {
    hub.settings.lock().unwrap().config.clone()
}

/// Saves what the Network screen edits: name, port and the two switches.
/// The role and the paired list stay the server's. A new name or port on a
/// running server restarts it; assistants reconnect within seconds.
#[tauri::command(async)]
pub fn set_config(hub: H, config: Config) -> Result<Config, String> {
    let before = hub.settings.lock().unwrap().config.clone();
    let saved = hub.update_config(|c| {
        if !config.network.name.trim().is_empty() {
            c.network.name = config.network.name.trim().to_string();
        }
        c.network.port = config.network.port;
        c.notify_on_awaiting = config.notify_on_awaiting;
        c.notify_on_completed = config.notify_on_completed;
    })?;
    let running = hub.server.lock().unwrap().is_some();
    if running && (before.listen_port() != saved.listen_port() || before.network.name != saved.network.name) {
        log::line("network", "settings changed; restarting the server");
        hub.start_server()?;
    }
    Ok(saved)
}

#[tauri::command]
pub fn log_lines() -> Vec<log::Line> {
    log::lines()
}

#[tauri::command]
pub fn log_clear() {
    log::clear()
}

#[tauri::command]
pub fn log_path(hub: H) -> String {
    hub.maya_dir.join("maya.log").to_string_lossy().into_owned()
}

/// Starts the server, remembers that it should run, and opens pairing.
#[tauri::command(async)]
pub fn server_start(hub: H) -> Result<NetworkStatus, String> {
    hub.update_config(|c| c.network.role = NetworkRole::Main)?;
    hub.start_server()?;
    hub.pairing_code()
}

#[tauri::command(async)]
pub fn server_stop(hub: H) -> Result<NetworkStatus, String> {
    hub.stop_server();
    hub.update_config(|c| c.network.role = NetworkRole::Off)?;
    Ok(hub.status())
}

#[tauri::command]
pub fn local_addresses(app: AppHandle) -> Vec<String> {
    android::local_addresses(&app)
}

#[tauri::command(async)]
pub fn request_battery_exemption(app: AppHandle) -> Result<(), String> {
    android::request_battery_exemption(&app)
}

#[tauri::command]
pub fn notifications_allowed(app: AppHandle) -> bool {
    android::notifications_allowed(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_must_be_named() {
        assert_eq!(machine_arg(None).unwrap_err(), "Choose a machine first.");
        assert_eq!(machine_arg(Some("  ".into())).unwrap_err(), "Choose a machine first.");
        assert_eq!(machine_arg(Some(" laptop ".into())).unwrap(), "laptop");
    }

    #[test]
    fn missing_data_is_an_error_not_a_panic() {
        assert_eq!(data::<Vec<Turn>>(None).unwrap_err(), "The assistant sent no result.");
        assert!(data::<Vec<Turn>>(Some(serde_json::json!([]))).unwrap().is_empty());
    }

}
```

- [ ] **Step 6: Wire `lib.rs`**

Replace `mobile/src/lib.rs` with:

```rust
//! Maya for Android: a main Maya with no sessions of its own. The server,
//! pairing, merging and routing are `maya_core`'s; this crate is the Tauri
//! commands the page calls, each routed to an assistant, and the Android
//! glue: the foreground service and the notifications.

pub mod alerts;
pub mod android;
mod commands;
pub mod hub;

use hub::{Hub, Sink};
use maya_core::config::NetworkRole;
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::NetworkStatus;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// The hub's way out: events to the page, and the foreground service.
struct TauriSink {
    app: AppHandle,
}

impl Sink for TauriSink {
    fn sessions(&self, cards: &[Card]) {
        let _ = self.app.emit("sessions", cards);
    }

    fn network(&self, status: &NetworkStatus) {
        let _ = self.app.emit("network", status);
    }

    fn service_start(&self, line: &str) {
        android::service_start(&self.app, line);
    }

    fn service_stop(&self) {
        android::service_stop(&self.app);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(android::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_sessions,
            commands::session_history,
            commands::send_reply,
            commands::answer_question,
            commands::set_session_option,
            commands::cycle_session_mode,
            commands::send_slash_command,
            commands::rename_session,
            commands::compact_session,
            commands::close_session,
            commands::save_attachment,
            commands::open_url,
            commands::open_pr,
            commands::list_machines,
            commands::list_project_dirs,
            commands::list_agents,
            commands::list_resumable_sessions,
            commands::resume_session,
            commands::start_session,
            commands::network_status,
            commands::network_pairing_code,
            commands::network_remove_assistant,
            commands::get_config,
            commands::set_config,
            commands::log_lines,
            commands::log_clear,
            commands::log_path,
            commands::server_start,
            commands::server_stop,
            commands::local_addresses,
            commands::request_battery_exemption,
            commands::notifications_allowed
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let maya_dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("maya");
            if let Err(e) = log::init(&maya_dir.join("maya.log")) {
                eprintln!("{e}");
            }
            let emitter = handle.clone();
            log::install_emitter(move |line| {
                let _ = emitter.emit("log", line);
            });
            let settings = hub::load_settings(&maya_dir, &android::device_model(&handle));
            let hub = Hub::new(maya_dir, settings, android::alerts(&handle), Arc::new(TauriSink { app: handle.clone() }));
            app.manage(hub.clone());
            if let Err(e) = android::create_channels(&handle) {
                log::line("android", format!("notification channels: {e}"));
            }
            log::line("app", format!("started Maya {} for Android", env!("CARGO_PKG_VERSION")));
            // The server comes back on its own when it was running last time.
            let was_main = hub.settings.lock().unwrap().config.network.role == NetworkRole::Main;
            if was_main {
                let hub = hub.clone();
                std::thread::spawn(move || {
                    let _ = hub.start_server();
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Maya");
}
```

- [ ] **Step 7: Build and test**

Run: `cargo test -p maya-mobile`
Expected: the unit tests from Tasks 3 and 4 and both integration tests PASS. `cargo build -p maya-mobile` succeeds.

- [ ] **Step 8: Commit**

```bash
git add mobile
git commit -m "feat(mobile): the hub, the page's commands, and a localhost round trip"
```

---

### Task 5: The dialogs without a local machine

The "+" and Resume dialogs assume this machine runs sessions: "This Mac" heads the picker and the first loads go to `""`. The phone has no such machine. A small module owns the switch and the choices; both dialogs use it, and show "Pair an assistant first." when no assistant is connected.

**Files:**
- Create: `src/machines.ts`, `src/machines.test.ts`
- Modify: `src/newsession.ts` (the `MachineChoice` interface at :82-86, `NewSessionModel` at :88-110, `renderNewSession` picker gate at :178, the `needsSetup` block at :195, `openNewSession` at :515-553), `src/resume.ts` (the same five places: :18-21, :23-39, the picker gate, the `needsSetup` block at :136, `openResume` at :348-380)
- Test: `src/newsession.test.ts`, `src/resume.test.ts`, `src/newsession-flow.test.ts`

**Interfaces:**
- Produces (`src/machines.ts`): `setLocalMachine(on: boolean)`, `hasLocalMachine(): boolean`, `machineChoices(machines: MachineInfo[]): MachineChoice[]`, `initialChoices(): MachineChoice[]`, types `MachineChoice { name; value }`, `MachineInfo { name; hostname; platform; connected }`.
- `NewSessionModel` and `ResumeModel` gain `noMachines?: boolean`.
- The "no machines" panel: `.modal__setup` with the text "Pair an assistant first." and `button[data-action=open-settings]` labelled "Open Network".

- [ ] **Step 1: Write the failing tests**

`src/machines.test.ts`:

```ts
import { afterEach, describe, expect, it } from "vitest";
import { hasLocalMachine, initialChoices, machineChoices, setLocalMachine } from "./machines";

const laptop = { name: "laptop", hostname: "laptop", platform: "macos", connected: true };
const away = { name: "mini", hostname: "mini", platform: "linux", connected: false };

describe("machine choices", () => {
  afterEach(() => setLocalMachine(true));

  it("lists this machine first, then connected assistants only", () => {
    expect(hasLocalMachine()).toBe(true);
    expect(machineChoices([laptop, away]).map((m) => m.value)).toEqual(["", "laptop"]);
    expect(machineChoices([laptop])[0].name).toMatch(/^This /);
    expect(initialChoices()).toHaveLength(1);
  });

  it("without a local machine lists assistants alone", () => {
    setLocalMachine(false);
    expect(hasLocalMachine()).toBe(false);
    expect(machineChoices([laptop, away])).toEqual([{ name: "laptop", value: "laptop" }]);
    expect(machineChoices([away])).toEqual([]);
    expect(initialChoices()).toEqual([]);
  });
});
```

In `src/newsession.test.ts`, add inside `describe("renderNewSession")` (import `setLocalMachine` from `./machines`):

```ts
  it("without a local machine shows the lone assistant, and asks to pair when there is none", () => {
    setLocalMachine(false);
    try {
      const one = renderNewSession({ ...base, machines: [{ name: "laptop", value: "laptop" }], machine: "laptop" }, handlers());
      expect([...one.querySelectorAll<HTMLOptionElement>("select[name=machine] option")].map((o) => o.textContent)).toEqual(["laptop"]);
      const h = handlers();
      const none = renderNewSession({ ...base, machines: [], machine: "", noMachines: true }, h);
      expect(none.textContent).toContain("Pair an assistant first.");
      expect(none.querySelector("select[name=dir]")).toBeNull();
      expect(none.querySelector("textarea[name=prompt]")).toBeNull();
      none.querySelector<HTMLButtonElement>("button[data-action=open-settings]")!.click();
      expect(h.onOpenSettings).toHaveBeenCalled();
    } finally {
      setLocalMachine(true);
    }
  });
```

In `src/resume.test.ts`, the same shape against `renderResume` (its "none" render must have no `select[name=dir]`, and the text).

In `src/newsession-flow.test.ts`, add a test (import `setLocalMachine` from `./machines`):

```ts
  it("without a local machine opens on the first connected assistant and never asks this machine", async () => {
    setLocalMachine(false);
    try {
      invoke.mockImplementation((cmd: string, args?: { machine?: string }) => {
        if (cmd === "list_machines") return Promise.resolve([{ name: "mini", hostname: "mini", platform: "linux", connected: false }, { name: "laptop", hostname: "laptop", platform: "macos", connected: true }]);
        if (cmd === "list_project_dirs") return args?.machine === "laptop" ? Promise.resolve(["hexgrid"]) : Promise.reject(new Error("asked " + args?.machine));
        if (cmd === "list_agents") return Promise.resolve({ agents: [CLAUDE_AGENT], names: true });
        if (cmd === "get_config") return Promise.resolve({});
        return Promise.reject(new Error("unexpected " + cmd));
      });
      await openNewSession();
      await flush();
      await flush();
      expect(invoke).not.toHaveBeenCalledWith("list_project_dirs", { machine: "" });
      expect(invoke).toHaveBeenCalledWith("list_project_dirs", { machine: "laptop" });
      expect(document.querySelector<HTMLSelectElement>("select[name=machine]")!.value).toBe("laptop");
      expect([...document.querySelectorAll<HTMLOptionElement>("select[name=dir] option")].map((o) => o.textContent)).toContain("hexgrid");
    } finally {
      setLocalMachine(true);
    }
  });

  it("without any connected assistant asks to pair one", async () => {
    setLocalMachine(false);
    try {
      invoke.mockImplementation((cmd: string) => {
        if (cmd === "list_machines") return Promise.resolve([]);
        if (cmd === "get_config") return Promise.resolve({});
        return Promise.reject(new Error("unexpected " + cmd));
      });
      await openNewSession();
      await flush();
      expect(document.getElementById("modal-host")!.textContent).toContain("Pair an assistant first.");
      expect(invoke).not.toHaveBeenCalledWith("list_project_dirs", expect.anything());
    } finally {
      setLocalMachine(true);
    }
  });
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm install && pnpm test -- machines newsession resume`
Expected: FAIL, "Cannot find module './machines'" and the render tests failing on the missing panel.

- [ ] **Step 3: Write `src/machines.ts`**

```ts
// Whether this Maya runs sessions of its own, and what the Machine pickers
// list. The desktop does; the phone does not, so its pickers list the
// connected assistants alone and a dialog with none says to pair one.

import { thisComputer } from "./platform";

export interface MachineChoice {
  name: string;
  value: string;
}

/** One row of `list_machines`. */
export interface MachineInfo {
  name: string;
  hostname: string;
  platform: string;
  connected: boolean;
}

let local = true;

/** Off on the phone, which has no sessions of its own. */
export function setLocalMachine(on: boolean): void {
  local = on;
}

export function hasLocalMachine(): boolean {
  return local;
}

/** "This Mac" first when this machine runs sessions, then each connected assistant. */
export function machineChoices(machines: MachineInfo[]): MachineChoice[] {
  const remote = machines.filter((m) => m.connected).map((m) => ({ name: m.name, value: m.name }));
  return local ? [{ name: thisComputer(), value: "" }, ...remote] : remote;
}

/** What a dialog shows before `list_machines` answers. */
export function initialChoices(): MachineChoice[] {
  return local ? [{ name: thisComputer(), value: "" }] : [];
}
```

- [ ] **Step 4: Change `newsession.ts`**

- Replace the local `MachineChoice` interface with `export type { MachineChoice } from "./machines";` and add `import { hasLocalMachine, initialChoices, machineChoices, type MachineInfo } from "./machines";`. Drop the now-unused `thisComputer` import if nothing else in the file uses it (`thisComputerLower` may still be used; keep what is).
- Add to `NewSessionModel`: `/** No assistant is connected and this machine runs no sessions: the form gives way to "Pair an assistant first." */ noMachines?: boolean;`
- Add beside `renderSetup`:

```ts
function renderNoMachines(h: { onOpenSettings(): void }): HTMLElement {
  const setup = el("div", "modal__setup");
  setup.append(el("p", "", "Pair an assistant first."));
  const open = el("button", "card__btn card__btn--primary", "Open Network");
  open.type = "button";
  open.dataset.action = "open-settings";
  open.addEventListener("click", () => h.onOpenSettings());
  setup.append(open);
  return setup;
}
```

- In `renderNewSession`, change the picker gate `if (m.machines.length > 1) {` to `if (m.machines.length > 1 || (!hasLocalMachine() && m.machines.length > 0)) {`, and before the `if (m.needsSetup) {` block add:

```ts
  if (m.noMachines) {
    panel.append(renderNoMachines(h));
    root.append(backdrop, panel);
    return root;
  }
```

- In `openNewSession`, set `machines: initialChoices(),` in the model, and replace everything from `void invoke<…>("list_machines")` to the end of the function with:

```ts
  const listing = invoke<MachineInfo[]>("list_machines").catch(() => [] as MachineInfo[]);
  if (hasLocalMachine()) {
    void listing.then((machines) => {
      if (!current) return;
      current.model.machines = machineChoices(machines);
      paint();
    });
    void loadAgents("");
    await loadDirs("");
    return;
  }
  // No sessions here: the first connected assistant is the machine, or there is nothing to start.
  const machines = machineChoices(await listing);
  if (!current) return;
  current.model.machines = machines;
  if (machines.length === 0) {
    current.model.noMachines = true;
    paint();
    return;
  }
  await chooseMachine(machines[0].value);
```

- [ ] **Step 5: The same change in `resume.ts`**

Mirror Step 4: the type re-export and import, `noMachines?: boolean` on `ResumeModel`, `renderNoMachines` beside its `renderSetup`, the picker gate and the `noMachines` block in `renderResume` before its `needsSetup` block, and in `openResume` the model's `machines: initialChoices()` plus the same tail (its loaders are `loadAgents` and `loadDirs`, its switch is `chooseMachine` at `resume.ts:276`).

- [ ] **Step 6: Run the tests**

Run: `pnpm test`
Expected: all green, including the new ones. Then `pnpm build` (tsc) is clean.

- [ ] **Step 7: Commit**

```bash
git add src/machines.ts src/machines.test.ts src/newsession.ts src/newsession.test.ts src/newsession-flow.test.ts src/resume.ts src/resume.test.ts
git commit -m "feat(page): dialogs that work on a machine with no sessions of its own"
```

---

### Task 6: The phone board: column strip, one-to-four columns, and the page entry

**Files:**
- Modify: `src/board.ts` (`swapBoard`), `package.json` (scripts, dependency)
- Create: `src/mobile/strip.ts`, `src/mobile/strip.test.ts`, `src/mobile/mobile.css`, `src/mobile/main.ts`, `mobile/web/index.html`, `vite.mobile.config.ts`
- Test: `src/board.test.ts`

**Interfaces:**
- Produces (`src/mobile/strip.ts`): `GAP = 12`, `SIDE = 12`, `MIN_COLUMN = 200`, `columnsFor(width: number): number`, `columnWidth(width: number, n: number): number`, `activeColumn(scrollLeft: number, colWidth: number): number`, `scrollLeftFor(index: number, colWidth: number): number`, `visibleColumns(active: number, n: number): number[]`, `openingColumn(cards: Card[]): number`, `renderStrip(counts: Record<CardState, number>, visible: number[], onPick: (index: number) => void): HTMLElement`.
- `swapBoard` keeps `.board`'s `scrollLeft` across a repaint.
- `package.json` scripts: `dev:mobile`, `build:mobile`, `mobile:dev`, `mobile:build`, `mobile:desktop`; dependency `@tauri-apps/plugin-notification`.

- [ ] **Step 1: Write the failing tests**

Add to `src/board.test.ts`:

```ts
describe("swapBoard on a sideways-scrolling board", () => {
  it("keeps the board's own horizontal scroll across a repaint", () => {
    const host = document.createElement("div");
    host.append(renderBoard([card({})], NOW));
    const old = host.querySelector<HTMLElement>(".board")!;
    Object.defineProperty(old, "scrollLeft", { value: 640, writable: true });
    const fresh = renderBoard([card({})], NOW);
    let set = 0;
    Object.defineProperty(fresh, "scrollLeft", { get: () => set, set: (v: number) => { set = v; } });
    swapBoard(host, fresh);
    expect(set).toBe(640);
  });
});
```

`src/mobile/strip.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import type { Card } from "../types";
import { activeColumn, columnWidth, columnsFor, openingColumn, renderStrip, scrollLeftFor, visibleColumns } from "./strip";

const NOW = 1_790_600_000_000;
function card(state: Card["state"]): Card {
  return { sessionId: state, pid: 1, name: "x", cwd: "/x", state, stateSince: NOW, snippet: "", awaiting: null, hasInbox: true, harness: "claude-code", pr: null, context: null };
}

describe("columns for a width", () => {
  it("gives one column on a phone upright, three or four sideways, never more than four", () => {
    expect(columnsFor(360)).toBe(1);
    expect(columnsFor(411)).toBe(1);
    expect(columnsFor(640)).toBe(3);
    expect(columnsFor(800)).toBe(3);
    expect(columnsFor(915)).toBe(4);
    expect(columnsFor(1280)).toBe(4);
  });

  it("sizes columns to fill the width minus the gutters and gaps", () => {
    expect(columnWidth(360, 1)).toBe(336);
    expect(columnWidth(915, 4)).toBe((915 - 24 - 36) / 4);
  });
});

describe("the active column", () => {
  it("follows the scroll position and survives a change of column count", () => {
    const portrait = columnWidth(360, 1);
    expect(activeColumn(0, portrait)).toBe(0);
    expect(activeColumn(scrollLeftFor(2, portrait), portrait)).toBe(2);
    expect(activeColumn(scrollLeftFor(2, portrait) + 40, portrait)).toBe(2);
    // Rotating to landscape (three columns): the same column stays in view.
    const landscape = columnWidth(800, 3);
    const after = scrollLeftFor(activeColumn(scrollLeftFor(2, portrait), portrait), landscape);
    expect(activeColumn(after, landscape)).toBe(2);
    expect(activeColumn(10_000, portrait)).toBe(3);
  });

  it("lists the columns on screen, clamped to the board", () => {
    expect(visibleColumns(0, 1)).toEqual([0]);
    expect(visibleColumns(2, 1)).toEqual([2]);
    expect(visibleColumns(0, 3)).toEqual([0, 1, 2]);
    expect(visibleColumns(2, 3)).toEqual([1, 2, 3]);
    expect(visibleColumns(3, 4)).toEqual([0, 1, 2, 3]);
  });

  it("opens on Awaiting Decision when it has cards, else Working", () => {
    expect(openingColumn([card("idle"), card("awaiting")])).toBe(2);
    expect(openingColumn([card("idle"), card("completed")])).toBe(1);
    expect(openingColumn([])).toBe(1);
  });
});

describe("renderStrip", () => {
  it("names the four columns with counts, marks the visible ones, and picks on tap", () => {
    const onPick = vi.fn();
    const el = renderStrip({ idle: 11, working: 3, awaiting: 1, completed: 0 }, [1, 2], onPick);
    const tabs = [...el.querySelectorAll<HTMLButtonElement>(".strip__tab")];
    expect(tabs.map((t) => t.dataset.column)).toEqual(["idle", "working", "awaiting", "completed"]);
    expect(tabs.map((t) => t.querySelector(".strip__count")!.textContent)).toEqual(["11", "3", "1", "0"]);
    expect(tabs.map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "true", "true", "false"]);
    tabs[3].click();
    expect(onPick).toHaveBeenCalledWith(3);
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- strip board`
Expected: FAIL, "Cannot find module './strip'" and the `swapBoard` test expecting 640 but getting 0.

- [ ] **Step 3: `swapBoard` keeps the sideways scroll**

In `src/board.ts`, `swapBoard` becomes:

```ts
export function swapBoard(host: HTMLElement, fresh: HTMLElement): void {
  const scroll = new Map<string, number>();
  for (const col of host.querySelectorAll<HTMLElement>(".column")) {
    const list = col.querySelector<HTMLElement>(".column__cards");
    if (list && col.dataset.state) scroll.set(col.dataset.state, list.scrollTop);
  }
  // The board itself scrolls sideways on a phone; that position is kept too.
  const across = host.querySelector<HTMLElement>(".board")?.scrollLeft ?? 0;
  host.replaceChildren(fresh);
  for (const col of fresh.querySelectorAll<HTMLElement>(".column")) {
    const list = col.querySelector<HTMLElement>(".column__cards");
    const top = col.dataset.state ? scroll.get(col.dataset.state) : undefined;
    if (list && top) list.scrollTop = top;
  }
  if (across) fresh.scrollLeft = across;
}
```

- [ ] **Step 4: Write `src/mobile/strip.ts`**

```ts
// The phone's column strip: which of the four columns are on screen, how
// many fit the width, and where the board scrolls to show one.

import { COLUMNS, type Card, type CardState } from "../types";

/** The gap between columns and the gutter either side, in CSS pixels (dp). */
export const GAP = 12;
export const SIDE = 12;
/** A column narrower than this is unreadable. */
export const MIN_COLUMN = 200;

/** How many columns fit: as many of at least 200 dp as the width takes, one to four. */
export function columnsFor(width: number): number {
  const usable = width - 2 * SIDE;
  for (let n = 4; n > 1; n--) {
    if (n * MIN_COLUMN + (n - 1) * GAP <= usable) return n;
  }
  return 1;
}

export function columnWidth(width: number, n: number): number {
  return (width - 2 * SIDE - (n - 1) * GAP) / n;
}

/** The leftmost column on screen, from the board's scroll position. */
export function activeColumn(scrollLeft: number, colWidth: number): number {
  return Math.max(0, Math.min(COLUMNS.length - 1, Math.round(scrollLeft / (colWidth + GAP))));
}

export function scrollLeftFor(index: number, colWidth: number): number {
  return index * (colWidth + GAP);
}

/** The indices on screen when `active` is the leftmost and `n` fit, kept inside the four. */
export function visibleColumns(active: number, n: number): number[] {
  const first = Math.max(0, Math.min(active, COLUMNS.length - n));
  return Array.from({ length: Math.min(n, COLUMNS.length) }, (_, i) => first + i);
}

/** Awaiting Decision when something waits, else Working. */
export function openingColumn(cards: Card[]): number {
  const wanted: CardState = cards.some((c) => c.state === "awaiting") ? "awaiting" : "working";
  return COLUMNS.findIndex((c) => c.state === wanted);
}

export function renderStrip(counts: Record<CardState, number>, visible: number[], onPick: (index: number) => void): HTMLElement {
  const nav = document.createElement("nav");
  nav.className = "strip";
  nav.setAttribute("aria-label", "Columns");
  COLUMNS.forEach((col, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = `strip__tab strip__tab--${col.state}`;
    b.dataset.column = col.state;
    b.setAttribute("aria-selected", String(visible.includes(i)));
    const name = document.createElement("span");
    name.className = "strip__name";
    name.textContent = col.title;
    const count = document.createElement("span");
    count.className = "strip__count";
    count.textContent = String(counts[col.state]);
    b.append(name, count);
    b.addEventListener("click", () => onPick(i));
    nav.append(b);
  });
  return nav;
}
```

- [ ] **Step 5: Run the tests**

Run: `pnpm test -- strip board`
Expected: PASS.

- [ ] **Step 6: The page shell, the stylesheet and the Vite config**

`mobile/web/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
    <title>Maya</title>
    <link rel="stylesheet" href="../../src/styles.css" />
    <link rel="stylesheet" href="../../src/mobile/mobile.css" />
  </head>
  <body>
    <div id="app">
      <header class="topbar">
        <h1 class="topbar__title" title="Manage All Your Agents"><img class="topbar__logo" src="../../src/assets/maya-logo.png" alt="" width="26" height="26" /> Maya</h1>
        <nav class="tabs" role="tablist">
          <button class="tab" type="button" role="tab" data-tab="sessions">Sessions</button>
          <span class="tabs__spacer"></span>
          <button class="tab" type="button" role="tab" data-tab="debug">Debug</button>
          <button class="tab" type="button" role="tab" data-tab="settings">Network</button>
        </nav>
      </header>
      <div id="strip-host"></div>
      <div id="board"></div>
      <div id="debug" class="pane" hidden></div>
      <div id="settings" class="settings pane" hidden></div>
      <div id="setup" hidden></div>
      <div id="modal-host"></div>
      <div id="toast" class="toast" hidden></div>
    </div>
    <script type="module" src="../../src/mobile/main.ts"></script>
  </body>
</html>
```

`src/mobile/mobile.css`:

```css
/* The phone's layout over styles.css: one to four columns that snap
   sideways, a strip naming them, full-screen cards and dialogs, and
   touch-sized controls. */
#board { --col-w: 100%; }
.board { display: flex; grid-template-columns: none; overflow-x: auto; overflow-y: hidden; scroll-snap-type: x mandatory; gap: 12px; padding: 8px 12px 12px; scrollbar-width: none; }
.board::-webkit-scrollbar { display: none; }
.column { flex: 0 0 var(--col-w); scroll-snap-align: start; min-width: 0; }
.board__hint { margin: 24px auto; padding: 0 24px; text-align: center; color: var(--muted); max-width: 360px; }
.board__hint button { margin-top: 12px; }

.strip { display: flex; gap: 4px; padding: 6px 12px 0; }
.strip__tab { flex: 1; display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 6px 2px; background: none; border: 0; border-bottom: 3px solid var(--border); color: var(--muted); font: inherit; font-size: 11px; text-transform: uppercase; letter-spacing: .04em; }
.strip__tab[aria-selected="true"] { color: var(--text); border-bottom-color: var(--gold); }
.strip__count { font-size: 13px; font-weight: 600; }
.strip__tab--awaiting[aria-selected="true"] { border-bottom-color: var(--awaiting); }
.strip__tab--working[aria-selected="true"] { border-bottom-color: var(--working); }
.strip__tab--completed[aria-selected="true"] { border-bottom-color: var(--completed); }

/* Cards and dialogs fill the screen; the × in the header is the way back. */
.modal__panel, .modal__panel--compact { width: 100vw; height: 100vh; max-width: none; max-height: none; min-width: 0; min-height: 0; border: 0; border-radius: 0; }
.modal__backdrop { display: none; }
.modal__attach { margin-right: 6px; }

.topbar { padding: 8px 12px; }
.tabs .tab__count { display: none; }
.card__btn, .settings button, .strip__tab, .modal__close { min-height: 40px; }
.card { padding: 12px 14px; }

#setup { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; padding: 24px; background: var(--bg); z-index: 20; }
.setup__card { width: 100%; max-width: 420px; background: var(--panel); border: 1px solid var(--border); border-radius: 12px; padding: 20px; display: flex; flex-direction: column; gap: 12px; }
.setup__card h2 { margin: 0; color: var(--gold-bright); }
.setup__card label { display: flex; flex-direction: column; gap: 4px; color: var(--muted); font-size: 12px; }
.setup__card input { font: inherit; font-size: 15px; padding: 8px 10px; background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 6px; }
.setup__error { color: #f87171; }

.network__code { font-size: 32px; letter-spacing: .12em; font-variant-numeric: tabular-nums; color: var(--gold-bright); }
.network__addresses { color: var(--muted); }
.network__assistant { display: flex; align-items: center; gap: 8px; padding: 8px 0; border-bottom: 1px solid var(--border); }
.network__assistant-name { flex: 1; min-width: 0; }
.network__assistant-meta { color: var(--muted); font-size: 12px; }
```

`vite.mobile.config.ts`:

```ts
import { defineConfig, searchForWorkspaceRoot } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// The phone's page: the same src/ modules behind mobile/web/index.html,
// served on 1430 for `tauri android dev` and built to dist-mobile/.
export default defineConfig(() => ({
  root: "mobile/web",
  publicDir: false,
  clearScreen: false,
  build: { outDir: "../../dist-mobile", emptyOutDir: true },
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    fs: { allow: [searchForWorkspaceRoot(process.cwd())] },
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/core/**", "**/cli/**", "**/hook/**", "**/vendor/**", "**/mobile/gen/**"] },
  },
}));
```

`package.json`: add to `scripts`

```json
    "dev:mobile": "vite --config vite.mobile.config.ts",
    "build:mobile": "tsc && vite build --config vite.mobile.config.ts",
    "mobile:desktop": "cd mobile && tauri dev",
    "mobile:dev": "cd mobile && tauri android dev",
    "mobile:build": "cd mobile && tauri android build --apk --target aarch64"
```

and to `dependencies`: `"@tauri-apps/plugin-notification": "^2"`. Run `pnpm install`. (The CLI finds `mobile/tauri.conf.json` because it checks the current directory before `src-tauri`, and runs `beforeDevCommand` in the repo root, the parent of the Tauri directory, since no `package.json` sits under `mobile/`.)

- [ ] **Step 7: Write `src/mobile/main.ts`**

```ts
// The phone's page: the desktop's board, cards, modal and dialogs over the
// commands the mobile crate serves, laid out one to four columns wide.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cardActionFor } from "../actions";
import { answerGuard } from "../answer";
import { renderBoard, swapBoard } from "../board";
import { makeClickGuard } from "../clickguard";
import { initDebug } from "../debug";
import { setLocalMachine } from "../machines";
import { closeModal, openModal, refreshModal, setComposerOptions, setProgress } from "../modal";
import { closeNewSession, openNewSession } from "../newsession";
import { nextEnableDelay } from "../options";
import { makeProgress } from "../progress";
import { closeResume, openResume } from "../resume";
import { makeTabs } from "../tabs";
import { showToast } from "../toast";
import type { Card, CardState } from "../types";
import { initNetwork, type NetworkStatus } from "./network";
import { watchTaps } from "./notify-tap";
import { initSetup } from "./setup";
import { activeColumn, columnWidth, columnsFor, openingColumn, renderStrip, scrollLeftFor, visibleColumns } from "./strip";

setLocalMachine(false);
setComposerOptions({ attachButton: true });

let cards: Card[] = [];
let status: NetworkStatus | null = null;
let opened = false;
let pendingTap: string | null = null;
let colWidth = 0;
let columns = 1;
const guard = makeClickGuard();
const progress = makeProgress();
setProgress(progress);
let retry: ReturnType<typeof setTimeout> | undefined;
let enableTimer: ReturnType<typeof setTimeout> | undefined;
let known = new Set<string>();
let tabs: ReturnType<typeof makeTabs> | null = null;

function boardEl(): HTMLElement | null {
  return document.querySelector<HTMLElement>("#board .board");
}

function counts(): Record<CardState, number> {
  const c = { idle: 0, working: 0, awaiting: 0, completed: 0 };
  for (const card of cards) c[card.state] += 1;
  return c;
}

/** Re-measures the columns for the viewport and keeps the active column in view. */
function layout(): void {
  const host = document.getElementById("board");
  if (!host) return;
  const width = host.clientWidth || window.innerWidth;
  const before = colWidth ? activeColumn(boardEl()?.scrollLeft ?? 0, colWidth) : null;
  columns = columnsFor(width);
  colWidth = columnWidth(width, columns);
  host.style.setProperty("--col-w", `${colWidth}px`);
  const board = boardEl();
  if (board && before !== null) board.scrollLeft = scrollLeftFor(before, colWidth);
  paintStrip();
}

function paintStrip(): void {
  const host = document.getElementById("strip-host");
  if (!host) return;
  const active = activeColumn(boardEl()?.scrollLeft ?? 0, colWidth);
  host.replaceChildren(renderStrip(counts(), visibleColumns(active, columns), (i) => boardEl()?.scrollTo({ left: scrollLeftFor(i, colWidth), behavior: "smooth" })));
}

function paintHint(host: HTMLElement): void {
  if (!status || status.assistants.length > 0) return;
  const hint = document.createElement("div");
  hint.className = "board__hint";
  const p = document.createElement("p");
  p.textContent = "Pair an assistant to see its sessions.";
  const open = document.createElement("button");
  open.type = "button";
  open.className = "card__btn card__btn--primary";
  open.textContent = "Open Network";
  open.addEventListener("click", () => tabs?.show("settings"));
  hint.append(p, open);
  host.append(hint);
}

function paint(): void {
  if (!guard.canPaint(Date.now())) {
    if (!retry) retry = setTimeout(() => { retry = undefined; paint(); }, 200);
    return;
  }
  const host = document.getElementById("board");
  if (!host) return;
  swapBoard(host, renderBoard(cards, Date.now(), (c) => progress.next(c)));
  paintHint(host);
  guard.markPaint(Date.now(), { movedUnderPointer: false });
  const delay = nextEnableDelay(cards, Date.now());
  if (enableTimer) clearTimeout(enableTimer);
  enableTimer = delay === null ? undefined : setTimeout(() => { enableTimer = undefined; paint(); }, delay + 50);
  const board = boardEl();
  board?.addEventListener("scroll", paintStrip, { passive: true });
  if (!opened && board && cards.length > 0) {
    opened = true;
    board.scrollLeft = scrollLeftFor(openingColumn(cards), colWidth);
  }
  paintStrip();
}

async function run(label: string, f: () => Promise<unknown>, done?: string): Promise<void> {
  try {
    await f();
    if (done) showToast(done);
  } catch (e) {
    showToast(`${label}: ${String(e)}`);
  }
}

function open(card: Card): void {
  closeNewSession();
  closeResume();
  history.pushState({ card: card.sessionId }, "");
  void openModal(card);
}

function act(target: Element): void {
  const action = cardActionFor(target);
  if (!action) return;
  const card = cards.find((c) => c.sessionId === action.sessionId);
  if (!card) return;
  if (action.kind === "terminal") showToast(`That session runs on ${card.machine ?? "another machine"}`);
  else if (action.kind === "pr") void run("Pull request", () => invoke("open_pr", { sessionId: card.sessionId }));
  else if (action.kind === "compact") void run("Compact", () => invoke("compact_session", { sessionId: card.sessionId }), "Sent /compact to the terminal");
  else if (action.kind === "close") void run("Close", () => invoke("close_session", { sessionId: card.sessionId }), `Closed ${card.name}`);
  else if (action.kind === "answer") void answer(card, action.questionIndex, action.optionIndex, target);
  else open(card);
}

async function answer(card: Card, questionIndex: number, optionIndex: number, button: Element): Promise<void> {
  try {
    const outcome = await answerGuard.answer(card, questionIndex, optionIndex, button as HTMLElement);
    if (outcome === "dropped") return;
    progress.advance(card);
    paint();
  } catch (e) {
    showToast(String(e));
  }
}

function openFromTap(sessionId: string): void {
  const card = cards.find((c) => c.sessionId === sessionId);
  if (card) {
    tabs?.show("sessions");
    open(card);
  } else {
    pendingTap = sessionId;
  }
}

async function start(): Promise<void> {
  void initDebug();
  tabs = makeTabs();
  initNetwork((s) => {
    status = s;
    paint();
  });
  initSetup();
  layout();
  window.addEventListener("resize", layout);
  // Android's back button pops the history entry a card pushed: close the card.
  window.addEventListener("popstate", () => closeModal());
  const board = document.getElementById("board");
  board?.addEventListener("pointerdown", () => guard.setPointerDown(true));
  window.addEventListener("pointerup", () => guard.setPointerDown(false));
  board?.addEventListener("click", (ev) => {
    const target = ev.target as Element;
    if (target.closest("[data-action=new-session]")) {
      closeResume();
      void openNewSession();
      return;
    }
    if (target.closest("[data-action=resume-session]")) {
      closeNewSession();
      void openResume();
      return;
    }
    if (!guard.allowClick(Date.now())) return;
    act(target);
  });
  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    for (const c of cards) if (c.state !== "awaiting") progress.reset(c.sessionId);
    for (const id of known) if (!cards.some((c) => c.sessionId === id)) progress.reset(id);
    known = new Set(cards.map((c) => c.sessionId));
    paint();
    refreshModal(cards);
    if (pendingTap) {
      const id = pendingTap;
      pendingTap = null;
      openFromTap(id);
    }
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  void watchTaps(openFromTap);
  setInterval(paint, 10_000);
}

void start();
```

This imports `setComposerOptions` (Task 8), `./network` and `./setup` (Task 7) and `./notify-tap` (Task 8). Until those land, `pnpm build:mobile` fails on them; the vitest suite does not import `main.ts`, so the tests run. Tasks 7 and 8 close the gap, and Task 8's last step builds the page.

- [ ] **Step 8: Run the tests and commit**

Run: `pnpm test`
Expected: PASS.

```bash
git add src/board.ts src/board.test.ts src/mobile/strip.ts src/mobile/strip.test.ts src/mobile/mobile.css src/mobile/main.ts mobile/web/index.html vite.mobile.config.ts package.json pnpm-lock.yaml
git commit -m "feat(page): the phone board, one to four columns with a strip"
```

---

### Task 7: The Network screen and the setup overlay

**Files:**
- Create: `src/mobile/network.ts`, `src/mobile/network.test.ts`, `src/mobile/setup.ts`, `src/mobile/setup.test.ts`

**Interfaces:**
- Produces (`src/mobile/network.ts`): `type NetworkStatus` (re-exported from `../settings`), `interface NetworkModel { status: NetworkStatus; name: string; port: number; notifyAwaiting: boolean; notifyCompleted: boolean; addresses: string[]; notificationsAllowed: boolean; busy: boolean; error: string | null }`, `interface NetworkHandlers { onSave(name: string, port: number): void; onStart(): void; onStop(): void; onCode(): void; onRemove(id: string): void; onSwitch(which: "awaiting" | "completed", on: boolean): void; onBattery(): void }`, `renderNetwork(m: NetworkModel, h: NetworkHandlers, nowMs?: number): HTMLElement`, `initNetwork(onStatus: (s: NetworkStatus) => void): void`.
- Produces (`src/mobile/setup.ts`): `interface SetupModel { firstRun: boolean; name: string; port: number; error: string | null; busy: boolean }`, `renderSetup(m: SetupModel, h: { onStart(name: string, port: number): void }): HTMLElement`, `isFirstRun(status: NetworkStatus): boolean`, `initSetup(): void`.
- Both modules call `get_config`, `set_config`, `network_status`, `network_pairing_code`, `network_remove_assistant`, `server_start`, `server_stop`, `local_addresses`, `notifications_allowed`, `request_battery_exemption`, and the setup overlay asks notification permission through `@tauri-apps/plugin-notification` before `server_start`.

- [ ] **Step 1: Write the failing tests**

`src/mobile/network.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { renderNetwork, type NetworkModel, type NetworkStatus } from "./network";

const NOW = Date.parse("2026-10-07T10:00:00Z");
const running: NetworkStatus = {
  role: "main",
  code: { code: "483921", expiresAt: NOW + 4 * 60_000 },
  assistants: [
    { id: "a", name: "laptop (10.0.0.2)", hostname: "laptop", platform: "macos", address: "10.0.0.2", connected: true, lastSeen: NOW, note: null },
    { id: "b", name: "laptop (10.0.0.3)", hostname: "laptop", platform: "linux", address: "10.0.0.3", connected: false, lastSeen: NOW - 3_600_000, note: "runs Maya 0.11.0; this phone runs 0.13.0" },
  ],
  assistant: { connected: false, mainName: null, error: null },
  mainError: null,
};
const base: NetworkModel = { status: running, name: "Pixel 8", port: 4127, notifyAwaiting: true, notifyCompleted: false, addresses: ["192.168.1.20"], notificationsAllowed: true, busy: false, error: null };
const handlers = () => ({ onSave: vi.fn(), onStart: vi.fn(), onStop: vi.fn(), onCode: vi.fn(), onRemove: vi.fn(), onSwitch: vi.fn(), onBattery: vi.fn() });

describe("renderNetwork", () => {
  it("shows the code, when it expires, and the phone's addresses while the server runs", () => {
    const el = renderNetwork(base, handlers(), NOW);
    expect(el.querySelector(".network__code")!.textContent).toBe("483 921");
    expect(el.textContent).toContain("expires in 4 min");
    expect(el.textContent).toContain("192.168.1.20");
    expect(el.querySelector<HTMLButtonElement>("button[data-action=stop]")).not.toBeNull();
    expect(el.querySelector("button[data-action=start]")).toBeNull();
  });

  it("lists assistants with the labels the cards use, their state and notes, and removes on tap", () => {
    const h = handlers();
    const el = renderNetwork(base, h, NOW);
    const rows = [...el.querySelectorAll<HTMLElement>(".network__assistant")];
    expect(rows.map((r) => r.querySelector(".network__assistant-name")!.textContent)).toEqual(["laptop (10.0.0.2)", "laptop (10.0.0.3)"]);
    expect(rows[0].textContent).toContain("Connected");
    expect(rows[1].textContent).toContain("Last seen");
    expect(rows[1].textContent).toContain("runs Maya 0.11.0");
    rows[1].querySelector<HTMLButtonElement>("button[data-action=remove-assistant]")!.click();
    expect(h.onRemove).toHaveBeenCalledWith("b");
  });

  it("offers Start with the error under Port while the server is down, and no code", () => {
    const h = handlers();
    const down: NetworkModel = { ...base, status: { ...running, role: "off", code: null, assistants: [], mainError: "Could not listen on port 4127: address in use. Choose another port." } };
    const el = renderNetwork(down, h, NOW);
    expect(el.querySelector(".network__code")).toBeNull();
    expect(el.textContent).toContain("Could not listen on port 4127");
    expect(el.textContent).toContain("No assistants paired yet.");
    el.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalled();
  });

  it("saves name and port, flips the switches, and asks for the battery exemption", () => {
    const h = handlers();
    const el = renderNetwork(base, h, NOW);
    el.querySelector<HTMLInputElement>("input[name=name]")!.value = "Fold";
    el.querySelector<HTMLInputElement>("input[name=port]")!.value = "5000";
    el.querySelector<HTMLButtonElement>("button[data-action=save]")!.click();
    expect(h.onSave).toHaveBeenCalledWith("Fold", 5000);
    const completed = el.querySelector<HTMLInputElement>("input[name=notifyCompleted]")!;
    expect(completed.checked).toBe(false);
    completed.checked = true;
    completed.dispatchEvent(new Event("change"));
    expect(h.onSwitch).toHaveBeenCalledWith("completed", true);
    el.querySelector<HTMLButtonElement>("button[data-action=battery]")!.click();
    expect(h.onBattery).toHaveBeenCalled();
    expect(el.textContent).toContain("Android may otherwise stop the server with the screen off.");
  });

  it("says when Android has notifications off for Maya", () => {
    const el = renderNetwork({ ...base, notificationsAllowed: false }, handlers(), NOW);
    expect(el.textContent).toContain("Notifications are off for Maya in Android settings.");
    expect(renderNetwork(base, handlers(), NOW).textContent).not.toContain("Notifications are off");
  });

  it("drops an expired code and offers to show one", () => {
    const el = renderNetwork(base, handlers(), NOW + 10 * 60_000);
    expect(el.querySelector(".network__code")).toBeNull();
    expect(el.querySelector<HTMLButtonElement>("button[data-action=regenerate-code]")!.textContent).toBe("Show pairing code");
  });
});
```

`src/mobile/setup.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import type { NetworkStatus } from "./network";
import { isFirstRun, renderSetup } from "./setup";

const off: NetworkStatus = { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null };

describe("the setup overlay", () => {
  it("is a first run until the server has run or an assistant is paired", () => {
    expect(isFirstRun(off)).toBe(true);
    expect(isFirstRun({ ...off, role: "main" })).toBe(false);
    expect(isFirstRun({ ...off, assistants: [{ id: "a", name: "x", hostname: "x", platform: "macos", address: "", connected: false, lastSeen: null, note: null }] })).toBe(false);
  });

  it("starts with the name and port typed, and explains itself differently the first time", () => {
    const h = { onStart: vi.fn() };
    const first = renderSetup({ firstRun: true, name: "Pixel 8", port: 4127, error: null, busy: false }, h);
    expect(first.textContent).toContain("pair them with the code");
    expect(first.querySelector<HTMLInputElement>("input[name=name]")!.value).toBe("Pixel 8");
    first.querySelector<HTMLInputElement>("input[name=port]")!.value = "4200";
    first.querySelector<HTMLButtonElement>("button[data-action=start]")!.click();
    expect(h.onStart).toHaveBeenCalledWith("Pixel 8", 4200);
    const later = renderSetup({ firstRun: false, name: "Pixel 8", port: 4127, error: "Could not listen on port 4127", busy: true }, h);
    expect(later.textContent).toContain("The server is stopped.");
    expect(later.textContent).toContain("Could not listen on port 4127");
    expect(later.querySelector<HTMLButtonElement>("button[data-action=start]")!.disabled).toBe(true);
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- mobile/network mobile/setup`
Expected: FAIL, modules not found.

- [ ] **Step 3: Write `src/mobile/network.ts`**

```ts
// The Network tab: this phone's name and port, the pairing code and the
// addresses to type into an assistant, the paired assistants, the two
// notification switches, and the battery exemption.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "../format";
import type { NetworkStatus } from "../settings";
import { showToast } from "../toast";

export type { NetworkStatus } from "../settings";

export interface NetworkModel {
  status: NetworkStatus;
  name: string;
  port: number;
  notifyAwaiting: boolean;
  notifyCompleted: boolean;
  addresses: string[];
  notificationsAllowed: boolean;
  busy: boolean;
  error: string | null;
}

export interface NetworkHandlers {
  onSave(name: string, port: number): void;
  onStart(): void;
  onStop(): void;
  onCode(): void;
  onRemove(id: string): void;
  onSwitch(which: "awaiting" | "completed", on: boolean): void;
  onBattery(): void;
}

const PLATFORM_NAMES: Record<string, string> = { macos: "macOS", linux: "Linux", windows: "Windows" };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function section(title: string): HTMLElement {
  const s = el("section", "settings__section");
  s.append(el("h2", "settings__heading", title));
  return s;
}

function button(label: string, action: string, onClick: () => void, primary = false): HTMLButtonElement {
  const b = el("button", primary ? "card__btn card__btn--primary" : "card__btn", label);
  b.type = "button";
  b.dataset.action = action;
  b.addEventListener("click", onClick);
  return b;
}

/** "483 921" from "483921". */
function formatCode(code: string): string {
  return code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

function minutesLeft(expiresAt: number, nowMs: number): number {
  return Math.max(0, Math.ceil((expiresAt - nowMs) / 60_000));
}

export function renderNetwork(m: NetworkModel, h: NetworkHandlers, nowMs: number = Date.now()): HTMLElement {
  const root = el("div", "settings__body network");
  const running = m.status.role === "main";

  const phone = section("This phone");
  const name = el("label", "", "Name ");
  const nameInput = el("input", "");
  nameInput.name = "name";
  nameInput.value = m.name;
  name.append(nameInput);
  const port = el("label", "", "Port ");
  const portInput = el("input", "");
  portInput.name = "port";
  portInput.type = "number";
  portInput.value = String(m.port);
  port.append(portInput);
  const row = el("div", "network__row");
  row.append(button("Save", "save", () => h.onSave(nameInput.value, Number(portInput.value) || 0)));
  row.append(running ? button("Stop server", "stop", h.onStop) : button("Start server", "start", h.onStart, true));
  phone.append(name, port, row);
  if (m.status.mainError) phone.append(el("p", "settings__error", m.status.mainError));
  if (m.error) phone.append(el("p", "settings__error", m.error));
  root.append(phone);

  if (running) {
    const pairing = section("Pairing");
    const code = m.status.code;
    if (code && nowMs <= code.expiresAt) {
      pairing.append(el("div", "network__code", formatCode(code.code)));
      pairing.append(el("p", "settings__code-expiry", `expires in ${minutesLeft(code.expiresAt, nowMs)} min`));
    }
    pairing.append(button(code && nowMs <= code.expiresAt ? "Regenerate" : "Show pairing code", "regenerate-code", h.onCode));
    pairing.append(el("p", "network__addresses", m.addresses.length > 0 ? `Assistants reach this phone at ${m.addresses.join(", ")}, port ${m.port}.` : "Connect the phone to Wi‑Fi to see its address."));
    root.append(pairing);
  }

  const assistants = section("Assistants");
  if (m.status.assistants.length === 0) {
    assistants.append(el("p", "", "No assistants paired yet."));
  }
  for (const a of m.status.assistants) {
    const r = el("div", "network__assistant");
    const text = el("div", "network__assistant-text");
    text.append(el("div", "network__assistant-name", a.name));
    const platform = PLATFORM_NAMES[a.platform] ?? a.platform;
    const seen = a.connected ? "Connected" : a.lastSeen ? `Last seen ${formatAge(a.lastSeen, nowMs)} ago` : "Never connected";
    text.append(el("div", "network__assistant-meta", [platform, a.address, seen].filter(Boolean).join(" · ")));
    if (a.note) text.append(el("div", "network__assistant-meta", a.note));
    const rm = button("Remove", "remove-assistant", () => h.onRemove(a.id));
    rm.dataset.id = a.id;
    r.append(text, rm);
    assistants.append(r);
  }
  root.append(assistants);

  const notifications = section("Notifications");
  for (const [which, label, on, nameAttr] of [["awaiting", "Notify on Awaiting Decision", m.notifyAwaiting, "notifyAwaiting"], ["completed", "Notify on Completed", m.notifyCompleted, "notifyCompleted"]] as const) {
    const l = el("label", "settings__switch");
    const box = el("input", "");
    box.type = "checkbox";
    box.name = nameAttr;
    box.checked = on;
    box.addEventListener("change", () => h.onSwitch(which, box.checked));
    l.append(box, document.createTextNode(` ${label}`));
    notifications.append(l);
  }
  if (!m.notificationsAllowed) notifications.append(el("p", "settings__error", "Notifications are off for Maya in Android settings."));
  root.append(notifications);

  const battery = section("Battery");
  battery.append(button("Keep Maya awake", "battery", h.onBattery));
  battery.append(el("p", "settings__hint", "Android may otherwise stop the server with the screen off."));
  root.append(battery);

  for (const b of root.querySelectorAll("button")) if (m.busy) b.disabled = true;
  return root;
}

interface ConfigView {
  network: { name: string; port: number };
  notifyOnAwaiting: boolean;
  notifyOnCompleted: boolean;
}

/** Mounts the tab into `#settings`, keeps it current, and reports every status to `onStatus`. */
export function initNetwork(onStatus: (s: NetworkStatus) => void): void {
  const pane = document.getElementById("settings");
  if (!pane) return;
  let config: ConfigView | null = null;
  const model: NetworkModel = {
    status: { role: "off", code: null, assistants: [], assistant: { connected: false, mainName: null, error: null }, mainError: null },
    name: "",
    port: 4127,
    notifyAwaiting: true,
    notifyCompleted: true,
    addresses: [],
    notificationsAllowed: true,
    busy: false,
    error: null,
  };
  const paint = () => pane.replaceChildren(renderNetwork(model, handlers));
  const setStatus = (s: NetworkStatus) => {
    model.status = s;
    onStatus(s);
    paint();
  };
  const saveConfig = async (edit: (c: ConfigView) => void) => {
    const full = await invoke<ConfigView>("get_config");
    edit(full);
    config = await invoke<ConfigView>("set_config", { config: full });
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
  };
  const busy = async (f: () => Promise<void>) => {
    model.busy = true;
    model.error = null;
    paint();
    try {
      await f();
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    paint();
  };
  const handlers: NetworkHandlers = {
    onSave: (name, port) => void busy(async () => {
      await saveConfig((c) => {
        c.network.name = name;
        c.network.port = port;
      });
      showToast("Saved");
    }),
    onStart: () => void busy(async () => setStatus(await invoke<NetworkStatus>("server_start"))),
    onStop: () => void busy(async () => setStatus(await invoke<NetworkStatus>("server_stop"))),
    onCode: () => void busy(async () => setStatus(await invoke<NetworkStatus>("network_pairing_code"))),
    onRemove: (id) => void busy(async () => setStatus(await invoke<NetworkStatus>("network_remove_assistant", { id }))),
    onSwitch: (which, on) => void busy(() => saveConfig((c) => {
      if (which === "awaiting") c.notifyOnAwaiting = on;
      else c.notifyOnCompleted = on;
    })),
    onBattery: () => void invoke("request_battery_exemption").catch((e) => showToast(String(e))),
  };
  void listen<NetworkStatus>("network", (e) => setStatus(e.payload));
  void (async () => {
    config = await invoke<ConfigView>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    model.notifyAwaiting = config.notifyOnAwaiting;
    model.notifyCompleted = config.notifyOnCompleted;
    model.addresses = await invoke<string[]>("local_addresses").catch(() => []);
    model.notificationsAllowed = await invoke<boolean>("notifications_allowed").catch(() => true);
    setStatus(await invoke<NetworkStatus>("network_status"));
  })();
  // The code's countdown and the addresses move without an event.
  setInterval(() => {
    void invoke<string[]>("local_addresses").then((a) => { model.addresses = a; }).catch(() => undefined);
    if (!pane.hidden) paint();
  }, 30_000);
}
```

- [ ] **Step 4: Write `src/mobile/setup.ts`**

```ts
// The overlay over the board while the server is not running: the first
// run's name and port, or "The server is stopped." with Start.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import type { NetworkStatus } from "./network";

export interface SetupModel {
  firstRun: boolean;
  name: string;
  port: number;
  error: string | null;
  busy: boolean;
}

/** Nothing has ever paired and the server is not meant to run: say what Maya is for. */
export function isFirstRun(status: NetworkStatus): boolean {
  return status.role !== "main" && status.assistants.length === 0;
}

export function renderSetup(m: SetupModel, h: { onStart(name: string, port: number): void }): HTMLElement {
  const card = document.createElement("div");
  card.className = "setup__card";
  const title = document.createElement("h2");
  title.textContent = m.firstRun ? "Maya for Android" : "The server is stopped.";
  const blurb = document.createElement("p");
  blurb.textContent = m.firstRun
    ? "Maya is the main for your assistants: start the server, then pair them with the code it shows."
    : "Start it again to see your assistants' sessions.";
  const name = document.createElement("label");
  name.textContent = "Name";
  const nameInput = document.createElement("input");
  nameInput.name = "name";
  nameInput.value = m.name;
  name.append(nameInput);
  const port = document.createElement("label");
  port.textContent = "Port";
  const portInput = document.createElement("input");
  portInput.name = "port";
  portInput.type = "number";
  portInput.value = String(m.port);
  port.append(portInput);
  const start = document.createElement("button");
  start.type = "button";
  start.className = "card__btn card__btn--primary";
  start.dataset.action = "start";
  start.textContent = m.busy ? "Starting…" : "Start";
  start.disabled = m.busy;
  start.addEventListener("click", () => h.onStart(nameInput.value, Number(portInput.value) || 0));
  card.append(title, blurb, name, port, start);
  if (m.error) {
    const err = document.createElement("p");
    err.className = "setup__error";
    err.textContent = m.error;
    card.append(err);
  }
  return card;
}

/** Shows the overlay whenever the server is down and starts it on request. */
export function initSetup(): void {
  const host = document.getElementById("setup");
  if (!host) return;
  const model: SetupModel = { firstRun: true, name: "", port: 4127, error: null, busy: false };
  const paint = (status: NetworkStatus) => {
    host.hidden = status.role === "main";
    model.firstRun = isFirstRun(status);
    model.error = status.mainError;
    host.replaceChildren(renderSetup(model, { onStart: (name, port) => void start(name, port) }));
  };
  const start = async (name: string, port: number) => {
    model.busy = true;
    model.error = null;
    host.replaceChildren(renderSetup(model, { onStart: () => undefined }));
    try {
      const config = await invoke<{ network: { name: string; port: number } }>("get_config");
      config.network.name = name;
      config.network.port = port;
      await invoke("set_config", { config });
      if (!(await isPermissionGranted().catch(() => true))) await requestPermission().catch(() => undefined);
      paint(await invoke<NetworkStatus>("server_start"));
    } catch (e) {
      model.error = String(e);
    }
    model.busy = false;
    if (!host.hidden) host.replaceChildren(renderSetup(model, { onStart: (n, p) => void start(n, p) }));
  };
  void listen<NetworkStatus>("network", (e) => paint(e.payload));
  void (async () => {
    const config = await invoke<{ network: { name: string; port: number } }>("get_config");
    model.name = config.network.name;
    model.port = config.network.port || 4127;
    paint(await invoke<NetworkStatus>("network_status"));
  })();
}
```

- [ ] **Step 5: Run the tests**

Run: `pnpm test -- mobile`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/mobile/network.ts src/mobile/network.test.ts src/mobile/setup.ts src/mobile/setup.test.ts
git commit -m "feat(page): the phone's Network tab and setup overlay"
```

---

### Task 8: Attachments from the picker, notification taps, and a built page

**Files:**
- Modify: `src/modal.ts` (`ModalHandlers` region :28-51, the composer at :518-565, and a new `setComposerOptions` export)
- Create: `src/mobile/notify-tap.ts`, `src/mobile/notify-tap.test.ts`
- Test: `src/modal.test.ts`

**Interfaces:**
- Produces (`src/modal.ts`): `export function setComposerOptions(o: Partial<{ attachButton: boolean }>): void`. With `attachButton`, the composer has `button[data-action=attach]` and a hidden `input[type=file][multiple]` whose files go to `onPasteFiles`, the path pasted files already take.
- Produces (`src/mobile/notify-tap.ts`): `tappedSession(payload: unknown): string | null`, `watchTaps(open: (sessionId: string) => void): Promise<void>`.

- [ ] **Step 1: Write the failing tests**

In `src/modal.test.ts` (it already renders modals with a `handlers()` helper and a base card; follow its local names):

```ts
describe("the Attach button", () => {
  it("is absent by default and, when on, hands picked files to onPasteFiles", () => {
    const h = handlers();
    expect(renderModal(baseModel, h).querySelector("button[data-action=attach]")).toBeNull();
    setComposerOptions({ attachButton: true });
    try {
      const el = renderModal(baseModel, h);
      const attach = el.querySelector<HTMLButtonElement>("button[data-action=attach]")!;
      const pick = el.querySelector<HTMLInputElement>("input[type=file]")!;
      expect(pick.multiple).toBe(true);
      expect(pick.hidden).toBe(true);
      const file = new File(["hi"], "note.txt", { type: "text/plain" });
      Object.defineProperty(pick, "files", { value: [file] });
      pick.dispatchEvent(new Event("change"));
      expect(h.onPasteFiles).toHaveBeenCalledWith([{ name: "note.txt", file }]);
      expect(attach.textContent).toBe("Attach");
    } finally {
      setComposerOptions({ attachButton: false });
    }
  });
});
```

`baseModel` is whatever that file's model fixture for a card with an inbox is called; `handlers()` must include `onPasteFiles: vi.fn()`.

`src/mobile/notify-tap.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { tappedSession } from "./notify-tap";

describe("tappedSession", () => {
  it("reads the session id from either payload shape the plugin has used", () => {
    expect(tappedSession({ id: 7, extra: { sessionId: "abc" } })).toBe("abc");
    expect(tappedSession({ actionId: "tap", notification: { id: 7, extra: { sessionId: "abc" } } })).toBe("abc");
    expect(tappedSession({ id: 7 })).toBeNull();
    expect(tappedSession({ extra: { sessionId: "" } })).toBeNull();
    expect(tappedSession(null)).toBeNull();
    expect(tappedSession("junk")).toBeNull();
  });
});
```

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm test -- modal notify-tap`
Expected: FAIL on the missing export and module.

- [ ] **Step 3: The composer option in `modal.ts`**

Near the top of `modal.ts` (after the imports):

```ts
/** Composer features the page turns on: the phone has a picker instead of drag-and-drop. */
const composer = { attachButton: false };

export function setComposerOptions(o: Partial<typeof composer>): void {
  Object.assign(composer, o);
}
```

In `renderModal`, inside `if (m.card.hasInbox || !claude) {`, replace `form.append(ta, send);` with:

```ts
    if (composer.attachButton) {
      const pick = el("input", "modal__file");
      pick.type = "file";
      pick.multiple = true;
      pick.hidden = true;
      pick.addEventListener("change", () => {
        const files = [...(pick.files ?? [])].map((file) => ({ name: file.name, file }));
        if (files.length > 0) h.onPasteFiles?.(files);
        pick.value = "";
      });
      const attach = el("button", "card__btn modal__attach", "Attach");
      attach.type = "button";
      attach.dataset.action = "attach";
      attach.addEventListener("click", () => pick.click());
      form.append(ta, pick, attach, send);
    } else {
      form.append(ta, send);
    }
```

- [ ] **Step 4: Write `src/mobile/notify-tap.ts`**

```ts
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
```

- [ ] **Step 5: Run the tests, type-check and build the page**

Run: `pnpm test && pnpm build:mobile`
Expected: tests PASS; `tsc` is clean; `dist-mobile/index.html` and its assets exist.

- [ ] **Step 6: Commit**

```bash
git add src/modal.ts src/modal.test.ts src/mobile/notify-tap.ts src/mobile/notify-tap.test.ts
git commit -m "feat(page): attach from a file picker and open a card from its notification"
```

---

### Task 9: The Android project: the foreground service, the plugin, and a debug APK

**Files:**
- Create (generated): `mobile/gen/android/**` via `tauri android init`
- Create: `mobile/gen/android/app/src/main/java/com/dosaki/maya/mobile/KeepAlivePlugin.kt`, `mobile/gen/android/app/src/main/java/com/dosaki/maya/mobile/KeepAliveService.kt`
- Modify: `mobile/gen/android/app/src/main/AndroidManifest.xml`, `mobile/gen/android/app/build.gradle.kts`, `.gitignore`

**Interfaces:**
- Consumes: Task 3's `android.rs` command names (`start`, `update`, `stop`, `requestBatteryExemption`, `notificationsAllowed`, `deviceModel`, `localAddresses`), each returning `{ value }` where it returns anything.
- The service: `KeepAliveService`, foreground type `connectedDevice`, notification id 1 on channel `service`, holding a partial wake lock and a Wi‑Fi lock; `ACTION_STOP` ends it.

- [ ] **Step 1: Generate the project**

With Task 0's environment loaded:

```bash
cd mobile && pnpm exec tauri android init && cd ..
git status --short mobile/gen
```

Expected: `mobile/gen/android/` with `app/`, `buildSrc/`, `build.gradle.kts`, `settings.gradle`, `gradle/`, `gradlew`, and the template's own `.gitignore`. `MainActivity.kt` is under `app/src/main/java/com/dosaki/maya/mobile/`. Append to the repo's `.gitignore`:

```
mobile/gen/android/keystore.properties
*.jks
```

- [ ] **Step 2: The plugin class**

`mobile/gen/android/app/src/main/java/com/dosaki/maya/mobile/KeepAlivePlugin.kt`:

```kotlin
package com.dosaki.maya.mobile

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.net.Inet4Address
import java.net.NetworkInterface

@InvokeArg
class LineArgs {
    var line: String = ""
}

/**
 * What the Rust side cannot do itself: the foreground service that keeps
 * the process alive, the battery exemption dialog, whether Android lets
 * Maya notify, and the device's name and addresses.
 */
@TauriPlugin
class KeepAlivePlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(LineArgs::class.java)
        KeepAliveService.start(activity, args.line)
        invoke.resolve()
    }

    @Command
    fun update(invoke: Invoke) {
        val args = invoke.parseArgs(LineArgs::class.java)
        KeepAliveService.update(activity, args.line)
        invoke.resolve()
    }

    @Command
    fun stop(invoke: Invoke) {
        KeepAliveService.stop(activity)
        invoke.resolve()
    }

    @Command
    fun requestBatteryExemption(invoke: Invoke) {
        val pm = activity.getSystemService(Context.POWER_SERVICE) as PowerManager
        if (!pm.isIgnoringBatteryOptimizations(activity.packageName)) {
            val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:" + activity.packageName))
            activity.startActivity(intent)
        }
        invoke.resolve()
    }

    @Command
    fun notificationsAllowed(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", NotificationManagerCompat.from(activity).areNotificationsEnabled())
        invoke.resolve(ret)
    }

    @Command
    fun deviceModel(invoke: Invoke) {
        val ret = JSObject()
        ret.put("value", Build.MODEL ?: "")
        invoke.resolve(ret)
    }

    @Command
    fun localAddresses(invoke: Invoke) {
        val out = JSArray()
        try {
            for (nic in NetworkInterface.getNetworkInterfaces().toList()) {
                if (!nic.isUp || nic.isLoopback) continue
                for (addr in nic.inetAddresses.toList()) {
                    if (addr is Inet4Address && !addr.isLoopbackAddress) out.put(addr.hostAddress)
                }
            }
        } catch (_: Exception) {
            // no interfaces to list: the page says to connect to Wi‑Fi
        }
        val ret = JSObject()
        ret.put("value", out)
        invoke.resolve(ret)
    }
}
```

- [ ] **Step 3: The service**

`mobile/gen/android/app/src/main/java/com/dosaki/maya/mobile/KeepAliveService.kt`:

```kotlin
package com.dosaki.maya.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Keeps the process, and so the Rust server in it, alive with the screen
 * off: a foreground service with an ongoing notification, a partial wake
 * lock and a Wi‑Fi lock. It runs nothing itself.
 */
class KeepAliveService : Service() {
    private var wake: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            release()
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        val line = intent?.getStringExtra(EXTRA_LINE) ?: ""
        ensureChannel()
        val type = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(line), type)
        acquire()
        return START_STICKY
    }

    override fun onDestroy() {
        release()
        super.onDestroy()
    }

    private fun notification(line: String): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val open = PendingIntent.getActivity(this, 0, launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Maya")
            .setContentText(line)
            .setOngoing(true)
            .setContentIntent(open)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun ensureChannel() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val channel = NotificationChannel(CHANNEL, "Server", NotificationManager.IMPORTANCE_LOW)
        channel.description = "Keeps the main Maya running with the screen off"
        nm.createNotificationChannel(channel)
    }

    private fun acquire() {
        if (wake == null) {
            val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
            wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "maya:server").also { it.acquire() }
        }
        if (wifi == null) {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            @Suppress("DEPRECATION")
            wifi = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "maya:server").also { it.acquire() }
        }
    }

    private fun release() {
        wake?.takeIf { it.isHeld }?.release()
        wake = null
        wifi?.takeIf { it.isHeld }?.release()
        wifi = null
    }

    companion object {
        const val CHANNEL = "service"
        const val NOTIFICATION_ID = 1
        const val ACTION_STOP = "com.dosaki.maya.mobile.STOP"
        const val EXTRA_LINE = "line"

        fun start(ctx: Context, line: String) {
            val intent = Intent(ctx, KeepAliveService::class.java).putExtra(EXTRA_LINE, line)
            if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(intent) else ctx.startService(intent)
        }

        /** Re-posting the notification with a new line; the service is already up. */
        fun update(ctx: Context, line: String) = start(ctx, line)

        fun stop(ctx: Context) {
            ctx.startService(Intent(ctx, KeepAliveService::class.java).setAction(ACTION_STOP))
        }
    }
}
```

- [ ] **Step 4: The manifest and the signing config**

In `mobile/gen/android/app/src/main/AndroidManifest.xml`, after the `INTERNET` permission add:

```xml
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE" />
    <uses-permission android:name="android.permission.CHANGE_WIFI_STATE" />
    <uses-permission android:name="android.permission.ACCESS_WIFI_STATE" />
    <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
    <uses-permission android:name="android.permission.WAKE_LOCK" />
    <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
    <uses-permission android:name="android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS" />
```

and inside `<application>`, after the `<activity>`:

```xml
        <service
            android:name=".KeepAliveService"
            android:exported="false"
            android:foregroundServiceType="connectedDevice" />
```

In `mobile/gen/android/app/build.gradle.kts`: add `import java.io.FileInputStream` at the top, and inside `android { … }` before `buildTypes`:

```kotlin
    // A release APK signed with the keystore in keystore.properties when the
    // file exists (CI writes it from the ANDROID_* secrets), else with the
    // debug key, so a build without secrets still installs on your own phone.
    signingConfigs {
        create("release") {
            val props = rootProject.file("keystore.properties")
            if (props.exists()) {
                val keystoreProperties = Properties().apply { FileInputStream(props).use { load(it) } }
                keyAlias = keystoreProperties["keyAlias"] as String
                keyPassword = keystoreProperties["password"] as String
                storeFile = file(keystoreProperties["storeFile"] as String)
                storePassword = (keystoreProperties["storePassword"] ?: keystoreProperties["password"]) as String
            } else {
                initWith(getByName("debug"))
            }
        }
    }
```

and in `buildTypes { getByName("release") { … } }` add `signingConfig = signingConfigs.getByName("release")`.

- [ ] **Step 5: Build a debug APK and run it**

```bash
pnpm mobile:build -- --debug
ls mobile/gen/android/app/build/outputs/apk/universal/debug/
```

Expected: `app-universal-debug.apk`. Then on the emulator (the AVD `storychat` exists; `$ANDROID_HOME/emulator/emulator -avd storychat &`) or a phone over USB:

```bash
adb install -r mobile/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb logcat -s RustStdoutStderr:V Tauri:V | head -50   # the Rust log lines, "started Maya … for Android"
```

Hand checks, in order:
1. The setup overlay shows the device model as the name; Start asks for notification permission (Android 13+), then the Network tab shows a six-digit code and the persistent "Maya" notification reads "No assistants paired yet".
2. Pair the desktop Maya: Settings › Network › "Assistant to a main Maya", host = the phone's Wi‑Fi address from the Network tab (on the emulator run `adb forward tcp:4127 tcp:4127` and use `127.0.0.1`), the code. The phone's assistants list shows it connected; the service notification reads "Main for 1 assistant, connected"; the board fills.
3. Rotate: one column upright, three or four sideways, the same column staying in view; swipe and tap the strip.
4. Open a card, reply, answer an ask, Attach a file from the picker; Back closes the card.
5. "+" starts a session on the desktop; Resume lists its past sessions.
6. Lock the phone, make a session ask (a permission prompt will do): the notification arrives; tapping it opens the card; answering at the desk clears it.
7. Stop server: the overlay says "The server is stopped.", the service notification goes, and the desktop shows "Reconnecting…" until Start.

If `register_android_plugin` fails at start with a class-not-found error in logcat, the plugin class was not compiled into the app: confirm the file is under `app/src/main/java/com/dosaki/maya/mobile/` and the `namespace` in `build.gradle.kts` is `com.dosaki.maya.mobile`.

- [ ] **Step 6: Commit**

```bash
git add .gitignore mobile/gen
git commit -m "feat(mobile): the Android project, with a foreground service that keeps the server up"
```

---

### Task 10: CI, the release asset, and the docs

**Files:**
- Modify: `.github/workflows/build.yml`, `.github/workflows/release.yml`, `docs/DEVELOPING.md`, `README.md`, `docs/superpowers/specs/2026-10-07-maya-android-companion-design.md`

- [ ] **Step 1: The `android` job**

In `build.yml`, under `on.workflow_call.secrets` add four optional secrets:

```yaml
      ANDROID_KEYSTORE:
        required: false
      ANDROID_KEYSTORE_PASSWORD:
        required: false
      ANDROID_KEY_ALIAS:
        required: false
      ANDROID_KEY_PASSWORD:
        required: false
```

and update the header comment ("Tests and builds Maya on Linux, Windows, macOS and Android…"). Add the job after `macos`:

```yaml
  # The phone's app: the host-side tests (the hub against a core client on
  # localhost, the planner, the page), then an arm64 APK. Signed with the
  # keystore from the ANDROID_* secrets on a release; debug-signed
  # otherwise, which still installs on your own phone.
  android:
    name: Android
    runs-on: ubuntu-latest
    env:
      NDK_VERSION: 27.2.12479018
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 11
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
      - uses: actions/setup-java@v4
        with:
          distribution: temurin
          java-version: 17
      - uses: android-actions/setup-android@v3
        with:
          packages: platform-tools platforms;android-35 build-tools;35.0.0 ndk;27.2.12479018
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-linux-android
      - uses: swatinem/rust-cache@v2
        with:
          workspaces: .
          key: android
      - name: Install the desktop libraries the host-side tests link against
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
      - name: Test
        if: inputs.test
        run: |
          pnpm test
          cargo test -p maya-mobile
      - name: Signing (releases with the secrets set)
        if: inputs.sign
        env:
          ANDROID_KEYSTORE: ${{ secrets.ANDROID_KEYSTORE }}
          ANDROID_KEYSTORE_PASSWORD: ${{ secrets.ANDROID_KEYSTORE_PASSWORD }}
          ANDROID_KEY_ALIAS: ${{ secrets.ANDROID_KEY_ALIAS }}
          ANDROID_KEY_PASSWORD: ${{ secrets.ANDROID_KEY_PASSWORD }}
        run: |
          set -euo pipefail
          if [ -z "${ANDROID_KEYSTORE:-}" ] || [ -z "${ANDROID_KEY_ALIAS:-}" ]; then
            echo "no signing secrets; the APK will be debug-signed"
            exit 0
          fi
          echo "$ANDROID_KEYSTORE" | base64 --decode > "$RUNNER_TEMP/maya.jks"
          {
            echo "keyAlias=$ANDROID_KEY_ALIAS"
            echo "password=$ANDROID_KEY_PASSWORD"
            echo "storePassword=$ANDROID_KEYSTORE_PASSWORD"
            echo "storeFile=$RUNNER_TEMP/maya.jks"
          } > mobile/gen/android/keystore.properties
          echo "signing with the release keystore"
      - name: Build the APK
        env:
          NDK_HOME: ${{ env.ANDROID_HOME }}/ndk/27.2.12479018
        run: pnpm mobile:build
      - name: Package
        run: |
          set -euo pipefail
          version="$(node -p "require('./mobile/tauri.conf.json').version")"
          mkdir -p release
          apk="$(ls mobile/gen/android/app/build/outputs/apk/*/release/*.apk | head -n 1)"
          cp "$apk" "release/Maya_${version}.apk"
          ls -la release
      - uses: actions/upload-artifact@v4
        with:
          name: maya-android
          path: release/*
          if-no-files-found: error
```

`android-actions/setup-android` exports `ANDROID_HOME`; if the APK step cannot find the NDK, print `ls "$ANDROID_HOME/ndk"` and match the version. The `release` job in `release.yml` already downloads every `maya-*` artifact; extend its `--notes` line with: "The Android APK is debug-signed unless the release keystore secrets are set; allow installs from your browser to install it."

- [ ] **Step 2: Push the branch and watch CI**

```bash
git push -u origin feat/android-companion
gh pr create --draft --title "feat: Maya for Android, a main Maya on the phone" --body "Draft while CI settles."
gh run watch
```

Expected: the `Android` job green beside the others, `maya-android` among the run's artifacts. Fix what fails before going on.

- [ ] **Step 3: `docs/DEVELOPING.md`**

Add an `## Android` section after `## Linux`:

```markdown
## Android

The phone app is a second Tauri crate, `mobile/`, that depends on
`maya_core` and reuses the frontend's board, card, modal and dialogs from
`mobile/web/index.html` through `src/mobile/main.ts`. It is only ever a
main: it runs no sessions, so it needs no sidecar and no `jq`.

Requirements: `rustup` (Homebrew's `rust` has no Android targets), Java 17
or later, and the Android SDK command-line tools (`brew install
--cask android-commandlinetools`, or Android Studio's SDK Manager) with:

    sdkmanager "platform-tools" "platforms;android-35" "build-tools;35.0.0" "ndk;27.2.12479018"
    rustup target add aarch64-linux-android x86_64-linux-android
    export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools   # or ~/Library/Android/sdk
    export NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"

Then:

    pnpm install
    pnpm mobile:dev            # on the connected phone or the running emulator
    pnpm mobile:build          # a release APK, debug-signed without a keystore
    pnpm mobile:build -- --debug
    pnpm mobile:desktop        # the phone's app in a desktop window, for quick page work
    pnpm test                  # the page, src/mobile included
    cargo test -p maya-mobile  # the hub against a core client on localhost

The APK lands in `mobile/gen/android/app/build/outputs/apk/universal/`.
`adb logcat -s RustStdoutStderr:V` shows the Rust log lines. On the
emulator the phone's address is not reachable from the Mac: `adb forward
tcp:4127 tcp:4127` and pair the desktop with `127.0.0.1`.

What is where:

- **`mobile/src/hub.rs`** is the app with the page and Android behind
  traits: the config, the server handle, the notifier diff and the alert
  state. `mobile/tests/round_trip.rs` drives it with a real core client.
- **`mobile/src/commands.rs`** is the page's commands, by the desktop's
  names, every session command routed with `merge::route(&[], …)`.
- **`mobile/src/alerts.rs`** decides which notifications a board change
  posts and clears; **`mobile/src/android.rs`** is the `keepalive` plugin
  bridge and the Android notifications, with host no-ops.
- **`mobile/gen/android/`** is the generated project, committed, plus
  `KeepAliveService.kt` (the foreground service) and `KeepAlivePlugin.kt`.
  Everything else in it is Tauri's; regenerate with `tauri android init`
  only if you must, then restore those two files, the manifest's
  permissions and service, and `build.gradle.kts`'s signing block.
- The Android job of the build workflow (see [CI](#ci)) runs the tests
  and builds the APK; `release.yml` attaches it as `Maya_<version>.apk`.
  With `ANDROID_KEYSTORE` (a `.jks` as base64), `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` set as repository secrets
  the APK is release-signed; without them it is debug-signed.
```

In `### Layout`, add `mobile/` to the workspace list and these entries:

```markdown
- `mobile/` — `maya-mobile`, the Android app: `hub.rs`, `commands.rs`,
  `alerts.rs`, `android.rs`, `tests/round_trip.rs`, and `gen/android/`.
- `src/mobile/` — the phone's page: `main.ts`, `strip.ts` (the column
  strip and the one-to-four-column layout), `network.ts`, `setup.ts`,
  `notify-tap.ts`, `mobile.css`; `src/machines.ts` is the dialogs'
  machine switch both pages share.
```

In `## CI`, add Android to the first bullet's list of what `build.yml` builds and the `maya-android` artifact. In `## Releasing`, note `mobile/tauri.conf.json` among the version files.

- [ ] **Step 4: `README.md`**

In "What it does", add: "- Comes as an Android app too: a main Maya in your pocket that shows every assistant's sessions and notifies you when one needs you." In Requirements add "- Android 8 or later, for the phone app." Add a `### Android` platform section after Linux:

```markdown
### Android

Download `Maya_<version>.apk` from the release and open it on the phone;
Android asks to allow installs from your browser once (the builds are not
on the Play Store). The phone is a **main** Maya and nothing else: it runs
no agents and shows the sessions of the assistants paired with it.

1. Open Maya and press **Start**. It asks to post notifications; allow it.
   The Network tab shows a six-digit pairing code and the phone's Wi‑Fi
   address.
2. On each computer, open Maya's Settings › Network, choose "Assistant to
   a main Maya", type the phone's address, port 4127 and the code, and
   Pair. Its sessions appear on the phone's board.
3. Upright, the board shows one column at a time: swipe, or tap the
   strip at the top. Sideways it shows up to four.

With the screen off Maya keeps running behind a persistent notification
("Main for 2 assistants, 1 connected"). Android may still stop it on some
phones: press **Keep Maya awake** on the Network tab to exempt it from
battery optimisation. If the notification disappears, open Maya again and
the assistants reconnect on their own. A new Wi‑Fi address has to be
retyped on the assistants.

Each ask and each finished turn is a notification you can turn off on the
Network tab, or silence per channel in Android's settings; tapping one
opens the card. There is no voice on the phone, and no Pull Requests tab.

Hand checks for a new build, which CI cannot run: pair a computer, rotate
between one and four columns, answer a decision from the phone, lock the
phone and receive a notification, start a session on the computer from
the phone, and stop and start the server and watch the computer reconnect.
```

- [ ] **Step 5: Bring the spec in line with what was built**

In the spec's Components section: replace the `AppState` sentence under `mobile/src/lib.rs` with the `hub.rs` description (the hub behind `Sink` and `Alerts`, tested by `tests/round_trip.rs`); drop `tauri-plugin-dialog` from the dependency list and say the picker is the WebView's own `<input type="file">` behind the modal's Attach button; in the Core section add `net::routing` ("the main-side helpers the desktop had, now shared"); under Frontend list `src/machines.ts`. Set `Status: implemented`.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/build.yml .github/workflows/release.yml docs/DEVELOPING.md README.md docs/superpowers/specs/2026-10-07-maya-android-companion-design.md
git commit -m "docs: building, testing and installing Maya for Android"
```

---

### Task 11: Version bump and the pull request

- [ ] **Step 1: Bump from `main`'s version**

```bash
git fetch origin main
main_version="$(git show origin/main:Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)"
echo "main is at $main_version"
```

This pull request adds a feature, so the minor version: for `0.12.0` that is `0.13.0`. If `main` moved to `0.13.x` meanwhile, use `0.14.0`.

```bash
sh scripts/set-version.sh 0.13.0
git add package.json src-tauri/tauri.conf.json mobile/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.13.0"
```

- [ ] **Step 2: Final checks**

```bash
sh scripts/release-version.sh   # needs GH_TOKEN; prints version= and release=true
pnpm test && pnpm build && pnpm build:mobile
cargo test -p maya-core -p maya-cli -p maya-mobile
```

Expected: all green.

- [ ] **Step 3: Mark the pull request ready**

```bash
git push
gh pr ready
gh pr edit --title "feat: Maya for Android, a main Maya on the phone" --body "$(cat <<'BODY'
## Summary

- A new Tauri crate `mobile/` that is a main Maya and nothing else: it runs the existing server, merges the paired assistants' boards, and routes every card action to the assistant that lists the session.
- The page reuses the desktop's board, cards, modal and dialogs, laid out one column upright and up to four sideways, with a strip to move between them.
- A Kotlin foreground service keeps the server up with the screen off; each ask and finished turn is an Android notification that opens the card.
- `maya_core` gains `net::routing` (the main-side helpers the desktop had) and a `notify_on_completed` switch; the dialogs gain a mode for a machine with no sessions of its own.
- CI builds `Maya_<version>.apk` on every pull request and release, release-signed when the `ANDROID_*` secrets are set.

Spec: `docs/superpowers/specs/2026-10-07-maya-android-companion-design.md`. Plan: `docs/superpowers/plans/2026-10-07-maya-android-companion.md`.

## Test plan

- [ ] `pnpm test`, `cargo test -p maya-core -p maya-cli -p maya-mobile`, CI green on all five jobs
- [ ] On a phone: pair the desktop, rotate, answer a decision, receive a notification with the screen locked and open the card from it, start a session on the desktop from the phone, stop and start the server

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
```

- [ ] **Step 4: Done**

Merging publishes `v0.13.0` with the APK beside the desktop builds. The spec's Out of scope lists what comes next: speech, the iPhone's remote-of-desktop mode.
