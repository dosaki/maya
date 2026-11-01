# Native Terminal (remote viewers) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Native session's live terminal can be watched and typed into from the main (for an assistant's session) and from the phone (for the main's or an assistant's session), through the Maya that hosts it, without ever resizing it.

**Architecture:** A terminal subscription is a `sub` id per link. A viewer's Maya sends `term_attach`; the hosting Maya attaches a host viewer and streams `term_frame`s back, each numbered (`seq`), the first being the snapshot (screen plus the last 1,000 lines). The main serves its own sessions to the phone directly and relays everything else between the phone (or its own page) and the assistant that hosts the session. Cards carry a `live` flag that the main sets only when the whole path supports it, so an older Maya on either end keeps today's transcript views and is never sent a frame it cannot read.

**Tech Stack:** Rust (`maya_core` net, Tauri 2 desktop and Android), TypeScript (`@xterm/xterm` 6), vitest.

**Spec:** `docs/superpowers/specs/2026-10-30-native-terminal-design.md`, section "Remote viewers" (the local part shipped in 0.44.0–0.44.2).

## Global Constraints

- Capability: `CAP_TERMINAL = "terminal"`. A desktop Maya announces it in its `hello` caps. The main sends `term_attach` to an assistant only when that assistant announced it.
- Remote snapshot: the screen plus the last 1,000 lines (`REMOTE_LINES = 1_000`). A local viewer keeps all 10,000.
- A remote viewer never resizes the PTY. It draws the host's size; when the host's size changes it is told (`size`) and follows.
- Bytes cross the network and the IPC as base64 strings, never as text.
- Frame numbering: the snapshot (`attached`) is `seq` 0; every later event of the subscription is numbered 1, 2, 3… in the order the host produced it. `lost`, and an `error` sent instead of a snapshot, carry `seq` 0.
- While a link carries a live subscription, its read timeout is `LIVE_TICK = 10 ms` instead of `TICK = 100 ms`, on both ends, so a keystroke and its echo are not held up to 100 ms per hop.
- Remote input is trusted as card replies are: a mirroring link that may mirror (`may_mirror`) may type; one that may not is refused.
- The viewer's messages: `[Session ended]` (exists), `[Connection lost. Reconnecting…]`, and `[<message>]` for an error. A lost viewer tries to attach again every 2 s (`REATTACH_MS = 2000`) while it is open.
- Phone extra keys, in this order: Esc, Tab, Ctrl+C, ←, ↑, ↓, →.
- Phone font: fit the host's columns to the viewer's width, between 8 px and 12.5 px; past 8 px the terminal scrolls sideways.
- Every local path (the local live terminal, external terminals) behaves exactly as in 0.44.2.
- Release: a `feat`, so bump `main`'s minor version (0.44.2 → 0.45.0 if `main` has not moved) with `sh scripts/set-version.sh`, in its own `chore(release): <version>` commit, together with the new top section of `RELEASE_NOTES.md`.

## Review Focus

1. **The phone opens a session mid-redraw.** Frames can arrive before the snapshot. They are held, then drawn after it in order, with no duplicate and no gap. *Test: Task 7 `frames_that_arrive_before_the_snapshot_are_drawn_after_it` (live.test.ts).*
2. **The phone loses Wi-Fi while watching.** The viewer says it lost the connection and attaches again once the link is back, showing the current screen. *Tests: Task 6 `a_disconnect_tells_every_live_viewer_it_is_lost`; Task 7 `a_lost_viewer_attaches_again`.*
3. **Someone resizes the local viewer on the host while the phone watches.** The phone's terminal follows the new size and never sends a resize. *Tests: Task 2 `a_resize_tells_viewers_the_new_size_after_the_output_before_it`; Task 7 `a_remote_viewer_never_resizes`.*
4. **The assistant quits or drops while the main or the phone watches its session.** The viewer is told `lost` at once and never hangs. *Test: Task 5 `an_assistant_that_leaves_tells_its_viewers_they_are_lost`.*
5. **An older assistant or main on either end.** No `live` flag, so the viewer keeps the transcript, and no `term_attach` is ever sent to a Maya without `CAP_TERMINAL`. *Tests: Task 4 `only_a_board_from_a_maya_with_live_terminals_marks_its_native_cards_live`; Task 5 `a_relay_to_a_maya_without_live_terminals_is_refused`.*

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/net/live.rs` (new) | `TermEvent`, `TermFrame`, sinks, `LiveTerms` trait, `open` (numbering), `REMOTE_LINES`, `LIVE_TICK`, `tick_for` |
| `core/src/net/protocol.rs` | `CAP_TERMINAL`; `Up`/`Down` `term_*` messages |
| `core/src/model.rs` | `Card.live` |
| `core/src/net/merge.rs` | `RemoteBoard.terminals`; `merged` sets `live` |
| `core/src/net/client.rs` | Assistant side: serves `term_*` from the main; phone side: `term_frame` to `ClientNotify`; `RequestLane::send`; live tick |
| `core/src/net/server.rs` | Main side: serves its own sessions to mirrors, relays to assistants, `term_attach`/`term_input`/`term_detach` for its page, cleanup, live tick |
| `src-tauri/src/native/host.rs` | `Frame::Size` on resize, ordered after earlier output |
| `src-tauri/src/native/live_terms.rs` (new) | `NativeTerms`: `LiveTerms` on the native host, by session id |
| `src-tauri/src/native_app.rs` | `NativeFrame::Size`; shared `term_for_pid`; `live_attach_remote`, `live_input_remote`, `live_detach_remote` |
| `src-tauri/src/net_app.rs`, `src-tauri/src/lib.rs` | Caps, `Executor::live_terms`, `Notify::live_terms`, command registration |
| `mobile/src/hub.rs`, `mobile/src/commands.rs`, `mobile/src/lib.rs` | The phone's subscriptions and the same three commands |
| `src/live.ts` | Remote target, numbering, lost/reattach, size, font fit, extra keys |
| `src/modal.ts` | `isLive` for remote live cards; remote target; extra key row |
| `src/mobile/main.ts`, `src/types.ts`, `src/styles.css` | Phone option, `Card.live`, key row and sideways scroll |
| `README.md`, `RELEASE_NOTES.md` | Docs and release |

---

### Task 1: Wire types and the `live` flag

**Files:**
- Create: `core/src/net/live.rs` (add `pub mod live;` in `core/src/net/mod.rs`)
- Modify: `core/src/net/protocol.rs` (caps near `:33`, `Up` `:48`, `Down` `:85`), `core/src/model.rs:160` (`Card`), `src/types.ts:180`
- Modify (compile only): every exhaustive `match` on `Up`/`Down` (`client.rs` `handle`, `server.rs` `handle`). Each new arm logs `"network", "<type> not handled yet"` and does nothing; Tasks 3–6 replace them.
- Test: `core/src/net/protocol.rs` tests, `core/src/net/live.rs` tests

**Interfaces:**
- Produces, in `core/src/net/live.rs`:
  - `pub const REMOTE_LINES: usize = 1_000;` `pub const LIVE_TICK: Duration = Duration::from_millis(10);`
  - `pub fn tick_for(live: bool, idle: Duration) -> Duration` (`LIVE_TICK` when live, else `idle`).
  - `#[serde(tag = "kind", rename_all = "snake_case")] pub enum TermEvent { Attached { snapshot: String, rows: u16, cols: u16 }, Output { data: String }, Size { rows: u16, cols: u16 }, Exit, Error { message: String }, Lost }`
  - `pub struct TermFrame { pub seq: u64, #[serde(flatten)] pub event: TermEvent }` (Serialize, Deserialize, Clone, Debug, PartialEq).
  - `pub type EventSink = Box<dyn Fn(TermEvent) -> bool + Send>;` `pub type FrameSink = Box<dyn Fn(TermFrame) -> bool + Send>;` A sink returning false means its viewer is gone.
  - `pub struct TermView { pub term: u64, pub viewer: u64 }`; `pub struct TermOpened { pub view: TermView, pub snapshot: Vec<u8>, pub rows: u16, pub cols: u16 }`.
  - `pub trait LiveTerms: Send + Sync { fn attach(&self, session: &str, sink: EventSink) -> Result<TermOpened, String>; fn input(&self, view: &TermView, data: &[u8]) -> Result<(), String>; fn detach(&self, view: &TermView); }`
  - `pub fn open(terms: &dyn LiveTerms, session: &str, out: FrameSink) -> Result<TermView, String>`: wraps `out` so each event gets the next `seq` from 1. Attaches, then sends `TermFrame { seq: 0, event: Attached { snapshot: base64, rows, cols } }` through `out`. On an attach error it returns the error and sends nothing.
- Produces, in `protocol.rs`: `pub const CAP_TERMINAL: &str = "terminal";` and
  - `Up::TermAttach { sub: u64, #[serde(default)] machine: Option<String>, session: String }`, `Up::TermInput { sub: u64, data: String }`, `Up::TermDetach { sub: u64 }`, `Up::TermFrame { sub: u64, frame: TermFrame }`
  - `Down::TermAttach { sub: u64, session: String }`, `Down::TermInput { sub: u64, data: String }`, `Down::TermDetach { sub: u64 }`, `Down::TermFrame { sub: u64, frame: TermFrame }`
- Produces: `Card.live: bool` (`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`, doc: "This card's live terminal can be opened from the Maya showing it"). TS `live?: boolean`.

- [ ] **Step 1: Write the failing tests**

```rust
// protocol.rs
#[test]
fn terminal_frames_round_trip_with_their_tags() {
    let f = TermFrame { seq: 3, event: TermEvent::Output { data: "w6k=".into() } };
    let up = Up::TermFrame { sub: 9, frame: f.clone() };
    let s = encode(&up);
    assert!(s.contains(r#""type":"term_frame""#) && s.contains(r#""seq":3"#) && s.contains(r#""kind":"output""#), "{s}");
    assert_eq!(decode_up(&s).unwrap(), up);
    for up in [Up::TermAttach { sub: 1, machine: Some("laptop".into()), session: "s1".into() }, Up::TermInput { sub: 1, data: "aGk=".into() }, Up::TermDetach { sub: 1 }] {
        assert_eq!(decode_up(&encode(&up)).unwrap(), up);
    }
    for down in [Down::TermAttach { sub: 2, session: "s1".into() }, Down::TermInput { sub: 2, data: "aGk=".into() }, Down::TermDetach { sub: 2 }, Down::TermFrame { sub: 2, frame: TermFrame { seq: 0, event: TermEvent::Lost } }] {
        assert_eq!(decode_down(&encode(&down)).unwrap(), down);
    }
    assert_eq!(decode_up(r#"{"type":"term_attach","sub":1,"session":"s1"}"#).unwrap(), Up::TermAttach { sub: 1, machine: None, session: "s1".into() });
    assert_eq!(CAP_TERMINAL, "terminal");
}

#[test]
fn a_card_says_live_only_when_it_is() {
    // Build a Card as in a_card_from_an_agent_this_maya_does_not_know_still_decodes, with live: false.
    // assert!(!serde_json::to_string(&card).unwrap().contains("live"));
    // with live: true → contains r#""live":true"#; and a card without the field decodes with live == false.
}
```

```rust
// live.rs
#[test]
fn open_sends_the_snapshot_as_seq_0_and_numbers_what_follows_from_1() {
    // A fake LiveTerms whose attach keeps the EventSink, calls it once with Output before returning
    // (as the host's flusher can), and returns TermOpened { snapshot: b"hi".to_vec(), rows: 40, cols: 120, .. }.
    // Collect frames from `out` into a Vec; then call the kept sink with Size { rows: 30, cols: 100 }.
    // assert: frames contain {seq:1, Output}, {seq:0, Attached{snapshot:"aGk=", rows:40, cols:120}}, {seq:2, Size}
    // in that push order (the output may precede the snapshot; the viewer reorders).
}

#[test]
fn open_returns_the_error_and_sends_nothing_when_attach_fails() { /* fake attach → Err("This session is not running in Maya.") */ }

#[test]
fn a_live_link_ticks_fast() {
    assert_eq!(tick_for(true, Duration::from_millis(100)), Duration::from_millis(10));
    assert_eq!(tick_for(false, Duration::from_millis(100)), Duration::from_millis(100));
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p maya_core net::` — expected: does not compile (types missing).

- [ ] **Step 3: Add the types, the messages, `Card.live` and placeholder arms.** Fill `live: false` wherever a `Card` is built in tests and code (the compiler lists them).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p maya_core && cargo build -p maya && pnpm test` — expected: PASS.

- [ ] **Step 5: Commit** — `feat(net): live terminal messages and the live card flag`

---

### Task 2: The host says when its size changes, and serves sessions by id

**Files:**
- Modify: `src-tauri/src/native/host.rs` (`Frame` `:34`, `resize` `:533`)
- Modify: `src-tauri/src/native_app.rs` (`NativeFrame`, `frame_of`, `native_attach`)
- Create: `src-tauri/src/native/live_terms.rs` (add `pub mod live_terms;` in `native/mod.rs`)
- Modify: `src/live.ts` (local mode ignores `size`)
- Test: `host.rs` tests, `native_app.rs` tests, `live_terms.rs` tests, `src/live.test.ts`

**Interfaces:**
- Consumes: Task 1 `LiveTerms`, `TermEvent`, `TermOpened`, `TermView`, `EventSink`, `REMOTE_LINES`.
- Produces: `Frame::Size { rows: u16, cols: u16 }`; `NativeFrame::Size { rows, cols }` (`{"kind":"size","rows":30,"cols":100}`).
- Produces: `pub(crate) fn term_for_pid(tree: &ProcessTree, pid: i32) -> Option<TermId>` in `native_app.rs`, used by `native_attach` and by `NativeTerms`. Use whatever type `process_tree()` returns.
- Produces: `pub struct NativeTerms { app: AppHandle }` with `impl LiveTerms`.
  - `attach` finds the session's card in the store (`card_for(session, now)`, as `net_app::execute` does), then `term_for_pid`. It calls `HOST.attach(term, REMOTE_LINES, sink)` and maps `Frame` to `TermEvent`: output to base64, `Exit`, and `Size`.
  - `input` calls `HOST.enqueue`. `detach` calls `HOST.detach`.
  - When the session is missing, the error is "This session is not running in Maya."

- [ ] **Step 1: Write the failing tests**

```rust
// host.rs
#[test]
fn a_resize_tells_viewers_the_new_size_after_the_output_before_it() {
    // Spawn `sh -c 'printf ab; read l'`, attach a viewer collecting frames, wait for "ab".
    // Feed more output that is still pending (or resize immediately after output arrives), then host.resize(t, 30, 100).
    // assert: the frames end with Frame::Size { rows: 30, cols: 100 }, and every Output byte produced before the
    // resize arrives before it; a resize to the same size sends nothing.
}
```

```rust
// native_app.rs
#[test]
fn a_size_frame_reaches_the_viewer() {
    assert_eq!(serde_json::to_string(&frame_of(Frame::Size { rows: 30, cols: 100 })).unwrap(), r#"{"kind":"size","rows":30,"cols":100}"#);
}
```

```rust
// live_terms.rs: the Frame → TermEvent mapping as a pure fn `event_of(Frame) -> TermEvent`
#[test]
fn host_frames_become_wire_events() {
    assert_eq!(event_of(Frame::Output(vec![0xC3, 0xA9])), TermEvent::Output { data: "w6k=".into() });
    assert_eq!(event_of(Frame::Exit), TermEvent::Exit);
    assert_eq!(event_of(Frame::Size { rows: 30, cols: 100 }), TermEvent::Size { rows: 30, cols: 100 });
}
```

```ts
// live.test.ts
it("a local viewer ignores size frames", async () => {
  // Mount a local target, deliver {kind:"size",rows:30,cols:100}: the stub terminal's resize is not called
  // with (100, 30), and native_resize is not invoked because of it.
});
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p maya native` and `pnpm vitest run src/live.test.ts`.

- [ ] **Step 3: Implement.** In `resize`, while it holds the master lock, take `sending`, deliver the pending batch, then send `Size` to each viewer. Do this only when the size actually changed. This keeps the order every viewer sees.

- [ ] **Step 4: Run the tests** — `cargo test -p maya native:: && pnpm test` — expected: PASS.

- [ ] **Step 5: Commit** — `feat(native): tell viewers the terminal's new size and serve sessions by id`

---

### Task 3: An assistant serves its live terminals to the main

**Files:**
- Modify: `core/src/net/client.rs` (`Executor` `:82`, `pump` `:560`, `handle` `:629`; the stream's read timeout `:408`)
- Modify: `src-tauri/src/net_app.rs` (`TauriExecutor` `:242`; hello caps `:178` gain `CAP_TERMINAL`)
- Test: `core/src/net/client.rs` tests

**Interfaces:**
- Consumes: Task 1 `open`, `LiveTerms`, `TermView`, `tick_for`; Task 2 `NativeTerms`.
- Produces:
  - `Executor::live_terms(&self) -> Option<Arc<dyn LiveTerms>>`, defaulting to `None`. `TauriExecutor` returns `Some(Arc::new(NativeTerms { app }))`.
  - `pump` keeps `subs: HashMap<u64, TermView>`, passed to `handle` as `&mut`.
  - `Down::TermAttach { sub, session }` calls `open(terms, &session, out)`. `out` sends `Up::TermFrame { sub, frame }` on the pump's `tx`, and returns false once the pump has ended.
    - On success the view goes into `subs`.
    - On error the assistant sends `Up::TermFrame { sub, frame: TermFrame { seq: 0, event: Error { message } } }`.
    - With no `live_terms`, the message is "This Maya has no live terminals."
  - `Down::TermInput { sub, data }` decodes base64 and calls `terms.input`. An error, or an unknown `sub`, gets an `Error` frame (seq 0).
  - `Down::TermDetach { sub }` detaches and removes the sub.
  - When the pump ends, it detaches every sub.
  - Each pump turn sets the read timeout to `tick_for(!subs.is_empty() || notify.live(), TICK)`, through a `try_clone()` of the stream. `ClientNotify::live(&self) -> bool` defaults to false (the phone overrides it in Task 6).

- [ ] **Step 1: Write the failing tests** (use the file's existing fake-main harness that drives `pump` over a `secure::Channel`; a `FakeTerms` that records calls and keeps the sink)

```rust
#[test]
fn an_assistant_streams_a_live_terminal_to_the_main() {
    // Main sends Down::TermAttach { sub: 5, session: "s1" }.
    // assert: it receives Up::TermFrame { sub: 5, frame.seq == 0, Attached { .. } }.
    // FakeTerms pushes Output("x") through the kept sink → main receives Up::TermFrame { sub: 5, seq: 1, Output }.
    // Main sends Down::TermInput { sub: 5, data: "aGk=" } → FakeTerms.input saw b"hi".
    // Main sends Down::TermDetach { sub: 5 } → FakeTerms.detach saw the view.
}

#[test]
fn an_unknown_session_or_no_terminals_answers_with_an_error_frame() {
    // FakeTerms attach → Err("This session is not running in Maya.") → Up::TermFrame { seq: 0, Error { message } }.
    // An executor with live_terms() == None → message "This Maya has no live terminals."
}

#[test]
fn the_pump_detaches_its_viewers_when_the_link_ends() { /* attach, then drop the main's end; FakeTerms.detach called */ }
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p maya_core net::client`.

- [ ] **Step 3: Implement, as described in Interfaces.**

- [ ] **Step 4: Run the tests** — `cargo test -p maya_core && cargo build -p maya` — expected: PASS.

- [ ] **Step 5: Commit** — `feat(net): an assistant streams its live terminals to the main`

---

### Task 4: The main serves its own live terminals to the phone, and marks live cards

**Files:**
- Modify: `core/src/net/merge.rs` (`RemoteBoard` `:11`, `merged` `:40`)
- Modify: `core/src/net/server.rs` (`Notify` `:85`; `Server` `:143`; `boards` `:305`; `Link` `:633`; `handle` `:1089`; `mirror_due` `:1001`; the pump's read timeout `:688`)
- Modify: `src-tauri/src/lib.rs` (`TauriNetNotify` `:238`)
- Test: `merge.rs` and `server.rs` tests

**Interfaces:**
- Consumes: Task 1, Task 2 `NativeTerms`.
- Produces:
  - `Notify::live_terms(&self) -> Option<Arc<dyn LiveTerms>>`, defaulting to `None`. `TauriNetNotify` returns `NativeTerms`.
  - `Server.terminals: bool` is set when the server starts, from `ctx.notify.live_terms().is_some()`. It means this main can host and relay; a phone acting as main cannot.
  - `RemoteBoard.terminals: bool` is set like `worktrees`: the peer announced `CAP_TERMINAL` and `s.terminals`. `merged` sets `c.live = c.native && b.terminals && b.connected`.
  - `mirror_due` sets `live = c.native && s.terminals` on the main's own cards, which are the ones it stamps with `main_name`.
  - `Link.terms: RefCell<HashMap<u64, LinkTerm>>`, with `enum LinkTerm { Local(TermView), Relayed(u64) }`. `Relayed` is filled in Task 5.
  - `Up::TermAttach { sub, machine, session }` from a link that `wants_mirror && may_mirror`, with `machine` None or `main_name`, calls `open(terms, &session, out)`. `out` sends `Down::TermFrame { sub, frame }` on this link's `Conn.tx`.
    - Errors go back as `Error` frames (seq 0).
    - A link that may not mirror gets `Error { message: "This Maya may not see the main's sessions." }`.
    - An attach with another machine is Task 5. Until then it gets `Error { message: "Not yet." }`.
  - `Up::TermInput` from a link that may no longer mirror is refused. It gets the same `Error` frame, and the view is detached.
  - `Up::TermDetach` detaches and removes the sub.
  - When the link ends, every `Local` view is detached.
  - The link's read timeout is `tick_for(!link.terms.borrow().is_empty(), TICK)`, for now.

- [ ] **Step 1: Write the failing tests**

```rust
// merge.rs
#[test]
fn only_a_board_from_a_maya_with_live_terminals_marks_its_native_cards_live() {
    // Two connected boards with a native card each: terminals: true and terminals: false; plus a non-native card.
    // assert: merged marks live only the native card of the terminals: true board; a disconnected board's card is not live.
}
```

```rust
// server.rs (existing harness: a server with a fake Notify, a mirroring client with CAP_MIRROR + CAP_CHAT)
#[test]
fn the_main_streams_its_own_live_terminal_to_a_mirror() {
    // Notify::live_terms → FakeTerms. Mirror sends Up::TermAttach { sub: 1, machine: Some(main_name), session: "s1" }.
    // assert: Down::TermFrame { sub: 1, seq 0 Attached }; output via kept sink → seq 1; TermInput reaches FakeTerms;
    // TermDetach detaches; closing the mirror's connection detaches an attached view.
}

#[test]
fn a_mirror_that_may_not_mirror_gets_an_error_frame() { /* switch off; attach → Error { message: "This Maya may not see the main's sessions." } */ }

#[test]
fn the_main_marks_its_own_native_cards_live_for_a_mirror_only_when_it_hosts_terminals() {
    // mirror(cards with one native card): with terminals the mirrored card has live == true; with a Notify
    // whose live_terms is None it is false.
}
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p maya_core net::`.

- [ ] **Step 3: Implement, as described in Interfaces.**

- [ ] **Step 4: Run the tests** — `cargo test -p maya_core && cargo build -p maya` — expected: PASS.

- [ ] **Step 5: Commit** — `feat(net): the main streams its live terminals to the phone and marks live cards`

---

### Task 5: The main relays live terminals between viewers and assistants

**Files:**
- Modify: `core/src/net/server.rs` (`Server` relays, `handle`, `leave` `:1223`, link end; public API)
- Test: `server.rs` tests

**Interfaces:**
- Consumes: Tasks 1, 3, 4.
- Produces:
  - `Server.relays: HashMap<u64, Relay>`, with:
    - `struct Relay { assistant: String, assistant_serial: u64, to: RelayTo }`;
    - `enum RelayTo { Mirror { id: String, serial: u64, sub: u64 }, Page(FrameSink) }`.
    - Relay ids come from `s.next()`.
  - `fn relay_attach(s: &mut Server, machine: &str, session: &str, to: RelayTo) -> Result<u64, String>` (private):
    - It resolves the label with `id_for`.
    - An unknown or unconnected machine gives "That Maya is not connected."
    - A peer without `CAP_TERMINAL` gives "That Maya cannot share a live terminal. Update it."
    - On success it sends `Down::TermAttach { sub: rid, session }` on the assistant's `Conn.tx`.
  - A mirror's `Up::TermAttach` with another machine creates `RelayTo::Mirror`. It records `LinkTerm::Relayed(rid)`; a failure is answered as an `Error` frame.
  - `Up::TermInput` and `Up::TermDetach` on a relayed sub are forwarded to the assistant as `Down::TermInput { sub: rid }` and `Down::TermDetach { sub: rid }`. A detach also removes the relay.
  - `Up::TermFrame { sub: rid, frame }` from an assistant link is handled as follows:
    - The relay must have `assistant_serial == link.serial`.
    - The frame is forwarded unchanged (same `seq`), to the mirror's conn as `Down::TermFrame { sub, frame }` when its `serial` still matches, or to the `Page` sink.
    - If the target is gone, or the sink returns false, the relay is removed and the assistant gets `Down::TermDetach`.
    - After an `Exit` or an `Error` frame, the relay is removed.
  - When a mirror link ends, each of its `Relayed` subs gets a `Down::TermDetach` to the assistant, and its relay is removed.
  - When an assistant leaves (`leave`), each of its relays tells its target `TermFrame { seq: 0, event: Lost }`, and the relay is removed.
  - The live tick also applies to an assistant link with any relay (`assistant_serial == link.serial`).
  - Public, for the main's own page:
    - `pub fn term_attach(shared: &Arc<Mutex<Server>>, machine: &str, session: &str, sink: FrameSink) -> Result<u64, String>` (`RelayTo::Page`);
    - `pub fn term_input(shared: &Arc<Mutex<Server>>, sub: u64, data: String) -> Result<(), String>`;
    - `pub fn term_detach(shared: &Arc<Mutex<Server>>, sub: u64)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_mirror_watches_an_assistant_s_terminal_through_the_main() {
    // Harness: assistant client with CAP_TERMINAL and FakeTerms; mirroring client.
    // Mirror: Up::TermAttach { sub: 1, machine: Some("laptop"), session: "s1" }.
    // assert: mirror gets Down::TermFrame { sub: 1, seq 0 Attached }, then seq 1 Output pushed by the assistant's FakeTerms;
    // mirror's TermInput reaches the assistant's FakeTerms; mirror's TermDetach reaches its detach.
}

#[test]
fn the_main_s_page_watches_an_assistant_s_terminal() { /* term_attach with a collecting FrameSink; term_input; term_detach */ }

#[test]
fn a_relay_to_a_maya_without_live_terminals_is_refused() {
    // Assistant without CAP_TERMINAL: term_attach → Err("That Maya cannot share a live terminal. Update it."),
    // and the assistant receives no Down::TermAttach.
}

#[test]
fn an_assistant_that_leaves_tells_its_viewers_they_are_lost() {
    // Mirror and page watching; drop the assistant → both get TermFrame { seq: 0, Lost }; relays empty.
}

#[test]
fn an_ended_session_ends_its_relay() { /* FakeTerms sends Exit → mirror gets it; relays empty; later frames dropped */ }

#[test]
fn a_mirror_that_leaves_detaches_from_the_assistant() { /* drop the mirror → assistant's FakeTerms.detach called */ }
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p maya_core net::server`.

- [ ] **Step 3: Implement, as described in Interfaces.** Keep the lock order: `network`, then the server mutex, then `store`. Never call a `Page` sink while holding the server mutex: take the sink's result after releasing, or clone what is needed first.

- [ ] **Step 4: Run the tests** — `cargo test -p maya_core` — expected: PASS.

- [ ] **Step 5: Commit** — `feat(net): the main relays live terminals between viewers and assistants`

---

### Task 6: The commands the page uses, on the desktop and the phone

**Files:**
- Modify: `src-tauri/src/native_app.rs` (the three commands), `src-tauri/src/lib.rs` (registration)
- Modify: `core/src/net/client.rs` (`RequestLane::send`; `ClientNotify::term_frame`; `handle` arm `Down::TermFrame`)
- Modify: `mobile/src/hub.rs`, `mobile/src/commands.rs`, `mobile/src/lib.rs`
- Test: `client.rs` tests, `mobile/src/hub.rs` tests

**Interfaces:**
- Consumes: Tasks 1, 5.
- Produces: the same three commands on both apps. A frame reaches the page as `TermFrame` JSON, e.g. `{"seq":1,"kind":"output","data":"aGk="}`.
  - `live_attach_remote(session_id: String, on_frame: Channel<TermFrame>) -> Result<u64, String>` (async).
  - `live_input_remote(sub: u64, data: String) -> Result<(), String>`. It is sync, on the main thread, so keys keep their order (as `native_input`).
  - `live_detach_remote(sub: u64)` (async).
- Desktop:
  - `machine = remote_machine_of(&state, &session_id)`, or "This session is not on a connected Maya."
  - It then calls `server::term_attach` / `term_input` / `term_detach` on the server handle in `state.network`.
  - With no server running: "This Maya is not the main."
- `RequestLane::send(&self, up: Up) -> Result<(), String>` sends on the current out. When detached it returns the lane's `LOST` message.
- `ClientNotify::term_frame(&self, sub: u64, frame: TermFrame)` defaults to doing nothing. `handle` calls it for `Down::TermFrame`.
- Phone `Hub`:
  - Holds `terms: Mutex<HashMap<u64, FrameSink>>` and `next_term: AtomicU64`.
  - `pub fn term_attach(&self, session: &str, sink: FrameSink) -> Result<u64, String>`:
    - It uses the card from `cards()`, which must be `live`; otherwise "This session's live terminal can't be opened from here."
    - It sends `Up::TermAttach { sub, machine: card.machine, session }` with `lane.send`.
  - `term_input(sub, data)` sends `Up::TermInput`. `term_detach(sub)` sends `Up::TermDetach` and removes the sub.
  - `HubClientNotify::term_frame` hands the frame to the sink. After `Exit`, or an `Error` with seq 0, it removes the sub. If the sink returns false, it removes the sub and sends `TermDetach`.
  - `HubClientNotify::disconnected` sends every sub `TermFrame { seq: 0, event: Lost }` and clears them.
  - `HubClientNotify::live()` is true while `terms` is not empty.

- [ ] **Step 1: Write the failing tests**

```rust
// mobile/src/hub.rs
#[test]
fn a_disconnect_tells_every_live_viewer_it_is_lost() {
    // Insert two subs with collecting sinks (a hub with a live card, lane detached so attach's send fails is fine:
    // insert directly through a test helper), call the notify's disconnected("x").
    // assert: both sinks got TermFrame { seq: 0, event: Lost }; terms is empty.
}

#[test]
fn only_a_live_card_s_terminal_opens() { /* card with live == false → Err("This session's live terminal can't be opened from here.") */ }

#[test]
fn a_frame_reaches_its_viewer_and_an_exit_ends_it() { /* term_frame(sub, Output) → sink; Exit → sub removed */ }
```

```rust
// client.rs
#[test]
fn a_terminal_frame_from_the_main_reaches_the_notify() { /* main sends Down::TermFrame; ClientNotify::term_frame called with it */ }
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p maya_core net::client` and `cargo test -p maya-mobile` (use the mobile crate's package name from `mobile/Cargo.toml`).

- [ ] **Step 3: Implement, as described in Interfaces.**

- [ ] **Step 4: Run the tests** — `cargo test --workspace` — expected: PASS.

- [ ] **Step 5: Commit** — `feat: open a remote live terminal from the main's page and the phone`

---

### Task 7: The viewer draws a remote live terminal

**Files:**
- Modify: `src/live.ts`, `src/modal.ts` (`isLive` `:44`, banner `:643`, `syncLive` `:992`, `mountLater` `:959`), `src/mobile/main.ts:34`, `src/styles.css`
- Test: `src/live.test.ts`, `src/modal.test.ts` (or `modal-open.test.ts`, whichever holds the live tests)

**Interfaces:**
- Consumes: Task 6 commands and `TermFrame` JSON. Task 1 `Card.live`.
- Produces, in `live.ts`:
  - `LiveTarget` gains `{ session: string; remote: true }`.
  - `LiveHandle` gains `press(key: ExtraKey): void`.
  - `export type ExtraKey = "esc" | "tab" | "ctrl-c" | "left" | "up" | "down" | "right";`
  - `export const EXTRA_KEYS: readonly (readonly [ExtraKey, string])[]` holds, in order, `esc` "Esc", `tab` "Tab", `ctrl-c` "Ctrl+C", `left` "←", `up` "↑", `down` "↓", `right` "→".
  - `export function keyBytes(key: ExtraKey, appCursor: boolean): string` maps:
    - esc to `"\x1b"`, tab to `"\t"`, ctrl-c to `"\x03"`;
    - arrows to `"\x1b[A"` / `"\x1b[B"` / `"\x1b[C"` / `"\x1b[D"` for up / down / right / left, or `"\x1bO…"` when `appCursor`.
  - `export function fitFont(width: number, cols: number): number` returns `Math.max(8, Math.min(12.5, Math.floor((width / (cols * 0.6)) * 2) / 2))`.
- Remote mode in `mountLive`:
  - **Attaching:**
    - It invokes `live_attach_remote { sessionId, onFrame }`.
    - Frames with `seq > 0` that come before `attached` are held.
    - On `attached`, it calls `term.resize(cols, rows)`, `reset`, sets `fontSize = fitFont(host width, cols)` and writes the snapshot. It then draws the held frames whose `seq` is above 0, in order.
  - **Frames:**
    - `size` calls `term.resize(cols, rows)` and refits the font.
    - `exit` writes `[Session ended]`; `error` writes `[<message>]`.
    - `lost` writes `[Connection lost. Reconnecting…]` and attaches again every 2 s until an `attached` arrives. Each try uses a new channel, and the old sub is detached.
    - A `seq` that is not the last one plus 1 detaches and attaches again.
  - **Input:** `live_input_remote { sub, data }`.
  - **Never resizes:** no `native_resize` and no `live_*` resize, ever. The `ResizeObserver` only refits the font.
  - **Close:** `dispose` calls `live_detach_remote { sub }`.
  - `press` sends `keyBytes(key, term.modes.applicationCursorKeysMode)` through the same input path.
- `modal.ts`:
  - `isLive(card, theme)` is `theme === "terminal" && !!card.native && (!card.machine || !!card.live)`.
  - `syncLive` passes `{ session: card.sessionId, remote: true }` when `card.machine` is set, and `{ pid }` otherwise.
  - The banner's `native` becomes `remote ? !!card.live : !!card.native`.
  - `composer.terminalKeys: boolean` defaults to false; the phone sets it true in `src/mobile/main.ts`. When it is true, a live viewer shows `<div class="modal__keys">` below `.modal__live`, with one button per `EXTRA_KEYS` entry. Each button calls `handle.press(key)`, and `pointerdown` calls `preventDefault()` so the terminal keeps focus and the keyboard stays up.
- CSS:
  - `.modal__live` in remote mode scrolls sideways (`overflow-x: auto`).
  - `.modal__keys` is a single row of equal buttons that scrolls sideways if it overflows.

- [ ] **Step 1: Write the failing tests** (xterm is stubbed as in the existing `live.test.ts`; the stub records `resize`, `write`, `reset`, and exposes `modes.applicationCursorKeysMode`)

```ts
it("frames_that_arrive_before_the_snapshot_are_drawn_after_it", async () => {
  // Remote target. Deliver {seq:1,kind:"output",data:b64("b")} then {seq:0,kind:"attached",snapshot:b64("a"),rows:40,cols:120},
  // then {seq:2,kind:"output",data:b64("c")}. assert writes after reset: "a","b","c" in that order; resize(120,40).
});
it("a_remote_viewer_never_resizes", async () => { /* trigger the ResizeObserver; no invoke of native_resize or any *resize* command */ });
it("a size frame resizes the remote terminal", async () => { /* {seq:1,kind:"size",rows:30,cols:100} → resize(100,30) */ });
it("a_lost_viewer_attaches_again", async () => {
  // fake timers; {seq:0,kind:"lost"} → writes "[Connection lost. Reconnecting…]"; after 2000 ms live_attach_remote invoked again;
  // once attached arrives, no further tries.
});
it("a gap in the numbering attaches again", async () => { /* seq 1 then seq 3 → live_detach_remote + live_attach_remote */ });
it("keyBytes follows the cursor mode", () => {
  expect(keyBytes("up", false)).toBe("\x1b[A"); expect(keyBytes("up", true)).toBe("\x1bOA");
  expect(keyBytes("left", false)).toBe("\x1b[D"); expect(keyBytes("ctrl-c", true)).toBe("\x03");
  expect(EXTRA_KEYS.map(([, l]) => l)).toEqual(["Esc", "Tab", "Ctrl+C", "←", "↑", "↓", "→"]);
});
it("fitFont fits the columns between 8 and 12.5", () => {
  expect(fitFont(1200, 120)).toBe(12.5); expect(fitFont(400, 120)).toBe(8); expect(fitFont(800, 120)).toBe(11);
});
```

```ts
// modal tests
it("a remote card is live only when it says so", () => {
  // isLive({...native card, machine:"desk", live:true}, "terminal") → true; live absent → false; local native → true.
});
it("the phone's live viewer shows the extra keys and they type", () => {
  // setComposerOptions({ terminalKeys: true }); open a remote live card in the terminal look;
  // .modal__keys has 7 buttons in EXTRA_KEYS order; clicking "Ctrl+C" invokes live_input_remote with data b64("\x03").
});
```

- [ ] **Step 2: Run them to see them fail** — `pnpm vitest run src/live.test.ts src/modal*.test.ts`.

- [ ] **Step 3: Implement, as described in Interfaces.**

- [ ] **Step 4: Run the tests and builds** — `pnpm test && pnpm build && pnpm build:mobile` — expected: PASS, with no chunk-size warning.

- [ ] **Step 5: Commit** — `feat(viewer): draw a remote live terminal, with extra keys on the phone`

---

### Task 8: Docs and release

**Files:**
- Modify: `README.md` (the Native paragraphs: the live terminal also opens from the main for an assistant's session and from the phone, through the Maya that runs it; it keeps its size there; the phone's extra keys; both Mayas need this version or later, else the transcript shows)
- Modify: `RELEASE_NOTES.md` (new top section), version files via `sh scripts/set-version.sh <version>`

- [ ] **Step 1: Update the README.**
- [ ] **Step 2: Bump.** From `main`'s version, bump one minor, using `sh scripts/set-version.sh`. Add `# <version>` with `## New` bullets in plain words:
  - the live terminal opens on the main for an assistant's Native session and on the phone, and you can type into it;
  - it keeps the size of the Maya that runs it;
  - the phone has Esc, Tab, Ctrl+C and arrow keys under it;
  - it reconnects by itself after the connection drops;
  - both Mayas must have this version or later.
- [ ] **Step 3: Check** — `sh scripts/release-notes.sh check <version>`, then `cargo test --workspace && pnpm test`.
- [ ] **Step 4: Commit** — `chore(release): <version>` (version files and `RELEASE_NOTES.md` only); the README goes in a `docs:` commit before it.
