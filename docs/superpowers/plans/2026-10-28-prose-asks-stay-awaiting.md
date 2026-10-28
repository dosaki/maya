# Prose Asks Stay Awaiting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a session's last turn asks the user for a decision or action, in any wording and anywhere in the message, its card stays Awaiting until the user replies. This holds through idle compaction, Maya restarts and session resume.

**Architecture:** The transcript tail finds the last finished turn even when compaction has written megabytes after it, and derive treats that turn end as a Stop when no hook said so. Maya's agent model judges each finished message once ("does it ask?"). The verdict is cached on disk by session and message text. Derive and the foreign harnesses read the cached verdict, and a wider wording check is the fallback when no agent can judge.

**Tech Stack:** Rust (`maya-core`, Tauri app in `src-tauri`), serde_json, the agent CLIs' one-shot mode (`launch::oneshot_args`).

**Spec:** No separate spec. This plan argues from the bug report (six sessions that asked in prose and show as Idle) and the evidence under Background.

## Background (evidence)

The current `asking_line` (`core/src/state.rs:67`) returns None for all six final messages.

| Session | Ask | Bytes after the final message | Hook events | Causes |
|---|---|---|---|---|
| Collector `53e2b561` | "Which way should I build it?", bold, 7 sentences from the end | 322 KB | Stop | wording, compaction |
| Forcefield Domains `5c18439a` | "your call, not mine", second-last paragraph | 1.3 KB | Stop | wording |
| Lilypad `eb39cf35` | "tell me which thread to use", second-last paragraph | 463 KB | Stop | wording, compaction |
| Maya `67704235` | "If you'd like that fixed, I can…" | 566 KB | Stop, SessionEnd | wording, compaction |
| Website Email Domains `40759d42` | "I can make it now if you say go." | 438 KB | Stop | wording, compaction |
| Zendesk MCP `090e7451` | `! …` commands, then "After you run them, I'll check…" | 3 KB | none (resumed 26 Oct, last turn 23 Oct) | wording, no Stop |

- **Compaction:** idle compaction writes a `system` `compact_boundary` record, a `user` record with `"isCompactSummary": true` and `"isVisibleInTranscriptOnly": true`, and up to 220 KB per `attachment` record.
- **Wording check:** a wider check (questions and a longer phrase list over the last 6 sentences) caught 5 of 6 and flagged 33% of 4,034 recent turn ends, against 18% today, with some wrong matches. That is why the model is the authority here.

## Global Constraints

- Branch from `origin/main` (0.37.0), not `feat/chat-markdown`. This adds a capability, so the release is `0.38.0`: `sh scripts/set-version.sh 0.38.0`, committed on its own as `chore(release): 0.38.0`.
- Cache file: `<claude_dir>/maya/asks.json`.
- Judge model: `config.brain_model()`. When that is None and `config.brain()` is `Harness::ClaudeCode`, use `"haiku"`.
- Judge timeout: 30 s. Text sent: the last 8,000 characters of the message.
- Scan cap for the transcript tail: `TURNS_SCAN_MAX_BYTES` (8 MB).
- No new crate dependencies.
- No new field on `Card`. It is built literally in about 20 files.

## Review Focus

1. **A reply after the ask:** a typed prompt, an Esc (`MayaInterrupt`) or a user record in the transcript must clear Awaiting at once, also when no hook event exists. Pinned in Task 1 (`a_user_prompt_after_the_turn_clears_the_turn_end`) and Task 3 (`a_reply_event_clears_a_judged_ask`).
2. **The compaction summary:** the `isCompactSummary` user record must not count as a reply. Pinned in Task 1.
3. **Double announcements:** a card waiting for its verdict must not be announced as finished and then announced again as asking. Pinned in Task 6 (the notifier skips judging sessions) and Task 5 (`judging_sessions` lists them).
4. **A failed or unusable model reply:** a timeout, a missing binary or JSON without `asks` must fall back to the wording check once. It must not retry on every refresh. Pinned in Task 5 (`a_failed_judgement_falls_back_to_the_wording_check`).
5. **A turn with no text:** a turn that ended on a tool call or was interrupted (`stop_reason` not `end_turn`) gives no turn end, so nothing is judged. Pinned in Task 1.

Accepted behavior: an ask is judged once per message. The first time Maya judges an old ask (just after this release) it becomes Awaiting and is announced once. After that, a restart finds the verdict cached and says nothing.

---

### Task 1: The transcript reports the last turn end, even past compaction

**Files:**
- Modify: `core/src/transcript.rs` (`TranscriptTail`, `parse_tail`, `TailCache::get`)
- Test: `core/src/transcript.rs` tests module

**Interfaces:**
- Produces:
  - `pub struct TurnEnd { pub at_ms: u64, pub text: String }` (derive `Debug, Clone, PartialEq, Eq`)
  - a new field `TranscriptTail.turn_end: Option<TurnEnd>`
  - `pub fn read_last_turn(path: &Path) -> TranscriptTail`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_turn_end_is_the_last_end_turn_message_with_its_text_and_time() {
    let text = concat!(
        r#"{"type":"user","message":{"content":"go"},"timestamp":"2026-10-28T13:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Which way? A or B."}]},"timestamp":"2026-10-28T13:01:14.734Z"}"#, "\n",
        r#"{"type":"system","subtype":"compact_boundary"}"#, "\n",
        r#"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"content":"This session is being continued"}}"#, "\n",
    );
    let t = parse_tail(text);
    assert_eq!(t.turn_end, Some(TurnEnd { at_ms: crate::codex::ms_of("2026-10-28T13:01:14.734Z").unwrap(), text: "Which way? A or B.".into() }));
}

#[test]
fn a_user_prompt_after_the_turn_clears_the_turn_end() {
    // A typed prompt clears it; a tool result, an isMeta record and a sidechain record do not.
}

#[test]
fn a_turn_that_did_not_end_on_text_has_no_turn_end() {
    // stop_reason "tool_use", or a synthetic "<synthetic>" interruption record: turn_end is None.
}

#[test]
fn read_last_turn_reads_back_past_megabytes_of_compaction_records() {
    // Temp file: the end_turn ask, then 400 KB of {"type":"attachment",...} lines, then the compact summary.
    // read_tail(p, TAIL_BYTES).turn_end is None; read_last_turn(p).turn_end is Some and last_assistant_text is the ask.
    // TailCache::get(p) agrees with read_last_turn.
}
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p maya-core transcript::tests::the_turn_end transcript::tests::a_user_prompt transcript::tests::a_turn_that transcript::tests::read_last_turn`
Expected: they fail to compile because `TurnEnd` and `turn_end` are not defined.

- [ ] **Step 3: Implement**

- **`parse_tail`:** a main-chain record (no `isSidechain: true`) of type `assistant` with `message.stop_reason == "end_turn"` sets `turn_end` from the last text seen up to that record, with the record's `timestamp` (parse it with `crate::codex::ms_of`). Synthetic records, whose model starts with `<`, never set it.
- **What clears it:** a `user` record clears `turn_end` when it is a prompt: its content is a string or has a `text` block, and it has none of `isCompactSummary`, `isMeta` or `isVisibleInTranscriptOnly`. Records that carry only `tool_result` blocks do not clear it.
- **`read_last_turn`:** reads `TAIL_BYTES`, then doubles the window while `last_assistant_text` is None and the window is smaller than both the file and `TURNS_SCAN_MAX_BYTES`.
- **`TailCache::get`:** calls `read_last_turn` instead of `read_tail`.

- [ ] **Step 4: Run the transcript tests**

Run: `cargo test -p maya-core transcript::`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/transcript.rs
git commit -m "fix(state): find the last turn end past idle compaction"
```

### Task 2: The wording check looks at the whole message

**Files:**
- Modify: `core/src/state.rs:28-79` (`DECISION_PHRASES`, `last_line`, `asking_line`)
- Test: `core/src/state.rs` tests (`asking_line_spots_questions_and_decision_phrases_at_the_tail_only` becomes the test below)

**Interfaces:**
- Produces: `pub fn asking_line(text: &str) -> Option<String>`. The signature stays the same. It now returns the last sentence anywhere in the message that asks, or the first `! ` command line when the message asks the user to run commands.

- [ ] **Step 1: Rewrite the test**

```rust
#[test]
fn asking_line_finds_the_ask_anywhere_in_the_message() {
    assert_eq!(asking_line("**Which way should I build it?**\n- A: one.\n- B: two.\n\nI recommend A. It is safer. It costs more."), Some("**Which way should I build it?**".into()));
    assert_eq!(asking_line("Option 2 is a question about who opens PRs — your call, not mine.\n\nAnd it isn't collector-specific."), Some("Option 2 is a question about who opens PRs — your call, not mine.".into()));
    assert_eq!(asking_line("I'll only do it if you say so and tell me which thread to use.\n\nOtherwise, try the checks yourself."), Some("I'll only do it if you say so and tell me which thread to use.".into()));
    assert_eq!(asking_line("All green. If you'd like that fixed, I can do it in a separate PR."), Some("If you'd like that fixed, I can do it in a separate PR.".into()));
    assert_eq!(asking_line("It's a single AWS call, and I can make it now if you say go."), Some("It's a single AWS call, and I can make it now if you say go.".into()));
    assert_eq!(asking_line("Run these:\n```\n! cd ~/dev/x && sh a.sh\n```\nAfter you run them, I'll check the policies."), Some("After you run them, I'll check the policies.".into()));
    // Kept from the old test: closing lines, code fences, trailing asides.
    assert_eq!(asking_line("Ready?\n```\ncode\n```"), Some("Ready?".into()));
    assert_eq!(asking_line("Go ahead? [y/n]"), Some("Go ahead? [y/n]".into()));
    // A question inside code, a table or a quote is not an ask.
    assert_eq!(asking_line("```\nwhy?\n```\nDone."), None);
    assert_eq!(asking_line("| a? | b |\n|---|---|\n\nAll done."), None);
    assert_eq!(asking_line("> Should we?\n\nFixed and pushed."), None);
    assert_eq!(asking_line("All done."), None);
    assert_eq!(asking_line(""), None);
}
```

The old case `"Why did it fail? Because of X.\n\nFixed and pushed."` → None is deliberately dropped. The wider check is the fallback, and the model decides cases like it.

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p maya-core state::tests::asking_line_finds_the_ask_anywhere_in_the_message`
Expected: FAIL on the first assertion (left `None`).

- [ ] **Step 3: Implement**

- **Lines to drop:** fenced code blocks, table rows (`|`), blockquotes (`>`) and lines that are only `! ` commands.
- **Sentences:** split the rest into sentences at `.`, `?` or `!` followed by whitespace. Keep the text a sentence had in the message, including markdown, so the returned line reads as written.
- **Scan:** scan sentences from the end. A sentence asks when it ends in `?` (after `strip_trailing_asides` and trailing markdown) or contains a phrase from `DECISION_PHRASES` on whole words.
- **Phrases:** extend `DECISION_PHRASES` with: `"if you'd like"`, `"if you would like"`, `"if you want"`, `"if you say"`, `"say go"`, `"say so"`, `"tell me"`, `"up to you"`, `"your decision"`, `"after you run"`, `"once you run"`, `"please run"`, `"when you're ready"`.
- **Commands:** when nothing matches but the message held `! ` command lines (inside a code fence or not), return the first of those lines.

- [ ] **Step 4: Run the state tests**

Run: `cargo test -p maya-core state::`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/state.rs
git commit -m "fix(state): look for a prose ask anywhere in the last message"
```

### Task 3: Derive takes a verdict and the transcript's turn end

**Files:**
- Modify: `core/src/state.rs` (`DeriveInput`, `derive`), `core/src/foreign.rs:42` (`derive`), `core/src/store.rs` (the call sites at :425 and :491, passing `&Prose::Unjudged` for now)
- Test: `core/src/state.rs` and `core/src/foreign.rs` tests

**Interfaces:**
- Consumes: `TranscriptTail.turn_end` (Task 1), `asking_line` (Task 2).
- Produces:
  - in `state.rs`: `pub enum Prose { Unjudged, Judging, Asks(String), Done }` (derive `Debug, Clone, PartialEq, Eq`)
  - a new field `DeriveInput.prose: &'a Prose`
  - `foreign::derive(s, t, prose: &Prose, now_ms, completed_timeout_ms)`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_judged_ask_stays_awaiting_long_after_the_turn() {
    // Stop at 0, now = 5 h, prose = Asks("Which way?") → Awaiting, kind Text, detail "Which way?", state_since 0.
}
#[test]
fn a_done_verdict_overrides_a_question_mark() {
    // last text "Why? Because X.", prose = Done → Completed before the timeout, Idle after.
}
#[test]
fn judging_and_unjudged_fall_back_as_documented() {
    // Judging → Completed (or Idle) with no ask; Unjudged → asking_line on the text.
}
#[test]
fn a_resumed_session_with_no_events_takes_its_turn_end_from_the_transcript() {
    // events = [], registry "idle", transcript.turn_end = Some(at 1_000), prose = Asks("Run these") → Awaiting since 1_000.
    // The same with prose = Done and now far past the timeout → Idle since 1_000 + TIMEOUT.
}
#[test]
fn a_reply_event_clears_a_judged_ask() {
    // Stop at 0, UserPromptSubmit at 10, prose = Asks(..) → not Awaiting.
}
#[test]
fn a_busy_session_is_working_whatever_the_verdict() {}
// In foreign.rs: a_foreign_session_follows_its_verdict (Asks → Awaiting; Done → Completed).
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p maya-core state:: foreign::`
Expected: they fail to compile because `Prose` is not defined.

- [ ] **Step 3: Implement**

- **Turn end from the transcript:** after the event loop, if `last_turn` is None, the status is not `busy` and `transcript.turn_end` is Some, set `last_turn = Some((true, turn_end.at_ms))`.
- **`asked`:** `Asks(line)` → `Some(line)`; `Unjudged` → `asking_line(text)`, where text is the turn end's text, else `last_assistant_text`; `Judging` and `Done` → None.
- **`foreign::derive`:** same mapping, with `t.last_agent_text` in place of the turn end's text.

- [ ] **Step 4: Run the core tests**

Run: `cargo test -p maya-core`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/state.rs core/src/foreign.rs core/src/store.rs
git commit -m "fix(state): read a finished turn from the transcript when no Stop arrived"
```

### Task 4: The verdict cache and the judge call

**Files:**
- Create: `core/src/asks.rs` (add `pub mod asks;` to `core/src/lib.rs`)
- Test: `core/src/asks.rs` tests module

**Interfaces:**
- Consumes: `interpreter::output_within`, `interpreter::RunError`, `launch::oneshot_args`, `launch::final_text`, `launch::binary_name`, `state::asking_line`.
- Produces:
  - `#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)] pub struct Verdict { pub asks: bool, pub line: Option<String> }`
  - `pub fn key(session_id: &str, text: &str) -> String` — `"<session_id>:<16 hex digits of FNV-1a 64 over text>"`
  - `pub struct AskCache` with `load(path: PathBuf) -> AskCache`, `get(&self, key: &str) -> Option<&Verdict>`, `insert(&mut self, key: String, v: Verdict)`, `retain(&mut self, live: &HashSet<String>)` and `save(&mut self) -> std::io::Result<()>`. `save` writes only after a change, through a temp file and a rename.
  - `pub fn parse_verdict(agent: Harness, stdout: &str, text: &str) -> Result<Verdict, String>`
  - `pub fn judge(agent: Harness, binary: &Path, model: Option<&str>, text: &str, cwd: &Path) -> Result<Verdict, RunError>`
  - `pub const SYSTEM_PROMPT: &str`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn keys_are_stable_and_change_with_the_text() {
    assert_eq!(key("s", "abc"), key("s", "abc"));
    assert_ne!(key("s", "abc"), key("s", "abd"));
    assert_eq!(key("s", "").len(), "s:".len() + 16);
}
#[test]
fn the_cache_round_trips_through_its_file_and_forgets_dead_keys() {}
#[test]
fn a_verdict_reads_from_the_agents_reply() {
    // Claude Code JSON envelope whose result is {"asks":true,"line":"Which way should I build it?"} → Verdict { asks: true, line: Some(..) }.
    // {"asks":false,"line":null} → Verdict { asks: false, line: None }.
    // {"asks":true} with no line → line from asking_line(text), else the message's last sentence.
    // A line not found in the text is replaced in the same way.
    // No JSON, or no boolean "asks" → Err.
}
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p maya-core asks::`
Expected: fail to compile (module missing).

- [ ] **Step 3: Implement**

- **`judge`:**
  - Build the command as `interpreter::run` does: `crate::command(binary)`, `oneshot_args(agent, model, Some(SYSTEM_PROMPT), &user)`, `current_dir(cwd)`, `env_clear().envs(launch::clean_env(..))`.
  - Run it with `output_within(.., Duration::from_secs(30))`.
  - `user` is `"Message:\n<<<\n{last 8000 chars of text}\n>>>"`.
  - Log one `asks` line per call through `crate::log::line`, with the verdict and the time taken.
- **`SYSTEM_PROMPT`** (exact copy):

```text
You read the last message an AI coding agent wrote to its user when it finished a turn. Decide whether the message asks the user for something before work can go on: a decision, an answer, an approval, or an action such as running a command or checking a result. The ask can be anywhere in the message and in any wording. An offer that waits on the user's go-ahead ("I can fix that if you want") counts. A message that only reports what was done, with nothing for the user to decide or do, does not.
Reply with JSON only: {"asks": true, "line": "<the sentence that asks, copied exactly from the message>"} or {"asks": false, "line": null}.
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core asks::`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/asks.rs core/src/lib.rs
git commit -m "feat(state): judge whether a finished message asks, cached per message"
```

### Task 5: The store gives derive a verdict and queues the judging

**Files:**
- Modify: `core/src/store.rs` (struct fields, `new`, `refresh`, `claude_card`)
- Test: `core/src/store.rs` tests (use `Store::new(claude).with_alive(|_| true)` as the existing tests do)

**Interfaces:**
- Consumes: `asks::{AskCache, Verdict, key}` (Task 4), `Prose` (Task 3), `TranscriptTail.turn_end` (Task 1).
- Produces:
  - `pub judge_asks: bool` (false by default, so the CLI and tests use the wording check)
  - `pub fn take_due_asks(&mut self) -> Vec<(String, String)>` — `(key, text)`, each key handed out once
  - `pub fn set_ask_verdict(&mut self, key: String, v: Verdict)`
  - `pub fn set_ask_failed(&mut self, key: String)`
  - `pub fn judging_sessions(&self) -> HashSet<String>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_finished_turn_is_judged_once_and_its_verdict_is_kept_across_stores() {
    // A registry session at "idle", a transcript whose turn ended on "I can make it now if you say go."
    // judge_asks = true: refresh → card Completed, judging_sessions() has it, take_due_asks() returns one (key, text), and a second call returns none.
    // set_ask_verdict(key, Verdict{asks:true, line:Some("…say go.")}) → refresh → Awaiting; judging_sessions() is empty.
    // A new Store on the same claude_dir with judge_asks = true → Awaiting on its first refresh, nothing due.
}
#[test]
fn a_failed_judgement_falls_back_to_the_wording_check() {
    // set_ask_failed(key) → Prose::Unjudged → Awaiting from asking_line; nothing due again in this store.
}
#[test]
fn without_a_judge_the_wording_check_decides() {
    // judge_asks = false → no due asks, the card follows asking_line.
}
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p maya-core store::tests::a_finished_turn store::tests::a_failed_judgement store::tests::without_a_judge`
Expected: they fail to compile.

- [ ] **Step 3: Implement**

- **Fields:** `asks: AskCache` loaded from `claude_dir/maya/asks.json` in `new`, `due_asks`, `queued: HashSet<String>` and `failed: HashSet<String>`.
- **For each Claude Code card:** the text is `tail.turn_end.text` (else `last_assistant_text` when the events end on a Stop), and `key(session_id, text)`.
  - A cached verdict gives `Asks(line)` or `Done`.
  - With no verdict, the key failed or `judge_asks` off, the result is `Unjudged`.
  - Otherwise it is `Judging`, and the key is queued once.
- **Foreign sessions:** the same, with `last_agent_text` when the tail is not working.
- **When refresh ends:** `retain` the cache to the keys this refresh used, and `save` it. Log a save error once, as `save_memory` does.

- [ ] **Step 4: Run the core tests**

Run: `cargo test -p maya-core`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/store.rs
git commit -m "feat(state): hold a finished turn's card on its cached verdict"
```

### Task 6: The app judges in the background and holds back early announcements

**Files:**
- Modify: `src-tauri/src/lib.rs`: a new `poll_asks(app: &AppHandle)` next to `poll_pull_requests` (:1132); its thread next to the review and deployment loops (:1674); `refresh_and_emit` (:688), so that `take_finished` skips judging sessions; startup sets `store.judge_asks = true`.

**Interfaces:**
- Consumes: `Store::{take_due_asks, set_ask_verdict, set_ask_failed, judging_sessions}` (Task 5), `asks::judge` (Task 4), `launch::find_binary`.

- [ ] **Step 1: Implement `poll_asks`**

- Take the due asks, the brain and the model (Global Constraints) under the store lock.
- Find the binary. If none is found, call `set_ask_failed` for every key.
- With no lock held, `judge` each text in turn. Use `<claude_dir>/maya` as `cwd`.
- Under the lock, call `set_ask_verdict` or `set_ask_failed`, then `refresh_and_emit(app)` once.
- The thread loops `poll_asks` then sleeps 2 s.

- [ ] **Step 2: Keep finished announcements for judged cards**

In `refresh_and_emit`, read `judging_sessions()` while the store lock is held. Pass `take_finished` only the cards not in that set, so a card is announced once its verdict lands, as finished or as asking.

- [ ] **Step 3: Build and run the app tests**

Run: `cargo test -p maya-core && cargo test -p maya`
Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(app): judge finished turns with Maya's agent in the background"
```

### Task 7: Check against the six transcripts, then release

**Files:**
- Scratch only (nothing in the repo): a test under `core/tests/` that is deleted before commit, or a scratch-dir example.
- Modify: version files through `scripts/set-version.sh`.

- [ ] **Step 1: Run the tail and the wording check on the six real transcripts**

For each of the six paths under `~/.claude/projects/*/<id>*.jsonl`:
- call `read_last_turn`;
- assert that `turn_end` is Some and its text is the final message from the Background table;
- assert that `asking_line(text)` is Some.

Expected: 6/6.

- [ ] **Step 2: Run the judge on the same six texts**

Call `asks::judge` with the user's agent. Expected: `asks: true` for all six, each `line` naming the ask in the Background table. Run it on 10 sampled turn ends that only report work done. Expected: `asks: false` on at least 9.

- [ ] **Step 3: Check it in the running app**

Start Maya (`docs/DEVELOPING.md`). The live sessions among the six show Awaiting, with the ask as the card's detail. Restart Maya: they are still Awaiting, with no new announcement. Reply in one of them: its card leaves Awaiting.

- [ ] **Step 4: Bump the version and commit**

```bash
sh scripts/set-version.sh 0.38.0
git add -A src-tauri/tauri.conf.json package.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.38.0"
```
