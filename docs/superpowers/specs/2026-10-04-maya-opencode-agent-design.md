# OpenCode as a Maya agent — design

Date: 2026-10-04
Status: approved
Builds on: `2026-10-03-maya-kiro-agent-design.md` (the fifth agent, the
`Other` fallback, the `close` capability) and
`2026-10-03-maya-agent-design.md` (Maya's agent, one-shots, resume for
every agent, session controls by capability).

## Purpose

OpenCode becomes the sixth agent Maya knows, with everything the others
have: live sessions on the board with their state and replies, New session
with model, effort, mode and name, Resume, rename, the session controls,
Maya's brain (the voice interpreter and the folder classifier), pull
request reviews, other machines, and Windows and Linux.

OpenCode has a different shape from the five agents before it. One
background server per user owns every session, permission and tool run;
the terminal UI is only a client, one window can show several sessions as
tabs, and nothing on disk maps a process to a session. So Maya talks to
that server over HTTP instead of reading files and typing into a terminal.

Decisions taken during design (2026-10-03, with the user):

- Through the server, not process-plus-tty. The tty approach cannot tell
  which session a window shows, never sees a permission prompt, and would
  type replies into whichever tab is in front.
- Replies, approvals, answers, rename, compact, model and effort switches,
  the mode cycle, `/` lines and `!` lines all go through the server.
  Nothing is typed into OpenCode's terminal except `/exit`, for Close.
- New sessions are created on the server first, then opened in a
  terminal, so the name, model and effort are set at creation and no
  pending name is needed.
- An HTTP client crate (`ureq`, blocking) joins the workspace: nothing in
  it speaks HTTP today.
- OpenCode's own events stream (`GET /api/event`, server-sent events) is
  not used in this release; the store polls the server on its refresh, as
  it reads files for the other agents. A push subscription can follow.

## Findings (verified on this Mac, OpenCode 2.0.22, 2026-10-03)

Binary: `~/.opencode/bin/opencode` (the official installer's location; not
on Maya's binary search path today). `opencode --version` prints
`opencode v2.0.22`.

The server:

| Fact | Value |
|---|---|
| state file | `~/.local/state/opencode/service.json`: `{"id", "version", "url": "http://127.0.0.1:<port>", "pid", "password"}` |
| auth | HTTP basic, user `opencode`, the file's password; no auth and bearer get 401 |
| process | `opencode serve --service`, started by any OpenCode client, or by `opencode service start` |
| spec | `GET /openapi.json` |

Routes Maya uses (all under the state file's url):

| Purpose | Route |
|---|---|
| sessions, newest first | `GET /api/session?limit=N` → `data[]` of `Session.Info`: `id`, `parentID`, `title`, `agent`, `model {providerID, id, variant}`, `tokens {input, output, reasoning, cache {read, write}}`, `time {created, updated, idle, viewed, archived}`, `location {directory}` |
| running sessions | `GET /api/session/active` → `data {"<id>": {"type": "running"}}` |
| pending permissions | `GET /api/session/{id}/permission` → `data[]` of `{id "per…", sessionID, action, resources[], message?}` |
| pending questions | `GET /api/session/{id}/form` → `data[]` of `{id "frm_…", title, fields[] {key, type, title, options[] {value, label}, …}, state {status}}` |
| messages | `GET /api/session/{id}/message?limit=N&order=desc` → `data[]` of `{id, type: user|assistant|idle|…, text?, content[] {type: text|reasoning|tool, text?}}` |
| prompt | `POST /api/session/{id}/prompt {text}` |
| permission reply | `POST /api/session/{id}/permission/{rid}/reply {decision: once|always|reject}` |
| question reply | `POST /api/session/{id}/form/{fid}/reply {answer: {<key>: <value>}}` |
| rename | `PATCH /api/session/{id} {title}` |
| compact | `POST /api/session/{id}/compact` |
| model | `POST /api/session/{id}/model {model: {providerID, id, variant?}}` |
| agent (mode) | `POST /api/session/{id}/agent {agent}`; primary agents from `GET /api/agent` (`build`, `plan`) |
| slash line | `POST /api/session/{id}/command {name, text}` |
| shell line | `POST /api/session/{id}/shell {command}` |
| create | `POST /api/session {title?, agent?, model?, location: {directory}}` → `data.id` |
| models | `GET /api/model` → `data[]` of `{id, providerID, modelID, name, variants[] {id}, limit {context, output}, status, enabled}` |

The pending lists without a session (`/api/permission/request`,
`/api/form`) are scoped to a default directory; the per-session routes are
not.

The terminal UI:

| Purpose | Command |
|---|---|
| open a session | `opencode --session <id> [dir]`; `--auto` auto-approves permissions not denied by a rule |
| one-shot | `opencode run --format json [-m provider/model[#variant]] --title <t> "<prompt>"`, JSON lines; the reply is the `text` of every `{"type":"text","part":{"text":…}}` line joined |
| listing | `opencode api GET /api/model` prints the models JSON (starts the server when it is down) |

`ps` shows a window's `opencode` process on its tty with the folder it
was opened in as its working directory; `--model` and `--agent` are not
flags of the terminal UI.

## Behaviour

### The server

Every store refresh reads the state file. With no file, or a pid that is
not alive, OpenCode has no sessions on the board, and nothing is reported:
the user simply has no OpenCode running. The password is read from the
file for each request and never kept, logged or sent anywhere else.

### Which sessions are cards

On each refresh Maya lists the newest 50 root sessions (no `parentID`:
children are subagent runs), the running ones, and the pending
permissions and questions of each candidate. A session is a card while:

- it is running, or has a pending permission or question; else
- its last activity (the latest of `time.updated`, `time.idle` and
  `time.viewed`) is within the completed timeout: **Completed**; else
- an `opencode` terminal process is alive whose working directory is the
  session's folder: **Idle**, as a session whose terminal is still open.

Otherwise it is not on the board. Sessions whose folder is Maya's data
folder are skipped: those are the brain's one-shots. The card's pid is the
terminal process for its folder when one exists (the newest), else the
server's pid, so Focus and Terminal have something to raise.

### State

- Running: **Working** since `time.updated`.
- A pending permission: **Awaiting**, kind Question, detail
  `<action>: <first resource>` (`edit: /tmp/x`), one question with the
  choices **Once**, **Always** and **Reject**. The first pending request
  is shown; the next appears after it is answered.
- A pending question (a form): **Awaiting**, kind Question, detail the
  form's title; one Maya question per field, with the field's options as
  choices when it has them (a field without options is answered as text
  from the composer).
- Otherwise **Completed** decaying to **Idle** by the timeout.
- Snippet: the latest assistant message's text parts, trimmed as every
  snippet is.
- Context: the session's `tokens.input + tokens.cache.read + tokens.output`
  against the model's `limit.context` from the model list (cached with
  the agent listing).
- History: the last 60 messages in order: user text as the user, assistant
  text parts as the assistant, tool parts as `<tool>: <command or path>`.

### Replies and controls

A per-card channel decides how an action reaches a session: Claude Code's
inbox, typing into the tty for the file agents, or the server for
OpenCode. For OpenCode:

| Action | Call |
|---|---|
| reply | prompt |
| answer a permission choice | permission reply with `once`, `always` or `reject` |
| answer a form | form reply with the chosen option's value, or the composer's text, under the field's key |
| rename | `PATCH` title |
| Compact | compact |
| `/model <provider/model>` | model switch, keeping the session's variant |
| `/effort <variant>` | model switch with the session's model and the chosen variant |
| mode cycle | agent switch to the next primary agent after the session's (`build` → `plan` → `build`) |
| `/name text` line | command `{name, text}` |
| `!line` | shell `{command}` |
| Close | types `/exit` into the newest `opencode` terminal process for the session's folder, then `exit` once it has gone, as for every agent; refused when no such process exists |

The free rule for OpenCode: a reply or a control is accepted whenever the
session is not Working; the server queues nothing while a turn runs, so
Working is refused with "Wait until the session is free."

Capabilities: compact, model switch, effort switch, mode cycle, `/` lines,
`!` lines, close.

### New session

- Agent list: OpenCode is listed when `opencode` is found. The binary
  search gains `~/.opencode/bin` (and `%USERPROFILE%\.opencode\bin` on
  Windows).
- Model: the model list's enabled models, id `provider/model`, label the
  model's name; a model id for OpenCode may contain one `/` (it never
  reaches a shell line: the only command line it goes to is the one-shot's
  argument list).
- Effort: the chosen model's variants (`low`, `high`, `max` for the
  example model); with "Default", the variants every listed model has.
- Mode: `default` or `auto` (`--auto`).
- Name: set at creation.
- Starting: Maya creates the session (`POST /api/session` with the title,
  model, variant and folder), posts the prompt, then opens a terminal on
  `cd <dir> && opencode --session '<id>' [--auto]`. The agent loop starts
  on the server at once; the window opens on a session already working.
  When the server is down Maya runs `opencode service start` and waits
  up to ten seconds for the state file, failing with "OpenCode's server
  did not start." otherwise.
- The prompt is sent as JSON, so no prompt file and no quoting rules;
  the only shell line carries the folder and the id, both quoted.

### Resume

The server's root sessions whose folder is the chosen folder, newest
first, titled by `title`, running marked from the active list. Resume runs
`cd <dir> && opencode --session '<id>'`. With the server down, the list is
empty rather than an error.

### Maya's brain

The interpreter and the classifier run `opencode run --format json
[-m <model>[#<variant>]] --title "Maya" "<system>\n\n<user>"` in Maya's
data folder, with the cleared environment and the 25 s timeout as today.
The reply is the `text` of every `text` event joined; no `text` event is
an error naming the last event type. The session the run leaves behind
sits in Maya's data folder, which the board skips.

### Reviews

As New session with the review prompt, named `review <repo> #<number>` at
creation.

### Other machines

The harness value `opencode` travels in the existing fields; a main on
0.11.0 or later shows the card, an older one shows "Unknown agent" (the
`Other` fallback) or the unreadable-board note before that. Commands
route to the owning machine as today, which holds the server.

### Windows and Linux

The state file is `$XDG_STATE_HOME/opencode/service.json` on Linux
(default `~/.local/state`). Its Windows location is unverified; Maya
tries `%LOCALAPPDATA%\opencode\state\service.json` and
`%USERPROFILE%\.local\state\opencode\service.json`, and the README says
so until the laptop confirms one. The terminal process for Close and
Focus is `opencode.exe` on Windows, found with its console as for the
other agents.

## Code

- `Cargo.toml`, `core/Cargo.toml`: `ureq` (blocking, JSON feature).
- `core/src/model.rs`: `Harness::OpenCode` (wire name `opencode`).
- `core/src/opencode.rs` (new): `service(state_dir) -> Option<Service
  {url, pid, password}>`, a `Client` over `ureq` with `get`/`post`/`patch`
  taking and returning `serde_json::Value`, the parsers (`sessions`,
  `active`, `permissions`, `forms`, `messages`, `models`), `card_for(…)`
  applying the rules above, and the actions (`prompt`, `reply_permission`,
  `reply_form`, `rename`, `compact`, `switch_model`, `switch_agent`,
  `command`, `shell`, `create_session`). Tests run against a fake server
  on a local port replaying saved real replies.
- `core/src/store.rs`: `opencode_state_dir`, the per-refresh fetch, the
  `opencode` terminal processes (from the process tree, their folder from
  `lsof` on macOS and Linux as for Codex; on Windows no process's folder
  is readable, so there the card's pid is the server's, Focus raises
  nothing better and Close is refused, which the README notes), and the
  cards merged with the others.
- `core/src/actions.rs`: `Channel { Inbox, Tty(String), Server(Service,
  session_id) }` chosen per card; every action dispatches on it.
- `core/src/launch.rs`: the table rows; `find_binary` candidates;
  `oneshot_args` and `final_text`; a `plain_model_ref` for `provider/model`.
- `core/src/agents.rs`: `parse_opencode(json)` with variants as per-model
  efforts; listing args `api GET /api/model`.
- `core/src/resume.rs`: the OpenCode listing through the client and the
  resume line.
- `src/harness.ts`, `src/types.ts`, icon `src/assets/icons/opencode.svg`
  or `.png`, `firstrun.ts` and `settings.ts` copy ("… or OpenCode").
- `README.md`, `docs/DEVELOPING.md`.

## Testing

- Rust: the state file parsed and a dead pid ignored; each parser from
  saved replies; the card rules (running, permission, form, completed,
  idle with a window, dropped without one, child and data-folder sessions
  skipped); context from tokens and the model limit; every action's
  request path and body against the fake server; New session's create,
  prompt and shell line; the server started on demand and the timeout;
  the resume listing; the one-shot args and final text; the listing
  parser; the capability row.
- vitest: six agents in the lists; the capability row; the permission
  choices rendered as a question.
- Real app (debug binary): with the user starting the sessions: an
  OpenCode session on the board with its state, a permission answered
  from the card, a reply, a rename, a named New session with a model and
  effort, Resume, Close, and OpenCode as Maya's agent for one voice
  command.

## Out of scope

- The server's event stream (push instead of polling).
- Standalone or remote servers (`--standalone`, `--server`); only the
  shared background service is found.
- Session tabs: Focus raises the window for the folder, not the tab.

## Release

A `feat`: bump the minor version from `main`'s version (0.11.0 → 0.12.0
once the Kiro pull request has merged), in its own `chore(release)`
commit.
