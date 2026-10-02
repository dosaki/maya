# Choose the agent and name a new session — design

Date: 2026-10-02
Status: implemented
Builds on: `2026-09-30-maya-remote-assistants-design.md` (the machine
picker and the start command over the network).

## Purpose

The New session modal starts only Claude Code, and the session's name
comes later from the agent. The user wants to:

1. pick the agent for a new session: Claude Code, Codex, Antigravity or
   Grok Build (the agents Maya already watches);
2. give the new session a name;
3. rename a running Codex, Antigravity or Grok session from Maya, which
   today fails with "That command is only available for Claude Code
   sessions."

Decisions taken during design:

- The Model, Effort and Mode row follows the chosen agent. A field the
  agent does not have is not shown.
- Models come from the agent itself, not from a list in Maya.
- A name is passed as a flag when the agent has one (only `claude -n`
  today). For the other agents Maya types `/rename <name>` when the
  session is free. It does not type it before the first prompt: a probe
  showed that keys typed into a TUI that is not at its input box act as
  shortcuts (in Codex 0.160.0 they opened an "Archive task?" dialog).
- It works on every machine. A machine that runs an older Maya offers
  Claude only.

## Behaviour

### The modal

Fields, top to bottom:

- **Machine** (as now).
- **Agent**: the agents installed on the chosen machine, Claude Code
  first. Hidden when Claude Code is the only one.
- **Directory** (as now). "Let Claude choose" uses the Claude classifier
  for every agent.
- **Model / Effort / Mode**: rebuilt when the agent changes; see the table.
  Each has a "Default" entry first, which passes no flag.
- **Name**: optional, one line, at most `answer::MAX_NAME_CHARS`
  characters. Empty means the agent names the session itself.
- **Prompt** (as now).

Maya keeps the last agent and, for each agent, the last options, as it
keeps `lastOptions` today. A remembered model that the agent no longer
lists falls back to "Default".

| Agent | Model | Effort | Mode |
|---|---|---|---|
| Claude Code | `fable`, `opus`, `sonnet`, `haiku` → `--model` | `low`…`max` → `--effort` | the 6 values → `--permission-mode` |
| Codex | `codex debug models`, entries with `visibility: "list"` → `-m` | the chosen model's `supported_reasoning_levels`; with "Default" model, the levels every listed model supports → `-c model_reasoning_effort=<v>` | `read-only`, `workspace-write`, `danger-full-access` → `-s` |
| Antigravity | `agy models` (first column) → `--model` | `low`, `medium`, `high`, `max` → `--effort` | `accept-edits`, `plan` → `--mode` |
| Grok Build | `grok models` (the `Available models` list) → `-m` | not shown (its help lists no values) | `default`, `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions`, `plan` → `--permission-mode` |

While the models load, the Model field shows "Loading…" and only
"Default" can be chosen. If the listing fails, only "Default" is offered.

As built: the models arrive with the agent list, so the Agent field
appears once the listing is in, instead of a "Loading…" Model field.

### Starting the session

The shell line keeps today's shape (`cd`, read the prompt file, delete
it, run the agent) with the agent's binary and flags:

- Claude Code: `claude <flags> [-n '<name>'] -- "$p"`
- Codex: `codex <flags> -- "$p"`
- Antigravity: `agy <flags> -i "$p"`. A prompt that starts with `-` is
  sent as `-i="$p"` instead, which Go's flag parser reads as the value.
- Grok Build: `grok <flags> --session-id <uuid> -- "$p"`

The binary is found the way `claude_binary` finds `claude` today: PATH,
then `~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin`, then the
login shell (on Windows, the `.exe` on PATH and the installer's folder).

Validation, before anything runs:

- effort and mode must be in that agent's fixed list;
- a model must be in that agent's model list and match
  `[A-Za-z0-9._-]+`;
- the name is checked with `answer::rename_command`'s rules and goes into
  the shell line only single-quoted (`shell_single_quote`).

### Naming the other agents

When a Codex, Antigravity or Grok session is started with a name, Maya
keeps a **pending name**: agent, folder, launch time, name, and for Grok
the session id it passed.

- **Matching.** A pending name belongs to the first new session of that
  agent in that folder whose start is after the launch time and that no
  other pending name has claimed. A Grok session matches by its id.
- **Showing.** From the match on, the card shows the pending name.
- **Renaming.** When the session is free, Maya types `/rename <name>`
  into its terminal once.
- **Done.** The pending name is dropped when the agent's own name for the
  session equals it, when the session ends, or 10 minutes after launch if
  no session matched.
- Pending names are kept in memory. If Maya quits before the rename, the
  session keeps the agent's own title.

### Free

A session of another agent is free when its card is Completed or Idle:
not Working and not Awaiting. Claude Code keeps today's rule
(`answer::check_free`), because Claude Code queues what is typed while it
works.

### Renaming from the session modal

`rename_session` types `/rename <name>` into Codex, Antigravity and Grok
sessions too, when they are free. When one is not free it fails with
"Wait until the session is free to rename it." `/model`, `/effort`, slash
commands and Shift+Tab stay Claude Code only.

### Other machines

- Each machine's board already carries its project folders. It also
  carries `agents`: for each installed agent, its harness and its models.
  A board without `agents` (an older Maya) means Claude Code only, and
  the modal hides the Name field for that machine.
- The start command over the network carries the agent and the name. The
  machine that starts the session keeps the pending name and does the
  rename.
- Models are listed when the board is built and cached for 10 minutes,
  so the network never waits on `agy models`.

## Code

- `core/src/launch.rs`: an `Agent` enum (`ClaudeCode`, `Codex`,
  `Antigravity`, `Grok`) with, per agent, the binary name, the option
  lists and how to render flags, the prompt and the name.
  `LaunchOptions` gains `agent` (default Claude Code) and `name`.
  `session_command` takes the agent. A generic `agent_binary(Agent)`
  replaces `claude_binary` (which stays for the classifier).
- `core/src/agents.rs` (new): which agents are installed, and parsing of
  the three model listings.
- `core/src/pending_names.rs` (new): pending names, matching, expiry. The
  store's refresh applies them to cards and reports the sessions ready
  for `/rename`.
- `core/src/actions.rs`: `start_session` records pending names;
  `rename_session` accepts the other agents (one `free` check shared with
  the automatic rename).
- `core/src/net/protocol.rs`, `merge.rs`: `agents` on the board; agent
  and name on `Start`.
- `src-tauri/src/lib.rs`: a `list_agents(machine)` command;
  `start_session` passes agent and name through.
- `src/newsession.ts`: the Agent and Name fields, the per-agent option
  row, remembered choices.

## Testing

- Rust: each agent's shell line (quoting, flags, a prompt that starts
  with `-`, the name); validation rejects unknown values and unsafe model
  names; parsing of the three model listings from saved real output;
  pending-name matching, claiming, expiry and the free rule; rename of a
  foreign session refused while it works.
- vitest: the Agent field lists only installed agents; changing it
  rebuilds the option row and hides missing fields; Codex effort follows
  the model; the Name field; remembered agent and options; an older
  remote offers Claude only.
- Real app (debug binary): with the user's go-ahead, start a "Say hi"
  session with a name for each agent, check the card and the agent's own
  name, then rename one of each from the session modal.

## Release

A `feat`: bump the minor version from `main`'s version (0.7.0 → 0.8.0
unless `main` moves), in its own `chore(release)` commit.
