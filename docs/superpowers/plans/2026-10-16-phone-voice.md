# The phone speaks and listens — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A phone joined to a desktop main takes voice commands (tap-to-talk, plus an optional "Maya" wake word) that the main's agent interprets, speaks the answers, and announces board events aloud, all only while the app is on screen.

**Architecture:** The wake-word state machine moves into `core` so both apps share it. Two new protocol requests (`Interpret`, `Execute`) let the phone use the main's interpreter without touching the desktop's own conversation; the main advertises them with a `voice` cap in `welcome`. On the phone, a host-testable Rust `Voice` drives a `VoiceIo` trait implemented by a new Kotlin `VoicePlugin` (Android's `SpeechRecognizer` and `TextToSpeech`), whose events come back to Rust over a channel; the page only shows the view and sends taps. Kotlin itself stops the mic and the voice when the app leaves the screen.

**Tech Stack:** Rust (maya-core, Tauri 2 desktop and mobile), Kotlin (Tauri Android plugin, `android.speech`, `ProcessLifecycleOwner`), TypeScript (vitest).

**Spec:** `docs/superpowers/specs/2026-10-16-phone-voice-design.md`

## Global Constraints

- Voice is offered only while the phone has joined a desktop main whose `welcome` carries the `voice` cap and whose mirror switch allows this phone.
- Nothing listens or speaks while the app is not on screen; Kotlin enforces this independently of the page and of Rust.
- The page never calls the voice plugin: its events reach Rust through a `tauri::ipc::Channel` given to the plugin's `watch` command, and Rust asks for the microphone (ledger ruling R1).
- The wake word runs only on the on-device recogniser (API 31+); tap-to-talk may use the default recogniser.
- The read-back for a confirmation is the main's `spoken_for(action)`, never the model's `say`.
- Actions needing a yes: reply, answer, start, resume, review (`interpreter::needs_confirm`).
- `COMMAND_WAIT_MS` 8 000, `CONFIRM_WAIT_MS` 10 000 (unchanged, from `wake.rs`).
- History sent with `Interpret`: at most the last 12 turns.
- Wake word loop backoff: 1, 2, 4, 8, 16 s; stop after 5 failures in a row with "Wake word stopped: <reason>".
- `PROTOCOL` stays 2; `Welcome.caps` is optional (`#[serde(default)]`).
- Speak setting reuses `speak_notifications` (default on); the wake word is a new `wake_word` field (default off).
- Board events are spoken only with Speak on, the app on screen, and the phone not on silent, vibrate or Do Not Disturb; replies are always spoken.
- A spoken event's notification goes to a new channel `spoken` (low importance: no sound, no vibration).
- Commit messages follow Conventional Commits and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- The version bump is one `chore(release)` commit at the end, a minor bump from `main`'s version at that time (`sh scripts/set-version.sh`, then check `Cargo.lock` changed).

## Review Focus

- **A late answer after the app left the screen.** The user asks, then switches app before the main answers: nothing is spoken and no pending action survives. (Task 6: `a_late_answer_after_leaving_the_screen_is_dropped`.)
- **Silence after a tap.** The user taps and says nothing; the recogniser ends with no result: the phone goes idle quietly in the command window and says "OK." once a confirmation window runs out. (Task 6: `silence_after_a_tap_goes_idle` and `no_answer_to_a_read_back_cancels`.)
- **An event while a conversation is under way.** A decision arrives while the phone waits for "yes": the event waits until the conversation is idle, and is not read into the yes/no. (Task 6: `an_event_waits_for_the_conversation`.)
- **An action gone stale between read-back and yes.** The ask was answered at the desk during the 10 s window: the main re-validates and the phone speaks the reason. (Task 4: `execute_revalidates_against_the_board_now`.)
- **An old main.** A main without the `voice` cap: the mic is disabled with "Update <main> to talk to Maya." and no request is sent. (Task 2: `an_old_welcome_without_caps_still_decodes`; Task 6: `no_tap_without_the_voice_cap`.)

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `core/src/wake.rs` | moved from `src-tauri/src/wake.rs` | wake word and `Flow` |
| `core/src/lib.rs` | modify | `pub mod wake;` |
| `src-tauri/src/lib.rs` | modify | drop `pub mod wake;`, use `maya_core::wake` |
| `core/src/net/protocol.rs` | modify | `Request::Interpret`, `Request::Execute`, `VoiceTurn`, `CAP_VOICE`, `Welcome.caps` |
| `core/src/net/server.rs` | modify | `Notify::main_caps`, send caps in `welcome` |
| `core/src/net/client.rs` | modify | pass `welcome.caps` to `ClientNotify::main_caps` |
| `core/src/net/mod.rs` | modify | `AssistantLink.voice` |
| `core/src/config.rs` | modify | `wake_word` |
| `src-tauri/src/listener.rs` | modify | `Interpreted`, `VoiceContext`, `decide`, `exchanges`, `interpret_text`; `interpret` uses them |
| `src-tauri/src/net_app.rs` | modify | serve `Interpret` and `Execute` |
| `src-tauri/src/lib.rs` | modify | `TauriNetNotify::main_caps` |
| `mobile/src/voice.rs` | create | `Conversation`, `VoiceIo`, `VoiceView` |
| `mobile/src/alerts.rs` | modify | `Post.spoken`, `Post.quiet`, `SPOKEN_CHANNEL` |
| `mobile/src/hub.rs` | modify | link `voice`, the `Voice`, speaking events in `refresh`, `serve_request` refusals |
| `mobile/src/android.rs` | modify | `voice` plugin bridge (`AndroidVoice`), the `spoken` channel |
| `mobile/src/commands.rs` | modify | `voice_*` commands; `set_config` learns two fields |
| `mobile/src/lib.rs` | modify | register the plugin and commands, make the `Voice` |
| `mobile/gen/android/app/src/main/java/com/dosaki/maya/mobile/VoicePlugin.kt` | create | recogniser, voice, on-screen state, quiet state |
| `mobile/gen/android/app/src/main/AndroidManifest.xml` | modify | `RECORD_AUDIO` |
| `src/mobile/voice.ts` | create | mic bar (shows the `voice` view, sends taps) |
| `src/mobile/voice.test.ts` | create | mic bar tests |
| `src/mobile/network.ts` | modify | Voice card |
| `src/mobile/network.test.ts` | modify | Voice card tests |
| `src/mobile/main.ts` | modify | mount the bar, start the bridge |
| `src/mobile/mobile.css` | modify | bar styles |
| `docs/DEVELOPING.md` | modify | hand-edited files list |

---

### Task 1: Move `wake` into `core`

**Files:**
- Move: `src-tauri/src/wake.rs` → `core/src/wake.rs`
- Modify: `core/src/lib.rs`, `src-tauri/src/lib.rs:21`, `src-tauri/src/listener.rs:8`

**Interfaces:**
- Produces: `maya_core::wake::{extract, answer, Wake, Answer, Pending, Effect, Flow, COMMAND_WAIT_MS, CONFIRM_WAIT_MS}`, unchanged signatures. `Flow::new`, `ignore_line`, `await_reply(now_ms)`, `set_pending(Pending, now_ms)`, `has_pending`, `state(now_ms) -> &'static str`, `drop_expired(now_ms) -> bool`, `on_segment(&str, now_ms) -> Vec<Effect>`.

- [ ] **Step 1: Move the file with history**

```bash
git mv src-tauri/src/wake.rs core/src/wake.rs
```

- [ ] **Step 2: Declare it in core**

In `core/src/lib.rs`, add next to the other `pub mod` lines (keep them alphabetical):

```rust
pub mod wake;
```

- [ ] **Step 3: Point the desktop at it**

In `src-tauri/src/lib.rs`, delete line 21 `pub mod wake;` and add `wake` to the existing `use maya_core::{…}` re-export list the file already uses for `interpreter`, `launch`, `notify` (find it with `grep -n "use maya_core::{" src-tauri/src/lib.rs`), so `crate::wake` keeps resolving. If the file re-exports with `pub use maya_core::{…}`, add `wake` there; listener.rs line 8 (`use crate::{config, ear, …, wake};`) then needs no change.

- [ ] **Step 4: Run both crates' tests**

Run: `cargo test -p maya-core wake && cargo test -p maya listener`
Expected: PASS; the wake tests now run under `maya-core`.

- [ ] **Step 5: Commit**

```bash
git add core/src/wake.rs core/src/lib.rs src-tauri/src/lib.rs src-tauri/src/listener.rs
git commit -m "refactor: move the wake word and its flow into core

The phone will run the same conversation as the desktop.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Protocol: voice requests and the `voice` cap

**Files:**
- Modify: `core/src/net/protocol.rs:20-123` and its tests (from line 200)
- Modify: `core/src/net/server.rs:84` (trait), `:858` (welcome), test `Counter` (~1365-1430)
- Modify: `core/src/net/client.rs:97-130` (trait), `:486-520` (handshake)
- Modify: `core/src/net/mod.rs:91-100` (`AssistantLink`)
- Modify: `mobile/src/hub.rs:500-510` (`serve_request`), `:653-692` (`HubClientNotify`)
- Modify: `src-tauri/src/net_app.rs:261` (`serve_request`: temporary refusal, replaced in Task 4)

**Interfaces:**
- Produces:
  - `pub const CAP_VOICE: &str = "voice";`
  - `pub struct VoiceTurn { pub who: String, pub text: String }` (serde camelCase)
  - `Request::Interpret { command: String, history: Vec<VoiceTurn> }`, `Request::Execute { action: Value }`
  - `Down::Welcome { name, mirror, caps: Vec<String> }`
  - `Notify::main_caps(&self) -> Vec<String>` (default empty)
  - `ClientNotify::main_caps(&self, caps: &[String])` (default no-op), called right after `mirror_offer`
  - `AssistantLink.voice: bool` (serialised `voice`)

- [ ] **Step 1: Write the failing protocol tests**

Add to the `tests` module of `core/src/net/protocol.rs`:

```rust
    #[test]
    fn voice_requests_round_trip() {
        let ask = Up::Request { id: 7, machine: None, request: Request::Interpret { command: "what needs me".into(), history: vec![VoiceTurn { who: "user".into(), text: "hi".into() }, VoiceTurn { who: "maya".into(), text: "Hello.".into() }] } };
        assert_eq!(decode_up(&encode(&ask)).unwrap(), ask);
        let run = Up::Request { id: 8, machine: None, request: Request::Execute { action: serde_json::json!({"kind": "answer", "session": "s1", "option": 1}) } };
        assert_eq!(decode_up(&encode(&run)).unwrap(), run);
    }

    #[test]
    fn welcome_carries_caps_only_when_there_are_some() {
        let voiced = Down::Welcome { name: "desk".into(), mirror: Some(true), caps: vec![CAP_VOICE.into()] };
        assert_eq!(decode_down(&encode(&voiced)).unwrap(), voiced);
        assert!(!encode(&Down::Welcome { name: "desk".into(), mirror: None, caps: vec![] }).contains("caps"));
    }

    #[test]
    fn an_old_welcome_without_caps_still_decodes() {
        assert_eq!(decode_down(r#"{"type":"welcome","name":"desk","mirror":true}"#).unwrap(), Down::Welcome { name: "desk".into(), mirror: Some(true), caps: vec![] });
    }
```

Also update the three existing `Down::Welcome { name: …, mirror: … }` literals in this test module (lines ~216-246) to add `caps: vec![]`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core protocol`
Expected: FAIL to compile (`VoiceTurn`, `CAP_VOICE`, `Interpret`, `caps` unknown).

- [ ] **Step 3: Add the types**

In `core/src/net/protocol.rs`, after `CAP_MIRROR`:

```rust
/// A main with this cap interprets voice commands for a mirroring Maya
/// (`Request::Interpret`, `Request::Execute`).
pub const CAP_VOICE: &str = "voice";
```

Before `pub enum Request`:

```rust
/// One turn of a phone's voice conversation, oldest first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurn {
    /// "user" or "maya".
    pub who: String,
    pub text: String,
}
```

Add to `enum Request`:

```rust
    /// A spoken command for the main's agent; `history` is the phone's
    /// conversation so far. Answered with `{say, pending?: {say, action}}`.
    Interpret { command: String, #[serde(default)] history: Vec<VoiceTurn> },
    /// A validated action the user confirmed by voice; the main validates it
    /// again against its board before running it. Answered with `{say}`.
    Execute { action: Value },
```

Change `Down::Welcome` to:

```rust
    Welcome {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mirror: Option<bool>,
        /// What this main can do for a mirroring Maya (`CAP_VOICE`); an older main sends none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        caps: Vec<String>,
    },
```

- [ ] **Step 4: Run the protocol tests**

Run: `cargo test -p maya-core protocol`
Expected: the protocol tests PASS; the crate may still fail to compile elsewhere (`server.rs`, `client.rs` literals) — fixed next.

- [ ] **Step 5: Write the failing server test**

In `core/src/net/server.rs` tests, give `Counter` a field and the trait method:

```rust
        /// What `main_caps` answers (`CAP_VOICE` for a main that interprets).
        pub caps: Mutex<Vec<String>>,
```

```rust
        fn main_caps(&self) -> Vec<String> {
            self.caps.lock().unwrap().clone()
        }
```

and the test:

```rust
    #[test]
    fn welcome_names_what_the_main_can_do() {
        let counter = Arc::new(Counter::default());
        *counter.caps.lock().unwrap() = vec![CAP_VOICE.into()];
        let (handle, port) = test_server_with(counter.clone());
        let (ch, id, me) = pair_client(&handle, port, "Pixel");
        drop(ch);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ch = connect_as(&handle, port, &me, "Pixel", &["mirror"]);
        let Down::Welcome { caps, .. } = recv(&mut ch) else { panic!("expected welcome") };
        assert_eq!(caps, vec![CAP_VOICE.to_string()]);
        handle.stop();
    }
```

Add `CAP_VOICE` to the test module's `use` of protocol items.

- [ ] **Step 6: Run it to see it fail**

Run: `cargo test -p maya-core welcome_names_what_the_main_can_do`
Expected: FAIL to compile (`main_caps` not in trait).

- [ ] **Step 7: Send the caps**

In `trait Notify` (`server.rs:84`), after `request`:

```rust
    /// What this main offers a mirroring Maya, sent in `welcome`.
    fn main_caps(&self) -> Vec<String> {
        vec![]
    }
```

At `server.rs:858`:

```rust
    if let Err(e) = send_down(ch, &Down::Welcome { name: ctx.main_name.clone(), mirror: offer, caps: ctx.notify.main_caps() }) {
```

- [ ] **Step 8: Pass the caps to the client's notifier**

In `core/src/net/client.rs`, `trait ClientNotify`, after `mirror_offer`:

```rust
    /// What the main said it can do at `welcome` (`CAP_VOICE`); empty for
    /// an older main. Called right after `mirror_offer`.
    fn main_caps(&self, _caps: &[String]) {}
```

Change `handshake` to return `(String, Option<bool>, Vec<String>)`: the match arm becomes `Down::Welcome { name, mirror, caps } => { … return Ok((…, mirror, caps)); }` and the doc comment says "returns the main's name, its mirror offer and its caps". At the caller (`client.rs:486`):

```rust
    let out = handshake(&mut ch, config, &**notify, stop, learned, deadline).and_then(|(main, mirror, caps)| {
        log::line("network", format!("connected to {main}"));
        notify.connected(&main);
        notify.mirror_offer(mirror);
        notify.main_caps(&caps);
        pump(&mut ch, exec, notify, stop)
    });
```

Fix any other `Down::Welcome { name, mirror }` patterns the compiler reports (e.g. `client.rs:651` uses `{ .. }` and is fine).

- [ ] **Step 9: Record voice on the link**

In `core/src/net/mod.rs`, `AssistantLink`, add:

```rust
    /// The main interprets voice commands for this Maya (`CAP_VOICE` at the last welcome).
    pub voice: bool,
```

Fix every `AssistantLink { … }` literal the compiler reports by adding `voice: false` (e.g. `mobile/src/hub.rs:657`, `src-tauri/src/net_app.rs`).

In `mobile/src/hub.rs`, `impl ClientNotify for HubClientNotify`:

```rust
    fn main_caps(&self, caps: &[String]) {
        let voice = caps.iter().any(|c| c == maya_core::net::protocol::CAP_VOICE);
        self.set(|l| l.voice = voice);
    }
```

- [ ] **Step 10: Refuse voice where it is not served**

In `mobile/src/hub.rs`, `serve_request` (the phone as a main has no agent):

```rust
            Request::Interpret { .. } | Request::Execute { .. } => Err("This phone cannot interpret voice commands.".into()),
```

In `src-tauri/src/net_app.rs`, `serve_request`, add the same arm for now (Task 4 replaces it):

```rust
        Request::Interpret { .. } | Request::Execute { .. } => Err("This Maya cannot interpret voice commands yet.".into()),
```

- [ ] **Step 11: Write the hub test**

In `mobile/src/hub.rs` tests:

```rust
    #[test]
    fn the_phone_as_main_refuses_voice_requests() {
        let hub = test_hub();
        let err = hub.serve_request(None, Request::Interpret { command: "hi".into(), history: vec![] }).unwrap_err();
        assert_eq!(err, "This phone cannot interpret voice commands.");
    }
```

If the test module has no `test_hub()` helper, build the hub the way `mobile/tests/round_trip.rs` does (`Hub::new(dir, hub::load_settings(&dir, "Pixel"), Arc::new(LogAlerts), Arc::new(NullSink), …)`) and name that helper `test_hub` so Task 7 can reuse it.

- [ ] **Step 12: Run everything that touches the protocol**

Run: `cargo test -p maya-core && cargo test -p maya-mobile && cargo test -p maya`
Expected: PASS.

- [ ] **Step 13: Commit**

```bash
git add core/src/net core/src/lib.rs mobile/src/hub.rs src-tauri/src/net_app.rs
git commit -m "feat: voice requests and a voice cap in the protocol

Interpret carries a spoken command and the phone's history, Execute a
confirmed action. A main names what it can do in welcome's caps, so a
phone knows before offering the mic; an older main sends none.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Desktop: interpreting text without the listener

**Files:**
- Modify: `src-tauri/src/listener.rs:237-252` (`recent_exchanges`), `:300-426` (`interpret`), tests module (~line 1000)

**Interfaces:**
- Consumes: `maya_core::interpreter::{run, validate, needs_confirm, Reply, RunError, TIMEOUT}`, `wake::Pending`.
- Produces:

```rust
pub(crate) enum Interpreted { Say(String), Confirm(wake::Pending), Run(serde_json::Value) }
pub(crate) struct VoiceContext { pub cards: Vec<Card>, pub dirs: Vec<String>, pub machines: Vec<String>, pub prs: Vec<ReviewPr> }
pub(crate) fn voice_context(app: &AppHandle) -> VoiceContext
pub(crate) fn decide(reply: &interpreter::Reply, ctx: &VoiceContext) -> Interpreted
pub(crate) fn exchanges<'a>(turns: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)>
pub(crate) fn interpret_text(app: &AppHandle, cmd: &str, history: &[(String, String)]) -> Interpreted
pub(crate) fn execute_action(app: &AppHandle, action: &serde_json::Value) -> Result<String, String>  // was private
```

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/listener.rs` tests:

```rust
    use maya_core::model::{AwaitKind, Awaiting, Card, Harness, State};

    fn card(id: &str, name: &str, ask: &str) -> Card {
        Card {
            session_id: id.into(), pid: 1, name: name.into(), cwd: format!("/Users/x/dev/{name}"), state: State::Awaiting, state_since: 42, snippet: "".into(),
            awaiting: Some(Awaiting { kind: AwaitKind::Text, detail: ask.into(), questions: vec![] }),
            has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false, model: None,
        }
    }

    fn ctx() -> VoiceContext {
        VoiceContext { cards: vec![card("s1", "hexgrid", "Ship it?")], dirs: vec![], machines: vec![], prs: vec![] }
    }

    fn reply(say: &str, action: Option<serde_json::Value>) -> interpreter::Reply {
        interpreter::Reply { say: say.into(), action, confirm: false }
    }

    #[test]
    fn a_report_is_only_said() {
        assert_eq!(decide(&reply("Two sessions need you.", None), &ctx()), Interpreted::Say("Two sessions need you.".into()));
        assert_eq!(decide(&reply("All quiet.", Some(serde_json::json!({"kind": "report"}))), &ctx()), Interpreted::Say("All quiet.".into()));
    }

    #[test]
    fn a_reply_waits_for_a_yes_with_the_main_read_back() {
        let r = reply("Sure!", Some(serde_json::json!({"kind": "reply", "session": "hexgrid", "text": "ship it"})));
        let Interpreted::Confirm(p) = decide(&r, &ctx()) else { panic!("expected a confirmation") };
        assert_eq!(p.say, "Telling hexgrid: Ship it. Yes?");
        assert_eq!(p.action["session"], "s1");
    }

    #[test]
    fn compact_runs_at_once() {
        let r = reply("Compacting.", Some(serde_json::json!({"kind": "compact", "session": "hexgrid"})));
        assert!(matches!(decide(&r, &ctx()), Interpreted::Run(a) if a["kind"] == "compact"));
    }

    #[test]
    fn an_action_on_nothing_says_why() {
        let r = reply("Done.", Some(serde_json::json!({"kind": "reply", "session": "nope", "text": "x"})));
        assert!(matches!(decide(&r, &ctx()), Interpreted::Say(_)));
    }

    #[test]
    fn exchanges_pair_the_last_twelve_turns_into_at_most_six() {
        let turns: Vec<(String, String)> = (0..20).flat_map(|i| [("user".to_string(), format!("u{i}")), ("maya".to_string(), format!("m{i}"))]).collect();
        let pairs = exchanges(turns.iter().map(|(w, t)| (w.as_str(), t.as_str())));
        assert_eq!(pairs.len(), 6);
        assert_eq!(pairs.last().unwrap(), &("u19".to_string(), "m19".to_string()));
        assert_eq!(pairs.first().unwrap(), &("u14".to_string(), "m14".to_string()));
    }
```

The exact read-back string comes from `spoken_for` and `sentence`; if `sentence("ship it")` produces different capitalisation or punctuation, change the expected string in the test to what `spoken_for` returns — the point of the test is that the read-back is `spoken_for`'s and not "Sure!".

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya listener`
Expected: FAIL to compile (`decide`, `Interpreted`, `VoiceContext`, `exchanges` missing).

- [ ] **Step 3: Add the pure pieces**

In `src-tauri/src/listener.rs`, before `recent_exchanges`:

```rust
/// What a spoken command comes to before anything is said or run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Interpreted {
    /// Say this; nothing runs.
    Say(String),
    /// Read this back, and run its action on a yes.
    Confirm(wake::Pending),
    /// Run this at once (focus, compact) and say what it did.
    Run(serde_json::Value),
}

/// What the interpreter sees and checks an action against.
pub(crate) struct VoiceContext {
    pub cards: Vec<maya_core::model::Card>,
    pub dirs: Vec<String>,
    pub machines: Vec<String>,
    pub prs: Vec<maya_core::reviews::ReviewPr>,
}

/// The board, the project dirs (a remote one as "<dir> (on <machine>)"),
/// the connected machines and the PRs, as the page shows them now.
pub(crate) fn voice_context(app: &AppHandle) -> VoiceContext {
    let state = app.state::<AppState>();
    let mut dirs = {
        let store = state.store.lock().unwrap();
        store.config.projects_dir_path().map(|r| launch::list_project_dirs(&r)).unwrap_or_default()
    };
    // Taken after `store` is released (lock order: `store` then `network`).
    let cards = crate::merged_cards(&state);
    let mut machines = vec![];
    for board in crate::remote_boards(&state).into_iter().filter(|b| b.connected) {
        for d in &board.dirs {
            dirs.push(format!("{d} (on {})", board.machine));
        }
        machines.push(board.machine);
    }
    let prs = state.reviews.lock().unwrap().prs.clone();
    VoiceContext { cards, dirs, machines, prs }
}

/// The interpreter's reply as one of the three outcomes. The read-back for
/// a confirmation is what will run (`spoken_for`), never the model's `say`.
pub(crate) fn decide(reply: &interpreter::Reply, ctx: &VoiceContext) -> Interpreted {
    let Some(proposed) = reply.action.as_ref().filter(|a| a["kind"] != "report") else {
        return Interpreted::Say(reply.say.clone());
    };
    match interpreter::validate(proposed, &ctx.cards, &ctx.dirs, &ctx.machines, &ctx.prs) {
        Err(why) => Interpreted::Say(why),
        Ok(action) if interpreter::needs_confirm(&action) => {
            let say = spoken_for(&action).unwrap_or_else(|| "Shall I?".into());
            Interpreted::Confirm(wake::Pending { say, action })
        }
        Ok(action) => Interpreted::Run(action),
    }
}

/// (user, maya) pairs from the last 12 turns, at most 6, oldest first.
pub(crate) fn exchanges<'a>(turns: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, String)> {
    let turns: Vec<(&str, &str)> = turns.into_iter().collect();
    let mut pairs = Vec::new();
    let mut last_user: Option<&str> = None;
    for (who, text) in &turns[turns.len().saturating_sub(12)..] {
        if *who == "user" {
            last_user = Some(text);
        } else if let Some(u) = last_user.take() {
            pairs.push((u.to_string(), text.to_string()));
        }
    }
    let skip = pairs.len().saturating_sub(6);
    pairs.split_off(skip)
}

/// Asks Maya's agent what `cmd` means. Speaks nothing and stores nothing:
/// the desktop listener and a phone's request both act on the outcome.
pub(crate) fn interpret_text(app: &AppHandle, cmd: &str, history: &[(String, String)]) -> Interpreted {
    let (brain, model, maya_dir) = {
        let state = app.state::<AppState>();
        let store = state.store.lock().unwrap();
        (store.config.brain(), store.config.brain_model().map(String::from), store.claude_dir().join("maya"))
    };
    let ctx = voice_context(app);
    let Ok(binary) = launch::find_binary(brain) else {
        return Interpreted::Say(format!("I can't find the {} command.", launch::binary_name(brain)));
    };
    match interpreter::run(brain, &binary, model.as_deref(), cmd, &ctx.cards, &ctx.prs, history, &maya_dir, interpreter::TIMEOUT) {
        Ok(reply) => decide(&reply, &ctx),
        Err(interpreter::RunError::TimedOut) => Interpreted::Say("Sorry, that took too long.".into()),
        Err(_) => Interpreted::Say("Sorry, I didn't catch that.".into()),
    }
}
```

Replace the body of `recent_exchanges` with:

```rust
fn recent_exchanges(app: &AppHandle) -> Vec<(String, String)> {
    let state = app.state::<AppState>();
    let v = state.voice.lock().unwrap();
    exchanges(v.history.iter().map(|t| (t.who.as_str(), t.text.as_str())))
}
```

Make `execute_action` `pub(crate)`.

If `Reply` lacks `PartialEq`/`Clone` derives needed by the tests, or `interpreter::Reply` fields are private, add `#[derive(Debug, Clone, PartialEq)]` and keep fields `pub` in `core/src/interpreter.rs:12-17` (they are `pub` today).

- [ ] **Step 4: Rewire the desktop's `interpret`**

Replace the body of `fn interpret` from `let (dirs_local, brain, model, maya_dir) = {` down to (and including) the `match interpreter::validate(…) { … }` block with:

```rust
    let outcome = interpret_text(app, cmd, &recent_exchanges(app));
    if !is_current(app, generation) {
        log::line("listener", "run replaced while thinking; reply dropped");
        return;
    }
    match outcome {
        Interpreted::Say(s) => {
            log::line("action", "nothing to run");
            reply_then_idle(app, generation, &s, inbox)
        }
        Interpreted::Confirm(pending) => {
            let say = pending.say.clone();
            log::line("action", format!("needs confirmation: {}", pending.action));
            // … keep the existing block unchanged from
            // `// Stored before the read-back so a tap on the page can answer it.`
            // to the final `f.set_pending(pending, now_ms());` and its closing braces …
        }
        Interpreted::Run(action) => {
            // Focus and compact run at once and say what they did.
            log::line("action", format!("running now: {action}"));
            let said = match execute_action(app, &action) {
                Ok(s) => spoken_for(&action).unwrap_or(s),
                Err(e) => {
                    log::line("action", format!("failed: {e}"));
                    e
                }
            };
            reply_then_idle(app, generation, &said, inbox);
        }
    }
```

The `Confirm` arm's body is the existing code from the old `Ok(action) if interpreter::needs_confirm(&action) =>` arm, minus its first three lines (`let say = …`, the log line, `let pending = …`), which the arm's first two lines replace. Move that code across verbatim; do not rewrite it.

- [ ] **Step 5: Run the desktop tests**

Run: `cargo test -p maya`
Expected: PASS, including the listener's existing tests.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/listener.rs core/src/interpreter.rs
git commit -m "refactor: interpret a spoken command without the desktop listener

interpret_text gathers the board, asks Maya's agent and decides between
saying, confirming and running, with nothing spoken or stored, so a
phone's request can use it beside the desktop's own conversation.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Desktop: serve a phone's `Interpret` and `Execute`

**Files:**
- Modify: `src-tauri/src/net_app.rs:261-288` (`serve_request`) and its tests
- Modify: `src-tauri/src/lib.rs:181-218` (`impl Notify for TauriNetNotify`)

**Interfaces:**
- Consumes: `listener::{interpret_text, decide, Interpreted, voice_context, exchanges, execute_action, spoken_for}`, `protocol::{Request, VoiceTurn, CAP_VOICE}`.
- Produces:

```rust
pub(crate) fn interpret_data(outcome: &Interpreted) -> Value   // {say, pending?: {say, action}}
pub(crate) fn said_data(say: &str) -> Value                     // {say}
pub(crate) fn refuse_remote(action: &Value) -> Option<String>   // focus cannot run for a phone
pub(crate) fn revalidate(action: &Value, ctx: &VoiceContext) -> Result<Value, String>
```

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/net_app.rs` tests (create `#[cfg(test)] mod tests { use super::*; … }` if absent):

```rust
    use crate::listener::{Interpreted, VoiceContext};

    #[test]
    fn interpret_answers_say_and_a_pending_action() {
        let said = interpret_data(&Interpreted::Say("All quiet.".into()));
        assert_eq!(said, serde_json::json!({"say": "All quiet."}));
        let pending = maya_core::wake::Pending { say: "Answering hexgrid with \"Yes\". Yes?".into(), action: serde_json::json!({"kind": "answer"}) };
        let asked = interpret_data(&Interpreted::Confirm(pending));
        assert_eq!(asked["say"], "Answering hexgrid with \"Yes\". Yes?");
        assert_eq!(asked["pending"]["say"], asked["say"]);
        assert_eq!(asked["pending"]["action"]["kind"], "answer");
    }

    #[test]
    fn focus_cannot_run_for_a_phone() {
        assert!(refuse_remote(&serde_json::json!({"kind": "focus", "session": "s1"})).is_some());
        assert!(refuse_remote(&serde_json::json!({"kind": "compact", "session": "s1"})).is_none());
    }

    #[test]
    fn execute_revalidates_against_the_board_now() {
        let gone = VoiceContext { cards: vec![], dirs: vec![], machines: vec![], prs: vec![] };
        let err = revalidate(&serde_json::json!({"kind": "reply", "session": "s1", "name": "hexgrid", "text": "ship it"}), &gone).unwrap_err();
        assert!(!err.is_empty(), "a session that has gone fails with a reason");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya net_app`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

In `src-tauri/src/net_app.rs`:

```rust
use crate::listener::{self, Interpreted, VoiceContext};

/// What a phone's `Interpret` gets back: the line to say, and the action to
/// run on its yes when there is one.
pub(crate) fn interpret_data(outcome: &Interpreted) -> Value {
    match outcome {
        Interpreted::Say(s) => serde_json::json!({ "say": s }),
        Interpreted::Confirm(p) => serde_json::json!({ "say": p.say, "pending": { "say": p.say, "action": p.action } }),
        // Run is executed before answering; see `serve_voice`.
        Interpreted::Run(_) => serde_json::json!({ "say": "" }),
    }
}

pub(crate) fn said_data(say: &str) -> Value {
    serde_json::json!({ "say": say })
}

/// Why an action that runs at once cannot run for a phone: focus brings a
/// window forward on this machine, which the user holding the phone cannot see.
pub(crate) fn refuse_remote(action: &Value) -> Option<String> {
    (action["kind"] == "focus").then(|| "I can only bring a window forward when you're at the computer.".to_string())
}

/// A confirmed action checked again against the board as it is now: the
/// ask may have been answered at the desk while the phone waited for yes.
pub(crate) fn revalidate(action: &Value, ctx: &VoiceContext) -> Result<Value, String> {
    // `validate` resolves the session by the name the read-back used.
    let mut proposed = action.clone();
    if let Some(name) = action["name"].as_str() {
        proposed["session"] = Value::String(name.to_string());
    }
    let fresh = maya_core::interpreter::validate(&proposed, &ctx.cards, &ctx.dirs, &ctx.machines, &ctx.prs)?;
    // An answer must still be for the ask the user heard read back.
    if action["kind"] == "answer" && fresh["askId"] != action["askId"] {
        return Err("The question has changed; ask me again.".into());
    }
    Ok(fresh)
}

/// A phone's voice request: interpret (running a focus/compact at once), or
/// run a confirmed action. Nothing is spoken here, and the desktop's own
/// conversation is untouched.
fn serve_voice(app: &AppHandle, request: Request) -> Result<Option<Value>, String> {
    match request {
        Request::Interpret { command, history } => {
            let pairs = listener::exchanges(history.iter().map(|t| (t.who.as_str(), t.text.as_str())));
            let outcome = listener::interpret_text(app, &command, &pairs);
            if let Interpreted::Run(action) = &outcome {
                if let Some(why) = refuse_remote(action) {
                    return Ok(Some(said_data(&why)));
                }
                let said = listener::execute_action(app, action).map(|s| listener::spoken_for(action).unwrap_or(s)).unwrap_or_else(|e| e);
                return Ok(Some(said_data(&said)));
            }
            Ok(Some(interpret_data(&outcome)))
        }
        Request::Execute { action } => {
            let fresh = revalidate(&action, &listener::voice_context(app))?;
            let said = listener::execute_action(app, &fresh)?;
            Ok(Some(said_data(&said)))
        }
        _ => unreachable!("serve_voice is only called for voice requests"),
    }
}
```

In `serve_request`, replace the temporary arm from Task 2 with:

```rust
        voice @ (Request::Interpret { .. } | Request::Execute { .. }) => serve_voice(app, voice),
```

If `validate` does not accept a session *name* (check `core/src/interpreter.rs:320-335`: it "resolves the session … loosely"), keep `proposed["session"]` as the original `action["session"]` id instead; the test in Step 1 holds either way. Make `listener::spoken_for` reachable (`pub(crate)`, it is already).

In `src-tauri/src/lib.rs`, `impl Notify for TauriNetNotify`:

```rust
    fn main_caps(&self) -> Vec<String> {
        // Voice needs Maya's agent on this machine to interpret.
        let brain = self.app.state::<AppState>().store.lock().unwrap().config.brain();
        if launch::find_binary(brain).is_ok() {
            vec![net::protocol::CAP_VOICE.to_string()]
        } else {
            vec![]
        }
    }
```

- [ ] **Step 4: Run the desktop tests**

Run: `cargo test -p maya`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/net_app.rs src-tauri/src/lib.rs
git commit -m "feat: the desktop main interprets voice commands for a phone

Interpret runs the main's agent on the phone's words and history and
answers with what to say and the action awaiting a yes; Execute checks
a confirmed action against the board again before running it. The main
offers voice in welcome when it can find its agent.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Phone config and quiet notifications

**Files:**
- Modify: `core/src/config.rs:77-200` (field, default, test)
- Modify: `mobile/src/commands.rs:229-247` (`set_config`) and tests (line 325)
- Modify: `mobile/src/alerts.rs:28-35` (`Post`), `:136-140` (`push`), tests
- Modify: `mobile/src/android.rs:139-161` (channel, post)

**Interfaces:**
- Produces: `Config.wake_word: bool` (serde `wakeWord`, default false); `alerts::SPOKEN_CHANNEL = "spoken"`; `Post { …, spoken: Option<String>, quiet: bool }` (`spoken` = `notify::spoken_line(card)`, `quiet` false from `plan`).

- [ ] **Step 1: Write the failing tests**

In `core/src/config.rs` tests:

```rust
    #[test]
    fn the_wake_word_defaults_off_and_round_trips() {
        assert!(!Config::default().wake_word);
        let c: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30,"wakeWord":true}"#).unwrap();
        assert!(c.wake_word);
        let old: Config = serde_json::from_str(r#"{"completedTimeoutMinutes":30}"#).unwrap();
        assert!(!old.wake_word);
    }
```

In `mobile/src/alerts.rs` tests:

```rust
    #[test]
    fn a_post_carries_the_line_to_speak_and_starts_loud() {
        let mut posted = Posted::default();
        let c = card("s1", "hexgrid", State::Awaiting);
        let actions = plan(&[c.clone()], &[c], &[], &mut posted, Switches { awaiting: true, completed: true });
        let Action::Post(p) = &actions[0] else { panic!("expected a post") };
        assert_eq!(p.spoken.as_deref(), Some("hexgrid needs a decision"));
        assert!(!p.quiet);
        assert_eq!(p.channel_id(), "decisions");
        assert_eq!(Post { quiet: true, ..p.clone() }.channel_id(), SPOKEN_CHANNEL);
    }
```

Use the module's existing card helper; if its name or arguments differ, adapt the call, not the assertions.

In `mobile/src/commands.rs` tests, if the module tests `set_config`'s field mapping through a helper, add `speak_notifications` and `wake_word` to it; otherwise extract the closure into `fn apply_settings(c: &mut Config, from: &Config)` and test:

```rust
    #[test]
    fn settings_carry_speak_and_the_wake_word() {
        let mut c = Config::default();
        let from = Config { speak_notifications: false, wake_word: true, ..Config::default() };
        apply_settings(&mut c, &from);
        assert!(!c.speak_notifications);
        assert!(c.wake_word);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-core wake_word && cargo test -p maya-mobile`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`core/src/config.rs`, in `Config` after `speak_notifications`:

```rust
    /// The phone listens for "Maya" while its app is on screen. The desktop ignores it.
    #[serde(default)]
    pub wake_word: bool,
```

and `wake_word: false,` in `Default`.

`mobile/src/commands.rs`:

```rust
/// What the Network screen edits, from the page's config onto the saved one.
fn apply_settings(c: &mut Config, from: &Config) {
    if !from.network.name.trim().is_empty() {
        c.network.name = from.network.name.trim().to_string();
    }
    c.network.port = from.network.port;
    c.notify_on_awaiting = from.notify_on_awaiting;
    c.notify_on_completed = from.notify_on_completed;
    c.speak_notifications = from.speak_notifications;
    c.wake_word = from.wake_word;
}
```

and in `set_config`: `let saved = hub.update_config(|c| apply_settings(c, &config))?;`. Update its doc comment to "name, port, the notification switches and voice".

`mobile/src/alerts.rs`:

```rust
/// The channel for a notification the phone also speaks: no sound, so none
/// clashes with the speech (`silent()` is iOS-only in the plugin).
pub const SPOKEN_CHANNEL: &str = "spoken";
```

`Post` gains:

```rust
    /// What the phone says for it when it speaks events (`notify::spoken_line`).
    pub spoken: Option<String>,
    /// Posted on `SPOKEN_CHANNEL` because the phone is saying it aloud.
    pub quiet: bool,
```

```rust
impl Post {
    pub fn channel_id(&self) -> &'static str {
        if self.quiet { SPOKEN_CHANNEL } else { self.kind.channel_id() }
    }
}
```

In `push`: `spoken: maya_core::notify::spoken_line(c), quiet: false,`.

`mobile/src/android.rs`: in `create_channels`, add a third channel:

```rust
        n.create_channel(Channel::builder(crate::alerts::SPOKEN_CHANNEL, "Spoken").description("A session Maya just announced aloud").importance(Importance::Low).vibration(false).build()).map_err(|e| e.to_string())?;
```

(put it before the final `Finished` line and end that line with `?;` then `Ok(())`), and in `AndroidAlerts::post` use `.channel_id(p.channel_id())`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-core && cargo test -p maya-mobile`
Expected: PASS. Fix any other `Post { … }` literals the compiler reports by adding `spoken: None, quiet: false`.

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs mobile/src/commands.rs mobile/src/alerts.rs mobile/src/android.rs
git commit -m "feat: phone settings for voice, and a quiet channel for spoken events

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The phone's conversation (`mobile/src/voice.rs`)

**Files:**
- Create: `mobile/src/voice.rs`
- Modify: `mobile/src/lib.rs` (`mod voice;`)

**Interfaces:**
- Consumes: `maya_core::wake::{Flow, Effect, Pending}`, `protocol::{Request, VoiceTurn}`.
- Produces:

```rust
pub trait VoiceIo: Send + Sync {
    fn listen(&self, wake: bool);
    fn stop_listening(&self);
    fn speak(&self, line: &str);
    fn stop_speaking(&self);
}
pub type Ask = Arc<dyn Fn(Request) -> Result<Option<Value>, String> + Send + Sync>;
pub type Show = Arc<dyn Fn(&VoiceView) + Send + Sync>;
#[derive(Serialize, Clone, Debug, Default, PartialEq)] #[serde(rename_all = "camelCase")]
pub struct VoiceView { pub state: String, pub heard: String, pub said: String, pub pending: Option<String>, pub error: Option<String>, pub offer: Option<String>, pub note: Option<String> }
pub struct Voice { … }
impl Voice {
    pub fn new(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64) -> Arc<Voice>;
    pub fn set_offer(&self, offer: Option<String>);     // None = voice offered
    pub fn set_note(&self, note: Option<String>);       // shown while idle (an online recogniser)
    pub fn set_wake(&self, on: bool);
    pub fn tap(&self);
    pub fn partial(&self, text: &str);
    pub fn heard(&self, text: &str);
    pub fn ended(&self);                                 // the recogniser stopped with nothing heard
    pub fn failed(&self, message: &str, fatal: bool);
    pub fn spoken(&self, ok: bool);
    pub fn screen(&self, on: bool);
    pub fn confirm(&self, yes: bool);
    pub fn announce(&self, line: &str) -> bool;          // true: will be spoken (now or when idle)
    pub fn view(&self) -> VoiceView;
}
```

`state` values: `"off"` (off screen or not offered), `"idle"`, `"listening"`, `"thinking"`, `"awaiting-command"`, `"awaiting-confirm"`, `"speaking"`.

- [ ] **Step 1: Write the failing tests**

Create `mobile/src/voice.rs` with the tests module first (the implementation follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Mutex;

    // Per test thread: cargo runs tests in parallel, and `new_sync` calls
    // `now` on the test's own thread.
    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(1_000) };
    }
    fn now() -> u64 {
        NOW.with(|n| n.get())
    }
    fn advance(ms: u64) {
        NOW.with(|n| n.set(n.get() + ms));
    }

    #[derive(Default)]
    struct FakeIo {
        calls: Mutex<Vec<String>>,
    }
    impl FakeIo {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.calls.lock().unwrap())
        }
    }
    impl VoiceIo for FakeIo {
        fn listen(&self, wake: bool) {
            self.calls.lock().unwrap().push(if wake { "listen wake".into() } else { "listen".into() });
        }
        fn stop_listening(&self) {
            self.calls.lock().unwrap().push("stop listening".into());
        }
        fn speak(&self, line: &str) {
            self.calls.lock().unwrap().push(format!("say {line}"));
        }
        fn stop_speaking(&self) {
            self.calls.lock().unwrap().push("stop speaking".into());
        }
    }

    /// A voice whose requests answer with `answers` in order, synchronously.
    fn voice(answers: Vec<Result<Value, String>>) -> (Arc<Voice>, Arc<FakeIo>, Arc<Mutex<Vec<Request>>>) {
        let io = Arc::new(FakeIo::default());
        let asked = Arc::new(Mutex::new(vec![]));
        let queue = Arc::new(Mutex::new(answers));
        let (a, q) = (asked.clone(), queue.clone());
        let ask: Ask = Arc::new(move |r| {
            a.lock().unwrap().push(r);
            q.lock().unwrap().remove(0).map(Some)
        });
        let v = Voice::new_sync(io.clone(), ask, Arc::new(|_| {}), now);
        v.screen(true);
        v.set_offer(None);
        io.take();
        (v, io, asked)
    }

    #[test]
    fn a_tap_asks_the_main_and_speaks_the_answer() {
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Two sessions need you."}))]);
        v.tap();
        assert_eq!(io.take(), vec!["listen"]);
        v.heard("what needs me");
        assert!(matches!(&asked.lock().unwrap()[0], Request::Interpret { command, .. } if command == "what needs me"));
        assert_eq!(io.take(), vec!["say Two sessions need you."]);
        v.spoken(true);
        assert_eq!(v.view().state, "idle");
        assert_eq!(io.take(), Vec::<String>::new(), "no follow-up after a statement");
    }

    #[test]
    fn a_question_back_reopens_the_mic() {
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Which one?"}))]);
        v.tap();
        v.heard("answer it");
        io.take();
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
        assert_eq!(v.view().state, "awaiting-command");
    }

    #[test]
    fn a_read_back_then_yes_runs_the_action() {
        let action = serde_json::json!({"kind": "answer", "session": "s1", "askId": 42});
        let (v, io, asked) = voice(vec![
            Ok(serde_json::json!({"say": "Answering hexgrid with \"Yes\". Yes?", "pending": {"say": "Answering hexgrid with \"Yes\". Yes?", "action": action}})),
            Ok(serde_json::json!({"say": "Answered."})),
        ]);
        v.tap();
        v.heard("answer hexgrid yes");
        assert_eq!(io.take(), vec!["say Answering hexgrid with \"Yes\". Yes?"]);
        assert_eq!(v.view().pending.as_deref(), Some("Answering hexgrid with \"Yes\". Yes?"));
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
        v.heard("yes");
        assert!(matches!(&asked.lock().unwrap()[1], Request::Execute { action: a } if a["askId"] == 42));
        assert_eq!(io.take(), vec!["say Answered."]);
        assert_eq!(v.view().pending, None);
    }

    #[test]
    fn the_buttons_answer_a_read_back_like_words() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        v.confirm(false);
        assert_eq!(io.take(), vec!["stop listening", "say OK."]);
        assert_eq!(asked.lock().unwrap().len(), 1, "no means nothing runs");
    }

    #[test]
    fn no_answer_to_a_read_back_cancels() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        v.spoken(true);
        io.take();
        advance(maya_core::wake::CONFIRM_WAIT_MS + 1);
        v.ended();
        assert_eq!(io.take(), vec!["say OK."]);
        assert_eq!(v.view().pending, None);
        assert_eq!(asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn silence_after_a_tap_goes_idle() {
        let (v, io, asked) = voice(vec![]);
        v.tap();
        io.take();
        advance(maya_core::wake::COMMAND_WAIT_MS + 1);
        v.ended();
        assert_eq!(io.take(), Vec::<String>::new());
        assert_eq!(v.view().state, "idle");
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_wake_word_loops_and_answers_bare_maya_with_yes() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        assert_eq!(io.take(), vec!["listen wake"]);
        v.heard("nothing for me here");
        assert_eq!(io.take(), vec!["listen wake"], "chatter without the word restarts the loop");
        v.heard("Maya");
        assert_eq!(io.take(), vec!["say Yes?"]);
        v.spoken(true);
        assert_eq!(io.take(), vec!["listen"]);
    }

    #[test]
    fn leaving_the_screen_stops_everything_and_drops_a_pending_action() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        io.take();
        v.screen(false);
        assert_eq!(io.take(), vec!["stop listening", "stop speaking"]);
        assert_eq!(v.view().state, "off");
        assert_eq!(v.view().pending, None);
        v.screen(true);
        v.heard("yes");
        assert_eq!(io.take(), Vec::<String>::new(), "a yes after coming back confirms nothing");
    }

    #[test]
    fn a_late_answer_after_leaving_the_screen_is_dropped() {
        let (v, io, _) = voice(vec![]);
        v.tap();
        let gen = v.begin_request();
        v.screen(false);
        io.take();
        v.finish_interpret(gen, Ok(Some(serde_json::json!({"say": "Too late."}))));
        assert_eq!(io.take(), Vec::<String>::new());
    }

    #[test]
    fn an_event_waits_for_the_conversation() {
        let action = serde_json::json!({"kind": "reply", "session": "s1"});
        let (v, io, _) = voice(vec![Ok(serde_json::json!({"say": "Telling x: y. Yes?", "pending": {"say": "Telling x: y. Yes?", "action": action}}))]);
        v.tap();
        v.heard("tell x y");
        io.take();
        assert!(v.announce("laptop needs a decision"));
        assert_eq!(io.take(), Vec::<String>::new(), "not during a read-back");
        v.confirm(false);
        assert_eq!(io.take(), vec!["stop listening", "say OK."]);
        v.spoken(true);
        assert_eq!(io.take(), vec!["say laptop needs a decision"]);
    }

    #[test]
    fn an_event_off_screen_is_not_spoken() {
        let (v, io, _) = voice(vec![]);
        v.screen(false);
        io.take();
        assert!(!v.announce("laptop needs a decision"));
        assert_eq!(io.take(), Vec::<String>::new());
    }

    #[test]
    fn no_tap_without_the_voice_cap() {
        let (v, io, asked) = voice(vec![]);
        v.set_offer(Some("Update Yhi to talk to Maya.".into()));
        io.take();
        v.tap();
        assert_eq!(io.take(), Vec::<String>::new());
        assert_eq!(v.view().offer.as_deref(), Some("Update Yhi to talk to Maya."));
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_main_failing_resets_the_conversation_with_its_reason() {
        let (v, io, _) = voice(vec![Err("Yhi did not answer in time.".into())]);
        v.tap();
        v.heard("what needs me");
        assert_eq!(io.take(), vec!["say Yhi did not answer in time."]);
    }

    #[test]
    fn the_wake_loop_backs_off_then_gives_up() {
        let (v, io, _) = voice(vec![]);
        v.set_wake(true);
        io.take();
        for _ in 0..4 {
            v.failed("busy", false);
        }
        assert_eq!(v.view().error, None);
        v.failed("busy", false);
        assert_eq!(v.view().error.as_deref(), Some("Wake word stopped: busy"));
        assert_eq!(v.backoffs(), vec![1_000, 2_000, 4_000, 8_000]);
    }

    #[test]
    fn the_note_shows_in_the_view() {
        let (v, _, _) = voice(vec![]);
        v.set_note(Some("Speech goes through Google's service and may leave the phone.".into()));
        assert_eq!(v.view().note.as_deref(), Some("Speech goes through Google's service and may leave the phone."));
    }

    #[test]
    fn history_keeps_the_last_twelve_turns() {
        let answers = (0..10).map(|i| Ok(serde_json::json!({"say": format!("a{i}")}))).collect();
        let (v, _, asked) = voice(answers);
        for i in 0..10 {
            v.tap();
            v.heard(&format!("q{i}"));
            v.spoken(true);
        }
        let Request::Interpret { history, .. } = &asked.lock().unwrap()[9] else { panic!() };
        assert_eq!(history.len(), 12);
        assert_eq!(history.last().unwrap().text, "a8");
    }
}
```

`new_sync` runs requests on the calling thread (for tests); `new` runs them on a spawned thread. `begin_request`, `finish_interpret`, and `backoffs` are test seams: `begin_request` returns the generation a request would carry, `finish_interpret` applies an answer, and `backoffs` returns the delays scheduled so far (the real build sleeps them on a thread before calling `io.listen(true)`).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-mobile voice`
Expected: FAIL to compile (`Voice` missing).

- [ ] **Step 3: Implement**

Put above the tests in `mobile/src/voice.rs`:

```rust
//! The phone's voice conversation: the shared wake-word `Flow`, the turns
//! sent with each command, and what to say and when to listen. The main
//! interprets (`Request::Interpret`) and runs (`Request::Execute`); the
//! recogniser and the voice sit behind `VoiceIo` so this is tested on the
//! host. Nothing here listens or speaks while the app is off screen.

use maya_core::log;
use maya_core::net::protocol::{Request, VoiceTurn};
use maya_core::wake::{Effect, Flow, Pending};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// The recogniser and the voice: Android's, or a fake in tests.
pub trait VoiceIo: Send + Sync {
    /// One utterance; results come back through `Voice::heard`/`ended`/`failed`.
    /// `wake` asks for the on-device recogniser the wake word loop needs.
    fn listen(&self, wake: bool);
    fn stop_listening(&self);
    /// Says `line`; `Voice::spoken` follows when it is done.
    fn speak(&self, line: &str);
    fn stop_speaking(&self);
}

pub type Ask = Arc<dyn Fn(Request) -> Result<Option<Value>, String> + Send + Sync>;
pub type Show = Arc<dyn Fn(&VoiceView) + Send + Sync>;

/// What the page shows.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub state: String,
    pub heard: String,
    pub said: String,
    /// The read-back waiting for yes or no.
    pub pending: Option<String>,
    pub error: Option<String>,
    /// Why voice is not offered; `None` when it is.
    pub offer: Option<String>,
    /// Shown while idle: tap-to-talk goes through an online recogniser.
    pub note: Option<String>,
}

/// Turns sent with each command (the interpreter pairs at most 6 of them).
const HISTORY: usize = 12;
/// Wake word failures in a row before the loop stops.
const MAX_FAILURES: u32 = 5;
/// The offer until the hub says otherwise: not joined to a main.
pub const NO_MAIN: &str = "Voice needs a desktop main.";

/// What to do once the current line has been spoken.
#[derive(Clone, Debug, PartialEq)]
enum After {
    Idle,
    /// Listen for a command (after "Yes?" or a question back).
    Command,
    /// Restart the confirmation window and listen for yes or no.
    Confirm(Pending),
}

struct Inner {
    flow: Flow,
    history: Vec<VoiceTurn>,
    on_screen: bool,
    wake: bool,
    offer: Option<String>,
    speaking: bool,
    thinking: bool,
    after: After,
    events: VecDeque<String>,
    generation: u64,
    failures: u32,
    backoffs: Vec<u64>,
    view: VoiceView,
}

pub struct Voice {
    inner: Mutex<Inner>,
    io: Arc<dyn VoiceIo>,
    ask: Ask,
    show: Show,
    now: fn() -> u64,
    /// Run requests on the calling thread (tests) instead of a spawned one.
    sync: bool,
    me: Mutex<std::sync::Weak<Voice>>,
}

impl Voice {
    pub fn new(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64) -> Arc<Voice> {
        Self::build(io, ask, show, now, false)
    }

    #[cfg(test)]
    fn new_sync(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64) -> Arc<Voice> {
        Self::build(io, ask, show, now, true)
    }

    fn build(io: Arc<dyn VoiceIo>, ask: Ask, show: Show, now: fn() -> u64, sync: bool) -> Arc<Voice> {
        let inner = Inner {
            flow: Flow::new(), history: vec![], on_screen: false, wake: false, offer: Some(NO_MAIN.into()), speaking: false, thinking: false,
            after: After::Idle, events: VecDeque::new(), generation: 0, failures: 0, backoffs: vec![], view: VoiceView { state: "off".into(), ..Default::default() },
        };
        let v = Arc::new(Voice { inner: Mutex::new(inner), io, ask, show, now, sync, me: Mutex::new(std::sync::Weak::new()) });
        *v.me.lock().unwrap() = Arc::downgrade(&v);
        v
    }

    pub fn view(&self) -> VoiceView {
        self.inner.lock().unwrap().view.clone()
    }

    fn active(i: &Inner) -> bool {
        i.on_screen && i.offer.is_none()
    }

    /// Recomputes the view's state and shows it.
    fn publish(&self, i: &mut Inner) {
        let now = (self.now)();
        i.view.state = if !Self::active(i) {
            "off"
        } else if i.speaking {
            "speaking"
        } else if i.thinking {
            "thinking"
        } else {
            match i.flow.state(now) {
                "idle" if i.wake => "listening",
                s => s,
            }
        }
        .to_string();
        i.view.pending = if i.flow.has_pending() { i.view.pending.clone() } else { None };
        i.view.offer = i.offer.clone();
        (self.show)(&i.view);
    }

    pub fn set_offer(&self, offer: Option<String>) {
        let mut i = self.inner.lock().unwrap();
        if i.offer == offer {
            return;
        }
        i.offer = offer;
        if !Self::active(&i) {
            self.reset(&mut i);
        } else if i.wake {
            self.io.listen(true);
        }
        self.publish(&mut i);
    }

    pub fn set_note(&self, note: Option<String>) {
        let mut i = self.inner.lock().unwrap();
        i.view.note = note;
        self.publish(&mut i);
    }

    pub fn set_wake(&self, on: bool) {
        let mut i = self.inner.lock().unwrap();
        i.wake = on;
        i.failures = 0;
        i.view.error = None;
        if Self::active(&i) && !i.speaking && !i.thinking {
            if on {
                self.io.listen(true);
            } else {
                self.io.stop_listening();
            }
        }
        self.publish(&mut i);
    }

    /// Off screen, or voice withdrawn: stop at once and forget the turn.
    fn reset(&self, i: &mut Inner) {
        i.generation += 1;
        i.flow = Flow::new();
        i.speaking = false;
        i.thinking = false;
        i.after = After::Idle;
        i.events.clear();
        i.view.pending = None;
        self.io.stop_listening();
        self.io.stop_speaking();
    }

    pub fn screen(&self, on: bool) {
        let mut i = self.inner.lock().unwrap();
        if i.on_screen == on {
            return;
        }
        i.on_screen = on;
        if !on {
            self.reset(&mut i);
        } else if Self::active(&i) && i.wake {
            self.io.listen(true);
        }
        self.publish(&mut i);
    }

    pub fn tap(&self) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        if i.speaking {
            self.io.stop_speaking();
            i.speaking = false;
        }
        i.flow.await_reply((self.now)());
        i.view.heard.clear();
        self.io.listen(false);
        self.publish(&mut i);
    }

    pub fn partial(&self, text: &str) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        i.view.heard = text.to_string();
        self.publish(&mut i);
    }

    pub fn confirm(&self, yes: bool) {
        {
            let i = self.inner.lock().unwrap();
            if !i.flow.has_pending() {
                return;
            }
        }
        self.io.stop_listening();
        self.heard(if yes { "yes" } else { "no" });
    }

    pub fn heard(&self, text: &str) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) || i.thinking {
            return;
        }
        i.failures = 0;
        i.view.heard = text.to_string();
        let effects = i.flow.on_segment(text, (self.now)());
        if effects.is_empty() {
            // Chatter, or Maya's own line: keep listening in the wake loop.
            if i.wake && !i.speaking {
                self.io.listen(true);
            }
            self.publish(&mut i);
            return;
        }
        for e in effects {
            match e {
                Effect::Say(s) => self.say(&mut i, &s, After::Command),
                Effect::Interpret(cmd) => {
                    // The history sent is what came before this command.
                    let history = last(&i.history);
                    push(&mut i.history, "user", &cmd);
                    self.request(i, Request::Interpret { command: cmd, history });
                    return;
                }
                Effect::Execute(action) => {
                    push(&mut i.history, "user", "yes");
                    i.view.pending = None;
                    self.request(i, Request::Execute { action });
                    return;
                }
                Effect::Cancelled => {
                    push(&mut i.history, "user", "no");
                    i.view.pending = None;
                    self.say(&mut i, "OK.", After::Idle);
                }
            }
        }
        self.publish(&mut i);
    }

    pub fn ended(&self) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) || i.speaking || i.thinking {
            return;
        }
        let now = (self.now)();
        if i.flow.drop_expired(now) {
            i.view.pending = None;
            self.say(&mut i, "OK.", After::Idle);
        } else if i.flow.state(now) != "idle" || i.wake {
            // Still inside a window, or the wake loop: listen again.
            self.io.listen(i.wake && i.flow.state(now) == "idle");
        }
        self.publish(&mut i);
    }

    pub fn failed(&self, message: &str, fatal: bool) {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return;
        }
        if fatal || !i.wake {
            i.view.error = Some(message.to_string());
            i.flow = Flow::new();
            self.publish(&mut i);
            return;
        }
        i.failures += 1;
        if i.failures >= MAX_FAILURES {
            i.view.error = Some(format!("Wake word stopped: {message}"));
            self.publish(&mut i);
            return;
        }
        let delay = 1_000u64 << (i.failures - 1);
        i.backoffs.push(delay);
        let generation = i.generation;
        drop(i);
        if !self.sync {
            let me = self.me.lock().unwrap().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(delay));
                if let Some(v) = me.upgrade() {
                    let i = v.inner.lock().unwrap();
                    if i.generation == generation && Self::active(&i) && i.wake && !i.speaking && !i.thinking {
                        v.io.listen(true);
                    }
                }
            });
        }
    }

    #[cfg(test)]
    fn backoffs(&self) -> Vec<u64> {
        self.inner.lock().unwrap().backoffs.clone()
    }

    pub fn spoken(&self, ok: bool) {
        let mut i = self.inner.lock().unwrap();
        if !ok {
            i.view.error = Some("No text-to-speech voice is installed; replies show here instead.".into());
        }
        if !i.speaking {
            return;
        }
        i.speaking = false;
        if !Self::active(&i) {
            return;
        }
        let now = (self.now)();
        match std::mem::replace(&mut i.after, After::Idle) {
            After::Confirm(p) => {
                i.flow.set_pending(p, now);
                self.io.listen(false);
            }
            After::Command => {
                i.flow.await_reply(now);
                self.io.listen(false);
            }
            After::Idle => {
                if let Some(line) = i.events.pop_front() {
                    self.say(&mut i, &line, After::Idle);
                } else if i.wake {
                    self.io.listen(true);
                }
            }
        }
        self.publish(&mut i);
    }

    /// Speaks a board event now, or once the conversation is idle. False
    /// when it will not be spoken at all (off screen, voice not offered).
    pub fn announce(&self, line: &str) -> bool {
        let mut i = self.inner.lock().unwrap();
        if !Self::active(&i) {
            return false;
        }
        let busy = i.speaking || i.thinking || i.flow.state((self.now)()) != "idle";
        if busy {
            i.events.push_back(line.to_string());
        } else {
            // Speaking cancels a running recognition on the Android side.
            self.say(&mut i, line, After::Idle);
            self.publish(&mut i);
        }
        true
    }

    fn say(&self, i: &mut Inner, line: &str, after: After) {
        i.flow.ignore_line(line);
        i.view.said = line.to_string();
        i.speaking = true;
        i.after = after;
        self.io.speak(line);
    }

    /// The generation a request started now carries; a test seam too.
    fn begin_request(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }

    /// Sends `request` (with the history) and acts on the answer, on a
    /// thread unless `sync`. The lock is released while the main thinks.
    fn request(&self, mut i: std::sync::MutexGuard<'_, Inner>, request: Request) {
        let execute = matches!(request, Request::Execute { .. });
        // The recogniser has stopped: it hears one utterance per `listen`.
        i.thinking = true;
        let generation = i.generation;
        self.publish(&mut i);
        drop(i);
        let ask = self.ask.clone();
        let me = self.me.lock().unwrap().clone();
        let run = move || {
            let answer = ask(request);
            if let Some(v) = me.upgrade() {
                if execute {
                    v.finish_execute(generation, answer);
                } else {
                    v.finish_interpret(generation, answer);
                }
            }
        };
        if self.sync {
            run();
        } else {
            std::thread::spawn(run);
        }
    }

    fn finish_interpret(&self, generation: u64, answer: Result<Option<Value>, String>) {
        let mut i = self.inner.lock().unwrap();
        if i.generation != generation {
            log::line("voice", "an answer arrived after the conversation ended; dropped");
            return;
        }
        i.thinking = false;
        let data = match answer {
            Ok(Some(d)) => d,
            Ok(None) => Value::Null,
            Err(e) => {
                i.flow = Flow::new();
                self.say(&mut i, &e, After::Idle);
                self.publish(&mut i);
                return;
            }
        };
        let say = data["say"].as_str().unwrap_or("").to_string();
        push(&mut i.history, "maya", &say);
        let pending = data.get("pending").filter(|p| !p.is_null()).map(|p| Pending { say: p["say"].as_str().unwrap_or(&say).to_string(), action: p["action"].clone() });
        match pending {
            Some(p) => {
                i.flow.set_pending(p.clone(), (self.now)());
                i.view.pending = Some(p.say.clone());
                self.say(&mut i, &p.say, After::Confirm(p));
            }
            None if say.trim_end().ends_with('?') => self.say(&mut i, &say, After::Command),
            None if say.trim().is_empty() => {}
            None => self.say(&mut i, &say, After::Idle),
        }
        self.publish(&mut i);
    }

    fn finish_execute(&self, generation: u64, answer: Result<Option<Value>, String>) {
        let mut i = self.inner.lock().unwrap();
        if i.generation != generation {
            return;
        }
        i.thinking = false;
        let say = match answer {
            Ok(d) => d.and_then(|d| d["say"].as_str().map(str::to_string)).unwrap_or_else(|| "Done.".into()),
            Err(e) => e,
        };
        push(&mut i.history, "maya", &say);
        self.say(&mut i, &say, After::Idle);
        self.publish(&mut i);
    }
}

fn push(history: &mut Vec<VoiceTurn>, who: &str, text: &str) {
    history.push(VoiceTurn { who: who.into(), text: text.into() });
    let extra = history.len().saturating_sub(HISTORY * 2);
    history.drain(..extra);
}

fn last(history: &[VoiceTurn]) -> Vec<VoiceTurn> {
    history[history.len().saturating_sub(HISTORY)..].to_vec()
}
```

`MutexGuard` is not `Send`; `request` drops it before spawning, so the closure captures only `ask`, `me`, `request`, `generation`, `execute`.

In `mobile/src/lib.rs` add `mod voice;` (or `pub mod voice;` if `hub.rs` uses it via `crate::voice`).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya-mobile voice`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mobile/src/voice.rs mobile/src/lib.rs
git commit -m "feat: the phone's voice conversation

Tap-to-talk and the wake word over the shared Flow; commands go to the
main with the last 12 turns, read-backs wait for yes or no, and events
wait until the conversation is idle. Leaving the screen stops it and
drops anything pending.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Wire the voice into the hub, the commands and Android

**Files:**
- Modify: `mobile/src/hub.rs` (field, `voice_offer`, `refresh`)
- Modify: `mobile/src/android.rs` (`voice` plugin, `AndroidVoice`, `quiet`)
- Modify: `mobile/src/commands.rs` (commands)
- Modify: `mobile/src/lib.rs` (registration)

**Interfaces:**
- Consumes: Task 6's `Voice`, `VoiceIo`, `VoiceView`; Task 5's `Post.spoken`/`quiet`; Task 2's `AssistantLink.voice`.
- Produces: Tauri commands `voice_view() -> VoiceView`, `voice_tap()`, `voice_confirm(yes)`; event `voice` (payload `VoiceView`). `voice::dispatch(&Voice, &Value)` routes one plugin event. Plugin `voice` with Kotlin commands `watch{channel}`, `listen{wake}`, `stopListening`, `speak{line}`, `stopSpeaking`, `quiet -> {value}`, `state -> {onScreen, available, onDevice}`, and the built-in `checkPermissions`/`requestPermissions` (alias `microphone`).
- The plugin's events (`partial{text}`, `final{text}`, `ended`, `failed{message, fatal}`, `spoken{ok}`, `screen{on}`, each as `{event, …}`) reach Rust through a `tauri::ipc::Channel` handed to `watch`; the page never calls the plugin.
- `Hub::voice_offer(&self) -> Option<String>`: `None` when voice is offered.

- [ ] **Step 1: Write the failing tests**

In `mobile/src/voice.rs` tests:

```rust
    #[test]
    fn plugin_events_reach_the_conversation() {
        let (v, io, asked) = voice(vec![Ok(serde_json::json!({"say": "Hi."}))]);
        v.tap();
        io.take();
        dispatch(&v, &serde_json::json!({"event": "partial", "text": "what ne"}));
        assert_eq!(v.view().heard, "what ne");
        dispatch(&v, &serde_json::json!({"event": "final", "text": "what needs me"}));
        assert_eq!(asked.lock().unwrap().len(), 1);
        dispatch(&v, &serde_json::json!({"event": "spoken", "ok": true}));
        assert_eq!(v.view().state, "idle");
        dispatch(&v, &serde_json::json!({"event": "screen", "on": false}));
        assert_eq!(v.view().state, "off");
        dispatch(&v, &serde_json::json!({"event": "nonsense"}));
    }
```

In `mobile/src/hub.rs` tests:

In `mobile/src/hub.rs` tests:

```rust
    #[test]
    fn voice_is_offered_only_when_joined_to_a_main_that_interprets() {
        let hub = test_hub();
        assert_eq!(hub.voice_offer().as_deref(), Some("Voice needs a desktop main."));
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: false, ..Default::default() }, true);
        assert_eq!(hub.voice_offer().as_deref(), Some("Update Yhi to talk to Maya."));
        hub.set_link_for_test(AssistantLink { connected: true, main_name: Some("Yhi".into()), mirror: Some(MirrorAccess::Allowed), voice: true, ..Default::default() }, true);
        assert_eq!(hub.voice_offer(), None);
    }

    #[test]
    fn events_are_spoken_only_with_speak_on_and_the_phone_not_silenced() {
        assert!(speaks_events(true, false));
        assert!(!speaks_events(false, false), "Speak off");
        assert!(!speaks_events(true, true), "silent, vibrate or Do Not Disturb");
    }
```

`set_link_for_test(link, joined)` sets `self.link` and makes `joined()` answer `joined` (add a `#[cfg(test)] joined_override: Mutex<Option<bool>>` consulted by `joined()` under `cfg(test)`), so the test needs no running client.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya-mobile hub`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the hub side**

In `mobile/src/hub.rs`:

```rust
    /// Why voice is not offered, or `None` when it is: the phone must have
    /// joined a main that interprets (`CAP_VOICE`) and lets it control sessions.
    pub fn voice_offer(&self) -> Option<String> {
        if !self.joined() {
            return Some("Voice needs a desktop main.".into());
        }
        let link = self.link.lock().unwrap();
        let main = link.main_name.clone().unwrap_or_else(|| "the main Maya".into());
        if link.mirror != Some(MirrorAccess::Allowed) {
            return Some(format!("{main} has not allowed this phone to control its sessions."));
        }
        if !link.voice {
            return Some(format!("Update {main} to talk to Maya."));
        }
        None
    }
```

```rust
/// Whether board events may be spoken at all: Speak is on and the phone is
/// not on silent, vibrate or Do Not Disturb. Replies are always spoken.
pub(crate) fn speaks_events(speak_on: bool, silenced: bool) -> bool {
    speak_on && !silenced
}
```

Give `Hub` two fields, both empty in `Hub::new` (`OnceLock::new()`) and set once in `lib.rs`'s `setup`, so no caller of `Hub::new` changes:

```rust
    /// The phone's voice conversation, once the app has made it.
    pub voice: OnceLock<Arc<crate::voice::Voice>>,
    /// On silent, vibrate or Do Not Disturb (the voice plugin's `quiet`).
    pub silenced: OnceLock<Box<dyn Fn() -> bool + Send + Sync>>,
```

In `refresh`, replace the `for a in actions` loop with:

```rust
        let speak_on = self.settings.lock().unwrap().config.speak_notifications;
        // Asked only when there is something to post: it is a call into Android.
        let mut events_spoken = None;
        for a in actions {
            match a {
                Action::Post(mut p) => {
                    let may_speak = *events_spoken.get_or_insert_with(|| speaks_events(speak_on, self.silenced.get().is_some_and(|f| f())));
                    if let (true, Some(v), Some(line)) = (may_speak, self.voice.get(), p.spoken.as_deref()) {
                        // Spoken (now or once the conversation is idle): post without a sound.
                        p.quiet = v.announce(line);
                    }
                    self.alerts.post(&p)
                }
                Action::Clear(id) => self.alerts.clear(id),
            }
        }
        if let Some(v) = self.voice.get() {
            v.set_offer(self.voice_offer());
        }
```

(`announce` is only called when Speak is on and the phone is not silenced, so a silenced phone never queues an event.)

In `HubClientNotify`, after `main_caps`, `mirror_offer` and `disconnected` change the link, call `h.refresh()` (they already do or call `self.refresh()`), so `set_offer` follows.

- [ ] **Step 4: Route plugin events**

In `mobile/src/voice.rs`:

```rust
/// One event from the voice plugin (`{event, …}`) onto the conversation.
pub fn dispatch(v: &Voice, e: &Value) {
    let text = || e["text"].as_str().unwrap_or("");
    match e["event"].as_str().unwrap_or("") {
        "partial" => v.partial(text()),
        "final" => v.heard(text()),
        "ended" => v.ended(),
        "failed" => v.failed(e["message"].as_str().unwrap_or("speech recognition failed"), e["fatal"].as_bool().unwrap_or(false)),
        "spoken" => v.spoken(e["ok"].as_bool().unwrap_or(true)),
        "screen" => v.screen(e["on"].as_bool().unwrap_or(false)),
        other => log::line("voice", format!("ignored plugin event {other:?}")),
    }
}
```

- [ ] **Step 5: Implement the Android bridge**

In `mobile/src/android.rs`, add a second plugin and the `VoiceIo`:

```rust
#[derive(Serialize)]
struct ListenArgs {
    wake: bool,
}

/// The Kotlin `VoicePlugin`, once registered.
#[cfg(target_os = "android")]
struct VoiceHandle<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// The voice plugin: Android's recogniser and voice, the on-screen state
/// and the quiet state. Its events reach the page, which hands them to the
/// `voice_*` commands.
pub fn voice_init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("voice")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("com.dosaki.maya.mobile", "VoicePlugin")?;
                app.manage(VoiceHandle(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}
```

Inside `mod imp` (Android):

```rust
    fn voice_call<R: Runtime>(app: &AppHandle<R>, command: &str, args: impl Serialize) {
        if let Err(e) = app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<serde_json::Value>(command, args) {
            log::line("voice", format!("{command}: {e}"));
        }
    }

    pub struct AndroidVoice<R: Runtime> {
        pub app: AppHandle<R>,
    }

    impl<R: Runtime> crate::voice::VoiceIo for AndroidVoice<R> {
        fn listen(&self, wake: bool) {
            voice_call(&self.app, "listen", ListenArgs { wake })
        }
        fn stop_listening(&self) {
            voice_call(&self.app, "stopListening", ())
        }
        fn speak(&self, line: &str) {
            voice_call(&self.app, "speak", LineArgs { line })
        }
        fn stop_speaking(&self) {
            voice_call(&self.app, "stopSpeaking", ())
        }
    }

    /// On silent or vibrate, or in Do Not Disturb.
    pub fn silenced<R: Runtime>(app: &AppHandle<R>) -> bool {
        app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<BoolReply>("quiet", ()).map(|r| r.value).unwrap_or(false)
    }

    pub fn voice_io<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn crate::voice::VoiceIo> {
        Arc::new(AndroidVoice { app: app.clone() })
    }

    #[derive(Serialize)]
    struct WatchArgs {
        channel: tauri::ipc::Channel<serde_json::Value>,
    }

    /// Hands the plugin a channel for its events, which go straight to the
    /// conversation. `Channel::new` registers itself with mobile plugins.
    pub fn watch_voice<R: Runtime>(app: &AppHandle<R>, voice: std::sync::Weak<crate::voice::Voice>) {
        let channel = tauri::ipc::Channel::<serde_json::Value>::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body {
                match (serde_json::from_str::<serde_json::Value>(&json), voice.upgrade()) {
                    (Ok(e), Some(v)) => crate::voice::dispatch(&v, &e),
                    (Err(e), _) => log::line("voice", format!("unreadable plugin event: {e}")),
                    _ => {}
                }
            }
            Ok(())
        });
        voice_call(app, "watch", WatchArgs { channel });
    }

    #[derive(Deserialize)]
    struct MicPermission {
        microphone: String,
    }

    #[derive(Serialize)]
    struct PermissionArgs {
        permissions: Vec<&'static str>,
    }

    /// Whether Maya may use the microphone, asking the user the first time.
    /// Blocks until they answer; call it from an async command.
    pub fn microphone_allowed<R: Runtime>(app: &AppHandle<R>) -> bool {
        let h = &app.state::<VoiceHandle<R>>().0;
        if h.run_mobile_plugin::<MicPermission>("checkPermissions", ()).is_ok_and(|p| p.microphone == "granted") {
            return true;
        }
        h.run_mobile_plugin::<MicPermission>("requestPermissions", PermissionArgs { permissions: vec!["microphone"] }).is_ok_and(|p| p.microphone == "granted")
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RecognizerState {
        on_device: bool,
    }

    /// The phone has an on-device recogniser (the wake word needs one).
    pub fn on_device<R: Runtime>(app: &AppHandle<R>) -> bool {
        app.state::<VoiceHandle<R>>().0.run_mobile_plugin::<RecognizerState>("state", ()).map(|s| s.on_device).unwrap_or(true)
    }
```

And in the host (`#[cfg(not(target_os = "android"))]`) module, the matching no-ops: `voice_io` returning a struct whose four methods `log::line("voice", …)`, `silenced` returning `false`, `watch_voice` doing nothing, `microphone_allowed` returning `true` and `on_device` returning `true`. Re-export all five the same way the file re-exports `alerts` and `device_model`.

- [ ] **Step 6: Add the commands**

In `mobile/src/commands.rs`:

```rust
fn voice(hub: &H) -> Result<Arc<crate::voice::Voice>, String> {
    hub.voice.get().cloned().ok_or_else(|| "Voice is not ready.".into())
}

/// The microphone was refused: the wake word turns off and the bar says why.
fn microphone_refused(hub: &H, v: &crate::voice::Voice) -> Result<(), String> {
    hub.update_config(|c| c.wake_word = false)?;
    v.set_wake(false);
    v.failed("Microphone access is off.", true);
    Ok(())
}

#[tauri::command]
pub fn voice_view(hub: H) -> Result<crate::voice::VoiceView, String> {
    let v = voice(&hub)?;
    v.set_offer(hub.voice_offer());
    Ok(v.view())
}

/// Asks for the microphone the first time, then listens for one command.
#[tauri::command(async)]
pub fn voice_tap(app: tauri::AppHandle, hub: H) -> Result<(), String> {
    let v = voice(&hub)?;
    if !crate::android::microphone_allowed(&app) {
        return microphone_refused(&hub, &v);
    }
    v.tap();
    Ok(())
}

#[tauri::command]
pub fn voice_confirm(hub: H, yes: bool) -> Result<(), String> {
    voice(&hub)?.confirm(yes);
    Ok(())
}
```

In `set_config`, take `app: tauri::AppHandle` as its first parameter and, after saving, tell the voice — asking for the microphone when the wake word was just turned on:

```rust
    if let Some(v) = hub.voice.get() {
        if saved.wake_word && !before.wake_word && !crate::android::microphone_allowed(&app) {
            microphone_refused(&hub, v)?;
            return Ok(hub.settings.lock().unwrap().config.clone());
        }
        v.set_wake(saved.wake_word);
    }
```

- [ ] **Step 7: Register and build the voice**

In `mobile/src/lib.rs`: `.plugin(android::voice_init())` after `.plugin(android::init())`; add `commands::voice_view`, `commands::voice_tap` and `commands::voice_confirm` to `generate_handler!`; and in `setup`, after `app.manage(hub.clone())`:

```rust
            let ask_hub = Arc::downgrade(&hub);
            let ask: voice::Ask = Arc::new(move |r| ask_hub.upgrade().ok_or("Maya is closing.")?.request(None, r));
            let shown = handle.clone();
            let show: voice::Show = Arc::new(move |v| {
                let _ = shown.emit("voice", v);
            });
            let v = voice::Voice::new(android::voice_io(&handle), ask, show, maya_core::store::now_ms);
            v.set_wake(hub.settings.lock().unwrap().config.wake_word);
            let _ = hub.voice.set(v);
            let quiet = handle.clone();
            let _ = hub.silenced.set(Box::new(move || android::silenced(&quiet)));
            if let Some(v) = hub.voice.get() {
                android::watch_voice(&handle, Arc::downgrade(v));
                if !android::on_device(&handle) {
                    v.set_note(Some("Speech goes through Google's service and may leave the phone.".into()));
                }
            }
```

(`maya_core::store::now_ms` is the clock the desktop uses; use the hub's own `now` if `store` is not public.)

- [ ] **Step 8: Run the tests and a host build**

Run: `cargo test -p maya-mobile && cargo build -p maya-mobile`
Expected: PASS and builds (the host build compiles the no-op bridge).

- [ ] **Step 9: Commit**

```bash
git add mobile/src mobile/tests
git commit -m "feat: wire the phone's voice to the hub, the page and Android

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The Kotlin `VoicePlugin`

**Files:**
- Create: `mobile/gen/android/app/src/main/java/com/dosaki/maya/mobile/VoicePlugin.kt`
- Modify: `mobile/gen/android/app/src/main/AndroidManifest.xml`
- Modify: `docs/DEVELOPING.md` (the hand-edited files list, ~line 294)

**Interfaces:**
- Consumes: Task 7's calls `watch{channel}`, `listen{wake}`, `stopListening`, `speak{line}`, `stopSpeaking`, `quiet`, `state`, `checkPermissions`, `requestPermissions{permissions: ["microphone"]}`.
- Produces: events on the watched channel, each `{event, …}`: `partial{text}`, `final{text}`, `ended`, `failed{message, fatal}`, `spoken{ok}`, `screen{on}` (also sent once when `watch` is called, so Rust learns the current state); permission alias `microphone`; `state -> {onScreen, available, onDevice}`.

- [ ] **Step 1: Add the permission**

In `AndroidManifest.xml`, next to the other `uses-permission` lines:

```xml
    <uses-permission android:name="android.permission.RECORD_AUDIO" />
```

- [ ] **Step 2: Write the plugin**

```kotlin
package com.dosaki.maya.mobile

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import android.webkit.WebView
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.UUID

@InvokeArg
class ListenArgs {
    var wake: Boolean = false
}

@InvokeArg
class SpeakArgs {
    var line: String = ""
}

@InvokeArg
class WatchArgs {
    lateinit var channel: Channel
}

/**
 * Android's recogniser and voice for Maya's voice conversation, which the
 * Rust side runs. Nothing listens or speaks while the app is off screen:
 * this class stops both itself on `ON_STOP` and refuses `listen` and
 * `speak` until `ON_START`, whatever the page or Rust ask.
 */
@TauriPlugin(permissions = [Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone")])
class VoicePlugin(private val activity: Activity) : Plugin(activity), DefaultLifecycleObserver {
    private val main = Handler(Looper.getMainLooper())
    private var recognizer: SpeechRecognizer? = null
    private var recognizerOnDevice = false
    private var tts: TextToSpeech? = null
    @Volatile private var ttsReady = false
    @Volatile private var onScreen = true
    /** Where events go: the Rust side's channel, from `watch`. */
    @Volatile private var events: Channel? = null

    private fun emit(event: String, fill: JSObject.() -> Unit = {}) {
        events?.send(JSObject().apply { put("event", event); fill() })
    }

    @Command
    fun watch(invoke: Invoke) {
        events = invoke.parseArgs(WatchArgs::class.java).channel
        // ON_START may have come before the channel did.
        emit("screen") { put("on", onScreen) }
        invoke.resolve()
    }

    override fun load(webView: WebView) {
        super.load(webView)
        main.post { ProcessLifecycleOwner.get().lifecycle.addObserver(this) }
        tts = TextToSpeech(activity) { status ->
            ttsReady = status == TextToSpeech.SUCCESS
            tts?.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                override fun onStart(utteranceId: String?) {}
                override fun onDone(utteranceId: String?) = spoken(true)
                @Deprecated("Deprecated in Java")
                override fun onError(utteranceId: String?) = spoken(false)
                override fun onError(utteranceId: String?, errorCode: Int) = spoken(false)
            })
        }
    }

    override fun onStart(owner: LifecycleOwner) {
        onScreen = true
        emit("screen") { put("on", true) }
    }

    override fun onStop(owner: LifecycleOwner) {
        onScreen = false
        stopRecognizer()
        tts?.stop()
        emit("screen") { put("on", false) }
    }

    private fun spoken(ok: Boolean) {
        emit("spoken") { put("ok", ok) }
    }

    private fun onDeviceAvailable(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && SpeechRecognizer.isOnDeviceRecognitionAvailable(activity)

    @Command
    fun state(invoke: Invoke) {
        val ret = JSObject()
        ret.put("onScreen", onScreen)
        ret.put("available", SpeechRecognizer.isRecognitionAvailable(activity))
        ret.put("onDevice", onDeviceAvailable())
        invoke.resolve(ret)
    }

    @Command
    fun listen(invoke: Invoke) {
        val args = invoke.parseArgs(ListenArgs::class.java)
        if (!onScreen) return invoke.reject("Maya only listens while she is on screen.")
        if (!SpeechRecognizer.isRecognitionAvailable(activity)) {
            fail("This phone has no speech recognition.", true)
            return invoke.resolve()
        }
        if (getPermissionState("microphone") != PermissionState.GRANTED) {
            fail("Microphone access is off.", true)
            return invoke.resolve()
        }
        if (args.wake && !onDeviceAvailable()) {
            fail("The wake word needs on-device speech recognition, which this phone does not have.", true)
            return invoke.resolve()
        }
        main.post { startRecognizer(args.wake) }
        invoke.resolve()
    }

    @Command
    fun stopListening(invoke: Invoke) {
        main.post { stopRecognizer() }
        invoke.resolve()
    }

    @Command
    fun speak(invoke: Invoke) {
        val args = invoke.parseArgs(SpeakArgs::class.java)
        val engine = tts
        if (!onScreen || engine == null || !ttsReady) {
            spoken(false)
            return invoke.resolve()
        }
        main.post { stopRecognizer() }
        engine.speak(args.line, TextToSpeech.QUEUE_FLUSH, Bundle(), UUID.randomUUID().toString())
        invoke.resolve()
    }

    @Command
    fun stopSpeaking(invoke: Invoke) {
        tts?.stop()
        invoke.resolve()
    }

    /** On silent or vibrate, or in Do Not Disturb: board events stay unspoken. */
    @Command
    fun quiet(invoke: Invoke) {
        val audio = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val notifications = activity.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val quiet = audio.ringerMode != AudioManager.RINGER_MODE_NORMAL ||
            notifications.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL
        invoke.resolve(JSObject().apply { put("value", quiet) })
    }

    private fun fail(message: String, fatal: Boolean) {
        emit("failed") { put("message", message); put("fatal", fatal) }
    }

    private fun startRecognizer(wake: Boolean) {
        if (!onScreen) return
        val onDevice = wake || onDeviceAvailable()
        if (recognizer == null || recognizerOnDevice != onDevice) {
            recognizer?.destroy()
            recognizer = if (onDevice && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                SpeechRecognizer.createOnDeviceSpeechRecognizer(activity)
            } else {
                SpeechRecognizer.createSpeechRecognizer(activity)
            }
            recognizerOnDevice = onDevice
            recognizer?.setRecognitionListener(listener)
        }
        val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
            putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            putExtra(RecognizerIntent.EXTRA_LANGUAGE, "en-GB")
            putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
            if (onDevice) putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
        }
        recognizer?.startListening(intent)
    }

    private fun stopRecognizer() {
        recognizer?.cancel()
    }

    private val listener = object : RecognitionListener {
        override fun onReadyForSpeech(params: Bundle?) {}
        override fun onBeginningOfSpeech() {}
        override fun onRmsChanged(rmsdB: Float) {}
        override fun onBufferReceived(buffer: ByteArray?) {}
        override fun onEndOfSpeech() {}
        override fun onEvent(eventType: Int, params: Bundle?) {}

        override fun onPartialResults(partialResults: Bundle?) {
            val text = partialResults?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull() ?: return
            emit("partial") { put("text", text) }
        }

        override fun onResults(results: Bundle?) {
            val text = results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
            if (text.isNullOrBlank()) emit("ended")
            else emit("final") { put("text", text) }
        }

        override fun onError(error: Int) {
            when (error) {
                // Nothing said, or nothing understood: not a failure.
                SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> emit("ended")
                // Our own cancel (leaving the screen, speaking): say nothing.
                SpeechRecognizer.ERROR_CLIENT -> {}
                SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> fail("Microphone access is off.", true)
                SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED, SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE ->
                    fail("English speech recognition is not installed on this phone.", true)
                else -> fail(describe(error), false)
            }
        }
    }

    private fun describe(error: Int): String = when (error) {
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "the recogniser is busy"
        SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT -> "no network for speech recognition"
        SpeechRecognizer.ERROR_AUDIO -> "the microphone failed"
        SpeechRecognizer.ERROR_SERVER, SpeechRecognizer.ERROR_SERVER_DISCONNECTED -> "the speech service failed"
        SpeechRecognizer.ERROR_TOO_MANY_REQUESTS -> "too many requests"
        else -> "speech recognition failed ($error)"
    }

    override fun onDestroy() {
        recognizer?.destroy()
        tts?.shutdown()
        super.onDestroy()
    }
}
```

Check `ERROR_LANGUAGE_NOT_SUPPORTED`/`ERROR_LANGUAGE_UNAVAILABLE` (API 31) and `ERROR_SERVER_DISCONNECTED`/`ERROR_TOO_MANY_REQUESTS` (API 31) compile against `compileSdk = 37`; they are compile-time constants, so they work on `minSdk 26` devices too. If `app.tauri.PermissionState` lives elsewhere, find it with `grep -rn "enum class PermissionState" ~/.cargo/registry/src/*/tauri-2.*/mobile/android`.

- [ ] **Step 3: List the plugin in DEVELOPING.md**

In the "What is where" bullet for `mobile/gen/android/`, change "plus `KeepAliveService.kt` (the foreground service), `KeepAlivePlugin.kt`," to "plus `KeepAliveService.kt` (the foreground service), `KeepAlivePlugin.kt`, `VoicePlugin.kt` (the recogniser and the voice)," and "restore those four files, the manifest's permissions" to "restore those five files, the manifest's permissions (`RECORD_AUDIO` included)".

- [ ] **Step 4: Build the APK**

Run (with `ANDROID_HOME` and `NDK_HOME` set as in DEVELOPING.md): `pnpm mobile:build --debug`
Expected: the build succeeds and an APK lands in `mobile/gen/android/app/build/outputs/apk/universal/`. If the local machine has no Android SDK, push the branch and use the CI Android job's result instead; say so in the task report.

- [ ] **Step 5: Commit**

```bash
git add mobile/gen/android/app/src/main docs/DEVELOPING.md
git commit -m "feat: Android's recogniser and voice for the phone

VoicePlugin listens and speaks only while the app is on screen and stops
both itself when it leaves; the wake word uses the on-device recogniser.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: The phone page: mic bar and Voice card

**Files:**
- Create: `src/mobile/voice.ts`, `src/mobile/voice.test.ts`
- Modify: `src/mobile/network.ts` (model, handlers, Voice card, `ConfigView`), `src/mobile/network.test.ts`
- Modify: `src/mobile/main.ts` (mount), `src/mobile/mobile.css`, `mobile/web/index.html` (a host element)

**Interfaces:**
- Consumes: Task 7's commands `voice_view`, `voice_tap`, `voice_confirm` and its `voice` event. The page never calls the plugin: Rust receives its events and asks for the microphone.
- Produces: `renderVoiceBar(view: VoiceView, h: VoiceHandlers): HTMLElement`, `initVoice(): Promise<void>`.

- [ ] **Step 1: Write the failing tests**

`src/mobile/voice.test.ts`:

```ts
import { describe, expect, it, vi } from "vitest";
import { renderVoiceBar, type VoiceView } from "./voice";

const base: VoiceView = { state: "idle", heard: "", said: "", pending: null, error: null, offer: null, note: null };
const handlers = () => ({ onTap: vi.fn(), onConfirm: vi.fn() });

describe("renderVoiceBar", () => {
  it("offers the mic when voice is offered, and taps through", () => {
    const h = handlers();
    const el = renderVoiceBar(base, h);
    const mic = el.querySelector<HTMLButtonElement>("button[data-action=mic]")!;
    expect(mic.disabled).toBe(false);
    mic.click();
    expect(h.onTap).toHaveBeenCalled();
  });

  it("disables the mic with the reason when voice is not offered", () => {
    const el = renderVoiceBar({ ...base, state: "off", offer: "Update Yhi to talk to Maya." }, handlers());
    expect(el.querySelector<HTMLButtonElement>("button[data-action=mic]")!.disabled).toBe(true);
    expect(el.textContent).toContain("Update Yhi to talk to Maya.");
  });

  it("hides entirely when the phone is its own main", () => {
    const el = renderVoiceBar({ ...base, state: "off", offer: "Voice needs a desktop main." }, handlers());
    expect(el.hidden).toBe(true);
  });

  it("shows what was heard and said, and Thinking while the main works", () => {
    expect(renderVoiceBar({ ...base, heard: "what needs me", said: "Two sessions need you." }, handlers()).textContent).toContain("what needs me");
    expect(renderVoiceBar({ ...base, said: "Two sessions need you." }, handlers()).textContent).toContain("Two sessions need you.");
    expect(renderVoiceBar({ ...base, state: "thinking" }, handlers()).textContent).toContain("Thinking…");
  });

  it("shows Yes and No while a read-back waits", () => {
    const h = handlers();
    const el = renderVoiceBar({ ...base, state: "awaiting-confirm", pending: "Telling x: y. Yes?" }, h);
    el.querySelector<HTMLButtonElement>("button[data-action=yes]")!.click();
    el.querySelector<HTMLButtonElement>("button[data-action=no]")!.click();
    expect(h.onConfirm.mock.calls).toEqual([[true], [false]]);
    expect(renderVoiceBar(base, h).querySelector("button[data-action=yes]")).toBeNull();
  });

  it("notes an online recogniser while idle", () => {
    const note = "Speech goes through Google's service and may leave the phone.";
    expect(renderVoiceBar({ ...base, note }, handlers()).querySelector(".voice__note")?.textContent).toBe(note);
    expect(renderVoiceBar({ ...base, note, state: "thinking" }, handlers()).querySelector(".voice__note")).toBeNull();
  });

  it("shows an error under the mic", () => {
    expect(renderVoiceBar({ ...base, error: "Microphone access is off." }, handlers()).querySelector(".voice__error")?.textContent).toBe("Microphone access is off.");
  });
});
```

In `src/mobile/network.test.ts`, extend the model fixture with `speak: true, wakeWord: false`, add `onSwitch` coverage for the new switches:

```ts
  it("has a Voice card with Speak and the wake word", () => {
    const h = handlers();
    const el = renderNetwork({ ...base, speak: true, wakeWord: false }, h);
    const speak = el.querySelector<HTMLInputElement>("input[name=speak]")!;
    const wake = el.querySelector<HTMLInputElement>("input[name=wakeWord]")!;
    expect(speak.checked).toBe(true);
    expect(wake.checked).toBe(false);
    wake.checked = true;
    wake.dispatchEvent(new Event("change"));
    expect(h.onSwitch).toHaveBeenCalledWith("wakeWord", true);
    expect(el.textContent).toContain("experimental");
  });
```

(Use the test file's existing base model name in place of `base`.)

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm vitest run src/mobile/voice.test.ts src/mobile/network.test.ts`
Expected: FAIL (module missing, card missing).

- [ ] **Step 3: Implement `voice.ts`**

```ts
// The phone's mic bar. The conversation runs in Rust, which hears the
// voice plugin's events and asks for the microphone itself; the page only
// shows the `voice` view and sends taps and Yes/No.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface VoiceView {
  state: "off" | "idle" | "listening" | "thinking" | "awaiting-command" | "awaiting-confirm" | "speaking";
  heard: string;
  said: string;
  pending: string | null;
  error: string | null;
  /** Why voice is not offered; null when it is. */
  offer: string | null;
  /** Shown while idle: tap-to-talk goes through an online recogniser. */
  note: string | null;
}

export interface VoiceHandlers {
  onTap(): void;
  onConfirm(yes: boolean): void;
}

const NO_MAIN = "Voice needs a desktop main.";

function line(className: string, text: string): HTMLElement {
  const d = document.createElement("div");
  d.className = className;
  d.textContent = text;
  return d;
}

export function renderVoiceBar(v: VoiceView, h: VoiceHandlers): HTMLElement {
  const bar = document.createElement("div");
  bar.className = `voice voice--${v.state}`;
  bar.hidden = v.offer === NO_MAIN;
  const mic = document.createElement("button");
  mic.type = "button";
  mic.className = "voice__mic";
  mic.dataset.action = "mic";
  mic.setAttribute("aria-label", "Talk to Maya");
  mic.textContent = "🎙";
  mic.disabled = v.offer !== null;
  mic.addEventListener("click", () => h.onTap());
  const said = document.createElement("div");
  said.className = "voice__line";
  if (v.offer) said.textContent = v.offer;
  else if (v.state === "thinking") said.textContent = "Thinking…";
  else {
    if (v.heard) said.append(line("voice__heard", v.heard));
    if (v.said) said.append(line("voice__said", v.said));
  }
  bar.append(mic, said);
  if (v.pending && v.state === "awaiting-confirm") {
    const answer = document.createElement("div");
    answer.className = "voice__answer";
    for (const [label, yes] of [["Yes", true], ["No", false]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.dataset.action = yes ? "yes" : "no";
      b.textContent = label;
      b.addEventListener("click", () => h.onConfirm(yes));
      answer.append(b);
    }
    bar.append(answer);
  }
  if (v.note && v.state === "idle" && !v.offer) bar.append(line("voice__note", v.note));
  if (v.error) bar.append(line("voice__error", v.error));
  return bar;
}

export async function initVoice(): Promise<void> {
  const host = document.getElementById("voice-host");
  if (!host) return;
  const handlers: VoiceHandlers = {
    onTap: () => void invoke("voice_tap").catch(() => undefined),
    onConfirm: (yes) => void invoke("voice_confirm", { yes }).catch(() => undefined),
  };
  const paint = (v: VoiceView) => host.replaceChildren(renderVoiceBar(v, handlers));
  await listen<VoiceView>("voice", (e) => paint(e.payload));
  const off: VoiceView = { state: "off", heard: "", said: "", pending: null, error: null, offer: NO_MAIN, note: null };
  paint(await invoke<VoiceView>("voice_view").catch(() => off));
}
```

- [ ] **Step 4: The Voice card**

In `src/mobile/network.ts`:
- `NetworkModel` gains `speak: boolean; wakeWord: boolean;`.
- `NetworkHandlers.onSwitch`'s `which` becomes `"awaiting" | "completed" | "speak" | "wakeWord"`.
- `ConfigView` gains `speakNotifications: boolean; wakeWord: boolean;`.
- After the Notifications section in `renderNetwork`:

```ts
  const voice = section("Voice");
  for (const [which, label, on] of [["speak", "Speak", m.speak], ["wakeWord", "Wake word \"Maya\" (experimental)", m.wakeWord]] as const) {
    const l = el("label", "settings__switch");
    const box = el("input", "");
    box.type = "checkbox";
    box.name = which;
    box.checked = on;
    box.addEventListener("change", () => h.onSwitch(which, box.checked));
    l.append(box, document.createTextNode(` ${label}`));
    voice.append(l);
  }
  voice.append(el("p", "settings__hint", "While Maya is on screen she reads out sessions that need you, and with the wake word on she listens for \"Maya\". She never listens with the app closed. Voice needs a desktop main."));
  root.append(voice);
```

- In `initNetwork`: the model's defaults `speak: true, wakeWord: false`; wherever the config is loaded into the model (search `notifyAwaiting = ` in `loadConfig`), also set `model.speak = config.speakNotifications ?? true; model.wakeWord = config.wakeWord ?? false;`; `onSwitch` gains:

```ts
      else if (which === "speak") c.speakNotifications = on;
      else if (which === "wakeWord") c.wakeWord = on;
```

(keep `awaiting`/`completed` as they are, turning the `else` into `else if (which === "completed")`).

- [ ] **Step 5: Mount the bar**

In `mobile/web/index.html`, after `<div id="strip-host"></div>`: `<div id="voice-host"></div>`. In `src/mobile/main.ts`: `import { initVoice } from "./voice";` and call `void initVoice();` where the page starts its other listeners (next to `initNetwork(...)`). Hide `#voice-host` when a tab other than Sessions shows: in the `makeTabs` change callback already passed in `main.ts`, add `document.getElementById("voice-host")!.hidden = tab !== "sessions";`.

In `src/mobile/mobile.css`:

```css
/* The mic bar under the column strip: the mic, what was heard and said, Yes and No. */
.voice { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 8px 12px; border-bottom: 1px solid var(--border); }
.voice__mic { width: 44px; height: 44px; border-radius: 50%; border: 1px solid var(--gold-deep); background: var(--panel); color: var(--gold-bright); font-size: 20px; }
.voice--listening .voice__mic, .voice--awaiting-command .voice__mic, .voice--awaiting-confirm .voice__mic { background: var(--gold); color: var(--on-gold); }
.voice__mic:disabled { opacity: .5; }
.voice__line { flex: 1; min-width: 0; font-size: 14px; }
.voice__heard { color: var(--muted); }
.voice__answer { display: flex; gap: 8px; }
.voice__answer button { min-height: 40px; padding: 0 16px; }
.voice__error { flex-basis: 100%; color: #dc2626; font-size: 12.5px; }
.voice__note { flex-basis: 100%; color: var(--muted); font-size: 12.5px; }
```

- [ ] **Step 6: Run the page tests and builds**

Run: `pnpm test && pnpm exec tsc --noEmit -p . && pnpm build:mobile`
Expected: PASS, no type errors, build succeeds.

- [ ] **Step 7: Commit**

```bash
git add src/mobile mobile/web/index.html
git commit -m "feat: the phone's mic bar and Voice settings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Whole-branch check, version bump

**Files:**
- Modify: `package.json`, `src-tauri/tauri.conf.json`, `mobile/tauri.conf.json`, `Cargo.toml`, `Cargo.lock` (through the script)

- [ ] **Step 1: Run every suite**

Run: `pnpm test && pnpm exec tsc --noEmit -p . && cargo test --workspace && pnpm build && pnpm build:mobile`
Expected: all PASS. Rerun once before debugging any of the known flaky tests listed in the project memory (`run_pairs_connects…`, the Linux terminal focus test, `a_move_with_a_bad_key`).

- [ ] **Step 2: Bump from `main`**

```bash
git fetch -q origin
git show origin/main:package.json | grep '"version"'
```

Take that version, bump the minor (e.g. `0.22.1` → `0.23.0`), then:

```bash
sh scripts/set-version.sh 0.23.0
git status --short   # Cargo.lock must be listed; if not:
PATH="$HOME/.cargo/bin:$PATH" cargo update --workspace --offline
git add package.json src-tauri/tauri.conf.json mobile/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.23.0

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 3: Mark the spec implemented**

In `docs/superpowers/specs/2026-10-16-phone-voice-design.md`, change `Status: proposed` to `Status: implemented`, and commit it as `docs: mark the phone voice design implemented` (before the release commit if the bump has not been made yet, so the bump stays the last commit).

The PR's test plan carries the spec's hand checks (tap-to-talk with a report and with a confirmed answer; the wake word; leaving the screen mid-confirmation; an event spoken on screen and not on silent; permission refused), to be done by the user on the phone joined to the Mac.
