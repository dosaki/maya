# Kiro CLI as a Maya agent — design

Date: 2026-10-03
Status: approved
Builds on: `2026-10-02-maya-choose-agent-design.md` (the agent list, the
per-agent launch flags and pending names) and
`2026-10-03-maya-agent-design.md` (Maya's agent, one-shot commands,
resume for every agent, session controls by capability).

## Purpose

Kiro CLI becomes the fifth agent Maya knows, with everything the other
four have: live sessions on the board with their state and replies, New
session with model, effort, mode and name, Resume, rename, the session
controls Kiro has, Maya's brain (the voice interpreter and the folder
classifier), pull request reviews, other machines, and Windows and Linux.

OpenCode follows in a design of its own: it runs behind a shared server
with an HTTP API, which is a different shape from the four file-and-tty
agents and from Kiro.

Decisions taken during design:

- No Kiro hook. Kiro's hooks (session start, prompt, before and after a
  tool, file saved, stop) would tell Maya nothing its files do not, and
  the one state the files cannot show, waiting for a tool approval, has
  no hook event either. With the default V2 engine hooks also live inside
  an agent config, and the built-in default agent cannot carry one.
- A Kiro session waiting for a tool approval shows as Working, with the
  pending tool call as its snippet. Verified: while an approval sat
  unanswered for five hours the per-turn marker kept its heartbeat and
  the event log ended at the tool call, exactly as while the tool ran.
- Kiro's `/compact` summarises older messages to free context (its
  terminal-UI table calls it "compact message display", which misled the
  first draft of this design), so Kiro has the Compact control.
- Maya's one-shot runs leave sessions in Kiro's store. They run in Maya's
  own data folder, so they never appear in a project's Resume list.

## Findings (verified on this Mac, Kiro CLI 2.27.0, 2026-10-03)

Binary: `kiro-cli`, a symlink in `~/.local/bin` to
`/Applications/Kiro CLI.app/Contents/MacOS/kiro-cli`. The default engine
is V2.

Processes of one interactive session, from `ps -axo pid=,ppid=,tty=,command=`:

| pid | parent | tty | command |
|---|---|---|---|
| 95245 | shell | ttys010 | `kiro-cli chat` |
| 95295 | 95245 | ttys010 | `…/kiro-cli-chat chat` |
| 95441 | 95295 | ttys010 | `…/kiro-cli/bun --no-env-file …/kiro-cli/tui.js chat` |
| 95508 | 95441 | none | `…/kiro-cli-chat acp` |

The last one is the agent; the one above it is the TUI. Neither holds a
session file open, so `lsof` does not map them to a session.

Files, with `<sessions>` = `~/.kiro/sessions/cli` and `<run>` =
`~/Library/Application Support/kiro-cli/run` (Linux: the XDG data dir,
Windows: `%LOCALAPPDATA%`, both to be verified on the VM and the laptop):

| File | Written | Content |
|---|---|---|
| `<sessions>/<id>.lock` | at session start | `{"pid": <agent pid>, "started_at": "<RFC 3339>"}` |
| `<sessions>/<id>.json` | at session start and after each turn | `session_id`, `cwd`, `created_at`, `updated_at`, `title` (null until named), `session_state.rts_model_state.context_usage_percentage`, `model_info.model_id` |
| `<sessions>/<id>.jsonl` | during the turn | one event per line: `{"version":"v1","kind":…,"data":…}` |
| `<sessions>/<id>.history` | as typed | the input lines |
| `<run>/turn-markers/<tui pid>-<turn start ms>.json` | for the turn's duration | `pid`, `turn_started_at_ms`, `last_alive_at_ms` (heartbeat), `engine`, `session_interface` |

Event kinds in the `.jsonl`: `Prompt` (`content[].kind == "text"`,
`meta.timestamp` in seconds), `AssistantMessage` (`content[]` of kinds
`thinking`, `text`, `toolUse` with `name` and `input.command` or
`input.__tool_use_purpose`), `ToolResults` (`content[].data.toolUseId`,
`status`). The prompt and the assistant's tool call are appended before
the tool runs; the result and the final text when the turn ends.

Commands:

| Purpose | Command | Output |
|---|---|---|
| models | `kiro-cli chat --list-models -f json` | `{"models":[{"model_id","model_name","description","context_window_tokens"}]}` |
| sessions | `kiro-cli chat --list-sessions -f json` | per cwd; not used, Maya reads the session files |
| launch | `kiro-cli chat [--model m] [--effort e] [-a] -- "$p"` | `--effort` takes `low`, `medium`, `high`, `xhigh`, `max`; `-a` is `--trust-all-tools` |
| resume | `kiro-cli chat --resume-id <id>` | |
| one-shot | `kiro-cli chat --no-interactive --trust-tools= --output-format stream-json [--model m] "<prompt>"` | JSON lines; the last is `{"type":"runFinished","data":{"status":"success","finalText":"…"}}` |

No system-prompt flag: the system text is folded into the prompt, as for
Codex and Antigravity. `--trust-tools=` with an empty list trusts no tool.

Session controls, from the terminal UI docs and the live session:
`/model` and `/effort <level>` switch; Shift+Tab (or `/plan`) toggles plan
mode; `/` lines work; `!` lines run shell commands; `/compact` summarises
the conversation; `/rename <name>` sets the title and is written to the
`.json` file.

## Behaviour

### Discovery

Every `<sessions>/<id>.lock` whose pid is alive is a live session, the
way Grok's registry entries are. The session's folder is `cwd` from the
`.json`; its name is the `.json` title, else `kiro-<pid>`.

The agent pid has no terminal. Maya walks up its parents (`ps -axo
pid=,ppid=,tty=` on macOS and Linux, the process list on Windows) to the
first process on a tty: that tty takes typed replies and is what Focus
raises. The TUI's pid, one level up from the agent, names the turn
marker. A lock whose parent chain has no tty (a `kiro-cli chat` under
`--no-interactive`, or Maya's own one-shot) is not a session.

The store re-reads the lock files on every refresh, drops sessions whose
pid is gone, and re-reads names for `/rename`, as it does for Grok.

### State

- **Working** while `<run>/turn-markers/` has a marker whose `pid` is the
  session's TUI pid. The marker's `turn_started_at_ms` is the state's
  start. A marker whose heartbeat is older than two minutes is ignored.
- Otherwise the event log decides, as `foreign::derive` does for every
  foreign agent: a prose question in the last assistant text is
  **Awaiting** (text), else **Completed** since the last event, decaying
  to **Idle** after the completed timeout.
- The snippet is the last assistant text. When the last event is a tool
  call with no result, the snippet names it instead: `shell: <command>`
  for the shell tool, else `<tool>: <purpose>`. A Kiro session waiting
  for an approval therefore shows as Working with the command it is
  waiting on; the limitation is documented.
- The last event time is the latest of the marker's start, the event
  log's last prompt timestamp and the `.json` file's `updated_at`.
- Context usage is `context_usage_percentage` from the `.json`, with the
  model's `context_window_tokens` as the window, when both are present.

### History

The session modal's turns come from the event log: `Prompt` text as the
user, `AssistantMessage` text as the assistant, each `toolUse` as a tool
line.

### Replies and controls

Replies are typed into the tty, one line, when the session is free (not
Working and not Awaiting a permission), the rule the other foreign agents
use. Capabilities: Compact (`/compact`), model switch (`/model <id>`),
effort switch (`/effort <level>`), mode cycle (Shift+Tab), `/` lines, `!`
lines, and Close: every agent takes `/exit`, so Close types it, waits for
the agent's process to end and then closes the terminal, for Kiro and the
other four alike (until now Close was Claude Code only). `/rename` follows
the shared rename path.

### New session

- Agent list: Kiro is listed when `kiro-cli` is found (PATH, then the
  usual folders, then the login shell, as for every agent).
- Model: from `--list-models -f json`, `model_id` values, "Default" first.
- Effort: `low`, `medium`, `high`, `xhigh`, `max`.
- Mode: `default` (passes nothing) or `trust-all` (`-a`).
- Name: no flag; a pending name, typed as `/rename <name>` when the
  session is free, as for Codex and Antigravity. Matching is by folder
  and launch time.
- Shell line: `cd <dir> && p=$(cat <file>) && rm <file> && kiro-cli
  chat <flags> -- "$p"`, the prompt file and quoting as today. `--`
  ends the options so a prompt starting with `-` is still a prompt (to
  be verified against clap's handling in the implementation; if `--`
  is not accepted, the prompt is passed through stdin instead).

### Resume

Lists `<sessions>/*.json` whose `cwd` is the chosen folder, newest
`updated_at` first, with the title (else the first prompt from the
`.jsonl`, else the id) and a running mark when the session's lock pid is
alive. Resume runs `kiro-cli chat --resume-id <id>` in the folder.

### Maya's brain

The interpreter and the classifier run `kiro-cli chat --no-interactive
--trust-tools= --output-format stream-json [--model <agentModel>]
"<system>\n\n<user>"` in Maya's data folder, with the cleared environment
and the 25 s timeout as today. The reply is `finalText` of the
`runFinished` line; a run whose `status` is not `success`, or with no
`runFinished` line, is an error naming the last `type` seen. The first
JSON object in the reply is the interpreter's answer, as for every agent.

### Reviews

The Review button and the voice `review` action start Kiro with the
review prompt the way New session does, named `review <repo> #<number>`
through a pending name.

### Other machines

The harness value `kiro` travels in the existing fields of cards, the
board's agent list, Start, Resume and the resumable-sessions request. No
new messages.

A main on an older release cannot decode a card whose harness it does
not know, and keeps that machine's last board with an "unreadable" note.
From this release on, an unknown harness decodes to `Harness::Other`,
shown as a text badge with no controls, so later agents never freeze a
board again. Both machines need this release to show Kiro cards; the
README says so.

### Windows and Linux

The same lock, session and marker files, under Kiro's data folders on
each platform. The tty is the parent chain's console on Windows
(`console:<pid>` of the first ancestor with a console) and the parent
chain's tty on Linux, where a session inside tmux keeps today's rules.
Both are verified on the laptop and the VM before release; a platform
that cannot be verified ships with the macOS rules and a note.

## Code

- `core/src/model.rs`: `Harness::Kiro` (wire name `kiro`) and
  `Harness::Other` with `#[serde(other)]`, the fallback for unknown
  wire names. `Other` has no binary, no capabilities and no listing.
- `core/src/kiro.rs` (new): `locks(dir)`, `session_meta(json)`,
  `markers(run_dir)`, `working_for(tui_pid, markers, now)`,
  `parse_events(jsonl) -> ForeignTail` (last text, pending tool call,
  last prompt time), `parse_turns`, `first_prompt`, `resume listing`.
- `core/src/foreign.rs`: `kiro_sessions(sessions_dir, run_dir, alive,
  parents)` beside `grok_sessions`; `tail_for` and `turns_for` arms. A
  `parents()` helper returns `(pid, ppid, tty)` for every process on
  macOS and Linux; Windows uses `win_process::list` with parent pids
  (added) and `win_console::console_key`.
- `core/src/store.rs`: `kiro_dir` and `kiro_run_dir`; `refresh_foreign`
  gathers Kiro sessions after Grok's; test builders take the folders.
- `core/src/launch.rs`: `binary_name`, `label`, `efforts`, `modes`,
  `capabilities`, `flags` (`--model`, `--effort`, `-a` for `trust-all`),
  `session_command` (`chat -- "$p"`), `oneshot_args`, `final_text`
  arms for Kiro; `Other` arms that refuse.
- `core/src/agents.rs`: `parse_kiro(json)`, listing args
  `chat --list-models -f json`, Kiro in `build`'s loop.
- `core/src/resume.rs`: `kiro_sessions(dir, folder, running)`;
  `resume_command` arm `kiro-cli chat --resume-id <id>`; `AgentDirs.kiro`.
- `core/src/pending_names.rs`: nothing new; Kiro matches by folder and
  launch time like Codex.
- `cli/src/args.rs`, `cli/src/commands.rs`: `kiro` in `--agent`.
- `src/types.ts`, `src/harness.ts`: `kiro` and `other`; Kiro's label,
  icon (`src/assets/icons/kiro.png`) and capabilities; `other` gets the
  text badge and no capabilities. `src/firstrun.ts` and
  `src/settings.ts` name five agents.
- `src/harness.test.ts`: the unknown-harness stand-in becomes
  `"nonesuch"`.
- `README.md`, `docs/DEVELOPING.md`: Kiro beside the others; the
  approval limitation; the older-main note.
- Fixtures in `core/fixtures/kiro/`: `models.json`, `session.json`,
  `events.jsonl` (a turn with a tool call and one still pending),
  `lock.json`, `marker.json`, `oneshot.jsonl`, all saved from this Mac.

## Testing

- Rust: lock and marker parsing; the tty and TUI pid from a parent
  chain, with no-tty chains skipped; Working from a marker and ignored
  when stale; Completed, Awaiting (prose) and the tool-call snippet from
  the events fixture; context from the session file; the resume listing
  with titles, first-prompt fallback and the running mark; the shell
  line for each flag and for a prompt starting with `-`; the one-shot
  final text and its failure; `Harness::Other` decoding from an unknown
  wire name and refusing every action; the model listing.
- vitest: five agents in the first-run and Settings lists; Kiro's
  capability row; the `other` badge.
- Real app (debug binary), with the user's go-ahead: the running Kiro
  session on the board with its state and snippet; a reply typed into
  it; rename from the session modal; a named New session; Resume; Kiro
  as Maya's agent for one voice command and one "Let Maya choose".

## Out of scope

- OpenCode (its own design).
- Detecting a Kiro tool approval.
- Kiro's V3 engine's session dashboard and cloud sessions.

## Release

A `feat`: bump the minor version from `main`'s version (0.10.0 → 0.11.0
unless `main` moves), in its own `chore(release)` commit.
