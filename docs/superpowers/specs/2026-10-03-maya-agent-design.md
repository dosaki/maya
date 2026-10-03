# Choose the agent that powers Maya — design

Date: 2026-10-03
Status: implemented
Builds on: `2026-10-02-maya-choose-agent-design.md` (the agent list, the
per-agent launch flags and pending names) and
`2026-09-29-maya-voice-assistant-design.md` (the interpreter).

## Purpose

Maya tracks Claude Code, Codex, Antigravity and Grok Build sessions, but
everything that thinks for her runs on the `claude` command: the voice
interpreter and the folder classifier are one-shot `claude -p` calls, and
the Review button starts a `claude` session. Someone who uses one of the
other agents cannot run Maya without also installing Claude Code.

After this change the user picks **Maya's agent** once, on first start, and
can change it in Settings. That agent interprets voice commands, picks the
folder for "Let Maya choose", and is the default for new sessions, resumed
sessions and pull request reviews. Resume works for every agent, not only
Claude Code. The session controls that were Claude Code only (`/compact`,
model and effort changes, the mode cycle, `/` lines) are offered for every
agent that has them. The Review button no longer depends on a skill: its
prompt is configurable, with a built-in default.

Decisions taken during design:

- One setting, not two: Maya's agent is both her brain and the default
  for sessions. A session's agent can still be chosen per session in the
  New session and Resume modals.
- The interpreter sends the same prompts to every agent and reads the
  first JSON object in the reply, as it does today. Each agent's
  structured-output flag differs too much to rely on.
- Session controls follow a per-agent capability table, not the brain.
  Close stays Claude Code only.
- Maya's data folder stays `~/.claude/maya`.

## Findings (verified on this Mac, 2026-10-03)

Every agent has a one-shot mode:

| Agent | One-shot | Model | Output | No tools / sandbox |
|---|---|---|---|---|
| Claude Code | `claude -p` | `--model` | `--output-format json`, `--system-prompt` | `--tools ""`, `--max-turns 1` |
| Codex | `codex exec --ephemeral --skip-git-repo-check` | `-m` | `--json` (the last `agent_message` item on stdout) | `-s read-only` |
| Antigravity | `agy -p=<prompt>` (the prompt attached to the flag; other flags before it) | `--model` | `--output-format json` | `--sandbox` |
| Grok Build | `grok -p` | `-m` | `--output-format json`, `--system-prompt-override` | `--tools ""`, `--max-turns 1` |

Codex and Antigravity have no system-prompt flag; the system text is
folded into the prompt for them.

Every agent resumes by id, and keeps its past sessions where Maya already
reads for the board:

| Agent | Past sessions | Resume |
|---|---|---|
| Claude Code | `~/.claude/projects/<folder>/` transcripts | `claude --resume <id>` |
| Codex | `~/.codex/session_index.jsonl` (id, name, updated) and each rollout's `session_meta.cwd` | `codex resume <id>` |
| Antigravity | `~/.gemini/antigravity-cli/history.jsonl` (conversationId, workspace, display, timestamp) | `agy --conversation <id>` |
| Grok Build | `~/.grok/sessions/<percent-encoded folder>/<id>/summary.json` | `grok -r <id>` |

Session controls, from each agent's docs and binary:

| Agent | /compact | /model, /effort | Mode cycle | `/` lines | `!` lines |
|---|---|---|---|---|---|
| Claude Code | yes | yes | Shift+Tab | yes | yes |
| Codex | yes | no | no | yes | no |
| Antigravity | yes | no | Shift+Tab (default, accept-edits, plan) | yes | no |
| Grok Build | yes | `/model` only | Shift+Tab (Normal, Plan, Always-approve) | yes | no |

The Codex, Antigravity and Grok Build cells that the probe would have
settled are at their safe defaults until it runs.

## Behaviour

### Maya's agent

- Config gains `agent` (a harness: `claude-code`, `codex`, `antigravity`,
  `grok`) and `agentModel` (a model id of that agent, or empty for the
  agent's default). `interpreterModel` is dropped; an existing value is
  carried into `agentModel` when `agent` is Claude Code.
- Settings gets a **Maya** section at the top:
  - **Agent**: the agents installed on this machine, from the same
    listing New session uses. Hint: "Interprets your voice commands,
    picks folders for 'Let Maya choose', and is the default for new and
    resumed sessions and reviews."
  - **Model**: "Default" first, then that agent's models. Rebuilt when
    the agent changes. A remembered model the agent no longer lists falls
    back to "Default".
- The "Voice interpreter" select in the Voice assistant section goes
  away.

### First start

- When the config has no `agent` (a fresh install, or the first start
  after this release), a modal opens over the board: "Which agent should
  power Maya?", one row per installed agent with its icon and label, the
  first preselected, and a **Continue** button that saves `agent` and
  closes the modal. It cannot be dismissed otherwise.
- With none of the four installed, the modal says so, names the four,
  and its button reads **Continue with Claude Code**. Maya runs on Claude
  Code until Settings says otherwise, failing at use time as today
  ("I can't find the claude command.").

### Thinking

- The interpreter and the folder classifier run on Maya's agent and model
  with a per-agent one-shot command (see Findings). Working directory,
  cleared environment, 25 s timeout and the kill at timeout are unchanged.
- The system and user prompts are the same for every agent. For Codex and
  Antigravity the system prompt goes first in the prompt text, separated
  by a blank line.
- The reply is the agent's final text: Claude's `result`, the text of
  Codex's last `agent_message` item, Antigravity's `response`, Grok's
  `text`. The first JSON object in it is the reply, as today. Validation
  against the board is unchanged.
- The classifier keeps its own prompt and text output, run the same way,
  with the agent's default model when `agentModel` is empty.
- When the agent's binary is missing, Maya says "I can't find the
  <binary> command."
- "Let Claude choose" becomes "Let Maya choose", and "(chosen by Claude)"
  becomes "(chosen by Maya)".

### New session and Resume

- New session's Agent field defaults to Maya's agent when the machine has
  it installed, else to the first agent listed. The per-agent remembered
  options stay.
- Resume gains an **Agent** field under Machine, shown when the machine
  has more than one agent, defaulting the same way. The session list is
  that agent's past sessions in the chosen folder, newest first, each with
  title, last activity, and a "running" mark when a live session has that
  id. Titles: Claude as today; Codex the latest `thread_name` for the id,
  else the first prompt; Antigravity the latest `/rename` for the
  conversation, else its first prompt; Grok the title in `summary.json`.
- Resume runs the agent's resume line (see Findings) in the folder, in a
  new terminal, as Claude's resume does today.

### Session controls by capability

- A capability table per agent (compact, model switch, effort switch,
  mode cycle and its key or command, `/` lines, `!` lines) in
  `core/src/launch.rs` replaces the "only available for Claude Code
  sessions" check. A control the agent lacks is not shown in the session
  modal; a request for one fails with "<Agent> has no <control>."
- Model and effort lists for the session modal's selects come from the
  agent listing, as New session's do. The `/model` and `/effort` values
  are checked against them before they are typed.
- The free check stays as it is per agent: Claude Code queues what is
  typed while it works; the others must be Idle or Completed.
- Close stays Claude Code only.

### Pull request reviews

- The Review button and the voice `review` action start the review
  session with Maya's agent, named `review <repo> #<number>`: with `-n`
  for Claude Code, through a pending name for the others.
- Settings' Pull requests part gains a **Review prompt** text area.
  Empty means the built-in prompt:

  > Review pull request #{number} of {repo} ({url}). Make use of code
  > review skills, and at the end give at most 3 options: Approve; Ask
  > <questions here>; Request changes <changes here>. Mark the recommended
  > option "(Recommended)". Show Ask and Request changes only when they
  > are needed.

- A custom prompt may use `{number}`, `{repo}` and `{url}`. When it uses
  none, Maya appends ` PR #<number> (<url>)`, so `/should-i-approve` alone
  still names the pull request.
- The prompt is passed to the agent the way New session passes a prompt
  (a prompt file read into `$p`), so quoting and `-` prefixes are handled
  the same way.

### Other machines

- `Resume` over the network carries the agent; listing resumable
  sessions carries it too. A board without `agents` (an older Maya)
  offers Claude Code only in both modals.
- Reviews start on the main machine, as today; Maya's agent and the
  review prompt are its settings.

## Code

- `core/src/config.rs`: `agent: Option<Harness>`, `agent_model: String`,
  `review_prompt: String`; migration from `interpreter_model`.
- `core/src/launch.rs`: `oneshot(agent, model, system, user) -> Command`,
  `resume_command(agent, dir, id)`, the `Capabilities` table and
  `capabilities(agent)`.
- `core/src/interpreter.rs`: `run` takes the agent; `parse_reply` takes
  the agent's final text; `classify` moves next to it and takes the agent.
- `core/src/resume.rs`: `list_sessions(agent, dir, running_ids)` with the
  three new listings, reusing `codex::thread_name`,
  `antigravity::conversation_name` and `grok::title`.
- `core/src/reviews.rs`: `shell_command(agent, target, pr, prompt)` and
  `render_prompt(template, pr)`.
- `core/src/actions.rs`: the capability check in `type_into_session`;
  `resume_session` and `start_review` take the agent; the review session
  gets a pending name for the other agents.
- `core/src/net/protocol.rs`, `merge.rs`: agent on `Resume` and on the
  resumable-sessions request; agent and prompt on the review start.
- `src-tauri/src/lib.rs`: `list_resumable_sessions` and `resume_session`
  take the agent; `review_pr` reads the agent and prompt from config.
- `src-tauri/src/listener.rs`: the interpreter call takes the agent.
- `src/settings.ts`: the Maya section and the Review prompt field.
- `src/firstrun.ts` (new): the first-start modal, opened from `main.ts`
  when `get_config` has no `agent`.
- `src/resume.ts`: the Agent field. `src/modal.ts`: controls gated on
  the agent's capabilities, mirrored in `src/harness.ts` from the Rust
  table.
- `src/newsession.ts`: default agent, "Let Maya choose" copy.
- `README.md`: requirements say one of the four agents, installed and
  signed in.

## Testing

- Rust: each agent's one-shot command line; `parse_reply` from saved real
  output of each agent; each agent's resume listing from fixture stores,
  with titles and the running mark; each resume line's quoting; the
  capability table and the refusal message; review prompt rendering with
  placeholders, with none, and the default; config migration from
  `interpreterModel`; agent on the network messages.
- vitest: the first-start modal opens once and saves; the Maya section's
  model list follows the agent and falls back to Default; Resume's Agent
  field and its default; the session modal hides controls the agent
  lacks; the Review prompt field.
- Real app (debug binary), with the user's go-ahead: pick Codex as Maya's
  agent, say "Maya, what's waiting on me?", start a Codex session, resume
  it, review a pull request with the default prompt, then switch back.

## Out of scope

- Close for the other agents.
- Moving Maya's data folder out of `~/.claude/maya`.
- A per-session agent other than the four Maya already watches.

## Release

A `feat`: bump the minor version from `main`'s version (0.9.0 → 0.10.0
unless `main` moves), in its own `chore(release)` commit.
