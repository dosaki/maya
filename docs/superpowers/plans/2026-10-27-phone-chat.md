# The phone is always a mirror, and gets Chat — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the phone-as-main mode, and give a joined phone a live Chat tab that is the main computer's Chat.

**Architecture:** The main pushes two new frames, `Down::Chat` and `Down::Brain`, to mirroring links that announced `chat`. The phone fetches the history with a new `Request::ChatHistory` and sends Send, Seen, Cancel and Stop as requests. The phone's crate serves the desktop's chat command names and emits the desktop's `chat` and `brain` events, so `src/chat.ts` is shared. The phone loses its server, pairing code, assistants list and Make main.

**Tech Stack:** Rust (Cargo workspace: `core/`, `src-tauri/`, `mobile/`), Tauri 2, vanilla TypeScript with vitest.

**Spec:** `docs/superpowers/specs/2026-10-26-phone-chat-design.md`

## Global Constraints

- `PROTOCOL` stays at 2. Every change to the wire is additive and serde-defaulted.
- `CAP_CHAT` is the string `"chat"`. `phone_caps()` is `follow`, `mirror` and `chat`, and no longer `lead`.
- `Down::Chat` and `Down::Brain` go only to links where `wants_mirror && wants_chat && may_mirror`.
- Typed phone messages use `Via::Chat`.
- The copy is exact: `"A phone cannot be the main."`, `"Update {main} to chat from the phone."`, `"Reconnecting to {main}…"`, `"Joined to {main}"`.
- Every commit builds, and `cargo test --workspace` and `pnpm test` pass.
- Commit messages follow Conventional Commits, and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- The release is `0.35.0`, set with `sh scripts/set-version.sh 0.35.0` in its own `chore(release): 0.35.0` commit, as the last commit.

## Review Focus

1. **The switch goes off while P is on Chat.** No further `Chat` or `Brain` frame reaches P, P's chat requests are refused, and the tab hides. Tested in Task 2 and Task 7.
2. **P reconnects after missing turns.** The history is fetched again and replaces the turns on the page; there are no duplicates and no gaps. Tested in Task 6 and Task 7.
3. **The history fetch from inside a client callback.** `connected` and `main_caps` run on the client thread, and that thread also answers the lane, so a fetch made there would deadlock. Tested in Task 6 by a real round trip that must finish within `wait_until`.
4. **A phone saved as `Main` from 0.34.** It loads as `Off` with no assistants, starts no server and shows Join. Tested in Task 5.
5. **An older phone that sends `lead`.** It gets no Make main on the desktop, and `promote` refuses it. Tested in Task 3.

---

### Task 1: Chat on the wire (core protocol)

**Files:**
- Modify: `core/src/net/protocol.rs` (constants near line 27, `Down`, `Request`, and the tests)

**Interfaces:**
- Produces:
  - `pub const CAP_CHAT: &str = "chat";`
  - `Down::Chat { turn: serde_json::Value }` (wire type `"chat"`)
  - `Down::Brain { status: serde_json::Value }` (wire type `"brain"`)
  - `Request::ChatHistory`, `Request::ChatSend { text: String }`, `Request::ChatSeen`, `Request::WindowCancel` and `Request::BrainStop` (snake_case, as `Request` already is)

- [ ] **Step 1: Write the failing test** `chat_frames_and_requests_round_trip` in the protocol tests:

```rust
let turn = serde_json::json!({"who": "maya", "text": "Done.", "at": 5, "via": "chat"});
let chat = Down::Chat { turn: turn.clone() };
assert_eq!(decode_down(&encode(&chat)).unwrap(), chat);
assert!(encode(&chat).contains(r#""type":"chat""#));
let brain = Down::Brain { status: serde_json::json!({"dot": "working", "pending": null, "job": "x", "ready": true}) };
assert_eq!(decode_down(&encode(&brain)).unwrap(), brain);
for r in [Request::ChatHistory, Request::ChatSend { text: "hi".into() }, Request::ChatSeen, Request::WindowCancel, Request::BrainStop] {
    let up = Up::Request { id: 1, machine: None, request: r };
    assert_eq!(decode_up(&encode(&up)).unwrap(), up);
}
assert_eq!(CAP_CHAT, "chat");
```

- [ ] **Step 2: Run it.** Run `cargo test -p maya_core protocol::tests::chat_frames_and_requests_round_trip`. It should fail to compile, with no `Down::Chat`.
- [ ] **Step 3: Add the constant, the two `Down` variants and the five `Request` variants.** Put a doc line on each, in the file's style.
- [ ] **Step 4: Fix the non-exhaustive matches** the compiler reports:
  - In `core/src/net/client.rs`, `Down::Chat` and `Down::Brain` are handled in Task 4. For now, add a match arm that only logs.
  - In `src-tauri/src/net_app.rs::serve_request`, add a temporary arm that returns `Err("Not yet.")`, which Task 2 replaces.
  - In `mobile/src/hub.rs::serve_request`, refuse the new requests. That code is removed in Task 5.
- [ ] **Step 5: Run the tests.** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 6: Commit.** `feat(net): add chat frames and requests to the protocol`

### Task 2: The main serves Chat to phones (core server + desktop)

**Files:**
- Modify: `core/src/net/server.rs` (`ServerHandle`, its tests)
- Modify: `src-tauri/src/lib.rs` (`main_caps`)
- Modify: `src-tauri/src/listener.rs` (`remember`)
- Modify: `src-tauri/src/brain_app.rs` (`emit_status`)
- Modify: `src-tauri/src/net_app.rs` (`serve_request`, a new helper, tests)

**Interfaces:**
- Consumes: Task 1's frames, requests and `CAP_CHAT`.
- Produces:
  - `ServerHandle::to_mirrors(&self, frame: Down)`: sends to every connected link whose peer caps hold both `CAP_MIRROR` and `CAP_CHAT` and whose entry `may_mirror`, through `conns[id].tx`.
  - `net_app::to_mirrors(app: &AppHandle, frame: Down)`: clones the server handle out of `state.network`, with no other lock held, and calls the method above.

- [ ] **Step 1: Write the failing server test** `chat_frames_reach_only_allowed_mirrors_that_asked_for_chat`. Use the existing helpers `pair_client_caps` and `recv`:
  - Pair `phone` with caps `["mirror", "chat"]`, `old` with `["mirror"]`, and `laptop` with `["lead"]`.
  - Call `handle.to_mirrors(Down::Chat { turn: json!({"text": "a"}) })`.
  - Assert that `phone` receives it (skip any `Mirror` frame first), and that `old` and `laptop` receive nothing within 300 ms.
  - Call `set_mirror(phone_id, false)`, then call `to_mirrors` again. Assert that `phone` gets only `Mirror { allowed: false, .. }` and no `Chat`.
- [ ] **Step 2: Run it.** Run `cargo test -p maya_core server::tests::chat_frames_reach`. It should fail, with no `to_mirrors`.
- [ ] **Step 3: Implement `ServerHandle::to_mirrors`.**
- [ ] **Step 4: Run it.** It should pass.
- [ ] **Step 5: Write the failing desktop test** `chat_requests_are_answered_as_their_commands`. Add `pub(crate) fn chat_request_kind(r: &Request) -> Option<&'static str>`, a pure mapping that `serve_request` uses, and assert that it returns `Some("chat_history")`, `Some("chat_send")`, `Some("chat_seen")`, `Some("window_cancel")` and `Some("brain_stop")` for the five requests, and `None` for `Request::ListDirs`.
- [ ] **Step 6: Implement the five arms of `serve_request`.** Each calls the existing command function:
  - `ChatHistory` returns `{ "turns": listener::chat_history(..), "status": brain_app::status(app) }`.
  - `ChatSend` calls `listener::chat_send(app.clone(), text)`.
  - `ChatSeen` calls `listener::chat_seen`.
  - `WindowCancel` calls `listener::window_cancel`.
  - `BrainStop` calls `brain_app::brain_stop`.

  `chat_history` takes a `TauriState`, so either pass `app.state()` or read `state.chat` directly.
- [ ] **Step 7: Push the frames.**
  - In `listener::remember`, after the `chat` emit, call `net_app::to_mirrors(app, Down::Chat { turn: serde_json::to_value(&turn) })`. Build the `ChatTurn` once and use it for both.
  - In `brain_app::emit_status`, compute `status(app)` once, emit it, then call `to_mirrors(app, Down::Brain { status: to_value(&status) })`.
- [ ] **Step 8: Advertise chat.** `main_caps` always includes `CAP_CHAT`, because a main always runs a brain. `CAP_VOICE` stays conditional, as now.
- [ ] **Step 9: Run the tests.** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 10: Commit.** `feat: the main sends its Chat to phones that ask for it`

### Task 3: The desktop never makes a phone the main

**Files:**
- Modify: `core/src/net/server.rs` (`promote_inner`, `HandoverTo`, `set_own_mirror`, `Ctx.own_mirror`, the tests near line 1781 and 1815)
- Modify: `core/src/net/promote.rs` (`own_entry` and its test `only_a_phone_s_own_entry_may_mirror`)
- Modify: `src-tauri/src/brain_app.rs:805` (`handover_summary`)
- Modify: `src-tauri/src/brain_route.rs:270-286` (`SummarySource`, `summary_source`, and the tests near line 681)
- Modify: `src/settings.ts:1450` (the Make main condition), and `src/settings.test.ts`

**Interfaces:**
- Produces:
  - `own_entry(main_name: &str, key: &str) -> PairedAssistant` (always `mirror: false`)
  - `HandoverTo { label, brain_dormant_since }`
  - `summary_source(brain_ready: bool) -> SummarySource` (`Ask` or `Digest`; the `None` variant is removed)

- [ ] **Step 1: Write the failing server test** `promote_refuses_a_maya_that_mirrors`. Pair a link with caps `["follow", "lead", "mirror"]`, then call `handle.promote(&id)`. Assert `Err("A phone cannot be the main.")`, and that `handle.status()` still shows that link paired and connected.
- [ ] **Step 2: Run it.** It should fail, because the handover is attempted.
- [ ] **Step 3: Implement the refusal.** In `promote_inner`, before the `CAP_LEAD` check, refuse when `entry.mirror` or the peer's caps hold `CAP_MIRROR`. Remove `HandoverTo.phone`, `set_own_mirror`, `Ctx.own_mirror` and the `mirror` parameter of `own_entry`. Update the callers and the tests the compiler shows. The test at server.rs:1781 that calls `set_own_mirror(true)` goes.
- [ ] **Step 4: Simplify the summary source.** Change it to `summary_source(brain_ready)`, and update `handover_summary` and the tests at brain_route.rs:681.
- [ ] **Step 5: Write the failing page test** in `src/settings.test.ts`: `a row that mirrors gets the switch but never Make main`. Use a connected assistant row with `canLead: true, wantsMirror: true`. Assert that there is no `[data-action=make-main]` and there is a `[data-action=mirror]`.
- [ ] **Step 6: Change the condition** at `src/settings.ts:1450` to `a.connected && a.canLead && !(a.wantsMirror || a.mirror)`.
- [ ] **Step 7: Run the tests.** Run `cargo test --workspace && pnpm test`. Expected: PASS.
- [ ] **Step 8: Commit.** `feat!: a phone can no longer be made the main`

### Task 4: The client hands chat frames to its app (core client)

**Files:**
- Modify: `core/src/net/client.rs` (`ClientNotify`, the frame dispatch near line 667, tests)
- Modify: `core/src/net/mod.rs:91` (`AssistantLink`)

**Interfaces:**
- Produces:
  - `ClientNotify::chat_turn(&self, _turn: Value) {}` and `ClientNotify::brain_status(&self, _status: Value) {}`, defaulted, called for `Down::Chat` and `Down::Brain`
  - `AssistantLink.chat: bool`: the main sent `CAP_CHAT` at the last welcome (serialized as `chat`, as the struct's other fields are)

- [ ] **Step 1: Write the failing client test** `chat_and_brain_frames_reach_the_app`. Follow the file's existing pattern of a recording `ClientNotify` and a scripted main. Send `Down::Chat { turn }` and then `Down::Brain { status }`, and assert that the recorder saw that turn and then that status, in order.
- [ ] **Step 2: Run it.** It should fail.
- [ ] **Step 3: Add the two trait methods and replace Task 1's logging arm** with calls to them. Add `chat: bool` to `AssistantLink`, with `Default` false.
- [ ] **Step 4: Run the tests.** Run `cargo test -p maya_core`. Expected: PASS.
- [ ] **Step 5: Commit.** `feat(net): pass chat frames from the main to the app`

### Task 5: The phone never runs a server (mobile crate)

**Files:**
- Modify: `mobile/src/hub.rs`: remove `server`, `main_error`, `start_server`, `stop_server`, `pairing_code`, `remove_assistant`, `set_mirror`, `promote`, `serve_request`, the server `Notify` impl, the `NoSessions` executor's server use, and the main branch of `status`; also `phone_caps()`, `refresh()` (the `s.mirror` and `s.mirror_remembered` calls), and the module doc.
- Modify: `mobile/src/commands.rs`: remove `server_start`, `server_stop`, `network_pairing_code`, `network_remove_assistant`, `network_promote` and `network_set_mirror`.
- Modify: `mobile/src/lib.rs`: the `generate_handler!` list, and the `was_main` start-up branch.
- Modify: `mobile/src/alerts.rs:158`: `service_line` for the joined phone only.
- Modify: `core/src/config.rs` only if a helper is needed. Prefer doing the migration in `hub::load_settings`.
- Test: `mobile/tests/round_trip.rs`, and the unit tests in `hub.rs`.

**Interfaces:**
- Produces:
  - `hub::load_settings(maya_dir, model) -> Settings`: a saved `NetworkRole::Main` loads as `Off`, with `network.assistants` emptied, the change saved, and one log line: `"this phone is no longer a main: join a main computer"`.
  - `phone_caps() -> Vec<String>` is `[CAP_FOLLOW, CAP_MIRROR, CAP_CHAT]`.
  - `alerts::service_line(main: &str, connected: bool) -> String`: `"Joined to {main}"` or `"Reconnecting to {main}…"`.

- [ ] **Step 1: Write the failing test** `a_phone_saved_as_main_loads_off_with_no_assistants` in `hub.rs`. Write a config with `role: Main` and one assistant to a temporary directory, then call `load_settings`. Assert that `role == Off`, that `assistants` is empty, and that the file on disk says the same.
- [ ] **Step 2: Write the failing test** `the_phone_asks_to_follow_mirror_and_chat_but_not_lead`, which asserts `phone_caps()`.
- [ ] **Step 3: Write the failing test** `the_service_line_names_the_main` in `alerts.rs`, for both strings.
- [ ] **Step 4: Run them.** Run `cargo test -p maya-mobile`. They should fail.
- [ ] **Step 5: Implement the three items, then remove the phone-main code** listed under Files. Let the compiler lead.
- [ ] **Step 6: Update `round_trip.rs`:**
  - Delete the tests that need the phone as a main: `pairs_merges_a_board_…`, `a_taken_port_…`, `a_refresh_with_no_new_frame_…`, `the_phone_hands_the_main_role_over_…`, `a_phone_main_mirrors_its_board_…` and `a_join_with_a_wrong_code_puts_a_main_phone_s_server_back`.
  - Keep the joined-phone tests, and change what no longer compiles.
- [ ] **Step 7: Run the tests.** Run `cargo test --workspace`. Expected: PASS. Then run `grep -rn "net::server" mobile/src`. Expected: no `start`, `start_with` or `ServerHandle`.
- [ ] **Step 8: Commit.** `feat(mobile)!: the phone only joins a main`

### Task 6: The phone's crate serves Chat (mobile crate)

**Files:**
- Modify: `mobile/src/hub.rs`: a `chat` field, the `ClientNotify` methods, the fetch on connect, and `chat_status()`.
- Modify: `mobile/src/commands.rs`: six commands.
- Modify: `mobile/src/lib.rs`: the `Sink` methods on `TauriSink`, and the handler list.
- Test: `mobile/tests/round_trip.rs`

**Interfaces:**
- Consumes: Task 1's requests, Task 4's `chat_turn`, `brain_status` and `AssistantLink.chat`, and Task 2's main behaviour.
- Produces:
  - `pub struct ChatView { pub turns: Vec<Value>, pub status: Option<Value> }` (`Default`), held as `Hub.chat: Mutex<ChatView>`, and taken alone, like `mirror`.
  - `Sink::chat(&self, _turn: &Value) {}`, `Sink::brain(&self, _status: &Value) {}` and `Sink::chat_reset(&self, _turns: &[Value]) {}`. `TauriSink` emits `"chat"`, `"brain"` and `"chat-reset"`.
  - `Hub::chat_status(&self) -> Value`:
    - when not connected: `{dot: "idle", pending: null, job: null, ready: false, dormant: "Reconnecting to {main}…"}`;
    - when connected without `link.chat`: the same shape, with `dormant: "Update {main} to chat from the phone."`;
    - otherwise: the stored status, or the idle shape with `dormant: null` before one arrives.
  - The commands are `chat_history(hub) -> Vec<Value>`, `brain_status(hub) -> Value`, `chat_send(hub, text: String) -> Result<(), String>`, `chat_seen(hub)`, `window_cancel(hub)` and `brain_stop(hub)`. The last four send their `Request` with `hub.request(None, …)`. `chat_seen`, `window_cancel` and `brain_stop` ignore errors.

- [ ] **Step 1: Write the failing round-trip test** `a_joined_phone_gets_the_chat_live_and_again_after_a_reconnect`. Use `desk()`, and give `MainNotify` a `main_caps` returning `["chat"]`, and a `request` that answers `ChatHistory` with `{"turns": [{"text": "old"}], "status": {"dot": "idle"}}`.
  - Join, then `wait_until` the page's recorded `chat_reset` holds `[{"text":"old"}]`.
  - Call `main.to_mirrors(Down::Chat { turn: json!({"text": "new"}) })`, then `wait_until` the page's `chat` events hold `"new"`, and `hub.chat.lock().unwrap().turns.len() == 2`.
  - Call `hub.request(None, Request::ChatSend { text: "hi".into() })`, then assert that `notify.requests` holds `(None, Request::ChatSend { text: "hi" })`.
  - Stop the server and start one again on the same key and port with `desk_with`. `wait_until` a second `chat_reset`, and assert that `turns` is the fetched list (length 1), not appended to.
  - `Page` (the test's `Sink`) records the three new calls.
- [ ] **Step 2: Write the failing unit test** `the_chat_status_says_why_the_phone_cannot_chat`, which covers the three cases of `chat_status` using the `joined_override` and `link` setters the hub tests already use.
- [ ] **Step 3: Run them.** Run `cargo test -p maya-mobile`. They should fail.
- [ ] **Step 4: Implement.**
  - In `main_caps`, set `link.chat`.
  - When `link.chat` is set on a connect, **spawn a thread**: the client thread answers the lane, so it must not wait on it. The thread sends `ChatHistory`, stores the turns and status, then calls `sink.chat_reset` and `sink.brain`.
  - `chat_turn` pushes onto `ChatView.turns` (keeping at most 500) and calls `sink.chat`.
  - `brain_status` stores the status and calls `sink.brain(&self.chat_status())`.
  - `connected`, `disconnected` and `mirrored(false, …)` call `sink.brain(&chat_status())`, so the note follows the link.
  - On `mirrored(false, …)`, also clear `ChatView`.
- [ ] **Step 5: Run the tests.** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 6: Commit.** `feat(mobile): serve the main's Chat to the phone's page`

### Task 7: The phone's page: Join only, and a Chat tab

**Files:**
- Modify: `src/chat.ts`: a `chat-reset` listener, and a Send button option.
- Modify: `src/chat.test.ts` and `src/chat-flow.test.ts`.
- Modify: `mobile/web/index.html`: the Chat tab and `<section id="chat" class="pane" hidden>`.
- Modify: `src/mobile/main.ts`: `initChat`, the tab visibility, and removing `paintHint`.
- Modify: `src/mobile/setup.ts` and `src/mobile/setup.test.ts`: Join only.
- Modify: `src/mobile/network.ts`: remove the pairing, assistants, Start and Stop, and Make main from the model, the handlers and the render.

**Interfaces:**
- Consumes: Task 6's commands and events.
- Produces:
  - `renderChat(m: ChatModel, h: ChatHandlers)`, where `ChatModel` gains `sendButton?: boolean`. When it is true, the bar has `<button class="chat__send" data-action="chat-send">Send</button>`, which sends what the keys would send.
  - `initChat(opts?: { sendButton?: boolean })`
  - `export function chatTabShown(s: NetworkStatus): boolean` in `src/mobile/main.ts` (or a small `src/mobile/chat-tab.ts` if `main.ts` cannot be imported in tests): true only when `s.role === "assistant"` and `s.assistant.mirror === "allowed"`.

- [ ] **Step 1: Write the failing test** in `chat-flow.test.ts`: `a chat-reset replaces the turns`.
  - Call `initChat()`, fire `chat` with `{text: "a"}`, then fire `chat-reset` with `[{text: "b"}, {text: "c"}]`.
  - Assert that `.chat__turn` texts are `["b", "c"]`.
- [ ] **Step 2: Write the failing test** in `chat.test.ts`: `a Send button sends the trimmed text and clears the box`.
  - Render with `sendButton: true` and draft `" hi "`, then click `[data-action=chat-send]`.
  - Assert that `onSend` was called with `"hi"`.
  - Also assert that without `sendButton` there is no such button.
- [ ] **Step 3: Write the failing test** in `setup.test.ts`, replacing `offers Be the main or Join a main…`: `offers only Join on the first run, and joins with what was typed`. Assert that there is no `[data-mode]` choice and no Start button, and that Join calls `onJoin` with the fields.
- [ ] **Step 4: Write the failing test** `chatTabShown` for: assistant + allowed is true; assistant + refused, assistant + unsupported, and role off are all false.
- [ ] **Step 5: Run the tests.** Run `pnpm test`. The new tests should fail.
- [ ] **Step 6: Implement.**
  - `chat.ts`: add the reset listener and the Send button.
  - `index.html`: add the Chat tab after Sessions, with the `tab__dot`, before the spacer.
  - `main.ts`:
    - call `initChat({ sendButton: true })`;
    - in the `initNetwork` callback, set the Chat tab button's `hidden` to `!chatTabShown(s)`, and `tabs.show("sessions")` if Chat was showing and is now hidden;
    - add the `chat` pane to what the tabs switch (`#strip-host`, `#voice-host` and `#remembered` already hide off Sessions);
    - remove `paintHint`.
  - `setup.ts`: remove the mode choice and the stopped-server card. A phone that is not joined shows the Join card.
  - `network.ts`:
    - remove the This phone port and Start, Pairing, Assistants and Key's main-only parts, together with their handlers and invokes (`server_start`, `server_stop`, `network_pairing_code`, `network_remove_assistant`, `network_promote` and `network_set_mirror`);
    - keep the name, Join a main, Leave, Notifications, Voice, Session viewer and Battery.
- [ ] **Step 7: Run the tests.** Run `pnpm test`. Expected: PASS. Then run `grep -rn "server_start\|network_promote\|network_pairing_code" src/mobile`. Expected: nothing.
- [ ] **Step 8: Check the page in a desktop window.** Run `pnpm mobile:desktop` against a running desktop main joined with a code. The Chat tab shows the main's conversation, a message sent from it appears on the desktop, and the Network tab shows no pairing code.
- [ ] **Step 9: Commit.** `feat(mobile): a Chat tab, and Join as the only first screen`

### Task 8: Docs, and the release

**Files:**
- Modify: `README.md`, the Android section (lines 186–243): the phone always joins a main; a paragraph for phones that were the main ("On one computer choose Main in Settings › Network, then Join the phone and pair the other computers to it"); Chat on the phone; the notification text.
- Modify: `docs/DEVELOPING.md`, the Android paragraph ("It is a main, or it joins a main…") and the `hub.rs` bullet.
- Modify: `docs/superpowers/specs/2026-10-26-phone-chat-design.md`: `Status: implemented`.
- Version files, via the script.

- [ ] **Step 1: Write the doc changes.** Run `grep -n "Be the main\|main for your assistants\|Make main" README.md docs/DEVELOPING.md`. Expected: no line that offers a phone as the main.
- [ ] **Step 2: Commit.** `docs: the phone joins a main and has Chat`
- [ ] **Step 3: Test by hand on a real phone.** Use `pnpm mobile:build --debug`, installed over 0.34, with the Mac as the main. Follow the spec's success scenario:
  - a phone that was the main opens on Join;
  - the Chat tab matches the Mac's;
  - a message typed on the phone gets her reply on both screens;
  - Cancel and Stop work;
  - turning the Mac's switch off hides the tab.

  Report anything that fails. Do not mark it passed without the device.
- [ ] **Step 4: Bump the version.** Run `git fetch origin main && git show origin/main:package.json | grep '"version"'`. If it is still `0.34.x`, run `sh scripts/set-version.sh 0.35.0`. Otherwise, take the next minor from that version.
- [ ] **Step 5: Run the tests.** Run `cargo test --workspace && pnpm test`. Expected: PASS.
- [ ] **Step 6: Commit.** `chore(release): 0.35.0`. Include only the four version files.
