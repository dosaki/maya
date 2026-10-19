# Resume sessions after a reboot, and a terminal look for the session viewer — design

Date: 2026-10-19
Status: approved design, not yet implemented
Builds on: `2026-10-18-git-integration-design.md` (0.24.0)

## Purpose

Three things ship together as 0.25.0:

1. **Resume after a reboot.** Each Maya remembers the agent sessions
   running on its machine. After the machine restarts, the Sessions tab
   offers to bring back the ones that were running, and "Maya, resume my
   sessions" does the same by voice. The main covers its assistants'
   sessions too.
2. **A terminal look for the session viewer.** The panel that opens on a
   card can look as it does today (chat) or like the agent's own terminal.
3. **README:** the Settings → Git first-run step that 0.24.0 left out.

Success:

- With three sessions open on the Mac and two on the laptop (an
  assistant), the user restarts both machines and opens Maya on the Mac.
  The Sessions tab says "5 sessions were running when Maya last
  stopped · 3 here · 2 on Gnowee". **Resume all** reopens each one, in
  its own folder, in its own terminal, on its own machine, with its own
  conversation. The laptop's two wait until it reconnects.
- A session the user closed themselves while Maya was running is not
  offered.
- "Maya, resume my sessions" reads back the count and resumes on "yes".
- Switching the viewer to Terminal redraws the open panel at once in the
  agent-terminal style; switching back restores today's look.

## Decisions

- **What is offered:** the sessions that were running when the machine
  (or Maya) went down. A session closed by the user while Maya watched is
  forgotten; one cut off by Maya or the machine stopping is remembered.
- **How it is offered:** a banner on the Sessions tab plus a voice command.
  Nothing resumes without the user asking.
- **Machines:** each machine remembers its own sessions; the main's banner
  and voice command cover every connected or expected assistant, and each
  session resumes on its own machine.
- **Detecting a reboot casualty:** a grace window with a heartbeat (below),
  the same on macOS, Windows and Linux. Not OS shutdown notifications (per
  platform, and a crash or power cut sends none) and not a snapshot at
  Maya's exit (misses terminals that quit before Maya).
- **Viewer theme:** B, the agent-terminal look. A setting in Settings →
  Sessions plus a toggle in the viewer header, both saving the same value;
  the phone keeps its own.

## Remembering sessions

### The record

Each machine keeps `~/.claude/maya/remembered.json`:

```json
{
  "heartbeatMs": 1792400000000,
  "sessions": [
    { "agent": "claude-code", "sessionId": "1f0c…", "dir": "/Users/x/dev/calibre",
      "name": "calibre-88", "firstSeenMs": 1792390000000, "lastSeenMs": 1792399990000,
      "goneAtMs": null }
  ]
}
```

- A session is added the first time a refresh sees it live, keyed by
  `(agent, sessionId)`. Its `name`, `dir` and `lastSeenMs` are updated on
  each refresh while it lives.
- Only agents that can resume are kept: Claude Code, Codex, Antigravity,
  Grok, Kiro, OpenCode. `Other` is never recorded. A session id that is not
  `resume::plain_session_id` is never recorded.
- The file is written only when something changed, at most once per
  refresh, readable by its owner only (as `config.json`).
- `heartbeatMs` is rewritten every 30 s while Maya runs.

### Pruning and the grace window

`GRACE_MS = 120_000`.

- **Closed from its card** (the × / `close_session`, local or relayed from
  the main): dropped at once.
- **Vanished while Maya runs:** `goneAtMs` is set. If it comes back (same
  id) it is live again and `goneAtMs` clears. Once `now - goneAtMs >
  GRACE_MS` and Maya has been running since before `goneAtMs` (so the
  heartbeat kept moving), it is dropped: the user closed it.
- **At startup**, before the first refresh, the previous run's file is
  read and each entry classified against the stored `heartbeatMs` (the
  previous run's last sign of life):
  - `goneAtMs` is null (still live when Maya died), or `heartbeatMs -
    goneAtMs <= GRACE_MS` (it vanished in the last two minutes of the
    previous run): **was running**.
  - otherwise: dropped (it had been gone for a while; the user closed it).
- An entry that is live again after startup (the session survived Maya,
  e.g. Maya was just restarted) is simply live; it is not offered.
- **Accepted edge:** the user quits Maya, closes sessions by hand, then
  restarts Maya: those sessions are offered. The banner's Choose list and
  Dismiss handle it.

The pure logic lives in `core/src/remembered.rs`: `observe(&mut self, live:
&[LiveSession], now_ms)`, `closed(&mut self, agent, id)`,
`classify_at_start(&mut self, now_ms) -> Vec<Remembered>`,
`was_running(&self) -> Vec<Remembered>`, `forget(&mut self, keys)`, load
and save. The store calls `observe` from `refresh` and `closed` from the
close path; `src-tauri` runs the heartbeat on the existing PR-badge loop's
thread (it already ticks every 10 s; the heartbeat writes every third).

### The network

- `Up::Board` gains `remembered: Option<Vec<Remembered>>` (`#[serde(default)]`):
  an assistant sends its "was running" list with its board. An older main
  ignores the field; an older assistant sends none.
- The main merges: its own list plus each assistant's, labelled with the
  assistant's display name. An assistant that is paired but not connected
  and was in the main's last-known merged list shows as "waiting for
  <name>" (the main keeps each assistant's last list in memory for this
  run; it does not persist it).
- Resuming on an assistant uses the existing `CommandKind::Resume { dir,
  session, agent }`. Forgetting on an assistant uses a new
  `CommandKind::Forget { sessions: Vec<(Harness, String)> }`; an older
  assistant cannot decode it and ignores it without answering, so the main
  sends it on a thread of its own and logs the timeout.
- The phone, joined to the main, receives the merged list with the board
  it already mirrors and shows the same banner; its buttons send the same
  commands to the main.

## Offering and resuming

### Banner (Sessions tab)

Shown above the board while the merged "was running" list is not empty:

- Text: "N sessions were running when Maya last stopped", with "a here ·
  b on <assistant>" when more than one machine is involved. (Neutral on
  purpose: the list is the same whether the machine restarted or only
  Maya stopped.)
- **Resume all** resumes every entry it can.
- **Choose…** opens a list grouped by machine, every row ticked: name,
  folder (`~`-shortened), "N ago" from `lastSeenMs`, agent badge; a
  waiting assistant's rows say "waiting for <name>" and are not ticked.
  Buttons: **Resume N** (the ticked ones), **Cancel**, **Forget unticked**.
- **Dismiss** forgets the whole list (local and, by `Forget`, on each
  connected assistant).
- After a resume, the banner shows any that failed: "Couldn't resume 1:
  raffinad-3c (folder missing)", with Dismiss.
- An entry resumed by hand (it turns live) leaves the list on its own.

### Resuming

`actions::resume_remembered(l, entries) -> Vec<(key, Result<(), String>)>`:

- For each entry, in `lastSeenMs` order, oldest first (so the most recent
  ends up in front), 400 ms apart:
  - skip it if a live session already has that id;
  - check the folder exists and the agent knows the session (the same
    `resume::list_sessions` check `resume_session` makes, for that `dir`;
    OpenCode through its server listing);
  - open `resume::resume_command(agent, dir, id)` in a terminal, as
    `resume_session` does, with the tmux label on Linux.
- Unlike `resume_session`, the folder need not be under the projects
  directory: it is the remembered `dir`, which Maya itself recorded. It
  must be an absolute path that exists and is a directory.
- Successful entries leave the "was running" list (they are live, then
  `observe` takes over); failures stay with their reason until dismissed.
- On the main, entries for an assistant become `CommandKind::Resume`
  commands to it, one at a time; its results come back as the command's
  answer.

### Voice

A new action `{"kind":"resume_all","scope":"all"|"here"}` for the
interpreter, listed in its prompt:

- "resume my sessions", "bring back my sessions", "restore everything" →
  `scope: all`; "resume my sessions on this Mac/here" → `here`.
- Read-back: "Resume 5 sessions, 3 here and 2 on Gnowee?" (or "Resume 3
  sessions?" for `here`, or "There's nothing to resume." with no action).
- On "yes" it runs Resume all with that scope and says "Resuming 5." or
  "Resumed 4; one couldn't be resumed."
- It is a confirmed action like `start`/`resume`, so the phone's voice
  path validates it again on the main before running it.

## Session viewer theme

- Config gains `viewer_theme: ViewerTheme` (`"chat"` | `"terminal"`,
  `#[serde(default)]` = `chat`). Settings → Sessions gets "Session viewer"
  with Chat / Terminal. The viewer header gets a small toggle button
  (title "Terminal look" / "Chat look") that saves the same field through
  `set_config` and redraws the open panel.
- The phone has its own config and the same two controls in its own
  settings and viewer.
- Rendering: one renderer. The panel root gets `modal--terminal` when the
  theme is terminal; turns get the markers the look needs (a `turn__mark`
  span: `>` for the user, `⏺` for the agent and tools; a `⎿` line for a
  tool result when the transcript has one). CSS draws both looks:
  - **Terminal:** monospace (`ui-monospace, SFMono-Regular, Menlo,
    monospace`), background `#0a0a0a`, text `#e8e2d4`, no bubbles, full
    width, the user's `>` in Maya's gold, tool lines and results dimmed;
    the composer as a `>` prompt line with an ⏎ button.
  - **Chat:** today's look, markers hidden.
  - Markdown, code blocks, links, question and option buttons,
    attachments, the context meter and peer/notice turns keep working in
    both.

## README

- "First run" gains a step: open Settings → Git, turn on GitHub and/or
  GitLab, give each a token (or the name of a variable that holds one),
  and pick SSH (the default) or HTTPS for review clones.
- The feature list gains: after a restart, resume every session that was
  running (on this machine and on paired ones); a terminal look for the
  session viewer.

## Errors

| Situation | What the user sees |
|---|---|
| Folder gone | "Couldn't resume <name>: folder missing" |
| Transcript gone (agent does not list the id) | "Couldn't resume <name>: the agent no longer has that session" |
| Agent not installed | "Couldn't resume <name>: <Agent> is not installed" |
| Assistant not connected | rows say "waiting for <name>"; Resume all skips them and says so |
| `remembered.json` unreadable | treated as empty, logged; never blocks startup |

## Testing

Rust (`core`, TDD):

- `remembered`: add on first sight; name/dir/lastSeen update; `goneAt` on
  vanish and cleared on return; dropped after `GRACE_MS` gone while
  running; dropped at once on `closed`; `Other` and non-plain ids never
  recorded.
- `classify_at_start`: still-live entries and ones gone within the grace
  of the last heartbeat are "was running"; older ones dropped.
- Save/load round trip; an unreadable file reads as empty.
- `resume_remembered`: skips live ids, missing folders, unknown sessions;
  resumes outside the projects directory; order and spacing (the spacing
  injected for tests).
- `Up::Board` with and without `remembered`; `CommandKind::Forget`
  round trip; an older peer's board parses.
- Interpreter: `resume_all` validation and read-back for `all`/`here` and
  nothing to resume.
- Config: `viewer_theme` default and round trip.

Frontend (vitest):

- Banner: counts and machine split, the three buttons, the waiting state,
  failures line; Choose list ticks, Forget unticked, Resume N.
- Viewer: `modal--terminal` and markers in terminal; none visible in chat;
  the header toggle and the Settings choice call the save.

By hand (owed): a real restart of the Mac with sessions open, with and
without the laptop; the terminal look on a long session with tool calls,
a question and attachments; the phone banner.

## Release

Two `feat`s on `feat/resume-sessions-and-terminal-theme` from `main`
(0.24.0): ships as 0.25.0, set with `sh scripts/set-version.sh 0.25.0` in
its own `chore(release): 0.25.0` commit.

## Addendum (2026-10-19): the viewer shows what the terminal shows

Seen side by side with an agent's terminal, the viewer showed the
conversation but not tool output, edit diffs, the working line or the
status line, so the terminal look read as "Maya in a monospace font". The
user chose to show the same content, in both looks, keeping the terminal
look's styling as it is.

### Reading more of the transcript

- Claude Code: a tool turn is now `Name(arg)` (e.g. `Bash(cargo test)`,
  replacing `Bash: cargo test`) paired with its result from the matching
  `tool_result` block: the result text, at most `TOOL_OUTPUT_LINES = 200`
  lines kept, and whether it is an error (`is_error`).
- Edit / MultiEdit / Write: the old and new text become a line diff (at
  most 200 lines kept) with `+a −b` counts; a Write to a new file is all
  additions.
- Codex and OpenCode: the same where their transcripts carry tool outputs
  (Codex `function_call_output`, OpenCode tool part `state.output`);
  agents whose files carry none render as before.
- The tool turn gains optional `output: Option<String>`, `diff:
  Option<Vec<DiffLine>>`, `added`/`removed` counts and `is_error`, all
  `#[serde(default)]` on the wire: older Mayas and phones ignore them, an
  older assistant sends none. Output in a history reply across machines
  is capped at `REMOTE_OUTPUT_LINES = 40` lines.

### Showing it

- Terminal look: `● Bash(cmd)`, then the first 4 lines of output under
  `⎿`, then `… +N lines` which expands in place on click; edits as
  `● Update(path)` with `⎿ Updated path (+a −b)` and the diff, red and
  green lines with line numbers when known; an error result in red.
- Chat look: each tool call is a compact row (`▸ Bash · cmd`, `▸ Update ·
  path (+a −b)`, a red mark when it failed); clicking opens the output or
  diff in a monospace block.
- Both looks, while the session is working: a `✻ Working… (12m 4s)` line
  from `state_since` (no token count: Maya has none live).
- Status line: model · folder · branch · PR · ctx %. Terminal look: under
  the `›` prompt; chat look: in the viewer header beside ctx %. The
  branch comes from the same lookup as the PR badge; the agent's own
  permission mode is not known and not shown.
- Unchanged: the terminal look's colours and font, the panel chrome, the
  Model/Effort pickers.

### Testing

Transcript fixtures: a Bash call and result, an error result, an Edit
diff with counts, a Write to a new file, truncation at 200 lines, the
40-line cap on the wire, an older peer's history without the fields.
Rendering in both looks: the 4-line preview and expand, diff colours,
the working line, the status line.

## Not in this change

- Resuming automatically at startup without asking.
- Remembering sessions across more than the last run (no history).
- Per-session viewer themes.
