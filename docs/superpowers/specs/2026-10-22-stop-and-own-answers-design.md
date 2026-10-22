# Stop a session, and answer a question your own way

Status: design, agreed with the user on 2026-10-22. Items marked
**(my call)** were decided by Claude while writing this up and were not
discussed; the user can overturn any of them.

## Intent

Two gaps, on the Mac, the laptop and the phone alike:

1. **There is no way to stop a session from Maya.** The user wants the
   equivalent of pressing Esc in the agent's terminal: the agent abandons
   its current turn, the session stays open, and the user can then tell it
   what to do instead.
2. **When a session offers choices, the user can only pick one of them.**
   The agents' own escape hatches ("Type something", "Chat about this",
   "No, and tell Claude what to do differently") are not shown, and while
   a choice is pending the backend refuses typed text with "waiting for a
   decision; answer it first" (`core/src/answer.rs:76-82`). The user wants
   both: to answer the question in their own words, and to set the
   question aside and talk it over.

The user chose (from mockups) to make Maya's reply box **mirror the CLI's
menu** while a session is asking, rather than adding buttons above the
usual reply box.

## Goals

- A **Stop** button on any working or asking session, on the board card
  and in the session view, on all three devices. Stop = Esc.
- While a session asks a question, the **reply box becomes the CLI-style
  menu**: header, question, numbered options with descriptions,
  "Type something…" with an inline text box, a divider, "Chat about this",
  and Stop. When the question is answered the normal reply box returns.
- **Pick** an option by click/tap, or with ↑/↓, number keys and Enter on
  desktop, as in the CLI.
- **Type something** sends the user's text to the agent as the answer to
  that question.
- **Chat about this** sets the question aside the way the CLI entry does,
  then the normal reply box returns and the next send is an ordinary
  message.
- **Esc keeps closing the session view**, as it does today
  (`src/modal.ts:1017`); Stop is the button only **(my call)**. The mockup's
  "Esc to stop" hint is dropped. Arrow and number keys act on the menu only
  once the user has clicked or tabbed into it, so a stray key while reading
  picks nothing **(my call)**.
- Claude's **permission prompts and plan approvals** get the same menu,
  with the options the CLI shows for them, including "No, and tell Claude
  what to do differently" with its text box.
- **Multi-select** questions get ticks and a Submit row, like the CLI,
  instead of today's "Multi-select: answer in the terminal".
- **Voice:** a spoken reply while an AskUserQuestion is open is taken as
  "Type something" **(my call)**. On a permission or plan prompt a spoken
  reply is never turned into "No, and tell Claude…"; the spoken choice goes
  through the existing numbered `answer` action when Maya knows the list,
  and is refused as today otherwise **(my call)**. And and "Maya, stop <session>" stops that
  session **(my call)**. "Maya, stop" on its own keeps stopping the brain.

## Non-goals

- Ending or killing a session. Stop interrupts the turn only; Close stays
  as it is.
- Menus for Codex, Grok, Kiro and Antigravity prompts. Maya cannot see
  their options (`core/src/foreign.rs:43-48` only gives `Permission` or
  `Text`). They keep "answer it in the terminal", and gain Stop.
- Changing how Maya's own brain is stopped (Chat's Stop, "Maya, stop").
- Supporting terminals other than those Maya already drives
  (Terminal.app, tmux, Windows consoles).

## Anti-goals

- **Stop presses Enter.** Every existing typing path appends Enter
  (`Terminal::type_line`). An Esc followed by Enter could confirm
  something in the agent's UI. Stop must send Esc and nothing else.
- **A stale menu answers the wrong question.** The ask id
  (`stateSince`) and open-delay checks in `answer::check`
  (`core/src/answer.rs:169`) must cover every new path: Type something,
  Chat about this, permission and plan options, multi-select submits.
- **A stray tap stops a session or picks an option.** The existing
  ~1 s open delay (`OPEN_DELAY_MS`, `src/options.ts:4`) applies to the
  menu and to Stop on the phone.
- **The menu and the terminal disagree.** If the user answers in the
  terminal, Maya's menu must go away as soon as the state changes, as the
  current buttons do.

## How it behaves

### Stop

- Shown only when there is a turn to stop **(my call)**: Working because
  the agent is busy, or Awaiting on a menu ask (question, permission,
  plan). Hidden on Idle and Completed; hidden when the card is Working only
  because a background shell outlived the turn (`shell_running`,
  `core/src/state.rs:151`; Esc does not stop that shell, and stopping
  background commands is out of scope); hidden on a prose `Text` ask, where
  the turn has already ended and a second Esc would open Claude's rewind
  menu. The UI needs busy-vs-shell and the ask kind to decide.
- After a Stop that Maya delivered successfully, Maya marks the card's turn
  as ended (Idle) and clears the pending ask, because Claude sends no Stop
  hook on an interrupt and may send nothing else that clears it
  (`core/src/state.rs:118-145`). This lifts the "answer it first" refusal so
  the user can say what to do instead. If the agent was not in fact
  stopped, its next event moves the card back to Working, so the mark
  corrects itself **(my call)**. The marker travels between devices like
  any other state change. Placement: next to Send in
  the session view, and as a card action on the board.
- No confirmation (**my call**): Stop is cheap to recover from.
- Per agent:
  - **Claude, Codex:** a single Esc into the session's terminal.
  - **OpenCode:** `POST /api/session/{id}/interrupt`, already used by the
    brain driver (`core/src/brain/opencode_drv.rs:785-800`,
    `Client::post_empty` in `core/src/opencode.rs:96`).
  - **Grok, Kiro, Antigravity:** the builder confirms which key each one
    interrupts on (Esc or Ctrl-C) before wiring it. Probing these CLIs needs
    the user to run them (see the project memory on agent probes).
- Per platform, a new `Terminal` method that sends raw keys **without**
  Enter (`core/src/terminal.rs`):
  - **Linux:** `tmux send-keys Escape` (or `-H 1b`; raw bytes already
    exist at `core/src/terminal_tmux.rs:64`).
  - **Windows:** `WriteConsoleInputW` with a VK_ESCAPE record
    (`core/src/win_console.rs`).
  - **macOS:** Terminal.app's `do script` always appends a CR, so it
    cannot be used. The builder should find the quietest working route;
    the fallback the user has seen is to bring the session's tab forward
    for a moment and send Esc via System Events, which needs Accessibility
    permission and briefly raises the window. If Accessibility is not
    granted, Stop shows a clear error saying so rather than failing
    silently.
- When the session is Awaiting, Stop also cancels the question (that is
  what Esc does in the CLI).

### The menu

- Rendered in place of the composer (`src/modal.ts:603-665`) while the
  card is Awaiting with a menu-capable ask. The current banner options
  (`src/modal.ts:540-570`) go; the banner keeps only its text and the
  "Open terminal" button.
- The board card keeps a compact version of the options as today
  (`src/card.ts:62`) **(my call)**, plus Stop. "Type something" and
  "Chat about this" open the session view rather than appearing on the
  card **(my call)**.
- Multi-question asks: one question at a time, as today, with the
  existing "Question n of m" label and the existing warning against
  mixing Maya and the terminal.
- The phone uses the same code (`src/mobile/main.ts:139-161`), so it gets
  the same menu; keyboard hints are hidden there.

### What each entry does, per agent

| Ask | Options | Type something | Chat about this |
|---|---|---|---|
| Claude AskUserQuestion | from the tool input (`parse_questions`, `core/src/model.rs:45`) | move to the CLI's "Type something" row, type the text, Enter | move to the CLI's "Chat about this" row, Enter |
| Claude permission | the CLI's list for that prompt, e.g. Yes / Yes, and don't ask again… / No, and tell Claude what to do differently | the "No, and tell Claude…" row with its text box | not offered unless the CLI offers it |
| Claude plan approval | the CLI's list for plan prompts | per the CLI (e.g. "No, keep planning" with feedback, if offered) | not offered unless the CLI offers it |
| OpenCode permission | Once / Always / Reject, as today (`core/src/opencode.rs:492`) | only if OpenCode's API accepts a written reply with a reject | not offered |
| OpenCode form | as today | the form's free-text field, where it has one | not offered |
| Codex, Grok, Kiro, Antigravity | none (unchanged) | n/a | n/a |

Claude's permission and plan options are not in the hook events Maya
receives today, and they change between Claude Code versions. The builder
decides how Maya learns them: hook input fields (for example
`permission_suggestions` on `PermissionRequest`, if present), the
transcript, or a list kept per Claude Code version. If Maya cannot be sure
of the list for a prompt, it falls back to today's "answer it in the
terminal" for that prompt rather than guessing key presses.

### Typed text while asking

- In the menu, text goes through the "Type something" (or "No, and
  tell…") path, never as a bare line into the terminal while a menu is
  open.
- `answer::check_free` keeps refusing a plain reply while a menu-capable
  ask is pending; the UI no longer offers one in that state, so the
  refusal only matters for stale clients and voice.
- Voice: the interpreter's `reply` action
  (`core/src/interpreter.rs:99-110`) against an Awaiting question is
  turned into a Type-something answer, with the same `revalidate` stale
  check (`src-tauri/src/net_app.rs:371`).

### Across devices

New protocol commands in `CommandKind` (`core/src/net/protocol.rs:154`),
following how `Answer` and `Reply` travel today: `Stop{session}`, and an
extension of `Answer` (or a new command) that carries the ask id plus one
of: an option index, a set of option indexes (multi-select), written text
(Type something / No-and-tell), or Chat about this. Every place that lists
commands needs the new ones: `kind_name` (`core/src/net/server.rs:1242`),
`target_of` (`core/src/net/routing.rs:99-104`), `execute`
(`src-tauri/src/net_app.rs:476-511`), the Tauri commands and their
`remote_machine_of` routing (`src-tauri/src/lib.rs:896-1030`), the phone's
commands (`mobile/src/commands.rs:47-85`), and `Capabilities`
(`core/src/launch.rs:75-106`, mirrored in `src/harness.ts:54-74`) so the
UI only shows Stop and menu entries an agent supports.

## Constraints

- Every existing test keeps passing on macOS, Linux and Windows CI;
  unix-only tests are gated with `cfg(unix)`.
- Release: one version bump in the PR (`feat` → minor), per CLAUDE.md.
- The Claude menu key sequences assume the CLI's current row order; tests
  pin the sequences per ask kind so a change is visible.

## Live checks during the build

The user is available while this is built but cannot sit at the machine
(meetings), and answers by text. So the plan puts a short probe before
each piece that depends on a live agent: the builder writes the exact
steps, the user runs them when free and reports back, and the builder
works on other pieces meanwhile. Probes owed: Claude's key sequences for
Type something, Chat about this, No-and-tell and multi-select; Claude's
permission and plan option lists; the interrupt key for Grok, Kiro and
Antigravity; the macOS Esc route; what events Claude sends after an Esc.
Anything still unconfirmed when the rest is done ships switched off
through `Capabilities` (that prompt keeps "answer it in the terminal",
and Stop is hidden for that agent), and is listed in the PR as owed.

## Left to the builder

- The exact macOS route for a bare Esc, and its permission handling.
- Which key interrupts Grok, Kiro and Antigravity (needs a live check by
  the user).
- How to learn Claude's permission and plan option lists, and the key
  sequences for "Type something", "Chat about this", "No, and tell…" and
  multi-select ticks/Submit in the installed Claude Code.
- Whether OpenCode accepts a written reply on reject.
- The shape of the protocol change (extend `Answer` or add commands), and
  the interpreter's new `stop` action and its wording in `ACTIONS`.
- Styling, within the look of layout A in the mockup
  (`.superpowers/brainstorm/*/content/composer-choices.html`, not
  committed).

## Hand checks owed after building

Real sessions on the Mac, the laptop and the phone: Stop on a working
Claude, Codex and OpenCode session; each menu path on a Claude
AskUserQuestion, permission prompt and plan approval; a multi-select
question; voice "Maya, stop <session>" and a spoken own answer.
