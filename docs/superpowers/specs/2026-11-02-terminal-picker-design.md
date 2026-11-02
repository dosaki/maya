# Pick the terminal when starting or resuming a session

## Intent

Today one setting, Settings → "New sessions open in", decides where every
session opens. It is read on the machine that opens the session, at the
moment it opens it. The user wants to choose the terminal for one session
at the moment of starting or resuming it, without changing that setting.
The setting stays the default: the picker starts on it, so leaving the
picker alone behaves exactly as today.

## Goals

- New Session and Resume each get a **Terminal** picker.
- The picker lists the terminals of the **target machine** (this Mac, a
  paired assistant, or, on the phone, the main or an assistant), with that
  machine's Settings default selected.
- An assistant tells the main its terminals and default when it connects
  or reconnects, and again whenever its "New sessions open in" setting
  changes, so the main never offers a stale default.
- The phone gets the same information for every machine it can start on,
  the main's own included.

## Non-goals

- Every other open keeps using Settings: reviews, Make main, worktree
  opens other than Start, voice/brain-started sessions, card actions.
- No "remember my last pick": each dialog opens on the Settings default.
- No change to Settings itself or to which terminals a platform has
  (`terminalChoices()` in `src/settings.ts` and the Rust side stay the
  source of truth; Terminal/iTerm2/Native on macOS, Windows Terminal/Native
  on Windows, Terminal (tmux)/Native on Linux).

## Anti-goals

- A pick that silently opens somewhere else. A machine sent a terminal it
  does not have (iTerm2 to Linux) refuses with a plain message.
- A pick that leaks into later opens: the override applies to that one
  open only, and Settings is never written.
- Breaking an older Maya on either end.

## Constraints

- The protocol stays compatible both ways, as with worktrees
  (`CAP_WORKTREE`): an older assistant sends no terminal information, so
  the main shows no picker for it and does not send it a pick; an older main
  never sees the new update message.
- `WithNative::choice` and `MacTerminal::choice` read the setting on each
  use through `crate::new_session_terminal()`; both must see the override.

## Approaches considered

1. **The machine's information carries its terminals (picked).** The
   assistant's `Hello` carries its terminal choices and default; a new
   up message re-sends them when the setting changes. The main stores them
   with the assistant's other details and hands them to the dialogs with the
   agents. The picker fills the moment the dialog opens. The cost is a new
   field and one new message.
2. **Ask the machine when the dialog opens.** A new request per dialog.
   Always fresh, but a new round trip, an empty picker until it answers, and
   a timeout path. Dropped: the user wants the information sent on connect.
3. **Work it out from the platform the board already reports.** No
   protocol change, but cannot know the machine's default. Dropped: the
   default must be preselected.

## How it works (decided)

### What a machine announces

- A `TerminalChoices { choices: Vec<NewSessionTerminal>, default:
  NewSessionTerminal }` (name is the builder's) describes a machine: the
  terminals its platform has, in Settings order, and its saved setting (the
  same fallback Settings uses when the saved one is not on this platform).
- `Up::Hello` gains an optional `terminals` field, and a new capability
  (e.g. `CAP_PICK_TERMINAL`) says this Maya honours a pick in Start and
  Resume.
- A new up message (e.g. `Up::Terminals { terminals }`) re-sends it when
  the setting changes while connected. It is sent only to a main that
  says, in `Welcome.caps`, that it understands it.
- The main keeps it per assistant beside `worktrees` and `agents`
  (`RemoteBoard`/`merge.rs`), and drops it when the assistant disconnects.

### What the dialogs get

- `AgentsReply` (from `list_agents`, and from `Request::ListAgents` for a
  mirroring Maya such as the phone) gains an optional `terminals`. Locally it
  is built from this machine's platform and config; for an assistant it is
  what that assistant announced; absent for an older one.
- On the phone, `Request::ListAgents` to the main answers the same way for
  the main itself and for each assistant, so the phone needs no extra
  message. If the main is too old to send it, the phone shows no picker.

### The picker

- A "Terminal" select in New Session and in Resume, beside Agent, built
  with the existing `renderChoice` helper. It lists the target machine's
  terminals with their platform labels and selects its default.
- Switching machine reloads it with that machine's choices, the same way
  agents reload today. No information for that machine: no picker, and the
  session opens where that machine's Settings say.
- A single-choice platform still shows the picker (every platform has at
  least two choices today).

### Carrying the pick

- New Session: `LaunchOptions` gains an optional `terminal` (absent = use
  Settings), so `CommandKind::Start` carries it to an assistant unchanged.
- Resume: `resume_session` and `CommandKind::Resume` gain an optional
  `terminal`, `#[serde(default)]`.
- The main sends a pick to an assistant only when that assistant announced
  the capability, and otherwise refuses with a plain message, like
  `check_remote_worktree`.
- The machine that opens the session checks the pick is one of its choices,
  and refuses with a plain message otherwise.
- Locally the open runs inside a scoped override, a thread-local read by
  `crate::new_session_terminal()` before config, in the style of
  `native::route::opening`. It is cleared when the open returns, so other
  opens and other threads keep reading Settings.

### Calls made after the review (not discussed with the user)

- **The dialog always sends what it shows.** With the picker left alone it
  sends the default it displays, so the session opens where the dialog said,
  even if the machine's setting changed a moment before.
- **Rust gets its own per-platform list** (by `cfg(target_os)`), matching
  `terminalChoices()`. The machine opening the session checks the pick
  against it, and it builds what the machine announces.
- **The check runs before the open.** Today `WithNative` quietly sends an
  iTerm2 pick off macOS to the platform terminal; a pick must be refused
  before that point, never quietly rerouted.
- **A new name for the stored field.** `RemoteBoard.terminals` already means
  "can show live terminals" (`CAP_TERMINAL`), so the new per-assistant field
  gets another name (e.g. `terminal_choices`).
- The phone path needs no new message: the phone already sends
  `Request::ListAgents`, `Start` and `Resume` to the main, and the main's
  `serve_request` checks the pick where it checks the agent and worktree.

## Left to the builder

- Exact names of the struct, field, capability and message.
- Whether the change watcher is the save path in `save_config` or a
  config-file watch, as long as a change made in Settings reaches the main.
- Whether the TypeScript side reuses `terminalChoices()` for labels with the
  platform passed in, or gets labels from Rust.
- Tests in the project's existing style: protocol round trips (old and new
  shapes), the override scoping, refusal of an unknown pick, and the dialogs
  rendering and sending the pick.
- Release notes and a minor version bump (it is a `feat`).
