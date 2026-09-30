# Maya: main and assistant Mayas over the local network — design

Date: 2026-09-30
Status: draft for review
Builds on: `2026-09-28-eye-session-board-design.md`,
`2026-09-29-maya-voice-assistant-design.md`

## Purpose

Run Maya on several computers and watch them all from one. A **main** Maya
shows the sessions of every **assistant** Maya on its own board as if they
were local, notifies and listens for all of them, and drives them through
the assistant. Each assistant keeps showing its own sessions but goes quiet.

The main never touches an assistant's machine. Every action on a remote
session is a command the assistant executes with its own platform code, so
a macOS main can drive a Linux or Windows assistant once those exist. The
messages carry only platform-neutral data: session ids, text, option
numbers, folder names.

## Behaviour

### Roles and settings

- **Main.** Settings › Network gains "Act as main Maya" with a port (default
  4127). Ticking it starts the server and shows a six-digit pairing code,
  valid for five minutes, with Regenerate, and the list of paired assistants
  (name, platform, connected or last seen) with Remove. A Name field (the
  computer's hostname when blank) is what assistants show as the main's
  name; changing it restarts the server, and assistants reconnect.
- **Assistant.** Settings › Network gains "Assistant to a main Maya" with
  Host, Port (default 4127), Name (the computer's hostname when blank), a
  Pairing code field and a Pair button. After pairing, a status line shows
  "Connected to <main name>", "Reconnecting…" or the last error.
- One Maya is one role at a time; ticking one box clears the other.
- While assistant mode is on, notifications and speech are suppressed and
  "Listen for 'Maya'" is off and disabled, with a note: "The main Maya
  notifies and listens for this machine."

### Pairing and trust

- Pairing happens over the same WebSocket: the assistant connects, sends
  `pair` with the code and its name; the main checks the code and its
  expiry, mints a random 32-byte token, stores it against the assistant's
  id, and returns it once. Both sides keep the token in their config
  (`~/.claude/maya/config.json`); it never travels again.
- Every later connection is challenge-response: the main sends a random
  nonce, the assistant answers with HMAC-SHA256(token, nonce). A wrong
  answer, an unknown assistant or a removed one is closed with a reason.
  The handshake is mutual: the assistant sends a nonce of its own with its
  answer, and the main's `welcome` carries HMAC-SHA256(token, that nonce);
  an assistant that holds a token refuses a `welcome` without a valid proof
  ("The main Maya failed to prove it holds the pairing token.") and tries
  again after 30 s. Only the `welcome` right after pairing carries no proof.
- A wrong or expired code is reported on the assistant ("Wrong or expired
  pairing code."). Three wrong codes from one address in five minutes are
  refused for five minutes.
- Traffic is plain text on the LAN; encryption is out of scope for this
  version and noted in the README.

### Protocol

One WebSocket per assistant, opened by the assistant, reconnecting with
backoff (1, 2, 4, 8, 16 s, then every 30 s). Messages are JSON text frames
with a `type`. Protocol version 1.

Up (assistant → main):

- `hello { protocol, app, name, hostname, platform }`
- `pair { code }` (first connection only) or `auth { mac, nonce }`
- `board { cards, dirs }` — the assistant's cards as the main should show
  them, and its project folders for the "+" dialog. Sent after `welcome`,
  then whenever the board changes, at most once a second.
- `result { id, ok, error?, data? }` — the answer to a command; `data`
  carries a conversation for `history` and the list for `list_resumable`.
- `ping` every ten seconds; the main answers `pong`.

Down (main → assistant):

- `challenge { nonce }`, `welcome { name, mac }`, `paired { token }`,
  `bye { reason }`
- `command { id, kind, ... }` with kinds `reply { session, text,
  attachments? }`, `answer { session, question, option }`, `compact
  { session }`, `rename { session, name }`, `set_option { session, model?,
  effort? }`, `cycle_mode { session }`, `start { dir, prompt, options }`,
  `resume { dir, session }`, `list_resumable { dir }`, `history
  { session }`.

Cards in `board` are the assistant's own `Card` values with `pid` and
`has_inbox` left in (the main ignores them) and no `machine`; the main sets
`machine` when it merges them.

### The main's board

- `Card` gains `machine: Option<String>`: `None` for local cards, the
  assistant's name for remote ones. The board is local cards plus every
  connected assistant's cards, in the same columns and order rules.
- A remote card shows a small remote glyph after its name, the machine name
  and platform as its subtitle and the glyph's tooltip, and no Terminal
  button. Everything else on the card and in its modal works: the
  conversation (fetched with `history`), reply with attachments, answer
  buttons, compact, rename, model, effort and mode.
- Every session command in the app (`send_reply`, `answer_question`,
  `compact_session`, `rename_session`, `set_session_option`,
  `cycle_session_mode`, `session_history`) looks the session id up first: a
  local id runs as today, a remote id becomes a `command` to its assistant,
  and the command's `result` is the Tauri command's result. The page does
  not change for that.
- The "+" and resume dialogs gain a Machine picker, "This Mac" first, then
  each connected assistant. A remote machine's folders come from its last
  `board`; start and resume forward to it.
- A snapshot older than thirty seconds greys the machine's cards (a
  `stale` flag on `Card`); after five minutes without one they disappear.
  A disconnect greys them at once.
- Attachments on a remote reply are sent inline (base64 in the command, the
  same 20 MB cap) and saved by the assistant where it saves its own.

### Notifications and voice on the main

- Announcements for remote sessions name the machine: "hexgrid on maya-mini
  needs a decision", "hexgrid on maya-mini is finished". The Focus rule and
  the notify/speak settings apply as for local sessions.
- The interpreter's board summary gains a machine column
  (`name | harness | state | id | project | machine | asks`), so "tell
  hexgrid on maya-mini to go ahead" resolves when two machines have a
  hexgrid; the loose session matcher tries `name on machine` before
  `name`, and an ambiguous name asks "Which one: hexgrid on maya-mini or
  hexgrid on this Mac?". Read-backs say the machine for remote sessions.
  Folders for start and resume are listed as `project (on machine)` for
  remote machines.

### The assistant

- Shows only its own sessions, exactly as today. The main's cards never
  appear on an assistant.
- Runs commands from the main through the same functions its own buttons
  use, and answers each with a `result`.
- Sends a `board` whenever its own refresh runs and the cards or folders
  changed, throttled to one a second.
- Logs connection events and every command to the Debug tab under
  `network`; the main logs the same under `network` with the machine name.

### Failure modes

- Malformed or unknown messages are logged and ignored on both sides;
  neither side ever crashes on input from the other.
- A command for a session that no longer exists returns `ok: false` with
  "Session is no longer running."; the main shows it where the action was
  taken, as it shows local errors.
- Two assistants with the same name are shown as `name (hostname)`.
- A removed assistant is sent `bye { reason: "removed" }`, its cards are
  dropped, and it cannot reconnect; on its side the status shows "Removed
  by the main Maya; pair again."
- Ticking off "Act as main Maya" closes every connection and drops remote
  cards; assistants show "Reconnecting…" until it is back.

## Components

- `src-tauri/src/net/protocol.rs` — message types, encode/decode, protocol
  version, the pairing-code and HMAC helpers (pure).
- `src-tauri/src/net/server.rs` — the main: `tungstenite` listener, one
  thread per assistant, pairing window, connected assistants and their
  snapshots, command dispatch with ids and result matching (timeout 30 s).
- `src-tauri/src/net/client.rs` — the assistant: connection thread with
  backoff, pairing, board sending, command execution.
- `src-tauri/src/net/merge.rs` — merging local and remote cards, stale and
  expiry rules, name de-duplication, routing a session id to a machine
  (pure).
- `src-tauri/src/model.rs` — `machine`, `stale` on `Card`.
- `src-tauri/src/lib.rs` — role setup, the session commands routing to the
  server, `list_machines`, pairing commands for Settings.
- `src-tauri/src/notify.rs`, `interpreter.rs`, `listener.rs` — machine in
  spoken lines, summary and matching.
- `src/settings.ts` (Network section), `src/card.ts` (glyph, subtitle, no
  Terminal), `src/newsession.ts` and `src/resume.ts` (Machine picker).
- Crates: `tungstenite`, `hmac`, `sha2`, `rand` (or `getrandom`).

## Testing

- Unit: encode/decode of every message; pairing code format and expiry;
  HMAC handshake accepts the right answer and refuses the wrong one and a
  reused nonce; merge order, stale and expiry; routing by session id; name
  de-duplication; machine-qualified spoken lines and summary lines; the
  `name on machine` matcher.
- Integration (one process, localhost): a server and a client pair, the
  client sends a board, the server merges it, a `reply` command round-trips
  to a `result`, a wrong HMAC is refused, a removal disconnects.
- End to end on two Macs: pair, see the remote cards, reply and answer from
  the main, start a session on the assistant from the main, hear the main
  announce a remote decision, and confirm the assistant stays silent.

## Out of scope

- Discovery of the main (mDNS/Bonjour); the host is typed.
- Encryption on the wire.
- Terminal access to remote sessions.
- The main's Pull Requests tab showing an assistant's pull requests.
- Assistants of assistants.
