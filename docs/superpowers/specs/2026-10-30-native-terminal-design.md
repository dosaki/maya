# Native terminal: Maya hosts a session's terminal herself (design)

Date: 2026-10-30
Status: approved
Builds on: the terminal seam (`core/src/terminal.rs`, `src-tauri/src/terminal_app.rs`,
`terminal_linux.rs`, `terminal_win.rs`), the New sessions open in setting
(`config::NewSessionTerminal`, `src/settings.ts`), the session viewer
(`src/modal.ts`), remembered sessions (`core/src/remembered.rs`) and the
network protocol (`core/src/net/protocol.rs`)

## Purpose

Today every session Maya starts or resumes opens in an external terminal
app: Terminal.app or iTerm2 on macOS, Windows Terminal with Git Bash on
Windows, and tmux shown in GNOME Terminal on Linux. Maya reaches those
sessions from outside, through AppleScript, console attachment or tmux.
The user wants a third choice, **Native**, where Maya runs the session's
terminal herself and no external app opens.

A Native session has no window of its own. It is its card and its viewer.
The viewer of a Native session behaves like a real terminal.

Success:

- With Native chosen, starting, resuming or reviewing opens no external
  app, and the session shows on the board as any other does.
- Opening a Native session's viewer shows its live screen. Every key works
  as it would in Terminal.app, including arrows, Ctrl+C, Esc, Tab, paste,
  `/login`, `/config` and the agent's own menus.
- Closing and reopening the viewer, or opening it from the main or from the
  phone partway through, shows the correct current screen. It is never a
  garbled replay.
- Quitting Maya while a Native session is Working asks first.
- A Native session that was open when Maya quit is offered back at the next
  start, and it reopens as a Native session.

## Decisions (by the user)

- **Native is a fourth value of New sessions open in**, next to the
  existing choices.
- **Native terminals end with Maya.** When Maya quits, her terminals close
  and the sessions in them end. Nothing keeps them alive (no tmux, no
  daemon).
- **Quitting warns when a Native session is Working.** If any Native
  session is in the Working column, Maya shows a warning before she exits.
- **Native sessions are remembered and offered back** when Maya starts.
- **A Native session's terminal appears nowhere else.** The session is the
  terminal.
- **The session viewer behaves like a real terminal in Native mode.** Using
  it must be the same as or better than using an external terminal.
- **The viewer's look (chat or terminal) is the user's saved choice**, as
  for every session. It is only a visual preference and does not change
  what the user can do.
- **A remote session is reached through its own Maya.** The main reaches an
  assistant's Native session, and the phone reaches the main's, through the
  Maya that hosts it, over the existing encrypted link.
- **Approach 1:** Maya tracks each Native terminal's screen in the backend
  with a headless terminal emulator (see Approaches).

## Approaches considered

All approaches share this: the Rust backend owns one pseudo-terminal per
Native session (`portable-pty`, which uses a Unix PTY on macOS and Linux
and ConPTY on Windows), and viewers draw it with xterm.js.

1. **Maya tracks the screen (chosen).** Every byte the agent prints also
   goes through a headless emulator (the `vt100` crate) in the backend, so
   Maya always holds the current screen and scrollback. A viewer that
   attaches gets an exact snapshot (`vt100`'s formatted contents), then the
   live byte stream. Reopening, joining late from the phone and reconnecting
   after a network drop all show the right screen. It costs one more crate
   and a little CPU per session.
2. **Replay buffer only.** The backend keeps the last megabyte or so of raw
   output and replays it to each new viewer. Rejected: the agents' TUIs
   redraw constantly and switch screen modes, so a replay that starts
   mid-stream comes out garbled until the next redraw. Scrollback is
   unreliable, and the replay is the biggest thing sent to a phone.
3. **Hidden tmux.** Rejected: Windows has no tmux, and tmux sessions
   outlive Maya, which contradicts "her terminals quit with her".

## Goals

- A Native choice for new, resumed and review sessions, on macOS, Windows
  and Linux, in the desktop app.
- Every card action reaches a Native session by writing to its terminal:
  reply, answer, pick, Stop (Esc), compact, close, rename, slash commands,
  `!` lines, model, effort and mode changes.
- A live, typeable terminal in the viewer, locally, on the main for an
  assistant's session, and on the phone.
- A quit warning, and resumption after a restart.

## Non-goals

- Keeping Native sessions alive after Maya quits.
- Native in the headless `maya` command. It keeps using tmux.
- Moving a Native session into an external terminal app, or the reverse.
- Turning sessions started outside Maya into Native ones. Sessions the user
  started by hand in any terminal keep working as today.

## Anti-goals (a failure even if it works)

- A Working Native session killed by a quit with no warning.
- A viewer that shows a garbled or stale screen after reopening.
- Native support that breaks typing into or focusing sessions in Terminal.app,
  iTerm2, Windows Terminal or tmux.
- A phone or remote viewer that resizes, and so rewraps, the terminal on the
  machine that hosts it.

## Design

### The setting

- `NewSessionTerminal` gets a `Native` variant (`"native"` in
  `config.json`). The default stays as it is today.
- Settings › Sessions › New sessions open in is shown on every desktop
  platform, not only macOS:
  - macOS: Terminal, iTerm2, Native;
  - Windows: Windows Terminal, Native;
  - Linux: Terminal (tmux), Native.
- The labels of the non-Native options are the builder's call. On Windows
  and Linux, the non-Native choice keeps today's behaviour exactly.
- Changing the setting affects only sessions started after the change.

### The native host (backend)

A new module in `core` (proposed `core/src/terminal_native.rs`) owns every
Native terminal on this machine:

- **Spawn.** `open(command, cwd, label)` starts the agent in a new
  pseudo-terminal, in `cwd`. It runs through the user's login shell, the
  way the external terminals run it today, so `PATH` and the rest of the
  environment match even when Maya was opened from the Dock. It sets
  `TERM=xterm-256color`. On Windows it runs the same Git Bash command line
  the Windows path uses today, inside ConPTY.
- **Reader.** One thread per terminal reads output, feeds it to the session's
  `vt100::Parser` (scrollback: 10,000 lines, the builder may tune this),
  and fans the bytes out to attached viewers. Output to viewers is batched
  in short intervals (about 16 to 50 ms), so a busy TUI does not send one
  message per byte.
- **Writes.** It implements the `Terminal` trait. `type_line`,
  `press_keys` and `type_reply` write bytes straight to the PTY,
  keeping the key sequences `answer.rs` already uses (`NEW_LINE`, `SEND`
  and `ESC`). `focus` opens the viewer (see below). `name_for_tty` returns
  `None`.
- **Finding a session.** The board finds sessions through hooks and `ps`, as
  today. A Native session is found the same way. On macOS and Linux its tty
  is the PTY's device path, which the host knows. On Windows the host knows
  the child's process id and maps it to the key the Windows code uses
  (`console:<pid>` or the agent's pid inside the console). The builder
  decides the exact mapping.
- **Routing.** The platform terminal is wrapped. Every call first asks the
  native host whether it owns that tty or key; if not, the call goes to
  Terminal.app, iTerm2, Windows Terminal or tmux as today. `open` goes to
  the native host when the setting says Native.
- **End.** When the agent exits, the PTY closes, the host drops the
  terminal, and any open viewer shows the final screen with a note that
  the session ended. Running through `<shell> -l -c` (not an interactive
  shell left behind) means no invisible shell is left over. `close`
  (`/exit` then `exit`) succeeds as soon as the process is gone, since
  `reach_after_exit` has nothing left to reach.
- **Quit.** On exit Maya kills every Native terminal's process group (on
  Windows, its job or process tree), so no agent is orphaned.

### The viewer (frontend)

- For a Native session, the viewer's terminal look is a live xterm.js
  terminal (`@xterm/xterm` and its fit add-on). The chat look is unchanged.
- **The look follows the user's choice, as for every session (user).** A
  Native viewer opens in the saved look (`viewerTheme`), and the `>_` toggle
  saves the choice as it does today. The look is only a visual preference:
  either look can do everything with the session.
- **In the terminal look, the terminal is the whole viewer (user: the
  experience must be the same as or better than an external terminal).**
  It has no reply box, answer menu or "waiting for a decision" banner below
  it; the user types straight into the terminal, as in Terminal.app. The
  chat look keeps its reply box and answer menu, which write into the
  terminal.
- On open, the viewer attaches. It gets the snapshot, then the live stream.
  On close, it detaches. **The snapshot includes the scrollback (my call)**,
  so the user can scroll back after reopening: all of it locally, and the
  screen plus the last 1,000 lines for a remote viewer, to keep the link
  light.
- Every key goes to the terminal while it has focus, including Esc, Tab,
  Ctrl+C and the Enter combinations. Maya's own viewer shortcuts (Esc to
  close, the Settings › Keyboard Enter rules) do not apply inside it.
  Closing the viewer is by its close button or a click outside. Copy and
  paste follow the platform: Cmd+C and Cmd+V on macOS; Ctrl+Shift+C and
  Ctrl+Shift+V on Windows and Linux, so Ctrl+C still reaches the agent.
- The terminal's size follows the local viewer: opening or resizing the
  viewer resizes the PTY. While no local viewer is open, the PTY keeps its
  last size. New terminals start at 120 x 40, which the builder may tune.
- The card's Terminal button on a Native card opens the viewer, in the
  saved look, instead of calling `focus_session`.
- The viewer's colours follow Maya's theme (dark or light).
- The card is unchanged: columns, reply box, answer menu and Stop all work.
  They now write into the hidden terminal.

### Remote viewers

- A new capability, proposed `CAP_TERMINAL`, marks a Maya that hosts or
  relays Native terminals. A remote card for a Native session on a Maya
  without it keeps today's transcript views only.
- New messages carry, in each direction:
  - **attach:** session to snapshot, then subscribe;
  - **output:** batched bytes, with a sequence number so a gap can trigger
    a fresh snapshot;
  - **input:** keystroke bytes;
  - **detach.**
- The main relays between the phone and an assistant, as it relays other
  requests today (`Request { machine, … }`).
- Remote input is trusted as card replies are today: anyone who may reply
  to a session on the main may type into it.
- **A remote viewer never resizes the PTY.** It renders the host's size,
  scaled to fit or scrolled sideways. The phone opens a Native session in
  the user's saved look, as the desktop does.
- **The phone's live terminal has a row of extra keys (my call):** Esc, Tab,
  Ctrl+C and the arrows, which an on-screen keyboard cannot send and the
  agents' menus need.
- The protocol version bumps or the capability gates it. A mismatched
  pair degrades to the transcript views, never to an error.

### Quit warning

- When Maya is about to exit, and any local Native session's card is in
  the Working column, she prevents the exit and asks. The question names
  those sessions and says they will stop. It offers Quit anyway and Cancel.
- With no Working Native session, she quits without asking, as today.
- A restart to install an update asks the same question before it
  installs. The comment in `updater.rs` saying the relaunch ends no
  session is no longer true for Native sessions and must be updated.
- Shutdown or logout of the machine is not blocked by the question. If the
  system is ending the session, Maya exits.

### Resume

- The remembered list already offers sessions that were running when Maya
  stopped (`remembered::classify_at_start`). A Native session open at quit
  falls into it with no change to that logic.
- A remembered entry records that its session was Native (a new optional
  field). Resuming it, from the Sessions tab or by voice, reopens it as a
  Native session, whatever the setting says now.
- **A remembered session that was not Native reopens where the setting
  says (my call)**, as resuming does today, so with the setting on Native it
  comes back as a Native session. A resume starts a new process, so this is
  not "moving a session into Native". Only the Native flag is recorded, not
  the full terminal kind.
- Nothing resumes on its own. The offer stays as it is today.

## Constraints

- Tauri 2, a plain TypeScript frontend, and vitest. jsdom has no canvas, so
  frontend tests stub xterm.js and test the attach, detach and key wiring
  around it.
- Every external terminal path keeps working unchanged. The `Terminal`
  trait's existing tests and the `FakeTerminal` stay valid.
- The phone app (`mobile/`, `src/mobile/`) uses the same protocol and needs
  xterm.js in its bundle too.
- Release: this is a `feat`, so it bumps the minor version, adds a
  `RELEASE_NOTES.md` section, and updates the README (macOS, Windows and
  Linux sections, and What it does).

## Left to the builder

- The exact Windows mapping between a Native terminal and the board's
  session key.
- Batch interval, scrollback length, default size, and how the phone scales
  a wide screen.
- The wire format of the new messages, and whether to gate them on a
  capability, a protocol bump, or both.
- The labels of the per-platform choices in Settings.
- How the viewer marks a session that ended.
