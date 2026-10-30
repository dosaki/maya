# Native Terminal (local) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A fourth "New sessions open in" choice, Native, where the desktop app runs a session's terminal itself, shows it live in the session viewer, warns before a quit stops a working one, and offers it back after a restart.

**Architecture:** The desktop app (`src-tauri`) gets a `native` module. It holds one pseudo-terminal per Native session (`portable-pty`), feeds every byte to a headless `vt100` screen, and streams batched output to the viewer over a Tauri `Channel`. A routing wrapper puts the host in front of each platform's existing `Terminal`, so every card action reaches a Native session through the unchanged `core` seam. The core learns only three things: a `native` flag on cards and remembered sessions, a hook that says which process ids are Native roots, and two defaulted `Terminal` methods.

**Tech Stack:** Rust (Tauri 2, `portable-pty` 0.9, `vt100` 0.18), TypeScript (`@xterm/xterm` 6, `@xterm/addon-fit` 0.11), vitest.

**Spec:** `docs/superpowers/specs/2026-10-30-native-terminal-design.md`

**Scope of this plan:** The spec's "Remote viewers" section (the live terminal on the main and on the phone, `CAP_TERMINAL`, the phone's extra keys) is a second plan, written after this one lands. In this plan a remote Native card keeps today's transcript views, which the spec already requires for a Maya without the capability.

## Global Constraints

- Config value: `"native"` in `newSessionTerminal`. The default stays `"terminal"`.
- Scrollback: 10,000 lines. A local viewer gets all of it on attach.
- New terminals start at 40 rows × 120 columns. Resizes clamp to at least 5 rows × 20 columns.
- Output batching interval: 16 ms.
- Environment of a Native terminal: `TERM=xterm-256color`, `COLORTERM=truecolor`. On Linux, also remove the AppImage variables `launch::scrub_appimage_env` removes.
- Command line: on macOS and Linux, `<shell> -l -c <command>`, where shell is `$SHELL` when it is an absolute path that exists, else `/bin/zsh` on macOS and `/bin/bash` on Linux. On Windows, `<git bash> -l -c <command>` (`maya_core::launch::git_bash()`). No interactive shell is left behind.
- Every external terminal path (Terminal.app, iTerm2, Windows Terminal, tmux) behaves exactly as today.
- Copy and paste in the live terminal: Cmd+C and Cmd+V on macOS; Ctrl+Shift+C and Ctrl+Shift+V on Windows and Linux.
- Release: a `feat`, so bump `main`'s version by one minor (0.43.0 → 0.44.0 if `main` has not moved) with `sh scripts/set-version.sh`, in its own `chore(release): <version>` commit, together with the new top section of `RELEASE_NOTES.md`.

## Review Focus

1. **The agent exits by itself** (the user types `/exit` or Ctrl+D in the live terminal) while the viewer is open. The viewer keeps the final screen and says the session ended. The host drops the terminal, and nothing panics. *Test: Task 4 `a_terminal_whose_program_exits_sends_exit_and_is_dropped`.*
2. **A burst of output** (`cat` of a large file, a fast-redrawing TUI) reaches the viewer as a few batched frames, not one frame per read. *Test: Task 4 `a_burst_of_output_arrives_in_few_frames`.*
3. **A multi-byte character split across two reads** is neither corrupted in the snapshot nor in the stream. *Test: Task 3 `a_character_split_across_two_feeds_survives_the_snapshot`. Bytes travel base64-encoded, never as `String`.*
4. **Maya opened from the Dock with a bare `PATH`** still finds `claude`, because the command runs in a login shell. *Test: Task 4 `the_command_runs_in_a_login_shell`.*
5. **A collapsed or tiny viewer** asks for a 0 × 0 or 3 × 2 size. The PTY is clamped to 5 × 20 and never errors. *Test: Task 4 `a_tiny_resize_is_clamped`.*

---

## File Structure

| File | Responsibility |
|---|---|
| `core/src/config.rs` | `NewSessionTerminal::Native` |
| `core/src/model.rs` | `Card.native` |
| `core/src/native.rs` (new) | `root_of`: which Native root, if any, a pid descends from |
| `core/src/store.rs` | `native_roots` hook; marks `card.native` each refresh |
| `core/src/remembered.rs` | `LiveSession.native`, `Remembered.native` |
| `core/src/terminal.rs` | `open_native`, `closes_with_agent` (defaulted); `FakeTerminal` support |
| `core/src/actions.rs` | Native remembered entries resume through `open_native`; `close_session` skips the shell `exit` for terminals that close with the agent |
| `src-tauri/src/native/mod.rs` (new) | `pub mod screen; pub mod host; pub mod route;` and `pub static HOST` |
| `src-tauri/src/native/screen.rs` (new) | `Screen`: vt100 wrapper and the snapshot |
| `src-tauri/src/native/host.rs` (new) | `NativeHost`: spawn, read, batch, write, resize, attach, detach, kill |
| `src-tauri/src/native/route.rs` (new) | `WithNative`: the `Terminal` in front of the platform terminal |
| `src-tauri/src/native_app.rs` (new) | Tauri commands `native_attach`, `native_input`, `native_resize`, `native_detach` |
| `src-tauri/src/quit_guard.rs` (new) | Working-Native check, exit and close interception, `quit_confirmed`, `native_working` |
| `src-tauri/src/lib.rs` | Wires the route, the store hook, the commands and the quit guard |
| `src-tauri/src/terminal_app.rs` | `Native` arm in `MacTerminal::open` |
| `src-tauri/src/updater.rs` | Comment corrected |
| `src/types.ts` | `NewSessionTerminal` gains `"native"`; `Card.native?` |
| `src/settings.ts` | Per-platform options; shown on every desktop platform |
| `src/live.ts` (new) | `mountLive`: the xterm.js terminal bound to a Native session |
| `src/modal.ts` | Live terminal look for local Native cards; Esc; repaint keeps the terminal |
| `src/main.ts` | A Native card's Terminal button opens the viewer; xterm CSS; the quit dialog listener |
| `src/quitguard.ts` (new) | The quit and update confirmation |
| `src/update.ts` | Asks before installing when Native sessions are working |
| `README.md`, `RELEASE_NOTES.md`, `docs/DEVELOPING.md` | Docs and release |

---

### Task 1: The Native setting

**Files:**
- Modify: `core/src/config.rs:244-252` (enum), tests near `:798`
- Modify: `src-tauri/src/terminal_app.rs:171-177`
- Modify: `src/types.ts:333`, `src/settings.ts:834-849`
- Test: `core/src/config.rs` tests, `src/settings.test.ts:1349-1373`

**Interfaces:**
- Produces: `NewSessionTerminal::Native` (serde `"native"`); TS `type NewSessionTerminal = "terminal" | "iterm" | "native"`; `export function terminalChoices(): [NewSessionTerminal, string][]` in `src/settings.ts`.

- [ ] **Step 1: Write the failing tests**

Rust, in `core/src/config.rs` tests:
```rust
#[test]
fn native_round_trips_as_native() {
    let set = Config { new_session_terminal: NewSessionTerminal::Native, ..Config::default() };
    let text = serde_json::to_string(&set).unwrap();
    assert!(text.contains(r#""newSessionTerminal":"native""#), "{text}");
    assert_eq!(serde_json::from_str::<Config>(&text).unwrap().new_session_terminal, NewSessionTerminal::Native);
}
```
TS, in `src/settings.test.ts`. Replace the two tests that expect no select off macOS (`:1370`, `:1372`) with these, mocking the platform the way the neighbouring tests do:
```ts
it("offers Terminal, iTerm2 and Native on macOS", () => {
  expect(terminalChoices()).toEqual([["terminal", "Terminal"], ["iterm", "iTerm2"], ["native", "Native"]]);
});
it("offers Windows Terminal and Native on Windows", () => {
  expect(terminalChoices()).toEqual([["terminal", "Windows Terminal"], ["native", "Native"]]);
});
it("offers Terminal (tmux) and Native on Linux", () => {
  expect(terminalChoices()).toEqual([["terminal", "Terminal (tmux)"], ["native", "Native"]]);
});
it("shows New sessions open in on every desktop platform and saves native", () => {
  // Windows and Linux: the select exists; choosing native calls onNewSessionTerminal("native").
});
it("shows Terminal on Windows when the saved value is iterm", () => {
  // value "iterm" off macOS renders as "terminal".
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core config::tests::native_round_trips_as_native` and `pnpm test src/settings.test.ts`
Expected: FAIL (`no variant named Native`; `terminalChoices` is not exported).

- [ ] **Step 3: Implement**

- Add `Native` to `NewSessionTerminal`, after `Iterm`. Update its doc comment: on macOS it is "the app that opens the windows of sessions Maya starts, or Native", and on every platform Native means Maya hosts the terminal.
- In `MacTerminal::open`, match `NewSessionTerminal::Native` with `Terminal`. `WithNative` (Task 5) takes Native before the call reaches here, so this arm only keeps the match exhaustive.
- In `src/settings.ts`, add `terminalChoices()` (using `isMac`/`isWindows`/`isLinux` from `./platform`). Drop the `if (isMac())` guard around the select. A saved value missing from the list shows as `"terminal"`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test -p maya-core config` and `pnpm test src/settings.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs src-tauri/src/terminal_app.rs src/types.ts src/settings.ts src/settings.test.ts
git commit -m "feat(settings): Native choice for where new sessions open"
```

---

### Task 2: Cards and remembered sessions know they are Native

**Files:**
- Create: `core/src/native.rs` (add `pub mod native;` to `core/src/lib.rs`)
- Modify: `core/src/model.rs:115-172` (`Card`), every `Card { … brain: false, … }` literal in the workspace (add `native: false` beside it)
- Modify: `core/src/store.rs` (field beside `opencode_windows` at `:33`, default at `:217`, refresh at `:758-761`)
- Modify: `core/src/remembered.rs:36-77`, `observe` at `:178`
- Modify: `src/types.ts` (`Card`)

**Interfaces:**
- Produces:
  - `Card.native: bool`, serde `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`, wire name `native`; TS `native?: boolean`.
  - `pub fn root_of(pid: i32, roots: &[i32], tree: &[crate::foreign::Proc]) -> Option<i32>` in `core/src/native.rs`: walks `ppid` from `pid` until it reaches a pid in `roots` (returns it), pid 0 or 1, or a loop (returns `None`).
  - `Store::set_native_roots(&mut self, roots: Box<dyn Fn() -> Vec<i32> + Send>)`. Default `Box::new(Vec::new)`.
  - `LiveSession.native: bool` (`LiveSession::of` copies `card.native`); `Remembered.native: bool` with `#[serde(default, skip_serializing_if = "is_false")]`.

- [ ] **Step 1: Write the failing tests**

```rust
// core/src/native.rs
#[test]
fn a_pid_under_a_root_finds_that_root() {
    let tree = vec![proc(10, 1), proc(20, 10), proc(30, 20)];
    assert_eq!(root_of(30, &[10], &tree), Some(10));
    assert_eq!(root_of(10, &[10], &tree), Some(10));
}
#[test]
fn a_pid_outside_every_root_or_in_a_loop_finds_none() {
    let tree = vec![proc(10, 1), proc(40, 41), proc(41, 40)];
    assert_eq!(root_of(10, &[99], &tree), None);
    assert_eq!(root_of(40, &[99], &tree), None);
}

// core/src/model.rs
#[test]
fn a_native_flag_is_omitted_when_false_and_read_back_when_true() { /* as the brain test at :199 */ }

// core/src/store.rs (with an existing test store, a registered session pid 30 whose parent chain reaches 10)
#[test]
fn a_session_under_a_native_root_is_marked_native() {
    // store.set_native_roots(Box::new(|| vec![10])); refresh; the card's native is true;
    // with roots returning vec![] the same card is not native.
}

// core/src/remembered.rs
#[test]
fn observe_records_that_a_session_is_native() {
    // observe(&[LiveSession { native: true, ..live("a") }]); entry("a").native == true;
    // a later observe with native: false clears it.
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core native` and `cargo test -p maya-core remembered`
Expected: FAIL (module and fields missing).

- [ ] **Step 3: Implement**

The store asks for the roots once per refresh. Only when the list is non-empty does it read `self.process_tree()` and set `card.native = native::root_of(card.pid, &roots, &tree).is_some()`. It does this before the remembered `observe` at `:758`, so `LiveSession::of` sees the flag. `observe` updates `native` on a known entry and marks the record dirty when the value changed.

- [ ] **Step 4: Run the workspace tests and see them pass**

Run: `cargo test --workspace`
Expected: PASS (every `Card` literal compiles with `native: false`).

- [ ] **Step 5: Commit**

```bash
git add -A core src-tauri mobile cli src/types.ts
git commit -m "feat(core): mark Native sessions on cards and in the remembered list"
```

---

### Task 3: The screen and its snapshot

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `vt100 = "0.18"`, `portable-pty = "0.9"`, `base64 = { workspace = true }`)
- Create: `src-tauri/src/native/mod.rs`, `src-tauri/src/native/screen.rs`; add `pub mod native;` to `src-tauri/src/lib.rs` (`#[cfg(not(target_os = "android"))]` is not needed: `src-tauri` is desktop-only)

**Interfaces:**
- Produces, in `native::screen`:
  - `pub const SCROLLBACK: usize = 10_000;`
  - `pub struct Screen` with `pub fn new(rows: u16, cols: u16) -> Screen`, `pub fn feed(&mut self, bytes: &[u8])`, `pub fn resize(&mut self, rows: u16, cols: u16)`, `pub fn size(&self) -> (u16, u16)` (rows, cols), and `pub fn snapshot(&mut self, scrollback: usize) -> Vec<u8>`.

- [ ] **Step 1: Write the failing tests**

Each snapshot test feeds the snapshot into a fresh `vt100::Parser` of the same size and `SCROLLBACK`, then compares.
```rust
#[test]
fn a_snapshot_redraws_the_same_screen_and_cursor() {
    // feed "\x1b[31mred\x1b[0m\r\nplain"; replay snapshot(0) into a fresh parser;
    // screen().contents() equal; cursor_position() equal; cell(0,0).fgcolor() == Idx(1).
}
#[test]
fn a_snapshot_carries_the_scrollback_it_is_asked_for() {
    // 5 rows; feed lines "l0".."l19"; replay snapshot(SCROLLBACK);
    // the fresh parser's scrollback_len() is 15 and set_scrollback(15) shows "l0" on row 0;
    // snapshot(3) gives scrollback_len() 3.
}
#[test]
fn a_snapshot_of_the_alternate_screen_enters_it() {
    // feed "\x1b[?1049hTUI"; replay; fresh.screen().alternate_screen() is true and contents() has "TUI".
}
#[test]
fn a_snapshot_keeps_the_input_modes() {
    // feed "\x1b[?2004h\x1b[?1h"; replay; bracketed_paste() and application_cursor() are true.
}
#[test]
fn a_character_split_across_two_feeds_survives_the_snapshot() {
    // "é" is [0xC3, 0xA9]: feed(&[b'a', 0xC3]); feed(&[0xA9]); replayed contents() == "aé".
}
#[test]
fn the_snapshot_leaves_the_live_view_at_the_bottom() {
    // after snapshot(SCROLLBACK), the screen's scrollback() offset is 0 again.
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya native::screen`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement the snapshot**

`Screen` wraps `vt100::Parser::new(rows, cols, SCROLLBACK)`. `snapshot` builds, in this order:
1. `"\x1bc"` (full reset).
2. When the primary screen is active, the last `min(scrollback, scrollback_len())` lines of history, oldest first. Read them page by page: for offset `s` from `n` down to 1 in steps of `rows`, `set_scrollback(s)`, then take the first `min(rows, s)` of `rows_formatted(0, cols)`. Write each line followed by `"\x1b[0m\r\n"`. Then `set_scrollback(0)`, and write `rows` × `"\r\n"` so all of them scroll off into the viewer's history.
3. `"\x1b[?1049h"` when `alternate_screen()`.
4. `state_formatted()` (visible contents, cursor, input modes).

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya native::screen`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml Cargo.lock src-tauri/src/lib.rs src-tauri/src/native
git commit -m "feat(native): headless screen with scrollback snapshots"
```

---

### Task 4: The native host

**Files:**
- Create: `src-tauri/src/native/host.rs`; `HOST` in `src-tauri/src/native/mod.rs`
- Modify: `core/src/launch.rs` (expose the AppImage variable list `scrub_appimage_env` removes as `pub const APPIMAGE_VARS: &[&str]`, and use it there)

**Interfaces:**
- Consumes: `Screen` (Task 3).
- Produces, in `native::host`:
```rust
pub type TermId = u64;
pub const DEFAULT_ROWS: u16 = 40;
pub const DEFAULT_COLS: u16 = 120;
pub const BATCH: Duration = Duration::from_millis(16);
pub enum Frame { Output(Vec<u8>), Exit }
/// A viewer's sink; returning false detaches it.
pub type Sink = Box<dyn Fn(Frame) -> bool + Send>;
pub struct Attached { pub term: TermId, pub viewer: u64, pub snapshot: Vec<u8>, pub rows: u16, pub cols: u16 }
pub fn shell_argv(command: &str) -> Vec<String>; // the Global Constraints command line
impl NativeHost {
    pub fn new(batch: Duration) -> NativeHost;
    pub fn spawn(&self, command: &str, cwd: &Path) -> Result<TermId, String>;
    pub fn write(&self, term: TermId, bytes: &[u8]) -> Result<(), String>;
    pub fn resize(&self, term: TermId, rows: u16, cols: u16) -> Result<(), String>;
    pub fn attach(&self, term: TermId, scrollback: usize, sink: Sink) -> Result<Attached, String>;
    pub fn detach(&self, term: TermId, viewer: u64);
    pub fn term_for_tty(&self, tty: &str) -> Option<TermId>;
    pub fn term_for_root(&self, root_pid: i32) -> Option<TermId>;
    pub fn root_pids(&self) -> Vec<i32>;
    pub fn kill_all(&self);
}
```
- In `native/mod.rs`: `pub static HOST: LazyLock<host::NativeHost> = LazyLock::new(|| host::NativeHost::new(host::BATCH));`

- [ ] **Step 1: Write the failing tests** (`#[cfg(unix)]`; they spawn real PTYs running `/bin/sh`)

```rust
#[test]
fn the_command_runs_in_a_login_shell() {
    // with SHELL=/bin/sh: shell_argv("claude") == ["/bin/sh", "-l", "-c", "claude"];
    // with SHELL unset or relative: the platform fallback (/bin/zsh on macOS, /bin/bash on Linux).
}
#[test]
fn output_reaches_an_attached_viewer_and_the_snapshot() {
    // spawn("printf ready; cat"); attach with a sink collecting Output bytes;
    // within 2 s the collected bytes or the snapshot contain "ready".
}
#[test]
fn written_bytes_reach_the_program() {
    // spawn("cat"); attach; write(b"ping\r"); within 2 s the output contains "ping".
}
#[test]
fn a_burst_of_output_arrives_in_few_frames() {
    // NativeHost::new(50 ms); spawn("i=0; while [ $i -lt 2000 ]; do printf x; i=$((i+1)); done; sleep 1");
    // after 1 s: the total of Output bytes counts 2000 'x', in at most 40 frames.
}
#[test]
fn a_terminal_whose_program_exits_sends_exit_and_is_dropped() {
    // spawn("exit 0"); attach; within 2 s the sink saw Frame::Exit;
    // term_for_root(old root) is None and write(term, b"x") is Err.
}
#[test]
fn a_tiny_resize_is_clamped() {
    // spawn("cat"); resize(term, 0, 0) is Ok; attach().rows == 5 and .cols == 20.
}
#[test]
fn a_sink_that_returns_false_is_detached() {
    // a sink returning false; after the next output, the host holds no viewer for it.
}
#[test]
fn the_terminal_is_found_by_its_tty_and_its_root() {
    // spawn("cat"); root_pids() has one pid; term_for_root(pid) == Some(term);
    // term_for_tty with the PTY's device, with and without "/dev/", == Some(term).
}
#[test]
fn kill_all_ends_every_program() {
    // spawn two "sleep 30"; kill_all(); within 2 s both root pids are no longer alive.
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya native::host`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement**

- `spawn`: `native_pty_system().openpty(PtySize { rows: DEFAULT_ROWS, cols: DEFAULT_COLS, .. })`. Build a `CommandBuilder` from `shell_argv(command)` and set `cwd`, `TERM` and `COLORTERM`; on Linux remove `APPIMAGE_VARS`. Then `slave.spawn_command`. Keep the master, the writer (`take_writer`), the child, its pid (the root) and `master.tty_name()` (Unix). Drop the slave once spawned.
- Reader: one thread per terminal. It loops `read` into a 64 KiB buffer, feeds `Screen`, and appends to the terminal's `pending`. On EOF or error: mark ended, flush `pending`, send `Frame::Exit` to every viewer, and remove the terminal.
- Flusher: one host-wide thread. Every `batch`, it moves each terminal's non-empty `pending` into one `Frame::Output` per viewer and drops viewers whose sink returns false.
- `attach` takes the snapshot and registers the sink under the same lock as `pending`, so no byte falls between them.
- `resize` clamps to at least 5 × 20, then resizes both the master and the `Screen`.
- `term_for_tty` compares with the leading `/dev/` stripped on both sides. On Windows, it parses `maya_core::win_console::console_pid` and walks `maya_core::win_process::tree()` with `maya_core::native::root_of`.
- `kill_all`: on Unix, `libc::kill(-root, SIGHUP)` then `child.kill()`, because portable-pty makes the child a session leader. On Windows, `child.kill()`; closing the pseudo console ends its processes.

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya native::host`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/native core/src/launch.rs
git commit -m "feat(native): host pseudo-terminals with batched output"
```

---

### Task 5: Route card actions to Native terminals

**Files:**
- Modify: `core/src/terminal.rs` (trait, `FakeTerminal`, `Call`)
- Modify: `core/src/actions.rs:791-830` (`resume_remembered_with`), `:347-382` (`close_session_with`)
- Create: `src-tauri/src/native/route.rs`
- Modify: `src-tauri/src/lib.rs:37-44` (`terminal()`), store setup (where the store is built for `AppState`), `RunEvent::Exit` at `:1837`

**Interfaces:**
- Consumes: `NativeHost` (Task 4), `NewSessionTerminal::Native` (Task 1), `Store::set_native_roots` and `Remembered.native` (Task 2).
- Produces:
  - On `Terminal`: `fn open_native(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> { self.open(command, cwd, label) }` and `fn closes_with_agent(&self, _tty: &str) -> bool { false }`.
  - `Call::OpenNative { command, cwd, label }`; `FakeTerminal.native: Mutex<HashSet<String>>` (ttys for which `closes_with_agent` is true).
  - `pub struct WithNative { pub inner: &'static dyn Terminal, pub host: &'static NativeHost, pub choice: fn() -> NewSessionTerminal }` implementing `Terminal`; `pub static TERMINAL: LazyLock<WithNative>` in `native::route`.

- [ ] **Step 1: Write the failing tests**

```rust
// core/src/actions.rs
#[test]
fn a_native_remembered_session_resumes_through_open_native() {
    // after_restart with was(ClaudeCode, …) marked native: true and another not native;
    // resume_remembered_with: the native one is Call::OpenNative, the other Call::Open.
}
#[test]
fn closing_a_session_whose_terminal_closes_with_it_types_no_shell_exit() {
    // fake.native has the tty; close_session_with(…, |_| true) is Ok;
    // calls hold one Type "/exit" and no Type "exit".
}

// src-tauri/src/native/route.rs (stub inner and a real host, #[cfg(unix)] for spawning)
#[test]
fn open_follows_the_setting() {
    // choice Native: open spawns in the host and returns Ok(None), inner untouched;
    // choice Terminal: inner.open called once.
}
#[test]
fn open_native_always_uses_the_host() {}
#[test]
fn keys_for_a_native_tty_go_to_the_host_and_others_to_the_inner_terminal() {
    // type_line on the host's tty writes "text\r" to the PTY (cat echoes it);
    // type_line on "/dev/ttys999" reaches the stub.
}
#[test]
fn a_reply_to_a_native_tty_is_written_in_one_go() {
    // type_reply(["a","b"]) writes answer::reply_keys(["a","b"]) as one write.
}
#[test]
fn a_native_tty_closes_with_its_agent_and_has_no_focus_window() {
    // closes_with_agent(native tty) is true; for any other tty, false.
    // focus(native tty) is Err("Open the session from its card.").
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya-core actions` and `cargo test -p maya native::route`
Expected: FAIL

- [ ] **Step 3: Implement**

- `WithNative`: every tty method asks `host.term_for_tty(tty)` first. `type_line` writes `text + "\r"`, `press_keys` writes the keys as given, and `type_reply` writes `answer::reply_keys(lines)` (failure is `(0, why)`). `name_for_tty` and `names_for_ttys` delegate to `inner` for ttys the host does not own. `reach_after_exit` on a Native tty returns `vec![]`.
- `focus` on a Native tty is an error. The frontend never calls it for Native cards (Task 7); the message is a safety net.
- `resume_remembered_with` calls `open_native` for an entry with `native: true`, else `open` as today. `close_session_with` returns `Ok(())` after the agent exits when `l.terminal.closes_with_agent(&tty)`, without the `exit` loop.
- In `lib.rs`, `terminal()` returns `&*native::route::TERMINAL` on every platform, with `inner` set to today's `term::TERMINAL`. Set the store's roots to `|| native::HOST.root_pids()` where `AppState`'s store is made. In `RunEvent::Exit`, call `native::HOST.kill_all()` before `brain_app::quit`.

- [ ] **Step 4: Run the workspace tests and see them pass**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add core/src/terminal.rs core/src/actions.rs src-tauri/src/native/route.rs src-tauri/src/native/mod.rs src-tauri/src/lib.rs
git commit -m "feat(native): route new sessions and card actions to Native terminals"
```

---

### Task 6: Commands the viewer uses

**Files:**
- Create: `src-tauri/src/native_app.rs`; register it in `src-tauri/src/lib.rs` (`mod native_app;` and the four commands in `generate_handler!` at `:1655`)

**Interfaces:**
- Consumes: `HOST`, `Attached`, `Frame` (Task 4); `maya_core::native::root_of` (Task 2); `store.process_tree()`.
- Produces (Tauri commands; bytes are base64 strings):
```rust
#[derive(Serialize, Clone)] #[serde(tag = "kind", rename_all = "lowercase")]
pub enum NativeFrame { Output { data: String }, Exit }
#[derive(Serialize)] #[serde(rename_all = "camelCase")]
pub struct NativeAttached { pub term: u64, pub viewer: u64, pub snapshot: String, pub rows: u16, pub cols: u16 }
#[tauri::command] fn native_attach(state: TauriState<AppState>, pid: i32, on_frame: tauri::ipc::Channel<NativeFrame>) -> Result<NativeAttached, String>;
#[tauri::command] fn native_input(term: u64, data: String) -> Result<(), String>;
#[tauri::command] fn native_resize(term: u64, rows: u16, cols: u16) -> Result<(), String>;
#[tauri::command] fn native_detach(term: u64, viewer: u64);
```
- `native_attach` finds the terminal by `root_of(pid, &HOST.root_pids(), &store.process_tree())` then `term_for_root`. It attaches with `screen::SCROLLBACK`. Its sink sends `NativeFrame` on the channel and returns `false` when `send` fails. With no terminal it returns `Err("This session is not running in Maya.")`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn frames_go_to_the_viewer_as_base64() {
    // frame_of(Frame::Output(vec![0xC3, 0xA9])) serialises to {"kind":"output","data":"w6k="};
    // frame_of(Frame::Exit) to {"kind":"exit"}.
}
#[test]
fn input_is_decoded_before_it_is_written() {
    // decode_input("aGk=") == b"hi"; decode_input("!!") is Err.
}
```
(Keep the encoding in two pure functions, `frame_of(Frame) -> NativeFrame` and `decode_input(&str) -> Result<Vec<u8>, String>`. The commands are thin.)

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya native_app`
Expected: FAIL

- [ ] **Step 3: Implement the four commands with the signatures above**

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya native_app`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/native_app.rs src-tauri/src/lib.rs
git commit -m "feat(native): commands to attach, type into and resize a Native terminal"
```

---

### Task 7: The live terminal in the viewer

**Files:**
- Modify: `package.json` (`@xterm/xterm` ^6.0.0, `@xterm/addon-fit` ^0.11.0)
- Create: `src/live.ts`, `src/live.test.ts`
- Modify: `src/modal.ts` (`renderModal` `:557`, `patchModal` `:759`, `paint` `:916`, `openModal` keyHandler `:1117`, `closeModal` `:1140`, `toggleTheme` `:1029`), `src/modal.test.ts`
- Modify: `src/main.ts:146` (the card's Terminal button), and import `@xterm/xterm/css/xterm.css` there
- Modify: `src/styles.css` (`.modal__live` fills the panel between head and foot)

**Interfaces:**
- Consumes: the Task 6 commands; `Card.native` (Task 2).
- Produces:
  - `export interface LiveHandle { dispose(): void; focus(): void }`
  - `export function mountLive(host: HTMLElement, pid: number): LiveHandle` in `src/live.ts`
  - `export function isLive(card: Card, theme: ViewerTheme | undefined): boolean` in `src/modal.ts`: `theme === "terminal" && !!card.native && !card.machine`
  - `export function copyPasteKey(ev: KeyboardEvent, mac: boolean): "copy" | "paste" | null` in `src/live.ts`

- [ ] **Step 1: Write the failing tests** (mock `@xterm/xterm`, `@xterm/addon-fit` and `@tauri-apps/api/core` with `vi.mock`; jsdom has no canvas)

```ts
// src/live.test.ts
it("attaches by pid, writes the snapshot, then streamed output", async () => {
  // invoke("native_attach", { pid: 42, onFrame }) resolves { term: 7, viewer: 1, snapshot: "aGk=", rows: 40, cols: 120 };
  // the fake Terminal's write got bytes "hi"; an output frame "eA==" writes "x".
});
it("sends typed keys as base64 to native_input", () => {
  // fake onData("é") → invoke("native_input", { term: 7, data: "w6k=" }).
});
it("resizes the terminal to the fitted size", () => {
  // after fit proposes 30x100 → invoke("native_resize", { term: 7, rows: 30, cols: 100 }).
});
it("shows that the session ended on an exit frame and stops sending input", () => {});
it("detaches on dispose", () => { /* invoke("native_detach", { term: 7, viewer: 1 }) */ });
it("maps the platform's copy and paste keys", () => {
  // mac: Cmd+C → "copy", Cmd+V → "paste", Ctrl+C → null;
  // not mac: Ctrl+Shift+C → "copy", Ctrl+Shift+V → "paste", Ctrl+C → null.
});

// src/modal.test.ts
it("a local Native card in the terminal look is a live terminal and nothing else", () => {
  // renderModal({ card: { ...card, native: true }, theme: "terminal", … }):
  // has .modal__live; has no .modal__composer, .modal__banner, .modal__history or menu, even when awaiting.
});
it("a local Native card in the chat look keeps its reply box", () => {});
it("a remote Native card in the terminal look shows the transcript", () => { /* machine: "laptop" → no .modal__live */ });
it("Escape inside the live terminal does not close the viewer", () => {});
it("a repaint keeps the live terminal's element", () => {
  // patchModal(root, fresh) where both have .modal__live: the old node is still in root.
});
```

- [ ] **Step 2: Run them and see them fail**

Run: `pnpm test src/live.test.ts src/modal.test.ts`
Expected: FAIL

- [ ] **Step 3: Implement**

- `mountLive`: create `new Terminal({ scrollback: 10000, cursorBlink: true, allowProposedApi: false })` with the fit add-on, and `open(host)`. Its colours come from the viewer's terminal-look CSS variables, read with `getComputedStyle`. Create a `Channel<NativeFrame>` and attach. Then `reset()` and write the decoded snapshot. Output frames write decoded bytes. An exit frame writes a dim `"\r\n[Session ended]"` line and stops input. `onData` sends `native_input`. A `ResizeObserver` on `host` calls `fit()`, then `native_resize` when rows or columns changed. `attachCustomKeyEventHandler` uses `copyPasteKey`: copy writes the selection with `navigator.clipboard.writeText`, and paste reads the clipboard and calls `term.paste`. Both return `false`, so xterm does not also send the key.
- `renderModal`: when `isLive(m.card, m.theme)`, the panel holds the head, a `div.modal__live`, and `statusFoot` only.
- `paint`: after rendering, if the model is live and no handle exists, mount into `.modal__live` and keep the handle on `current`. If the model is no longer live (theme toggled, card gone), dispose it. `closeModal` disposes it. `patchModal` leaves an existing `.modal__live` in place.
- `openModal`'s key handler ignores Escape whose target is inside `.modal__live`.
- In `main.ts`, the card's Terminal button calls `openModal(card)` when `card.native && !card.machine`, else `focus_session` as today. In the chat look, the banner's "Open terminal" button is not shown for a local Native card, and its text says "Answer below." in place of "…or in its terminal".

- [ ] **Step 4: Run all frontend tests and see them pass**

Run: `pnpm test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add package.json pnpm-lock.yaml src/live.ts src/live.test.ts src/modal.ts src/modal.test.ts src/main.ts src/styles.css
git commit -m "feat(viewer): live terminal for Native sessions"
```

---

### Task 8: Warn before a quit or an update stops a working Native session

**Files:**
- Create: `src-tauri/src/quit_guard.rs`; register in `src-tauri/src/lib.rs` (`mod quit_guard;`, the two commands, `.run(...)` at `:1833`, and a `on_window_event` for `WindowEvent::CloseRequested`)
- Modify: `src-tauri/src/updater.rs:70-72` (comment), `src-tauri/Cargo.toml` (`objc2-app-kit` features: add `"NSWorkspace"`)
- Create: `src/quitguard.ts`, `src/quitguard.test.ts`
- Modify: `src/main.ts` (listen for `quit-confirm`), `src/update.ts:96-99`

**Interfaces:**
- Consumes: `Card.native` (Task 2).
- Produces:
  - `pub fn working_native(cards: &[Card]) -> Vec<String>`: names of cards with `native`, `state == State::Working`, `machine.is_none()` and `!brain`.
  - `pub fn should_ask(confirmed: bool, system_ending: bool, working: &[String]) -> bool`
  - Commands: `native_working(state) -> Vec<String>` and `quit_confirmed(app)` (sets `CONFIRMED`, then `app.exit(0)`).
  - Event `quit-confirm` with payload `{ names: string[] }`.
  - TS: `export function quitText(names: string[], update: boolean): { text: string; yes: string }` and `export function askQuit(names: string[], update: boolean): Promise<boolean>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn only_local_working_native_cards_count() {
    // native+working+local → named; native+idle, native+working+remote, working+not native, brain → not.
}
#[test]
fn it_asks_only_when_something_would_stop_and_nobody_already_said_yes() {
    assert!(should_ask(false, false, &["a".into()]));
    assert!(!should_ask(true, false, &["a".into()]));
    assert!(!should_ask(false, true, &["a".into()]));
    assert!(!should_ask(false, false, &[]));
}
```
```ts
it("names the sessions that will stop", () => {
  expect(quitText(["fix-ci", "docs"], false)).toEqual({
    text: "These Native sessions are working and will stop if Maya quits: fix-ci, docs.",
    yes: "Quit anyway",
  });
  expect(quitText(["fix-ci"], true)).toEqual({
    text: "This Native session is working and will stop when Maya restarts to update: fix-ci.",
    yes: "Restart anyway",
  });
});
it("resolves true on the yes button and false on Cancel", async () => {});
it("installs the update without asking when no Native session is working", async () => {});
it("does not install the update when the user cancels", async () => {});
```

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test -p maya quit_guard` and `pnpm test src/quitguard.test.ts src/update.test.ts`
Expected: FAIL

- [ ] **Step 3: Implement**

- `RunEvent::ExitRequested { api, .. }` and `WindowEvent::CloseRequested { api, .. }` both work out `working_native` from a store refresh, outside any held lock. When `should_ask`: `api.prevent_exit()` or `api.prevent_close()`, show and focus the main window, and emit `quit-confirm`.
- `SYSTEM_ENDING`: on macOS, observe `NSWorkspaceWillPowerOffNotification` on the shared workspace's notification centre and set the flag. On Windows and Linux the system ends the process itself; leave the flag false there.
- `askQuit` renders a dialog in `#modal-host` with `quitText`'s sentence, a primary button (`yes`) and Cancel. The `quit-confirm` listener in `main.ts` calls `quit_confirmed` when it resolves true.
- In `update.ts`, before `update_install`, call `native_working`. When it is non-empty and `askQuit(names, true)` resolves false, return to the available state without installing.
- The `updater.rs` comment becomes: "Sessions in external terminals live on; Native sessions end, which the window asks about before it calls this."

- [ ] **Step 4: Run them and see them pass**

Run: `cargo test -p maya quit_guard` and `pnpm test`
Expected: PASS

- [ ] **Step 5: Check the real app** (`pnpm tauri dev`)

1. Set Native. Start a session with a long task.
2. Quit with Cmd+Q, then by closing the window. Both ask; Cancel keeps Maya running and Quit anyway quits.
3. With the session Idle, quitting does not ask.
4. If Cmd+Q does not raise `ExitRequested` on macOS, replace the app menu's predefined Quit item with a custom one that runs the same check. Record which was needed in the commit message.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/quit_guard.rs src-tauri/src/lib.rs src-tauri/src/updater.rs src-tauri/Cargo.toml Cargo.lock src/quitguard.ts src/quitguard.test.ts src/main.ts src/update.ts src/update.test.ts
git commit -m "feat(native): ask before a quit or an update stops a working Native session"
```

---

### Task 9: Docs, release notes and version

**Files:**
- Modify: `README.md` (What it does; the macOS, Windows and Linux sections; "How it works" Replies paragraph at `:536-556`), `docs/DEVELOPING.md` (the `native` module in one paragraph)
- Modify: `RELEASE_NOTES.md`, version files through `scripts/set-version.sh`

- [ ] **Step 1: Check the whole feature by hand** (`pnpm tauri dev`, Settings › Sessions › New sessions open in › Native)

1. A new session opens no external app. Its card appears.
2. The viewer in the terminal look shows the live screen. Arrows, Esc, Tab, Ctrl+C, paste and `/config` all work.
3. Close and reopen the viewer: the same screen, and scrolling up shows the earlier output.
4. From the chat look, a reply, Stop, `/compact` and an answer to a question all reach the session.
5. Quit with the session Idle, start Maya again: the session is offered, and resuming it opens it as Native.

- [ ] **Step 2: Update the docs**

- README "What it does": a bullet saying sessions can run inside Maya (Settings › Sessions › New sessions open in › Native). They then have no window, the session viewer is their live terminal, and they stop when Maya quits.
- README platform sections: mention Native as the choice besides Terminal.app and iTerm2, Windows Terminal, and tmux. On Linux, a Native session takes typed keys without tmux.

- [ ] **Step 3: Bump and write the notes**

Run: `git fetch origin main && git show origin/main:package.json | grep '"version"'`, then `sh scripts/set-version.sh <next minor>`.
Add at the top of `RELEASE_NOTES.md`:
```markdown
# <next minor>

## New

- Sessions can now run inside Maya: choose Native in Settings › Sessions › New sessions open in. A Native session opens no terminal app; its session window is its terminal, live, and takes every key.
- Quitting or updating Maya asks first when a Native session is working, since Native sessions stop with Maya. They are offered back the next time Maya starts.
```

- [ ] **Step 4: Run every test**

Run: `cargo test --workspace && pnpm test && pnpm build`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add README.md docs/DEVELOPING.md
git commit -m "docs: Native sessions"
git add RELEASE_NOTES.md package.json src-tauri/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): <next minor>"
```
