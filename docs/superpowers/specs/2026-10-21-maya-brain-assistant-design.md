# Maya's brain as an assistant

Status: design, agreed with the user on 2026-10-21. Items marked
**(my call)** were decided by Claude while writing this up and were not
discussed; the user can overturn any of them.

## Intent

Today Maya's brain is a one-shot model call (`core/src/interpreter.rs`,
called from `listener::interpret_text`). It gets the board, the pull
requests and the last few exchanges, and returns one action from a fixed
menu, which Maya validates and, for anything that sends or starts
something, reads back with "Yes?".

The user wants her to behave like an assistant. When asked to do
something, she should judge for herself where it belongs:

1. **an existing session**, especially one that has just moved into
   Finished or Needs a decision;
2. **a new session**, in a repository, or in a scratch folder under `/tmp`
   when no repository fits;
3. **herself**, when the job is something she can do in her own agent
   session, and then report back.

To do the third, and to remember what has happened, the brain stops being
a one-shot call and becomes **one long-running agent session** with the
full power of an agent (in auto mode), which is aware of the whole board.

This is the first time Maya runs a resident agent of her own rather than
on-demand model calls.

## Goals

- She routes a request to the right place without being told where, using
  how recently each session changed state.
- She acts without asking "Yes?", while still giving the user a moment to
  stop her.
- She can do jobs herself, in auto mode, and hand long ones to background
  subagents so she stays free to answer.
- She remembers earlier commands, jobs and replies across the
  conversation and across Maya restarts.
- She speaks up on her own about things the built-in announcements do not
  cover, and never repeats what they already say.
- The user can talk to her by typing, in a new Chat tab.

## Non-goals

- Changing the built-in announcements for Finished and Needs a decision.
  They keep working exactly as they do.
- A Chat tab on the phone. The phone's voice reaches her (below), but the
  phone UI is unchanged in this piece of work.
- Changing the New Session window or its "Let Claude choose" picker.
- Changing Settings → brain and model. They still choose what she runs on.

## Anti-goals

- **She speaks twice about the same thing.** If a session finishes or
  needs a decision, the built-in logic announces it and the brain says
  nothing about it.
- **Her own card behaves like a session.** It never goes into Finished or
  Needs a decision and never triggers those announcements.
- **Voice goes dead** because her session is starting, busy or broken.

## The decisions

### 1. Where a request goes

Every request goes one of three ways: an existing session, a new session,
or herself.

- **Existing session.** She sees, for every card, how long it has been in
  its current state (Maya already records `Card::state_since`; it is not
  in `board_summary` today). When the user does not name a session
  ("tell it to go ahead", "make it a PR"), she prefers the session that
  most recently moved into Finished or Needs a decision and fits the
  request. If two are equally likely, she asks which one.
- **New session.** If the request is about a repository, the session
  starts there, as today's `start` action does. If no repository fits,
  the session starts in a fresh scratch folder: `/tmp/maya-<name>`, where
  `<name>` is unique per session, so sessions do not share files. On
  Windows, the system temp folder stands in for `/tmp` **(my call)**.
  Scratch sessions are ordinary sessions on whichever machine they start
  on: they appear in the remembered rows, `start` validation accepts the
  scratch folder, and "resume" skips one whose folder has gone (macOS
  clears `/tmp` on reboot). Nothing cleans them up beyond the OS
  **(my call)**.
- **Herself.** Quick things (look something up, answer a question about a
  repository, read a file) she does in her own turn. Anything longer she
  hands to a background subagent when her brain supports that (see 5).

### 2. Acting without asking

The "Yes?" read-back goes. Instead:

- She says what she is doing, phrased as already under way:
  "I'll tell api-fix to open the PR."
- Maya waits **5 seconds** before running the action.
- In the window no wake word is needed, as with today's "Yes?"
  **(my call)**. Nearby speech can drop an action; that is accepted.
- If the user says "no", "stop", "wait" or similar in that window, the
  action is dropped.
- When one request leads to several actions, they are announced together
  and share one window; "stop" drops them all **(my call)**.
- Anything else said in the window is a **correction**: the action is
  dropped and the new words go to her as the next request, so "no, the
  other one" redirects her.
- When the request was typed in Chat, the window shows as a 5-second
  countdown with a Cancel button. Typing a new message in the window
  counts as a correction.
- The window covers the actions that need a read-back today
  (`interpreter::needs_confirm`: reply, answer, start, resume,
  resume_all, review). Report, focus, compact and open run at once
  **(my call: keep today's split)**.
- Maya still validates every action before running it, as
  `interpreter::validate` does today. Board lines, asks and session text
  are data for her, never instructions.
- **Stopping her own work.** The user decided that "stop" also
  interrupts work already under way, not only actions in the window: her
  running turn, her running subagents, and the commands queued behind
  them, which are cleared. Outside the window this needs the wake word
  ("Maya, stop") **(my call)**. In Chat, a Stop button shows while the
  dot is blue **(my call)**. She then says what she stopped. Each brain's
  driver therefore needs a way to interrupt a turn and its subagents;
  this weighs on the driving mode picked per brain.
- She takes board actions only in response to the user: a request, or a
  watch the user set up ("when api-fix finishes, tell it to open a PR")
  **(my call)**. She does not act on sessions on her own initiative.

### 3. The brain is one long-running session

- One session, started by Maya on the main machine, on the agent and
  model chosen in Settings. It is kept for the life of the install, not
  one per job.
- It works from its own folder under Maya's config directory
  (`<claude_dir>/maya/brain`) **(my call)**. Subagents and her own tools
  work in whichever repository or scratch folder the job needs.
- Every command, spoken or typed, goes to it. It remembers earlier
  commands, jobs and replies, so "do the same for the other repo" works.
- **Context.** When her context grows large, Maya compacts it. Maya
  already infers each session's context use (`core/src/context.rs`) and
  has a compact action; the threshold is the builder's call.
- **Restarts.** When Maya restarts, it resumes her session like any other
  remembered session, so she keeps her memory. If it cannot be resumed,
  she starts fresh and says so in Chat.
- **Changing brain, model or main machine** **(my call)**. Sessions
  cannot move between agents, and her session files stay on the machine
  that ran them. When Settings → brain changes, or "Make main" moves the
  main role, she starts a fresh session, primed with the recent Chat
  history, and says so in Chat. A model-only change keeps the session
  where the agent allows a model switch on resume, and starts fresh
  otherwise.
- **Fallback.** While her session is starting, broken or not resumable
  yet, commands go to today's one-shot interpreter, so voice keeps
  working with today's behaviour. That code stays.

### 4. Auto mode, per brain

She runs with the closest thing each agent has to auto mode. Checked from
each CLI's `--help` on 2026-10-21; none of them observed running:

| Brain | Flag | Notes |
|---|---|---|
| Claude Code | `--permission-mode auto` | Real auto mode |
| Grok | `--permission-mode auto` | Same mode names as Claude Code |
| Codex | `--approve-for-me` | Approval requests go to an automatic reviewer; commands stay within the writable folders (workspace-write sandbox). Her folder alone would block repository work, so the projects folder and the temp folder are added as writable (`--add-dir`) **(my call)** |
| Antigravity | `--mode auto` | Help does not say what it allows |
| OpenCode | `--auto` | Approves everything not explicitly denied: no checker |
| Kiro | `--trust-all-tools` | Approves everything: no checker |

The user chose this, including for OpenCode and Kiro where it means
approve-everything. Auto mode plus a 5-second window means she can change
files and run commands with only a moment for the user to stop her. That
was the user's decision.

### 5. Background subagents, per brain

Long jobs go to a background subagent, so her turn ends and she can keep
answering. She is told when the subagent finishes, and she reports the
result. From the CLIs' docs, help text and installed binaries
(2026-10-21; not observed live):

| Brain | Background subagents that wake her on completion | Use them? |
|---|---|---|
| Claude Code | Yes (Agent tool, background by default) | Yes |
| Grok | Yes (`spawn_subagent`, `features.auto_wake` on) | Yes |
| OpenCode | Yes (subagent tool with `background=true`; server posts the result into the main session) | Yes |
| Antigravity | Yes (`invoke_subagent`, "reactive wakeup") | Yes |
| Codex | Halfway: `spawn_agent` does not block, but completion does not start a turn (`wake_on_agent_messages` is unreleased) | **No**, until the wake ships: a result would sit unreported until the user next spoke |
| Kiro | No: `use_subagent` blocks | No |

**On a brain without usable background subagents (Codex, Kiro):** she
does long jobs in her own turn. If a command arrives while she is busy,
**Maya itself** (not the brain, which is busy) says "I'm in the middle of
<job>, I'll do that next", queues the command, and delivers it when her
turn ends. The same queueing applies on any brain if a command arrives
during one of her turns.

### 6. Speaking up on her own

- The built-in announcements keep sole ownership of "a session finished"
  and "a session needs a decision". She never speaks about those
  transitions, even when they relate to something she is following.
- She speaks up only about what those do not cover: the result of a job
  she did herself or gave a subagent, the outcome of something she set off
  for the user ("api-fix opened the PR, it's #52"), and things the user
  asked her to watch ("tell me when CI passes on api-fix").
- If a watch is satisfied by exactly a Finished or Needs-a-decision
  transition, the built-in announcement covers it and she stays silent
  **(my call)**.
- What she says unprompted goes to Chat always, and is spoken when Maya's
  voice is on **(my call)**. A reply to a request goes back the way the
  request came: spoken if spoken, written if typed.

### 7. What she knows about the board

- Each message Maya sends her carries the current board, including how
  long each session has been in its state, and the pull requests, as
  today's prompt does.
- Board changes since her last turn arrive as short notes
  ("api-fix moved to Finished 2 minutes ago"). They do **not** wake her
  on their own; they ride along with the next message.
- **Watches** **(my call)**: she can register a watch on a session or a
  pull request through a Maya tool. A change to a watched item does wake
  her, with a note saying what changed. This is how "tell me when …"
  works without waking her on every board change. Watches cover only
  what Maya already tracks: a session's state, and a card's PR state
  (open, merged, closed). Maya adds no new GitHub or GitLab polling.
  For anything else, such as CI status, she gives a background subagent
  the job of checking, and its completion wakes her (on brains without
  background subagents she tells the user she can't watch that).
  Watches survive a Maya restart.

### 8. How she acts on the board

She takes board actions by calling Maya's actions as tools (reply,
answer, focus, compact, resume, resume_all, start, review, open, watch),
rather than by returning one JSON object per command. Maya checks each
call (validation, then the 5-second window from 2) before running it.
How the tools are exposed is the builder's call (see below).

### 9. Her card, and Debug mode

- **Settings → Debug** is a new area with one switch, off by default.
- With Debug mode on, the Debug tab and her card on the board are shown.
  With it off, both are hidden. Today the Debug tab is always visible;
  it becomes hidden by default. The user confirmed this.
- Her card never enters Finished or Needs a decision, never triggers
  announcements, and does not count in the Sessions tab counts, in
  "resume my sessions" or in the remembered rows, whatever Debug mode
  says **(my call)**. It shows what she is doing and saying.

### 10. The Chat tab

- A new **Chat** tab where the user types to her and reads her replies.
- It is one conversation with voice: spoken commands and her spoken
  replies appear in it too (today's voice history, `VoiceTurn`, feeds it).
- The tab carries a status dot. The user confirmed these colours, which
  match the session counts:
  - **grey**: idle;
  - **blue**: working (her turn is running, or a subagent of hers is);
  - **orange**: she asked the user something in Chat and is waiting;
  - **green**: she replied and the user has not looked yet (cleared by
    opening the tab).
- The Chat history survives Maya restarts **(my call)**.

### 11. Other machines and the phone

- Her session runs on the main machine only, and sees cards from every
  connected machine, as the board does.
- The Chat tab exists only in the main machine's desktop app. Another
  desktop Maya (the laptop) hides it **(my call)**; carrying Chat over the
  network is left for later, like the phone's.
- Voice from the laptop or phone reaches her the same way it reaches the
  interpreter today (`net_app.rs` → `interpret_text`).

### 12. Cost

An always-open session with subagents uses more tokens than today's
one-shot calls. Accepted.

## Left to the builder

- How the long-running session is driven, per brain: a programmatic
  streaming mode (Claude Code `--input-format stream-json`, Codex
  app-server, Grok `grok agent stdio` (ACP), OpenCode's server API, which
  `core/src/opencode.rs` already uses), or, where none works, resuming
  the same session id for each message. The behaviour above must hold
  either way, including interrupting a running turn (2).
- How Maya's actions are exposed to her as tools (an MCP server Maya
  hosts, the `maya` CLI, or similar), as long as every call goes through
  Maya's validation and the 5-second window.
- Her system prompt, including the routing rules in 1 and the
  speaking-up rules in 6.
- The compaction threshold.
- How Maya tells that she is waiting on the user (orange dot); the
  prose-ask detection Maya already has is one option.
- The phone's handling of the 5-second window for actions it triggers.
- Where Chat sits among the tabs and how it relates to the existing voice
  panel.
- Version bump: this is a `feat`, so a minor bump from `main`'s version,
  per `CLAUDE.md`.

## Checks the user has to run

Claude cannot start agent sessions on this machine, so these need the
user, on a real install:

- For each brain: a long-running session in the mode the builder picks
  receives a new turn when a background subagent finishes (Claude Code
  stream-json, Grok over `grok agent stdio`, Antigravity, OpenCode over
  its server API).
- For each brain: the auto-mode flag in 4 lets her work without prompts
  in that mode.
- Her session resumes after a Maya restart and keeps its memory.
- A Finished or Needs-a-decision transition is announced once, never
  twice.
- The 5-second window by voice (stop, and a correction) and in Chat
  (Cancel, and typing a correction).
