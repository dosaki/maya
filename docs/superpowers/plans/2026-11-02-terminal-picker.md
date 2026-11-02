# Terminal Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** New Session and Resume get a Terminal picker that lists the target machine's terminals with its Settings default selected, and the pick opens that one session there.

**Architecture:** A small core module describes a machine's terminals (`TerminalChoices`). Assistants send it in `hello` and again in a new `Up::Terminals` when it changes; the main keeps it per assistant and returns it in `AgentsReply`, which both the desktop dialogs and the phone (via `Request::ListAgents`) already read. Start carries the pick in `LaunchOptions.terminal`, Resume in a new `terminal` field; the opening machine checks it, then opens inside a thread-local override that `crate::new_session_terminal()` reads before config.

**Tech Stack:** Rust (maya_core, src-tauri, mobile), TypeScript (vanilla DOM, vitest).

**Spec:** `docs/superpowers/specs/2026-11-02-terminal-picker-design.md`

## Global Constraints

- Every other open (reviews, Make main, brain/voice starts, card actions) keeps reading Settings; Settings is never written by a pick.
- Compatible both ways: an older assistant sends no `terminals` and gets no pick; an older main never receives `Up::Terminals`; every new field is `#[serde(default)]` and skipped when `None`.
- A pick the opening machine does not have is refused before anything opens, never rerouted.
- Platform lists, in Settings order: macOS `terminal, iterm, native`; Windows `terminal, native`; Linux `terminal, native`. Labels: macOS "Terminal", "iTerm2", "Native"; Windows "Windows Terminal", "Native"; Linux "Terminal (tmux)", "Native".
- Default = saved `new_session_terminal` when it is on the platform's list, else `terminal` (the fallback `src/settings.ts:852-853` uses).
- Gate unix-only tests with `cfg(unix)`; compare paths by component (Windows CI).
- Repaint dialogs with `morph`/`keepEqualChildren` patterns already in use, never `replaceChildren` on a live node (CLAUDE.md pitfall).
- Commit subjects are plain `type: description`; ask before every push.

## Review Focus

1. A late `list_agents` reply for the machine the user just left must not put the old machine's terminals in the picker (Task 7 test).
2. A repaint while the dialog is open (agents arriving late) keeps a pick the user already changed (Task 7 test).
3. An open that fails or panics inside the override must not leave the override set for the next open on that thread (Task 5 test).
4. An assistant that disconnects drops its terminals: the picker hides and a pick sent for it is refused as "older" (Task 3 test).
5. A saved `iterm` on Linux/Windows shows `terminal` as the default, and a pick of `iterm` there is refused (Task 1 test).

---

### Task 1: Terminal choices in core

**Files:**
- Create: `core/src/terminal_choice.rs` (and `pub mod terminal_choice;` in `core/src/lib.rs`)
- Modify: `core/src/launch.rs:150` (`LaunchOptions`)
- Modify: `core/src/net/protocol.rs:208` (`CommandKind::Resume`)

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)] #[serde(rename_all = "camelCase")] pub struct TerminalChoices { pub platform: String, pub choices: Vec<NewSessionTerminal>, pub default: NewSessionTerminal }`
  - `pub fn for_platform(os: &str) -> Vec<NewSessionTerminal>` (`"macos"` → three, anything else → `[Terminal, Native]`)
  - `pub fn describe(os: &str, saved: NewSessionTerminal) -> TerminalChoices`
  - `pub fn local(saved: NewSessionTerminal) -> TerminalChoices` = `describe(std::env::consts::OS, saved)`
  - `pub fn check_pick(pick: Option<NewSessionTerminal>, choices: &TerminalChoices, machine: &str) -> Result<(), String>`
  - `LaunchOptions.terminal: Option<NewSessionTerminal>` (struct already `#[serde(default)]`)
  - `CommandKind::Resume { dir, session, agent, #[serde(default, skip_serializing_if = "Option::is_none")] terminal: Option<NewSessionTerminal> }`

- [ ] **Step 1: Write the failing tests** in `core/src/terminal_choice.rs`:

```rust
#[test]
fn a_mac_offers_three_and_keeps_its_saved_default() {
    let t = describe("macos", NewSessionTerminal::Iterm);
    assert_eq!(t.choices, vec![NewSessionTerminal::Terminal, NewSessionTerminal::Iterm, NewSessionTerminal::Native]);
    assert_eq!(t.default, NewSessionTerminal::Iterm);
    assert_eq!(t.platform, "macos");
}
#[test]
fn iterm_saved_off_a_mac_falls_back_to_terminal() {
    assert_eq!(describe("linux", NewSessionTerminal::Iterm).default, NewSessionTerminal::Terminal);
    assert_eq!(describe("windows", NewSessionTerminal::Native).default, NewSessionTerminal::Native);
}
#[test]
fn a_pick_the_machine_lacks_is_refused() {
    let linux = describe("linux", NewSessionTerminal::Terminal);
    assert_eq!(check_pick(Some(NewSessionTerminal::Iterm), &linux, "laptop").unwrap_err(), "That terminal is not available on laptop.");
    assert!(check_pick(Some(NewSessionTerminal::Native), &linux, "laptop").is_ok());
    assert!(check_pick(None, &linux, "laptop").is_ok());
}
#[test]
fn choices_travel_as_camel_case() {
    let text = serde_json::to_string(&describe("macos", NewSessionTerminal::Native)).unwrap();
    assert_eq!(text, r#"{"platform":"macos","choices":["terminal","iterm","native"],"default":"native"}"#);
}
```

And in `core/src/net/protocol.rs` tests:

```rust
#[test]
fn a_resume_without_a_terminal_reads_as_before_and_one_with_it_round_trips() {
    let old: CommandKind = serde_json::from_str(r#"{"type":"resume","dir":"d","session":"s"}"#).unwrap(); // match the existing tag/shape of CommandKind
    assert!(matches!(old, CommandKind::Resume { terminal: None, .. }));
    let new = CommandKind::Resume { dir: "d".into(), session: "s".into(), agent: Default::default(), terminal: Some(NewSessionTerminal::Native) };
    let back: CommandKind = serde_json::from_str(&serde_json::to_string(&new).unwrap()).unwrap();
    assert_eq!(back, new);
}
```

(Copy the tag layout from the existing `CommandKind` round-trip tests in that file; derive `PartialEq` if it is missing.)

- [ ] **Step 2: Run** `cargo test -p maya-core terminal_choice resume_without_a_terminal` — Expected: FAIL to compile (module/field missing).
- [ ] **Step 3: Implement** the module, the `LaunchOptions.terminal` field and the `Resume.terminal` field. `check_pick` message is exactly `"That terminal is not available on {machine}."`. Fix every `CommandKind::Resume { .. }` literal the compiler flags (`mobile/src/hub.rs:233` gets `terminal: None`).
- [ ] **Step 4: Run** `cargo test -p maya-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: describe a machine's terminals and carry a pick in start and resume`

---

### Task 2: Protocol messages and capability

**Files:**
- Modify: `core/src/net/protocol.rs` (`Up::Hello`, new `Up::Terminals`, new cap const)

**Interfaces:**
- Consumes: `TerminalChoices` (Task 1)
- Produces:
  - `pub const CAP_PICK_TERMINAL: &str = "pick-terminal";` — from an assistant in `hello`: it honours a pick in Start and Resume; from a main in `welcome.caps`: it reads `Up::Terminals`.
  - `Up::Hello { .., #[serde(default, skip_serializing_if = "Option::is_none")] terminals: Option<TerminalChoices> }`
  - `Up::Terminals { terminals: TerminalChoices }`

- [ ] **Step 1: Write the failing tests** (in the protocol tests module):
  - `a_hello_from_an_older_maya_has_no_terminals`: decode the JSON of an existing `Up::Hello` test fixture without `terminals` → `terminals == None`.
  - `a_hello_carries_the_terminals`: round-trip a `Hello` with `Some(describe("linux", Terminal))`, assert equal and that the text contains `"terminals":{"platform":"linux"`.
  - `a_terminals_update_round_trips`: round-trip `Up::Terminals`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::protocol` — Expected: FAIL to compile.
- [ ] **Step 3: Implement**; update every `Up::Hello { .. }` literal (`client.rs:634`, `server.rs:1004` destructure, `server.rs:2026` test helper) with `terminals: None` where no value is known yet.
- [ ] **Step 4: Run** `cargo test -p maya-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: let a Maya announce its terminals in hello and after a change`

---

### Task 3: The main keeps each assistant's terminals

**Files:**
- Modify: `core/src/net/server.rs` (`Peer`, hello handling ~1004-1061, `handle` ~1297/1389, `sync_boards` ~362-376)
- Modify: `core/src/net/merge.rs` (`RemoteBoard`, new accessor)

**Interfaces:**
- Consumes: `CAP_PICK_TERMINAL`, `Up::Terminals`, `Up::Hello.terminals` (Task 2)
- Produces:
  - `RemoteBoard.terminal_choices: Option<TerminalChoices>` — `Some` only while connected and the peer announced `CAP_PICK_TERMINAL` and sent choices. (Not `terminals`: that name is taken by the live-terminal flag.)
  - `pub fn terminal_choices_of(remotes: &[RemoteBoard], machine: &str) -> Option<TerminalChoices>` in `merge.rs`

- [ ] **Step 1: Write the failing tests** in `server.rs` tests, using `pair_with_caps`/`connect_as` (extend `hello_up` to take terminals, or add `hello_with_terminals`):
  - `an_assistant_s_terminals_reach_its_board`: connect with `CAP_PICK_TERMINAL` and `Some(describe("linux", Native))`, send a board, assert `terminal_choices_of(&handle.boards(), "<label>") == Some(..)`.
  - `a_terminals_update_replaces_them`: then send `Up::Terminals` with default `Terminal`; assert the board's default is `Terminal` after the next sync.
  - `an_assistant_without_the_cap_has_none`: same hello without the cap → `None`.
  - `a_disconnected_assistant_has_none` (Review Focus 4): drop the connection; once the board is marked disconnected, `terminal_choices_of` → `None`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::server::tests::an_assistant_s_terminals` (and the others) — Expected: FAIL.
- [ ] **Step 3: Implement**: `Peer.terminals: Option<TerminalChoices>` filled from hello; `Up::Terminals` sets it (current connection only, like `Board`) then `sync_boards()`; `sync_boards` sets `terminal_choices` when `b.connected && can(CAP_PICK_TERMINAL)`. Add the field (as `None`) to every `RemoteBoard` literal the compiler flags.
- [ ] **Step 4: Run** `cargo test -p maya-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: keep each assistant's terminals on the main`

---

### Task 4: The assistant sends its terminals

**Files:**
- Modify: `core/src/net/client.rs` (`Executor`, `handshake` ~631, `run` ~619, `pump` ~659-702)

**Interfaces:**
- Consumes: Tasks 1-2
- Produces: `Executor::terminals(&self) -> Option<TerminalChoices> { None }` (default method)

- [ ] **Step 1: Write the failing tests** in `client.rs` tests, with a test executor whose `terminals()` reads from a `Mutex<Option<TerminalChoices>>`:
  - `hello_carries_the_executor_s_terminals`: the server side of the test reads the hello and asserts `terminals == Some(..)`.
  - `a_change_is_sent_to_a_main_that_reads_it`: welcome with `caps: [CAP_PICK_TERMINAL]`; change the mutex and set `board_requested`; assert an `Up::Terminals` arrives with the new default, and only once for one change.
  - `an_older_main_is_not_sent_updates`: welcome without the cap; change it; assert no `Up::Terminals` before the next board.
- [ ] **Step 2: Run** `cargo test -p maya-core net::client` — Expected: FAIL.
- [ ] **Step 3: Implement**: `handshake` takes `exec` (or the terminals value) and puts `exec.terminals()` in `hello`; `pump` takes `reads_terminals: bool` (from `caps.contains(CAP_PICK_TERMINAL)`) and keeps `last_terminals` (seeded with what hello sent); on each board turn, when `reads_terminals` and `exec.terminals() != last_terminals` and it is `Some`, send `Up::Terminals` and update `last_terminals`. A Settings save already triggers a board turn (`refresh_and_emit` → `push_board`), so no new trigger is needed.
- [ ] **Step 4: Run** `cargo test -p maya-core` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: send the main this Maya's terminals on connect and on change`

---

### Task 5: Opening with a pick on this machine

**Files:**
- Modify: `src-tauri/src/native/route.rs` (override)
- Modify: `src-tauri/src/lib.rs:46-48` (`new_session_terminal`), `:1356` (`resume_session`), `:1398` (`start_session`)

**Interfaces:**
- Consumes: `terminal_choice::{local, check_pick}`, `LaunchOptions.terminal`
- Produces:
  - `pub fn with_terminal<T>(pick: Option<NewSessionTerminal>, f: impl FnOnce() -> T) -> T` — sets a thread-local for the call, restores the previous value on return *and on unwind* (drop guard).
  - `pub fn picked() -> Option<NewSessionTerminal>`
  - `crate::new_session_terminal()` returns `picked()` when set, else config.
  - `pub(crate) fn open_with_pick<T>(pick: Option<NewSessionTerminal>, f: impl FnOnce() -> Result<T, String>) -> Result<T, String>` in `lib.rs`: `check_pick(pick, &terminal_choice::local(<config>), "this machine")?` then `with_terminal(pick, f)`.
  - Tauri `resume_session(.., terminal: Option<NewSessionTerminal>)`.

- [ ] **Step 1: Write the failing tests** in `route.rs` tests (`cfg(all(test, unix))` already):
  - `a_pick_counts_only_inside_its_call`: `assert_eq!(with_terminal(Some(Native), picked), Some(Native)); assert_eq!(picked(), None);`
  - `a_pick_is_cleared_when_the_open_panics` (Review Focus 3): `let _ = std::panic::catch_unwind(|| with_terminal(Some(Native), || panic!("x"))); assert_eq!(picked(), None);`
  - `a_pick_routes_the_open`: build a `WithNative` whose `choice` is `|| picked().unwrap_or(NewSessionTerminal::Terminal)` (mirroring the real fn) with the existing stub inner/host from this test module; `with_terminal(Some(Native), || r.open(..))` reaches the host, and the same open without a pick reaches the inner terminal.
  - `another_thread_does_not_see_the_pick`: inside `with_terminal(Some(Native), ..)` spawn a thread that returns `picked()`; assert `None`.
- [ ] **Step 2: Run** `cargo test -p maya route::tests::a_pick` — Expected: FAIL.
- [ ] **Step 3: Implement** the override, `new_session_terminal()`, `open_with_pick`, and wrap the local paths: `start_session` (`opening(|| open_with_pick(options.terminal, || actions::start_session(..)))`) and `resume_session` (same with the new `terminal` arg). Remote paths are Task 6.
- [ ] **Step 4: Run** `cargo test -p maya` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: open a started or resumed session in the picked terminal`

---

### Task 6: Routing the pick over the network, and the phone

**Files:**
- Modify: `core/src/net/routing.rs` (`AgentsReply`, `agents_reply`, new `check_remote_terminal`)
- Modify: `src-tauri/src/lib.rs` (`list_agents` ~1379, remote branches of `start_session`/`resume_session`, `main_caps` ~242)
- Modify: `src-tauri/src/net_app.rs` (`caps` ~178, `Executor::terminals`, `serve_request` ~467-486, `execute` ~542-543)
- Modify: `mobile/src/commands.rs` (`list_agents` ~174, `resume_session` ~190)

**Interfaces:**
- Consumes: Tasks 1-5
- Produces:
  - `AgentsReply.terminals: Option<TerminalChoices>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`)
  - `agents_reply(local, remote, remote_worktrees, terminals: Option<TerminalChoices>, local_list)` — the caller passes `Some(local(config))` locally, `terminal_choices_of(..)` for an assistant.
  - `pub fn check_remote_terminal(pick: Option<NewSessionTerminal>, remote: &Option<TerminalChoices>, machine: &str) -> Result<(), String>` — `Some` pick with `None` remote → `"{machine} runs an older Maya that cannot pick the terminal."`; otherwise `check_pick`.
  - Phone `resume_session(.., terminal: Option<NewSessionTerminal>)`.

- [ ] **Step 1: Write the failing tests** in `routing.rs`:
  - `check_remote_terminal_refuses_an_older_maya_and_a_missing_terminal`: the older message exactly as above; `Some(Iterm)` against a linux `describe` → `"That terminal is not available on laptop."`; `None` pick against `None` → ok.
  - `an_agents_reply_from_an_older_main_decodes_without_terminals`: `{"agents":[],"names":true}` → `terminals == None`.
  - `a_local_agents_reply_carries_this_machine_s_terminals`: `agents_reply(true, None, false, Some(t.clone()), Vec::new).terminals == Some(t)`.
- [ ] **Step 2: Run** `cargo test -p maya-core net::routing` — Expected: FAIL.
- [ ] **Step 3: Implement** routing; then wire:
  - `lib.rs list_agents` and `net_app serve_request ListAgents` pass the terminals (local: from the store's config; remote: `terminal_choices_of`).
  - Remote `start_session`/`resume_session` in `lib.rs` and `Request::Command` Start/Resume in `serve_request` call `check_remote_terminal` next to `check_remote_worktree`; `CommandKind::Resume` carries `terminal`.
  - `net_app::execute` Start/Resume run inside `open_with_pick(..)` (this covers an assistant opening for the main, and the main opening for the phone).
  - `net_app` client `caps()` adds `CAP_PICK_TERMINAL`; `Executor::terminals()` returns `Some(local(config.new_session_terminal))`; desktop `main_caps` adds `CAP_PICK_TERMINAL`.
  - `mobile/src/commands.rs`: fallback `agents_reply(false, None, false, None, Vec::new)`; `resume_session` takes and sends `terminal`.
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: carry the terminal pick to assistants and from the phone`

---

### Task 7: The picker in New Session and Resume

**Files:**
- Create: `src/terminalpick.ts`
- Modify: `src/types.ts` (add `TerminalChoices`), `src/settings.ts:586-590` (labels by platform)
- Modify: `src/newsession.ts` (model, `renderNewSession`, `capture`, `readChoices`, `loadAgents`, `chooseMachine`, `StartChoices`)
- Modify: `src/resume.ts` (model, `renderResume`, `loadAgents`, `chooseMachine`, `resume`)
- Test: `src/terminalpick.test.ts`, `src/newsession.test.ts`, `src/newsession-flow.test.ts`, `src/resume.test.ts`

**Interfaces:**
- Consumes: `AgentsReply.terminals` as JSON `{ platform, choices, default }`; `start_session` options `terminal`; `resume_session` arg `terminal`.
- Produces:
  - `src/types.ts`: `export interface TerminalChoices { platform: string; choices: NewSessionTerminal[]; default: NewSessionTerminal }`
  - `src/settings.ts`: `export function terminalLabel(value: NewSessionTerminal, platform: string): string`; `terminalChoices()` keeps its signature and uses it with this machine's platform.
  - `src/terminalpick.ts`: `export function renderTerminalPick(t: TerminalChoices, value: NewSessionTerminal | null): HTMLLabelElement` — label "Terminal", `select[name=terminal]`, one option per choice (no blank "Default" entry, so `renderChoice` does not fit), selected = `value` if listed, else `t.default`.
  - Models: `terminals: TerminalChoices | null; terminal: NewSessionTerminal | null` in `NewSessionModel` and `ResumeModel`; `StartChoices` gains `terminal?: NewSessionTerminal`.

- [ ] **Step 1: Write the failing tests**:
  - `terminalpick.test.ts`: `renderTerminalPick({platform:"linux",choices:["terminal","native"],default:"native"}, null)` → options text `["Terminal (tmux)", "Native"]`, value `"native"`; with value `"iterm"` (not listed) → `"native"`; macOS labels `["Terminal","iTerm2","Native"]`.
  - `newsession.test.ts`: with `terminals` set the form has `select[name=terminal]` between Agent and Directory; with `terminals: null` it has none; Start sends `options.terminal` equal to the select's value (default left alone → the default).
  - `newsession-flow.test.ts` (Review Focus 1): choose machine A, then B before A's `list_agents` resolves; resolve A then B; the picker shows B's choices. (Review Focus 2): change the pick, then resolve a late agents reply for the same machine; the pick is kept.
  - `resume.test.ts`: the picker shows when `terminals` is set; resuming invokes `resume_session` with `terminal` = the select's value; no picker and no `terminal` arg when `null`.
- [ ] **Step 2: Run** `pnpm vitest run src/terminalpick.test.ts src/newsession.test.ts src/newsession-flow.test.ts src/resume.test.ts` — Expected: FAIL.
- [ ] **Step 3: Implement**: `loadAgents` sets `terminals = reply.terminals ?? null` and keeps `terminal` only if it is in the new choices, else the default; `chooseMachine` resets both to `null`; `capture` reads `select[name=terminal]` into `model.terminal`; `readChoices`/`resume()` send it when `terminals` is set. Phone: the same modules run in `src/mobile/main.ts`, so nothing extra.
- [ ] **Step 4: Run** `pnpm vitest run` and `pnpm tsc --noEmit` (or the project's typecheck script in `package.json`) — Expected: PASS.
- [ ] **Step 5: Commit** `feat: pick the terminal in New Session and Resume`

---

### Task 8: Release

**Files:**
- Modify: version files via `sh scripts/set-version.sh 0.47.0` (bump from `main`'s version at the time; 0.46.0 today), `RELEASE_NOTES.md`

- [ ] **Step 1:** Check `main`'s version (`git show origin/main:package.json | grep '"version"'`); run `sh scripts/set-version.sh <next minor>`; confirm `Cargo.lock` changed (memory: set-version can leave it; cargo may be off sh's PATH).
- [ ] **Step 2:** Add at the top of `RELEASE_NOTES.md`:

```markdown
# 0.47.0

## New

- New Session and Resume have a Terminal picker. It starts on the terminal Settings picks, and changing it opens only that session somewhere else: Terminal, iTerm2 or Native on a Mac, Windows Terminal or Native on Windows, Terminal (tmux) or Native on Linux.
- On another machine or from the phone, the picker lists that machine's terminals and its own default. An assistant tells the main when its setting changes.
```

- [ ] **Step 3:** Run the full check from `docs/DEVELOPING.md` (Rust workspace tests, vitest, typecheck, the version-agreement check) — Expected: all pass.
- [ ] **Step 4: Commit** `chore(release): 0.47.0`
