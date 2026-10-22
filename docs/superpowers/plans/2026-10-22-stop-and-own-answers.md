# Stop a session, and answer choices your own way: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Stop button (= Esc) for any session with a turn to stop, and a CLI-style menu in place of the reply box while a session asks, with "Type something", "Chat about this" and Claude's permission/plan options, on the Mac, the laptop and the phone.

**Architecture:** Core gains a raw-key path on the `Terminal` seam (no Enter), a `stop_session` action, and a general `Pick` answer (`Option`, `Options`, `Own`, `Chat`) that turns into typed key steps per question. Questions carry the rows for own words and chat; Claude's permission and plan lists come from a new `claude_menus` module filled from live probes. Two new protocol commands (`Stop`, `Pick`) carry them across devices. The frontend gets a `menu.ts` that replaces the composer while asking.

**Tech Stack:** Rust (core, Tauri 2 app, phone app), TypeScript (vitest), AppleScript, tmux, Win32 console input.

**Spec:** `docs/superpowers/specs/2026-10-22-stop-and-own-answers-design.md`

## Global Constraints

- Stop sends Esc **and nothing else**; never followed by Enter.
- Every new answer path is checked against the ask id (`stateSince`) and `OPEN_DELAY_MS` = 1000 ms, as `answer::check` does today.
- Esc in the session view keeps closing it; Stop is a button only.
- Unproven pieces ship switched off: a prompt with no verified option list keeps "answer it in the terminal"; an agent with no verified stop key has Stop hidden (`Capabilities.stop = None`).
- Tests pass on macOS, Linux and Windows CI; unix-only tests gated with `cfg(unix)`; paths compared by component.
- One version bump in the PR: `0.28.0` → `0.29.0` (or the next minor from `main` at merge time) via `sh scripts/set-version.sh`, committed alone as `chore(release): <version>`; check `Cargo.lock` changed.
- Never commit to or push `main`; ask the user before each push.
- Copy: "Type something", "Chat about this", "Stop"; errors are full sentences, like the existing ones.

## Review Focus

1. **The ask moves on between render and click** (answered in the terminal, a new question): every `Pick` kind and the menu must be refused with "The question has changed; look again." → tests in Task 5.
2. **Own words containing newlines, tabs, escape sequences or only control characters**: an embedded ESC must never reach the terminal as a key, newlines become spaces, an empty result is refused → tests in Task 5.
3. **A second Stop or a double click**: two Escs open Claude's rewind menu, so Stop must be disabled from the click until the card's `stateSince` changes → test in Task 7.
4. **Stop on a card that is no longer stoppable** (Idle, Completed, shell-only Working, prose ask, brain card): refused with "There is nothing to stop in this session." and never types → tests in Task 3.
5. **A remote card on an assistant that runs an older Maya**: `Stop`/`Pick` decode fails there; the user must see an error, not silence → test in Task 6.

---

### Task 0: Live probes (the user runs them; work continues in parallel)

**Files:**
- Modify: `docs/superpowers/specs/2026-10-22-stop-and-own-answers-design.md` (append `## Probe results`)

The user is in meetings and answers by text. Send all probes in one message at the start; record each answer under `## Probe results` as it arrives and commit (`docs: probe results for …`). Tasks that depend on a probe say so; do the rest first.

- [ ] **Step 1: Send the probes to the user**, each with exact steps and what to report:
  - **P1 Claude question menu** (Claude Code version from `claude --version`): in a session ask Claude to "use AskUserQuestion with two options". Report the rows below the options, in order (expected "Type something.", divider, "Chat about this"), and whether pressing ↓ to the "Type something" row then typing letters puts them in that row's box, and whether one Enter then sends it.
  - **P2 Claude multi-select**: same, with `multiSelect: true`. Report which key ticks a row (Space or Enter), and how the answers are submitted (a "Submit" row? a review tab?).
  - **P3 Claude permission and plan prompts**: trigger a Bash permission prompt and an Edit permission prompt, then a plan approval (plan mode, ask for a plan). Report every row, in order, for each; for the "No, and tell Claude…" / "Tell Claude what to change" row, whether typing on it puts text in a box and Enter sends it. Also run `tail -3 ~/.claude/maya/events.jsonl` right after each permission prompt appears and paste the `PermissionRequest` line (Maya needs its `permission_suggestions`).
  - **P4 Interrupt keys**: in Grok, Kiro and Antigravity, start a long task and press Esc; if it does not stop, press Ctrl-C once. Report which key stops the turn without ending the session.
  - **P5 macOS Esc**: run `osascript -e 'tell application "Terminal" to do script (character id 27) in front tab of front window'` while Claude is mid-turn in that tab. Report whether the turn stops, and what is left in Claude's input.
  - **P6 After Esc**: in Claude and Codex, press Esc on a running turn, on an AskUserQuestion, and on a permission prompt; report how Maya's card looks a few seconds later (Working, Idle, still Awaiting).
  - **P7 OpenCode reject with words**: whether rejecting a permission in OpenCode's TUI offers a message box.

- [ ] **Step 2: Record each answer under `## Probe results` and commit** as it arrives.

### Task 1: Raw keys on the `Terminal` seam

**Files:**
- Modify: `core/src/terminal.rs` (trait + `FakeTerminal`), `core/src/terminal_tmux.rs`, `core/src/win_console.rs`, `src-tauri/src/terminal_app.rs`, `src-tauri/src/terminal_linux.rs`, `src-tauri/src/terminal_win.rs`, `core/src/answer.rs`

**Interfaces:**
- Produces: `Terminal::press_keys(&self, tty: &str, keys: &str) -> Result<(), String>` (keys sent exactly, no Enter); `answer::ESC: &str = "\x1b"`; `Call::Keys { tty: String, keys: String }` in `FakeTerminal`; `terminal_tmux::press_keys_args(target: &str, keys: &str) -> Vec<String>`; `win_console::Key::Escape`; `terminal_app::applescript_press_escape(tty: &str) -> String`.

- [ ] **Step 1: Write the failing tests**
  - `terminal.rs`: `fake_terminal_records_raw_keys` — `t.press_keys("/dev/pts/3", answer::ESC)` records `Call::Keys { tty: "/dev/pts/3", keys: "\x1b" }` and no `Call::Type`.
  - `terminal_tmux.rs`: `raw_keys_are_sent_as_hex_without_enter` — `press_keys_args("maya-1:0.0", "\x1b") == ["send-keys","-t","maya-1:0.0","-H","1b"]`.
  - `win_console.rs`: `a_lone_escape_is_the_escape_key` — `keys_for("\x1b") == vec![Key::Escape]`, and `keys_for("\x1b[B")` is still `[Key::Down]`.
  - `terminal_app.rs`: `escape_script_targets_the_tab_and_presses_key_code_53` — the script contains `tty of t is "/dev/ttys004"`, `key code 53`, and no `do script`.
- [ ] **Step 2: Run** `cargo test -p maya-core press_keys raw_keys a_lone_escape` and `cargo test -p maya escape_script` → FAIL (not defined).
- [ ] **Step 3: Implement.** Trait method without a default (every impl must decide). tmux: one `send-keys -H` with each byte as two hex digits. Windows: `Key::Escape` (VK_ESCAPE, unit 0x1b) in `keys_for` for an ESC not followed by `[`; `press_keys` calls `type_keys` without the trailing Enter `type_line` adds. macOS: until P5 says otherwise, the script selects the tab and its window, activates Terminal, sends `key code 53` through System Events, and `press_keys` maps osascript's "not allowed assistive access" error (codes -1719 / -25211) to `"Maya needs Accessibility permission to press Esc in Terminal (System Settings → Privacy & Security → Accessibility)."`; any keys other than `answer::ESC` → `Err("Only Esc can be pressed in Terminal.app.")`. If P5 shows `do script (character id 27)` stops the turn and leaves nothing behind, use that instead and drop the Accessibility path.
- [ ] **Step 4: Run** the same tests → PASS; `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat: press raw keys such as Esc in a session's terminal`.

### Task 2: Know when a turn can be stopped

**Files:**
- Modify: `core/src/model.rs` (Card), `core/src/state.rs` (`derive`), `core/src/launch.rs` (`Capabilities`, `capabilities`, new `can_stop`), `src/types.ts`, `src/harness.ts`

**Interfaces:**
- Produces: `Card.shell: bool` (`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`, true when Working only because a background shell outlived the turn); `Capabilities.stop: Option<&'static str>` (the key, or `"server"` for OpenCode); `launch::can_stop(card: &Card) -> bool`; TS `Card.shell?: boolean`, `Capabilities.stop: boolean`, `canStop(card: Card): boolean` in `src/harness.ts`.
- Table: Claude Code `Some(answer::ESC)`, Codex `Some(answer::ESC)`, OpenCode `Some("server")`, Grok/Kiro/Antigravity `None` until P4 (then the probed key), Other `None`.

- [ ] **Step 1: Write the failing tests**
  - `state.rs`: `a_shell_left_running_marks_the_card` — registry status `"shell"`, no ask → `state == Working && card.shell`; status `"busy"` → `!card.shell`.
  - `launch.rs`: `can_stop_only_with_a_turn_to_stop` — true for Claude Working (not shell), Awaiting Question/Permission/Plan; false for Working+shell, Awaiting Text, Idle, Completed, `brain: true`, Grok Working.
  - `src/harness.test.ts`: `canStop mirrors the Rust table` with the same cases.
- [ ] **Step 2: Run** `cargo test -p maya-core can_stop a_shell_left` and `npx vitest run src/harness.test.ts` → FAIL.
- [ ] **Step 3: Implement.** `derive` sets `shell` in the `shell_running` branch only. Extend `capabilities_follow_the_table` with the `stop` column.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat: tell a stoppable turn from a background shell`.

### Task 3: `stop_session` in core

**Files:**
- Modify: `core/src/actions.rs`, `core/src/opencode.rs`, `core/src/state.rs`, `core/src/store.rs`

**Interfaces:**
- Consumes: `Terminal::press_keys`, `answer::ESC`, `launch::can_stop`.
- Produces: `actions::stop_session(l: &Local, session_id: &str) -> Result<(), String>`; `opencode::interrupt(client: &Client, session_id: &str) -> Result<(), String>` (the `post_empty("/api/session/{id}/interrupt")` the brain driver uses; make `opencode_drv.rs` call it); `Store::mark_interrupted(&mut self, session_id: &str, now_ms: u64)`, which appends `{"session_id", "hook_event_name": "MayaInterrupt", "received_at"}` to `maya/events.jsonl` through `hook_install::append_record_named`; `derive` treats a main-agent `"MayaInterrupt"` like the end of a prompt: `awaiting = None`.

- [ ] **Step 1: Write the failing tests** (in `actions.rs` tests, with `FakeTerminal` and the existing store fixtures)
  - `stop_presses_esc_and_nothing_else` — Claude Working card → calls are exactly `[Call::Keys { tty, keys: "\x1b" }]`.
  - `stop_clears_the_pending_ask` — Claude Awaiting Permission → after `stop_session`, `card_for` has `awaiting == None` and `check_free` passes.
  - `stop_refuses_when_there_is_nothing_to_stop` — Idle, Completed, shell-only Working, prose ask → `Err("There is nothing to stop in this session.")`, no calls.
  - `stop_refuses_an_agent_without_a_stop_key` — Grok Working → `Err("Grok has no Stop yet.")` (via `launch::label`).
  - `state.rs`: `a_maya_interrupt_clears_the_ask` — events `[PermissionRequest, MayaInterrupt]` → no awaiting.
- [ ] **Step 2: Run** `cargo test -p maya-core stop_ a_maya_interrupt` → FAIL.
- [ ] **Step 3: Implement.** OpenCode: `server_card(l, id, true)`, `can_stop`, `opencode::interrupt`. Others: card under the store lock, `can_stop`, tty via `session_tty`, then `press_keys` while holding `TYPING`; on success for Claude call `mark_interrupted`. If P6 shows a Codex card stays Awaiting after Esc, write the same marker to `codex-events.jsonl` and teach `codex_hooks::apply` to clear on it. Log one line through `crate::log::line("stop", …)` like `send_reply` does.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat: stop a session's turn from Maya`.

### Task 4: Question rows for own words and chat; Claude's permission and plan menus

**Depends on:** P1, P3 (write the tests from the recorded rows).

**Files:**
- Modify: `core/src/model.rs` (`Question`), `core/src/state.rs` (`awaiting_for_tool`), `core/src/opencode.rs` (questions it builds), `src/types.ts`
- Create: `core/src/claude_menus.rs` (+ `mod` in `core/src/lib.rs`)

**Interfaces:**
- Produces: `model::OwnRow { label: String, row: usize }`; `Question.own: Option<OwnRow>` and `Question.chat_row: Option<usize>` (both `#[serde(default, skip_serializing_if = "Option::is_none")]`); `claude_menus::ask_rows(q: &mut Question)` (sets `own = Some(OwnRow { label: "Type something", row: options.len() })`, `chat_row = Some(options.len() + 1)` for a single-select question, per P1); `claude_menus::permission_question(e: &HookEvent) -> Option<Question>`; `claude_menus::plan_question(e: &HookEvent) -> Option<Question>`. `None` means "not verified": the card keeps an empty `questions` and the UI keeps "answer it in the terminal".

- [ ] **Step 1: Write the failing tests** in `claude_menus.rs`
  - `ask_rows_follow_the_cli` — two options → own row 2 labelled "Type something", chat row 3.
  - `bash_permission_rows` / `edit_permission_rows` — from the P3 `PermissionRequest` lines (paste them as fixtures) → option labels exactly as P3 recorded, `own` on the "No, and tell Claude what to do differently" row.
  - `plan_rows` — an `ExitPlanMode` `PreToolUse` fixture → P3's plan labels, `own` on its feedback row if P3 found one.
  - `unknown_shapes_give_no_menu` — a `PermissionRequest` without the fields P3 relies on → `None`.
  - `state.rs`: `a_permission_request_carries_its_menu` — the card's `awaiting.kind == Permission` and `questions.len() == 1`.
- [ ] **Step 2: Run** `cargo test -p maya-core claude_menus a_permission_request_carries` → FAIL.
- [ ] **Step 3: Implement** from the probe results; `awaiting_for_tool` fills `questions` from these, and calls `ask_rows` on each AskUserQuestion question. OpenCode forms: set `own` on a field that takes free text (P7 decides whether a permission's Reject gets one).
- [ ] **Step 4: Run** → PASS, plus `cargo test --workspace` (serde round-trip tests of `Card` still pass).
- [ ] **Step 5: Commit** `feat: know the CLI's rows for own words, chat, permissions and plans`.

### Task 5: `Pick` answers in core

**Depends on:** P1, P2 (key steps), Task 4.

**Files:**
- Modify: `core/src/answer.rs`, `core/src/actions.rs`

**Interfaces:**
- Consumes: `Question.own`, `Question.chat_row`, `Terminal::type_line`.
- Produces:
  - `answer::Pick` — `#[serde(tag = "kind", rename_all = "snake_case")] enum Pick { Option { index: usize }, Options { indexes: Vec<usize> }, Own { text: String }, Chat }`.
  - `answer::check_pick(card: &Card, ask_id: u64, question: usize, pick: &Pick, now_ms: u64) -> Result<(), String>` — accepts Awaiting with non-empty `questions` (any kind but Text); `answer::check` becomes `check_pick(.., &Pick::Option { index })`.
  - `answer::Step` — `enum Step { Line(String), Pause(u64) }` (`Line` is typed with Enter).
  - `answer::steps_for_pick(q: &Question, question: usize, count: usize, pick: &Pick) -> Vec<Step>`.
  - `answer::own_line(text: &str) -> Result<String, String>` — `typeable`, newlines and runs of whitespace to one space, trimmed; empty → `Err("Type an answer first.")`; over `TYPED_MAX_CHARS` → the existing "too long" wording.
  - `actions::answer_pick(l: &Local, session_id: &str, ask_id: u64, question: usize, pick: &Pick) -> Result<(), String>`; `answer_question` calls it with `Pick::Option`.
- Errors: `Options` on a single-select or `Option` on a multi-select → "That question takes one answer." / "That question takes several answers."; empty or out-of-range or repeated `indexes` → "That question has no such option."; `Own` without `own` → "That question takes no answer of your own."; `Chat` without `chat_row` → "That question cannot be set aside."

- [ ] **Step 1: Write the failing tests** in `answer.rs`
  - `option_steps_are_unchanged` — `Option { index: 2 }` → `[Line("\x1b[B\x1b[B")]`.
  - `own_words_go_on_the_own_row` — two options, `Own { text: "use redis" }` → `[Line("\x1b[B\x1b[Buse redis")]`.
  - `chat_goes_to_the_chat_row` — two options → `[Line("\x1b[B\x1b[B\x1b[B")]`.
  - `multi_select_steps_follow_p2` — the sequence P2 recorded, for indexes `[0, 2]`.
  - `the_last_of_several_questions_still_submits` — `needs_submit` adds `[Pause(SUBMIT_DELAY_MS), Line("")]`.
  - `own_line_cleans_text` — `"a\nb\tc\x1b[31m"` → `"a b c"`; `"\x1b\x07"` → `Err("Type an answer first.")`.
  - `check_pick_refuses_a_changed_ask_for_every_kind` — each `Pick` with a wrong `ask_id` → `"The question has changed; look again."`.
  - `check_pick_respects_the_open_delay` and the error cases listed above.
  - `actions.rs`: `answer_pick_types_the_steps` — FakeTerminal records the lines in order.
- [ ] **Step 2: Run** `cargo test -p maya-core pick own_ chat_ multi_select` → FAIL.
- [ ] **Step 3: Implement.** OpenCode in `answer_pick`: `Option` as today; `Own` → the form's free-text field (`reply_form`) or, if P7 allows, reject with the text; anything else → `Err("OpenCode takes that answer in its own window.")`.
- [ ] **Step 4: Run** → PASS; `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat: answer a question with your own words or set it aside`.

### Task 6: Across devices

**Files:**
- Modify: `core/src/net/protocol.rs`, `core/src/net/server.rs` (`kind_name`), `core/src/net/routing.rs` (`target_of`), `src-tauri/src/net_app.rs` (`execute`), `src-tauri/src/lib.rs` (commands + `generate_handler!` at ~1347), `mobile/src/commands.rs`, `mobile/src/lib.rs` (`generate_handler!`)

**Interfaces:**
- Consumes: `actions::stop_session`, `actions::answer_pick`, `answer::Pick`.
- Produces: `CommandKind::Stop { session: String }`, `CommandKind::Pick { session: String, ask_id: u64, question: usize, pick: answer::Pick }`; `kind_name` → `"stop"`, `"pick"`; Tauri commands `stop_session(session_id: String) -> Result<(), String>` and `answer_pick(session_id: String, ask_id: u64, question_index: usize, pick: Pick) -> Result<(), String>` on desktop (with `remote_machine_of` → `route_done`) and phone (`hub.send`). `Answer` stays for older peers.

- [ ] **Step 1: Write the failing tests**
  - `protocol.rs`: `stop_and_pick_round_trip` — encode/decode both; `Pick::Own` keeps its text.
  - `routing.rs`: `stop_and_pick_target_their_session` — `target_of` gives `Target::Session(..)` for both.
  - `net_app.rs`: `an_assistant_error_reaches_the_caller` — an assistant `Result` carrying an error (what an older Maya sends back for a command it cannot decode) comes out of `route_done` as `Err`, following the existing failure test there.
- [ ] **Step 2: Run** `cargo test -p maya-core stop_and_pick` and `cargo test -p maya an_assistant_error` → FAIL.
- [ ] **Step 3: Implement** each listed place, following `Answer`'s wiring exactly.
- [ ] **Step 4: Run** → PASS; `cargo build -p maya-mobile` → builds.
- [ ] **Step 5: Commit** `feat: send Stop and own answers between devices`.

### Task 7: The menu and the Stop button in the UI

**Files:**
- Create: `src/menu.ts`, `src/menu.test.ts`
- Modify: `src/modal.ts` (banner ~540-570, composer ~603-665), `src/options.ts`, `src/card.ts`, `src/actions.ts`, `src/answer.ts`, `src/types.ts`, `src/styles.css`, `src/mobile/mobile.css`, `src/modal.test.ts`

**Interfaces:**
- Consumes: Tauri `stop_session`, `answer_pick`; `canStop`, `OPEN_DELAY_MS`; `Question.own`, `Question.chatRow`.
- Produces:
  - `type Pick = { kind: "option"; index: number } | { kind: "options"; indexes: number[] } | { kind: "own"; text: string } | { kind: "chat" }` in `src/types.ts`.
  - `renderMenu(card: Card, next: number, opts: { enabled: boolean; keyboardHints: boolean }): HTMLElement | null` in `src/menu.ts` — null unless Awaiting with `questions[next]`. Rows: header, question, numbered options with descriptions, the own row with a textarea when `own` is set, a divider, the chat row when `chatRow` is set; multi-select rows are tick boxes plus a "Submit" row. Each row has `data-action="pick"` and `data-row`.
  - `makeAnswerGuard` gains `pick(card: Card, questionIndex: number, pick: Pick, el: HTMLElement): Promise<AnswerOutcome>`; `answer()` delegates to it.
  - `makeStopGuard(send: (sessionId: string) => Promise<void>)` in `src/answer.ts` with `stop(card: Card, button: HTMLButtonElement)`: disables the button and keeps it disabled while `card.stateSince` is unchanged.
  - `CardAction` gains `{ kind: "stop"; sessionId: string }`.

- [ ] **Step 1: Write the failing tests** (`src/menu.test.ts`, `src/modal.test.ts`, `src/answer.test.ts`)
  - `menu shows options, Type something and Chat about this in CLI order`.
  - `menu replaces the composer while asking and the composer returns after` — modal paint with an Awaiting card has no `.modal__input` outside the menu; with a Working card it has one.
  - `number keys and arrows act only once the menu has focus` — keydown "1" on document picks nothing; after focusing the menu, "1" sends `{kind:"option",index:0}`.
  - `Escape still closes the view while the menu is shown`.
  - `Enter in the own box sends {kind:"own"}`; empty box sends nothing.
  - `the menu is disabled inside the open delay`.
  - `Stop shows only when canStop`, sits next to Send, and `a second Stop is ignored until the card changes`.
  - `card shows compact options and Stop`; a multi-select card says "Answer in the session view".
- [ ] **Step 2: Run** `npx vitest run src/menu.test.ts src/modal.test.ts src/answer.test.ts` → FAIL.
- [ ] **Step 3: Implement.** The banner keeps its text and "Open terminal" only; its options go. Text for a menu ask: "This session is asking. Answer below or in its terminal." (remote: "…Answer below."). `renderOptions` shows any ask with questions (not only `kind === "question"`). Keyboard hints hidden on the phone (`keyboardHints: false` from `src/mobile/main.ts`). Style to layout A of the mockup.
- [ ] **Step 4: Run** `npx vitest run` and `npx tsc --noEmit` → PASS.
- [ ] **Step 5: Commit** `feat: answer from a CLI-style menu and stop from the board`.

### Task 8: Voice

**Files:**
- Modify: `core/src/interpreter.rs` (`ACTIONS`, board lines ~22-30, `validate` ~410-431), `src-tauri/src/listener.rs` (`execute_action` ~229-239, `spoken_for` ~1324), `src-tauri/src/net_app.rs` (`revalidate`)

**Interfaces:**
- Consumes: `actions::stop_session`, `actions::answer_pick`, `launch::can_stop`.
- Produces: action `{"kind":"stop","session":"<name>"}` described in `ACTIONS` as "stop what that session is doing, as Esc does"; spoken "Stopping {name}."; a `reply` to an Awaiting card whose first question has `own` runs as `Pick::Own` on question 0; a `reply` to a Permission or Plan card is refused with "Pick one of its numbered options, or answer it in the terminal."

- [ ] **Step 1: Write the failing tests**
  - `interpreter.rs`: `stop_needs_a_stoppable_session` — validate passes for a Working Claude card, fails for Idle.
  - `interpreter.rs`: `permission_options_are_numbered_on_the_board` — a Permission card with a menu lists `[1 Yes, 2 …]`.
  - `listener.rs`: `spoken_stop` → "Stopping hexgrid."; `a_spoken_reply_to_a_question_is_its_own_answer` and `a_spoken_reply_to_a_permission_is_refused` (with the fake terminal / store fixtures already used there).
- [ ] **Step 2: Run** `cargo test -p maya-core stop_needs permission_options` and `cargo test -p maya spoken_stop a_spoken_reply` → FAIL.
- [ ] **Step 3: Implement**; remote cards route through `route_done` as the existing actions do. "Maya, stop" (no session) is unchanged.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat: stop a session and answer in your own words by voice`.

### Task 9: Finish

**Files:**
- Modify: `.gitignore` (add `.superpowers/`), `README.md` (one line each for Stop and the menu where session controls are described), version files via script

- [ ] **Step 1:** Add `.superpowers/` to `.gitignore`; README lines; commit `docs: describe Stop and answering in your own words`.
- [ ] **Step 2:** Turn on what the probes proved (`Capabilities.stop` for P4 agents, menus for P3), and confirm everything unproven is off. List what is off in the PR description.
- [ ] **Step 3:** `cargo test --workspace`, `npx vitest run`, `npx tsc --noEmit` → all PASS (paste the summary lines).
- [ ] **Step 4:** `git fetch origin && git show origin/main:package.json | grep version`; `sh scripts/set-version.sh <next minor>`; check `git diff --stat` includes `Cargo.lock`; commit `chore(release): <version>`.
- [ ] **Step 5:** Ask the user before pushing; then open the PR (`feat: stop a session and answer choices your own way`), listing the hand checks from the spec.
