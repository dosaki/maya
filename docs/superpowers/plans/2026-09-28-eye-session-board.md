# Eye Session Board Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Tauri desktop app that shows every local Claude Code session as a card on a four-column kanban board (Awaiting Decision, Working, Completed, Idle), with click-to-focus of the session's Terminal tab.

**Architecture:** A Rust core reads three local sources under `~/.claude/` (the session registry, a hook event log the app installs, and transcript tails), runs a pure state-derivation function per session, and pushes `Card[]` to the web view over a Tauri event. The frontend is framework-free TypeScript that renders the board from `Card[]`. Focus and hook install are Tauri commands.

**Tech Stack:** Tauri 2, Rust (serde, serde_json, notify, libc, dirs, tempfile for tests), Vite + TypeScript vanilla frontend, Vitest + jsdom for frontend tests, AppleScript via `osascript` for Terminal focus.

**Spec:** `docs/superpowers/specs/2026-09-28-eye-session-board-design.md`

## Global Constraints

- macOS only; Terminal.app is the only supported terminal.
- All data read from `~/.claude/`; the app writes only to `~/.claude/eye/` and, on explicit user action, `~/.claude/settings.json` (with a backup first).
- Registry: `~/.claude/sessions/<pid>.json`; statuses `busy`, `idle`, `shell`. `busy` and `shell` both mean Working.
- Hook event log: `~/.claude/eye/events.jsonl`, one JSON object per line, hook stdin payload plus `received_at` (epoch millis).
- Transcript path when not given by a hook event: `~/.claude/projects/<cwd with every "/" replaced by "-">/<sessionId>.jsonl`.
- State priority: Awaiting Decision, then Working, then Completed, then Idle.
- Completed decays to Idle after `completed_timeout_minutes` (default 30).
- Column order in the UI: Awaiting Decision, Working, Completed, Idle.
- Snippet is the last assistant text trimmed to 200 characters.
- Hook entries are identified by the marker string `.claude/eye/hook.sh` in their command; uninstall removes only those.
- Hook must never block or alter Claude Code: exits 0, prints nothing.
- Every task ends with a commit. Commit messages are conventional (`feat:`, `test:`, `chore:`), and end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Run Rust commands from `src-tauri/` with `source "$HOME/.cargo/env"` first if `cargo` is not on PATH.

## Review Focus

1. Registry file whose pid is dead (crashed session): must not appear as a card. Test in Task 2.
2. `events.jsonl` with a partially written last line (hook mid-append): the good lines are used, the partial line is left for the next read, nothing crashes. Test in Task 3.
3. Transcript tail that starts mid-line (we read the last 256 KB): the first partial line is skipped, later lines parse. Test in Task 4.
4. Two live sessions in the same cwd (the user has two in `dev/envoy`): both cards appear, keyed by session id, not cwd. Test in Task 9.
5. Focus on a session whose Terminal tab is gone: `focus_session` returns an error string and the UI shows a toast, no panic. Tests in Task 6 (parsing / not-found) and Task 10 (toast).

---

### Task 1: Toolchain and project scaffold

**Files:**
- Create: whole Tauri project in `/Users/tiagocorreia/dev/eye` (via `create-tauri-app`), then `src-tauri/src/model.rs`
- Modify: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `model::{State, AwaitKind, Awaiting, Card}` used by every later Rust task, and the same shape as TypeScript `Card` in Task 9.

- [ ] **Step 1: Install Rust**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo --version
```
Expected: prints `cargo 1.x.y`.

- [ ] **Step 2: Scaffold the Tauri app into the existing directory**

`create-tauri-app` refuses a non-empty directory, so scaffold in a sibling folder and move the files in.

```bash
cd /Users/tiagocorreia/dev
pnpm create tauri-app@latest eye-scaffold --template vanilla-ts --manager pnpm --yes
rsync -a --exclude .git eye-scaffold/ eye/
rm -rf eye-scaffold
cd eye && pnpm install
```
Expected: `eye/` now has `index.html`, `package.json`, `src/`, `src-tauri/`, `vite.config.ts`, `.gitignore`. The existing `docs/` and `.remember/` are untouched.

- [ ] **Step 3: Set the app identity and window**

Edit `src-tauri/tauri.conf.json`: set `"productName": "Eye"`, `"identifier": "com.tiagocorreia.eye"`, and in `app.windows[0]` set `"title": "Eye"`, `"width": 1280`, `"height": 820`.

- [ ] **Step 4: Add Rust dependencies**

In `src-tauri/Cargo.toml` under `[dependencies]` add (keep the existing `tauri`, `serde`, `serde_json` lines; make sure `serde` has `features = ["derive"]`):

```toml
notify = "8"
libc = "0.2"
dirs = "6"
```

And add:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 5: Create the shared model**

Create `src-tauri/src/model.rs`:

```rust
use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Awaiting,
    Working,
    Completed,
    Idle,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AwaitKind {
    Question,
    Plan,
    Permission,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Awaiting {
    pub kind: AwaitKind,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub session_id: String,
    pub pid: i32,
    pub name: String,
    pub cwd: String,
    pub state: State,
    /// Epoch millis when the session entered `state`.
    pub state_since: u64,
    /// Last assistant text, trimmed to 200 chars.
    pub snippet: String,
    pub awaiting: Option<Awaiting>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_serialises_camel_case_and_lowercase_enums() {
        let card = Card {
            session_id: "s1".into(),
            pid: 42,
            name: "eye-3b".into(),
            cwd: "/Users/x/dev/eye".into(),
            state: State::Awaiting,
            state_since: 1000,
            snippet: "hi".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Permission, detail: "Bash: rm -rf".into() }),
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["sessionId"], "s1");
        assert_eq!(json["stateSince"], 1000);
        assert_eq!(json["state"], "awaiting");
        assert_eq!(json["awaiting"]["kind"], "permission");
    }
}
```

- [ ] **Step 6: Register the module**

In `src-tauri/src/lib.rs`, add `pub mod model;` at the top (above `run`). Remove the template's `greet` command and its `generate_handler![greet]` entry; use `tauri::generate_handler![]` for now.

- [ ] **Step 7: Run the tests and build**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test
```
Expected: `test model::tests::card_serialises_camel_case_and_lowercase_enums ... ok`.

```bash
cd /Users/tiagocorreia/dev/eye && pnpm tauri build --debug --no-bundle 2>&1 | tail -5
```
Expected: finishes with no errors (first build takes several minutes).

- [ ] **Step 8: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add -A
git commit -m "chore: scaffold Tauri app and shared card model

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Session registry reader

**Files:**
- Create: `src-tauri/src/registry.rs`, `src-tauri/fixtures/registry/49643.json`, `src-tauri/fixtures/registry/bad.json`
- Modify: `src-tauri/src/lib.rs` (add `pub mod registry;`)

**Interfaces:**
- Produces:
  - `registry::RegistrySession { pid: i32, session_id: String, cwd: String, name: String, status: String, started_at: u64, status_updated_at: u64 }`
  - `registry::parse(json: &str) -> Result<RegistrySession, serde_json::Error>`
  - `registry::list(dir: &Path, alive: &dyn Fn(i32) -> bool) -> Vec<RegistrySession>` (sorted by name)
  - `registry::pid_alive(pid: i32) -> bool`
  - `registry::transcript_path(claude_dir: &Path, s: &RegistrySession) -> PathBuf`

- [ ] **Step 1: Add fixtures**

`src-tauri/fixtures/registry/49643.json`:

```json
{"pid":49643,"sessionId":"af62296a-4054-44ff-8c47-65ac4f472599","cwd":"/Users/tiagocorreia/dev/eye","startedAt":1790587340040,"procStart":"Mon Sep 28 09:22:19 2026","version":"2.1.283","peerProtocol":1,"peerFeatures":["notify_idle"],"kind":"interactive","entrypoint":"cli","pidDomain":"darwin","messagingSocketPath":"/tmp/cc-socks/49643.sock","name":"eye-3b","nameSource":"derived","nameSince":1790587340040,"status":"busy","updatedAt":1790587602132,"statusUpdatedAt":1790587602132}
```

`src-tauri/fixtures/registry/bad.json`:

```
{not json
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/registry.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/registry")
    }

    #[test]
    fn parses_real_registry_file() {
        let s = parse(&std::fs::read_to_string(fixtures().join("49643.json")).unwrap()).unwrap();
        assert_eq!(s.pid, 49643);
        assert_eq!(s.session_id, "af62296a-4054-44ff-8c47-65ac4f472599");
        assert_eq!(s.cwd, "/Users/tiagocorreia/dev/eye");
        assert_eq!(s.name, "eye-3b");
        assert_eq!(s.status, "busy");
        assert_eq!(s.status_updated_at, 1790587602132);
        assert_eq!(s.started_at, 1790587340040);
    }

    #[test]
    fn derives_transcript_path_from_cwd() {
        let s = parse(&std::fs::read_to_string(fixtures().join("49643.json")).unwrap()).unwrap();
        let p = transcript_path(Path::new("/Users/tiagocorreia/.claude"), &s);
        assert_eq!(
            p,
            Path::new("/Users/tiagocorreia/.claude/projects/-Users-tiagocorreia-dev-eye/af62296a-4054-44ff-8c47-65ac4f472599.jsonl")
        );
    }

    #[test]
    fn list_skips_bad_files_and_dead_pids() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(fixtures().join("49643.json"), dir.path().join("49643.json")).unwrap();
        std::fs::copy(fixtures().join("bad.json"), dir.path().join("bad.json")).unwrap();
        let mut dead = std::fs::read_to_string(fixtures().join("49643.json")).unwrap();
        dead = dead.replace("\"pid\":49643", "\"pid\":1").replace("eye-3b", "dead-1");
        std::fs::write(dir.path().join("1.json"), dead).unwrap();

        let all = list(dir.path(), &|_| true);
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["dead-1", "eye-3b"]);

        let live = list(dir.path(), &|pid| pid == 49643);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].name, "eye-3b");
    }

    #[test]
    fn own_pid_is_alive_and_huge_pid_is_not() {
        assert!(pid_alive(std::process::id() as i32));
        assert!(!pid_alive(2_000_000_000));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Add `pub mod registry;` to `src-tauri/src/lib.rs`, then:

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test registry
```
Expected: compile error, `cannot find function parse`.

- [ ] **Step 4: Implement**

Put this above the test module in `src-tauri/src/registry.rs`:

```rust
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RegistrySession {
    pub pid: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "startedAt", default)]
    pub started_at: u64,
    #[serde(rename = "statusUpdatedAt", default)]
    pub status_updated_at: u64,
}

pub fn parse(json: &str) -> Result<RegistrySession, serde_json::Error> {
    serde_json::from_str(json)
}

/// `~/.claude/projects/<cwd with "/" -> "-">/<sessionId>.jsonl`
pub fn transcript_path(claude_dir: &Path, s: &RegistrySession) -> PathBuf {
    let project = s.cwd.replace('/', "-");
    claude_dir.join("projects").join(project).join(format!("{}.jsonl", s.session_id))
}

/// True if a process with this pid exists (EPERM also means it exists).
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Every parseable `*.json` in `dir` whose pid passes `alive`, sorted by name.
pub fn list(dir: &Path, alive: &dyn Fn(i32) -> bool) -> Vec<RegistrySession> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(session) = parse(&text) else { continue };
        if alive(session.pid) {
            out.push(session);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test registry
```
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src/registry.rs src-tauri/src/lib.rs src-tauri/fixtures
git commit -m "feat: read Claude Code session registry

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Hook event log reader

**Files:**
- Create: `src-tauri/src/events.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod events;`)

**Interfaces:**
- Produces:
  - `events::HookEvent { session_id: String, hook_event_name: String, tool_name: Option<String>, tool_input: Option<serde_json::Value>, notification_type: Option<String>, transcript_path: Option<String>, received_at: u64 }`
  - `events::EventLog::new(path: PathBuf) -> EventLog`
  - `EventLog::read_new(&mut self) -> std::io::Result<usize>` (number of new events parsed; missing file is Ok(0))
  - `EventLog::events_for(&self, session_id: &str) -> &[HookEvent]`
  - `EventLog::transcript_path_for(&self, session_id: &str) -> Option<&str>`
  - `EventLog::compact(&mut self, keep: &HashSet<String>) -> std::io::Result<()>` (rewrites the file keeping only kept sessions, resets the offset, re-reads)

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/events.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Write;

    fn line(session: &str, event: &str, extra: &str, ts: u64) -> String {
        format!(
            "{{\"session_id\":\"{session}\",\"hook_event_name\":\"{event}\",\"transcript_path\":\"/t/{session}.jsonl\",\"cwd\":\"/x\"{extra},\"received_at\":{ts}}}\n"
        )
    }

    #[test]
    fn missing_file_reads_zero_events() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EventLog::new(dir.path().join("events.jsonl"));
        assert_eq!(log.read_new().unwrap(), 0);
        assert!(log.events_for("nope").is_empty());
    }

    #[test]
    fn reads_incrementally_and_groups_by_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(line("a", "UserPromptSubmit", "", 1).as_bytes()).unwrap();
        f.write_all(line("b", "Stop", "", 2).as_bytes()).unwrap();
        f.flush().unwrap();

        let mut log = EventLog::new(path.clone());
        assert_eq!(log.read_new().unwrap(), 2);
        assert_eq!(log.events_for("a").len(), 1);
        assert_eq!(log.events_for("b")[0].hook_event_name, "Stop");

        f.write_all(line("a", "PreToolUse", ",\"tool_name\":\"AskUserQuestion\",\"tool_input\":{\"questions\":[{\"question\":\"Which?\"}]}", 3).as_bytes()).unwrap();
        f.flush().unwrap();
        assert_eq!(log.read_new().unwrap(), 1);
        let a = log.events_for("a");
        assert_eq!(a.len(), 2);
        assert_eq!(a[1].tool_name.as_deref(), Some("AskUserQuestion"));
        assert_eq!(a[1].tool_input.as_ref().unwrap()["questions"][0]["question"], "Which?");
        assert_eq!(a[1].received_at, 3);
        assert_eq!(log.transcript_path_for("a"), Some("/t/a.jsonl"));
    }

    #[test]
    fn partial_last_line_is_kept_for_next_read_and_bad_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(line("a", "Stop", "", 1).as_bytes()).unwrap();
        f.write_all(b"this is not json\n").unwrap();
        f.write_all(b"{\"session_id\":\"a\",\"hook_event_na").unwrap();
        f.flush().unwrap();

        let mut log = EventLog::new(path.clone());
        assert_eq!(log.read_new().unwrap(), 1);

        f.write_all(b"me\":\"UserPromptSubmit\",\"received_at\":2}\n").unwrap();
        f.flush().unwrap();
        assert_eq!(log.read_new().unwrap(), 1);
        assert_eq!(log.events_for("a")[1].hook_event_name, "UserPromptSubmit");
    }

    #[test]
    fn truncated_file_resets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "Stop", "", 1) + &line("a", "Stop", "", 2)).unwrap();
        let mut log = EventLog::new(path.clone());
        assert_eq!(log.read_new().unwrap(), 2);
        std::fs::write(&path, line("b", "Stop", "", 3)).unwrap();
        log.read_new().unwrap();
        assert!(log.events_for("a").is_empty());
        assert_eq!(log.events_for("b").len(), 1);
    }

    #[test]
    fn compact_keeps_only_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, line("a", "Stop", "", 1) + &line("b", "Stop", "", 2) + &line("a", "UserPromptSubmit", "", 3)).unwrap();
        let mut log = EventLog::new(path.clone());
        log.read_new().unwrap();
        let keep: HashSet<String> = ["a".to_string()].into_iter().collect();
        log.compact(&keep).unwrap();
        assert_eq!(log.events_for("a").len(), 2);
        assert!(log.events_for("b").is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(!text.contains("\"session_id\":\"b\""));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Add `pub mod events;` to `lib.rs`, then `cargo test events`. Expected: compile error, `EventLog` not found.

- [ ] **Step 3: Implement**

Above the tests in `events.rs`:

```rust
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HookEvent {
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub received_at: u64,
}

pub struct EventLog {
    path: PathBuf,
    offset: u64,
    by_session: HashMap<String, Vec<HookEvent>>,
    raw_by_session: HashMap<String, Vec<String>>,
}

impl EventLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path, offset: 0, by_session: HashMap::new(), raw_by_session: HashMap::new() }
    }

    pub fn events_for(&self, session_id: &str) -> &[HookEvent] {
        self.by_session.get(session_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn transcript_path_for(&self, session_id: &str) -> Option<&str> {
        self.events_for(session_id).iter().rev().find_map(|e| e.transcript_path.as_deref())
    }

    /// Reads lines appended since the last call. A trailing line without `\n`
    /// is left unread. If the file shrank, everything is re-read from zero.
    pub fn read_new(&mut self) -> std::io::Result<usize> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
            self.by_session.clear();
            self.raw_by_session.clear();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;

        let complete_len = match buf.rfind('\n') {
            Some(i) => i + 1,
            None => return Ok(0),
        };
        let mut count = 0;
        for raw in buf[..complete_len].lines() {
            if raw.trim().is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<HookEvent>(raw) {
                self.raw_by_session.entry(ev.session_id.clone()).or_default().push(raw.to_string());
                self.by_session.entry(ev.session_id.clone()).or_default().push(ev);
                count += 1;
            }
        }
        self.offset += complete_len as u64;
        Ok(count)
    }

    /// Rewrites the file keeping only events of `keep` sessions, then re-reads.
    pub fn compact(&mut self, keep: &HashSet<String>) -> std::io::Result<()> {
        self.read_new()?;
        let mut kept: Vec<(u64, &String)> = Vec::new();
        for (sid, raws) in &self.raw_by_session {
            if !keep.contains(sid) {
                continue;
            }
            for (i, raw) in raws.iter().enumerate() {
                let ts = self.by_session[sid][i].received_at;
                kept.push((ts, raw));
            }
        }
        kept.sort_by_key(|(ts, _)| *ts);
        let mut text = String::new();
        for (_, raw) in kept {
            text.push_str(raw);
            text.push('\n');
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, text)?;
        self.offset = 0;
        self.by_session.clear();
        self.raw_by_session.clear();
        self.read_new()?;
        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test events
```
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src/events.rs src-tauri/src/lib.rs
git commit -m "feat: incremental reader for hook event log

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Transcript tail parser

**Files:**
- Create: `src-tauri/src/transcript.rs`, `src-tauri/fixtures/transcript/question-open.jsonl`, `src-tauri/fixtures/transcript/question-answered.jsonl`
- Modify: `src-tauri/src/lib.rs` (add `pub mod transcript;`)

**Interfaces:**
- Consumes: `model::AwaitKind`
- Produces:
  - `transcript::OpenQuestion { kind: AwaitKind, detail: String }`
  - `transcript::TranscriptTail { last_assistant_text: Option<String>, open_question: Option<OpenQuestion> }` (implements `Default`)
  - `transcript::parse_tail(text: &str) -> TranscriptTail`
  - `transcript::read_tail(path: &Path, max_bytes: u64) -> TranscriptTail` (missing file gives `Default`)
  - `transcript::TAIL_BYTES: u64 = 262_144`

- [ ] **Step 1: Add fixtures**

`src-tauri/fixtures/transcript/question-open.jsonl` (three lines):

```
{"type":"user","message":{"role":"user","content":"build me a board"},"timestamp":"2026-09-28T09:26:00.000Z"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I have enough context. One question first."}]},"timestamp":"2026-09-28T09:27:00.000Z"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"AskUserQuestion","input":{"questions":[{"question":"What should Completed mean?","header":"Completed","options":[]}]}}]},"timestamp":"2026-09-28T09:27:01.000Z"}
```

`src-tauri/fixtures/transcript/question-answered.jsonl` (the same three lines plus two more):

```
{"type":"user","message":{"role":"user","content":"build me a board"},"timestamp":"2026-09-28T09:26:00.000Z"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I have enough context. One question first."}]},"timestamp":"2026-09-28T09:27:00.000Z"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_01","name":"AskUserQuestion","input":{"questions":[{"question":"What should Completed mean?","header":"Completed","options":[]}]}}]},"timestamp":"2026-09-28T09:27:01.000Z"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"Finished a task, unreviewed"}]},"timestamp":"2026-09-28T09:28:00.000Z"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Great, writing the spec now."}]},"timestamp":"2026-09-28T09:28:05.000Z"}
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/transcript.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AwaitKind;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/transcript").join(name)).unwrap()
    }

    #[test]
    fn detects_open_question_and_last_text() {
        let t = parse_tail(&fixture("question-open.jsonl"));
        assert_eq!(t.last_assistant_text.as_deref(), Some("I have enough context. One question first."));
        let q = t.open_question.unwrap();
        assert_eq!(q.kind, AwaitKind::Question);
        assert_eq!(q.detail, "What should Completed mean?");
    }

    #[test]
    fn answered_question_is_not_open() {
        let t = parse_tail(&fixture("question-answered.jsonl"));
        assert!(t.open_question.is_none());
        assert_eq!(t.last_assistant_text.as_deref(), Some("Great, writing the spec now."));
    }

    #[test]
    fn exit_plan_mode_is_a_plan_approval() {
        let text = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t9","name":"ExitPlanMode","input":{"plan":"do things"}}]}}"#;
        let t = parse_tail(text);
        let q = t.open_question.unwrap();
        assert_eq!(q.kind, AwaitKind::Plan);
        assert_eq!(q.detail, "Plan approval");
    }

    #[test]
    fn skips_partial_first_line_and_junk() {
        let full = fixture("question-open.jsonl");
        let cut = &full[40..]; // starts mid-way through line 1
        let t = parse_tail(&format!("{cut}\nnot json at all\n"));
        assert!(t.open_question.is_some());
        assert_eq!(t.last_assistant_text.as_deref(), Some("I have enough context. One question first."));
    }

    #[test]
    fn empty_or_missing_file_is_default() {
        assert_eq!(parse_tail(""), TranscriptTail::default());
        assert_eq!(read_tail(Path::new("/nonexistent/x.jsonl"), TAIL_BYTES), TranscriptTail::default());
    }

    #[test]
    fn read_tail_only_reads_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let filler = format!("{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n", "x".repeat(500));
        let mut text = filler.repeat(10);
        text.push_str(&fixture("question-open.jsonl"));
        std::fs::write(&p, &text).unwrap();
        let t = read_tail(&p, 2000);
        assert!(t.open_question.is_some());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Add `pub mod transcript;` to `lib.rs`, then `cargo test transcript`. Expected: compile error.

- [ ] **Step 4: Implement**

Above the tests in `transcript.rs`:

```rust
use crate::model::AwaitKind;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const TAIL_BYTES: u64 = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenQuestion {
    pub kind: AwaitKind,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptTail {
    pub last_assistant_text: Option<String>,
    pub open_question: Option<OpenQuestion>,
}

/// Reads at most `max_bytes` from the end of `path` and parses it.
pub fn read_tail(path: &Path, max_bytes: u64) -> TranscriptTail {
    let Ok(mut file) = std::fs::File::open(path) else { return TranscriptTail::default() };
    let Ok(meta) = file.metadata() else { return TranscriptTail::default() };
    let start = meta.len().saturating_sub(max_bytes);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return TranscriptTail::default();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return TranscriptTail::default();
    }
    let text = String::from_utf8_lossy(&bytes);
    if start > 0 {
        // Drop the first, possibly partial, line.
        match text.find('\n') {
            Some(i) => parse_tail(&text[i + 1..]),
            None => TranscriptTail::default(),
        }
    } else {
        parse_tail(&text)
    }
}

pub fn parse_tail(text: &str) -> TranscriptTail {
    let mut last_text: Option<String> = None;
    let mut open: Vec<(String, OpenQuestion)> = Vec::new();
    let mut answered: HashSet<String> = HashSet::new();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let kind = v["type"].as_str().unwrap_or("");
        let Some(content) = v["message"]["content"].as_array() else { continue };
        match kind {
            "assistant" => {
                for block in content {
                    match block["type"].as_str() {
                        Some("text") => {
                            if let Some(t) = block["text"].as_str() {
                                if !t.trim().is_empty() {
                                    last_text = Some(t.trim().to_string());
                                }
                            }
                        }
                        Some("tool_use") => {
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            match block["name"].as_str() {
                                Some("AskUserQuestion") => {
                                    let q = block["input"]["questions"][0]["question"].as_str().unwrap_or("Question").to_string();
                                    open.push((id, OpenQuestion { kind: AwaitKind::Question, detail: q }));
                                }
                                Some("ExitPlanMode") => {
                                    open.push((id, OpenQuestion { kind: AwaitKind::Plan, detail: "Plan approval".to_string() }));
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in content {
                    if block["type"].as_str() == Some("tool_result") {
                        if let Some(id) = block["tool_use_id"].as_str() {
                            answered.insert(id.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let open_question = open.into_iter().rev().find(|(id, _)| !answered.contains(id)).map(|(_, q)| q);
    TranscriptTail { last_assistant_text: last_text, open_question }
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test transcript
```
Expected: 6 tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src/transcript.rs src-tauri/src/lib.rs src-tauri/fixtures/transcript
git commit -m "feat: parse transcript tail for snippet and open questions

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: State derivation

**Files:**
- Create: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod state;`)

**Interfaces:**
- Consumes: `registry::RegistrySession`, `events::HookEvent`, `transcript::TranscriptTail`, `model::*`
- Produces:
  - `state::DeriveInput<'a> { registry: &'a RegistrySession, events: &'a [HookEvent], transcript: &'a TranscriptTail, now_ms: u64, completed_timeout_ms: u64 }`
  - `state::derive(input: &DeriveInput) -> Card`
  - `state::truncate(s: &str, max_chars: usize) -> String`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/state.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::HookEvent;
    use crate::model::{AwaitKind, State};
    use crate::registry::RegistrySession;
    use crate::transcript::{OpenQuestion, TranscriptTail};

    const MIN: u64 = 60_000;
    const TIMEOUT: u64 = 30 * MIN;

    fn reg(status: &str, status_updated_at: u64) -> RegistrySession {
        RegistrySession {
            pid: 7,
            session_id: "s".into(),
            cwd: "/Users/x/dev/eye".into(),
            name: "eye-1".into(),
            status: status.into(),
            started_at: 0,
            status_updated_at,
        }
    }

    fn ev(name: &str, ts: u64) -> HookEvent {
        HookEvent {
            session_id: "s".into(),
            hook_event_name: name.into(),
            tool_name: None,
            tool_input: None,
            notification_type: None,
            transcript_path: None,
            received_at: ts,
        }
    }

    fn tool_ev(name: &str, tool: &str, input: serde_json::Value, ts: u64) -> HookEvent {
        HookEvent { tool_name: Some(tool.into()), tool_input: Some(input), ..ev(name, ts) }
    }

    fn run(r: &RegistrySession, events: &[HookEvent], t: &TranscriptTail, now: u64) -> Card {
        derive(&DeriveInput { registry: r, events, transcript: t, now_ms: now, completed_timeout_ms: TIMEOUT })
    }

    #[test]
    fn busy_registry_without_events_is_working() {
        let c = run(&reg("busy", 1000), &[], &TranscriptTail::default(), 5000);
        assert_eq!(c.state, State::Working);
        assert_eq!(c.state_since, 1000);
        assert_eq!(c.name, "eye-1");
        assert_eq!(c.pid, 7);
    }

    #[test]
    fn shell_registry_is_working() {
        assert_eq!(run(&reg("shell", 1), &[], &TranscriptTail::default(), 5).state, State::Working);
    }

    #[test]
    fn idle_registry_without_events_is_idle() {
        let c = run(&reg("idle", 1000), &[], &TranscriptTail::default(), 5000);
        assert_eq!(c.state, State::Idle);
        assert_eq!(c.state_since, 1000);
    }

    #[test]
    fn permission_request_is_awaiting_until_post_tool_use() {
        let asked = [tool_ev("PermissionRequest", "Bash", serde_json::json!({"command": "rm -rf build"}), 2000)];
        let c = run(&reg("busy", 1000), &asked, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.state_since, 2000);
        let aw = c.awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Permission);
        assert_eq!(aw.detail, "Bash: rm -rf build");

        let approved = [asked[0].clone(), tool_ev("PostToolUse", "Bash", serde_json::json!({}), 2500)];
        let c = run(&reg("busy", 1000), &approved, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Working);
        assert!(c.awaiting.is_none());
    }

    #[test]
    fn permission_denied_clears_awaiting() {
        let evs = [tool_ev("PermissionRequest", "Edit", serde_json::json!({"file_path": "/a/b.rs"}), 1), ev("PermissionDenied", 2)];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 3).state, State::Working);
    }

    #[test]
    fn edit_permission_detail_uses_file_path() {
        let evs = [tool_ev("PermissionRequest", "Edit", serde_json::json!({"file_path": "/a/b.rs"}), 1)];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 3).awaiting.unwrap().detail, "Edit: /a/b.rs");
    }

    #[test]
    fn ask_user_question_is_awaiting_with_question_text() {
        let evs = [tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Which stack?"}]}), 10)];
        let c = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20);
        assert_eq!(c.state, State::Awaiting);
        let aw = c.awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Question);
        assert_eq!(aw.detail, "Which stack?");
    }

    #[test]
    fn exit_plan_mode_is_awaiting_plan() {
        let evs = [tool_ev("PreToolUse", "ExitPlanMode", serde_json::json!({}), 10)];
        let aw = run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).awaiting.unwrap();
        assert_eq!(aw.kind, AwaitKind::Plan);
    }

    #[test]
    fn other_pre_tool_use_is_not_awaiting() {
        let evs = [tool_ev("PreToolUse", "Bash", serde_json::json!({}), 10)];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).state, State::Working);
    }

    #[test]
    fn notification_permission_prompt_counts_as_awaiting() {
        let mut e = ev("Notification", 10);
        e.notification_type = Some("permission_prompt".into());
        let c = run(&reg("busy", 0), &[e], &TranscriptTail::default(), 20);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.awaiting.unwrap().kind, AwaitKind::Permission);
    }

    #[test]
    fn new_prompt_clears_awaiting_question() {
        let evs = [
            tool_ev("PreToolUse", "AskUserQuestion", serde_json::json!({"questions": [{"question": "Q"}]}), 10),
            ev("UserPromptSubmit", 11),
        ];
        assert_eq!(run(&reg("busy", 0), &evs, &TranscriptTail::default(), 20).state, State::Working);
    }

    #[test]
    fn stop_makes_completed_then_decays_to_idle() {
        let evs = [ev("UserPromptSubmit", 1000), ev("Stop", 2000)];
        let c = run(&reg("idle", 2100), &evs, &TranscriptTail::default(), 3000);
        assert_eq!(c.state, State::Completed);
        assert_eq!(c.state_since, 2000);

        let c = run(&reg("idle", 2100), &evs, &TranscriptTail::default(), 2000 + TIMEOUT + 1);
        assert_eq!(c.state, State::Idle);
        assert_eq!(c.state_since, 2000 + TIMEOUT);
    }

    #[test]
    fn stop_then_new_prompt_is_working() {
        let evs = [ev("Stop", 2000), ev("UserPromptSubmit", 3000)];
        let c = run(&reg("busy", 3001), &evs, &TranscriptTail::default(), 4000);
        assert_eq!(c.state, State::Working);
    }

    #[test]
    fn stop_but_registry_still_busy_is_working() {
        let evs = [ev("Stop", 2000)];
        assert_eq!(run(&reg("busy", 1000), &evs, &TranscriptTail::default(), 2001).state, State::Working);
    }

    #[test]
    fn transcript_fallback_detects_question_only_without_events() {
        let t = TranscriptTail {
            last_assistant_text: Some("One question first.".into()),
            open_question: Some(OpenQuestion { kind: AwaitKind::Question, detail: "What?".into() }),
        };
        let c = run(&reg("busy", 500), &[], &t, 600);
        assert_eq!(c.state, State::Awaiting);
        assert_eq!(c.awaiting.unwrap().detail, "What?");
        assert_eq!(c.snippet, "One question first.");

        let c = run(&reg("busy", 500), &[tool_ev("PostToolUse", "Bash", serde_json::json!({}), 550)], &t, 600);
        assert_eq!(c.state, State::Working);
    }

    #[test]
    fn snippet_is_truncated_to_200_chars() {
        let t = TranscriptTail { last_assistant_text: Some("y".repeat(500)), open_question: None };
        let c = run(&reg("idle", 0), &[], &t, 1);
        assert_eq!(c.snippet.chars().count(), 200);
        assert!(c.snippet.ends_with('…'));
    }

    #[test]
    fn truncate_keeps_short_strings_and_counts_chars() {
        assert_eq!(truncate("héllo", 10), "héllo");
        assert_eq!(truncate("héllo wörld", 5), "héll…");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Add `pub mod state;` to `lib.rs`, then `cargo test state::`. Expected: compile error.

- [ ] **Step 3: Implement**

Above the tests in `state.rs`:

```rust
use crate::events::HookEvent;
use crate::model::{AwaitKind, Awaiting, Card, State};
use crate::registry::RegistrySession;
use crate::transcript::TranscriptTail;

pub const SNIPPET_CHARS: usize = 200;

pub struct DeriveInput<'a> {
    pub registry: &'a RegistrySession,
    pub events: &'a [HookEvent],
    pub transcript: &'a TranscriptTail,
    pub now_ms: u64,
    pub completed_timeout_ms: u64,
}

/// Trims to `max_chars` characters, replacing the last one with `…` when cut.
pub fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn permission_detail(e: &HookEvent) -> String {
    let tool = e.tool_name.clone().unwrap_or_else(|| "Tool".to_string());
    let input = e.tool_input.as_ref();
    let arg = input
        .and_then(|i| i.get("command").or_else(|| i.get("file_path")).or_else(|| i.get("path")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    match arg {
        Some(a) => format!("{tool}: {}", truncate(a.trim(), 120)),
        None => tool,
    }
}

fn question_detail(e: &HookEvent) -> String {
    e.tool_input
        .as_ref()
        .and_then(|i| i["questions"][0]["question"].as_str())
        .unwrap_or("Question")
        .to_string()
}

pub fn derive(i: &DeriveInput) -> Card {
    let r = i.registry;
    let mut awaiting: Option<(Awaiting, u64)> = None;
    // (is_stop, timestamp) of the most recent Stop or UserPromptSubmit.
    let mut last_turn: Option<(bool, u64)> = None;

    for e in i.events {
        match e.hook_event_name.as_str() {
            "PermissionRequest" => {
                awaiting = Some((Awaiting { kind: AwaitKind::Permission, detail: permission_detail(e) }, e.received_at));
            }
            "PreToolUse" => match e.tool_name.as_deref() {
                Some("AskUserQuestion") => {
                    awaiting = Some((Awaiting { kind: AwaitKind::Question, detail: question_detail(e) }, e.received_at));
                }
                Some("ExitPlanMode") => {
                    awaiting = Some((Awaiting { kind: AwaitKind::Plan, detail: "Plan approval".to_string() }, e.received_at));
                }
                _ => {}
            },
            "Notification" if e.notification_type.as_deref() == Some("permission_prompt") => {
                if awaiting.is_none() {
                    awaiting = Some((Awaiting { kind: AwaitKind::Permission, detail: "Permission prompt".to_string() }, e.received_at));
                }
            }
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => awaiting = None,
            "Stop" => {
                awaiting = None;
                last_turn = Some((true, e.received_at));
            }
            "UserPromptSubmit" => {
                awaiting = None;
                last_turn = Some((false, e.received_at));
            }
            _ => {}
        }
    }

    if i.events.is_empty() {
        if let Some(q) = &i.transcript.open_question {
            awaiting = Some((Awaiting { kind: q.kind, detail: q.detail.clone() }, r.status_updated_at));
        }
    }

    let is_busy = matches!(r.status.as_str(), "busy" | "shell");

    let (state, state_since, aw) = if let Some((a, ts)) = awaiting {
        (State::Awaiting, ts, Some(a))
    } else if is_busy {
        (State::Working, r.status_updated_at, None)
    } else if let Some((true, ts)) = last_turn {
        if i.now_ms.saturating_sub(ts) < i.completed_timeout_ms {
            (State::Completed, ts, None)
        } else {
            (State::Idle, ts + i.completed_timeout_ms, None)
        }
    } else {
        (State::Idle, r.status_updated_at, None)
    };

    Card {
        session_id: r.session_id.clone(),
        pid: r.pid,
        name: r.name.clone(),
        cwd: r.cwd.clone(),
        state,
        state_since,
        snippet: truncate(i.transcript.last_assistant_text.as_deref().unwrap_or(""), SNIPPET_CHARS),
        awaiting: aw,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test state::
```
Expected: 17 tests pass.

- [ ] **Step 5: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src/state.rs src-tauri/src/lib.rs
git commit -m "feat: derive kanban state from registry, hook events and transcript

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Terminal focus

**Files:**
- Create: `src-tauri/src/focus.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod focus;`)

**Interfaces:**
- Produces:
  - `focus::tty_from_ps(output: &str) -> Option<String>` (e.g. `"ttys021 \n"` gives `"/dev/ttys021"`; `"??"` gives `None`)
  - `focus::applescript_for(tty: &str) -> String`
  - `focus::focus_pid(pid: i32) -> Result<(), String>`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/focus.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tty_from_ps_output() {
        assert_eq!(tty_from_ps("ttys021 \n"), Some("/dev/ttys021".to_string()));
        assert_eq!(tty_from_ps("  ttys007\n"), Some("/dev/ttys007".to_string()));
        assert_eq!(tty_from_ps("??\n"), None);
        assert_eq!(tty_from_ps(""), None);
    }

    #[test]
    fn applescript_targets_the_tty_and_activates() {
        let s = applescript_for("/dev/ttys021");
        assert!(s.contains("tell application \"Terminal\""));
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(s.contains("set selected tab of w to t"));
        assert!(s.contains("activate"));
        assert!(s.contains("return \"not found\""));
    }

    #[test]
    fn focusing_a_dead_pid_is_an_error_not_a_panic() {
        let err = focus_pid(2_000_000_000).unwrap_err();
        assert!(err.to_lowercase().contains("tty") || err.to_lowercase().contains("process"), "{err}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Add `pub mod focus;` to `lib.rs`, then `cargo test focus`. Expected: compile error.

- [ ] **Step 3: Implement**

Above the tests in `focus.rs`:

```rust
use std::process::Command;

/// Turns `ps -o tty= -p <pid>` output into `/dev/ttysNNN`.
pub fn tty_from_ps(output: &str) -> Option<String> {
    let t = output.trim();
    if t.is_empty() || t == "??" || t == "-" {
        return None;
    }
    Some(format!("/dev/{t}"))
}

pub fn applescript_for(tty: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        set selected tab of w to t
        set index of w to 1
        activate
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#
    )
}

/// Brings the Terminal tab hosting `pid` to the front.
pub fn focus_pid(pid: i32) -> Result<(), String> {
    let ps = Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("could not run ps: {e}"))?;
    let stdout = String::from_utf8_lossy(&ps.stdout);
    let tty = tty_from_ps(&stdout).ok_or_else(|| format!("no tty for process {pid}"))?;

    let out = Command::new("osascript")
        .arg("-e")
        .arg(applescript_for(&tty))
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if result == "ok" {
        Ok(())
    } else {
        Err(format!("no Terminal tab found for {tty}"))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test focus
```
Expected: 3 tests pass.

- [ ] **Step 5: Manual check against a real session**

```bash
pid=$(ls ~/.claude/sessions/*.json | head -1 | xargs basename | sed 's/.json//')
tty=$(ps -o tty= -p $pid | tr -d ' ')
osascript -e 'tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "/dev/'"$tty"'" then
        set selected tab of w to t
        set index of w to 1
        activate
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found"'
```
Expected: prints `ok` and that Terminal tab comes to the front.

- [ ] **Step 6: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src/focus.rs src-tauri/src/lib.rs
git commit -m "feat: focus a session's Terminal tab by tty

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Hook install, remove and status

**Files:**
- Create: `src-tauri/src/hook_install.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod hook_install;`)

**Interfaces:**
- Produces:
  - `hook_install::HOOK_MARKER: &str = ".claude/eye/hook.sh"`
  - `hook_install::HOOK_SCRIPT: &str` (the shell script body)
  - `hook_install::HOOK_EVENTS: &[(&str, Option<&str>)]` (event name, matcher)
  - `hook_install::is_installed(settings: &serde_json::Value) -> bool`
  - `hook_install::install(settings: serde_json::Value, command: &str) -> serde_json::Value` (pure, idempotent)
  - `hook_install::remove(settings: serde_json::Value) -> serde_json::Value` (pure)
  - `hook_install::install_to(claude_dir: &Path) -> Result<(), String>`
  - `hook_install::remove_from(claude_dir: &Path) -> Result<(), String>`
  - `hook_install::status(claude_dir: &Path) -> Result<bool, String>`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/hook_install.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn install_adds_every_event_with_marker_command() {
        let out = install(json!({"model": "opus"}), "\"$HOME/.claude/eye/hook.sh\"");
        assert_eq!(out["model"], "opus");
        let hooks = out["hooks"].as_object().unwrap();
        for (event, matcher) in HOOK_EVENTS {
            let groups = hooks[*event].as_array().unwrap_or_else(|| panic!("missing {event}"));
            let g = groups.iter().find(|g| g["hooks"][0]["command"].as_str().unwrap().contains(HOOK_MARKER)).unwrap();
            assert_eq!(g["hooks"][0]["type"], "command");
            match matcher {
                Some(m) => assert_eq!(g["matcher"], *m),
                None => assert!(g.get("matcher").is_none()),
            }
        }
        assert!(is_installed(&out));
    }

    #[test]
    fn install_is_idempotent_and_preserves_other_hooks() {
        let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        let once = install(existing, "\"$HOME/.claude/eye/hook.sh\"");
        let twice = install(once.clone(), "\"$HOME/.claude/eye/hook.sh\"");
        assert_eq!(once, twice);
        let stop = twice["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
    }

    #[test]
    fn remove_strips_only_marker_entries() {
        let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        let installed = install(existing, "\"$HOME/.claude/eye/hook.sh\"");
        let removed = remove(installed);
        assert!(!is_installed(&removed));
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(removed["hooks"].get("PermissionRequest").is_none(), "empty arrays are dropped");
    }

    #[test]
    fn is_installed_false_without_hooks() {
        assert!(!is_installed(&json!({})));
        assert!(!is_installed(&json!({"hooks": {}})));
    }

    #[test]
    fn install_to_writes_script_backup_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{\"model\":\"opus\"}").unwrap();
        install_to(dir.path()).unwrap();

        let script = dir.path().join("eye/hook.sh");
        assert!(script.exists());
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&script).unwrap().permissions());
        assert!(mode & 0o100 != 0, "script must be executable");

        let backups: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("settings.json.eye-backup-")).collect();
        assert_eq!(backups.len(), 1);

        assert!(status(dir.path()).unwrap());
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap()).unwrap();
        assert_eq!(s["model"], "opus");

        remove_from(dir.path()).unwrap();
        assert!(!status(dir.path()).unwrap());
    }

    #[test]
    fn install_to_creates_settings_when_missing_and_refuses_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        install_to(dir.path()).unwrap();
        assert!(status(dir.path()).unwrap());

        let bad = tempfile::tempdir().unwrap();
        std::fs::write(bad.path().join("settings.json"), "{ not json").unwrap();
        let err = install_to(bad.path()).unwrap_err();
        assert!(err.contains("settings.json"));
        assert_eq!(std::fs::read_to_string(bad.path().join("settings.json")).unwrap(), "{ not json");
    }

    #[test]
    fn hook_script_appends_payload_with_received_at() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, HOOK_SCRIPT).unwrap();
        let out = std::process::Command::new("sh")
            .arg(&script)
            .env("HOME", dir.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(b"{\"session_id\":\"s1\",\"hook_event_name\":\"Stop\"}").unwrap();
                child.wait_with_output()
            })
            .unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty(), "hook must print nothing");
        let log = std::fs::read_to_string(dir.path().join(".claude/eye/events.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(v["session_id"], "s1");
        assert!(v["received_at"].as_u64().unwrap() > 1_700_000_000_000);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Add `pub mod hook_install;` to `lib.rs`, then `cargo test hook_install`. Expected: compile error.

- [ ] **Step 3: Implement**

Above the tests in `hook_install.rs`:

```rust
use serde_json::{json, Map, Value};
use std::path::Path;

pub const HOOK_MARKER: &str = ".claude/eye/hook.sh";

pub const HOOK_SCRIPT: &str = r#"#!/bin/sh
# Installed by Eye (Claude session board). Appends each hook payload to
# ~/.claude/eye/events.jsonl with a received_at timestamp. Never blocks,
# never prints, always exits 0.
dir="$HOME/.claude/eye"
mkdir -p "$dir" 2>/dev/null
t=$(date +%s000)
jq -c --arg t "$t" '. + {received_at: ($t|tonumber)}' >> "$dir/events.jsonl" 2>/dev/null
exit 0
"#;

/// (event name, matcher)
pub const HOOK_EVENTS: &[(&str, Option<&str>)] = &[
    ("PermissionRequest", None),
    ("PreToolUse", Some("AskUserQuestion|ExitPlanMode")),
    ("PostToolUse", None),
    ("PostToolUseFailure", None),
    ("PermissionDenied", None),
    ("Stop", None),
    ("UserPromptSubmit", None),
    ("SessionEnd", None),
    ("Notification", Some("permission_prompt")),
];

fn group_is_ours(group: &Value) -> bool {
    group["hooks"]
        .as_array()
        .map(|hs| hs.iter().any(|h| h["command"].as_str().map_or(false, |c| c.contains(HOOK_MARKER))))
        .unwrap_or(false)
}

pub fn is_installed(settings: &Value) -> bool {
    settings["hooks"]
        .as_object()
        .map(|hooks| hooks.values().any(|groups| groups.as_array().map_or(false, |g| g.iter().any(group_is_ours))))
        .unwrap_or(false)
}

/// Removes our entries, then adds one group per event. Idempotent.
pub fn install(settings: Value, command: &str) -> Value {
    let mut settings = remove(settings);
    let mut root = settings.as_object().cloned().unwrap_or_default();
    let mut hooks = root.get("hooks").and_then(|h| h.as_object()).cloned().unwrap_or_default();
    for (event, matcher) in HOOK_EVENTS {
        let mut group = Map::new();
        if let Some(m) = matcher {
            group.insert("matcher".into(), json!(m));
        }
        group.insert("hooks".into(), json!([{ "type": "command", "command": command }]));
        let groups = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
        if let Some(arr) = groups.as_array_mut() {
            arr.push(Value::Object(group));
        }
    }
    root.insert("hooks".into(), Value::Object(hooks));
    settings = Value::Object(root);
    settings
}

/// Strips every group whose command contains the marker; drops empty events.
pub fn remove(settings: Value) -> Value {
    let mut root = settings.as_object().cloned().unwrap_or_default();
    if let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()).cloned() {
        let mut cleaned = Map::new();
        for (event, groups) in hooks {
            let kept: Vec<Value> = groups.as_array().cloned().unwrap_or_default().into_iter().filter(|g| !group_is_ours(g)).collect();
            if !kept.is_empty() {
                cleaned.insert(event, Value::Array(kept));
            }
        }
        root.insert("hooks".into(), Value::Object(cleaned));
    }
    Value::Object(root)
}

fn read_settings(claude_dir: &Path) -> Result<Value, String> {
    let path = claude_dir.join("settings.json");
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read settings.json: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("settings.json is not valid JSON, refusing to modify it: {e}"))
}

fn write_settings_with_backup(claude_dir: &Path, settings: &Value) -> Result<(), String> {
    let path = claude_dir.join("settings.json");
    if path.exists() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let backup = claude_dir.join(format!("settings.json.eye-backup-{stamp}"));
        std::fs::copy(&path, &backup).map_err(|e| format!("cannot back up settings.json: {e}"))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("cannot write settings.json: {e}"))
}

pub fn install_to(claude_dir: &Path) -> Result<(), String> {
    let settings = read_settings(claude_dir)?;
    let eye_dir = claude_dir.join("eye");
    std::fs::create_dir_all(&eye_dir).map_err(|e| format!("cannot create {}: {e}", eye_dir.display()))?;
    let script = eye_dir.join("hook.sh");
    std::fs::write(&script, HOOK_SCRIPT).map_err(|e| format!("cannot write hook.sh: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let command = "\"$HOME/.claude/eye/hook.sh\"";
    write_settings_with_backup(claude_dir, &install(settings, command))
}

pub fn remove_from(claude_dir: &Path) -> Result<(), String> {
    let settings = read_settings(claude_dir)?;
    write_settings_with_backup(claude_dir, &remove(settings))
}

pub fn status(claude_dir: &Path) -> Result<bool, String> {
    Ok(is_installed(&read_settings(claude_dir)?))
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test hook_install
```
Expected: 7 tests pass. Note the `install_to` tests use a temp dir as `claude_dir`, so they never touch the real `~/.claude/settings.json`. The script test sets `HOME` to a temp dir.

- [ ] **Step 5: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src/hook_install.rs src-tauri/src/lib.rs
git commit -m "feat: install and remove the Claude Code status hook

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Config, store, watcher and Tauri commands

**Files:**
- Create: `src-tauri/src/config.rs`, `src-tauri/src/store.rs`, `src-tauri/src/watcher.rs`
- Modify: `src-tauri/src/lib.rs` (full rewrite of `run`)

**Interfaces:**
- Consumes: everything from Tasks 2 to 7.
- Produces (Tauri commands, called from Task 9 to 11 frontend):
  - `list_sessions() -> Vec<Card>`
  - `focus_session(pid: i32) -> Result<(), String>`
  - `hook_status() -> Result<bool, String>`
  - `install_hook() -> Result<bool, String>` (returns new status)
  - `remove_hook() -> Result<bool, String>`
  - `get_config() -> Config`
  - `set_config(config: Config) -> Result<Config, String>`
  - Event `"sessions"` with payload `Card[]`, emitted on every refresh.
  - `config::Config { completed_timeout_minutes: u64 }` serialised as `{"completedTimeoutMinutes": 30}`.

- [ ] **Step 1: Write failing config tests**

Create `src-tauri/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub completed_timeout_minutes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self { completed_timeout_minutes: 30 }
    }
}

impl Config {
    pub fn completed_timeout_ms(&self) -> u64 {
        self.completed_timeout_minutes * 60_000
    }
}

/// Missing or unreadable file gives `Config::default()`.
pub fn load(path: &Path) -> Config {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_thirty_minutes_when_missing() {
        let c = load(Path::new("/nonexistent/config.json"));
        assert_eq!(c.completed_timeout_minutes, 30);
        assert_eq!(c.completed_timeout_ms(), 1_800_000);
    }

    #[test]
    fn round_trips_and_uses_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested/config.json");
        save(&p, &Config { completed_timeout_minutes: 5 }).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("completedTimeoutMinutes"));
        assert_eq!(load(&p).completed_timeout_minutes, 5);
    }
}
```

Add `pub mod config;` to `lib.rs` and run `cargo test config`. Expected: 2 tests pass (this file is small enough to write tests and code together).

- [ ] **Step 2: Write the store with a test**

Create `src-tauri/src/store.rs`:

```rust
use crate::config::{self, Config};
use crate::events::EventLog;
use crate::model::Card;
use crate::registry::{self, RegistrySession};
use crate::state::{derive, DeriveInput};
use crate::transcript::{self, TAIL_BYTES};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct Store {
    claude_dir: PathBuf,
    events: EventLog,
    pub config: Config,
    alive: Box<dyn Fn(i32) -> bool + Send>,
}

impl Store {
    pub fn new(claude_dir: PathBuf) -> Self {
        let config = config::load(&claude_dir.join("eye/config.json"));
        Self {
            events: EventLog::new(claude_dir.join("eye/events.jsonl")),
            claude_dir,
            config,
            alive: Box::new(registry::pid_alive),
        }
    }

    #[cfg(test)]
    pub fn with_alive(mut self, alive: impl Fn(i32) -> bool + Send + 'static) -> Self {
        self.alive = Box::new(alive);
        self
    }

    pub fn config_path(&self) -> PathBuf {
        self.claude_dir.join("eye/config.json")
    }

    pub fn claude_dir(&self) -> &Path {
        &self.claude_dir
    }

    fn registry(&self) -> Vec<RegistrySession> {
        registry::list(&self.claude_dir.join("sessions"), &*self.alive)
    }

    /// Drops event-log lines for sessions no longer in the registry.
    pub fn compact_events(&mut self) {
        let keep: HashSet<String> = self.registry().into_iter().map(|s| s.session_id).collect();
        let _ = self.events.compact(&keep);
    }

    pub fn refresh(&mut self, now_ms: u64) -> Vec<Card> {
        let _ = self.events.read_new();
        let sessions = self.registry();
        let mut cards = Vec::with_capacity(sessions.len());
        for s in &sessions {
            let path = match self.events.transcript_path_for(&s.session_id) {
                Some(p) => PathBuf::from(p),
                None => registry::transcript_path(&self.claude_dir, s),
            };
            let tail = transcript::read_tail(&path, TAIL_BYTES);
            cards.push(derive(&DeriveInput {
                registry: s,
                events: self.events.events_for(&s.session_id),
                transcript: &tail,
                now_ms,
                completed_timeout_ms: self.config.completed_timeout_ms(),
            }));
        }
        cards
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    #[test]
    fn refresh_builds_cards_from_a_fake_claude_dir() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().to_path_buf();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::create_dir_all(claude.join("eye")).unwrap();
        std::fs::create_dir_all(claude.join("projects/-Users-x-dev-eye")).unwrap();

        std::fs::write(
            claude.join("sessions/7.json"),
            r#"{"pid":7,"sessionId":"s7","cwd":"/Users/x/dev/eye","name":"eye-7","status":"idle","startedAt":1,"statusUpdatedAt":100}"#,
        ).unwrap();
        std::fs::write(
            claude.join("projects/-Users-x-dev-eye/s7.jsonl"),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"All done."}]}}"#,
        ).unwrap();
        std::fs::write(
            claude.join("eye/events.jsonl"),
            "{\"session_id\":\"s7\",\"hook_event_name\":\"Stop\",\"received_at\":200}\n{\"session_id\":\"gone\",\"hook_event_name\":\"Stop\",\"received_at\":201}\n",
        ).unwrap();

        let mut store = Store::new(claude.clone()).with_alive(|_| true);
        store.compact_events();
        let cards = store.refresh(300);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "eye-7");
        assert_eq!(cards[0].state, State::Completed);
        assert_eq!(cards[0].snippet, "All done.");

        let log = std::fs::read_to_string(claude.join("eye/events.jsonl")).unwrap();
        assert!(!log.contains("gone"));
    }
}
```

Add `pub mod store;` to `lib.rs`; run `cargo test store`. Expected: 1 test passes.

- [ ] **Step 3: Write the watcher**

Create `src-tauri/src/watcher.rs`:

```rust
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

/// Calls `on_change` whenever `sessions_dir` or `eye_dir` changes (debounced
/// 200 ms) and at least every `tick`. Blocks forever; run on its own thread.
pub fn run(sessions_dir: &Path, eye_dir: &Path, tick: Duration, mut on_change: impl FnMut()) {
    let (tx, rx) = channel::<()>();
    let mut watcher: Option<RecommendedWatcher> = notify::recommended_watcher(move |_res| {
        let _ = tx.send(());
    })
    .ok();
    if let Some(w) = watcher.as_mut() {
        let _ = std::fs::create_dir_all(eye_dir);
        let _ = w.watch(sessions_dir, RecursiveMode::NonRecursive);
        let _ = w.watch(eye_dir, RecursiveMode::NonRecursive);
    }

    on_change();
    loop {
        match rx.recv_timeout(tick) {
            Ok(()) => {
                // Debounce: swallow events for 200 ms, then refresh once.
                std::thread::sleep(Duration::from_millis(200));
                while rx.try_recv().is_ok() {}
                on_change();
            }
            Err(RecvTimeoutError::Timeout) => on_change(),
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(tick);
                on_change();
            }
        }
    }
}
```

- [ ] **Step 4: Wire up lib.rs**

Replace the whole of `src-tauri/src/lib.rs` with:

```rust
pub mod config;
pub mod events;
pub mod focus;
pub mod hook_install;
pub mod model;
pub mod registry;
pub mod state;
pub mod store;
pub mod transcript;
pub mod watcher;

use config::Config;
use model::Card;
use std::sync::Mutex;
use std::time::Duration;
use store::{now_ms, Store};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

pub struct AppState {
    pub store: Mutex<Store>,
}

fn claude_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/")).join(".claude")
}

fn refresh_and_emit(app: &AppHandle) {
    let cards = {
        let state = app.state::<AppState>();
        let mut store = state.store.lock().unwrap();
        store.refresh(now_ms())
    };
    let _ = app.emit("sessions", &cards);
}

#[tauri::command]
fn list_sessions(state: TauriState<AppState>) -> Vec<Card> {
    state.store.lock().unwrap().refresh(now_ms())
}

#[tauri::command]
fn focus_session(pid: i32) -> Result<(), String> {
    focus::focus_pid(pid)
}

#[tauri::command]
fn hook_status(state: TauriState<AppState>) -> Result<bool, String> {
    let dir = state.store.lock().unwrap().claude_dir().to_path_buf();
    hook_install::status(&dir)
}

#[tauri::command]
fn install_hook(state: TauriState<AppState>) -> Result<bool, String> {
    let dir = state.store.lock().unwrap().claude_dir().to_path_buf();
    hook_install::install_to(&dir)?;
    hook_install::status(&dir)
}

#[tauri::command]
fn remove_hook(state: TauriState<AppState>) -> Result<bool, String> {
    let dir = state.store.lock().unwrap().claude_dir().to_path_buf();
    hook_install::remove_from(&dir)?;
    hook_install::status(&dir)
}

#[tauri::command]
fn get_config(state: TauriState<AppState>) -> Config {
    state.store.lock().unwrap().config.clone()
}

#[tauri::command]
fn set_config(app: AppHandle, state: TauriState<AppState>, config: Config) -> Result<Config, String> {
    if config.completed_timeout_minutes == 0 {
        return Err("Completed timeout must be at least 1 minute".into());
    }
    {
        let mut store = state.store.lock().unwrap();
        config::save(&store.config_path(), &config)?;
        store.config = config.clone();
    }
    refresh_and_emit(&app);
    Ok(config)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let dir = claude_dir();
    let mut store = Store::new(dir.clone());
    store.compact_events();

    tauri::Builder::default()
        .manage(AppState { store: Mutex::new(store) })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            focus_session,
            hook_status,
            install_hook,
            remove_hook,
            get_config,
            set_config
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let sessions_dir = dir.join("sessions");
            let eye_dir = dir.join("eye");
            std::thread::spawn(move || {
                watcher::run(&sessions_dir, &eye_dir, Duration::from_secs(5), || refresh_and_emit(&handle));
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 5: Compile, run all tests, smoke-run the app**

```bash
cd /Users/tiagocorreia/dev/eye/src-tauri && cargo test 2>&1 | tail -3
```
Expected: all tests pass (about 41), 0 failed.

Smoke run: temporarily replace the body of `src/main.ts` with

```ts
import { listen } from "@tauri-apps/api/event";
listen("sessions", (e) => console.log("sessions", JSON.stringify(e.payload).slice(0, 300)));
```

then `pnpm tauri dev`, open the webview devtools (right-click, Inspect Element) and confirm a `sessions` log line with real card data appears within 5 seconds. Stop the app.

- [ ] **Step 6: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src-tauri/src src/main.ts
git commit -m "feat: store, file watcher and Tauri commands for the session board

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Frontend board rendering with tests

**Files:**
- Create: `src/types.ts`, `src/format.ts`, `src/card.ts`, `src/board.ts`, `src/board.test.ts`, `vitest.config.ts`
- Modify: `package.json` (add vitest, jsdom, test script), `index.html`, `src/styles.css`

**Interfaces:**
- Consumes: the `Card` JSON shape from `model.rs`.
- Produces:
  - `types.ts`: `Card`, `CardState = 'awaiting' | 'working' | 'completed' | 'idle'`, `COLUMNS: {state: CardState, title: string}[]` in display order.
  - `format.ts`: `formatAge(sinceMs: number, nowMs: number): string`, `projectName(cwd: string): string`
  - `card.ts`: `renderCard(card: Card, nowMs: number): HTMLElement` (has `data-session-id`, `data-pid`, class `card card--<state>`)
  - `board.ts`: `renderBoard(cards: Card[], nowMs: number): HTMLElement` (four `.column` elements with `data-state`, each with `.column__count` and `.column__cards`)

- [ ] **Step 1: Add test tooling**

```bash
cd /Users/tiagocorreia/dev/eye && pnpm add -D vitest jsdom
```

Add to `package.json` `"scripts"`: `"test": "vitest run"`.

Create `vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
export default defineConfig({ test: { environment: "jsdom", include: ["src/**/*.test.ts"] } });
```

- [ ] **Step 2: Write types**

Create `src/types.ts`:

```ts
export type CardState = "awaiting" | "working" | "completed" | "idle";
export type AwaitKind = "question" | "plan" | "permission";

export interface Card {
  sessionId: string;
  pid: number;
  name: string;
  cwd: string;
  state: CardState;
  stateSince: number;
  snippet: string;
  awaiting: { kind: AwaitKind; detail: string } | null;
}

export const COLUMNS: { state: CardState; title: string }[] = [
  { state: "awaiting", title: "Awaiting Decision" },
  { state: "working", title: "Working" },
  { state: "completed", title: "Completed" },
  { state: "idle", title: "Idle" },
];
```

- [ ] **Step 3: Write the failing tests**

Create `src/board.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { renderBoard } from "./board";
import { renderCard } from "./card";
import { formatAge, projectName } from "./format";
import type { Card } from "./types";

const NOW = 1_790_600_000_000;

function card(over: Partial<Card>): Card {
  return {
    sessionId: "s",
    pid: 1,
    name: "eye-1",
    cwd: "/Users/x/dev/eye",
    state: "idle",
    stateSince: NOW - 90_000,
    snippet: "",
    awaiting: null,
    ...over,
  };
}

describe("formatAge", () => {
  it("formats seconds, minutes, hours and days", () => {
    expect(formatAge(NOW - 5_000, NOW)).toBe("5s");
    expect(formatAge(NOW - 90_000, NOW)).toBe("1m");
    expect(formatAge(NOW - 3 * 3_600_000, NOW)).toBe("3h");
    expect(formatAge(NOW - 2 * 86_400_000, NOW)).toBe("2d");
    expect(formatAge(NOW + 1000, NOW)).toBe("0s");
  });
});

describe("projectName", () => {
  it("uses the last path segment", () => {
    expect(projectName("/Users/x/dev/eye")).toBe("eye");
    expect(projectName("/Users/x")).toBe("x");
    expect(projectName("/")).toBe("/");
  });
});

describe("renderCard", () => {
  it("shows name, project, age and snippet", () => {
    const el = renderCard(card({ snippet: "All done." }), NOW);
    expect(el.dataset.sessionId).toBe("s");
    expect(el.dataset.pid).toBe("1");
    expect(el.className).toContain("card--idle");
    expect(el.querySelector(".card__name")?.textContent).toBe("eye-1");
    expect(el.querySelector(".card__project")?.textContent).toBe("eye");
    expect(el.querySelector(".card__age")?.textContent).toBe("1m");
    expect(el.querySelector(".card__snippet")?.textContent).toBe("All done.");
  });

  it("shows the awaiting detail with its kind", () => {
    const el = renderCard(card({ state: "awaiting", awaiting: { kind: "permission", detail: "Bash: rm -rf build" } }), NOW);
    expect(el.querySelector(".card__awaiting")?.textContent).toBe("Bash: rm -rf build");
    expect(el.querySelector(".card__awaiting")?.getAttribute("data-kind")).toBe("permission");
  });

  it("escapes text content", () => {
    const el = renderCard(card({ snippet: "<img src=x onerror=alert(1)>" }), NOW);
    expect(el.querySelector("img")).toBeNull();
  });
});

describe("renderBoard", () => {
  it("renders four columns in order with counts and cards in the right column", () => {
    const cards: Card[] = [
      card({ sessionId: "a", name: "a", state: "working" }),
      card({ sessionId: "b", name: "b", state: "awaiting", awaiting: { kind: "question", detail: "Which?" } }),
      card({ sessionId: "c", name: "c", state: "idle" }),
      card({ sessionId: "d", name: "d", state: "completed" }),
      card({ sessionId: "e", name: "e", state: "working" }),
    ];
    const board = renderBoard(cards, NOW);
    const cols = [...board.querySelectorAll<HTMLElement>(".column")];
    expect(cols.map((c) => c.dataset.state)).toEqual(["awaiting", "working", "completed", "idle"]);
    expect(cols.map((c) => c.querySelector(".column__title")?.textContent)).toEqual(["Awaiting Decision", "Working", "Completed", "Idle"]);
    expect(cols.map((c) => c.querySelector(".column__count")?.textContent)).toEqual(["1", "2", "1", "1"]);
    expect([...cols[1].querySelectorAll(".card")].map((c) => (c as HTMLElement).dataset.sessionId)).toEqual(["a", "e"]);
  });

  it("orders cards in a column by most recent state change first", () => {
    const cards: Card[] = [
      card({ sessionId: "old", state: "idle", stateSince: NOW - 10_000 }),
      card({ sessionId: "new", state: "idle", stateSince: NOW - 1_000 }),
    ];
    const ids = [...renderBoard(cards, NOW).querySelectorAll<HTMLElement>(".card")].map((c) => c.dataset.sessionId);
    expect(ids).toEqual(["new", "old"]);
  });

  it("keeps two sessions with the same cwd as separate cards", () => {
    const cards: Card[] = [
      card({ sessionId: "x1", name: "envoy-1b", cwd: "/Users/x/dev/envoy" }),
      card({ sessionId: "x2", name: "envoy-ab", cwd: "/Users/x/dev/envoy" }),
    ];
    expect(renderBoard(cards, NOW).querySelectorAll(".card").length).toBe(2);
  });

  it("shows an empty hint in an empty column", () => {
    const board = renderBoard([], NOW);
    expect(board.querySelectorAll(".column__empty").length).toBe(4);
  });
});
```

- [ ] **Step 4: Run tests to verify they fail**

```bash
cd /Users/tiagocorreia/dev/eye && pnpm test
```
Expected: fails, cannot resolve `./board`, `./card`, `./format`.

- [ ] **Step 5: Implement format, card and board**

`src/format.ts`:

```ts
export function formatAge(sinceMs: number, nowMs: number): string {
  const s = Math.max(0, Math.floor((nowMs - sinceMs) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

export function projectName(cwd: string): string {
  const parts = cwd.split("/").filter(Boolean);
  return parts.length ? parts[parts.length - 1] : cwd;
}
```

`src/card.ts`:

```ts
import { formatAge, projectName } from "./format";
import type { Card } from "./types";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function renderCard(card: Card, nowMs: number): HTMLElement {
  const root = el("article", `card card--${card.state}`);
  root.dataset.sessionId = card.sessionId;
  root.dataset.pid = String(card.pid);
  root.tabIndex = 0;
  root.title = card.cwd;

  const head = el("header", "card__head");
  head.append(el("span", "card__name", card.name), el("span", "card__age", formatAge(card.stateSince, nowMs)));
  root.append(head, el("div", "card__project", projectName(card.cwd)));

  if (card.awaiting) {
    const a = el("div", "card__awaiting", card.awaiting.detail);
    a.dataset.kind = card.awaiting.kind;
    root.append(a);
  }
  if (card.snippet) root.append(el("p", "card__snippet", card.snippet));
  return root;
}
```

`src/board.ts`:

```ts
import { renderCard } from "./card";
import { COLUMNS, type Card } from "./types";

export function renderBoard(cards: Card[], nowMs: number): HTMLElement {
  const board = document.createElement("main");
  board.className = "board";
  for (const col of COLUMNS) {
    const inCol = cards.filter((c) => c.state === col.state).sort((a, b) => b.stateSince - a.stateSince);
    const section = document.createElement("section");
    section.className = `column column--${col.state}`;
    section.dataset.state = col.state;

    const head = document.createElement("header");
    head.className = "column__head";
    const title = document.createElement("h2");
    title.className = "column__title";
    title.textContent = col.title;
    const count = document.createElement("span");
    count.className = "column__count";
    count.textContent = String(inCol.length);
    head.append(title, count);

    const list = document.createElement("div");
    list.className = "column__cards";
    if (inCol.length === 0) {
      const empty = document.createElement("div");
      empty.className = "column__empty";
      empty.textContent = "Nothing here";
      list.append(empty);
    }
    for (const c of inCol) list.append(renderCard(c, nowMs));

    section.append(head, list);
    board.append(section);
  }
  return board;
}
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cd /Users/tiagocorreia/dev/eye && pnpm test
```
Expected: all tests pass (10).

- [ ] **Step 7: Page shell and styles**

Replace `index.html` body with:

```html
<body>
  <div id="app">
    <header class="topbar">
      <h1 class="topbar__title">Eye</h1>
      <span class="topbar__meta" id="meta"></span>
      <button class="topbar__settings" id="settings-toggle" type="button">Settings</button>
    </header>
    <div id="board"></div>
    <aside id="settings" class="settings" hidden></aside>
    <div id="toast" class="toast" hidden></div>
  </div>
  <script type="module" src="/src/main.ts"></script>
</body>
```

Replace `src/styles.css` with:

```css
:root {
  --bg: #f4f4f6; --panel: #ffffff; --text: #1a1a1f; --muted: #6b6b76; --border: #e2e2e8;
  --awaiting: #d97706; --working: #2563eb; --completed: #16a34a; --idle: #9ca3af;
  --shadow: 0 1px 2px rgba(0,0,0,.06);
  font-family: -apple-system, BlinkMacSystemFont, "Inter", system-ui, sans-serif;
  font-size: 14px; color: var(--text); background: var(--bg);
}
@media (prefers-color-scheme: dark) {
  :root { --bg: #16161a; --panel: #1f1f25; --text: #ececf1; --muted: #9a9aa6; --border: #2c2c34; --shadow: none; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); }
#app { display: flex; flex-direction: column; height: 100vh; }
.topbar { display: flex; align-items: center; gap: 12px; padding: 10px 16px; border-bottom: 1px solid var(--border); }
.topbar__title { margin: 0; font-size: 16px; font-weight: 600; }
.topbar__meta { color: var(--muted); flex: 1; }
.topbar__settings, .settings button { background: var(--panel); border: 1px solid var(--border); border-radius: 6px; padding: 5px 10px; color: var(--text); cursor: pointer; }
.board { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; padding: 12px 16px; flex: 1; overflow: hidden; }
.column { display: flex; flex-direction: column; min-height: 0; border-top: 3px solid var(--idle); background: color-mix(in srgb, var(--panel) 60%, var(--bg)); border-radius: 8px; }
.column--awaiting { border-top-color: var(--awaiting); }
.column--working { border-top-color: var(--working); }
.column--completed { border-top-color: var(--completed); }
.column__head { display: flex; justify-content: space-between; align-items: baseline; padding: 10px 12px 6px; }
.column__title { margin: 0; font-size: 13px; text-transform: uppercase; letter-spacing: .04em; color: var(--muted); }
.column__count { color: var(--muted); font-variant-numeric: tabular-nums; }
.column__cards { overflow-y: auto; padding: 0 8px 8px; display: flex; flex-direction: column; gap: 8px; }
.column__empty { color: var(--muted); font-style: italic; padding: 12px; text-align: center; }
.card { background: var(--panel); border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px; box-shadow: var(--shadow); cursor: pointer; }
.card:hover, .card:focus-visible { border-color: var(--working); outline: none; }
.card__head { display: flex; justify-content: space-between; gap: 8px; }
.card__name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card__age { color: var(--muted); font-variant-numeric: tabular-nums; }
.card__project { color: var(--muted); font-size: 12px; margin-top: 2px; }
.card__awaiting { margin-top: 8px; padding: 6px 8px; border-radius: 6px; background: color-mix(in srgb, var(--awaiting) 14%, transparent); border-left: 3px solid var(--awaiting); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; white-space: pre-wrap; word-break: break-word; }
.card__awaiting[data-kind="question"] { background: color-mix(in srgb, var(--working) 12%, transparent); border-left-color: var(--working); font-family: inherit; }
.card__awaiting[data-kind="plan"] { background: color-mix(in srgb, var(--completed) 12%, transparent); border-left-color: var(--completed); font-family: inherit; }
.card__snippet { margin: 8px 0 0; color: var(--muted); font-size: 12.5px; line-height: 1.4; display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
.settings { position: absolute; top: 48px; right: 16px; width: 320px; background: var(--panel); border: 1px solid var(--border); border-radius: 10px; padding: 14px; box-shadow: 0 8px 30px rgba(0,0,0,.18); display: flex; flex-direction: column; gap: 10px; }
.settings[hidden], .toast[hidden] { display: none; }
.settings label { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
.settings input { width: 70px; padding: 4px 6px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); color: var(--text); }
.settings__status { color: var(--muted); font-size: 12.5px; }
.settings__error { color: #dc2626; font-size: 12.5px; }
.toast { position: fixed; bottom: 16px; left: 50%; transform: translateX(-50%); background: var(--text); color: var(--bg); padding: 8px 14px; border-radius: 8px; font-size: 13px; }
```

- [ ] **Step 8: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add package.json pnpm-lock.yaml vitest.config.ts index.html src/styles.css src/types.ts src/format.ts src/card.ts src/board.ts src/board.test.ts
git commit -m "feat: kanban board rendering with tests

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Wire the board to live data and click-to-focus

**Files:**
- Create: `src/toast.ts`, `src/toast.test.ts`
- Modify: `src/main.ts` (full rewrite)

**Interfaces:**
- Consumes: Tauri event `sessions` (`Card[]`), commands `list_sessions`, `focus_session`.
- Produces: `toast.ts`: `showToast(message: string, ms?: number): void` (uses `#toast`).

- [ ] **Step 1: Write the failing toast test**

`src/toast.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { showToast } from "./toast";

describe("showToast", () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="toast" class="toast" hidden></div>';
    vi.useFakeTimers();
  });

  it("shows the message then hides after the delay", () => {
    showToast("No Terminal tab found", 1000);
    const t = document.getElementById("toast")!;
    expect(t.hidden).toBe(false);
    expect(t.textContent).toBe("No Terminal tab found");
    vi.advanceTimersByTime(1001);
    expect(t.hidden).toBe(true);
  });

  it("resets the timer when called again", () => {
    showToast("one", 1000);
    vi.advanceTimersByTime(800);
    showToast("two", 1000);
    vi.advanceTimersByTime(800);
    expect(document.getElementById("toast")!.hidden).toBe(false);
    expect(document.getElementById("toast")!.textContent).toBe("two");
  });
});
```

Run `pnpm test`. Expected: fails to resolve `./toast`.

- [ ] **Step 2: Implement toast**

`src/toast.ts`:

```ts
let timer: ReturnType<typeof setTimeout> | undefined;

export function showToast(message: string, ms = 3000): void {
  const el = document.getElementById("toast");
  if (!el) return;
  el.textContent = message;
  el.hidden = false;
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    el.hidden = true;
  }, ms);
}
```

Run `pnpm test`. Expected: all pass (12).

- [ ] **Step 3: Write main.ts**

Replace `src/main.ts` with:

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderBoard } from "./board";
import { showToast } from "./toast";
import type { Card } from "./types";
import "./styles.css";

let cards: Card[] = [];

function paint(): void {
  const host = document.getElementById("board");
  if (!host) return;
  host.replaceChildren(renderBoard(cards, Date.now()));
  const meta = document.getElementById("meta");
  if (meta) meta.textContent = `${cards.length} session${cards.length === 1 ? "" : "s"}`;
}

async function focus(pid: number): Promise<void> {
  try {
    await invoke("focus_session", { pid });
  } catch (e) {
    showToast(String(e));
  }
}

async function start(): Promise<void> {
  document.getElementById("board")?.addEventListener("click", (ev) => {
    const card = (ev.target as HTMLElement).closest<HTMLElement>(".card");
    if (card?.dataset.pid) void focus(Number(card.dataset.pid));
  });
  document.getElementById("board")?.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter") return;
    const card = (ev.target as HTMLElement).closest<HTMLElement>(".card");
    if (card?.dataset.pid) void focus(Number(card.dataset.pid));
  });

  await listen<Card[]>("sessions", (e) => {
    cards = e.payload;
    paint();
  });
  cards = await invoke<Card[]>("list_sessions");
  paint();
  setInterval(paint, 10_000); // refresh the age labels
}

void start();
```

- [ ] **Step 4: Run the app and check against live sessions**

```bash
cd /Users/tiagocorreia/dev/eye && pnpm tauri dev
```
Expected: the window shows four columns, every live session from `ls ~/.claude/sessions` appears as a card in Working or Idle (hook not installed yet). Click a card: its Terminal tab comes to the front. Type in one of those sessions: within a few seconds its card moves to Working.

- [ ] **Step 5: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src/main.ts src/toast.ts src/toast.test.ts
git commit -m "feat: live board with click-to-focus and toasts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Settings panel: hook install and Completed timeout

**Files:**
- Create: `src/settings.ts`, `src/settings.test.ts`
- Modify: `src/main.ts` (call `initSettings`)

**Interfaces:**
- Consumes: commands `hook_status`, `install_hook`, `remove_hook`, `get_config`, `set_config`.
- Produces: `settings.ts`: `renderSettings(model: SettingsModel, handlers: SettingsHandlers): HTMLElement` (pure) and `initSettings(): Promise<void>` (wires `#settings` and `#settings-toggle`).
  - `SettingsModel { hookInstalled: boolean | null; completedTimeoutMinutes: number; error: string | null }`
  - `SettingsHandlers { onInstall(): void; onRemove(): void; onTimeout(minutes: number): void }`

- [ ] **Step 1: Write the failing test**

`src/settings.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { renderSettings } from "./settings";

const handlers = () => ({ onInstall: vi.fn(), onRemove: vi.fn(), onTimeout: vi.fn() });

describe("renderSettings", () => {
  it("offers install when the hook is missing", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: false, completedTimeoutMinutes: 30, error: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("not installed");
    const btn = el.querySelector<HTMLButtonElement>("button[data-action=install]")!;
    btn.click();
    expect(h.onInstall).toHaveBeenCalled();
    expect(el.querySelector("button[data-action=remove]")).toBeNull();
  });

  it("offers remove when the hook is installed", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: true, completedTimeoutMinutes: 30, error: null }, h);
    expect(el.querySelector(".settings__status")?.textContent).toContain("installed");
    el.querySelector<HTMLButtonElement>("button[data-action=remove]")!.click();
    expect(h.onRemove).toHaveBeenCalled();
  });

  it("reports timeout changes and shows errors", () => {
    const h = handlers();
    const el = renderSettings({ hookInstalled: null, completedTimeoutMinutes: 30, error: "boom" }, h);
    const input = el.querySelector<HTMLInputElement>("input[name=timeout]")!;
    expect(input.value).toBe("30");
    input.value = "45";
    input.dispatchEvent(new Event("change"));
    expect(h.onTimeout).toHaveBeenCalledWith(45);
    expect(el.querySelector(".settings__error")?.textContent).toBe("boom");
  });
});
```

Run `pnpm test`. Expected: cannot resolve `./settings`.

- [ ] **Step 2: Implement**

`src/settings.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export interface SettingsModel {
  hookInstalled: boolean | null;
  completedTimeoutMinutes: number;
  error: string | null;
}

export interface SettingsHandlers {
  onInstall(): void;
  onRemove(): void;
  onTimeout(minutes: number): void;
}

export function renderSettings(model: SettingsModel, h: SettingsHandlers): HTMLElement {
  const root = document.createElement("div");
  root.className = "settings__body";

  const status = document.createElement("div");
  status.className = "settings__status";
  status.textContent =
    model.hookInstalled === null
      ? "Checking hook…"
      : model.hookInstalled
        ? "Claude Code hook is installed. Awaiting Decision and Completed are precise."
        : "Claude Code hook is not installed. Permission prompts will show as Working.";
  root.append(status);

  const btn = document.createElement("button");
  btn.type = "button";
  if (model.hookInstalled) {
    btn.dataset.action = "remove";
    btn.textContent = "Remove hook";
    btn.addEventListener("click", () => h.onRemove());
  } else {
    btn.dataset.action = "install";
    btn.textContent = "Install hook";
    btn.disabled = model.hookInstalled === null;
    btn.addEventListener("click", () => h.onInstall());
  }
  root.append(btn);

  const label = document.createElement("label");
  label.textContent = "Completed decays to Idle after (minutes)";
  const input = document.createElement("input");
  input.type = "number";
  input.name = "timeout";
  input.min = "1";
  input.value = String(model.completedTimeoutMinutes);
  input.addEventListener("change", () => {
    const n = Number(input.value);
    if (Number.isFinite(n) && n >= 1) h.onTimeout(Math.floor(n));
  });
  label.append(input);
  root.append(label);

  if (model.error) {
    const err = document.createElement("div");
    err.className = "settings__error";
    err.textContent = model.error;
    root.append(err);
  }
  return root;
}

export async function initSettings(): Promise<void> {
  const panel = document.getElementById("settings");
  const toggle = document.getElementById("settings-toggle");
  if (!panel || !toggle) return;

  const model: SettingsModel = { hookInstalled: null, completedTimeoutMinutes: 30, error: null };

  const paint = () => panel.replaceChildren(renderSettings(model, handlers));

  const run = async (action: () => Promise<void>) => {
    model.error = null;
    try {
      await action();
    } catch (e) {
      model.error = String(e);
    }
    paint();
  };

  const handlers: SettingsHandlers = {
    onInstall: () => void run(async () => { model.hookInstalled = await invoke<boolean>("install_hook"); }),
    onRemove: () => void run(async () => { model.hookInstalled = await invoke<boolean>("remove_hook"); }),
    onTimeout: (minutes) =>
      void run(async () => {
        const c = await invoke<{ completedTimeoutMinutes: number }>("set_config", { config: { completedTimeoutMinutes: minutes } });
        model.completedTimeoutMinutes = c.completedTimeoutMinutes;
      }),
  };

  toggle.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
  });

  await run(async () => {
    const [installed, config] = await Promise.all([
      invoke<boolean>("hook_status"),
      invoke<{ completedTimeoutMinutes: number }>("get_config"),
    ]);
    model.hookInstalled = installed;
    model.completedTimeoutMinutes = config.completedTimeoutMinutes;
  });
}
```

In `src/main.ts`, add `import { initSettings } from "./settings";` and, as the first line inside `start()`, `void initSettings();`.

- [ ] **Step 3: Run tests**

```bash
cd /Users/tiagocorreia/dev/eye && pnpm test
```
Expected: all pass (15).

- [ ] **Step 4: Manual acceptance (the spec's acceptance list)**

Run `pnpm tauri dev`, then:

1. Open Settings, click **Install hook**. Status changes to installed. Check `jq .hooks ~/.claude/settings.json | grep -c hook.sh` prints `9`, and a `~/.claude/settings.json.eye-backup-*` file exists.
2. In a fresh terminal tab run `claude` in any project and ask it to run a command that needs permission, for example `run ls -la /tmp`. Within a few seconds its card moves to **Awaiting Decision** with a `Bash: ls -la /tmp` line. Approve it: the card returns to **Working**.
3. Ask that session something trivial and let it finish. When it stops, the card moves to **Completed**. Type a new prompt: it goes back to **Working**.
4. Set the timeout to `1` minute, let a session finish, wait 60 seconds: the card drops to **Idle**. Set it back to `30`.
5. Click a card: its Terminal tab comes forward. Quit one Claude session with `/exit`: its card disappears within 5 seconds.
6. Close and reopen the app: `wc -l ~/.claude/eye/events.jsonl` is not larger than before (compaction ran).

Record the outcome of each step in the commit message body or a note in the PR if any step fails.

- [ ] **Step 5: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add src/settings.ts src/settings.test.ts src/main.ts
git commit -m "feat: settings panel for hook install and completed timeout

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: README

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write the README**

```markdown
# Eye

A macOS desktop board for your local Claude Code sessions. Every running
session is a card in one of four columns: Awaiting Decision, Working,
Completed, Idle. Click a card to jump to its Terminal tab.

## How it works

- Reads the session registry Claude Code keeps at `~/.claude/sessions/`.
- Optionally installs a Claude Code hook (Settings → Install hook) that appends
  hook payloads to `~/.claude/eye/events.jsonl`. This is what makes permission
  prompts and finished turns show precisely. The hook never blocks and prints
  nothing; a backup of `settings.json` is taken before every change.
- Reads the tail of each session transcript for the card snippet.

## Develop

    pnpm install
    pnpm tauri dev      # run
    pnpm test           # frontend tests
    cd src-tauri && cargo test   # Rust tests

Requires Rust (rustup), Node 20+, pnpm, and `jq` on PATH for the hook.

## Build

    pnpm tauri build

The app bundle lands in `src-tauri/target/release/bundle/`.
```

- [ ] **Step 2: Commit**

```bash
cd /Users/tiagocorreia/dev/eye
git add README.md
git commit -m "docs: README

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
