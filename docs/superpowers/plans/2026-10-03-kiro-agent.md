# Kiro CLI as a Maya agent — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kiro CLI becomes Maya's fifth agent, with everything the other four have: live sessions on the board, New session, Resume, rename, the session controls Kiro has, Maya's brain, reviews, other machines, Windows and Linux.

**Architecture:** A `Harness::Kiro` variant and a new `core/src/kiro.rs` that reads Kiro's lock files, session files, event logs and per-turn markers. Discovery follows Grok's registry model (a file names the pid), the card state follows `foreign::derive`, and the launch, listing, resume and capability tables gain a Kiro row each. `Harness::Other` is the decode fallback for agents this build does not know, so a newer assistant never freezes an older main's board again.

**Tech Stack:** Rust (Cargo workspace: `core/`, `src-tauri/`, `cli/`), Tauri 2, vanilla TypeScript with vitest, serde_json.

**Spec:** `docs/superpowers/specs/2026-10-03-maya-kiro-agent-design.md`

**Amended after the real-app check (2026-10-04):** Kiro's `/compact` summarises the conversation, so `compact` is true for Kiro everywhere below; and every agent takes `/exit`, so the capability table gained a `close` field (true for every agent, false for `Other`) and Close works for all five. The spec is the authority; the snippets below were corrected where they said otherwise.

## Global Constraints

- Branch `feat/kiro-agent` (already created, holds the spec); never commit to or push `main`; ask the user before each push (user rule).
- Commit messages are conventional (`feat:`, `fix:`, `docs:`, `chore:`), ending with the attribution lines from the session reminder.
- Harness ids are kebab-case in JSON: `claude-code`, `codex`, `antigravity`, `grok`, `kiro`, `other` (`core/src/model.rs:57-62`, `src/types.ts:20`).
- Shell lines quote every user-derived value with `launch::shell_single_quote`; model ids pass `launch::plain_model_id`; session ids pass `resume::plain_session_id`.
- A one-shot run has no tools, one turn, 25 s timeout, Maya's data folder as its working directory, cleared environment plus `launch::clean_env`.
- Copy: the Kiro label is "Kiro CLI"; the unknown-harness label is "Unknown agent"; refusals read "<Agent> has no <control>."; the five-agent list reads "Claude Code, Codex, Antigravity, Grok Build or Kiro CLI".
- Kiro has the Compact control: its `/compact` summarises the conversation.
- Fixtures in `core/fixtures/kiro/` were saved from this Mac (Kiro CLI 2.27.0) and are already in the working tree, untracked: `session.json`, `events.jsonl` (a finished turn with a tool call, then a second prompt whose tool call has no result yet), `lock.json`, `marker.json`, `models.json`, `oneshot.jsonl`. `src/assets/icons/kiro.png` (64x64) is there too. Task 1 commits them.
- Tests: `cargo test --workspace` from the repo root and `pnpm test`; the frontend must also pass `pnpm build` (runs `tsc`).
- Release: a `feat`, so the last task bumps the minor version from `main`'s (0.10.0 → 0.11.0 unless `main` moved) with `sh scripts/set-version.sh`, committed alone as `chore(release): 0.11.0`.

## Review Focus

1. A Kiro lock file whose pid is alive but whose parent chain has no tty (a `kiro-cli chat --no-interactive` run, or Maya's own one-shot): no card. Pinned in Task 3 (`kiro_sessions` skips it).
2. A turn marker left behind by a crashed TUI, or one whose heartbeat stopped: the card must not stay Working forever. Pinned in Task 2 (`working_marker` ignores a heartbeat older than two minutes).
3. A session file with `"title": null` and an empty event log (a session just opened): the card is named `kiro-<pid>`, the resume row is named by its id, and nothing panics on the empty log. Pinned in Task 2 (`parse_events("")`) and Task 6 (first-prompt fallback).
4. A board from a newer Maya carrying a card with a harness this build does not know: the board still decodes, the card shows "Unknown agent", and every control on it is refused rather than typed into a tty. Pinned in Task 1 (`Harness::Other`) and Task 8 (`capabilitiesOf("other")`).
5. A New session prompt that starts with `-` (`-v please`): Kiro must read it as the prompt, not an option. Pinned in Task 5 (`session_command` ends with `-- "$p"`) and the real-app check in Task 10.

---

### Task 1: `Harness::Kiro`, `Harness::Other`, and the launch tables

**Files:**
- Modify: `core/src/model.rs:53-69`
- Modify: `core/src/launch.rs:16-85, 153-175, 219-307, 562-579`
- Modify: `core/src/agents.rs:105-111, 116-125, 138-149`
- Modify: `core/src/resume.rs:56-63, 170-200`
- Modify: `core/src/foreign.rs:161-209, 215-239`
- Modify: `core/src/net/protocol.rs` (tests)
- Add: `core/fixtures/kiro/*` (already on disk), `src/assets/icons/kiro.png` (already on disk)

**Interfaces:**
- Produces: `Harness::Kiro` (wire `kiro`), `Harness::Other` (wire `other`, the `#[serde(other)]` fallback); `launch::binary_name(Kiro) == "kiro-cli"`, `launch::label(Kiro) == "Kiro CLI"`, `launch::efforts(Kiro)`, `launch::modes(Kiro) == ["default", "trust-all"]`, `launch::capabilities(Kiro)`, `LaunchOptions::flags()` for Kiro; `resume::AgentDirs.kiro: PathBuf`.
- Later tasks fill the Kiro arms this task leaves empty in `agents.rs`, `resume.rs` and `foreign.rs`.

- [ ] **Step 1: Write the failing tests**

In `core/src/model.rs` tests module, add:

```rust
    #[test]
    fn kiro_and_unknown_harnesses_have_wire_names() {
        assert_eq!(serde_json::to_string(&Harness::Kiro).unwrap(), "\"kiro\"");
        assert_eq!(serde_json::from_str::<Harness>("\"kiro\"").unwrap(), Harness::Kiro);
        // A harness this build has never heard of decodes to Other instead of failing the whole board.
        assert_eq!(serde_json::from_str::<Harness>("\"future-agent\"").unwrap(), Harness::Other);
        assert_eq!(serde_json::to_string(&Harness::Other).unwrap(), "\"other\"");
    }
```

In `core/src/launch.rs` tests, extend `labels_name_every_agent`, `capabilities_follow_the_table` and `each_agent_renders_its_own_flags`:

```rust
        assert_eq!(label(Harness::Kiro), "Kiro CLI");
        assert_eq!(label(Harness::Other), "Unknown agent");
```

```rust
        let k = capabilities(Harness::Kiro);
        assert!(k.compact && k.model_switch && k.effort_switch && k.slash_lines && k.shell_lines);
        assert_eq!(k.mode_cycle, Some(crate::answer::SHIFT_TAB));
        let o = capabilities(Harness::Other);
        assert!(!o.compact && !o.model_switch && !o.effort_switch && !o.slash_lines && !o.shell_lines);
        assert_eq!(o.mode_cycle, None);
```

```rust
        assert_eq!(all(Harness::Kiro).flags(), " --model m-1 --effort high", "the default mode passes nothing");
        let trust = LaunchOptions { agent: Harness::Kiro, mode: Some("trust-all".into()), ..Default::default() };
        assert_eq!(trust.flags(), " --trust-all-tools");
        assert_eq!(all(Harness::Other).flags(), "", "an unknown agent gets no flags");
        assert!(o(Harness::Kiro, "high", "plan").validate_shape().unwrap_err().contains("mode"));
        assert!(o(Harness::Kiro, "ultra", "default").validate_shape().unwrap_err().contains("effort"));
```

(`o` is the closure defined in `effort_and_mode_must_be_the_agents_own`; put those two asserts there.)

In `core/src/net/protocol.rs` tests, add:

```rust
    #[test]
    fn a_card_from_an_agent_this_maya_does_not_know_still_decodes() {
        let mut card = crate::model::Card::default_for_test("s1");
        card.harness = crate::model::Harness::Kiro;
        let text = encode(&Up::Board { cards: vec![card], dirs: vec![], agents: None }).replace("\"harness\":\"kiro\"", "\"harness\":\"future-agent\"");
        let Up::Board { cards, .. } = decode_up(&text).unwrap() else { panic!() };
        assert_eq!(cards[0].harness, crate::model::Harness::Other);
    }
```

If `Card::default_for_test` does not exist, add it to `core/src/model.rs` under `#[cfg(any(test, feature = "test-support"))]`, building a Card with `session_id: id.into(), pid: 1, name: id.into(), cwd: "/x".into(), state: State::Idle, state_since: 0, snippet: String::new(), awaiting: None, has_inbox: false, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core -- harness kiro labels_name capabilities_follow each_agent_renders a_card_from_an_agent`
Expected: compile errors naming `Harness::Kiro` and `Harness::Other`.

- [ ] **Step 3: Add the variants**

In `core/src/model.rs` replace the enum and its doc:

```rust
/// The agent runner a session belongs to. A new harness adds a variant here
/// and its own reader module. `Other` is what an unknown wire name decodes
/// to, so a board from a newer Maya still reads; it has no binary, no
/// listing and no controls.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Antigravity,
    Grok,
    Kiro,
    #[serde(other)]
    Other,
}
```

- [ ] **Step 4: Fill every exhaustive match**

`core/src/launch.rs`:

```rust
pub fn binary_name(agent: Harness) -> &'static str {
    match agent {
        Harness::ClaudeCode => "claude",
        Harness::Codex => "codex",
        Harness::Antigravity => "agy",
        Harness::Grok => "grok",
        Harness::Kiro => "kiro-cli",
        // Never installed: find_binary names it in its "could not find" message.
        Harness::Other => "unknown-agent",
    }
}

pub fn efforts(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::ClaudeCode => EFFORTS,
        Harness::Codex => &["low", "medium", "high", "xhigh", "max", "ultra"],
        Harness::Antigravity => &["low", "medium", "high", "max"],
        Harness::Grok => &[],
        Harness::Kiro => &["low", "medium", "high", "xhigh", "max"],
        Harness::Other => &[],
    }
}

/// … Kiro's only choice is whether every tool is trusted.
pub fn modes(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::ClaudeCode => MODES,
        Harness::Codex => &["read-only", "workspace-write", "danger-full-access"],
        Harness::Antigravity => &["accept-edits", "plan"],
        Harness::Grok => &["default", "acceptEdits", "auto", "dontAsk", "bypassPermissions", "plan"],
        Harness::Kiro => &["default", "trust-all"],
        Harness::Other => &[],
    }
}

pub fn label(agent: Harness) -> &'static str {
    match agent {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::Antigravity => "Antigravity",
        Harness::Grok => "Grok Build",
        Harness::Kiro => "Kiro CLI",
        Harness::Other => "Unknown agent",
    }
}
```

In `capabilities`:

```rust
        // Kiro's /compact summarises the conversation; /model and /effort take a value.
        Harness::Kiro => Capabilities { compact: true, model_switch: true, effort_switch: true, mode_cycle: shift_tab, slash_lines: true, shell_lines: true },
        Harness::Other => Capabilities { compact: false, model_switch: false, effort_switch: false, mode_cycle: None, slash_lines: false, shell_lines: false },
```

In `LaunchOptions::flags`, replace the body:

```rust
    pub fn flags(&self) -> String {
        let (model, effort, mode) = (chosen(&self.model), chosen(&self.effort), chosen(&self.mode));
        let (m_flag, e_flag, mode_flag) = match self.agent {
            Harness::ClaudeCode => ("--model", Some("--effort"), Some("--permission-mode")),
            Harness::Codex => ("-m", Some("-c model_reasoning_effort="), Some("-s")),
            Harness::Antigravity => ("--model", Some("--effort"), Some("--mode")),
            Harness::Grok => ("-m", None, Some("--permission-mode")),
            // Kiro's "trust-all" mode is a bare flag; "default" passes nothing.
            Harness::Kiro => ("--model", Some("--effort"), None),
            Harness::Other => return String::new(),
        };
        let mut out = String::new();
        if let Some(m) = model {
            out.push_str(&format!(" {m_flag} {m}"));
        }
        match (e_flag, effort) {
            (Some(f), Some(e)) if f.ends_with('=') => out.push_str(&format!(" {f}{e}")),
            (Some(f), Some(e)) => out.push_str(&format!(" {f} {e}")),
            _ => {}
        }
        match (mode_flag, mode) {
            (Some(f), Some(m)) => out.push_str(&format!(" {f} {m}")),
            (None, Some("trust-all")) if self.agent == Harness::Kiro => out.push_str(" --trust-all-tools"),
            _ => {}
        }
        out
    }
```

In `oneshot_args` add two arms before the closing brace of the match (Task 5 tests them; the Kiro one is final):

```rust
        Harness::Kiro => {
            let mut a = vec![s("chat"), s("--no-interactive"), s("--trust-tools="), s("--output-format"), s("stream-json")];
            if let Some(m) = model {
                a.extend([s("--model"), s(m)]);
            }
            a.extend([s("--"), folded]);
            a
        }
        Harness::Other => vec![],
```

In `final_text` add (Task 5 tests it):

```rust
        Harness::Kiro => {
            let mut last_type = String::new();
            for line in stdout.lines() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                let kind = v["type"].as_str().unwrap_or("");
                last_type = kind.to_string();
                if kind == "runFinished" {
                    let status = v["data"]["status"].as_str().unwrap_or("");
                    if status != "success" {
                        return Err(format!("kiro-cli failed: {status} {}", clip(&v["data"].to_string())));
                    }
                    return Ok(v["data"]["finalText"].as_str().unwrap_or("").to_string());
                }
            }
            Err(format!("kiro-cli did not finish its run (last event: {})", if last_type.is_empty() { "none" } else { &last_type }))
        }
        Harness::Other => Err("unknown agent".into()),
```

In `session_command`, replace `binary_name(opts.agent)` in the `format!` with a program that includes Kiro's subcommand:

```rust
    let program = match opts.agent {
        Harness::Kiro => "kiro-cli chat",
        agent => binary_name(agent),
    };
```

and use `{program}` where `binary_name(opts.agent)` was.

`core/src/agents.rs`:

```rust
fn listing_args(agent: Harness) -> &'static [&'static str] {
    match agent {
        Harness::Codex => &["debug", "models"],
        Harness::Antigravity | Harness::Grok => &["models"],
        Harness::Kiro => &["chat", "--list-models", "-f", "json"],
        Harness::ClaudeCode | Harness::Other => &[],
    }
}
```

(`info_for`'s match already has a `_ => vec![]` arm; Task 4 adds Kiro's parser. `build`'s loop stays `[Codex, Antigravity, Grok]` until Task 4.)

`core/src/resume.rs`: add `pub kiro: PathBuf` to `AgentDirs`; in `list_sessions` add `Harness::Kiro | Harness::Other => vec![]` (Task 6 replaces the Kiro arm); in `resume_command` add:

```rust
        Harness::Kiro => format!("cd {d} && kiro-cli chat --resume-id {i}"),
        Harness::Other => format!("cd {d} && echo 'Maya does not know that agent.'"),
```

Update every `AgentDirs { … }` literal in `resume.rs` tests (`dirs_in`, and the Claude test at `:240`) and `store.rs:290-292` with `kiro: t.join("kiro")` / `kiro: self.kiro_dir.clone()` (the store field arrives in Task 3; until then use `PathBuf::new()` there).

`core/src/foreign.rs`: in `discover` and `discover_from_files` change `Harness::ClaudeCode | Harness::Grok => {}` to `Harness::ClaudeCode | Harness::Grok | Harness::Kiro | Harness::Other => {}`; in `tail_for` and `turns_for` add `Harness::Kiro | Harness::Other =>` arms returning `ForeignTail::default()` and `vec![]` (Task 3 replaces the Kiro arms).

- [ ] **Step 5: Run the whole workspace**

Run: `cargo test --workspace`
Expected: PASS. If `cli/src/args.rs` or `cli/src/commands.rs` fail to compile on a non-exhaustive match, add `Harness::Kiro | Harness::Other` to that match with the same behaviour as Grok's arm; Task 7 gives them their final copy.

- [ ] **Step 6: Commit**

```bash
git add core/src/model.rs core/src/launch.rs core/src/agents.rs core/src/resume.rs core/src/foreign.rs core/src/net/protocol.rs core/fixtures/kiro src/assets/icons/kiro.png
git commit -m "feat(core): Kiro CLI harness, and an Other fallback for agents this build does not know"
```

---

### Task 2: `core/src/kiro.rs`: locks, session files, markers and the event log

**Files:**
- Create: `core/src/kiro.rs`
- Modify: `core/src/lib.rs:12` (add `pub mod kiro;` after `grok`)
- Modify: `src-tauri/src/lib.rs:1` (add `kiro` to the `pub use maya_core::{…}` list)

**Interfaces:**
- Produces:
  - `pub struct Lock { pub session_id: String, pub pid: i32 }` and `pub fn locks(sessions_dir: &Path) -> Vec<Lock>`
  - `pub struct Meta { pub session_id: String, pub cwd: String, pub title: Option<String>, pub updated_ms: u64, pub context_percent: Option<f64>, pub context_window: Option<u64> }` and `pub fn session_meta(json: &str) -> Option<Meta>`
  - `pub struct Marker { pub pid: i32, pub started_ms: u64, pub alive_ms: u64 }`, `pub fn markers(run_dir: &Path) -> Vec<Marker>`, `pub const MARKER_STALE_MS: u64 = 120_000`, `pub fn working_marker(markers: &[Marker], tui_pid: i32, now_ms: u64) -> Option<Marker>`
  - `pub fn parse_events(text: &str) -> ForeignTail`, `pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn>`, `pub fn first_prompt(text: &str) -> Option<String>`
  - `pub fn apply_live(tail: &mut ForeignTail, run_dir: &Path, tui_pid: i32, events_path: &Path, now_ms: u64)`

- [ ] **Step 1: Write the failing tests**

Create `core/src/kiro.rs` with only the tests module for now:

```rust
//! Kiro CLI sessions: `~/.kiro/sessions/cli/<id>.lock` names the agent
//! process of a live session, `<id>.json` carries its folder, title and
//! context usage, `<id>.jsonl` is the event log, and Kiro's run folder keeps
//! one `turn-markers/<tui pid>-<ms>.json` for each turn in progress.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro").join(name)).unwrap()
    }

    #[test]
    fn locks_name_the_agent_pid_of_each_live_session() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("efcba1da-5c0b-4b39-96f9-9a4740ead331.lock"), fixture("lock.json")).unwrap();
        std::fs::write(t.path().join("bad.lock"), "not json").unwrap();
        std::fs::write(t.path().join("other.json"), "{}").unwrap();
        let l = locks(t.path());
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].session_id.as_str(), l[0].pid), ("efcba1da-5c0b-4b39-96f9-9a4740ead331", 95508));
        assert!(locks(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn session_meta_reads_folder_title_time_and_context() {
        let m = session_meta(&fixture("session.json")).unwrap();
        assert_eq!(m.session_id, "efcba1da-5c0b-4b39-96f9-9a4740ead331");
        assert_eq!(m.cwd, "/Users/tiagocorreia");
        assert_eq!(m.title.as_deref(), Some("run ls"));
        assert_eq!(m.updated_ms, 1_791_030_054_892);
        assert_eq!(m.context_window, Some(200_000));
        assert!((m.context_percent.unwrap() - 1.2537).abs() < 0.001);
        let fresh = session_meta(r#"{"session_id":"x","cwd":"/p","created_at":"2026-10-03T12:13:30.596448Z","updated_at":"2026-10-03T12:13:30.596448Z","title":null,"session_state":{"rts_model_state":{"model_info":null,"context_usage_percentage":null}}}"#).unwrap();
        assert_eq!(fresh.title, None);
        assert_eq!(fresh.context_percent, None);
        assert!(session_meta("nope").is_none());
    }

    #[test]
    fn markers_mark_a_turn_in_progress_for_their_tui_pid() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("turn-markers")).unwrap();
        std::fs::write(t.path().join("turn-markers/95441-1791030218653.json"), fixture("marker.json")).unwrap();
        let m = markers(t.path());
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].pid, m[0].started_ms, m[0].alive_ms), (95441, 1_791_030_218_653, 1_791_030_308_655));
        let now = m[0].alive_ms + 1_000;
        assert_eq!(working_marker(&m, 95441, now).map(|x| x.started_ms), Some(1_791_030_218_653));
        assert!(working_marker(&m, 95442, now).is_none(), "another TUI's turn");
        assert!(working_marker(&m, 95441, m[0].alive_ms + MARKER_STALE_MS + 1).is_none(), "a heartbeat that stopped is not a turn");
        assert!(markers(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn events_give_the_last_text_the_pending_tool_call_and_the_prompt_time() {
        let text = fixture("events.jsonl");
        let t = parse_events(&text);
        assert!(!t.working, "the event log alone never says working: the marker does");
        assert_eq!(t.awaiting, None);
        assert_eq!(t.last_agent_text.as_deref(), Some("shell: mkdir -p /tmp/kiro-probe && sleep 90"), "a tool call with no result yet is the snippet");
        assert_eq!(t.last_event_ms, 1_791_030_218_000, "the last prompt's timestamp");
        // The first four events are a finished turn: the final text is the snippet.
        let finished: String = text.lines().take(4).map(|l| format!("{l}\n")).collect();
        let f = parse_events(&finished);
        assert!(f.last_agent_text.unwrap().starts_with("```\nApplications"));
        assert_eq!(f.last_event_ms, 1_791_030_044_000);
        let empty = parse_events("");
        assert_eq!(empty.last_agent_text, None);
        assert_eq!(empty.last_event_ms, 0);
    }

    #[test]
    fn turns_are_prompts_text_and_tool_calls() {
        let turns = parse_turns(&fixture("events.jsonl"), 30);
        let got: Vec<(String, String)> = turns.iter().map(|t| (format!("{:?}", t.kind).to_lowercase(), t.text.chars().take(40).collect())).collect();
        assert_eq!(got[0], ("user".to_string(), "run ls".to_string()));
        assert_eq!(got[1], ("tool".to_string(), "shell: ls".to_string()));
        assert!(got[2].0 == "assistant" && got[2].1.starts_with("```\nApplications"));
        assert_eq!(got[3], ("user".to_string(), "run this: mkdir -p /tmp/kiro-probe && sl".to_string()));
        assert_eq!(got[4].0, "tool");
        assert_eq!(turns.len(), 5, "empty assistant text is not a turn");
        assert_eq!(parse_turns(&fixture("events.jsonl"), 2).len(), 2, "the newest turns are kept");
        assert_eq!(first_prompt(&fixture("events.jsonl")).as_deref(), Some("run ls"));
        assert_eq!(first_prompt(""), None);
    }

    #[test]
    fn apply_live_adds_the_marker_the_context_and_the_file_time() {
        let t = tempfile::tempdir().unwrap();
        let run = t.path().join("run");
        std::fs::create_dir_all(run.join("turn-markers")).unwrap();
        std::fs::write(run.join("turn-markers/95441-1791030218653.json"), fixture("marker.json")).unwrap();
        let events = t.path().join("s.jsonl");
        std::fs::write(&events, fixture("events.jsonl")).unwrap();
        std::fs::write(t.path().join("s.json"), fixture("session.json")).unwrap();
        let mut tail = parse_events(&fixture("events.jsonl"));
        apply_live(&mut tail, &run, 95441, &events, 1_791_030_308_655 + 500);
        assert!(tail.working);
        assert_eq!(tail.context_window, Some(200_000));
        assert_eq!(tail.context_used, Some(2_507), "1.2537% of 200000, rounded");
        assert!(tail.last_event_ms >= 1_791_030_218_653, "at least the turn's start: {}", tail.last_event_ms);
        let mut idle = parse_events(&fixture("events.jsonl"));
        apply_live(&mut idle, &run, 95441, &events, 1_791_030_308_655 + MARKER_STALE_MS + 1);
        assert!(!idle.working, "a stale marker is ignored");
        let mut none = parse_events("");
        apply_live(&mut none, Path::new("/nonexistent"), 1, Path::new("/nonexistent/x.jsonl"), 5);
        assert!(!none.working && none.context_window.is_none());
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core kiro::`
Expected: compile errors, nothing defined.

- [ ] **Step 3: Implement**

Above the tests in `core/src/kiro.rs`:

```rust
use crate::foreign::ForeignTail;
use crate::transcript::{Turn, TurnKind};
use serde_json::Value;
use std::path::Path;

/// A live session's lock: the pid of its `kiro-cli-chat acp` agent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lock {
    pub session_id: String,
    pub pid: i32,
}

/// Every `<id>.lock` in the sessions folder with a pid in it.
pub fn locks(sessions_dir: &Path) -> Vec<Lock> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else { return vec![] };
    let mut out: Vec<Lock> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "lock"))
        .filter_map(|e| {
            let id = e.path().file_stem()?.to_str()?.to_string();
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some(Lock { session_id: id, pid: v["pid"].as_i64()? as i32 })
        })
        .collect();
    out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    out
}

/// What `<id>.json` says about a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub updated_ms: u64,
    pub context_percent: Option<f64>,
    pub context_window: Option<u64>,
}

pub fn session_meta(json: &str) -> Option<Meta> {
    let v: Value = serde_json::from_str(json).ok()?;
    let state = &v["session_state"]["rts_model_state"];
    Some(Meta {
        session_id: v["session_id"].as_str()?.to_string(),
        cwd: v["cwd"].as_str()?.to_string(),
        title: v["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
        updated_ms: v["updated_at"].as_str().and_then(crate::codex::ms_of).unwrap_or(0),
        context_percent: state["context_usage_percentage"].as_f64(),
        context_window: state["model_info"]["context_window_tokens"].as_u64(),
    })
}

/// A turn in progress: Kiro writes one per turn, named `<tui pid>-<start ms>.json`,
/// and refreshes `last_alive_at_ms` while the turn runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub pid: i32,
    pub started_ms: u64,
    pub alive_ms: u64,
}

pub fn markers(run_dir: &Path) -> Vec<Marker> {
    let Ok(entries) = std::fs::read_dir(run_dir.join("turn-markers")) else { return vec![] };
    entries
        .flatten()
        .filter_map(|e| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some(Marker { pid: v["pid"].as_i64()? as i32, started_ms: v["turn_started_at_ms"].as_u64()?, alive_ms: v["last_alive_at_ms"].as_u64().unwrap_or(0) })
        })
        .collect()
}

/// A marker whose heartbeat is older than this belongs to a TUI that died
/// mid-turn, or to a turn Kiro forgot to clean up.
pub const MARKER_STALE_MS: u64 = 120_000;

pub fn working_marker(markers: &[Marker], tui_pid: i32, now_ms: u64) -> Option<Marker> {
    markers.iter().copied().filter(|m| m.pid == tui_pid && now_ms.saturating_sub(m.alive_ms) <= MARKER_STALE_MS).max_by_key(|m| m.started_ms)
}

/// `shell: <command>` for the shell tool, else `<tool>: <purpose>`, else the tool name.
fn tool_line(block: &Value) -> String {
    let name = block["name"].as_str().unwrap_or("tool");
    let input = &block["input"];
    let arg = if name == "shell" { input["command"].as_str() } else { None }.or_else(|| input["__tool_use_purpose"].as_str()).or_else(|| input["path"].as_str()).or_else(|| input["command"].as_str());
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name}: {}", crate::state::truncate(a, 120)),
        None => name.to_string(),
    }
}

fn blocks(v: &Value) -> impl Iterator<Item = &Value> {
    v["data"]["content"].as_array().into_iter().flatten()
}

/// The turn state the event log alone can give: the last assistant text,
/// or the tool call still without a result, and the last prompt's time.
/// Working comes from the marker (`apply_live`), never from here.
pub fn parse_events(text: &str) -> ForeignTail {
    let mut t = ForeignTail::default();
    let mut last_text: Option<String> = None;
    let mut pending_tool: Option<String> = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["kind"].as_str().unwrap_or("") {
            "Prompt" => {
                pending_tool = None;
                if let Some(s) = v["data"]["meta"]["timestamp"].as_u64() {
                    t.last_event_ms = t.last_event_ms.max(s * 1000);
                }
            }
            "AssistantMessage" => {
                for b in blocks(&v) {
                    match b["kind"].as_str() {
                        Some("text") => {
                            if let Some(s) = b["data"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                last_text = Some(s.to_string());
                            }
                        }
                        Some("toolUse") => pending_tool = Some(tool_line(&b["data"])),
                        _ => {}
                    }
                }
            }
            "ToolResults" => pending_tool = None,
            _ => {}
        }
    }
    t.last_agent_text = pending_tool.or(last_text);
    t
}

/// Prompts, assistant text and tool calls, newest last.
pub fn parse_turns(text: &str, max_turns: usize) -> Vec<Turn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["kind"].as_str().unwrap_or("") {
            "Prompt" => {
                let s: String = blocks(&v).filter(|b| b["kind"] == "text").filter_map(|b| b["data"].as_str()).collect::<Vec<_>>().join("\n");
                if !s.trim().is_empty() {
                    out.push(Turn { kind: TurnKind::User, text: s.trim().to_string() });
                }
            }
            "AssistantMessage" => {
                for b in blocks(&v) {
                    match b["kind"].as_str() {
                        Some("text") => {
                            if let Some(s) = b["data"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                out.push(Turn { kind: TurnKind::Assistant, text: s.to_string() });
                            }
                        }
                        Some("toolUse") => out.push(Turn { kind: TurnKind::Tool, text: tool_line(&b["data"]) }),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if out.len() > max_turns {
        out.drain(..out.len() - max_turns);
    }
    out
}

/// The first line of the first prompt, as a title.
pub fn first_prompt(text: &str) -> Option<String> {
    parse_turns(text, usize::MAX).into_iter().find(|t| t.kind == TurnKind::User).and_then(|t| t.text.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| crate::state::truncate(l, 120)))
}

fn mtime_ms(path: &Path) -> Option<u64> {
    Some(std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// What the event log cannot say: Working from the TUI's turn marker,
/// context usage from the session file, and the time of the last write.
pub fn apply_live(tail: &mut ForeignTail, run_dir: &Path, tui_pid: i32, events_path: &Path, now_ms: u64) {
    if let Some(m) = working_marker(&markers(run_dir), tui_pid, now_ms) {
        tail.working = true;
        tail.awaiting = None;
        tail.last_event_ms = tail.last_event_ms.max(m.started_ms);
    }
    if let Some(meta) = std::fs::read_to_string(events_path.with_extension("json")).ok().and_then(|s| session_meta(&s)) {
        tail.last_event_ms = tail.last_event_ms.max(meta.updated_ms);
        if let (Some(p), Some(w)) = (meta.context_percent, meta.context_window) {
            tail.context_window = Some(w);
            tail.context_used = Some((p / 100.0 * w as f64).round() as u64);
        }
    }
    if let Some(ms) = mtime_ms(events_path) {
        tail.last_event_ms = tail.last_event_ms.max(ms);
    }
}
```

Note for `apply_live_adds_the_marker…`: the test writes the events file just before, so its mtime is "now" in real time, far after the fixture's timestamps; the assertion only checks `>=`, which holds.

Add `pub mod kiro;` to `core/src/lib.rs` after `pub mod grok;` and `kiro` to the re-export list in `src-tauri/src/lib.rs:1`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core kiro::`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/kiro.rs core/src/lib.rs src-tauri/src/lib.rs
git commit -m "feat(core): read Kiro's locks, session files, turn markers and event log"
```

---

### Task 3: Discovery: the process tree, `kiro_sessions`, and the store

**Files:**
- Modify: `core/src/foreign.rs` (new `Proc`, `parse_ps_tree`, `list_process_tree`, `tty_above`, `kiro_sessions`; `tail_for` and `turns_for` Kiro arms)
- Modify: `core/src/win_process.rs` (new `tree()`)
- Modify: `core/src/store.rs:15-50, 95-120, 160-216, 231-253, 290-292, 385-393`

**Interfaces:**
- Consumes: `kiro::{locks, session_meta, parse_events, parse_turns, apply_live}` from Task 2.
- Produces:
  - `pub struct Proc { pub pid: i32, pub ppid: i32, pub tty: Option<String> }` in `foreign.rs`
  - `pub fn parse_ps_tree(ps: &str) -> Vec<Proc>`, `pub fn list_process_tree() -> Vec<Proc>`, `pub fn tty_above(procs: &[Proc], pid: i32) -> Option<String>`
  - `pub fn kiro_sessions(sessions_dir: &Path, procs: &[Proc]) -> Vec<ForeignSession>` where `ForeignSession.pid` is the **TUI** pid (the lock pid's parent), `tty` is the first tty up the chain, `transcript_path` is `<sessions>/<id>.jsonl`.
  - `Store::with_kiro(sessions_dir: PathBuf, run_dir: PathBuf, procs: Vec<Proc>) -> Self` (test support)
  - Windows: `win_process::tree() -> Vec<(u32, u32)>` (pid, parent pid)

- [ ] **Step 1: Write the failing tests**

In `core/src/foreign.rs` tests:

```rust
    #[test]
    fn a_process_tree_gives_the_first_tty_above_a_pid() {
        let ps = "95245 93903 ttys010\n95295 95245 ttys010\n95441 95295 ttys010\n95508 95441 ??\n  777     1 ?\n";
        let procs = parse_ps_tree(ps);
        assert_eq!(procs.len(), 5);
        assert_eq!(procs[3], Proc { pid: 95508, ppid: 95441, tty: None });
        assert_eq!(tty_above(&procs, 95508).as_deref(), Some("/dev/ttys010"), "the agent has no tty; its TUI has");
        assert_eq!(tty_above(&procs, 95441).as_deref(), Some("/dev/ttys010"));
        assert_eq!(tty_above(&procs, 777), None);
        assert_eq!(tty_above(&procs, 1), None, "an unknown pid has no tty");
    }

    #[test]
    fn kiro_sessions_come_from_live_locks_with_a_terminal_above_them() {
        let t = tempfile::tempdir().unwrap();
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro");
        let id = "efcba1da-5c0b-4b39-96f9-9a4740ead331";
        std::fs::copy(fixtures.join("lock.json"), t.path().join(format!("{id}.lock"))).unwrap();
        std::fs::copy(fixtures.join("session.json"), t.path().join(format!("{id}.json"))).unwrap();
        std::fs::copy(fixtures.join("events.jsonl"), t.path().join(format!("{id}.jsonl"))).unwrap();
        // A headless run: alive, but nothing above it has a tty.
        std::fs::write(t.path().join("headless.lock"), r#"{"pid":600}"#).unwrap();
        std::fs::write(t.path().join("headless.json"), r#"{"session_id":"headless","cwd":"/p","updated_at":"2026-10-03T12:00:00Z","title":null}"#).unwrap();
        // A lock whose pid is gone.
        std::fs::write(t.path().join("dead.lock"), r#"{"pid":700}"#).unwrap();
        let procs = parse_ps_tree("95245 93903 ttys010\n95295 95245 ttys010\n95441 95295 ttys010\n95508 95441 ??\n600 599 ??\n599 1 ??\n");
        let found = kiro_sessions(t.path(), &procs);
        assert_eq!(found.len(), 1, "{found:?}");
        let s = &found[0];
        assert_eq!((s.harness, s.pid, s.tty.as_deref(), s.session_id.as_str(), s.cwd.as_str(), s.name.as_str()), (Harness::Kiro, 95441, Some("/dev/ttys010"), id, "/Users/tiagocorreia", "run ls"));
        assert_eq!(s.transcript_path, t.path().join(format!("{id}.jsonl")));
        let tail = tail_for(s);
        assert_eq!(tail.last_agent_text.as_deref(), Some("shell: mkdir -p /tmp/kiro-probe && sleep 90"));
        assert_eq!(turns_for(s, 10).len(), 5);
    }

    #[test]
    fn an_untitled_kiro_session_is_named_after_its_tui_pid() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("n1.lock"), r#"{"pid":50}"#).unwrap();
        std::fs::write(t.path().join("n1.json"), r#"{"session_id":"n1","cwd":"/p","updated_at":"2026-10-03T12:00:00Z","title":null}"#).unwrap();
        let procs = parse_ps_tree("40 1 ttys001\n50 40 ??\n");
        let found = kiro_sessions(t.path(), &procs);
        assert_eq!(found[0].name, "kiro-40");
        assert_eq!(found[0].pid, 40);
    }
```

In `core/src/store.rs` tests:

```rust
    #[test]
    fn kiro_sessions_come_from_their_locks_and_show_working_from_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let (sessions, run) = (dir.path().join("kiro"), dir.path().join("kiro-run"));
        std::fs::create_dir_all(run.join("turn-markers")).unwrap();
        std::fs::create_dir_all(&sessions).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro");
        let id = "efcba1da-5c0b-4b39-96f9-9a4740ead331";
        for (from, to) in [("lock.json", "lock"), ("session.json", "json"), ("events.jsonl", "jsonl")] {
            std::fs::copy(fixtures.join(from), sessions.join(format!("{id}.{to}"))).unwrap();
        }
        std::fs::copy(fixtures.join("marker.json"), run.join("turn-markers/95441-1791030218653.json")).unwrap();
        let procs = foreign::parse_ps_tree("95245 93903 ttys010\n95295 95245 ttys010\n95441 95295 ttys010\n95508 95441 ??\n");
        let mut store = Store::new(dir.path().join("claude")).with_alive(|_| true).with_kiro(sessions, run, procs);
        let now = 1_791_030_308_655 + 500;
        let cards = store.refresh(now);
        let k = cards.iter().find(|c| c.harness == Harness::Kiro).expect("a Kiro card");
        assert_eq!((k.pid, k.name.as_str(), k.cwd.as_str(), k.state), (95441, "run ls", "/Users/tiagocorreia", State::Working));
        assert_eq!(k.snippet, "shell: mkdir -p /tmp/kiro-probe && sleep 90");
        assert_eq!(k.context.as_ref().map(|c| c.percent), Some(1));
        assert!(store.live_session_ids().contains(&id.to_string()));
        // The marker's heartbeat stops: the turn is over, the card is Completed.
        let later = now + crate::kiro::MARKER_STALE_MS + 1;
        let k = store.refresh(later).into_iter().find(|c| c.harness == Harness::Kiro).unwrap();
        assert_eq!(k.state, State::Completed);
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core -- kiro_sessions a_process_tree an_untitled_kiro`
Expected: compile errors (`Proc`, `parse_ps_tree`, `with_kiro` undefined).

- [ ] **Step 3: Implement the process tree and `kiro_sessions` in `foreign.rs`**

After `list_tui_processes`:

```rust
/// A process with its parent and terminal, from `ps -axo pid=,ppid=,tty=`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub tty: Option<String>,
}

pub fn parse_ps_tree(ps: &str) -> Vec<Proc> {
    ps.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let (pid, ppid, tty) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?);
            Some(Proc { pid, ppid, tty: crate::tty::tty_from_ps(tty) })
        })
        .collect()
}

#[cfg(unix)]
pub fn list_process_tree() -> Vec<Proc> {
    crate::command("ps")
        .args(["-axo", "pid=,ppid=,tty="])
        .stdin(Stdio::null())
        .output()
        .map(|o| parse_ps_tree(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// On Windows every process reaches its console by pid, so each one "has a tty".
#[cfg(windows)]
pub fn list_process_tree() -> Vec<Proc> {
    crate::win_process::tree().into_iter().map(|(pid, ppid)| Proc { pid: pid as i32, ppid: ppid as i32, tty: Some(crate::win_console::console_key(pid as i32)) }).collect()
}

/// The tty of `pid` or of the nearest ancestor that has one, up to six levels.
pub fn tty_above(procs: &[Proc], pid: i32) -> Option<String> {
    let mut cur = pid;
    for _ in 0..6 {
        let p = procs.iter().find(|p| p.pid == cur)?;
        if let Some(t) = &p.tty {
            return Some(t.clone());
        }
        cur = p.ppid;
    }
    None
}

/// Live Kiro sessions: each lock names its agent process; the session's TUI
/// is that process's parent, and the terminal is the first one above it.
/// A lock with no live pid, or none with a terminal above it (a headless
/// run), is not a session.
pub fn kiro_sessions(sessions_dir: &std::path::Path, procs: &[Proc]) -> Vec<ForeignSession> {
    crate::kiro::locks(sessions_dir)
        .into_iter()
        .filter_map(|lock| {
            let agent = procs.iter().find(|p| p.pid == lock.pid)?;
            let tui = agent.ppid;
            let tty = tty_above(procs, tui)?;
            let meta = crate::kiro::session_meta(&std::fs::read_to_string(sessions_dir.join(format!("{}.json", lock.session_id))).ok()?)?;
            let name = meta.title.unwrap_or_else(|| format!("kiro-{tui}"));
            Some(ForeignSession { harness: Harness::Kiro, pid: tui, tty: Some(tty), session_id: lock.session_id.clone(), cwd: meta.cwd, name, transcript_path: sessions_dir.join(format!("{}.jsonl", lock.session_id)) })
        })
        .collect()
}
```

In `tail_for` replace the Kiro arm with `Harness::Kiro => crate::kiro::parse_events(&tail_of(&s.transcript_path, crate::transcript::TAIL_BYTES)),` and in `turns_for` with `Harness::Kiro => crate::kiro::parse_turns(&tail_of(&s.transcript_path, big), max_turns),` (keep `Harness::Other` on its own arm).

In `core/src/win_process.rs` add, after `list`:

```rust
/// `(pid, parent pid)` of every running process.
pub fn tree() -> Vec<(u32, u32)> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    // SAFETY: as in `list`: a snapshot walked with a correctly sized entry, then closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            out.push((e.th32ProcessID, e.th32ParentProcessID));
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}
```

- [ ] **Step 4: Wire the store**

In `core/src/store.rs`:

- Fields, after `grok_dir`: `kiro_dir: PathBuf,` and `kiro_run_dir: PathBuf,`; after `processes`: `/// The process tree (pid, parent, tty), for agents whose session files name a pid with no terminal of its own; replaceable in tests.` `process_tree: Box<dyn Fn() -> Vec<foreign::Proc> + Send>,`.
- `Store::new`: `kiro_dir: home.join(".kiro/sessions/cli"),` and `kiro_run_dir: dirs::data_local_dir().unwrap_or_else(|| home.join(".local/share")).join("kiro-cli/run"),` and `process_tree: Box::new(foreign::list_process_tree),`. (`dirs::data_local_dir` is `~/Library/Application Support` on macOS, `~/.local/share` on Linux and `%LOCALAPPDATA%` on Windows.)
- `with_alive`: add `self.process_tree = Box::new(Vec::new);` and `self.kiro_dir = PathBuf::from("/nonexistent/kiro");`.
- `with_foreign`: add `self.kiro_dir = agy_dir.join("no-kiro");`.
- New builder:

```rust
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_kiro(mut self, sessions_dir: PathBuf, run_dir: PathBuf, procs: Vec<foreign::Proc>) -> Self {
        self.kiro_dir = sessions_dir;
        self.kiro_run_dir = run_dir;
        self.process_tree = Box::new(move || procs.clone());
        self
    }
```

- `refresh_foreign`, after the Grok block and before `self.refresh_names()`:

```rust
        // Kiro's locks name a pid too, and its title changes with `/rename`.
        let kiro = foreign::kiro_sessions(&self.kiro_dir, &(self.process_tree)());
        let kiro_pids: std::collections::HashSet<i32> = kiro.iter().map(|s| s.pid).collect();
        self.foreign.retain(|pid, s| s.harness != Harness::Kiro || kiro_pids.contains(pid));
        for s in kiro {
            self.foreign.entry(s.pid).and_modify(|e| e.name = s.name.clone()).or_insert(s);
        }
```

- `agent_dirs`: add `kiro: self.kiro_dir.clone()`.
- `refresh`, in the foreign loop after `let mut tail = foreign::tail_for(s);`:

```rust
            if s.harness == Harness::Kiro {
                crate::kiro::apply_live(&mut tail, &self.kiro_run_dir, s.pid, &s.transcript_path, now_ms);
            }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace`
Expected: PASS, including the three foreign tests and the store test. The Codex and Antigravity store tests still pass: `with_alive` and `with_foreign` point Kiro at folders that do not exist.

- [ ] **Step 6: Commit**

```bash
git add core/src/foreign.rs core/src/win_process.rs core/src/store.rs
git commit -m "feat(core): Kiro sessions on the board, from their locks and turn markers"
```

---

### Task 4: The model listing

**Files:**
- Modify: `core/src/agents.rs:1-4, 91-149` and its tests

**Interfaces:**
- Produces: `agents::parse_kiro(json: &str) -> Vec<ModelInfo>`; `build` lists Kiro when `kiro-cli` is found, running `kiro-cli chat --list-models -f json`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn kiro_models_are_the_listed_ids_with_their_names() {
        let m = parse_kiro(&fixture("kiro/models.json"));
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["auto", "claude-sonnet-4.5", "claude-sonnet-4", "claude-haiku-4.5", "deepseek-3.2", "minimax-m2.5", "minimax-m2.1", "glm-5", "qwen3-coder-next"]);
        assert_eq!(m[1].label, "claude-sonnet-4.5");
        assert!(m.iter().all(|m| m.efforts.is_empty()), "Kiro's efforts are one list for every model");
        assert!(parse_kiro("not json").is_empty());
        assert!(parse_kiro(r#"{"models":[{"model_id":"a b","model_name":"x"}]}"#).is_empty(), "an id with a space never reaches a shell line");
    }

    #[test]
    fn kiro_offers_its_efforts_and_modes_with_or_without_a_listing() {
        let info = info_for(Harness::Kiro, Some(&fixture("kiro/models.json")));
        assert_eq!(info.models.len(), 9);
        assert_eq!(info.efforts, vec!["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(info.modes, vec!["default", "trust-all"]);
        assert!(info_for(Harness::Kiro, None).models.is_empty());
    }
```

And in `build_lists_claude_then_each_agent_it_finds`, make `found` also match `"kiro-cli"`, add the arm `("/bin/kiro-cli", ["chat", "--list-models", "-f", "json"]) => Some(fixture("kiro/models.json")),` to `run`, and assert the list is `[ClaudeCode, Codex, Grok, Kiro]` with `list[3].models.len() == 9`. In `info_without_a_listing_offers_default_only`, add `Harness::Kiro` to the loop.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core agents::`
Expected: FAIL: `parse_kiro` undefined.

- [ ] **Step 3: Implement**

```rust
/// Models from `kiro-cli chat --list-models -f json`: `models[].model_id`
/// with `model_name` as the label.
pub fn parse_kiro(json: &str) -> Vec<ModelInfo> {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["model_id"].as_str().filter(|id| launch::plain_model_id(id))?;
            Some(ModelInfo { id: id.to_string(), label: m["model_name"].as_str().unwrap_or(id).to_string(), efforts: vec![] })
        })
        .collect()
}
```

In `info_for` add `(Harness::Kiro, Some(t)) => parse_kiro(t),`; in `build` change the loop to `for agent in [Harness::Codex, Harness::Antigravity, Harness::Grok, Harness::Kiro]`. Update the module doc's first lines to name `kiro-cli chat --list-models -f json`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core agents::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/agents.rs
git commit -m "feat(core): list Kiro's models for New session and Settings"
```

---

### Task 5: Launching, the one-shot, and the interpreter

**Files:**
- Modify: `core/src/launch.rs` tests (`each_agent_runs_its_binary_with_the_prompt`, `each_agent_takes_a_dash_prompt_as_the_prompt`, `oneshot_args_per_agent`, `final_text_reads_each_agents_envelope`, `final_text_reports_failures_and_garbage`)
- Modify: `core/src/interpreter.rs` tests (`parses_the_reply_inside_each_agents_output`)
- Modify: `core/src/actions.rs` tests (a Kiro controls test beside `controls_follow_the_agents_capabilities`)

**Interfaces:**
- Consumes: `oneshot_args`, `final_text`, `session_command` Kiro arms from Task 1.

- [ ] **Step 1: Write the failing tests**

In `launch.rs`:

```rust
        // in each_agent_runs_its_binary_with_the_prompt
        assert!(line(&opts(Harness::Kiro), None).ends_with("&& kiro-cli chat -- \"$p\""));
        let trusted = LaunchOptions { agent: Harness::Kiro, model: Some("auto".into()), effort: Some("high".into()), mode: Some("trust-all".into()), ..Default::default() };
        assert!(line(&trusted, None).ends_with("&& kiro-cli chat --model auto --effort high --trust-all-tools -- \"$p\""));
```

Add `Harness::Kiro` to the loop in `each_agent_takes_a_dash_prompt_as_the_prompt`.

```rust
        // in oneshot_args_per_agent
        let r = oneshot_args(Harness::Kiro, Some("auto"), Some("SYS"), "USER");
        assert_eq!(&r[..2], ["chat", "--no-interactive"]);
        assert!(r.contains(&"--trust-tools=".to_string()) && pair(&r, "--output-format", "stream-json") && pair(&r, "--model", "auto"));
        assert_eq!(r[r.len() - 2], "--");
        assert_eq!(r.last().unwrap(), "SYS\n\nUSER", "no system flag: the system text leads the prompt");
        assert!(!oneshot_args(Harness::Kiro, None, None, "U").iter().any(|x| x == "--model"));
        assert!(oneshot_args(Harness::Other, None, None, "U").is_empty());
```

```rust
        // in final_text_reads_each_agents_envelope
        assert_eq!(final_text(Harness::Kiro, &oneshot_fixture("kiro.jsonl")).unwrap(), "pong");
        // in final_text_reports_failures_and_garbage
        assert!(final_text(Harness::Kiro, "{\"type\":\"runStarted\",\"data\":{}}\n{\"type\":\"metadata\",\"data\":{}}\n").unwrap_err().contains("last event: metadata"));
        assert!(final_text(Harness::Kiro, "{\"type\":\"runFinished\",\"data\":{\"status\":\"error\",\"message\":\"Not logged in\"}}\n").unwrap_err().contains("Not logged in"));
        assert!(final_text(Harness::Kiro, "").unwrap_err().contains("last event: none"));
        assert!(final_text(Harness::Other, "{}").unwrap_err().contains("unknown agent"));
```

Copy the fixture: `cp core/fixtures/kiro/oneshot.jsonl core/fixtures/oneshot/kiro.jsonl` (keep both; the `oneshot/` folder is where `oneshot_fixture` looks).

In `interpreter.rs` `parses_the_reply_inside_each_agents_output`, add after the loop:

```rust
        // Kiro's final text is read from its last event; the reply inside is parsed as for every agent.
        let kiro = "{\"type\":\"runFinished\",\"data\":{\"status\":\"success\",\"finalText\":\"{\\\"say\\\":\\\"hi\\\",\\\"action\\\":null,\\\"confirm\\\":false}\"}}\n";
        let r = parse_reply(crate::model::Harness::Kiro, kiro).unwrap();
        assert_eq!((r.say.as_str(), r.action.is_none(), r.confirm), ("hi", true, false));
        assert!(parse_reply(crate::model::Harness::Kiro, &fixture("kiro.jsonl")).unwrap_err().contains("no JSON reply"), "a plain 'pong' has no reply in it");
```

In `actions.rs`, add a test next to `controls_follow_the_agents_capabilities`:

```rust
    #[test]
    fn a_kiro_session_takes_compact_model_effort_and_shell_lines() {
        let (t, store, _path) = store_with_codex(&format!("{TURN_STARTED}\n{TURN_COMPLETE}\n"));
        let sessions = t.path().join("kiro");
        std::fs::create_dir_all(&sessions).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kiro");
        let id = "efcba1da-5c0b-4b39-96f9-9a4740ead331";
        for (from, to) in [("lock.json", "lock"), ("session.json", "json")] {
            std::fs::copy(fixtures.join(from), sessions.join(format!("{id}.{to}"))).unwrap();
        }
        // A finished turn, no marker: the card is Completed, so free.
        let finished: String = std::fs::read_to_string(fixtures.join("events.jsonl")).unwrap().lines().take(4).map(|l| format!("{l}\n")).collect();
        std::fs::write(sessions.join(format!("{id}.jsonl")), finished).unwrap();
        let procs = crate::foreign::parse_ps_tree("95245 93903 ttys010\n95295 95245 ttys010\n95441 95295 ttys010\n95508 95441 ??\n");
        let store = Mutex::new(store.into_inner().unwrap().with_kiro(sessions, t.path().join("no-run"), procs));
        let info = crate::agents::AgentInfo { harness: Harness::Kiro, models: vec![crate::agents::ModelInfo { id: "auto".into(), label: "auto".into(), efforts: vec![] }], efforts: vec!["low".into(), "high".into()], modes: vec![] };
        let store = with_agents(store, vec![agents::claude(), info]);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        store.lock().unwrap().refresh(now_ms());
        compact_session(&l, id).unwrap();
        set_session_option(&l, id, "model", "auto").unwrap();
        set_session_option(&l, id, "effort", "high").unwrap();
        send_slash_command(&l, id, "!ls").unwrap();
        let typed: Vec<String> = fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { tty, text } if tty == "/dev/ttys010" => Some(text.clone()), _ => None }).collect();
        assert_eq!(typed, vec!["/compact", "/model auto", "/effort high", "!ls"]);
    }
```

(`store_with_codex`, `with_agents`, `FakeTerminal`, `Call` and `now_ms` are the helpers the neighbouring tests use in `actions.rs`; `compact_session`'s refusal text comes from `type_into_session`'s `"{} has no {control}."` with control `/compact`. If `compact_session` names its control differently, use the text the existing Codex test shows.)

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core -- each_agent_runs oneshot_args_per_agent final_text parses_the_reply a_kiro_session_takes_compact`
Expected: the launch and interpreter tests PASS already if Task 1's arms were typed exactly; the actions test FAILS until Step 3's check. Whatever fails, fix it in Step 3.

- [ ] **Step 3: Implement what is missing**

Most of this task's code landed in Task 1. Check:
- `session_command` renders `kiro-cli chat` then the flags then ` -- "$p"`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/launch.rs core/src/interpreter.rs core/src/actions.rs core/fixtures/oneshot/kiro.jsonl
git commit -m "feat(core): start, resume-less launch and one-shot runs for Kiro, with its controls"
```

---

### Task 6: Resume

**Files:**
- Modify: `core/src/resume.rs:153-200` and its tests

**Interfaces:**
- Consumes: `kiro::{session_meta, first_prompt}`.
- Produces: `resume::list_sessions(Harness::Kiro, dirs, dir, running)` reading `dirs.kiro` (the sessions folder); `resume_command(Harness::Kiro, …)` is `cd <dir> && kiro-cli chat --resume-id '<id>'`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn kiro_sessions_of_a_folder_come_from_their_session_files_newest_first() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs_in(t.path());
        std::fs::create_dir_all(&d.kiro).unwrap();
        let meta = |id: &str, cwd: &str, title: &str, at: &str| format!("{{\"session_id\":\"{id}\",\"cwd\":\"{cwd}\",\"created_at\":\"{at}\",\"updated_at\":\"{at}\",\"title\":{title},\"session_state\":{{}}}}");
        std::fs::write(d.kiro.join("k-old.json"), meta("k-old", "/p/maya", "\"Rename test\"", "2026-10-02T16:57:07.186467Z")).unwrap();
        std::fs::write(d.kiro.join("k-new.json"), meta("k-new", "/p/maya", "null", "2026-10-03T12:20:54.892589Z")).unwrap();
        std::fs::write(d.kiro.join("k-new.jsonl"), "{\"version\":\"v1\",\"kind\":\"Prompt\",\"data\":{\"content\":[{\"kind\":\"text\",\"data\":\"run ls\\nplease\"}],\"meta\":{\"timestamp\":1791030044}}}\n").unwrap();
        std::fs::write(d.kiro.join("k-bare.json"), meta("k-bare", "/p/maya", "null", "2026-10-01T10:00:00Z")).unwrap();
        std::fs::write(d.kiro.join("k-else.json"), meta("k-else", "/p/other", "\"Elsewhere\"", "2026-10-03T13:00:00Z")).unwrap();
        std::fs::write(d.kiro.join("k-new.lock"), "{\"pid\":1}").unwrap();
        let list = list_sessions(Harness::Kiro, &d, "/p/maya", &["k-new".to_string()]);
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["k-new", "k-old", "k-bare"]);
        assert_eq!(list[0].title, "run ls", "no title: the first prompt's first line");
        assert!(list[0].running);
        assert_eq!(list[1].title, "Rename test");
        assert_eq!(list[2].title, "k-bare", "no title and no prompt: the id");
        assert!(list_sessions(Harness::Kiro, &d, "/p/nowhere", &[]).is_empty());
    }
```

Add to `resume_commands_per_agent_quote_folder_and_id`:

```rust
        assert_eq!(resume_command(Harness::Kiro, dir, "abc-123"), "cd '/Users/x/dev/it'\\''s' && kiro-cli chat --resume-id 'abc-123'");
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya-core resume::`
Expected: the listing test FAILS (empty list); the command test passes from Task 1.

- [ ] **Step 3: Implement**

```rust
/// Kiro's sessions whose `cwd` is `dir`, from `<id>.json` files, named by
/// their title, else their first prompt, else their id.
fn kiro_sessions(kiro_dir: &Path, dir: &str) -> Vec<ResumableSession> {
    let Ok(entries) = std::fs::read_dir(kiro_dir) else { return vec![] };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let path = e.path();
            let meta = crate::kiro::session_meta(&std::fs::read_to_string(&path).ok()?)?;
            if meta.cwd != dir {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let last_active_ms = if meta.updated_ms > 0 { meta.updated_ms } else { mtime_ms(&path)? };
            let title = meta.title.or_else(|| crate::kiro::first_prompt(&head(&path.with_extension("jsonl"), HEAD_BYTES))).unwrap_or_else(|| id.clone());
            Some(ResumableSession { id, title, last_active_ms, running: false })
        })
        .collect()
}
```

In `list_sessions` replace the Kiro arm with `Harness::Kiro => kiro_sessions(&dirs.kiro, dir),` keeping `Harness::Other => vec![]`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core resume::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/resume.rs
git commit -m "feat(core): resume Kiro sessions from their session files"
```

---

### Task 7: The CLI

**Files:**
- Modify: `cli/src/args.rs:31, 133, 210-214`
- Modify: `cli/src/commands.rs:151-157, 305-313`

- [ ] **Step 1: Write the failing tests**

In `cli/src/args.rs` tests (`start_takes_an_agent_and_a_name`):

```rust
        let Cmd::Start { options, .. } = parse(&a("start --agent kiro --prompt go")).unwrap() else { panic!() };
        assert_eq!(options.agent, maya_core::model::Harness::Kiro);
        assert!(parse(&a("start --agent other")).unwrap_err().contains("unknown agent: other"), "the decode fallback is not a choice");
        assert!(USAGE.contains("kiro"));
```

(`a` is the helper that splits a string into args in that test module; if it is absent, build the `Vec<String>` by hand as the existing assertion does.)

In `cli/src/commands.rs` test `start_refuses_a_name_it_could_not_type_later`, add `Harness::Kiro` to the loop and change the expected text to:

```
--name only works for Claude Code from the command line; name Codex, Antigravity, Grok and Kiro sessions from the Maya app, or rename them once they are free.
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p maya_cli`
Expected: FAIL on the usage text, the `other` guard and the message.

- [ ] **Step 3: Implement**

- `USAGE`: `--agent <claude-code|codex|antigravity|grok|kiro>`.
- The `--agent` arm: after decoding, refuse the fallback:

```rust
            "--agent" => {
                let v = take_value(rest, &mut i, "--agent")?;
                let agent = serde_json::from_value::<maya_core::model::Harness>(serde_json::Value::String(v.clone())).ok().filter(|a| *a != maya_core::model::Harness::Other);
                options.agent = agent.ok_or_else(|| format!("{USAGE}\nunknown agent: {v} (claude-code, codex, antigravity, grok or kiro)"))?;
            }
```

- `commands.rs:156`: the new message above.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya_cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add cli/src/args.rs cli/src/commands.rs
git commit -m "feat(cli): maya start --agent kiro"
```

---

### Task 8: The frontend

**Files:**
- Modify: `src/types.ts:19-20`
- Modify: `src/harness.ts`
- Modify: `src/harness.test.ts`
- Modify: `src/firstrun.ts:53`, `src/firstrun.test.ts:32`
- Modify: `src/settings.test.ts`, `src/newsession.test.ts` or `src/resume.test.ts` only if a test enumerates every harness label (grep `Grok Build` in `src/*.test.ts` and extend each list that must stay complete)

- [ ] **Step 1: Write the failing tests**

`src/harness.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { CAPABILITIES, capabilitiesOf, harnessBadge, harnessLabel } from "./harness";
import type { Harness } from "./types";

describe("capabilities", () => {
  it("mirror the Rust table", () => {
    expect(CAPABILITIES).toEqual({
      "claude-code": { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true },
      codex: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: true, shellLines: false },
      antigravity: { compact: true, modelSwitch: false, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
      grok: { compact: true, modelSwitch: true, effortSwitch: false, modeCycle: true, slashLines: true, shellLines: false },
      kiro: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true },
      other: { compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false },
    });
    expect(capabilitiesOf("antigravity")).toBe(CAPABILITIES.antigravity);
  });

  it("offer nothing for an agent this build does not know", () => {
    expect(capabilitiesOf("nonesuch" as Harness)).toEqual({ compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false });
  });
});

describe("labels and badges", () => {
  it("name Kiro and show an unknown agent as text", () => {
    expect(harnessLabel("kiro")).toBe("Kiro CLI");
    expect(harnessLabel("other")).toBe("Unknown agent");
    expect(harnessBadge("kiro", "b").tagName).toBe("IMG");
    const badge = harnessBadge("other", "b");
    expect(badge.tagName).toBe("SPAN");
    expect(badge.textContent).toBe("Unknown agent");
  });
});
```

`src/firstrun.test.ts:32`: expect `"Claude Code, Codex, Antigravity, Grok Build or Kiro CLI"`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `pnpm test -- harness firstrun`
Expected: FAIL (`kiro` not in the table; text differs; `tsc` complains about `"kiro"` not being a `Harness`).

- [ ] **Step 3: Implement**

`src/types.ts`:

```ts
/** The agent runner behind a session; "other" is one this build does not know. */
export type Harness = "claude-code" | "codex" | "antigravity" | "grok" | "kiro" | "other";
```

`src/harness.ts`: add `import kiroIcon from "./assets/icons/kiro.png";`, then:

```ts
export const HARNESS_LABEL: Record<Harness, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  antigravity: "Antigravity",
  grok: "Grok Build",
  kiro: "Kiro CLI",
  other: "Unknown agent",
};
```

add `kiro: kiroIcon,` to `HARNESS_ICON` (no entry for `other`: it falls back to the text badge), and to `CAPABILITIES`:

```ts
  kiro: { compact: true, modelSwitch: true, effortSwitch: true, modeCycle: true, slashLines: true, shellLines: true },
  other: { compact: false, modelSwitch: false, effortSwitch: false, modeCycle: false, slashLines: false, shellLines: false },
```

`src/firstrun.ts:53`: `"None of the agents Maya can use is installed: Claude Code, Codex, Antigravity, Grok Build or Kiro CLI. Install one, then pick it in Settings."`

- [ ] **Step 4: Run the tests and the build**

Run: `pnpm test && pnpm build`
Expected: PASS, and `tsc` is clean. If `pnpm build` reports a `Record<Harness, …>` elsewhere missing `kiro` or `other`, add the entries there with the same values as above.

- [ ] **Step 5: Commit**

```bash
git add src/types.ts src/harness.ts src/harness.test.ts src/firstrun.ts src/firstrun.test.ts
git commit -m "feat(ui): Kiro CLI's icon, label and controls, and a text badge for unknown agents"
```

---

### Task 9: Documentation

**Files:**
- Modify: `README.md:17, 37, 271, 325-326, 358-361`
- Modify: `docs/DEVELOPING.md:78-82, 291-292`

- [ ] **Step 1: Edit**

- `README.md:17`: "Shows Claude Code, Codex, Antigravity, Grok Build and Kiro CLI sessions on one board…".
- `README.md:37`: add `[Kiro CLI](https://kiro.dev/cli)` to the "One of …" list.
- `README.md:271`: "…plus `lsof` and `ps` to find Codex, Antigravity and Kiro sessions."
- `README.md:325-326`: "Codex, Antigravity, Grok Build and Kiro CLI sessions are found through their running processes and their own session files." Add a sentence after it: "A Kiro session waiting for a tool approval shows as Working with the command it is waiting on: Kiro writes nothing to disk that tells the two apart."
- After the Windows bullet about Antigravity and Codex (`README.md:358-361`): "Kiro sessions are found as on macOS: each session's lock file names its agent process, and the console is the TUI's."
- In the networking part of the README (the section that explains pairing; find "paired" or "assistant"), add: "Both machines need version 0.11.0 or later to show Kiro sessions: a main on an older release cannot read a card from an agent it does not know, and keeps that machine's last board with an unreadable-board note. From 0.11.0 on, a card from an agent Maya does not know shows as 'Unknown agent' instead."
- `docs/DEVELOPING.md:291-292`: `foreign.rs` with `codex.rs`, `antigravity.rs`, `grok.rs`, `kiro.rs` (other agents).
- `docs/DEVELOPING.md:78-82` (Windows): add "a `kiro-cli-chat` agent pid to its session through the lock file beside the session, and to its console through the TUI that is its parent."

- [ ] **Step 2: Check the wording**

Run: `grep -n "Antigravity or Grok Build\|Antigravity and Grok Build\"" README.md docs/DEVELOPING.md src/*.ts cli/src/*.rs`
Expected: no line still lists four agents where five belong (the spec and plan files are historical and stay).

- [ ] **Step 3: Commit**

```bash
git add README.md docs/DEVELOPING.md
git commit -m "docs: Kiro CLI beside the other agents"
```

---

### Task 10: Real-app check and version bump

**Files:**
- Modify: `src-tauri/tauri.conf.json`, `package.json`, `Cargo.toml`, `Cargo.lock` (via the script)

- [ ] **Step 1: Run everything**

Run: `cargo test --workspace && pnpm test && pnpm build`
Expected: all PASS.

- [ ] **Step 2: Real-app check, with the user**

Follow the memory note "Maya run and verify" (vite plus the debug binary, screenshot then click). With the user's go-ahead:
1. The user's running Kiro session appears as a card: name "run ls" (or its current title), folder, Working while its approval is pending with the snippet `shell: mkdir -p /tmp/kiro-probe && sleep 90`.
2. Reply from the card once the session is free: the text lands in the Kiro terminal.
3. Rename it from the session modal: Kiro's title changes in its session file.
4. New session with agent Kiro CLI, a name, a model and effort: the terminal opens on `kiro-cli chat --model … --effort … -- "…"`, the session shows, and the name is typed as `/rename` once free. Also start one whose prompt begins with `-v please`: the prompt reaches Kiro as a prompt.
5. Resume: the Kiro sessions for that folder list with titles and the running mark; resuming opens `kiro-cli chat --resume-id`.
6. Settings: pick Kiro CLI as Maya's agent; say "Maya, what's waiting on me?" and start a session with "Let Maya choose".
If step 4's `--` is not accepted by Kiro as the end of options, change `session_command` to pass the prompt through stdin for Kiro (`printf '%s' "$p" | kiro-cli chat …`) and note it in the spec's Findings.

- [ ] **Step 3: Bump the version**

Run: `git fetch origin main && git show origin/main:package.json | grep '"version"'` to read `main`'s version; then `sh scripts/set-version.sh 0.11.0` (or the next minor above `main`'s).

```bash
git add src-tauri/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.11.0"
```

- [ ] **Step 4: Open the pull request**

Ask the user before pushing (user rule). Then `git push -u origin feat/kiro-agent` and open the PR titled `feat: Kiro CLI as a Maya agent` with a description that lists: the board, New session, Resume, rename, controls, brain, the approval limitation, the `Other` fallback and the older-main note, ending with the attribution lines from the session reminder.
