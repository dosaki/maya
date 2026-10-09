# Typed Replies Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A reply sent to a Claude Code session through Maya is typed into its terminal, so Claude takes it as the user's own input; the card history tells the user's turns, other sessions' messages and Claude Code's internal lines apart.

**Architecture:** `actions::send_reply` (the one place every reply ends, on the machine running the session) types a Claude Code reply line by line through the existing `Terminal` seam, using Claude Code's `\`+Enter continuation for newlines, and falls back to the inbox socket only when nothing reached the terminal. It returns a `Route` (`typed` / `inbox`), which the network carries in `Up::Result.data` and the page shows. `transcript.rs` gains a `Notice` turn kind for compaction and interrupts and strips paste tags.

**Tech Stack:** Rust (`maya-core`, `maya-cli`, Tauri app, Android `mobile` crate), TypeScript + Vitest (`src/`).

**Spec:** `docs/superpowers/specs/2026-10-09-typed-replies-design.md`

## Global Constraints

- Typed replies are capped at 20,000 characters: "Message is too long to type (over 20000 characters)."; the inbox keeps `inbox::MAX_CHARS` (100,000).
- Fallback to the inbox only when the first typed line failed (nothing reached the terminal) or the session has no tty.
- Partial typing failure: "Part of the reply was typed into the terminal; check it there." and no inbox resend.
- Neither route: "Maya can't type into this session's terminal and it has no inbox. Use the terminal."
- A pending choice refuses a Claude Code reply with `answer::check_free`'s existing message, "This session is waiting for a decision; answer it first." (The spec wrote "Answer the question on the card first."; the existing message is kept because a Claude permission prompt is not always answerable on the card.)
- The Awaiting Decision banner without card options reads "This session is waiting for a decision in its terminal. Answer it there before replying." (remote: "… on <machine>. Answer it there before replying."). The spec wrote "Answer it on the card first."; that banner only shows when the card has no answer buttons, so it points at the terminal.
- Reply statuses: "Sent as you" (green), "Sent as a message from another session: it can't approve anything" (amber), "Delivered" when no route came back.
- Peer label: "From another session". Notices: "Earlier conversation compacted", "Interrupted".
- OpenCode and the other agents keep their routes; both count as `Route::Typed` (the user's own input).
- No protocol change: the route rides in `Up::Result.data` as `{"route": "typed" | "inbox"}`.
- Version: the pull request bumps to 0.15.0 with `sh scripts/set-version.sh 0.15.0`, committed alone as `chore(release): 0.15.0`; check `Cargo.lock` changed (cargo may be off `sh`'s PATH: prefix `PATH="$HOME/.cargo/bin:$PATH"`).
- Tests run with `PATH="$HOME/.cargo/bin:$PATH" cargo test -p <crate>` and `pnpm test`. Unix-only tests are gated `#[cfg(unix)]` (Windows CI runs the workspace).

## Review Focus

1. A reply sent while the user's Claude session sits on a permission prompt: refused with the check_free message, nothing typed, nothing sent to the inbox (Task 2 test `a_reply_while_claude_asks_permission_is_refused_and_nothing_is_sent`).
2. A Claude session in iTerm/VS Code (Terminal.app has no tab for its tty) with an inbox: the reply arrives through the inbox and the page says it can't approve anything (Task 2 `falls_back_to_the_inbox_when_nothing_reached_the_terminal`, Task 5 `replyStatus`).
3. A multi-line reply with blank lines, Windows line endings, or a last line ending in `\` still arrives as one message that sends (Task 1 tests).
4. An older phone/Mac talking to a newer main, or the reverse: no route in `data` shows "Delivered" and nothing breaks (Task 3 `route_from_data_reads_the_route_and_tolerates_none`, Task 5 `replyStatus(null)`).
5. A compacted session's history: the long summary does not show as the user's message, and the history after it is intact (Task 4 `a_compaction_summary_is_a_notice_not_the_users_words`).

---

### Task 1: The lines a multi-line reply is typed as

**Files:**
- Modify: `core/src/answer.rs` (add beside `check_free`, ~line 83; tests in its `mod tests`)

**Interfaces:**
- Produces: `pub const TYPED_MAX_CHARS: usize = 20_000;` and `pub fn reply_lines(text: &str) -> Vec<String>` in `maya_core::answer`.

- [ ] **Step 1: Write the failing tests** (append inside `core/src/answer.rs`'s `mod tests`)

```rust
    #[test]
    fn reply_lines_continue_every_line_but_the_last() {
        assert_eq!(reply_lines("go on"), vec!["go on"]);
        assert_eq!(reply_lines("a\nb\nc"), vec!["a\\", "b\\", "c"]);
    }

    #[test]
    fn reply_lines_keep_blank_lines_and_drop_carriage_returns_and_outer_newlines() {
        assert_eq!(reply_lines("a\n\nb"), vec!["a\\", "\\", "b"]);
        assert_eq!(reply_lines("a\r\nb\r\n"), vec!["a\\", "b"]);
        assert_eq!(reply_lines("\n\nhi\n"), vec!["hi"]);
    }

    #[test]
    fn reply_lines_still_send_when_the_last_line_ends_in_a_backslash() {
        assert_eq!(reply_lines("see C:\\"), vec!["see C:\\ "]);
        // A middle line ending in `\` gets the continuation after it; the live check confirms what Claude Code makes of `\\`.
        assert_eq!(reply_lines("a\\\nb"), vec!["a\\\\", "b"]);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core answer::tests::reply_lines`
Expected: FAIL to compile, "cannot find function `reply_lines`".

- [ ] **Step 3: Implement** (in `core/src/answer.rs`, after `check_free`)

```rust
/// Most characters a reply typed into a terminal may have; the inbox takes more.
pub const TYPED_MAX_CHARS: usize = 20_000;

/// The lines to type for a reply to Claude Code, each followed by Enter.
/// Every line but the last ends in `\`, Claude Code's continuation, so its
/// Enter adds a line instead of sending; the last line's Enter sends. A
/// last line that itself ends in `\` gets a space, so its Enter still sends.
pub fn reply_lines(text: &str) -> Vec<String> {
    let text = text.replace('\r', "");
    let lines: Vec<&str> = text.trim_matches('\n').split('\n').collect();
    let last = lines.len() - 1;
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| match (i < last, l.ends_with('\\')) {
            (true, _) => format!("{l}\\"),
            (false, true) => format!("{l} "),
            (false, false) => l.to_string(),
        })
        .collect()
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core answer::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/src/answer.rs
git commit -m "feat(core): split a reply into the lines Claude Code takes as one message"
```

---

### Task 2: `send_reply` types a Claude Code reply and reports its route

**Files:**
- Modify: `core/src/actions.rs:100-152` (`send_reply`), `core/src/actions.rs:438-447` (`session_history` relabel), tests in its `mod tests`
- Modify: `core/src/terminal.rs:46-77` (`FakeTerminal`: `fail_after`)
- Modify: `core/src/store.rs` (remove `sent`, `SENT_KEPT`, `sent_hash`, `sent_path`, `note_sent`, `was_sent` and their test, ~lines 51-54, 97-105, 169, 177-215, 570-595)

**Interfaces:**
- Consumes: `answer::reply_lines`, `answer::TYPED_MAX_CHARS` (Task 1).
- Produces: in `maya_core::actions`:
  ```rust
  #[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
  #[serde(rename_all = "lowercase")]
  pub enum Route { Typed, Inbox }
  pub fn send_reply(l: &Local, session_id: &str, text: &str) -> Result<Route, String>;
  ```
  and `FakeTerminal.fail_after: Option<usize>` in `maya_core::terminal`.

- [ ] **Step 1: Give the fake terminal a mid-reply failure** (`core/src/terminal.rs`, in `FakeTerminal`'s fields and its `type_line`)

```rust
        /// Typing fails once this many lines have been typed, as a terminal
        /// closed in the middle of a reply does.
        pub fail_after: Option<usize>,
```

and at the top of `type_line`, after the `fail_type` check:

```rust
            if let Some(n) = self.fail_after {
                if self.calls.lock().unwrap().iter().filter(|c| matches!(c, Call::Type { .. })).count() >= n {
                    return Err("the terminal went away".into());
                }
            }
```

- [ ] **Step 2: Write the failing tests** (in `core/src/actions.rs`'s `mod tests`)

```rust
    /// Points `id`'s registry entry (pid `pid`, from `store_with_session`) at
    /// an inbox socket this test listens on; the listener and its path.
    #[cfg(unix)]
    fn with_inbox(store: &Mutex<Store>, id: &str, pid: i32) -> std::os::unix::net::UnixListener {
        let claude = store.lock().unwrap().claude_dir().to_path_buf();
        let socket = claude.join(format!("{id}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let path = claude.join(format!("sessions/{pid}.json"));
        let mut entry: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        entry["messagingSocketPath"] = socket.to_string_lossy().into_owned().into();
        std::fs::write(&path, entry.to_string()).unwrap();
        listener
    }

    fn typed(fake: &FakeTerminal) -> Vec<String> {
        fake.calls.lock().unwrap().iter().filter_map(|c| match c { Call::Type { text, .. } => Some(text.clone()), _ => None }).collect()
    }

    #[test]
    fn a_claude_reply_is_typed_line_by_line_as_the_user() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "push it\n\nthanks"), Ok(Route::Typed));
        assert_eq!(typed(&fake), vec!["push it\\", "\\", "thanks"]);
        assert!(fake.calls.lock().unwrap().iter().all(|c| matches!(c, Call::Type { tty, .. } if tty == "/dev/pts/3")));
    }

    #[cfg(unix)]
    #[test]
    fn falls_back_to_the_inbox_when_nothing_reached_the_terminal() {
        use std::io::Read;
        let pid = std::process::id() as i32;
        let (_d, store) = store_with_session("s1", pid);
        let inbox = with_inbox(&store, "s1", pid);
        let fake = FakeTerminal { fail_type: Some("No Terminal tab found for /dev/pts/3.".into()), ..Default::default() };
        let l = Local { store: &store, terminal: &fake };
        let reader = std::thread::spawn(move || {
            let (mut s, _) = inbox.accept().unwrap();
            let mut got = String::new();
            s.read_to_string(&mut got).unwrap();
            got
        });
        assert_eq!(send_reply(&l, "s1", "approved"), Ok(Route::Inbox));
        assert_eq!(reader.join().unwrap(), inbox::message_line("approved"));
    }

    #[test]
    fn a_reply_cut_off_partway_is_not_resent_through_the_inbox() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal { fail_after: Some(1), ..Default::default() };
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "one\ntwo"), Err("Part of the reply was typed into the terminal; check it there.".into()));
        assert_eq!(typed(&fake), vec!["one\\"]);
    }

    #[test]
    fn says_so_when_neither_the_terminal_nor_an_inbox_is_there() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal { fail_type: Some("No Terminal tab found for /dev/pts/3.".into()), ..Default::default() };
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "hi"), Err("Maya can't type into this session's terminal and it has no inbox. Use the terminal.".into()));
    }

    #[test]
    fn a_reply_while_claude_asks_permission_is_refused_and_nothing_is_sent() {
        let (_d, store) = store_with_session("s1", 4242);
        let maya = store.lock().unwrap().claude_dir().join("maya");
        crate::hook_install::append_record_named(&maya, "events.jsonl", r#"{"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"git push"}}"#, now_ms()).unwrap();
        store.lock().unwrap().refresh(now_ms());
        assert_eq!(store.lock().unwrap().card_for("s1", now_ms()).unwrap().state, State::Awaiting);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", "yes"), Err("This session is waiting for a decision; answer it first.".into()));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_typed_reply_has_its_own_length_cap() {
        let (_d, store) = store_with_session("s1", 4242);
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(send_reply(&l, "s1", &"x".repeat(answer::TYPED_MAX_CHARS + 1)), Err("Message is too long to type (over 20000 characters).".into()));
        assert_eq!(send_reply(&l, "s1", "  \n "), Err("Message is empty.".into()));
        assert!(fake.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_peer_message_in_the_history_stays_a_peer_turn() {
        let (_d, store) = store_with_session("s1", 4242);
        let path = store.lock().unwrap().transcript_path_for(&store.lock().unwrap().session("s1").unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let line = serde_json::json!({"type":"user","message":{"role":"user","content":"Another Claude session sent a message:\nhello\n\nThis came from another Claude session."}});
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let fake = FakeTerminal::default();
        let l = Local { store: &store, terminal: &fake };
        assert_eq!(session_history(&l, "s1").unwrap(), vec![transcript::Turn { kind: transcript::TurnKind::Peer, text: "hello".into() }]);
    }
```

If `refresh` or the `PermissionRequest` record does not make the card `Awaiting`, mirror `core/src/state.rs`'s `tool_ev("PermissionRequest", …)` test fields (`received_at`) in the record before changing anything else; the assertion on `State::Awaiting` is there so the test cannot pass for the wrong reason. If `store.lock()` twice on one line deadlocks in `a_peer_message_in_the_history_stays_a_peer_turn`, take the session first: `let s = store.lock().unwrap().session("s1").unwrap(); let path = store.lock().unwrap().transcript_path_for(&s);`.

- [ ] **Step 3: Run them to see them fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core actions::tests`
Expected: FAIL to compile, "cannot find type `Route`" (and `fail_after` compiles from Step 1).

- [ ] **Step 4: Implement the route** (`core/src/actions.rs`)

Above `send_reply`:

```rust
/// How a reply reached its session: as the user's own input (typed into its
/// terminal, or OpenCode's server), or through the inbox as a message from
/// another session, which cannot approve anything.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    Typed,
    Inbox,
}

/// Why a typed reply failed: before any key reached the terminal, or partway.
enum TypeFail {
    NotReached(String),
    Partial(String),
}

/// Types `lines` into the terminal on `tty`, one `type_line` each, holding
/// `TYPING` throughout so no other typed action lands inside the reply.
fn type_reply(l: &Local, tty: &str, lines: &[String]) -> Result<(), TypeFail> {
    let _typing = TYPING.lock().unwrap_or_else(|e| e.into_inner());
    for (i, line) in lines.iter().enumerate() {
        if let Err(e) = l.terminal.type_line(tty, line) {
            return Err(if i == 0 { TypeFail::NotReached(e) } else { TypeFail::Partial(e) });
        }
    }
    Ok(())
}
```

Change `send_reply`'s doc and signature to

```rust
/// Sends `text` to the session as the user: typed into its terminal, or a
/// call to OpenCode's server. A Claude Code session Maya cannot type into
/// gets it through its inbox instead, as a message from another session.
pub fn send_reply(l: &Local, session_id: &str, text: &str) -> Result<Route, String> {
```

make the OpenCode returns `….map(|()| Route::Typed)` (both `reply_form` and `prompt`), the foreign return `type_line_for(l, f.harness, &tty, &line).map(|()| Route::Typed)`, and replace everything from `let (socket, pid) = {` to the end of the function with:

```rust
    if text.trim().is_empty() {
        return Err("Message is empty.".into());
    }
    if text.chars().count() > answer::TYPED_MAX_CHARS {
        return Err(format!("Message is too long to type (over {} characters).", answer::TYPED_MAX_CHARS));
    }
    let (tty, inbox) = {
        let mut store = l.store.lock().unwrap();
        let card = store.card_for(session_id, now_ms()).ok_or("Session is no longer running.")?;
        answer::check_free(&card)?;
        let s = store.session(session_id).ok_or("Session is no longer running.")?;
        (session_tty(&store, session_id, s.pid).ok(), s.messaging_socket_path.clone().map(|p| (p, s.pid)))
    };
    let typed = match &tty {
        Some(tty) => type_reply(l, tty, &answer::reply_lines(text)),
        None => Err(TypeFail::NotReached("no terminal found for the session".into())),
    };
    let result = match typed {
        Ok(()) => Ok(Route::Typed),
        Err(TypeFail::Partial(e)) => {
            crate::log::line("reply", format!("typing into {session_id} stopped partway: {e}"));
            Err("Part of the reply was typed into the terminal; check it there.".into())
        }
        Err(TypeFail::NotReached(e)) => match inbox {
            Some((socket, pid)) => {
                crate::log::line("reply", format!("could not type into {session_id} ({e}); using its inbox"));
                inbox::send(Path::new(&socket), pid, text).map(|()| Route::Inbox)
            }
            None => Err("Maya can't type into this session's terminal and it has no inbox. Use the terminal.".into()),
        },
    };
    crate::log::line(
        "reply",
        match &result {
            Ok(route) => format!("sent {} characters to {session_id} ({route:?})", text.chars().count()),
            Err(e) => format!("could not send to {session_id}: {e}"),
        },
    );
    result
```

In `session_history`, replace the relabelling `None => { … }` arm with

```rust
        None => Ok(transcript::read_turns(&path, 30)),
```

- [ ] **Step 5: Remove the sent-reply hashes** (`core/src/store.rs`)

Delete the `sent` field and its doc comment, `SENT_KEPT`, `sent_hash`, the `sent: Default::default(),` initialiser, `sent_path`, `note_sent`, `was_sent`, and the store test that exercises them (the one asserting `was_sent`, ~lines 570-595). Remove any import only they used (check `cargo build` warnings).

- [ ] **Step 6: Run the tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core`
Expected: PASS, no warnings about unused items. The existing OpenCode tests still pass (their `send_reply(…).unwrap()` ignores the route).

- [ ] **Step 7: Commit**

```bash
git add core/src/actions.rs core/src/terminal.rs core/src/store.rs
git commit -m "feat(core): type replies into Claude Code's terminal, so they count as the user"
```

---

### Task 3: Carry the route to the page, over the network and on the phone

**Files:**
- Modify: `core/src/actions.rs` (`impl Route`), `core/src/net/client.rs:121` (`reply_with_attachments` generic)
- Modify: `cli/src/executor.rs:57-63`, `src-tauri/src/net_app.rs:166-173`
- Modify: `src-tauri/src/lib.rs:689-696` (`send_reply`), `mobile/src/commands.rs:47-51`
- Modify: `cli/tests/run_end_to_end.rs` (step 5)

**Interfaces:**
- Consumes: `actions::Route`, `actions::send_reply -> Result<Route, String>` (Task 2).
- Produces: `Route::data(self) -> serde_json::Value`, `Route::from_data(data: Option<&serde_json::Value>) -> Option<Route>`; the Tauri command `send_reply` (desktop and phone) resolves to `"typed" | "inbox" | null`.

- [ ] **Step 1: Write the failing test** (`core/src/actions.rs` `mod tests`)

```rust
    #[test]
    fn route_from_data_reads_the_route_and_tolerates_none() {
        assert_eq!(Route::Typed.data(), serde_json::json!({"route": "typed"}));
        assert_eq!(Route::from_data(Some(&Route::Inbox.data())), Some(Route::Inbox));
        assert_eq!(Route::from_data(None), None, "an older Maya sends no data");
        assert_eq!(Route::from_data(Some(&serde_json::json!({"route": "carrier pigeon"}))), None);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core route_from_data`
Expected: FAIL to compile, "no function or associated item named `data`".

- [ ] **Step 3: Implement** (`core/src/actions.rs`, after the `Route` enum)

```rust
impl Route {
    /// The `data` a `Reply` command's result carries.
    pub fn data(self) -> serde_json::Value {
        serde_json::json!({ "route": self })
    }

    /// The route in a `Reply` result's `data`; None from a Maya that sends none.
    pub fn from_data(data: Option<&serde_json::Value>) -> Option<Route> {
        data.and_then(|d| serde_json::from_value(d["route"].clone()).ok())
    }
}
```

`core/src/net/client.rs:121`: make the sender's result generic, body unchanged:

```rust
pub fn reply_with_attachments<T>(maya_dir: &Path, session_exists: bool, text: &str, attachments: Vec<Attachment>, now_ms: u64, send: impl FnOnce(String) -> Result<T, String>) -> Result<T, String> {
```

`cli/src/executor.rs` Reply arm:

```rust
                client::reply_with_attachments(&maya_dir, exists, &text, attachments, now_ms(), |text| actions::send_reply(&l, &session, &text)).map(|route| Some(route.data()))
```

`src-tauri/src/net_app.rs` Reply arm (keep its comment):

```rust
            reply_with_attachments(&maya_dir, exists, &text, attachments, now_ms(), |text| actions::send_reply(&l, &session, &text)).map(|route| Some(route.data()))
```

`src-tauri/src/lib.rs` `send_reply`:

```rust
#[tauri::command(async)]
fn send_reply(app: AppHandle, state: TauriState<AppState>, session_id: String, text: String, attachments: Vec<String>) -> Result<Option<actions::Route>, String> {
    if let Some(machine) = remote_machine_of(&state, &session_id) {
        let attachments = remote_attachments(&attachments)?;
        let data = net_app::send_command(&app, &machine, CommandKind::Reply { session: session_id, text, attachments }, ROUTE_TIMEOUT)?;
        return Ok(actions::Route::from_data(data.as_ref()));
    }
    actions::send_reply(&local(&state), &session_id, &text).map(Some)
}
```

(`src-tauri/src/listener.rs:194` calls `send_reply(…)?;` and keeps compiling.)

`mobile/src/commands.rs` `send_reply`:

```rust
#[tauri::command(async)]
pub fn send_reply(hub: H, session_id: String, text: String, attachments: Vec<String>) -> Result<Option<maya_core::actions::Route>, String> {
    let attachments = remote_attachments(&attachments)?;
    hub.send(&session_id, |session| CommandKind::Reply { session, text, attachments }).map(|d| maya_core::actions::Route::from_data(d.as_ref()))
}
```

(Use whatever path `mobile/src/commands.rs` already imports `maya_core` items by; add `use maya_core::actions::Route;` if it has a `use maya_core::…` block.)

- [ ] **Step 4: Update the CLI end-to-end test** (`cli/tests/run_end_to_end.rs`)

The reply is now typed through the fake terminal. Remove the inbox listener: the `use std::io::Read;` and `use std::os::unix::net::UnixListener;` imports, the `socket`/`inbox`/`entry_path`/`entry` lines that bind it and write `messagingSocketPath` (keep `claude_dir` and `maya_dir`), and replace step 5 with:

```rust
    // 5. A reply from the main is typed into the session's terminal, as the user.
    let reply = CommandKind::Reply { session: "s1".into(), text: "hello".into(), attachments: vec![] };
    assert_eq!(send_command_with(&handle.shared, "box", reply, Duration::from_secs(5)), Ok(Some(serde_json::json!({"route": "typed"}))));
    assert!(fake.calls.lock().unwrap().iter().any(|c| matches!(c, Call::Type { tty, text } if tty == "/dev/pts/3" && text == "hello")));
```

Update the file's opening doc comment: "reply into the session's inbox socket" → "type a reply into the session's terminal".

- [ ] **Step 5: Run the tests and builds**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core -p maya-cli && PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-mobile && PATH="$HOME/.cargo/bin:$PATH" cargo build -p maya`
Expected: PASS and a clean build. (If the Tauri crate is not named `maya`, use the `name` in `src-tauri/Cargo.toml`.)

- [ ] **Step 6: Commit**

```bash
git add core/src/actions.rs core/src/net/client.rs cli/src/executor.rs cli/tests/run_end_to_end.rs src-tauri/src/net_app.rs src-tauri/src/lib.rs mobile/src/commands.rs
git commit -m "feat: report whether a reply went in as the user or through the inbox"
```

---

### Task 4: Notices and pasted text in the card history

**Files:**
- Modify: `core/src/transcript.rs` (`TurnKind`, `parse_turns` user branch ~line 203-218, `drop_narration` ~line 183, tests)

**Interfaces:**
- Produces: `TurnKind::Notice` (serialised `"notice"`), whose text is "Earlier conversation compacted" or "Interrupted".

- [ ] **Step 1: Write the failing tests** (in `core/src/transcript.rs`'s `mod tests`, using its `user_line` and `assistant_line` helpers)

```rust
    #[test]
    fn a_compaction_summary_is_a_notice_not_the_users_words() {
        let mut summary: serde_json::Value = serde_json::from_str(&user_line("This session is being continued from a previous conversation that ran out of context. The summary below…", false)).unwrap();
        summary["isCompactSummary"] = true.into();
        summary["isVisibleInTranscriptOnly"] = true.into();
        let text = [user_line("old ask", false), assistant_line("old answer", None), summary.to_string(), user_line("new ask", false)].join("\n");
        assert_eq!(parse_turns(&text, 30), vec![
            turn(TurnKind::User, "old ask"),
            turn(TurnKind::Assistant, "old answer"),
            turn(TurnKind::Notice, "Earlier conversation compacted"),
            turn(TurnKind::User, "new ask"),
        ]);
    }

    #[test]
    fn an_interrupt_is_a_notice_and_keeps_claudes_last_words() {
        let text = [
            user_line("fix A", false),
            assistant_line("Looking at A.", None),
            assistant_line("Found it in a.rs.", None),
            user_line("[Request interrupted by user]", false),
            user_line("[Request interrupted by user for tool use]", false),
            user_line("carry on", false),
        ]
        .join("\n");
        assert_eq!(parse_turns(&text, 30), vec![
            turn(TurnKind::User, "fix A"),
            turn(TurnKind::Assistant, "Found it in a.rs."),
            turn(TurnKind::Notice, "Interrupted"),
            turn(TurnKind::Notice, "Interrupted"),
            turn(TurnKind::User, "carry on"),
        ]);
    }

    #[test]
    fn pasted_text_loses_its_tags_and_keeps_what_was_typed_around_it() {
        let pasted = "<pasted_content id=\"0b51\">\nLets reorganise the Settings page.\n</pasted_content>";
        let text = [user_line(pasted, false), user_line(&format!("Do this:\n{pasted}\nthanks"), false)].join("\n");
        assert_eq!(parse_turns(&text, 30), vec![
            turn(TurnKind::User, "Lets reorganise the Settings page."),
            turn(TurnKind::User, "Do this:\n\nLets reorganise the Settings page.\n\nthanks"),
        ]);
    }
```

If the module's tests have no `turn(kind, text)` helper (one is used at line ~458), add it: `fn turn(kind: TurnKind, text: &str) -> Turn { Turn { kind, text: text.into() } }`.

- [ ] **Step 2: Run them to see them fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core transcript::tests`
Expected: FAIL to compile, "no variant named `Notice`".

- [ ] **Step 3: Implement** (`core/src/transcript.rs`)

Add the variant:

```rust
    /// Something Claude Code noted in the conversation: a compaction or an interrupt.
    Notice,
```

Add the helpers near `unwrap_peer_message`:

```rust
/// Claude Code's line when the user pressed Esc, sometimes with " for tool use".
const INTERRUPTED: &str = "[Request interrupted by user";

/// The user's text without Claude Code's `<pasted_content …>` wrappers.
fn strip_paste_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<pasted_content") {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = &rest[i..];
                break;
            }
        }
    }
    out.push_str(rest);
    out.replace("</pasted_content>", "").trim().to_string()
}

fn notice(text: &str) -> Item {
    Item::Turn(Turn { kind: TurnKind::Notice, text: text.into() })
}
```

In `parse_turns`'s `Some("user")` arm, before `let t = text_blocks(content);`:

```rust
                if v["isCompactSummary"].as_bool() == Some(true) {
                    items.push(notice("Earlier conversation compacted"));
                    continue;
                }
```

and in the `match unwrap_peer_message(&text)`, before `None if meta => {}`:

```rust
                        None if text.starts_with(INTERRUPTED) => items.push(notice("Interrupted")),
```

and replace the last arm with:

```rust
                        None => {
                            let text = strip_paste_tags(&text);
                            if !text.is_empty() {
                                items.push(Item::Turn(Turn { kind: TurnKind::User, text }));
                            }
                        }
```

In `drop_narration`: `TurnKind::User | TurnKind::Peer | TurnKind::Notice => last_text_in_run = None,`.

Update `parse_turns`'s doc comment to: "… harness-injected user lines (skill bodies, notifications, subagent hand-backs) and progress narration are skipped; a compaction or an interrupt is a notice."

- [ ] **Step 4: Run the tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p maya-core`
Expected: PASS, including the existing `harness_lines_are_not_turns` and `a_hidden_command_still_ends_an_assistant_run`.

- [ ] **Step 5: Commit**

```bash
git add core/src/transcript.rs
git commit -m "feat(core): show compactions and interrupts as notices and unwrap pasted text"
```

---

### Task 5: The reply box and the history on the page

**Files:**
- Modify: `src/types.ts:135-137` (`Turn.kind`)
- Modify: `src/modal.ts` (model `status` type ~line 25; banner ~lines 486-488; turns ~line 510; composer ~lines 528-590; status line ~592; `paint` ~line 772; `guardedSend` ~lines 840-855)
- Modify: `src/styles.css:131-134`
- Test: `src/modal.test.ts`

**Interfaces:**
- Consumes: the `send_reply` command resolving to `"typed" | "inbox" | null` (Task 3); `Turn.kind` `"notice"` (Task 4).
- Produces: `export function replyStatus(route: "typed" | "inbox" | null): { ok: boolean; text: string; warn?: boolean }` in `src/modal.ts`.

- [ ] **Step 1: Write the failing tests** (`src/modal.test.ts`; add `replyStatus` to the import from `./modal`)

Change the existing "shows header, turns with kind classes and a composer" expectation of the labels to:

```ts
    expect([...el.querySelectorAll(".turn__who")].map((w) => w.textContent)).toEqual(["You", "Claude", "From another session"]);
```

Change "still offers the tweaks row when the session has no inbox" to expect a composer, and rename it:

```ts
  it("offers the tweaks row and the composer when a Claude session has no inbox", () => {
    const el = renderModal({ card: { ...base, hasInbox: false }, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector(".modal__tweaks")).not.toBeNull();
    expect(el.querySelector("textarea")).not.toBeNull();
    expect(el.querySelector(".modal__noinbox")).toBeNull();
  });
```

Add:

```ts
describe("replyStatus", () => {
  it("says whether a reply went in as the user or as another session", () => {
    expect(replyStatus("typed")).toEqual({ ok: true, text: "Sent as you" });
    expect(replyStatus("inbox")).toEqual({ ok: true, warn: true, text: "Sent as a message from another session: it can't approve anything" });
    expect(replyStatus(null)).toEqual({ ok: true, text: "Delivered" });
  });
});

describe("renderModal notices and statuses", () => {
  it("shows a notice as a line with no speaker", () => {
    const el = renderModal({ card: base, turns: [{ kind: "notice", text: "Interrupted" }], status: null, draft: "" }, handlers());
    const t = el.querySelector(".turn--notice")!;
    expect(t.querySelector(".turn__who")).toBeNull();
    expect(t.textContent).toBe("Interrupted");
  });

  it("colours an inbox fallback as a warning", () => {
    const el = renderModal({ card: base, turns: [], status: replyStatus("inbox"), draft: "" }, handlers());
    expect(el.querySelector(".modal__status")?.className).toBe("modal__status modal__status--warn");
  });

  it("tells the user to answer a pending decision before replying", () => {
    const awaiting = { ...base, state: "awaiting" as const, awaiting: { kind: "permission" as const, detail: "Bash: git push", questions: [] } };
    const el = renderModal({ card: awaiting, turns: [], status: null, draft: "" }, handlers());
    expect(el.querySelector(".modal__banner")?.textContent).toContain("Answer it there before replying.");
  });
});
```

(If `"permission"` is not an `AwaitKind` in `src/types.ts`, use the kind `src/patch.test.ts:38` uses.)

- [ ] **Step 2: Run them to see them fail**

Run: `pnpm vitest run src/modal.test.ts`
Expected: FAIL: `replyStatus` is not exported; labels read "Message"; no textarea without an inbox.

- [ ] **Step 3: Implement**

`src/types.ts`:

```ts
  kind: "user" | "assistant" | "tool" | "peer" | "notice";
```

`src/modal.ts`, the model's status type (~line 25):

```ts
  status: { ok: boolean; text: string; warn?: boolean } | null;
```

Add, near `setStatus`:

```ts
/** The status line after a reply, by how it reached the session. */
export function replyStatus(route: "typed" | "inbox" | null): { ok: boolean; text: string; warn?: boolean } {
  if (route === "typed") return { ok: true, text: "Sent as you" };
  if (route === "inbox") return { ok: true, warn: true, text: "Sent as a message from another session: it can't approve anything" };
  return { ok: true, text: "Delivered" };
}
```

Banner, the two "queue behind it" strings:

```ts
          ? `This session is waiting for a decision on ${m.card.machine}. Answer it there before replying.`
          : "This session is waiting for a decision in its terminal. Answer it there before replying.";
```

Turns:

```ts
    const who = { user: "You", assistant: "Claude", peer: "From another session", tool: "", notice: "" }[t.kind];
    if (who) turn.append(el("div", "turn__who", who));
    const text = el("div", "turn__text");
    if (t.kind === "tool" || t.kind === "notice") text.textContent = t.text;
```

Composer: replace `// Other harnesses have no inbox; …` and `if (m.card.hasInbox || !claude) {` with a plain block (every card now has a composer: Claude replies are typed, with the inbox as fallback), and delete the `} else { panel.append(el("div", "modal__noinbox", …)); }` branch. Keep the composer's body as is.

Status line:

```ts
  if (m.status) panel.append(el("div", `modal__status modal__status--${m.status.warn ? "warn" : m.status.ok ? "ok" : "error"}`, m.status.text));
```

`paint` (~line 772): every modal now has a composer:

```ts
  const composerUnchanged = !!existing && !!existing.querySelector("textarea");
```

`guardedSend`:

```ts
    const route = await invoke<"typed" | "inbox" | null>("send_reply", { sessionId: card.sessionId, text, attachments: current.model.attachments?.map((a) => a.path) ?? [] });
    …
    current.model.status = replyStatus(route ?? null);
```

`src/styles.css` (replace the `.turn--peer` rule, add the others beside the status rules):

```css
.modal__status--warn { color: #d97706; }
.turn--peer { align-self: flex-start; background: transparent; border-left: 3px solid var(--muted); }
.turn--peer .turn__text { color: var(--muted); }
.turn--notice { align-self: center; background: transparent; font-size: 11.5px; color: var(--muted); padding: 2px 8px; }
```

- [ ] **Step 4: Run the tests and the type check**

Run: `pnpm test && pnpm exec tsc --noEmit`
Expected: PASS. `src/patch.test.ts` ("Delivered" as a status) still passes: the status shape only gained an optional field.

- [ ] **Step 5: Commit**

```bash
git add src/types.ts src/modal.ts src/styles.css src/modal.test.ts
git commit -m "feat(ui): say whether a reply went in as you, and label other sessions and notices"
```

---

### Task 6: Docs, version and the live check

**Files:**
- Modify: `README.md:385-393` ("Replies"), `docs/DEVELOPING.md` (the macOS/Linux/Windows "Inbox." bullets, ~lines 63-71 and 131-132)
- Modify (via script): `package.json`, `src-tauri/tauri.conf.json`, `mobile/tauri.conf.json`, `Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Update the README's Replies bullet** to:

```markdown
- **Replies** are typed into the session's terminal, so the agent takes
  them as yours: a Claude Code session can be approved from Maya or the
  phone. A multi-line reply to Claude Code is typed line by line with its
  `\` continuation. When Maya cannot type into a Claude Code session (one
  in iTerm, VS Code or Ghostty, or over SSH outside tmux) it posts the
  reply to the session's inbox instead, where Claude takes it as a message
  from another session that cannot approve anything, and the reply box
  says so. Slash commands, answers, renames and the model, effort and mode
  changes are typed into the terminal too. On Windows every inbox
  connection must open with the session's token, …
```

keeping the rest of the bullet (the Windows token, Terminal.app, Windows Terminal and tmux sentences) as it is.

- [ ] **Step 2: Update `docs/DEVELOPING.md`**: in each platform's "Inbox." bullet add, after its first sentence, "It is the fallback for replies: `actions::send_reply` types a Claude Code reply into the terminal (`answer::reply_lines`) and uses the inbox only when nothing reached the terminal." Do it once per bullet; keep the rest.

- [ ] **Step 3: Run everything**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace && pnpm test`
Expected: PASS.

- [ ] **Step 4: Commit the docs**

```bash
git add README.md docs/DEVELOPING.md
git commit -m "docs: replies are typed as the user, the inbox is the fallback"
```

- [ ] **Step 5: Bump the version**

```bash
PATH="$HOME/.cargo/bin:$PATH" sh scripts/set-version.sh 0.15.0
git diff --stat   # Cargo.lock must be among the changed files
git commit -am "chore(release): 0.15.0"
```

- [ ] **Step 6: Live check, by the user** (Claude cannot start agent sessions here). Build and run the debug app (see `docs/DEVELOPING.md`), then ask the user to:
  1. Start a Claude Code session in Terminal.app from Maya, and in it ask: "Ask me in prose whether to run `ls`, then wait".
  2. From the card's reply box send a three-line reply whose middle line ends in `\` (e.g. `yes\npath C:\\\nthanks`). Confirm the session shows one user message with three lines, and note what the middle line became; if Claude Code dropped a backslash, adjust `reply_lines` and its test in Task 1 before merging.
  3. From the phone, reply "approved" to a prose approval question; confirm Claude acts on it as the user's approval, and the reply box says "Sent as you".
  4. Optionally start a session in iTerm or VS Code and reply: the box says "Sent as a message from another session: it can't approve anything".
