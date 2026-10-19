# Resume sessions after a reboot, and a terminal look for the viewer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each Maya remembers the agent sessions running on its machine and, after a restart, offers them back (a Sessions banner on the desktop and the phone, and "Maya, resume my sessions"), the main covering its assistants; the session viewer gains a terminal look; the README gains the Settings → Git first-run step.

**Architecture:** A pure `core/src/remembered.rs` (`Memory`) keeps `~/.claude/maya/remembered.json`: the store feeds it every refresh (`observe`), the close path (`closed`) and a 30 s heartbeat, and classifies the previous run's file at start. `actions::resume_remembered` reopens entries in their own folders. The network carries each assistant's "was running" list in `Up::Board`, the main merges it (`merge::remembered`) and mirrors it to phones in `Down::Mirror`; Resume uses the existing `CommandKind::Resume` (an absolute remembered folder), Forget a new `CommandKind::Forget`. The page gets a banner module (`src/remembered.ts`), the voice a confirmed `resume_all` action, and the viewer one renderer whose root takes `modal--terminal`, with CSS drawing both looks.

**Tech Stack:** Rust (Tauri 2, serde), vanilla TypeScript (vitest, jsdom).

**Spec:** `docs/superpowers/specs/2026-10-19-resume-sessions-and-terminal-theme-design.md`

## Global Constraints

- Branch `feat/resume-sessions-and-terminal-theme` (from `main` at 0.24.0). Never commit to or push `main`; never push without asking first.
- Commit messages: Conventional Commits, plain `type: description` (no work-item codes), ending with a `Co-Authored-By:` trailer naming the model that implements the task, e.g. `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- No new crates and no new npm packages.
- Unix-only tests are gated with `#[cfg(unix)]`; paths are compared as `Path`/`PathBuf` (by component), never as strings (Windows CI).
- Checks, from the repo root: `cargo test --workspace`, `pnpm test`, `pnpm exec tsc --noEmit`. `pnpm ear:build` must have run once (Tauri bundles the sidecar).
- `GRACE_MS = 120_000`; the heartbeat is written every 30 s (`HEARTBEAT_MS = 30_000`) on the PR-badge loop's thread, which ticks every 10 s (every third tick); resumed terminals open 400 ms apart (`RESUME_GAP_MS = 400`), oldest `lastSeenMs` first.
- The file is `~/.claude/maya/remembered.json` (`{"heartbeatMs": …, "sessions": [{agent, sessionId, dir, name, firstSeenMs, lastSeenMs, goneAtMs}]}`), written with `config::write_private` (0600), only when something changed, at most once per refresh. Only Claude Code, Codex, Antigravity, Grok, Kiro and OpenCode sessions with a `resume::plain_session_id` are recorded; `Other` never.
- Remembering is opt-in per store (`Store::start_remembering`): the desktop app turns it on; the CLI and tests that do not call it write nothing.
- Banner: "N sessions were running when Maya last stopped", plus " · a here · b on <assistant>" when more than one machine is involved; buttons **Resume all**, **Choose…**, **Dismiss**; Choose: **Resume N**, **Cancel**, **Forget unticked**; a waiting row says "waiting for <name>"; failures "Couldn't resume 1: raffinad-3c (folder missing)" with Dismiss.
- Failure reasons (exact): `folder missing`, `the agent no longer has that session`, `<Agent> is not installed` (e.g. `Codex is not installed`); a single failure toasts "Couldn't resume <name>: <reason>".
- Voice: action `{"kind":"resume_all","scope":"all"|"here"}`; read-back "Resume 5 sessions, 3 here and 2 on Gnowee?" / "Resume 3 sessions?" / "There's nothing to resume."; after yes "Resuming 5." or "Resumed 4; one couldn't be resumed.".
- Viewer: `Config.viewer_theme` (`"chat"` | `"terminal"`, default chat); Settings → Sessions "Session viewer" with Chat / Terminal; header toggle titled "Terminal look" / "Chat look"; terminal look: `ui-monospace, SFMono-Regular, Menlo, monospace`, background `#0a0a0a`, text `#e8e2d4`, `>` for the user in Maya's gold, `⏺` for agent and tools, `⎿` for a tool result, the composer as a `>` prompt with an ⏎ button.
- Release: two `feat`s ship as 0.25.0: `sh scripts/set-version.sh 0.25.0`, committed on its own as `chore(release): 0.25.0`; check `Cargo.lock` changed.

## Review Focus

1. The clock moved backwards across the reboot (NTP corrects it after boot, so the stored heartbeat or a `goneAtMs` is later than "now"): nothing underflows, panics or is dropped; those sessions are still offered (Task 1 `a_clock_that_moved_backwards_neither_panics_nor_drops_sessions`).
2. Only Maya was quit and reopened while the sessions kept running: once the first refresh sees them live they are not offered (Task 2 `sessions_that_outlived_a_restart_of_maya_alone_are_not_offered`).
3. A remembered folder with a space and a quote (`…/it's mine`) resumes with the folder quoted in the shell line; one that is now a file, or a relative path, is "folder missing" and never reaches a shell (Task 3 `a_folder_with_a_quote_is_quoted_and_a_file_or_relative_path_is_missing`).
4. A `remembered.json` cut off by a power cut mid-write, or a directory in its place, reads as empty, never blocks startup, and the next save writes a good file (Task 1 `an_unreadable_or_cut_off_file_reads_as_empty_and_the_next_save_writes_a_good_one`).
5. The same session id remembered on two machines (a laptop restored from the Mac's backup): the Choose list ticks, resumes and forgets each one separately (Task 7 `the same session id on two machines is ticked and resumed separately`).

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/remembered.rs` (new) | The record, `Memory` (load/save/observe/closed/classify/forget/failed/heartbeat), `RememberedRow`, `ResumeOutcome`, `plan`, `keys_by_machine`, `heartbeat_due`, the constants. |
| `core/src/lib.rs` | `pub mod remembered;` |
| `core/src/store.rs` | Holds `Option<Memory>`; `start_remembering`, observe on refresh, accessors. |
| `core/src/actions.rs` | `closed` on Close; `resume_remembered`, `resume_relayed`, the failure reasons. |
| `core/src/net/protocol.rs` | `Up::Board.remembered`, `Down::Mirror.remembered`, `CommandKind::Forget`. |
| `core/src/net/merge.rs` | `RemoteBoard.remembered`, `merge::remembered`. |
| `core/src/net/server.rs` | Keeps each assistant's list, mirrors the merged rows, `drop_remembered`, `kind_name`. |
| `core/src/net/client.rs` | `Executor::remembered`, `ClientNotify::mirrored_remembered`. |
| `core/src/net/routing.rs` | `target_of(Forget)`. |
| `core/src/interpreter.rs` | `resume_all` validation, read-back, said, prompt, `needs_confirm`. |
| `core/src/config.rs` | `ViewerTheme`, `Config.viewer_theme`. |
| `cli/src/executor.rs` | `Forget` arm; `Resume` through `resume_relayed`. |
| `src-tauri/src/remembered_app.rs` (new) | Rows, publish, Resume/Forget across machines, heartbeat, the Tauri commands. |
| `src-tauri/src/lib.rs` | Module, commands, start at setup, heartbeat on the PR loop, publish on refresh. |
| `src-tauri/src/net_app.rs` | `execute` Forget/Resume, `TauriExecutor::remembered`, `revalidate` for `resume_all`. |
| `src-tauri/src/listener.rs` | `VoiceContext.remembered`, `decide`, `execute_action`, `spoken_for`. |
| `mobile/src/hub.rs`, `mobile/src/lib.rs`, `mobile/src/commands.rs` | The phone's rows, Resume/Forget, the `remembered` event, `apply_settings` carries the look. |
| `src/types.ts` | `Remembered`, `RememberedRow`, `ResumeOutcome`, `ViewerTheme`. |
| `src/remembered.ts` (new) | Banner text, render, Choose list, toast text, `initRemembered`. |
| `src/main.ts`, `index.html`, `src/mobile/main.ts`, `mobile/web/index.html` | Mount the banner above the board. |
| `src/modal.ts` | Markers, ⎿ line, prompt, theme toggle, `setViewerTheme`, `saveViewerTheme`. |
| `src/styles.css` | Banner and terminal-look rules. |
| `src/settings.ts`, `src/mobile/network.ts` | "Session viewer" Chat / Terminal. |
| `README.md` | First-run Git step, two feature lines. |

---

### Task 1: Remembered sessions — the core logic

**Files:**
- Create: `core/src/remembered.rs`
- Modify: `core/src/lib.rs` (add `pub mod remembered;` after `pub mod registry;`)

**Interfaces:**
- Consumes: `crate::model::{Card, Harness}`, `crate::resume::plain_session_id(&str) -> bool`, `crate::config::write_private(&Path, &[u8]) -> io::Result<()>`, `crate::log::line(&str, impl Into<String>)`.
- Produces:
  - `pub const GRACE_MS: u64 = 120_000; pub const HEARTBEAT_MS: u64 = 30_000; pub const RESUME_GAP_MS: u64 = 400;`
  - `pub type Key = (Harness, String);`
  - `pub fn path_in(claude_dir: &Path) -> PathBuf` (`<claude>/maya/remembered.json`)
  - `pub struct Remembered { agent: Harness, session_id: String, dir: String, name: String, first_seen_ms: u64, last_seen_ms: u64, gone_at_ms: Option<u64>, offered: bool, error: Option<String> }` (all `pub`, serde camelCase) with `fn key(&self) -> Key`
  - `pub struct LiveSession { agent, session_id, dir, name }` with `fn of(&Card) -> LiveSession`
  - `pub struct Memory` with `load(PathBuf) -> Memory`, `save(&mut self) -> Result<(), String>`, `classify_at_start(&mut self, now_ms: u64) -> Vec<Remembered>`, `observe(&mut self, &[LiveSession], now_ms: u64)`, `closed(&mut self, Harness, &str)`, `forget(&mut self, &[Key])`, `failed(&mut self, &Key, &str)`, `heartbeat(&mut self, now_ms: u64)`, `was_running(&self) -> Vec<Remembered>` (offered, oldest `last_seen_ms` first), `entry(&self, Harness, &str) -> Option<Remembered>`, `heartbeat_ms(&self) -> u64`, `sessions(&self) -> &[Remembered]`
  - `pub struct RememberedRow { machine: Option<String>, waiting: bool, #[serde(flatten)] entry: Remembered }`
  - `pub struct ResumeOutcome { machine: Option<String>, agent: Harness, session_id: String, name: String, error: Option<String> }` with `fn of(Option<&str>, &Remembered, &Result<(), String>) -> ResumeOutcome`
  - `pub struct ResumePlan { here: Vec<Remembered>, remote: Vec<(String, Vec<Remembered>)>, waiting: Vec<RememberedRow> }`, `pub fn plan(&[RememberedRow]) -> ResumePlan`
  - `pub fn keys_by_machine(&[RememberedRow]) -> Vec<(Option<String>, bool, Vec<Key>)>`
  - `pub fn waiting_for(machine: &str) -> String` ("waiting for <machine>")
  - `pub fn heartbeat_due(tick: u64, tick_ms: u64) -> bool`

Design notes (decided here, the spec leaves them open): an entry carries `offered` (it was running when the last run stopped; offered until resumed, forgotten or seen live) and `error` (the last resume's failure), both persisted so a second restart before the user acts still offers them. `lastSeenMs` moves in memory on every refresh but does not by itself mark the file changed; the heartbeat's write carries it (30 s accuracy is enough for "N ago" and the order).

- [ ] **Step 1: Write the failing tests**

Create `core/src/remembered.rs` with only the tests module for now (the next step adds the code above it):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Harness;

    fn live(id: &str) -> LiveSession {
        LiveSession { agent: Harness::ClaudeCode, session_id: id.into(), dir: format!("/Users/x/dev/{id}"), name: format!("{id}-name") }
    }

    fn entry(id: &str) -> Remembered {
        Remembered { agent: Harness::ClaudeCode, session_id: id.into(), dir: format!("/Users/x/dev/{id}"), name: format!("{id}-name"), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None }
    }

    fn memory() -> (tempfile::TempDir, Memory) {
        let t = tempfile::tempdir().unwrap();
        let m = Memory::load(path_in(t.path()));
        (t, m)
    }

    fn ids(list: &[Remembered]) -> Vec<&str> {
        list.iter().map(|e| e.session_id.as_str()).collect()
    }

    #[test]
    fn a_session_is_added_on_first_sight_and_follows_its_name_and_folder() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 1_000);
        let a = m.entry(Harness::ClaudeCode, "a").unwrap();
        assert_eq!((a.first_seen_ms, a.last_seen_ms, a.gone_at_ms, a.offered), (1_000, 1_000, None, false));
        let moved = LiveSession { dir: "/Users/x/dev/two".into(), name: "renamed".into(), ..live("a") };
        m.observe(&[moved], 6_000);
        let a = m.entry(Harness::ClaudeCode, "a").unwrap();
        assert_eq!((a.dir.as_str(), a.name.as_str(), a.first_seen_ms, a.last_seen_ms), ("/Users/x/dev/two", "renamed", 1_000, 6_000));
    }

    #[test]
    fn other_agents_odd_ids_and_duplicates_are_never_recorded_twice_or_at_all() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        let other = LiveSession { agent: Harness::Other, ..live("o1") };
        m.observe(&[other, live("../etc"), live("ok"), live("ok")], 2);
        assert_eq!(ids(m.sessions()), ["ok"]);
    }

    #[test]
    fn a_vanished_session_is_marked_gone_comes_back_and_is_dropped_after_the_grace() {
        let (_t, mut m) = memory();
        m.classify_at_start(1_000);
        m.observe(&[live("a"), live("b")], 2_000);
        m.observe(&[live("a")], 10_000);
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().gone_at_ms, Some(10_000));
        m.observe(&[live("a"), live("b")], 20_000);
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().gone_at_ms, None, "back: live again");
        m.observe(&[live("a")], 30_000);
        m.observe(&[live("a")], 30_000 + GRACE_MS);
        assert!(m.entry(Harness::ClaudeCode, "b").is_some(), "not past the grace yet");
        m.observe(&[live("a")], 30_001 + GRACE_MS);
        assert!(m.entry(Harness::ClaudeCode, "b").is_none(), "gone for longer than the grace while Maya ran: the user closed it");
        assert!(m.was_running().is_empty());
    }

    #[test]
    fn a_session_closed_from_its_card_is_dropped_at_once() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a"), live("b")], 2);
        m.closed(Harness::ClaudeCode, "a");
        assert_eq!(ids(m.sessions()), ["b"]);
    }

    #[test]
    fn at_start_what_was_live_or_gone_within_the_grace_of_the_last_heartbeat_was_running() {
        let t = tempfile::tempdir().unwrap();
        let path = path_in(t.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let record = serde_json::json!({
            "heartbeatMs": 1_000_000,
            "sessions": [
                { "agent": "claude-code", "sessionId": "live", "dir": "/x", "name": "live", "firstSeenMs": 1, "lastSeenMs": 900_000, "goneAtMs": null },
                { "agent": "codex", "sessionId": "late", "dir": "/x", "name": "late", "firstSeenMs": 1, "lastSeenMs": 950_000, "goneAtMs": 950_000 },
                { "agent": "grok", "sessionId": "old", "dir": "/x", "name": "old", "firstSeenMs": 1, "lastSeenMs": 700_000, "goneAtMs": 700_000 },
                { "agent": "kiro", "sessionId": "before", "dir": "/x", "name": "before", "firstSeenMs": 1, "lastSeenMs": 100, "goneAtMs": null, "offered": true },
                { "agent": "future-agent", "sessionId": "f", "dir": "/x", "name": "f", "firstSeenMs": 1, "lastSeenMs": 5 }
            ]
        });
        std::fs::write(&path, record.to_string()).unwrap();
        let mut m = Memory::load(path);
        let offered = m.classify_at_start(2_000_000);
        assert_eq!(ids(&offered), ["before", "live", "late"], "oldest last-seen first; the long-gone one and the unknown agent are dropped");
        assert!(offered.iter().all(|e| e.offered && e.gone_at_ms.is_none()));
        assert_eq!(m.heartbeat_ms(), 2_000_000);
    }

    #[test]
    fn an_offered_session_seen_live_again_is_no_longer_offered() {
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 2);
        m.heartbeat(3);
        m.save().unwrap();
        // The next run, over the file this one left.
        let mut next = Memory::load(path_in(t.path()));
        assert_eq!(ids(&next.classify_at_start(10)), ["a"]);
        next.observe(&[], 11);
        assert_eq!(ids(&next.was_running()), ["a"], "still offered while it is not running");
        next.observe(&[live("a")], 12);
        assert!(next.was_running().is_empty(), "resumed by hand: it leaves the list on its own");
        assert_eq!(next.entry(Harness::ClaudeCode, "a").unwrap().gone_at_ms, None);
    }

    #[test]
    fn a_clock_that_moved_backwards_neither_panics_nor_drops_sessions() {
        let t = tempfile::tempdir().unwrap();
        let path = path_in(t.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The heartbeat is older than the moment the session went: the clock was put back.
        let record = serde_json::json!({ "heartbeatMs": 1_000, "sessions": [
            { "agent": "claude-code", "sessionId": "a", "dir": "/x", "name": "a", "firstSeenMs": 1, "lastSeenMs": 5_000, "goneAtMs": 5_000 }
        ]});
        std::fs::write(&path, record.to_string()).unwrap();
        let mut m = Memory::load(path);
        assert_eq!(ids(&m.classify_at_start(500)), ["a"], "now is before everything stored");
        m.observe(&[live("b")], 400);
        m.observe(&[], 300);
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().gone_at_ms, Some(300));
        m.observe(&[], 100);
        assert!(m.entry(Harness::ClaudeCode, "b").is_some(), "time going backwards is not time gone");
        assert_eq!(ids(&m.was_running()), ["a"]);
    }

    #[test]
    fn the_file_round_trips_and_is_written_only_when_something_changed() {
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a")], 2);
        m.save().unwrap();
        let path = path_in(t.path());
        let back = Memory::load(path.clone());
        assert_eq!(back.sessions(), m.sessions());
        assert_eq!(back.heartbeat_ms(), 1);
        std::fs::remove_file(&path).unwrap();
        m.observe(&[live("a")], 3);
        m.save().unwrap();
        assert!(!path.exists(), "only lastSeen moved: nothing written");
        assert_eq!(m.entry(Harness::ClaudeCode, "a").unwrap().last_seen_ms, 3);
        m.heartbeat(30_000);
        m.save().unwrap();
        let back = Memory::load(path);
        assert_eq!((back.heartbeat_ms(), back.entry(Harness::ClaudeCode, "a").unwrap().last_seen_ms), (30_000, 3), "the heartbeat's write carries lastSeen");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (t, mut m) = memory();
        m.classify_at_start(1);
        m.save().unwrap();
        assert_eq!(std::fs::metadata(path_in(t.path())).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn an_unreadable_or_cut_off_file_reads_as_empty_and_the_next_save_writes_a_good_one() {
        let t = tempfile::tempdir().unwrap();
        let path = path_in(t.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"heartbeatMs":123,"sessions":[{"agent":"claude-code","sessionId":"a","di"#).unwrap();
        let mut m = Memory::load(path.clone());
        assert!(m.sessions().is_empty());
        assert!(m.classify_at_start(5).is_empty());
        m.save().unwrap();
        assert_eq!(Memory::load(path.clone()).heartbeat_ms(), 5, "a good file replaced the cut-off one");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(Memory::load(path).sessions().is_empty(), "a directory in its place reads as empty too");
    }

    #[test]
    fn forget_and_failed_change_only_the_named_sessions() {
        let (_t, mut m) = memory();
        m.classify_at_start(1);
        m.observe(&[live("a"), live("b")], 2);
        m.failed(&(Harness::ClaudeCode, "b".into()), "folder missing");
        m.failed(&(Harness::Codex, "b".into()), "not this one");
        assert_eq!(m.entry(Harness::ClaudeCode, "b").unwrap().error.as_deref(), Some("folder missing"));
        m.forget(&[(Harness::ClaudeCode, "a".into()), (Harness::Codex, "b".into())]);
        assert_eq!(ids(m.sessions()), ["b"]);
    }

    #[test]
    fn a_resume_plan_splits_rows_by_machine_oldest_first() {
        let row = |m: Option<&str>, waiting: bool, id: &str, seen: u64| RememberedRow { machine: m.map(String::from), waiting, entry: Remembered { last_seen_ms: seen, ..entry(id) } };
        let p = plan(&[row(None, false, "l2", 20), row(Some("Gnowee"), false, "g1", 30), row(None, false, "l1", 10), row(Some("pi"), true, "p1", 5), row(Some("Gnowee"), false, "g0", 1)]);
        assert_eq!(ids(&p.here), ["l1", "l2"]);
        assert_eq!(p.remote.len(), 1);
        assert_eq!((p.remote[0].0.as_str(), ids(&p.remote[0].1)), ("Gnowee", vec!["g0", "g1"]));
        assert_eq!(p.waiting.iter().map(|r| r.entry.session_id.as_str()).collect::<Vec<_>>(), ["p1"]);
        assert_eq!(waiting_for("pi"), "waiting for pi");
    }

    #[test]
    fn keys_are_grouped_by_machine_with_whether_it_is_away() {
        let row = |m: Option<&str>, waiting: bool, id: &str| RememberedRow { machine: m.map(String::from), waiting, entry: entry(id) };
        let groups = keys_by_machine(&[row(None, false, "a"), row(Some("pi"), true, "p"), row(None, false, "b")]);
        assert_eq!(groups, vec![(None, false, vec![(Harness::ClaudeCode, "a".to_string()), (Harness::ClaudeCode, "b".to_string())]), (Some("pi".to_string()), true, vec![(Harness::ClaudeCode, "p".to_string())])]);
    }

    #[test]
    fn a_row_is_the_entry_with_its_machine_beside_it() {
        let r = RememberedRow { machine: Some("Gnowee".into()), waiting: true, entry: entry("g1") };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!((v["machine"].as_str(), v["waiting"].as_bool(), v["sessionId"].as_str(), v["agent"].as_str()), (Some("Gnowee"), Some(true), Some("g1"), Some("claude-code")));
        assert_eq!(serde_json::from_value::<RememberedRow>(v).unwrap(), r);
        let local: RememberedRow = serde_json::from_str(r#"{"agent":"codex","sessionId":"c1","dir":"/x","name":"c","firstSeenMs":1,"lastSeenMs":2}"#).unwrap();
        assert_eq!((local.machine, local.waiting, local.entry.offered), (None, false, false));
        let out = ResumeOutcome::of(Some("pi"), &entry("p"), &Err("folder missing".into()));
        assert_eq!((out.machine.as_deref(), out.name.as_str(), out.error.as_deref()), (Some("pi"), "p-name", Some("folder missing")));
    }

    #[test]
    fn the_heartbeat_is_due_every_thirty_seconds_of_ticks() {
        let due: Vec<u64> = (1..=9).filter(|t| heartbeat_due(*t, 10_000)).collect();
        assert_eq!(due, [3, 6, 9]);
        assert!(!heartbeat_due(0, 10_000));
    }
}
```

- [ ] **Step 2: Add the module and run the tests to see them fail**

Add `pub mod remembered;` after `pub mod registry;` in `core/src/lib.rs`.

Run: `cargo test -p maya-core remembered::`
Expected: FAIL to compile, e.g. "cannot find type `LiveSession` in this scope".

- [ ] **Step 3: Write the implementation**

Put this above the tests module in `core/src/remembered.rs`:

```rust
//! The sessions this machine remembers, so the ones that were running when
//! Maya or the machine stopped can be offered back after a restart. Kept in
//! `~/.claude/maya/remembered.json`. Pure logic: the store feeds it each
//! refresh (`observe`), the close path (`closed`) and the heartbeat.

use crate::model::{Card, Harness};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A session gone this long while Maya ran was closed by the user; one that
/// went within this of the last heartbeat went down with the machine.
pub const GRACE_MS: u64 = 120_000;
/// How often the heartbeat is written while Maya runs.
pub const HEARTBEAT_MS: u64 = 30_000;
/// The pause between two terminals one resume opens.
pub const RESUME_GAP_MS: u64 = 400;

/// What a remembered session is known by: its agent and its id.
pub type Key = (Harness, String);

/// `<claude dir>/maya/remembered.json`.
pub fn path_in(claude_dir: &Path) -> PathBuf {
    claude_dir.join("maya").join("remembered.json")
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A session this machine saw running.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Remembered {
    pub agent: Harness,
    pub session_id: String,
    pub dir: String,
    pub name: String,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    /// When a refresh first missed it; None while it runs.
    #[serde(default)]
    pub gone_at_ms: Option<u64>,
    /// It was running when the last run stopped: offered until resumed,
    /// forgotten, or seen live again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub offered: bool,
    /// Why its last resume failed ("folder missing"…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Remembered {
    pub fn key(&self) -> Key {
        (self.agent, self.session_id.clone())
    }
}

/// A session a refresh saw running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSession {
    pub agent: Harness,
    pub session_id: String,
    pub dir: String,
    pub name: String,
}

impl LiveSession {
    pub fn of(card: &Card) -> LiveSession {
        LiveSession { agent: card.harness, session_id: card.session_id.clone(), dir: card.cwd.clone(), name: card.name.clone() }
    }
}

/// Only a session its agent can resume by a plain id is kept.
fn recordable(s: &LiveSession) -> bool {
    s.agent != Harness::Other && crate::resume::plain_session_id(&s.session_id) && !s.dir.is_empty()
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    heartbeat_ms: u64,
    #[serde(default)]
    sessions: Vec<Remembered>,
}

/// The file and what this run made of it.
pub struct Memory {
    path: PathBuf,
    record: Record,
    /// When this run began; a session gone since before it is not this run's to prune.
    started_ms: u64,
    /// Something changed since the last save.
    dirty: bool,
}

impl Memory {
    /// The file at `path`. Missing reads as empty; unreadable too, logged,
    /// so it never blocks startup.
    pub fn load(path: PathBuf) -> Memory {
        let record = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::log::line("remembered", format!("{} is unreadable, starting empty: {e}", path.display()));
                Record::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Record::default(),
            Err(e) => {
                crate::log::line("remembered", format!("{} is unreadable, starting empty: {e}", path.display()));
                Record::default()
            }
        };
        Memory { path, record, started_ms: 0, dirty: false }
    }

    /// Writes the file, readable by its owner only, when something changed.
    pub fn save(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&self.record).map_err(|e| e.to_string())?;
        crate::config::write_private(&self.path, text.as_bytes()).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    /// At startup, before the first refresh: what the last run left is
    /// judged against its last heartbeat. Still running when it stopped, or
    /// gone within `GRACE_MS` of its last heartbeat: was running, offered.
    /// Gone earlier: the user closed it. Returns what is offered.
    pub fn classify_at_start(&mut self, now_ms: u64) -> Vec<Remembered> {
        let heartbeat = self.record.heartbeat_ms;
        self.record.sessions.retain_mut(|e| {
            if e.agent == Harness::Other {
                return false;
            }
            if e.offered {
                return true;
            }
            let running = match e.gone_at_ms {
                None => true,
                Some(gone) => heartbeat.saturating_sub(gone) <= GRACE_MS,
            };
            if running {
                e.offered = true;
                e.gone_at_ms = None;
                e.error = None;
            }
            running
        });
        self.started_ms = now_ms;
        self.record.heartbeat_ms = now_ms;
        self.dirty = true;
        self.was_running()
    }

    /// One refresh's live sessions: new ones are added, known ones follow
    /// their name and folder (and are no longer offered), missing ones are
    /// marked gone, and one gone longer than `GRACE_MS` while this run
    /// watched is dropped. Offered ones that are not live are left alone.
    pub fn observe(&mut self, live: &[LiveSession], now_ms: u64) {
        let mut seen: Vec<Key> = Vec::new();
        for s in live.iter().filter(|s| recordable(s)) {
            let key = (s.agent, s.session_id.clone());
            if seen.contains(&key) {
                continue;
            }
            seen.push(key.clone());
            match self.record.sessions.iter_mut().find(|e| e.key() == key) {
                Some(e) => {
                    if e.name != s.name || e.dir != s.dir || e.gone_at_ms.is_some() || e.offered || e.error.is_some() {
                        e.name = s.name.clone();
                        e.dir = s.dir.clone();
                        e.gone_at_ms = None;
                        e.offered = false;
                        e.error = None;
                        self.dirty = true;
                    }
                    // Kept in memory: the heartbeat's write carries it.
                    e.last_seen_ms = now_ms;
                }
                None => {
                    self.record.sessions.push(Remembered { agent: s.agent, session_id: s.session_id.clone(), dir: s.dir.clone(), name: s.name.clone(), first_seen_ms: now_ms, last_seen_ms: now_ms, gone_at_ms: None, offered: false, error: None });
                    self.dirty = true;
                }
            }
        }
        let started = self.started_ms;
        let before = self.record.sessions.len();
        let mut marked = false;
        self.record.sessions.retain_mut(|e| {
            if e.offered || seen.contains(&e.key()) {
                return true;
            }
            match e.gone_at_ms {
                None => {
                    e.gone_at_ms = Some(now_ms);
                    marked = true;
                    true
                }
                Some(gone) => !(now_ms.saturating_sub(gone) > GRACE_MS && started <= gone),
            }
        });
        if marked || self.record.sessions.len() != before {
            self.dirty = true;
        }
    }

    /// Closed from its card: forgotten at once.
    pub fn closed(&mut self, agent: Harness, session_id: &str) {
        self.forget(&[(agent, session_id.to_string())]);
    }

    pub fn forget(&mut self, keys: &[Key]) {
        let before = self.record.sessions.len();
        self.record.sessions.retain(|e| !keys.contains(&e.key()));
        if self.record.sessions.len() != before {
            self.dirty = true;
        }
    }

    /// Keeps why its resume failed, until it is dismissed.
    pub fn failed(&mut self, key: &Key, reason: &str) {
        if let Some(e) = self.record.sessions.iter_mut().find(|e| e.key() == *key) {
            e.error = Some(reason.to_string());
            self.dirty = true;
        }
    }

    /// This run's last sign of life.
    pub fn heartbeat(&mut self, now_ms: u64) {
        self.record.heartbeat_ms = now_ms;
        self.dirty = true;
    }

    /// What is offered, oldest `last_seen_ms` first.
    pub fn was_running(&self) -> Vec<Remembered> {
        let mut out: Vec<Remembered> = self.record.sessions.iter().filter(|e| e.offered).cloned().collect();
        out.sort_by_key(|e| e.last_seen_ms);
        out
    }

    pub fn entry(&self, agent: Harness, session_id: &str) -> Option<Remembered> {
        self.record.sessions.iter().find(|e| e.agent == agent && e.session_id == session_id).cloned()
    }

    pub fn heartbeat_ms(&self) -> u64 {
        self.record.heartbeat_ms
    }

    pub fn sessions(&self) -> &[Remembered] {
        &self.record.sessions
    }
}

/// One "was running" entry as the banner lists it: the machine it runs on
/// (None: this one) and whether that machine is away.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RememberedRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default)]
    pub waiting: bool,
    #[serde(flatten)]
    pub entry: Remembered,
}

/// What became of one row of a resume; `error` is its reason.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    pub agent: Harness,
    pub session_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ResumeOutcome {
    pub fn of(machine: Option<&str>, e: &Remembered, result: &Result<(), String>) -> ResumeOutcome {
        ResumeOutcome { machine: machine.map(String::from), agent: e.agent, session_id: e.session_id.clone(), name: e.name.clone(), error: result.as_ref().err().cloned() }
    }
}

/// The rows of one resume by where they run: this machine's, each connected
/// machine's (oldest first in each), and the ones whose machine is away.
#[derive(Debug, Default, PartialEq)]
pub struct ResumePlan {
    pub here: Vec<Remembered>,
    pub remote: Vec<(String, Vec<Remembered>)>,
    pub waiting: Vec<RememberedRow>,
}

pub fn plan(rows: &[RememberedRow]) -> ResumePlan {
    let mut p = ResumePlan::default();
    for r in rows {
        match (&r.machine, r.waiting) {
            (_, true) => p.waiting.push(r.clone()),
            (None, false) => p.here.push(r.entry.clone()),
            (Some(m), false) => match p.remote.iter_mut().find(|(n, _)| n == m) {
                Some((_, list)) => list.push(r.entry.clone()),
                None => p.remote.push((m.clone(), vec![r.entry.clone()])),
            },
        }
    }
    p.here.sort_by_key(|e| e.last_seen_ms);
    for (_, list) in &mut p.remote {
        list.sort_by_key(|e| e.last_seen_ms);
    }
    p
}

/// The rows' keys by machine (None: this one), in first-seen order, with
/// whether that machine is away.
pub fn keys_by_machine(rows: &[RememberedRow]) -> Vec<(Option<String>, bool, Vec<Key>)> {
    let mut out: Vec<(Option<String>, bool, Vec<Key>)> = Vec::new();
    for r in rows {
        match out.iter_mut().find(|(m, _, _)| *m == r.machine) {
            Some((_, waiting, keys)) => {
                *waiting |= r.waiting;
                keys.push(r.entry.key());
            }
            None => out.push((r.machine.clone(), r.waiting, vec![r.entry.key()])),
        }
    }
    out
}

/// What a row whose machine is away says, and why Resume all skipped it.
pub fn waiting_for(machine: &str) -> String {
    format!("waiting for {machine}")
}

/// Whether the `tick`th turn (from 1) of a loop ticking every `tick_ms`
/// writes the heartbeat: every `HEARTBEAT_MS`.
pub fn heartbeat_due(tick: u64, tick_ms: u64) -> bool {
    tick > 0 && tick_ms > 0 && (tick * tick_ms) % HEARTBEAT_MS == 0
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core remembered::`
Expected: `test result: ok. 15 passed` (14 on Windows, without the 0600 test).

- [ ] **Step 5: Commit**

```bash
git add core/src/remembered.rs core/src/lib.rs
git commit -m "feat: remember the sessions running on this machine" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The store remembers, and Close forgets

**Files:**
- Modify: `core/src/store.rs` (field, `new`, `refresh`, new methods, tests)
- Modify: `core/src/actions.rs` (`close_session_with`, a test)

**Interfaces:**
- Consumes: Task 1's `Memory`, `LiveSession::of`, `Key`, `Remembered`, `path_in`.
- Produces on `Store`:
  - `pub fn start_remembering(&mut self, now_ms: u64) -> Vec<Remembered>` (loads, classifies, saves; returns what is offered)
  - `pub fn was_running(&self) -> Vec<Remembered>`
  - `pub fn remembered_entry(&self, agent: Harness, session_id: &str) -> Option<Remembered>`
  - `pub fn session_closed(&mut self, agent: Harness, session_id: &str)`
  - `pub fn forget_remembered(&mut self, keys: &[Key])`
  - `pub fn remembered_failed(&mut self, key: &Key, reason: &str)`
  - `pub fn heartbeat(&mut self, now_ms: u64)`
  - every one is a no-op (and `was_running` empty) until `start_remembering` ran.

- [ ] **Step 1: Write the failing tests**

Append to `core/src/store.rs`'s `mod tests`:

```rust
    fn claude_with_session(root: &Path) -> PathBuf {
        let claude = root.join("claude");
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        std::fs::write(claude.join("sessions/7.json"), r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle"}"#).unwrap();
        claude
    }

    #[test]
    fn a_session_running_when_maya_stopped_is_offered_at_the_next_start() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let mut first = Store::new(claude.clone()).with_alive(|_| true);
        assert!(first.start_remembering(1_000).is_empty());
        first.refresh(2_000);
        first.heartbeat(30_000);
        drop(first);
        // The machine restarted: the session's process is gone.
        let mut second = Store::new(claude).with_alive(|_| false);
        let offered = second.start_remembering(400_000);
        assert_eq!(offered.iter().map(|e| (e.session_id.as_str(), e.dir.as_str(), e.name.as_str())).collect::<Vec<_>>(), [("s7", "/Users/x/dev/eye", "eye-7")]);
        second.refresh(401_000);
        assert_eq!(second.was_running().len(), 1, "still offered while it is not running");
        second.forget_remembered(&[(Harness::ClaudeCode, "s7".into())]);
        assert!(second.was_running().is_empty());
    }

    #[test]
    fn sessions_that_outlived_a_restart_of_maya_alone_are_not_offered() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let mut first = Store::new(claude.clone()).with_alive(|_| true);
        first.start_remembering(1_000);
        first.refresh(2_000);
        drop(first);
        // Only Maya was quit and reopened: the session still runs.
        let mut second = Store::new(claude).with_alive(|_| true);
        assert_eq!(second.start_remembering(60_000).len(), 1, "judged before the first refresh");
        second.refresh(61_000);
        assert!(second.was_running().is_empty(), "it is live: nothing to resume");
    }

    #[test]
    fn a_store_that_does_not_remember_writes_no_file() {
        let t = tempfile::tempdir().unwrap();
        let claude = claude_with_session(t.path());
        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.refresh(1);
        store.heartbeat(2);
        store.session_closed(Harness::ClaudeCode, "s7");
        assert!(!crate::remembered::path_in(&claude).exists());
        assert!(store.was_running().is_empty());
    }
```

Append to `core/src/actions.rs`'s `mod tests`:

```rust
    #[test]
    fn closing_from_the_card_forgets_the_session_at_once() {
        let (dir, store) = store_with_session("s1", 4242);
        {
            let mut s = store.lock().unwrap();
            s.start_remembering(1);
            s.refresh(2);
            assert!(s.remembered_entry(Harness::ClaudeCode, "s1").is_some());
        }
        let t = FakeTerminal::default();
        close_session_with(&Local { store: &store, terminal: &t }, "s1", |_| true).unwrap();
        assert!(store.lock().unwrap().remembered_entry(Harness::ClaudeCode, "s1").is_none());
        let file = std::fs::read_to_string(crate::remembered::path_in(dir.path())).unwrap();
        assert!(!file.contains("\"s1\""), "{file}");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core store::tests::a_session_running closing_from_the_card`
Expected: FAIL to compile: "no method named `start_remembering` found for struct `Store`".

- [ ] **Step 3: Write the implementation**

In `core/src/store.rs`, add to the imports:

```rust
use crate::remembered::{Key, LiveSession, Memory, Remembered};
```

Add the field at the end of `pub struct Store`:

```rust
    /// The sessions this machine remembers across restarts, once
    /// `start_remembering` ran (the app does; the CLI and most tests do not).
    memory: Option<Memory>,
```

and `memory: None,` at the end of the struct literal in `Store::new`.

In `refresh`, just before the final `cards` (after `self.log_pending_events();`):

```rust
        if let Some(m) = self.memory.as_mut() {
            let live: Vec<LiveSession> = cards.iter().map(LiveSession::of).collect();
            m.observe(&live, now_ms);
        }
        self.save_memory();
```

Add these methods inside `impl Store` (after `take_due_renames`):

```rust
    /// Starts remembering this machine's sessions: reads what the last run
    /// left, keeps what was running when it stopped, and saves. Called
    /// once, before the first refresh. Returns what is offered.
    pub fn start_remembering(&mut self, now_ms: u64) -> Vec<Remembered> {
        let mut m = Memory::load(crate::remembered::path_in(&self.claude_dir));
        let offered = m.classify_at_start(now_ms);
        self.memory = Some(m);
        self.save_memory();
        offered
    }

    /// Writes the memory when it changed; a failure is logged, never fatal.
    fn save_memory(&mut self) {
        if let Some(m) = self.memory.as_mut() {
            if let Err(e) = m.save() {
                crate::log::line("remembered", format!("could not save: {e}"));
            }
        }
    }

    /// What was running when Maya last stopped and is not running now.
    pub fn was_running(&self) -> Vec<Remembered> {
        self.memory.as_ref().map(Memory::was_running).unwrap_or_default()
    }

    pub fn remembered_entry(&self, agent: Harness, session_id: &str) -> Option<Remembered> {
        self.memory.as_ref()?.entry(agent, session_id)
    }

    /// Closed from its card (here or relayed from the main): forgotten at once.
    pub fn session_closed(&mut self, agent: Harness, session_id: &str) {
        if let Some(m) = self.memory.as_mut() {
            m.closed(agent, session_id);
        }
        self.save_memory();
    }

    pub fn forget_remembered(&mut self, keys: &[Key]) {
        if let Some(m) = self.memory.as_mut() {
            m.forget(keys);
        }
        self.save_memory();
    }

    pub fn remembered_failed(&mut self, key: &Key, reason: &str) {
        if let Some(m) = self.memory.as_mut() {
            m.failed(key, reason);
        }
        self.save_memory();
    }

    /// This run's last sign of life, every `remembered::HEARTBEAT_MS`.
    pub fn heartbeat(&mut self, now_ms: u64) {
        if let Some(m) = self.memory.as_mut() {
            m.heartbeat(now_ms);
        }
        self.save_memory();
    }
```

In `core/src/actions.rs`, `close_session_with`, right after the `if !exited(pid) { … }` block:

```rust
    // Closed by the user from its card: not one to offer after a restart.
    l.store.lock().unwrap().session_closed(harness, session_id);
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core`
Expected: `test result: ok.` with the four new tests among them.

- [ ] **Step 5: Commit**

```bash
git add core/src/store.rs core/src/actions.rs
git commit -m "feat: the store remembers its sessions and forgets one closed from its card" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Resuming remembered sessions

**Files:**
- Modify: `core/src/actions.rs` (constants, `resume_remembered`, `resume_remembered_with`, `resume_one`, `resume_relayed`, tests)

**Interfaces:**
- Consumes: Task 2's `Store::{was_running, remembered_entry, forget_remembered, remembered_failed, agent_dirs, live_session_ids, agents_source, opencode_service}`; `resume::{plain_session_id, list_sessions, resume_command}`; `opencode::{resumable, Client}`; `launch::label`; `tmux_label()`.
- Produces:
  - `pub const FOLDER_MISSING: &str = "folder missing"; pub const SESSION_GONE: &str = "the agent no longer has that session";`
  - `pub fn resume_remembered(l: &Local, entries: &[Remembered]) -> Vec<(Key, Result<(), String>)>`
  - `pub fn resume_relayed(l: &Local, agent: model::Harness, dir: &str, session_id: &str) -> Result<(), String>` (a `Resume` from the main: from memory when `dir` is the absolute folder this Maya recorded, else `resume_session`)

Decided here: with OpenCode's server down (it is, after a restart) the recorded id is trusted rather than listed; with it up the server's listing is asked, as `resume_session` does. A skip (already live) counts as success and leaves the list.

- [ ] **Step 1: Write the failing tests**

Append to `core/src/actions.rs`'s `mod tests` (it already has `use super::*`, `FakeTerminal`, `Call`, `Harness`):

```rust
    use crate::remembered::Remembered;

    /// A temp `~/.claude` and a folder `elsewhere/<name>` outside any projects directory.
    fn away_from_projects(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let claude = t.path().join("claude");
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("maya")).unwrap();
        let dir = t.path().join("elsewhere").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        (t, claude, dir)
    }

    /// Claude Code's transcript for `id` in `dir`: the agent knows the session.
    fn knows(claude: &Path, dir: &Path, id: &str) {
        let folder = claude.join("projects").join(crate::registry::project_dir_name(&dir.to_string_lossy()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{id}.jsonl")), "{\"type\":\"custom-title\",\"customTitle\":\"t\"}\n").unwrap();
    }

    fn was(agent: Harness, id: &str, dir: &Path, last_seen_ms: u64) -> Remembered {
        Remembered { agent, session_id: id.into(), dir: dir.to_string_lossy().into_owned(), name: format!("{id}-name"), first_seen_ms: 1, last_seen_ms, gone_at_ms: None, offered: false, error: None }
    }

    /// A store whose last run left `entries` running when it stopped.
    fn after_restart(claude: &Path, entries: Vec<Remembered>) -> Mutex<Store> {
        let record = serde_json::json!({ "heartbeatMs": 10_000, "sessions": entries });
        std::fs::write(crate::remembered::path_in(claude), record.to_string()).unwrap();
        let mut store = Store::new(claude.to_path_buf()).with_alive(|_| true);
        store.start_remembering(20_000);
        Mutex::new(store)
    }

    fn opened(fake: &FakeTerminal) -> Vec<(String, PathBuf)> {
        fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Open { command, cwd, .. } => Some((command.clone(), cwd.clone())), _ => None }).collect()
    }

    #[test]
    fn remembered_sessions_reopen_oldest_first_400_ms_apart_outside_the_projects_directory() {
        let (_t, claude, dir) = away_from_projects("calibre");
        knows(&claude, &dir, "new1");
        knows(&claude, &dir, "old1");
        let store = after_restart(&claude, vec![was(Harness::ClaudeCode, "new1", &dir, 9_000), was(Harness::ClaudeCode, "old1", &dir, 5_000)]);
        assert!(store.lock().unwrap().config.projects_dir_path().is_none(), "no projects directory at all");
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let entries = store.lock().unwrap().was_running();
        let mut waits = Vec::new();
        let out = resume_remembered_with(&l, &entries, &mut |d| waits.push(d));
        assert_eq!(out, vec![((Harness::ClaudeCode, "old1".to_string()), Ok(())), ((Harness::ClaudeCode, "new1".to_string()), Ok(()))]);
        let opened = opened(&fake);
        assert!(opened[0].0.ends_with("claude --resume 'old1'") && opened[1].0.ends_with("claude --resume 'new1'"), "{opened:?}");
        assert!(opened.iter().all(|(_, cwd)| *cwd == dir), "each in its own folder");
        assert_eq!(waits, [Duration::from_millis(400)], "one pause, between the two");
        assert!(store.lock().unwrap().was_running().is_empty(), "resumed: no longer offered");
    }

    #[test]
    fn a_live_session_is_skipped_and_failures_stay_offered_with_their_reason() {
        let (_t, claude, dir) = away_from_projects("app");
        write_session(&claude, "live1", 4242, &dir.to_string_lossy());
        let gone = dir.parent().unwrap().join("deleted");
        let store = after_restart(&claude, vec![was(Harness::ClaudeCode, "live1", &dir, 1), was(Harness::ClaudeCode, "lost", &gone, 2), was(Harness::ClaudeCode, "nolog", &dir, 3), was(Harness::Codex, "cx1", &dir, 4)]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let entries = store.lock().unwrap().was_running();
        let out: Vec<(String, Result<(), String>)> = resume_remembered_with(&l, &entries, &mut |_| ()).into_iter().map(|((_, id), r)| (id, r)).collect();
        assert_eq!(
            out,
            vec![
                ("live1".to_string(), Ok(())),
                ("lost".to_string(), Err("folder missing".to_string())),
                ("nolog".to_string(), Err("the agent no longer has that session".to_string())),
                ("cx1".to_string(), Err("Codex is not installed".to_string())),
            ]
        );
        assert!(opened(&fake).is_empty());
        let left: Vec<(String, Option<String>)> = store.lock().unwrap().was_running().into_iter().map(|e| (e.session_id, e.error)).collect();
        assert_eq!(
            left,
            vec![
                ("lost".to_string(), Some("folder missing".to_string())),
                ("nolog".to_string(), Some("the agent no longer has that session".to_string())),
                ("cx1".to_string(), Some("Codex is not installed".to_string())),
            ]
        );
    }

    #[test]
    fn a_folder_with_a_quote_is_quoted_and_a_file_or_relative_path_is_missing() {
        let (_t, claude, dir) = away_from_projects("it's mine");
        knows(&claude, &dir, "q1");
        let file = dir.parent().unwrap().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let relative = Remembered { dir: "elsewhere/it's mine".into(), ..was(Harness::ClaudeCode, "rel", &dir, 3) };
        let store = after_restart(&claude, vec![was(Harness::ClaudeCode, "q1", &dir, 1), was(Harness::ClaudeCode, "f1", &file, 2), relative]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        let entries = store.lock().unwrap().was_running();
        let out = resume_remembered_with(&l, &entries, &mut |_| ());
        assert_eq!(out[0].1, Ok(()));
        assert_eq!(out[1].1, Err(FOLDER_MISSING.to_string()), "a file is not a folder");
        assert_eq!(out[2].1, Err(FOLDER_MISSING.to_string()), "a relative path is never run");
        let opened = opened(&fake);
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].0, format!("cd {} && claude --resume 'q1'", crate::launch::shell_single_quote(&dir.to_string_lossy())));
    }

    #[test]
    fn a_relayed_resume_of_a_remembered_folder_resumes_it_and_a_project_name_goes_the_usual_way() {
        let (_t, claude, dir) = away_from_projects("calibre");
        knows(&claude, &dir, "r1");
        let store = after_restart(&claude, vec![was(Harness::ClaudeCode, "r1", &dir, 1)]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        resume_relayed(&l, Harness::ClaudeCode, &dir.to_string_lossy(), "r1").unwrap();
        assert_eq!(opened(&fake).len(), 1);
        assert!(store.lock().unwrap().was_running().is_empty());
        assert_eq!(resume_relayed(&l, Harness::ClaudeCode, "calibre", "r2").unwrap_err(), "Set a projects directory in Settings first.");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core actions::tests::a_relayed_resume`
Expected: FAIL to compile: "cannot find function `resume_remembered_with` in this scope".

- [ ] **Step 3: Write the implementation**

In `core/src/actions.rs`, add to the imports:

```rust
use crate::remembered::{Key, Remembered};
```

Add after `resume_session`:

```rust
/// Why a remembered session could not be resumed, as the banner words it.
pub const FOLDER_MISSING: &str = "folder missing";
pub const SESSION_GONE: &str = "the agent no longer has that session";

fn not_installed(agent: model::Harness) -> String {
    format!("{} is not installed", launch::label(agent))
}

/// Reopens remembered sessions, each in its own folder and terminal with
/// its own conversation, oldest first (so the newest ends up in front),
/// `RESUME_GAP_MS` apart. One already live is skipped. Unlike
/// `resume_session` the folder need not be under the projects directory:
/// Maya recorded it, and it must still be an absolute path to a directory.
/// Successes leave the "was running" list; failures stay with their reason.
pub fn resume_remembered(l: &Local, entries: &[Remembered]) -> Vec<(Key, Result<(), String>)> {
    resume_remembered_with(l, entries, &mut |d| std::thread::sleep(d))
}

/// `resume_remembered` with `pause`, which waits between two terminals.
fn resume_remembered_with(l: &Local, entries: &[Remembered], pause: &mut dyn FnMut(Duration)) -> Vec<(Key, Result<(), String>)> {
    let mut order: Vec<&Remembered> = entries.iter().collect();
    order.sort_by_key(|e| e.last_seen_ms);
    let (dirs, running, agents, service) = {
        let store = l.store.lock().unwrap();
        (store.agent_dirs(), store.live_session_ids(), store.agents_source(), store.opencode_service())
    };
    // Listing the agents can take seconds: only when one beyond Claude Code is asked for.
    let installed: Vec<model::Harness> = if order.iter().any(|e| e.agent != model::Harness::ClaudeCode) { agents().into_iter().map(|a| a.harness).collect() } else { vec![] };
    let mut opened = false;
    let mut out = Vec::with_capacity(order.len());
    for e in order {
        let result = match resume_one(e, &dirs, &running, &installed, service.as_ref()) {
            // Already live: nothing to open.
            Ok(None) => Ok(()),
            Ok(Some(path)) => {
                if opened {
                    pause(Duration::from_millis(crate::remembered::RESUME_GAP_MS));
                }
                opened = true;
                l.terminal.open(&resume::resume_command(e.agent, &path, &e.session_id), &path, &tmux_label()).map(|_| ())
            }
            Err(why) => Err(why),
        };
        if let Err(why) = &result {
            crate::log::line("remembered", format!("could not resume {} ({}): {why}", e.name, e.session_id));
        }
        out.push((e.key(), result));
    }
    let mut store = l.store.lock().unwrap();
    for (key, result) in &out {
        match result {
            Ok(()) => store.forget_remembered(std::slice::from_ref(key)),
            Err(why) => store.remembered_failed(key, why),
        }
    }
    out
}

/// The folder to resume `e` in, `None` when it is already live, or why it cannot be.
fn resume_one(e: &Remembered, dirs: &resume::AgentDirs, running: &[String], installed: &[model::Harness], service: Option<&crate::opencode::Service>) -> Result<Option<PathBuf>, String> {
    if running.iter().any(|id| *id == e.session_id) {
        return Ok(None);
    }
    if !resume::plain_session_id(&e.session_id) {
        return Err(SESSION_GONE.into());
    }
    let path = PathBuf::from(&e.dir);
    if !path.is_absolute() || !path.is_dir() {
        return Err(FOLDER_MISSING.into());
    }
    if e.agent == model::Harness::Other || (e.agent != model::Harness::ClaudeCode && !installed.contains(&e.agent)) {
        return Err(not_installed(e.agent));
    }
    let known = match (e.agent, service) {
        // OpenCode's sessions live in its server, which a restart stopped:
        // the id Maya recorded is trusted.
        (model::Harness::OpenCode, None) => true,
        (model::Harness::OpenCode, Some(s)) => crate::opencode::resumable(&crate::opencode::Client::new(s), &e.dir, running).iter().any(|r| r.id == e.session_id),
        _ => resume::list_sessions(e.agent, dirs, &e.dir, running).iter().any(|r| r.id == e.session_id),
    };
    if !known {
        return Err(SESSION_GONE.into());
    }
    Ok(Some(path))
}

/// A `Resume` from the main: a remembered session when `dir` is the
/// absolute folder this Maya recorded for it, else a project folder's.
pub fn resume_relayed(l: &Local, agent: model::Harness, dir: &str, session_id: &str) -> Result<(), String> {
    let remembered = l.store.lock().unwrap().remembered_entry(agent, session_id).filter(|e| e.dir == dir);
    match remembered {
        Some(e) => resume_remembered(l, &[e]).into_iter().next().map_or(Ok(()), |(_, r)| r),
        None => resume_session(l, agent, dir, session_id),
    }
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core actions::`
Expected: `test result: ok.` including the four new tests.

- [ ] **Step 5: Commit**

```bash
git add core/src/actions.rs
git commit -m "feat: resume remembered sessions in their own folders, oldest first" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The network carries each machine's list, and Forget

**Files:**
- Modify: `core/src/net/protocol.rs` (`Up::Board`, `Down::Mirror`, `CommandKind::Forget`, tests)
- Modify: `core/src/net/merge.rs` (`RemoteBoard.remembered`, `remembered()`, the test helper, a test)
- Modify: `core/src/net/server.rs` (store the list, mirror rows, `mirror_remembered`, `drop_remembered`, `kind_name`, literals, tests)
- Modify: `core/src/net/client.rs` (`Executor::remembered`, `ClientNotify::mirrored_remembered`, the board frame, `Down::Mirror` arm)
- Modify: `core/src/net/routing.rs` (`target_of`, a test)
- Modify: `cli/src/executor.rs` (`Forget` arm; `Resume` through `resume_relayed`)
- Modify: `src-tauri/src/net_app.rs` (`execute`: `Forget` arm; `Resume` through `resume_relayed`)

**Interfaces:**
- Consumes: Task 1's `Remembered`, `RememberedRow`, `Key`; Task 2's `Store::forget_remembered`; Task 3's `actions::resume_relayed`.
- Produces:
  - `Up::Board { cards, dirs, agents, remembered: Option<Vec<Remembered>> }` (`#[serde(default, skip_serializing_if = "Option::is_none")]`)
  - `Down::Mirror { allowed, cards, machines, remembered: Vec<RememberedRow> }` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`)
  - `CommandKind::Forget { sessions: Vec<(Harness, String)> }` (wire `{"kind":"forget","sessions":[["codex","c1"]]}`), `kind_name` "forget", `target_of` → `Target::Machine`
  - `RemoteBoard.remembered: Option<Vec<Remembered>>`
  - `pub fn merge::remembered(local: Vec<Remembered>, remotes: &[RemoteBoard]) -> Vec<RememberedRow>`
  - `ServerHandle::mirror_remembered(&self, rows: Vec<RememberedRow>)`, `ServerHandle::drop_remembered(&self, machine: &str, keys: &[Key])`
  - `Executor::remembered(&self) -> Option<Vec<Remembered>>` (default `None`), `ClientNotify::mirrored_remembered(&self, rows: Vec<RememberedRow>)` (default no-op; called just before `mirrored`)

Decided here: the phone receives the merged list in `Down::Mirror` (the spec says it rides with the board it mirrors); the main's own rows reach it labelled with the main's name, as its cards are. An older assistant cannot decode `Forget` and does not answer; the main sends it on a thread of its own (Task 5) and logs the timeout.

- [ ] **Step 1: Write the failing tests**

Append to `core/src/net/protocol.rs`'s `mod tests`:

```rust
    fn was(id: &str) -> crate::remembered::Remembered {
        crate::remembered::Remembered { agent: crate::model::Harness::ClaudeCode, session_id: id.into(), dir: "/Users/x/dev/calibre".into(), name: "calibre-88".into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None }
    }

    #[test]
    fn a_board_carries_what_was_running_and_an_older_board_has_none() {
        let board = Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: Some(vec![was("s1")]) };
        let s = encode(&board);
        assert!(s.contains(r#""remembered":[{"agent":"claude-code","sessionId":"s1""#), "{s}");
        assert_eq!(decode_up(&s).unwrap(), board);
        let Up::Board { remembered, .. } = decode_up(r#"{"type":"board","cards":[],"dirs":[]}"#).unwrap() else { panic!() };
        assert_eq!(remembered, None);
        assert!(!encode(&Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: None }).contains("remembered"));
    }

    #[test]
    fn a_board_with_fields_and_agents_this_maya_does_not_know_still_parses() {
        let newer = r#"{"type":"board","cards":[],"dirs":[],"remembered":[{"agent":"future-agent","sessionId":"x","dir":"/x","name":"x","firstSeenMs":1,"lastSeenMs":2,"somethingNew":true}],"alsoNew":1}"#;
        let Up::Board { remembered, .. } = decode_up(newer).unwrap() else { panic!() };
        assert_eq!(remembered.unwrap()[0].agent, crate::model::Harness::Other);
    }

    #[test]
    fn forget_names_each_session_by_agent_and_id() {
        let f = Down::Command { id: 4, kind: CommandKind::Forget { sessions: vec![(crate::model::Harness::Codex, "c1".into())] } };
        let s = encode(&f);
        assert!(s.contains(r#""kind":"forget","sessions":[["codex","c1"]]"#), "{s}");
        assert_eq!(decode_down(&s).unwrap(), f);
    }

    #[test]
    fn a_mirror_carries_the_remembered_rows_only_when_there_are_some() {
        let rows = vec![crate::remembered::RememberedRow { machine: Some("desk".into()), waiting: false, entry: was("s1") }];
        let m = Down::Mirror { allowed: true, cards: vec![], machines: vec![], remembered: rows };
        assert_eq!(decode_down(&encode(&m)).unwrap(), m);
        assert!(!encode(&Down::Mirror { allowed: false, cards: vec![], machines: vec![], remembered: vec![] }).contains("remembered"));
    }
```

Append to `core/src/net/merge.rs`'s `mod tests`:

```rust
    #[test]
    fn remembered_rows_put_this_machine_first_and_mark_an_away_assistant_waiting() {
        let was = |id: &str| crate::remembered::Remembered { agent: Harness::ClaudeCode, session_id: id.into(), dir: "/x".into(), name: id.into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None };
        let mut laptop = board("laptop", "laptop", &[], 1, false);
        laptop.remembered = Some(vec![was("r1")]);
        let mut pi = board("pi", "pi", &[], 1, true);
        pi.remembered = Some(vec![was("p1")]);
        let older = board("old", "old", &[], 1, true);
        let rows = remembered(vec![was("l1")], &[laptop, pi, older]);
        let got: Vec<(Option<&str>, bool, &str)> = rows.iter().map(|r| (r.machine.as_deref(), r.waiting, r.entry.session_id.as_str())).collect();
        assert_eq!(got, [(None, false, "l1"), (Some("laptop"), true, "r1"), (Some("pi"), false, "p1")], "kept however old the board, for this run");
    }
```

Append to `core/src/net/routing.rs`'s `mod tests`:

```rust
    #[test]
    fn forget_goes_to_the_machine_named() {
        assert_eq!(target_of(&crate::net::protocol::CommandKind::Forget { sessions: vec![] }), Target::Machine);
    }
```

Append to `core/src/net/server.rs`'s `mod tests`:

```rust
    fn was(id: &str) -> crate::remembered::Remembered {
        crate::remembered::Remembered { agent: crate::model::Harness::ClaudeCode, session_id: id.into(), dir: "/Users/x/dev/calibre".into(), name: format!("{id}-name"), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None }
    }

    #[test]
    fn an_assistant_s_remembered_list_is_kept_while_it_is_away_and_a_dismiss_drops_it() {
        let (handle, port) = test_server();
        let (mut ws, _, _) = pair_client(&handle, port, "desk");
        send(&mut ws, &Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: Some(vec![was("r9")]) });
        wait_until(|| handle.boards().iter().any(|b| b.remembered.as_ref().is_some_and(|l| l.len() == 1)));
        drop(ws);
        wait_until(|| handle.boards().iter().any(|b| !b.connected));
        assert_eq!(handle.boards()[0].remembered.as_ref().map(Vec::len), Some(1), "kept while it is away");
        handle.drop_remembered("desk", &[was("r9").key()]);
        assert_eq!(handle.boards()[0].remembered.as_ref().map(Vec::len), Some(0));
        handle.stop();
    }

    #[test]
    fn the_mirror_carries_the_remembered_rows_with_the_main_s_own_named() {
        let (handle, port) = test_server();
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.connected));
        handle.mirror(vec![]);
        let Down::Mirror { remembered, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        assert!(remembered.is_empty());
        let local = crate::remembered::RememberedRow { machine: None, waiting: false, entry: was("l1") };
        let away = crate::remembered::RememberedRow { machine: Some("laptop".into()), waiting: true, entry: was("r1") };
        handle.mirror_remembered(vec![local, away]);
        let Down::Mirror { remembered, cards, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        let main = main_name_of(&NetworkConfig::default());
        assert!(cards.is_empty(), "the board it already had");
        assert_eq!(remembered[0].machine.as_deref(), Some(main.as_str()), "the main's own rows carry its name");
        assert_eq!((remembered[1].machine.as_deref(), remembered[1].waiting), (Some("laptop"), true));
        handle.stop();
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core net::`
Expected: FAIL to compile: "struct `RemoteBoard` has no field named `remembered`" / "variant `Up::Board` has no field named `remembered`".

- [ ] **Step 3: Write the implementation**

`core/src/net/protocol.rs`:

```rust
    Board {
        cards: Vec<Card>,
        dirs: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "known_agents")]
        agents: Option<Vec<crate::agents::AgentInfo>>,
        /// What was running on this machine when its Maya last stopped; an older Maya sends none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remembered: Option<Vec<crate::remembered::Remembered>>,
    },
```

```rust
    Mirror {
        allowed: bool,
        #[serde(default)]
        cards: Vec<Card>,
        #[serde(default)]
        machines: Vec<crate::net::routing::MachineInfo>,
        /// The main's merged "was running" list; its own rows carry its name.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remembered: Vec<crate::remembered::RememberedRow>,
    },
```

and in `CommandKind`, after `History`:

```rust
    /// Forget these remembered sessions (Dismiss, Forget unticked).
    Forget { sessions: Vec<(crate::model::Harness, String)> },
```

Then make every literal compile: add `remembered: None` to each `Up::Board { … }` literal (`grep -n "Up::Board {" core/src/net/*.rs`; patterns ending in `..` need nothing) and `remembered: vec![]` to each `Down::Mirror { … }` literal (`grep -n "Down::Mirror {" core/src/net/*.rs`), and add `..` to the pattern `let Down::Mirror { allowed, cards, machines } = recv(&mut phone)` in `server.rs`'s mirror test.

`core/src/net/merge.rs`: add to `RemoteBoard`

```rust
    /// What was running on it when its Maya last stopped, as it last said;
    /// `None` for an older Maya. Kept in memory for this run only.
    pub remembered: Option<Vec<crate::remembered::Remembered>>,
```

add `remembered: None` to the test helper `board(…)`'s literal, and add the function after `merged`:

```rust
/// The "was running" list the banner shows: this machine's first, then each
/// assistant's last list, labelled with its machine and marked waiting while
/// it is not connected. Kept for this run however old the board.
pub fn remembered(local: Vec<crate::remembered::Remembered>, remotes: &[RemoteBoard]) -> Vec<crate::remembered::RememberedRow> {
    use crate::remembered::RememberedRow;
    let mut out: Vec<RememberedRow> = local.into_iter().map(|entry| RememberedRow { machine: None, waiting: false, entry }).collect();
    for b in remotes {
        for e in b.remembered.iter().flatten() {
            out.push(RememberedRow { machine: Some(b.machine.clone()), waiting: !b.connected, entry: e.clone() });
        }
    }
    out
}
```

`core/src/net/routing.rs` `target_of`:

```rust
        Start { .. } | Resume { .. } | ListResumable { .. } | Promote { .. } | Forget { .. } => Target::Machine,
```

`core/src/net/server.rs`:

- import: `use crate::remembered::{Key, RememberedRow};`
- `MirrorBoard` gains `remembered: Vec<RememberedRow>,`; `bump_mirror` builds `MirrorBoard { serial: self.mirror_serial, cards: board.cards, remembered: board.remembered }`.
- `ServerHandle::mirror` keeps the rows:

```rust
    pub fn mirror(&self, cards: Vec<Card>) {
        let mut s = lock(&self.shared);
        s.mirror_serial += 1;
        let remembered = s.mirror.as_ref().map(|b| b.remembered.clone()).unwrap_or_default();
        s.mirror = Some(MirrorBoard { serial: s.mirror_serial, cards, remembered });
    }

    /// The merged "was running" list for mirroring Mayas; it goes out with
    /// the board on the next tick when it changed.
    pub fn mirror_remembered(&self, rows: Vec<RememberedRow>) {
        let mut s = lock(&self.shared);
        match &s.mirror {
            Some(b) if b.remembered == rows => return,
            None if rows.is_empty() => return,
            _ => {}
        }
        s.mirror_serial += 1;
        let cards = s.mirror.as_ref().map(|b| b.cards.clone()).unwrap_or_default();
        s.mirror = Some(MirrorBoard { serial: s.mirror_serial, cards, remembered: rows });
    }

    /// Forgets `keys` from the list the assistant shown as `machine` last
    /// sent, so a Dismiss clears the banner even for one that is away. It
    /// sends its own list again when it reconnects.
    pub fn drop_remembered(&self, machine: &str, keys: &[Key]) {
        let mut s = lock(&self.shared);
        let Some(id) = s.id_for(machine) else { return };
        if let Some(list) = s.by_id.get_mut(&id).and_then(|b| b.remembered.as_mut()) {
            list.retain(|e| !keys.contains(&e.key()));
        }
        s.sync_boards();
    }
```

- `set_mirror`'s refusal: `Down::Mirror { allowed: false, cards: vec![], machines: vec![], remembered: vec![] }`.
- `mirror_due`, replacing its last three lines:

```rust
    let remembered = board
        .remembered
        .iter()
        .cloned()
        .map(|mut r| {
            r.machine.get_or_insert_with(|| ctx.main_name.clone());
            r
        })
        .collect();
    link.mirror_sent.set(board.serial);
    link.mirror_at.set(Some(Instant::now()));
    Some(Down::Mirror { allowed: true, cards, machines: s.mirror_machines(&ctx.main_name, &link.id), remembered })
```

- `handle`: the arm becomes `Up::Board { cards, dirs, agents, remembered } => {` and its insert `RemoteBoard { machine: String::new(), hostname: String::new(), platform: String::new(), address: String::new(), cards, dirs, agents, remembered, received_at: now_ms(), connected: true }`; the test literal near `s.by_id.insert(first.id.clone(), RemoteBoard { … })` gains `remembered: None`.
- `kind_name`: `CommandKind::Forget { .. } => "forget",`.

`core/src/net/client.rs`:

```rust
    /// What was running on this machine when its Maya last stopped, for
    /// the main's banner; `None` from a Maya that does not remember.
    fn remembered(&self) -> Option<Vec<crate::remembered::Remembered>> {
        None
    }
```

(in `trait Executor`), and in `trait ClientNotify`:

```rust
    /// The main's "was running" rows, just before `mirrored` with its board.
    fn mirrored_remembered(&self, _rows: Vec<crate::remembered::RememberedRow>) {}
```

The board frame: `let frame = encode(&Up::Board { cards, dirs, agents: exec.agents(), remembered: exec.remembered() });`. The `handle` arm:

```rust
        Down::Mirror { allowed, cards, machines, remembered } => {
            notify.mirrored_remembered(remembered);
            notify.mirrored(allowed, cards, machines);
        }
```

`cli/src/executor.rs`, in `execute`:

```rust
            CommandKind::Resume { dir, session, agent } => done(actions::resume_relayed(&l, agent, &dir, &session)),
```

and before the `Promote` arm:

```rust
            CommandKind::Forget { sessions } => {
                self.store.lock().unwrap().forget_remembered(&sessions);
                self.board_due.store(true, Ordering::SeqCst);
                Ok(None)
            }
```

`src-tauri/src/net_app.rs`, in `execute`:

```rust
        CommandKind::Resume { dir, session, agent } => done(actions::resume_relayed(&l, agent, &dir, &session)),
```

and before the `Promote` arm:

```rust
        CommandKind::Forget { sessions } => {
            state.store.lock().unwrap().forget_remembered(&sessions);
            // The banner here and the board the main holds catch up at once.
            crate::refresh_and_emit(app);
            Ok(None)
        }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core net:: && cargo test -p maya-cli && cargo build -p maya`
Expected: `test result: ok.` for both crates, the desktop app builds.

- [ ] **Step 5: Commit**

```bash
git add core/src/net cli/src/executor.rs src-tauri/src/net_app.rs
git commit -m "feat: assistants send what was running with their board, and take Forget" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The desktop app — start, heartbeat, rows, Resume and Forget

**Files:**
- Create: `src-tauri/src/remembered_app.rs`
- Modify: `src-tauri/src/lib.rs` (module, commands, setup, PR loop, `refresh_and_emit`, a test)
- Modify: `src-tauri/src/net_app.rs` (`TauriExecutor::remembered`)

**Interfaces:**
- Consumes: Task 2's `Store::{start_remembering, was_running, forget_remembered, heartbeat}`; Task 3's `actions::resume_remembered`; Task 4's `merge::remembered`, `ServerHandle::{mirror_remembered, drop_remembered}`, `CommandKind::Forget`, `Executor::remembered`; Task 1's `plan`, `keys_by_machine`, `ResumeOutcome::of`, `waiting_for`, `heartbeat_due`, `RESUME_GAP_MS`.
- Produces:
  - `pub(crate) fn remembered_app::rows(state: &AppState) -> Vec<RememberedRow>` (no refresh)
  - `pub(crate) fn remembered_app::publish(app: &AppHandle)` (event `remembered` + mirror)
  - `pub(crate) fn remembered_app::resume_rows(app: &AppHandle, rows: &[RememberedRow]) -> Vec<ResumeOutcome>`
  - Tauri commands `remembered_sessions() -> Vec<RememberedRow>`, `resume_remembered(rows: Vec<RememberedRow>) -> Vec<ResumeOutcome>`, `forget_remembered(rows: Vec<RememberedRow>)`
  - `const PR_POLL: Duration = Duration::from_secs(10)` in `lib.rs`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/src/lib.rs`'s `mod route_tests`:

```rust
    #[test]
    fn the_heartbeat_rides_every_third_pr_poll() {
        let due: Vec<u64> = (1..=9).filter(|t| maya_core::remembered::heartbeat_due(*t, PR_POLL.as_millis() as u64)).collect();
        assert_eq!(due, [3, 6, 9]);
    }
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test -p maya route_tests::the_heartbeat`
Expected: FAIL to compile: "cannot find value `PR_POLL` in this scope".

- [ ] **Step 3: Write the implementation**

Create `src-tauri/src/remembered_app.rs`:

```rust
//! Resume after a restart on the desktop: the "was running" list across this
//! machine and its assistants, Resume and Forget for the banner and the
//! voice, and the heartbeat. Locks: `store` and `network` one at a time.

use crate::AppState;
use maya_core::log;
use maya_core::net::merge;
use maya_core::net::protocol::CommandKind;
use maya_core::net::routing::ROUTE_TIMEOUT;
use maya_core::remembered::{self, RememberedRow, ResumeOutcome};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

/// The merged list as of the store's last refresh.
pub(crate) fn rows(state: &AppState) -> Vec<RememberedRow> {
    let local = state.store.lock().unwrap().was_running();
    merge::remembered(local, &crate::remote_boards(state))
}

/// Tells the page (`remembered`) and the mirroring Mayas; on every refresh.
pub(crate) fn publish(app: &AppHandle) {
    let state = app.state::<AppState>();
    let rows = rows(&state);
    let server = state.network.lock().unwrap().server.clone();
    if let Some(s) = server {
        s.mirror_remembered(rows.clone());
    }
    let _ = app.emit("remembered", &rows);
}

#[tauri::command(async)]
pub(crate) fn remembered_sessions(state: TauriState<AppState>) -> Vec<RememberedRow> {
    // A session that outlived a restart of Maya alone leaves the list on this refresh.
    crate::local_cards(&mut state.store.lock().unwrap());
    rows(&state)
}

/// Resumes `rows`, each on its own machine: this Mac's through the core,
/// each assistant's one at a time as `Resume` commands, `RESUME_GAP_MS`
/// apart; a waiting machine's rows are skipped and say so.
pub(crate) fn resume_rows(app: &AppHandle, rows: &[RememberedRow]) -> Vec<ResumeOutcome> {
    let state = app.state::<AppState>();
    let plan = remembered::plan(rows);
    let mut out = Vec::new();
    for (key, result) in maya_core::actions::resume_remembered(&crate::local(&state), &plan.here) {
        if let Some(e) = plan.here.iter().find(|e| e.key() == key) {
            out.push(ResumeOutcome::of(None, e, &result));
        }
    }
    for (machine, entries) in &plan.remote {
        for (i, e) in entries.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(Duration::from_millis(remembered::RESUME_GAP_MS));
            }
            let kind = CommandKind::Resume { dir: e.dir.clone(), session: e.session_id.clone(), agent: e.agent };
            let result = crate::net_app::send_command(app, machine, kind, ROUTE_TIMEOUT).map(|_| ());
            out.push(ResumeOutcome::of(Some(machine), e, &result));
        }
    }
    for w in &plan.waiting {
        let machine = w.machine.clone().unwrap_or_default();
        out.push(ResumeOutcome::of(Some(&machine), &w.entry, &Err(remembered::waiting_for(&machine))));
    }
    crate::refresh_and_emit(app);
    out
}

#[tauri::command(async)]
pub(crate) fn resume_remembered(app: AppHandle, rows: Vec<RememberedRow>) -> Vec<ResumeOutcome> {
    resume_rows(&app, &rows)
}

/// Forgets `rows`: this Mac's at once; each assistant's from the main's
/// memory at once and, when it is connected, by `Forget` on a thread of its
/// own (an older assistant never answers; that is logged and ignored).
#[tauri::command(async)]
pub(crate) fn forget_remembered(app: AppHandle, state: TauriState<AppState>, rows: Vec<RememberedRow>) {
    let server = state.network.lock().unwrap().server.clone();
    for (machine, waiting, keys) in remembered::keys_by_machine(&rows) {
        let Some(machine) = machine else {
            state.store.lock().unwrap().forget_remembered(&keys);
            continue;
        };
        if let Some(s) = &server {
            s.drop_remembered(&machine, &keys);
        }
        if waiting {
            continue;
        }
        let handle = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = crate::net_app::send_command(&handle, &machine, CommandKind::Forget { sessions: keys }, ROUTE_TIMEOUT) {
                log::line("remembered", format!("{machine} did not forget its sessions: {e}"));
            }
        });
    }
    crate::refresh_and_emit(&app);
}

/// Writes this run's last sign of life.
pub(crate) fn heartbeat(app: &AppHandle) {
    app.state::<AppState>().store.lock().unwrap().heartbeat(maya_core::store::now_ms());
}
```

In `src-tauri/src/lib.rs`:

- after `pub mod net_app;` add `mod remembered_app;`
- near `pub fn run()` add `/// The PR badges' poll, whose thread also writes the heartbeat. const PR_POLL: Duration = Duration::from_secs(10);`
- in `generate_handler!`, after `resume_session,`: `remembered_app::remembered_sessions, remembered_app::resume_remembered, remembered_app::forget_remembered,`
- in `setup`, right after the `log::line("app", format!("Maya {} started; log at {}", …));` line:

```rust
            // Before the first refresh: what the last run left is judged against its heartbeat.
            let offered = app.state::<AppState>().store.lock().unwrap().start_remembering(now_ms());
            log::line("app", format!("{} sessions were running when Maya last stopped", offered.len()));
```

- the PR loop becomes:

```rust
            let pr_handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut tick: u64 = 0;
                loop {
                    poll_pull_requests(&pr_handle);
                    tick += 1;
                    // The heartbeat rides this loop: every third tick, 30 s.
                    if maya_core::remembered::heartbeat_due(tick, PR_POLL.as_millis() as u64) {
                        remembered_app::heartbeat(&pr_handle);
                    }
                    std::thread::sleep(PR_POLL);
                }
            });
```

- in `refresh_and_emit`, after the `if let Some(s) = server { s.mirror(cards.clone()); }` block and before `if assistant`:

```rust
    remembered_app::publish(app);
```

In `src-tauri/src/net_app.rs`, `impl Executor for TauriExecutor`, add:

```rust
    fn remembered(&self) -> Option<Vec<maya_core::remembered::Remembered>> {
        Some(self.app.state::<AppState>().store.lock().unwrap().was_running())
    }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya`
Expected: `test result: ok.` with `the_heartbeat_rides_every_third_pr_poll` among them.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/remembered_app.rs src-tauri/src/lib.rs src-tauri/src/net_app.rs
git commit -m "feat: the desktop remembers its sessions and resumes them on each machine" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: "Maya, resume my sessions"

**Files:**
- Modify: `core/src/interpreter.rs` (prompt, `needs_confirm`, `resume_all_rows`, `validate_resume_all`, `resume_all_readback`, `resume_all_said`, tests)
- Modify: `src-tauri/src/listener.rs` (`VoiceContext.remembered`, `voice_context`, `decide`, `execute_action`, `spoken_for`, test helper, a test)
- Modify: `src-tauri/src/net_app.rs` (`revalidate`, test literals, a test)

**Interfaces:**
- Consumes: Task 1's `RememberedRow`; Task 5's `remembered_app::{rows, resume_rows}`.
- Produces:
  - `pub fn resume_all_rows(rows: &[RememberedRow], scope: &str) -> Vec<RememberedRow>` (not waiting; `here` keeps `machine == None`)
  - `pub fn validate_resume_all(action: &Value, rows: &[RememberedRow]) -> Result<Value, String>` → `{"kind":"resume_all","scope","count","here","machines":[{"machine","count"}]}`
  - `pub fn resume_all_readback(action: &Value) -> String`, `pub fn resume_all_said(resumed: usize, failed: usize) -> String`
  - `VoiceContext { cards, dirs, machines, prs, remembered: Vec<RememberedRow> }`

Decided here: a missing scope means `all`; any other scope answers "Should I resume them here or everywhere?"; one remote machine only reads "Resume 2 sessions on Gnowee?"; three groups read "Resume 3 sessions, 1 here, 1 on Gnowee and 1 on pi?"; when none could be resumed Maya says "None of them could be resumed."; from the phone, "here" means the main (the command is interpreted there).

- [ ] **Step 1: Write the failing tests**

Append to `core/src/interpreter.rs`'s `mod tests`:

```rust
    fn row(machine: Option<&str>, waiting: bool, id: &str) -> crate::remembered::RememberedRow {
        crate::remembered::RememberedRow { machine: machine.map(String::from), waiting, entry: crate::remembered::Remembered { agent: Harness::ClaudeCode, session_id: id.into(), dir: format!("/Users/x/dev/{id}"), name: id.into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None } }
    }

    #[test]
    fn resume_all_reads_back_how_many_and_where() {
        let rows = vec![row(None, false, "a"), row(None, false, "b"), row(None, false, "c"), row(Some("Gnowee"), false, "g1"), row(Some("Gnowee"), false, "g2"), row(Some("pi"), true, "p1")];
        let all = validate_resume_all(&json!({"kind": "resume_all", "scope": "all"}), &rows).unwrap();
        assert_eq!(all["count"], 5, "the one waiting for its machine is not counted");
        assert_eq!(resume_all_readback(&all), "Resume 5 sessions, 3 here and 2 on Gnowee?");
        let here = validate_resume_all(&json!({"kind": "resume_all", "scope": "here"}), &rows).unwrap();
        assert_eq!(resume_all_readback(&here), "Resume 3 sessions?");
        assert_eq!(validate_resume_all(&json!({"kind": "resume_all"}), &rows).unwrap()["scope"], "all");
        let one = validate_resume_all(&json!({"kind": "resume_all", "scope": "all"}), &[row(Some("Gnowee"), false, "g1")]).unwrap();
        assert_eq!(resume_all_readback(&one), "Resume 1 session on Gnowee?");
        let three = [row(None, false, "a"), row(Some("Gnowee"), false, "g"), row(Some("pi"), false, "p")];
        assert_eq!(resume_all_readback(&validate_resume_all(&json!({"kind": "resume_all", "scope": "all"}), &three).unwrap()), "Resume 3 sessions, 1 here, 1 on Gnowee and 1 on pi?");
    }

    #[test]
    fn resume_all_with_nothing_to_resume_says_so() {
        let waiting = [row(Some("pi"), true, "p1")];
        assert_eq!(validate_resume_all(&json!({"kind": "resume_all", "scope": "all"}), &waiting).unwrap_err(), "There's nothing to resume.");
        assert_eq!(validate_resume_all(&json!({"kind": "resume_all", "scope": "here"}), &[row(Some("Gnowee"), false, "g")]).unwrap_err(), "There's nothing to resume.");
        assert_eq!(validate_resume_all(&json!({"kind": "resume_all", "scope": "there"}), &waiting).unwrap_err(), "Should I resume them here or everywhere?");
    }

    #[test]
    fn what_maya_says_after_resuming() {
        assert_eq!(resume_all_said(5, 0), "Resuming 5.");
        assert_eq!(resume_all_said(4, 1), "Resumed 4; one couldn't be resumed.");
        assert_eq!(resume_all_said(3, 2), "Resumed 3; 2 couldn't be resumed.");
        assert_eq!(resume_all_said(0, 2), "None of them could be resumed.");
    }

    #[test]
    fn resume_all_is_confirmed_and_in_the_prompt() {
        assert!(needs_confirm(&json!({"kind": "resume_all"})));
        let p = system_prompt();
        assert!(p.contains(r#"{"kind":"resume_all","scope":"all"}"#), "{p}");
        assert!(p.contains("reply, answer, start, resume, resume_all and review"), "{p}");
    }
```

Append to `src-tauri/src/listener.rs`'s `mod tests` (after `use serde_json::json;`):

```rust
    fn remembered_row(machine: Option<&str>, id: &str) -> maya_core::remembered::RememberedRow {
        maya_core::remembered::RememberedRow { machine: machine.map(String::from), waiting: false, entry: maya_core::remembered::Remembered { agent: Harness::ClaudeCode, session_id: id.into(), dir: "/x".into(), name: id.into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None } }
    }

    #[test]
    fn resume_all_is_read_back_with_the_split_and_nothing_to_resume_is_only_said() {
        let asked = Some(json!({"kind": "resume_all", "scope": "all"}));
        let with_rows = VoiceContext { remembered: vec![remembered_row(None, "a"), remembered_row(Some("Gnowee"), "g")], ..ctx() };
        let Interpreted::Confirm(p) = decide(&reply("", asked.clone()), &with_rows) else { panic!("expected a confirmation") };
        assert_eq!(p.say, "Resume 2 sessions, 1 here and 1 on Gnowee?");
        assert_eq!(decide(&reply("", asked), &ctx()), Interpreted::Say("There's nothing to resume.".into()));
    }
```

Append to `src-tauri/src/net_app.rs`'s `mod tests`:

```rust
    #[test]
    fn a_confirmed_resume_all_is_counted_again_on_the_main() {
        let entry = maya_core::remembered::Remembered { agent: Harness::ClaudeCode, session_id: "a".into(), dir: "/x".into(), name: "a".into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None };
        let ctx = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![], remembered: vec![maya_core::remembered::RememberedRow { machine: None, waiting: false, entry }] };
        let action = serde_json::json!({"kind": "resume_all", "scope": "all", "count": 5});
        assert_eq!(revalidate(&action, &ctx).unwrap()["count"], 1, "counted against the list as it is now");
        let empty = VoiceContext { remembered: vec![], ..ctx };
        assert_eq!(revalidate(&action, &empty).unwrap_err(), "There's nothing to resume.");
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core interpreter::`
Expected: FAIL to compile: "cannot find function `validate_resume_all` in this scope".

- [ ] **Step 3: Write the implementation**

`core/src/interpreter.rs`: add `use crate::remembered::RememberedRow;`. In `system_prompt`, after the `resume` line add

```text
{"kind":"resume_all","scope":"all"}                 resume every session that was running when Maya last stopped ("resume my sessions", "bring back my sessions", "restore everything"); "scope":"here" for this machine only ("resume my sessions here")
```

and change the confirm sentence to `Set confirm to true for reply, answer, start, resume, resume_all and review, and phrase say as a read-back ending in "Yes?".`

`needs_confirm`:

```rust
pub fn needs_confirm(action: &Value) -> bool {
    matches!(action["kind"].as_str(), Some("reply") | Some("answer") | Some("start") | Some("resume") | Some("resume_all") | Some("review"))
}
```

Add after `validate`:

```rust
/// The rows "resume my sessions" resumes: every one whose machine is here
/// or connected; with `scope` "here", only this machine's.
pub fn resume_all_rows(rows: &[RememberedRow], scope: &str) -> Vec<RememberedRow> {
    rows.iter().filter(|r| !r.waiting && (scope != "here" || r.machine.is_none())).cloned().collect()
}

/// A `resume_all` checked against the "was running" list: its scope, and
/// how many it resumes here and on each machine, for the read-back.
pub fn validate_resume_all(action: &Value, rows: &[RememberedRow]) -> Result<Value, String> {
    let scope = action["scope"].as_str().unwrap_or("all");
    if scope != "all" && scope != "here" {
        return Err("Should I resume them here or everywhere?".into());
    }
    let chosen = resume_all_rows(rows, scope);
    if chosen.is_empty() {
        return Err("There's nothing to resume.".into());
    }
    let here = chosen.iter().filter(|r| r.machine.is_none()).count();
    let mut machines: Vec<(String, usize)> = vec![];
    for m in chosen.iter().filter_map(|r| r.machine.as_ref()) {
        match machines.iter_mut().find(|(n, _)| n == m) {
            Some((_, c)) => *c += 1,
            None => machines.push((m.clone(), 1)),
        }
    }
    Ok(serde_json::json!({
        "kind": "resume_all",
        "scope": scope,
        "count": chosen.len(),
        "here": here,
        "machines": machines.iter().map(|(m, c)| serde_json::json!({ "machine": m, "count": c })).collect::<Vec<_>>(),
    }))
}

/// "Resume 5 sessions, 3 here and 2 on Gnowee?"; "Resume 3 sessions?" when all are here.
pub fn resume_all_readback(action: &Value) -> String {
    let count = action["count"].as_u64().unwrap_or(0);
    let here = action["here"].as_u64().unwrap_or(0);
    let noun = if count == 1 { "session" } else { "sessions" };
    let machines: Vec<(String, u64)> = action["machines"].as_array().into_iter().flatten().map(|m| (m["machine"].as_str().unwrap_or("").to_string(), m["count"].as_u64().unwrap_or(0))).collect();
    if here == count {
        return format!("Resume {count} {noun}?");
    }
    if here == 0 && machines.len() == 1 {
        return format!("Resume {count} {noun} on {}?", machines[0].0);
    }
    let mut parts: Vec<String> = vec![];
    if here > 0 {
        parts.push(format!("{here} here"));
    }
    parts.extend(machines.iter().map(|(m, n)| format!("{n} on {m}")));
    let (last, rest) = parts.split_last().expect("two or more groups");
    format!("Resume {count} {noun}, {} and {last}?", rest.join(", "))
}

/// What Maya says once a confirmed `resume_all` ran.
pub fn resume_all_said(resumed: usize, failed: usize) -> String {
    match (resumed, failed) {
        (n, 0) => format!("Resuming {n}."),
        (0, _) => "None of them could be resumed.".into(),
        (n, 1) => format!("Resumed {n}; one couldn't be resumed."),
        (n, f) => format!("Resumed {n}; {f} couldn't be resumed."),
    }
}
```

`src-tauri/src/listener.rs`:

- `VoiceContext` gains `pub remembered: Vec<maya_core::remembered::RememberedRow>,`.
- `voice_context`, after `let prs = …;`: `let remembered = crate::remembered_app::rows(&state);` and return `VoiceContext { cards, dirs, machines, prs, remembered }`.
- `decide`, replacing its `match interpreter::validate(…) {`:

```rust
    let validated = if proposed["kind"] == "resume_all" { interpreter::validate_resume_all(proposed, &ctx.remembered) } else { interpreter::validate(proposed, &ctx.cards, &ctx.dirs, &ctx.machines, &ctx.prs) };
    match validated {
```

- `execute_action`, before the `"start"` arm:

```rust
        "resume_all" => {
            let scope = action["scope"].as_str().unwrap_or("all");
            let rows = interpreter::resume_all_rows(&crate::remembered_app::rows(&state), scope);
            if rows.is_empty() {
                return Err("There's nothing to resume.".into());
            }
            let outcomes = crate::remembered_app::resume_rows(app, &rows);
            let failed = outcomes.iter().filter(|o| o.error.is_some()).count();
            Ok(interpreter::resume_all_said(outcomes.len() - failed, failed))
        }
```

- `spoken_for`, after the `"resume"` arm: `"resume_all" => interpreter::resume_all_readback(action),`.
- The test helper `ctx()` and every other `VoiceContext { … }` literal gain `remembered: vec![]` (`grep -n "VoiceContext {" src-tauri/src/*.rs`).

`src-tauri/src/net_app.rs` `revalidate`, first lines of the body:

```rust
    if action["kind"] == "resume_all" {
        return maya_core::interpreter::validate_resume_all(action, &ctx.remembered);
    }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core interpreter:: && cargo test -p maya`
Expected: `test result: ok.` in both.

- [ ] **Step 5: Commit**

```bash
git add core/src/interpreter.rs src-tauri/src/listener.rs src-tauri/src/net_app.rs
git commit -m "feat: \"Maya, resume my sessions\" resumes what was running" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The Sessions banner on the desktop

**Files:**
- Modify: `src/types.ts` (types)
- Create: `src/remembered.ts`, `src/remembered.test.ts`
- Modify: `index.html` (host), `src/main.ts` (mount, tab), `src/styles.css` (banner rules)

**Interfaces:**
- Consumes: Task 5's commands `remembered_sessions`, `resume_remembered { rows }`, `forget_remembered { rows }`, event `remembered`; `formatAge`, `harnessBadge`, `showToast`.
- Produces (TS):
  - `interface Remembered { agent; sessionId; dir; name; firstSeenMs; lastSeenMs; goneAtMs?; offered?; error? }`, `interface RememberedRow extends Remembered { machine?; waiting? }`, `interface ResumeOutcome { machine?; agent; sessionId; name; error? }` in `src/types.ts`
  - `rowKey(r) -> string`, `tildePath(dir) -> string`, `bannerText(rows) -> string`, `failuresText(rows) -> string`, `resumeToast(outcomes) -> string`, `renderBanner(m: BannerModel, h: BannerHandlers, nowMs) -> HTMLElement | null`, `initRemembered(host: HTMLElement): void`

Decided here: Choose… opens the list inside the banner rather than a separate dialog; failed rows have their own line and Dismiss; Resume all sends waiting rows too so the result toast can say they were skipped.

- [ ] **Step 1: Write the failing tests**

Add to `src/types.ts`:

```ts
/** A session that was running when its machine's Maya last stopped. */
export interface Remembered {
  agent: Harness;
  sessionId: string;
  dir: string;
  name: string;
  firstSeenMs: number;
  lastSeenMs: number;
  goneAtMs?: number | null;
  offered?: boolean;
  /** Why its last resume failed ("folder missing"…). */
  error?: string | null;
}

/** A banner row: `machine` is absent for this machine; `waiting` while that machine is away. */
export interface RememberedRow extends Remembered {
  machine?: string | null;
  waiting?: boolean;
}

/** What became of one row of a resume; `error` is its reason. */
export interface ResumeOutcome {
  machine?: string | null;
  agent: Harness;
  sessionId: string;
  name: string;
  error?: string | null;
}
```

Create `src/remembered.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  calls: [] as { cmd: string; args: unknown }[],
  replies: {} as Record<string, unknown>,
  listeners: {} as Record<string, (e: { payload: unknown }) => void>,
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: unknown) => {
    tauri.calls.push({ cmd, args });
    return tauri.replies[cmd];
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, f: (e: { payload: unknown }) => void) => {
    tauri.listeners[name] = f;
    return () => undefined;
  }),
}));

import { bannerText, failuresText, initRemembered, renderBanner, resumeToast, rowKey, tildePath, type BannerModel } from "./remembered";
import type { RememberedRow } from "./types";

const NOW = Date.parse("2026-10-19T09:00:00Z");
const row = (id: string, machine: string | null, extra: Partial<RememberedRow> = {}): RememberedRow => ({ agent: "claude-code", sessionId: id, dir: `/Users/x/dev/${id}`, name: `${id}-name`, firstSeenMs: NOW - 3_600_000, lastSeenMs: NOW - 300_000, offered: true, machine, ...extra });
const handlers = () => ({ onResumeAll: vi.fn(), onChoose: vi.fn(), onDismiss: vi.fn(), onDismissFailed: vi.fn(), onTick: vi.fn(), onResumeChosen: vi.fn(), onCancel: vi.fn(), onForgetUnticked: vi.fn() });
const model = (rows: RememberedRow[], extra: Partial<BannerModel> = {}): BannerModel => ({ rows, choosing: false, ticked: new Set(), busy: false, ...extra });
const flush = () => new Promise((r) => setTimeout(r, 0));
const click = (root: ParentNode, action: string) => root.querySelector<HTMLButtonElement>(`button[data-action=${action}]`)!.click();

describe("banner text", () => {
  it("counts the sessions and splits them by machine only when more than one is involved", () => {
    const five = [row("a", null), row("b", null), row("c", null), row("g1", "Gnowee"), row("g2", "Gnowee")];
    expect(bannerText(five)).toBe("5 sessions were running when Maya last stopped · 3 here · 2 on Gnowee");
    expect(bannerText([row("a", null)])).toBe("1 session was running when Maya last stopped");
    expect(bannerText([row("a", "desk"), row("b", "desk")])).toBe("2 sessions were running when Maya last stopped");
  });

  it("shortens home folders to ~ on every platform", () => {
    expect(tildePath("/Users/x/dev/calibre")).toBe("~/dev/calibre");
    expect(tildePath("/home/tiago/dev")).toBe("~/dev");
    expect(tildePath("C:\\Users\\tiago\\dev")).toBe("~\\dev");
    expect(tildePath("/opt/work")).toBe("/opt/work");
  });

  it("names each failure with its reason", () => {
    expect(failuresText([row("raffinad-3c", null, { name: "raffinad-3c", error: "folder missing" })])).toBe("Couldn't resume 1: raffinad-3c (folder missing)");
  });

  it("sums up a resume, naming a single failure and the machines skipped", () => {
    expect(resumeToast([{ agent: "claude-code", sessionId: "a", name: "a" }, { agent: "claude-code", sessionId: "b", name: "b" }])).toBe("Resuming 2.");
    expect(resumeToast([{ agent: "claude-code", sessionId: "a", name: "a" }, { agent: "claude-code", sessionId: "g", name: "g", machine: "Gnowee", error: "waiting for Gnowee" }])).toBe("Resuming 1; skipped 1 waiting for Gnowee.");
    expect(resumeToast([{ agent: "codex", sessionId: "c", name: "calibre-88", error: "Codex is not installed" }])).toBe("Couldn't resume calibre-88: Codex is not installed");
    expect(resumeToast([])).toBe("Nothing to resume.");
  });
});

describe("renderBanner", () => {
  it("shows nothing for an empty list, and the three buttons otherwise", () => {
    expect(renderBanner(model([]), handlers(), NOW)).toBeNull();
    const h = handlers();
    const el = renderBanner(model([row("a", null)]), h, NOW)!;
    click(el, "resume-all");
    click(el, "choose");
    click(el, "dismiss");
    expect([h.onResumeAll, h.onChoose, h.onDismiss].every((f) => f.mock.calls.length === 1)).toBe(true);
  });

  it("lists the rows by machine, ticked, with a waiting machine's rows unticked and saying so", () => {
    const rows = [row("a", null), row("g1", "Gnowee", { waiting: true })];
    const h = handlers();
    const el = renderBanner(model(rows, { choosing: true, ticked: new Set([rowKey(rows[0])]) }), h, NOW)!;
    expect([...el.querySelectorAll(".remembered__machine")].map((m) => m.textContent)).toEqual(["Here", "Gnowee"]);
    const boxes = [...el.querySelectorAll<HTMLInputElement>(".remembered__row input")];
    expect(boxes.map((b) => [b.checked, b.disabled])).toEqual([[true, false], [false, true]]);
    expect(el.querySelector(".remembered__dir")!.textContent).toBe("~/dev/a");
    expect([...el.querySelectorAll(".remembered__age")].map((a) => a.textContent)).toEqual(["5m ago", "waiting for Gnowee"]);
    expect(el.querySelector<HTMLButtonElement>("button[data-action=resume-chosen]")!.textContent).toBe("Resume 1");
    click(el, "cancel");
    click(el, "forget-unticked");
    expect(h.onCancel).toHaveBeenCalled();
    expect(h.onForgetUnticked).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=resume-all]")).toBeNull();
  });

  it("puts failures on their own line with a Dismiss", () => {
    const h = handlers();
    const el = renderBanner(model([row("lost", null, { name: "raffinad-3c", error: "folder missing" })]), h, NOW)!;
    expect(el.textContent).toContain("Couldn't resume 1: raffinad-3c (folder missing)");
    expect(el.querySelector("button[data-action=resume-all]")).toBeNull();
    click(el, "dismiss-failed");
    expect(h.onDismissFailed).toHaveBeenCalled();
  });
});

describe("initRemembered", () => {
  const start = async (rows: RememberedRow[]) => {
    tauri.calls = [];
    tauri.replies = { remembered_sessions: rows, resume_remembered: [], forget_remembered: null };
    const host = document.createElement("div");
    initRemembered(host);
    await flush();
    return host;
  };
  const sent = (cmd: string) => (tauri.calls.filter((c) => c.cmd === cmd).pop()?.args as { rows: RememberedRow[] }).rows.map(rowKey);

  it("Resume all sends every row not failed, waiting ones included so the toast can say so", async () => {
    const rows = [row("a", null), row("g", "Gnowee", { waiting: true }), row("x", null, { error: "folder missing" })];
    const host = await start(rows);
    click(host, "resume-all");
    await flush();
    expect(sent("resume_remembered")).toEqual([rowKey(rows[0]), rowKey(rows[1])]);
  });

  it("Choose sends the ticked rows to Resume and the unticked ones to Forget", async () => {
    const rows = [row("a", null), row("b", null), row("g", "Gnowee", { waiting: true })];
    const host = await start(rows);
    click(host, "choose");
    const b = host.querySelector<HTMLInputElement>(`[data-key="${rowKey(rows[1])}"] input`)!;
    b.checked = false;
    b.dispatchEvent(new Event("change"));
    expect(host.querySelector<HTMLButtonElement>("button[data-action=resume-chosen]")!.textContent).toBe("Resume 1");
    click(host, "forget-unticked");
    await flush();
    expect(sent("forget_remembered")).toEqual([rowKey(rows[1])]);
  });

  it("the same session id on two machines is ticked and resumed separately", async () => {
    const rows = [row("same", null), row("same", "Gnowee")];
    const host = await start(rows);
    click(host, "choose");
    const remote = host.querySelector<HTMLInputElement>(`[data-key="${rowKey(rows[1])}"] input`)!;
    remote.checked = false;
    remote.dispatchEvent(new Event("change"));
    expect(host.querySelector<HTMLInputElement>(`[data-key="${rowKey(rows[0])}"] input`)!.checked).toBe(true);
    click(host, "resume-chosen");
    await flush();
    expect(sent("resume_remembered")).toEqual([rowKey(rows[0])]);
  });

  it("repaints from the remembered event", async () => {
    const host = await start([]);
    expect(host.childElementCount).toBe(0);
    tauri.listeners.remembered({ payload: [row("a", null)] });
    expect(host.textContent).toContain("1 session was running when Maya last stopped");
  });
});
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test src/remembered.test.ts`
Expected: FAIL: "Failed to resolve import \"./remembered\"".

- [ ] **Step 3: Write the implementation**

Create `src/remembered.ts`:

```ts
// The Sessions banner: the sessions that were running when Maya last
// stopped, on this machine and its assistants, with Resume all, Choose…
// and Dismiss. The desktop and the phone both mount it.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatAge } from "./format";
import { harnessBadge } from "./harness";
import { showToast } from "./toast";
import type { RememberedRow, ResumeOutcome } from "./types";

/** One row's identity across machines: the same id may be on two. */
export function rowKey(r: Pick<RememberedRow, "machine" | "agent" | "sessionId">): string {
  return `${r.machine ?? ""}|${r.agent}|${r.sessionId}`;
}

/** `/Users/x/dev/calibre` as `~/dev/calibre`; Linux's `/home/x` and Windows' `C:\Users\x` too. */
export function tildePath(dir: string): string {
  return dir.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~").replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/, "~");
}

/** "5 sessions were running when Maya last stopped · 3 here · 2 on Gnowee". */
export function bannerText(rows: RememberedRow[]): string {
  const n = rows.length;
  const head = `${n} ${n === 1 ? "session was" : "sessions were"} running when Maya last stopped`;
  const groups = new Map<string, number>();
  for (const r of rows) {
    const label = r.machine ? `on ${r.machine}` : "here";
    groups.set(label, (groups.get(label) ?? 0) + 1);
  }
  if (groups.size < 2) return head;
  return `${head} · ${[...groups].map(([label, count]) => `${count} ${label}`).join(" · ")}`;
}

/** "Couldn't resume 1: raffinad-3c (folder missing)". */
export function failuresText(rows: RememberedRow[]): string {
  return `Couldn't resume ${rows.length}: ${rows.map((r) => `${r.name} (${r.error})`).join(", ")}`;
}

/** The toast after a resume: what started, what waits for its machine, what failed. */
export function resumeToast(outcomes: ResumeOutcome[]): string {
  const waiting = outcomes.filter((o) => o.error?.startsWith("waiting for "));
  const failed = outcomes.filter((o) => o.error && !o.error.startsWith("waiting for "));
  const ok = outcomes.length - waiting.length - failed.length;
  if (outcomes.length === 1 && failed.length === 1) return `Couldn't resume ${failed[0].name}: ${failed[0].error}`;
  const parts: string[] = [];
  if (ok > 0) parts.push(`Resuming ${ok}`);
  if (waiting.length > 0) parts.push(`skipped ${waiting.length} waiting for ${[...new Set(waiting.map((o) => o.machine ?? ""))].join(" and ")}`);
  if (failed.length > 0) parts.push(`${failed.length} couldn't be resumed`);
  if (parts.length === 0) return "Nothing to resume.";
  const text = parts.join("; ");
  return `${text.charAt(0).toUpperCase()}${text.slice(1)}.`;
}

export interface BannerModel {
  rows: RememberedRow[];
  /** The Choose list is open. */
  choosing: boolean;
  /** `rowKey`s ticked in the Choose list. */
  ticked: Set<string>;
  /** A resume or forget is running. */
  busy: boolean;
}

export interface BannerHandlers {
  onResumeAll(): void;
  onChoose(): void;
  onDismiss(): void;
  onDismissFailed(): void;
  onTick(key: string, on: boolean): void;
  onResumeChosen(): void;
  onCancel(): void;
  onForgetUnticked(): void;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function button(label: string, action: string, onClick: () => void, primary = false): HTMLButtonElement {
  const b = el("button", primary ? "card__btn card__btn--primary" : "card__btn", label);
  b.type = "button";
  b.dataset.action = action;
  b.addEventListener("click", onClick);
  return b;
}

/** The Choose list: grouped by machine, each row ticked unless its machine is away. */
function renderChoose(rows: RememberedRow[], m: BannerModel, h: BannerHandlers, nowMs: number): HTMLElement {
  const list = el("div", "remembered__choose");
  const groups = new Map<string, RememberedRow[]>();
  for (const r of rows) groups.set(r.machine ?? "", [...(groups.get(r.machine ?? "") ?? []), r]);
  for (const [machine, group] of groups) {
    list.append(el("h3", "remembered__machine", machine || "Here"));
    for (const r of group) {
      const key = rowKey(r);
      const line = el("label", "remembered__row");
      line.dataset.key = key;
      const box = el("input", "");
      box.type = "checkbox";
      box.checked = !r.waiting && m.ticked.has(key);
      box.disabled = !!r.waiting;
      box.addEventListener("change", () => h.onTick(key, box.checked));
      line.append(box, el("span", "remembered__name", r.name), el("span", "remembered__dir", tildePath(r.dir)), el("span", "remembered__age", r.waiting ? `waiting for ${r.machine}` : `${formatAge(r.lastSeenMs, nowMs)} ago`), harnessBadge(r.agent, "remembered__agent"));
      list.append(line);
    }
  }
  const n = rows.filter((r) => !r.waiting && m.ticked.has(rowKey(r))).length;
  const actions = el("div", "remembered__actions");
  const resume = button(`Resume ${n}`, "resume-chosen", h.onResumeChosen, true);
  resume.disabled = n === 0;
  actions.append(resume, button("Cancel", "cancel", h.onCancel), button("Forget unticked", "forget-unticked", h.onForgetUnticked));
  list.append(actions);
  return list;
}

/** The banner, or null when nothing was running. */
export function renderBanner(m: BannerModel, h: BannerHandlers, nowMs: number): HTMLElement | null {
  const pending = m.rows.filter((r) => !r.error);
  const failed = m.rows.filter((r) => r.error);
  if (pending.length === 0 && failed.length === 0) return null;
  const root = el("section", "remembered");
  if (pending.length > 0) {
    const line = el("div", "remembered__line");
    line.append(el("span", "remembered__text", bannerText(pending)));
    if (!m.choosing) line.append(button("Resume all", "resume-all", h.onResumeAll, true), button("Choose…", "choose", h.onChoose), button("Dismiss", "dismiss", h.onDismiss));
    root.append(line);
    if (m.choosing) root.append(renderChoose(pending, m, h, nowMs));
  }
  if (failed.length > 0) {
    const line = el("div", "remembered__line remembered__line--failed");
    line.append(el("span", "remembered__text", failuresText(failed)), button("Dismiss", "dismiss-failed", h.onDismissFailed));
    root.append(line);
  }
  if (m.busy) for (const b of root.querySelectorAll("button")) b.disabled = true;
  return root;
}

/** Mounts the banner into `host` and keeps it current from the `remembered` event. */
export function initRemembered(host: HTMLElement): void {
  const m: BannerModel = { rows: [], choosing: false, ticked: new Set(), busy: false };
  let shown = "";
  const paint = () => {
    const banner = renderBanner(m, handlers, Date.now());
    host.replaceChildren(...(banner ? [banner] : []));
  };
  const set = (rows: RememberedRow[]) => {
    const text = JSON.stringify(rows);
    if (text === shown) return;
    shown = text;
    m.rows = rows;
    const keys = new Set(rows.map(rowKey));
    m.ticked = new Set([...m.ticked].filter((k) => keys.has(k)));
    if (m.choosing && !rows.some((r) => !r.error)) m.choosing = false;
    paint();
  };
  const act = async (f: () => Promise<void>) => {
    m.busy = true;
    paint();
    try {
      await f();
    } catch (e) {
      showToast(String(e));
    }
    m.busy = false;
    paint();
  };
  const resume = (rows: RememberedRow[]) =>
    act(async () => {
      const outcomes = await invoke<ResumeOutcome[]>("resume_remembered", { rows });
      m.choosing = false;
      showToast(resumeToast(outcomes ?? []));
    });
  const forget = (rows: RememberedRow[]) =>
    act(async () => {
      await invoke("forget_remembered", { rows });
      m.choosing = false;
    });
  const pending = () => m.rows.filter((r) => !r.error);
  const handlers: BannerHandlers = {
    onResumeAll: () => void resume(pending()),
    onChoose: () => {
      m.choosing = true;
      m.ticked = new Set(pending().filter((r) => !r.waiting).map(rowKey));
      paint();
    },
    onDismiss: () => void forget(pending()),
    onDismissFailed: () => void forget(m.rows.filter((r) => r.error)),
    onTick: (key, on) => {
      if (on) m.ticked.add(key);
      else m.ticked.delete(key);
      paint();
    },
    onResumeChosen: () => void resume(pending().filter((r) => !r.waiting && m.ticked.has(rowKey(r)))),
    onCancel: () => {
      m.choosing = false;
      paint();
    },
    onForgetUnticked: () => void forget(pending().filter((r) => !r.waiting && !m.ticked.has(rowKey(r)))),
  };
  void listen<RememberedRow[]>("remembered", (e) => set(e.payload));
  void invoke<RememberedRow[]>("remembered_sessions").then((rows) => set(rows ?? [])).catch(() => undefined);
  // The Choose list's "N ago" moves with time.
  setInterval(() => {
    if (m.choosing) paint();
  }, 30_000);
}
```

`index.html`: add `<div id="remembered" class="remembered-host"></div>` between `<div id="session-search" …>` and `<div id="board"></div>`.

`src/main.ts`: `import { initRemembered } from "./remembered";`, and in `start()` replace the tabs block with:

```ts
  const searchHost = document.getElementById("session-search");
  if (searchHost) search = makeSessionSearch(searchHost, () => paint());
  const rememberedHost = document.getElementById("remembered");
  if (rememberedHost) initRemembered(rememberedHost);
  // The search box and the banner belong to Sessions but sit outside the board pane.
  tabs = makeTabs((tab) => {
    if (searchHost) searchHost.hidden = tab !== "sessions";
    if (rememberedHost) rememberedHost.hidden = tab !== "sessions";
  });
```

`src/styles.css`, appended:

```css
/* The Sessions banner: what was running when Maya last stopped. */
.remembered-host:empty { display: none; }
.remembered { margin: 8px 16px 0; padding: 10px 14px; border: 1px solid var(--gold-deep); border-radius: 10px; background: color-mix(in srgb, var(--gold) 8%, var(--panel)); display: flex; flex-direction: column; gap: 8px; font-size: 13px; }
.remembered__line { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.remembered__text { flex: 1; min-width: 200px; }
.remembered__line--failed .remembered__text { color: var(--muted); }
.remembered__choose { display: flex; flex-direction: column; gap: 4px; }
.remembered__machine { margin: 6px 0 2px; font-size: 12px; font-weight: 600; color: var(--gold); }
.remembered__row { display: grid; grid-template-columns: auto minmax(0, 1fr) minmax(0, 1.4fr) auto auto; align-items: center; gap: 8px; }
.remembered__dir, .remembered__age { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.remembered__actions { display: flex; gap: 8px; margin-top: 6px; }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: all test files pass (the new `src/remembered.test.ts` with 11 tests); tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add src/types.ts src/remembered.ts src/remembered.test.ts index.html src/main.ts src/styles.css
git commit -m "feat: a Sessions banner offers back what was running" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The phone shows the banner

**Files:**
- Modify: `mobile/src/hub.rs` (`MirrorView.remembered`, `Sink::remembered`, `remembered`, `resume_remembered`, `forget_remembered`, `refresh`, `HubClientNotify`, tests)
- Modify: `mobile/src/lib.rs` (`TauriSink::remembered`, the three commands)
- Modify: `mobile/src/commands.rs` (the three commands)
- Modify: `mobile/web/index.html`, `src/mobile/main.ts` (mount the banner)

**Interfaces:**
- Consumes: Task 4's `Down::Mirror.remembered` via `ClientNotify::mirrored_remembered`, `merge::remembered`, `ServerHandle::{mirror_remembered, drop_remembered}`, `CommandKind::Forget`; Task 1's `plan`, `keys_by_machine`, `ResumeOutcome::of`, `waiting_for`, `RESUME_GAP_MS`; Task 7's `initRemembered`.
- Produces: `Hub::remembered(&self) -> Vec<RememberedRow>`, `Hub::resume_remembered(&self, rows: &[RememberedRow]) -> Vec<ResumeOutcome>`, `Hub::forget_remembered(&self, rows: &[RememberedRow])`, `Sink::remembered(&self, rows: &[RememberedRow])` (default no-op); phone commands `remembered_sessions`, `resume_remembered`, `forget_remembered` with the desktop's names and arguments.

Decided here: a phone that is itself a main (not joined) shows its assistants' lists from its own server's boards; it has no sessions, so a row with no machine fails with "This phone runs no sessions.".

- [ ] **Step 1: Write the failing tests**

Append to `mobile/src/hub.rs`'s `mod tests`:

```rust
    fn remembered_row(machine: Option<&str>, waiting: bool, id: &str) -> maya_core::remembered::RememberedRow {
        maya_core::remembered::RememberedRow { machine: machine.map(String::from), waiting, entry: maya_core::remembered::Remembered { agent: maya_core::model::Harness::ClaudeCode, session_id: id.into(), dir: "/x".into(), name: id.into(), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, error: None } }
    }

    #[test]
    fn a_joined_phone_shows_the_main_s_rows_and_skips_the_ones_waiting() {
        let hub = test_hub();
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("desk".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        let rows = vec![remembered_row(Some("Gnowee"), true, "g1")];
        let notify = HubClientNotify { hub: Arc::downgrade(&hub), stop: Arc::new(AtomicBool::new(false)) };
        notify.mirrored_remembered(rows.clone());
        notify.mirrored(true, vec![], vec![]);
        assert_eq!(hub.remembered(), rows, "the board that follows keeps the rows");
        let out = hub.resume_remembered(&rows);
        assert_eq!(out[0].error.as_deref(), Some("waiting for Gnowee"));
    }

    #[test]
    fn a_phone_main_has_no_sessions_of_its_own_to_resume() {
        let hub = test_hub();
        assert!(hub.remembered().is_empty(), "no server, no assistants");
        let out = hub.resume_remembered(&[remembered_row(None, false, "l1")]);
        assert_eq!(out[0].error.as_deref(), Some("This phone runs no sessions."));
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-mobile hub::tests::a_joined_phone`
Expected: FAIL to compile: "no method named `remembered` found for struct `Arc<Hub>`".

- [ ] **Step 3: Write the implementation**

`mobile/src/hub.rs`:

- imports: `use maya_core::remembered::{self, RememberedRow, ResumeOutcome};`
- `Sink` gains

```rust
    /// The "was running" list for the banner.
    fn remembered(&self, _rows: &[RememberedRow]) {}
```

- `MirrorView` gains `pub remembered: Vec<RememberedRow>,`.
- in `impl Hub`, after `cards`:

```rust
    /// The "was running" list the banner shows: the joined main's, or every
    /// assistant's as this phone's own server holds them.
    pub fn remembered(&self) -> Vec<RememberedRow> {
        if self.joined() {
            return self.mirror.lock().unwrap().remembered.clone();
        }
        merge::remembered(vec![], &self.boards())
    }

    /// Resumes `rows`, each on its own machine, one at a time,
    /// `RESUME_GAP_MS` apart; a waiting machine's rows are skipped.
    pub fn resume_remembered(&self, rows: &[RememberedRow]) -> Vec<ResumeOutcome> {
        let plan = remembered::plan(rows);
        let mut out = Vec::new();
        for e in &plan.here {
            out.push(ResumeOutcome::of(None, e, &Err("This phone runs no sessions.".into())));
        }
        for (machine, entries) in &plan.remote {
            for (i, e) in entries.iter().enumerate() {
                if i > 0 {
                    std::thread::sleep(Duration::from_millis(remembered::RESUME_GAP_MS));
                }
                let kind = CommandKind::Resume { dir: e.dir.clone(), session: e.session_id.clone(), agent: e.agent };
                out.push(ResumeOutcome::of(Some(machine), e, &self.send_to(machine, kind).map(|_| ())));
            }
        }
        for w in &plan.waiting {
            let machine = w.machine.clone().unwrap_or_default();
            out.push(ResumeOutcome::of(Some(&machine), &w.entry, &Err(remembered::waiting_for(&machine))));
        }
        self.refresh();
        out
    }

    /// Forgets `rows` on their machines; as a main, also from what its own
    /// server holds, so a machine that is away leaves the banner too.
    pub fn forget_remembered(&self, rows: &[RememberedRow]) {
        for (machine, waiting, keys) in remembered::keys_by_machine(rows) {
            let Some(machine) = machine else { continue };
            if let Some(s) = self.handle() {
                s.drop_remembered(&machine, &keys);
            }
            if waiting {
                continue;
            }
            if let Err(e) = self.send_to(&machine, CommandKind::Forget { sessions: keys }) {
                log::line("remembered", format!("{machine} did not forget its sessions: {e}"));
            }
        }
        self.refresh();
    }
```

- in `refresh`, after `self.sink.sessions(&cards);`:

```rust
        let rows = self.remembered();
        if let Some(s) = self.handle() {
            s.mirror_remembered(rows.clone());
        }
        self.sink.remembered(&rows);
```

- in `impl ClientNotify for HubClientNotify`, add

```rust
    fn mirrored_remembered(&self, rows: Vec<RememberedRow>) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        if let Some(h) = self.hub.upgrade() {
            h.mirror.lock().unwrap().remembered = rows;
        }
    }
```

  and in `mirrored`, replace `*h.mirror.lock().unwrap() = MirrorView { cards, machines };` with

```rust
        {
            let mut view = h.mirror.lock().unwrap();
            view.cards = cards;
            view.machines = machines;
        }
```

`mobile/src/commands.rs`:

```rust
#[tauri::command]
pub fn remembered_sessions(hub: H) -> Vec<maya_core::remembered::RememberedRow> {
    hub.remembered()
}

#[tauri::command(async)]
pub fn resume_remembered(hub: H, rows: Vec<maya_core::remembered::RememberedRow>) -> Vec<maya_core::remembered::ResumeOutcome> {
    hub.resume_remembered(&rows)
}

#[tauri::command(async)]
pub fn forget_remembered(hub: H, rows: Vec<maya_core::remembered::RememberedRow>) {
    hub.forget_remembered(&rows)
}
```

`mobile/src/lib.rs`: in `impl Sink for TauriSink`

```rust
    fn remembered(&self, rows: &[maya_core::remembered::RememberedRow]) {
        let _ = self.app.emit("remembered", rows);
    }
```

and in `generate_handler!`, after `commands::resume_session,`: `commands::remembered_sessions, commands::resume_remembered, commands::forget_remembered,`.

`mobile/web/index.html`: add `<div id="remembered" class="remembered-host"></div>` after `<div id="voice-host"></div>`.

`src/mobile/main.ts`: `import { initRemembered } from "../remembered";`; in `start()`, before `tabs = makeTabs(…)`:

```ts
  const rememberedHost = document.getElementById("remembered");
  if (rememberedHost) initRemembered(rememberedHost);
```

and inside the `makeTabs` callback: `if (rememberedHost) rememberedHost.hidden = tab !== "sessions";`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-mobile && pnpm test && pnpm exec tsc --noEmit`
Expected: `test result: ok.` (with the two new hub tests), all vitest files pass, tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add mobile/src/hub.rs mobile/src/lib.rs mobile/src/commands.rs mobile/web/index.html src/mobile/main.ts
git commit -m "feat: the phone shows the banner and resumes on each machine" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: A terminal look for the session viewer

**Files:**
- Modify: `core/src/config.rs` (`ViewerTheme`, `Config.viewer_theme`, test)
- Modify: `src/types.ts` (`ViewerTheme`)
- Modify: `src/modal.ts` (markers, ⎿ line, prompt, toggle, `setViewerTheme`, `viewerTheme`, `saveViewerTheme`, paint)
- Modify: `src/styles.css` (both looks)
- Test: `src/modal.test.ts`, `src/modal-open.test.ts`

**Interfaces:**
- Consumes: commands `get_config` / `set_config` (desktop and phone), the `maya:config-changed` window event Settings listens to.
- Produces:
  - Rust: `pub enum ViewerTheme { Chat, Terminal }` (serde lowercase, default `Chat`), `Config.viewer_theme: ViewerTheme` (`#[serde(default)]`, JSON `viewerTheme`)
  - TS: `export type ViewerTheme = "chat" | "terminal"`; in `src/modal.ts`: `setViewerTheme(t: ViewerTheme): void` (redraws an open panel at once), `viewerTheme(): ViewerTheme`, `saveViewerTheme(t: ViewerTheme): Promise<void>`, `THEME_TITLE`; `ModalModel.theme?: ViewerTheme`; `ModalHandlers.onToggleTheme?(): void`

Decided here: the markers, the `⎿` line and the `>` prompt are always rendered and the stylesheet hides them in the chat look, so a switch only toggles the root class. Transcripts carry no tool results today; a tool turn's lines after its first are drawn as its result under `⎿` (none do yet).

- [ ] **Step 1: Write the failing tests**

Append to `core/src/config.rs`'s `mod tests`:

```rust
    #[test]
    fn the_viewer_look_defaults_to_chat_and_round_trips() {
        assert_eq!(Config::default().viewer_theme, ViewerTheme::Chat);
        let older: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert_eq!(older.viewer_theme, ViewerTheme::Chat);
        let set = Config { viewer_theme: ViewerTheme::Terminal, ..Config::default() };
        let text = serde_json::to_string(&set).unwrap();
        assert!(text.contains(r#""viewerTheme":"terminal""#), "{text}");
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap().viewer_theme, ViewerTheme::Terminal);
    }
```

Append to `src/modal.test.ts` (and add `setViewerTheme` to its import from `./modal`; add `import styles from "./styles.css?raw";` at the top):

```ts
describe("viewer theme", () => {
  it("draws the terminal look's markers and prompt, the chat look without the class", () => {
    const term = renderModal({ card: base, turns, status: null, draft: "", theme: "terminal" }, handlers());
    expect(term.classList.contains("modal--terminal")).toBe(true);
    expect([...term.querySelectorAll(".turn")].map((t) => t.querySelector(".turn__mark")?.textContent ?? "")).toEqual([">", "⏺", "⏺", ""]);
    expect(term.querySelector(".modal__prompt")?.textContent).toBe(">");
    const chat = renderModal({ card: base, turns, status: null, draft: "" }, handlers());
    expect(chat.classList.contains("modal--terminal")).toBe(false);
  });

  it("hides the markers in the chat look and draws the terminal look's colours and font", () => {
    expect(styles).toContain(".turn__mark, .turn__result-mark, .modal__prompt { display: none; }");
    expect(styles).toContain(".modal--terminal .turn__mark { display: inline;");
    expect(styles).toContain(".modal--terminal .modal__panel { background: #0a0a0a; color: #e8e2d4; }");
    expect(styles).toContain("ui-monospace, SFMono-Regular, Menlo, monospace");
  });

  it("puts a tool's result under a ⎿ line when the transcript has one", () => {
    const el = renderModal({ card: base, turns: [{ kind: "tool", text: "Bash: ls\nsrc\ntests" }], status: null, draft: "", theme: "terminal" }, handlers());
    expect(el.querySelector(".turn--tool .turn__text")?.textContent).toBe("Bash: ls");
    expect(el.querySelector(".turn--tool .turn__result")?.textContent).toBe("⎿ src\ntests");
    const single = renderModal({ card: base, turns: [{ kind: "tool", text: "Bash: ls" }], status: null, draft: "" }, handlers());
    expect(single.querySelector(".turn__result")).toBeNull();
  });

  it("the header toggle asks for the other look, titled for what it switches to", () => {
    const h = { ...handlers(), onToggleTheme: vi.fn() };
    const chat = renderModal({ card: base, turns, status: null, draft: "" }, h);
    const toggle = chat.querySelector<HTMLButtonElement>("button[data-action=theme]")!;
    expect(toggle.title).toBe("Terminal look");
    toggle.click();
    expect(h.onToggleTheme).toHaveBeenCalled();
    expect(renderModal({ card: base, turns, status: null, draft: "", theme: "terminal" }, h).querySelector<HTMLButtonElement>("button[data-action=theme]")!.title).toBe("Chat look");
  });

  it("setViewerTheme redraws an open panel at once", () => {
    const host = document.createElement("div");
    host.id = "modal-host";
    document.body.append(host);
    host.append(renderModal({ card: base, turns, status: null, draft: "" }, handlers()));
    setViewerTheme("terminal");
    expect(host.querySelector(".modal")!.classList.contains("modal--terminal")).toBe(true);
    expect(host.querySelector<HTMLButtonElement>(".modal__theme")!.title).toBe("Chat look");
    setViewerTheme("chat");
    expect(host.querySelector(".modal")!.classList.contains("modal--terminal")).toBe(false);
    host.remove();
  });
});
```

Append to `src/modal-open.test.ts` (and import `setViewerTheme` from `./modal` alongside `openModal`):

```ts
describe("the viewer's look", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="modal-host"></div>';
    invoke.mockReset();
  });

  it("the header toggle switches the open panel at once and saves the look", async () => {
    let saved: unknown = null;
    invoke.mockImplementation((cmd: string, args?: { config?: Record<string, unknown> }) => {
      if (cmd === "session_history") return Promise.resolve([]);
      if (cmd === "get_config") return Promise.resolve({ completedTimeoutMinutes: 30, viewerTheme: "chat" });
      if (cmd === "set_config") {
        saved = args?.config;
        return Promise.resolve(args?.config);
      }
      return Promise.reject(new Error(cmd));
    });
    await openModal({ ...kiro, harness: "claude-code" });
    document.querySelector<HTMLButtonElement>("button[data-action=theme]")!.click();
    expect(document.querySelector(".modal")!.classList.contains("modal--terminal")).toBe(true);
    await flush();
    await flush();
    expect(saved).toEqual({ completedTimeoutMinutes: 30, viewerTheme: "terminal" });
    setViewerTheme("chat");
  });
});
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core config::tests::the_viewer_look && pnpm test src/modal.test.ts src/modal-open.test.ts`
Expected: Rust FAILS to compile ("cannot find type `ViewerTheme`"); vitest FAILS ("setViewerTheme is not a function" / missing `.turn__mark`).

- [ ] **Step 3: Write the implementation**

`core/src/config.rs`, after `VoiceProvider`:

```rust
/// How the session viewer draws a conversation: as chat, or like the agent's own terminal.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewerTheme {
    #[default]
    Chat,
    Terminal,
}
```

In `Config`, before `pub network`:

```rust
    /// The session viewer's look; the phone keeps its own.
    #[serde(default)]
    pub viewer_theme: ViewerTheme,
```

and `viewer_theme: ViewerTheme::Chat,` in `impl Default for Config`.

`src/types.ts`:

```ts
/** How the session viewer draws a conversation. */
export type ViewerTheme = "chat" | "terminal";
```

`src/modal.ts`:

- import `type ViewerTheme` from `./types` (add it to the existing `./types` import).
- `ModalModel` gains `/** The viewer's look; chat when absent. */ theme?: ViewerTheme;` and `ModalHandlers` gains `/** Switch between the chat and the terminal look. */ onToggleTheme?(): void;`.
- after `setComposerOptions`:

```ts
/** The toggle's title: the look it switches to. */
export const THEME_TITLE: Record<ViewerTheme, string> = { chat: "Terminal look", terminal: "Chat look" };

let theme: ViewerTheme = "chat";

export function viewerTheme(): ViewerTheme {
  return theme;
}

/** Sets the viewer's look; an open panel is redrawn at once. */
export function setViewerTheme(t: ViewerTheme): void {
  theme = t;
  const root = document.querySelector<HTMLElement>("#modal-host .modal");
  if (!root) return;
  root.classList.toggle("modal--terminal", t === "terminal");
  const toggle = root.querySelector<HTMLButtonElement>(".modal__theme");
  if (toggle) {
    toggle.title = THEME_TITLE[t];
    toggle.setAttribute("aria-pressed", String(t === "terminal"));
  }
}

/** Saves the look in this Maya's config, as Settings does, and tells Settings. */
export async function saveViewerTheme(t: ViewerTheme): Promise<void> {
  const config = await invoke<Record<string, unknown>>("get_config");
  await invoke("set_config", { config: { ...config, viewerTheme: t } });
  window.dispatchEvent(new CustomEvent("maya:config-changed"));
}
```

- in `renderModal`, the root: `const root = el("div", m.theme === "terminal" ? "modal modal--terminal" : "modal");`
- the header, replacing `head.append(titles, state, close);`:

```ts
  const look = el("button", "modal__theme", ">_");
  look.type = "button";
  look.dataset.action = "theme";
  look.title = THEME_TITLE[m.theme ?? "chat"];
  look.setAttribute("aria-label", "Switch the viewer's look");
  look.setAttribute("aria-pressed", String(m.theme === "terminal"));
  look.addEventListener("click", () => h.onToggleTheme?.());
  head.append(titles, state, look, close);
```

- the history loop becomes:

```ts
  for (const t of m.turns) {
    const kind = t.notice ? "notice" : t.kind;
    const turn = el("div", `turn turn--${kind}`);
    // The terminal look's markers; the chat look hides them.
    const mark = { user: ">", assistant: "⏺", tool: "⏺", peer: "", notice: "" }[kind];
    if (mark) turn.append(el("span", "turn__mark", mark));
    const who = { user: "You", assistant: "Claude", peer: "From another session", tool: "", notice: "" }[kind];
    if (who) turn.append(el("div", "turn__who", who));
    const text = el("div", "turn__text");
    if (kind === "tool") {
      // The call, and under it its result when the transcript carries one.
      const [call, ...result] = t.text.split("\n");
      text.textContent = call;
      turn.append(text);
      if (result.join("").trim()) {
        const res = el("div", "turn__result");
        res.append(el("span", "turn__result-mark", "⎿ "), document.createTextNode(result.join("\n")));
        turn.append(res);
      }
    } else {
      if (kind === "notice") text.textContent = t.text;
      else text.append(renderMarkdown(t.text));
      turn.append(text);
    }
    history.append(turn);
  }
```

- in the composer, create `const prompt = el("span", "modal__prompt", ">");` next to `form`, and start both appends with it: `form.append(prompt, ta, pick, attach, send);` and `form.append(prompt, ta, send);`.
- in `paint()`, before `const fresh = renderModal(…)`: `m.theme = theme;`, and add to the handlers object `onToggleTheme: () => void toggleTheme(),`.
- after `cycleMode`:

```ts
/** The header toggle: the open panel switches at once; a failed save puts it back. */
async function toggleTheme(): Promise<void> {
  const before = theme;
  const next: ViewerTheme = before === "terminal" ? "chat" : "terminal";
  setViewerTheme(next);
  try {
    await saveViewerTheme(next);
  } catch (e) {
    setViewerTheme(before);
    setStatus(false, String(e));
  }
}
```

`src/styles.css`, appended:

```css
/* The session viewer's two looks: chat (today's) and terminal. The markers,
   the ⎿ line and the > prompt are always there; only the terminal look shows them. */
.modal__theme { margin-left: 8px; padding: 2px 8px; border: 1px solid var(--border); border-radius: 6px; background: transparent; color: var(--muted); font: 600 11px ui-monospace, SFMono-Regular, Menlo, monospace; cursor: pointer; }
.modal__theme[aria-pressed=true] { color: var(--gold); border-color: var(--gold-deep); }
.turn__mark, .turn__result-mark, .modal__prompt { display: none; }
.modal--terminal .modal__panel { background: #0a0a0a; color: #e8e2d4; }
.modal--terminal .modal__history { gap: 6px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.modal--terminal .turn { display: grid; grid-template-columns: 2ch minmax(0, 1fr); column-gap: 4px; max-width: none; align-self: stretch; background: transparent; border: 0; border-radius: 0; padding: 0; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12.5px; color: #e8e2d4; }
.modal--terminal .turn > :not(.turn__mark) { grid-column: 2; }
.modal--terminal .turn__mark { display: inline; grid-column: 1; }
.modal--terminal .turn--user .turn__mark { color: var(--gold); }
.modal--terminal .turn--user .turn__who, .modal--terminal .turn--assistant .turn__who { display: none; }
.modal--terminal .turn--tool, .modal--terminal .turn__result { color: #8a8478; }
.modal--terminal .turn__result-mark { display: inline; }
.modal--terminal .turn__text .md pre { background: #161512; }
.modal--terminal .modal__composer { align-items: flex-start; background: #0a0a0a; border-top-color: #2a2824; }
.modal--terminal .modal__prompt { display: inline; padding-top: 8px; color: var(--gold); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.modal--terminal .modal__input { background: transparent; border: 0; color: #e8e2d4; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
.modal--terminal .modal__composer [data-action=send] { font-size: 0; }
.modal--terminal .modal__composer [data-action=send]::after { content: "⏎"; font-size: 14px; }
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test -p maya-core config:: && pnpm test && pnpm exec tsc --noEmit`
Expected: `test result: ok.`; all vitest files pass (6 new tests); tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs src/types.ts src/modal.ts src/styles.css src/modal.test.ts src/modal-open.test.ts
git commit -m "feat: a terminal look for the session viewer" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Session viewer in Settings, on the desktop and the phone

**Files:**
- Modify: `src/settings.ts` (model, `ConfigJson`, handler, the select, `applyConfig`, handlers)
- Modify: `src/mobile/network.ts` (model, handler, section, config load/save, the config-changed listener)
- Modify: `mobile/src/commands.rs` (`apply_settings`, a test)
- Test: `src/settings.test.ts`, `src/mobile/network.test.ts`

**Interfaces:**
- Consumes: Task 9's `ViewerTheme` (TS and Rust), `setViewerTheme`.
- Produces: `SettingsModel.viewerTheme?: ViewerTheme`, `SettingsHandlers.onViewerTheme?(t: ViewerTheme)`; `NetworkModel.viewerTheme?: ViewerTheme`, `NetworkHandlers.onViewerTheme?(t: ViewerTheme)`; both render `select[name=viewerTheme]` with options Chat / Terminal; `apply_settings` copies `viewer_theme`.

- [ ] **Step 1: Write the failing tests**

Append to `src/settings.test.ts`:

```ts
describe("session viewer", () => {
  it("offers the viewer's look under Sessions and saves the choice", () => {
    const h = { ...handlers(), onViewerTheme: vi.fn() };
    const el = renderSettings({ ...voiceBase, viewerTheme: "chat" }, h);
    const sessions = el.querySelector(".settings__section--sessions")!;
    expect(sessions.textContent).toContain("Session viewer");
    const sel = sessions.querySelector<HTMLSelectElement>("select[name=viewerTheme]")!;
    expect([...sel.options].map((o) => o.textContent)).toEqual(["Chat", "Terminal"]);
    expect(sel.value).toBe("chat");
    sel.value = "terminal";
    sel.dispatchEvent(new Event("change"));
    expect(h.onViewerTheme).toHaveBeenCalledWith("terminal");
  });
});
```

Append inside `describe("renderNetwork", …)` in `src/mobile/network.test.ts`:

```ts
  it("offers the viewer's look and saves the choice", () => {
    const h = { ...handlers(), onViewerTheme: vi.fn() };
    const el = renderNetwork({ ...base, viewerTheme: "terminal" }, h, NOW);
    expect(el.textContent).toContain("Session viewer");
    const sel = el.querySelector<HTMLSelectElement>("select[name=viewerTheme]")!;
    expect(sel.value).toBe("terminal");
    sel.value = "chat";
    sel.dispatchEvent(new Event("change"));
    expect(h.onViewerTheme).toHaveBeenCalledWith("chat");
  });
```

Append to `mobile/src/commands.rs`'s `mod tests`:

```rust
    #[test]
    fn settings_carry_the_viewer_look() {
        let mut c = Config::default();
        apply_settings(&mut c, &Config { viewer_theme: maya_core::config::ViewerTheme::Terminal, ..Config::default() });
        assert_eq!(c.viewer_theme, maya_core::config::ViewerTheme::Terminal);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test src/settings.test.ts src/mobile/network.test.ts && cargo test -p maya-mobile commands::`
Expected: vitest FAILS ("Cannot read properties of null (reading 'value')"); the Rust test FAILS (`left: Chat, right: Terminal`).

- [ ] **Step 3: Write the implementation**

`src/settings.ts`:

- imports: add `ViewerTheme` to the `./types` type import; `import { setViewerTheme } from "./modal";`
- `SettingsModel`: `/** The session viewer's look; chat when absent. */ viewerTheme?: ViewerTheme;`
- `ConfigJson`: `viewerTheme?: ViewerTheme;`
- `SettingsHandlers`: `onViewerTheme?(t: ViewerTheme): void;`
- in `renderSettings`, right after `sessions.append(dirLabel);`:

```ts
  const viewerLabel = document.createElement("label");
  viewerLabel.textContent = "Session viewer";
  const viewerSel = document.createElement("select");
  viewerSel.name = "viewerTheme";
  for (const [value, text] of [["chat", "Chat"], ["terminal", "Terminal"]] as const) {
    const o = document.createElement("option");
    o.value = value;
    o.textContent = text;
    viewerSel.append(o);
  }
  viewerSel.value = model.viewerTheme ?? "chat";
  viewerSel.addEventListener("change", () => h.onViewerTheme?.(viewerSel.value as ViewerTheme));
  viewerLabel.append(viewerSel);
  sessions.append(viewerLabel);
```

- `applyConfig`, at its end:

```ts
    model.viewerTheme = c.viewerTheme ?? "chat";
    setViewerTheme(model.viewerTheme);
```

- `handlers`: `onViewerTheme: (t) => void run(() => saveConfig({ viewerTheme: t })),`

`src/mobile/network.ts`:

- imports: `import { setViewerTheme } from "../modal";` and `import type { ViewerTheme } from "../types";`
- `NetworkModel`: `/** The session viewer's look; chat when absent. */ viewerTheme?: ViewerTheme;`
- `NetworkHandlers`: `onViewerTheme?(t: ViewerTheme): void;`
- in `renderNetwork`, after the Voice section's `root.append(voice);`:

```ts
  const viewer = section("Session viewer");
  const viewerSel = el("select", "");
  viewerSel.name = "viewerTheme";
  for (const [value, text] of [["chat", "Chat"], ["terminal", "Terminal"]] as const) {
    const o = el("option", "", text);
    o.value = value;
    viewerSel.append(o);
  }
  viewerSel.value = m.viewerTheme ?? "chat";
  viewerSel.addEventListener("change", () => h.onViewerTheme?.(viewerSel.value as ViewerTheme));
  viewer.append(viewerSel);
  root.append(viewer);
```

- `ConfigView`: `viewerTheme?: ViewerTheme;`
- in both `loadConfig` and `saveConfig`, after `model.wakeWord = …;`:

```ts
    model.viewerTheme = config.viewerTheme ?? "chat";
    setViewerTheme(model.viewerTheme);
```

- `handlers`: `onViewerTheme: (t) => void busy(() => saveConfig((c) => { c.viewerTheme = t; })),`
- after `void listen<NetworkStatus>("network", …)`: the viewer's own toggle saved the look:

```ts
  window.addEventListener("maya:config-changed", () => void loadConfig().then(paint).catch(() => undefined));
```

`mobile/src/commands.rs` `apply_settings`, last line: `c.viewer_theme = from.viewer_theme;`

- [ ] **Step 4: Run the tests to see them pass**

Run: `pnpm test && pnpm exec tsc --noEmit && cargo test -p maya-mobile`
Expected: all vitest files pass; tsc prints nothing; `test result: ok.`

- [ ] **Step 5: Commit**

```bash
git add src/settings.ts src/settings.test.ts src/mobile/network.ts src/mobile/network.test.ts mobile/src/commands.rs
git commit -m "feat: choose the session viewer's look in Settings, on the desktop and the phone" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 11: README

**Files:**
- Modify: `README.md` ("What it does", "First run")

**Interfaces:**
- Consumes: nothing; documents Tasks 5–10.
- Produces: the first-run Git step and two feature lines.

- [ ] **Step 1: Check what is missing**

Run: `grep -n "Settings → Git\|was running\|terminal look" README.md`
Expected: one match only, line 39 under Install ("Pull requests: turn on GitHub or GitLab in Settings → Git …"); nothing in "First run" or "What it does".

- [ ] **Step 2: Edit "What it does"**

After the line `- Starts new sessions and resumes old ones`, add:

```markdown
- After a restart, offers to resume every session that was running, on this machine and on the paired ones, each in its own folder and terminal: from the Sessions tab, or by voice ("Maya, resume my sessions")
- Shows a session as a chat or in a terminal look, like the agent's own terminal (Settings → Sessions, or the `>_` button on the session)
```

- [ ] **Step 3: Edit "First run"**

Replace step 3 with these two steps:

```markdown
3. Open **Settings → Git** and turn on GitHub and/or GitLab (with its URL,
   for a self-hosted GitLab). Give each a token, or the name of a variable
   that holds one (`GH_TOKEN`, `GITLAB_TOKEN`), and pick SSH (the default)
   or HTTPS for review clones.
4. Optionally turn on **Listen for "Maya"** and pick a speech recogniser
   (see Voice).
```

- [ ] **Step 4: Check the result**

Run: `grep -n "Settings → Git\|was running\|terminal look" README.md`
Expected: four matches: the Install line, the two new lines in "What it does", and step 3 of "First run".

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: the Git first-run step, resuming after a restart and the terminal look" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Release 0.25.0

**Files:**
- Modify: `package.json`, `src-tauri/tauri.conf.json`, `mobile/tauri.conf.json`, `Cargo.toml`, `Cargo.lock` (through the script)

**Interfaces:**
- Consumes: every task above.
- Produces: version 0.25.0 everywhere, in its own commit.

- [ ] **Step 1: Check main's version**

Run: `git fetch origin main && git show origin/main:Cargo.toml | grep '^version'`
Expected: `version = "0.24.0"`. If `main` has moved past 0.24.0, bump its minor instead (two `feat`s) and use that number below.

- [ ] **Step 2: Set the version**

Run: `sh scripts/set-version.sh 0.25.0`
Expected: four `"version": "0.25.0"` / `version = "0.25.0"` lines.

- [ ] **Step 3: Check the lock file moved too**

Run: `git diff --stat && grep -A1 '^name = "maya-core"' Cargo.lock`
Expected: `Cargo.lock` is in the stat and shows `version = "0.25.0"`. If it is not (cargo off `sh`'s PATH), run `cargo update --workspace --offline` and check again.

- [ ] **Step 4: Run every check**

Run: `cargo test --workspace && pnpm test && pnpm exec tsc --noEmit`
Expected: all pass, tsc prints nothing.

- [ ] **Step 5: Commit**

```bash
git add package.json src-tauri/tauri.conf.json mobile/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.25.0" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Owed by hand after merge (the spec's "By hand"): a real restart of the Mac with sessions open, with and without the laptop; the terminal look on a long session with tool calls, a question and attachments; the phone's banner.
