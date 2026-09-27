# Eye: card actions and reply modal — design

Date: 2026-09-28
Status: approved design, awaiting implementation plan
Builds on: `2026-09-28-eye-session-board-design.md`

## Purpose

Let the user act on a session from the board without leaving the app:
jump to its terminal, read what has been said, and send it a message.

## Findings from the feasibility spike

- Every interactive session binds an inbox socket, recorded in the registry
  as `messagingSocketPath` (`/tmp/cc-socks/<pid>.sock`). Documented at
  https://code.claude.com/docs/en/cross-session-messaging.md, section
  "The session's inbox socket".
- On macOS the auth line is optional. A newline-terminated line of the form
  `{"type":"user","message":{"role":"user","content":"<text>"}}` is delivered.
  Verified: a probe posted from a Python script arrived in a live session.
- A socket message reaches Claude as a message from another session, not as
  the user typing. It cannot answer a permission prompt or an open question;
  those need the terminal.
- Claude's own client refuses to write if the connected endpoint's pid is not
  the target session's pid or its uid is not ours. Eye mirrors the pid check.
- Sessions in bypass-permissions mode hold peer messages for approval. The
  user's sessions run in auto mode, so messages are delivered directly.

## Behaviour

### Card

- Footer with two buttons: **Terminal** (focus the tab, existing command) and
  **Reply** (open the modal).
- Clicking the card body or pressing Enter on a focused card opens the modal.
  Clicking a button does not also trigger the body action.

### Modal

- One modal at a time, covering the board, closed by Escape, the × button or
  clicking the backdrop.
- Header: session name, project folder, state badge.
- History: the last 30 turns of the session transcript, oldest first,
  scrolled to the bottom on open. Turn kinds:
  - **user**: the user's prompt text. A user entry whose content is only
    tool results is not a turn.
  - **assistant**: Claude's text blocks joined with blank lines.
  - **tool**: one line per `tool_use` block: `<name>: <summary>` where summary
    is `command`, `file_path`, `path`, `prompt`, `description` or the first
    question, whichever exists first, trimmed to 120 chars.
  - Markdown is shown as plain text.
- When the session is Awaiting Decision: a banner "This session is waiting
  for a decision in its terminal. A reply will queue behind it." with an
  Open terminal button.
- Composer: multi-line text box, Send button, Cmd+Enter sends. Empty or
  whitespace-only text is not sent. After a send: "Delivered" (green) or the
  error text (red) under the composer; the box clears on success.
- If the session has no `messagingSocketPath`, the composer is replaced by
  "This session has no inbox. Use the terminal."
- History refreshes while the modal is open whenever the board refreshes.

### Delivery (Rust)

- `inbox::send(socket_path, expected_pid, text) -> Result<(), String>`:
  connect with a 5 s timeout, read the peer pid (`LOCAL_PEERPID`) and refuse
  if it differs from `expected_pid`, write the JSON line, shut down the write
  side, close. Errors are short human sentences.
- Text is sent verbatim; newlines are preserved inside the JSON string.
- Size cap: refuse text over 100 000 characters (well under the documented
  ~1 M limit) with a clear error.

## Interfaces

- Registry: `RegistrySession.messaging_socket_path: Option<String>`.
- Card payload gains `hasInbox: boolean`.
- Transcript: `read_turns(path, max_turns) -> Vec<Turn>`,
  `Turn { kind: "user" | "assistant" | "tool", text: String }`, reading the
  last 1 MB of the file.
- Commands: `session_history(session_id) -> Result<Vec<Turn>, String>`,
  `send_reply(session_id, text) -> Result<(), String>`. Both look the session
  up in the current registry; an unknown id is an error.

## Errors

- Session gone between opening the modal and sending: "Session is no longer
  running."
- Socket missing or refused: the OS error in one sentence.
- Peer pid mismatch: "Refusing to send: the socket is not owned by that
  session."
- Transcript unreadable: empty history with a note "No transcript found."

## Testing

- Rust: `read_turns` against a fixture with user, assistant, tool_use and
  tool_result entries; ordering and the 30-turn cap; tool summary
  selection. `inbox::send` against a Unix socket bound in a temp dir by the
  test: asserts the exact line received and that a pid mismatch is refused.
- Vitest: card renders both buttons with `data-action`; modal renders
  header, turns with kind classes, banner only when awaiting, composer
  hidden without inbox; Send handler receives trimmed text and is not called
  for blank text.
- Manual: open the modal on a live idle session, send "reply to this with
  the word pong", see the session wake and answer; Terminal button focuses
  the tab; Escape closes.

## Out of scope

Typing into the terminal as the user, answering permission prompts from the
app, rendering Markdown, notifications on reply.
