# Eye: Claude Code session board — design

Date: 2026-09-28
Status: approved design, awaiting implementation plan

## Purpose

A macOS desktop app that shows every Claude Code session running on this
machine as a card on a kanban board with four columns: **Idle**, **Working**,
**Completed**, **Awaiting Decision**. The user runs many sessions at once
(25 live at the time of writing) in Apple Terminal tabs and needs to see at a
glance which ones need attention, then jump to the right tab.

Success looks like: open the app, every live session is a card in the right
column within a couple of seconds of a state change, and clicking a card
brings its Terminal tab to the front.

## Decisions taken during brainstorming

| Question | Decision |
|---|---|
| Meaning of Completed | Claude finished its turn and the user has not typed since. A "needs my review" column. Decays to Idle after a timeout. Exited sessions are simply removed. |
| Card interaction | Click jumps to the session's Terminal tab. Card shows the last assistant message, or the open question / permission. No replying from the board. |
| Stack | Tauri 2, Rust core, plain TypeScript frontend, no UI framework. |
| Precision of state | The app installs a Claude Code hook. Registry-only inference is the fallback until the hook has fired. |

## Data sources (all local, under `~/.claude/`)

### Session registry: `~/.claude/sessions/<pid>.json`

Maintained by Claude Code itself. Fields used:

- `pid`, `sessionId`, `cwd`, `name`, `kind`, `startedAt`
- `status`: observed values `busy`, `idle`, `shell`
- `statusUpdatedAt`, `updatedAt`

Files are removed when the session exits. A file whose `pid` is not alive
(`kill -0` fails) is treated as gone. The transcript path is derived as
`~/.claude/projects/<cwd with / replaced by ->/<sessionId>.jsonl`.

### Hook event log: `~/.claude/eye/events.jsonl`

Written by the hook the app installs (see below). One JSON object per line,
exactly the hook's stdin payload, plus a `received_at` epoch-millis field
added by the hook command. Fields used: `session_id`, `hook_event_name`,
`tool_name` (tool events), `notification_type` (Notification events),
`transcript_path`, `cwd`.

### Transcript tail: `<transcript_path>`

Read only the last N kilobytes. Used for the card snippet and for the
fallback question detection. Relevant line shapes:

- `{"type":"assistant","message":{"content":[{"type":"text","text":...}]}}`
- `{"type":"assistant","message":{"content":[{"type":"tool_use","name":"AskUserQuestion","input":{...}}]}}`
- `{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":...}]}}`

An AskUserQuestion or ExitPlanMode `tool_use` with no later `tool_result`
for its id means a question is open.

## State machine

Evaluated per session every time any input changes. First match wins.

1. **Awaiting Decision** — the most recent "gating" hook event for the
   session is one of:
   - `PermissionRequest`
   - `PreToolUse` with `tool_name` in {`AskUserQuestion`, `ExitPlanMode`}
   - `Notification` with `notification_type` = `permission_prompt` (belt and braces)

   and no clearing event has arrived after it. Clearing events:
   `PostToolUse`, `PostToolUseFailure`, `PermissionDenied`, `Stop`,
   `UserPromptSubmit`.

   Fallback when no hook events exist for the session: transcript tail shows
   an unanswered AskUserQuestion / ExitPlanMode tool_use.

2. **Working** — registry `status` is `busy` or `shell`.

3. **Completed** — the most recent of {`Stop`, `UserPromptSubmit`} for the
   session is `Stop`, and it is younger than the Completed timeout
   (default 30 minutes, configurable in settings).

4. **Idle** — everything else.

Time-in-state shown on the card is measured from the timestamp of the event
or registry change that put the session into its current state.

## Hook

Installed into `~/.claude/settings.json` by the app. Events registered:
`PermissionRequest`, `PreToolUse` (matcher `AskUserQuestion|ExitPlanMode`),
`PostToolUse`, `PostToolUseFailure`, `PermissionDenied`, `Stop`,
`UserPromptSubmit`, `SessionEnd`, `Notification` (matcher `permission_prompt`).

Every entry uses the same command so it can be identified and removed:

```
sh -c 'mkdir -p ~/.claude/eye && jq -c --arg t "$(date +%s000)" ". + {received_at: (\$t|tonumber)}" >> ~/.claude/eye/events.jsonl'
```

The command reads stdin, never blocks, always exits 0, and emits no output,
so it cannot alter Claude Code behaviour. Before writing, the app copies
`settings.json` to `settings.json.eye-backup-<timestamp>`. Uninstall removes
only hook entries whose command contains `~/.claude/eye/events.jsonl`.

The settings screen shows installed / not installed, with Install and Remove
buttons.

## Architecture

```
~/.claude/sessions/*.json ──┐
~/.claude/eye/events.jsonl ─┼─► Rust core: watcher → store → state machine ─► Tauri event "sessions"
transcripts (tail) ─────────┘                                                         │
                                                                                      ▼
                                                                    Web view: kanban board (TS)
                                                              click card ──► invoke "focus_session"
                                                                                      │
                                                                                      ▼
                                                                   Rust: pid → tty → osascript
```

### Rust core (`src-tauri/src/`)

- `registry.rs` — parse one registry file into a `RegistrySession`; list the
  directory; liveness check via `kill(pid, 0)`.
- `events.rs` — parse `events.jsonl` lines into `HookEvent`; keep per-session
  vectors in memory; incremental read from the last offset.
- `transcript.rs` — tail a transcript, return `last_assistant_text`,
  `open_question: Option<String>`.
- `state.rs` — pure function `derive(registry, events, transcript, now, config) -> Card`.
  No I/O. This is the unit-tested heart.
- `watcher.rs` — `notify` crate watching the sessions dir and the events
  file, debounced 200 ms, plus a 5 s tick for liveness and timeout decay.
- `focus.rs` — `ps -o tty= -p <pid>` then `osascript` telling Terminal to
  select the tab whose `tty` matches and `activate`.
- `hook_install.rs` — read / merge / write `settings.json` with backup.
- `main.rs` — Tauri setup, commands `list_sessions`, `focus_session`,
  `hook_status`, `install_hook`, `remove_hook`, `get_config`, `set_config`;
  emits `sessions` events.

### Frontend (`src/`)

- `main.ts` — subscribes to `sessions`, renders board.
- `board.ts` — pure render: `Card[] -> DOM`. Four columns in fixed order
  Awaiting Decision, Working, Completed, Idle (attention first).
- `card.ts` — one card: name, project folder (last path segment), state age,
  snippet. Awaiting cards show the question or `tool: command` line.
- `settings.ts` — hook status panel and Completed timeout input.
- `styles.css` — light and dark via `prefers-color-scheme`.

### Card payload (Rust → frontend)

```ts
type Card = {
  sessionId: string; pid: number; name: string; cwd: string;
  state: 'awaiting' | 'working' | 'completed' | 'idle';
  stateSince: number;        // epoch millis
  snippet: string;           // last assistant text, trimmed to ~200 chars
  awaiting?: { kind: 'question' | 'plan' | 'permission'; detail: string };
}
```

## Error handling

- Unparseable registry file or event line: log at debug, skip.
- Registry entry with dead pid: exclude.
- Transcript missing or unreadable: card still shown, empty snippet.
- `events.jsonl` compaction on app start: rewrite keeping only lines whose
  `session_id` is in the current registry.
- Focus failures (no tab with that tty, osascript error): non-blocking toast
  in the UI, nothing else.
- Hook install: if `settings.json` is not valid JSON, refuse and show the
  error rather than overwrite.

## Testing

- Rust unit tests for `state::derive` over synthetic sequences covering:
  each state, permission asked then approved, question asked then answered,
  Stop then timeout decay, Stop then new prompt, registry-only fallback.
- Rust tests for `registry`, `events`, `transcript` parsers using fixture
  files copied and anonymised from real data in `src-tauri/fixtures/`.
- Frontend: one test rendering `board.ts` from a fixed `Card[]` and checking
  column membership and ordering (Vitest + jsdom).
- Manual acceptance against live sessions: open the app, trigger a permission
  prompt in one session, see it move to Awaiting Decision, approve, see it
  return to Working; let one finish, see Completed; click a card, Terminal tab
  focused.

## Out of scope for this iteration

Replying from the board, notifications, menu bar mode, other terminal apps,
sessions running on other machines, drag and drop between columns.

## Prerequisites

Rust toolchain via `rustup` (not installed on this machine). Node 26 and
pnpm 11 are present. `jq` is present and used by the hook command.
