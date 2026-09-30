# Maya CLI: a headless assistant for SSH boxes and containers — design

Date: 2026-09-30
Status: implemented
Builds on: `2026-09-30-maya-remote-assistants-design.md`

## Purpose

Run Maya's assistant role on a machine with no desktop: a Linux box or a Mac
reached over SSH, or a container. Its Claude Code sessions appear on the main
Maya's board and are driven from there (reply, answer, compact, rename,
options, history, start, resume), exactly as with an app assistant. The CLI
has no board of its own and never runs the main.

Decisions taken during design:

- Start and resume on a headless machine use **tmux**: each session the main
  starts runs in a named tmux session the user can attach to over SSH.
- Configuration is by **subcommands and the same config file** the app uses;
  no environment variables, no wizard.
- Releases ship **plain binaries**: macOS (arm64, x86_64) and Linux
  (x86_64, aarch64). No container image.
- The code is split into a **core crate** shared by the app and the CLI
  (approach A); a `Terminal` trait is the only platform seam.

## Behaviour

### Commands

```
maya pair <host>[:port] --code <6 digits> [--name <label>]
maya run
maya status
maya hooks install | remove | status
maya config projects-dir <path>
maya start [--dir <project>] [--prompt "<text>"] [--model ..] [--effort ..] [--mode ..]
maya --version
```

- **pair** performs the pairing handshake once (the same `pair` frame and
  the same check of the main's proof as the app) and writes
  `~/.claude/maya/config.json` with `network.role = assistant`, `main_host`,
  `main_port` (default 4127), `name` (the hostname when omitted), and the
  assistant id and token. It prints `Paired with <main name> as <label>` and
  exits 0; on failure it prints the same message the app would show
  ("Wrong or expired pairing code.", "The main Maya failed to prove it
  received the pairing code.", "Could not reach <host>:<port>: …") and exits 1.
  A code that is not exactly six digits gets "Wrong or expired pairing
  code." without contacting the main; the name is trimmed (blank means the
  hostname); port 0 is a usage error (exit 2).
  Pairing again replaces the stored credentials. It refuses while a
  `maya run` is live (`stop \`maya run\` first (pid N)`, exit 2), since
  that run holds the old credentials.
- **run** is the assistant, in the foreground. It refuses to start without
  assistant credentials (`run \`maya pair\` first`, exit 2) or when the
  config's role is `main` (`the CLI is assistant-only`, exit 2). It watches
  the same files the app watches (the registry, the events log, the Codex
  and Antigravity directories), keeps the board, connects to the main with
  the stored token, pushes boards on change and every 10 s, executes the
  main's commands, and logs one line per connect, disconnect, executed
  command and error to stdout in the app's log format. SIGINT or SIGTERM
  closes the link cleanly and exits 0. It does not notify, speak or listen.
- **status** prints the role, the main's host and port, the stored name,
  whether a `maya run` is connected (read from a small status file
  `~/.claude/maya/cli-status.json` that `run` rewrites on every status
  change and removes on exit), the projects directory (`projects: <dir>`
  or `projects: unset`), and the number of live local sessions.
- **config projects-dir** saves the folder whose subfolders start and
  resume pick from into `config.json`'s `projectsDir` (a `~` is kept and
  expanded when read, as in the app; another relative path is made
  absolute). A folder that does not exist is refused. `run` warns on start
  when none is set: start and resume from the main fail until it is.
- **hooks** installs, removes or reports the Claude Code hooks, writing the
  same `~/.claude/maya/hook.sh` and `settings.json` entries the app does.
  `run` warns on start when the hooks are not installed (sessions would not
  be seen) and keeps going. `hooks install`, `hooks status` and `run` warn
  when `jq` is not on the PATH (`jq is not installed: Claude Code's hooks
  need it, so sessions will not be seen`); `hooks install` still installs.
- **start** is a local shortcut with the same behaviour as a `start`
  command from the main: it resolves the project directory (the classifier
  when `--dir` is omitted and a prompt is given), writes the prompt file and
  opens the session in tmux. It prints the tmux session name.

### Sessions on a headless machine

- Discovery is unchanged: the hooks write the registry and events; the
  store reads them; Codex and Antigravity TUIs are found by `ps`/`lsof` as
  today. A `claude` started in tmux, in a plain SSH shell or by `maya start`
  is a card on the main's board.
- On Linux a session's tty comes from `/proc/<pid>/fd/0` when that is a
  terminal, else from `ps` (whose "no terminal" is `?` there, `??` on
  macOS), and a reply's inbox socket is checked for the session's pid with
  `SO_PEERCRED` (`LOCAL_PEERPID` on macOS).
- `config.json` (it holds the token) and `cli-status.json` are written
  with mode 0600.
- **Start and resume** create `tmux new-session -d -s maya-<8 hex chars>
  -c <project dir> 'sh -c "<the same claude command line the app builds>;
  exec ${SHELL:-sh}"'`, creating the tmux server if needed; the shell that
  follows keeps the pane, and the session's last screen, after `claude`
  exits. The prompt goes through the prompt
  file as in the app. The card's remote tooltip on the main shows the tmux
  session name (`attach: tmux attach -t maya-1a2b3c4d`) when the assistant
  reports one; app assistants report none.
- **Missing tmux**: the command fails with `tmux is not installed on
  <machine>`, which the main shows in its Start/Resume dialog.
- **Focus** is not available headless; a `focus` request on such a card is
  answered `Attach with: tmux attach -t <name>` and the main shows that.
  Nothing else changes for remote cards (no Terminal button, as now).

### Networking

- The CLI reuses `net::client` as is: a `CliExecutor` implements the
  client's `Executor` over the core store and the tmux `Terminal`; a
  `CliNotify` logs status changes and writes the status file.
- Platform on the wire is `std::env::consts::OS` (`macos` or `linux`). The
  main tells a CLI assistant from an app one only by the tmux name in its
  cards, if at all.
- Pairing proof, keepalive, backoff, DNS timeout, attachment handling and
  command timeouts are those of the app's client.

### Release

- The release workflow builds `cli/` for `aarch64-apple-darwin`,
  `x86_64-apple-darwin` (signed with the "Maya Development" identity when
  the signing secrets are present, else unsigned), `x86_64-unknown-linux-musl`
  and `aarch64-unknown-linux-musl` (via `cross`), and attaches
  `maya-macos-arm64`, `maya-macos-x86_64`, `maya-linux-x86_64`,
  `maya-linux-aarch64` to the same `v<version>` release. The `linux` job
  runs first (tests on pull requests and pushes; on a release push it also
  builds the Linux binaries and uploads them as an artifact); the `macos`
  job runs after it on pushes only and creates the release with every
  asset at once, so a failed Linux build publishes nothing.
  `scripts/release-version.sh` makes the release decision for both.
  `maya --version` prints the workspace version.
- CI runs `cargo test` for `core` and `cli` on macOS and Linux runners, so
  core stays portable.

## Architecture

### Workspace

```
Cargo.toml            workspace: members core, src-tauri, cli; shared version
core/                 maya_core — no tauri, no objc2
src-tauri/            maya (the app) — depends on maya_core
cli/                  maya-cli — binary "maya", depends on maya_core
```

`core` receives, by `git mv`, every module that has no Tauri dependency:
model, store, state, registry, events, transcript, context, foreign, codex,
antigravity, grok, notify, interpreter, config, log, hook_install, launch,
resume, inbox, answer, attachments, pr, reviews, and `net` (protocol,
merge, server, client). `net::client` loses its Tauri executor and notifier
(they move to the app) and keeps the traits. The voice pipeline (`voice`,
`ear`, `wake`, `models`, `listener`), `focus`, `dock` and the Tauri commands
stay in the app: the CLI never listens or speaks, and `dock` and `focus`
use AppKit.

### The `Terminal` trait

```rust
pub trait Terminal: Send + Sync {
    /// Runs `command` in a terminal the user can see, in `cwd`, under `label`
    /// (the tmux session name; Terminal.app ignores it). Returns the name the
    /// card can show (the tmux session, or None for Terminal.app).
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String>;
    /// Types `text` followed by Enter into the terminal hosting `tty`: replies
    /// to sessions without an inbox, answers (arrow keys), slash commands,
    /// Shift+Tab. Err says why it cannot ("not in tmux", "no Terminal tab").
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String>;
    /// Brings the terminal hosting `tty` forward; Err carries what to do instead.
    fn focus(&self, tty: &str) -> Result<(), String>;
    /// The terminal's name for the card hosting `tty` (a tmux session), if any.
    fn name_for_tty(&self, tty: &str) -> Option<String>;
}
```

Typing into a session is what answers, slash commands and Shift+Tab are
made of: today `answer::type_into_tty` runs AppleScript's `do script … in
tab` for the tab whose tty matches. That is Terminal.app only, so it is
part of the seam too.

- App: `open` runs the AppleScript `do script` in Terminal.app as
  `launch::open_terminal_with` does today and returns `None`; `type_line`
  is today's `type_into_tty`; `focus` is today's `focus.rs` behaviour;
  `name_for_tty` is `None`.
- CLI: `open` runs `tmux new-session -d -s <label> -c <cwd> …` and returns
  `Some(label)`; `type_line` finds the pane whose `pane_tty` is `tty` with
  `tmux list-panes -a` and runs `tmux send-keys` (literal text, then
  `Enter`; the escape sequences for Down and Shift+Tab are sent as the tmux
  keys `Down` and `BTab`); `focus` returns `Err("Attach with: tmux attach
  -t <name>")`; `name_for_tty` is the pane's session name.
- A `claude` running headless outside tmux (a plain SSH shell) is shown on
  the board and can be replied to through its inbox, but answers, slash
  commands and Shift+Tab fail with `This session is not in tmux; only
  replies reach it.` Sessions the user starts inside tmux themselves work
  fully: the seam looks pane up by tty, not by who started it.

The assistant fills each card's `terminal` from `name_for_tty(tty of pid)`
when it builds a board (the CLI executor caches the tty per pid); the app
leaves it empty. No store state is needed for it.

### Local actions in core

The action bodies now in `lib.rs` (`send_reply`, `answer_question`,
`compact_session`, `rename_session`, `set_session_option`,
`cycle_session_mode`, `session_history`, `list_resumable_sessions`,
`list_project_dirs`, `start_session`, `resume_session`) move to
`core::actions` as functions taking `&Mutex<Store>` (or `&mut Store`), the
`maya_dir`, `now_ms`, and `&dyn Terminal` where a terminal is needed. The
app's Tauri commands become thin wrappers that route remote sessions as now
and call the core function for local ones. `net::client::execute` calls the
same core functions, so the app's and the CLI's executors share one body.

The `Card` gains `terminal: Option<String>` (`#[serde(default)]`,
`skip_serializing_if`), set by the assistant as described under the
`Terminal` trait.

### The CLI process

`main` parses the subcommand (a hand-rolled parser over `std::env::args`;
no clap), loads the config, and for `run` builds: the `Store` with the OS
`alive` check, the watcher thread, a `CliNotify`, a `CliExecutor { store,
terminal: Tmux }`, and starts `net::client` with them. A `refresh` on each
watcher change updates the store and marks the board due, as the app's
`refresh_and_emit` does without the emit. Signals are caught with a
`libc` handler setting the client's stop flag.

## Error handling

- `pair`, `status`, `hooks`, `config`, `start`: one line on stderr, exit 1 (2 for a
  usage or configuration error).
- `run`: errors from the main's commands go back as the command's error;
  link errors are logged and retried with the client's backoff; a config
  that stops being an assistant (the file edited while running) ends `run`
  with exit 2 and a message.
- tmux failures carry tmux's stderr in the message.

## Testing

- Core's tests move with their modules and stay green; the app's cargo,
  vitest, tsc and ear suites stay green through the move (the check that
  nothing was lost).
- `core::actions` tests use a `FakeTerminal` recording `open`/`focus` calls;
  start and resume are tested without tmux.
- `cli`: the tmux command line is tested as a string; one integration test
  runs `tmux` when it is on the PATH and is skipped otherwise; `maya run`
  is tested end to end against core's localhost main harness (pair from the
  CLI's pairing function, connect, push a board, execute a reply into a fake
  inbox socket); the argument parser is tested for every subcommand and
  usage error.
- CI runs the core and cli tests on ubuntu-latest and macos-latest.

## Out of scope

A terminal board view, Windows, a headless main, environment-variable
configuration, a container image.
