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
  (label, platform, IP address, connected or last seen) with Remove, e.g.
  "Gnowee (macos, 192.168.55.70) — Connected". When it was last seen is
  saved, so a restarted main still shows it. A Name field (the
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
  `pair` with the code and a nonce of its own; the main checks the code and
  its expiry, mints a random 32-byte token, stores it against the
  assistant's id, and returns it once in `paired`. Its `welcome` then
  carries HMAC-SHA256(code, that nonce): proof that it received the code.
  The assistant keeps the id and token only once that proof checks out;
  otherwise it reports "The main Maya failed to prove it knows the pairing
  code." and saves nothing. Both sides keep the token in their config
  (`~/.claude/maya/config.json`); it never travels again.
- What that proof does and does not give: the code travels in plain text
  to the host the user typed, and whatever answers there proves only that
  it received the code. A wrong host that is not a Maya main fails the
  handshake, and nothing is kept. It does not authenticate the main: an
  active impostor at the typed host (on a network the user does not
  trust) can read the code from `pair` and compute the proof, so pairing
  is not protected against it. A password-authenticated key exchange
  (PAKE) over the code would close this; it is out of scope for this
  version.
- The main saves a pairing once `paired` is out, before it sends
  `welcome`. If `paired` cannot be sent (the assistant is already gone),
  the entry it just made is removed, or for a machine pairing again given
  back its old token and names, and nothing is saved. If the save fails,
  the entry is taken back the same way and the assistant gets `bye
  { reason: "could not save the pairing" }` instead of `welcome`: it keeps
  nothing and shows "The main could not save the pairing." If `welcome`
  cannot be sent, the entry is taken back and the list saved without it.
- **Identity is the address; names are labels.** The main records each
  assistant's IP address as it sees it, at pairing and on every successful
  authentication (with the name, hostname and platform the machine just
  gave), and saves the list. A `pair` from the address of a paired entry
  that is not connected is the same machine pairing again (its config was
  reset, say): the entry keeps its id, gets a new token and the new names,
  and its old board is dropped. Any other `pair` is a new entry.
- "Pair again" on an assistant stops its running client and waits for its
  thread to end (its connection closed) before sending `pair`, and the main
  forgets a closed connection before it answers the close, so the main
  counts the machine as disconnected and reuses its entry. If pairing
  fails, the old client starts again.
- Every later connection is challenge-response: the main sends a random
  nonce, the assistant answers with HMAC-SHA256(token, nonce). A wrong
  answer, an unknown assistant or a removed one is closed with a reason.
  The handshake is mutual: the assistant sends a nonce of its own with its
  answer, and the main's `welcome` carries HMAC-SHA256(token, that nonce);
  an assistant that holds a token refuses a `welcome` without a valid proof
  ("The main Maya failed to prove it holds the pairing token.") and tries
  again after 30 s. The `welcome` right after pairing proves the code
  instead (above).
- A wrong or expired code is reported on the assistant ("Wrong or expired
  pairing code."). Three wrong codes from one address in five minutes are
  refused for five minutes.
- At most eight connections may be in their handshake (not yet through
  `welcome`) at once, and at most two from any one address; a third from
  the same address is closed before the WebSocket upgrade. An assistant
  frees its slot on `welcome`.
- Traffic is plain text on the LAN; encryption is out of scope for this
  version and noted in the README.

### Protocol

One WebSocket per assistant, opened by the assistant, reconnecting with
backoff (1, 2, 4, 8, 16 s, then every 30 s). Messages are JSON text frames
with a `type`. Protocol version 1.

Up (assistant → main):

- `hello { protocol, app, name, hostname, platform }`
- `pair { code, nonce }` (first connection only) or `auth { mac, nonce }`
- `board { cards, dirs }` — the assistant's cards as the main should show
  them, and its project folders for the "+" dialog. Sent after `welcome`,
  then whenever the board changes, at most once a second.
- `result { id, ok, error?, data? }` — the answer to a command; `data`
  carries a conversation for `history` and the list for `list_resumable`.
- `ping` every ten seconds; the main answers `pong`.

Down (main → assistant):

- `challenge { nonce }`, `welcome { name, mac }`, `paired { id, token }`,
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
  assistant's label for remote ones, and `machine_address`, its IP address
  as the main sees it. A label is the assistant's name, or `name
  (address)` for every assistant sharing that name. The board is local cards plus every
  connected assistant's cards, in the same columns and order rules.
- A remote card shows a small remote glyph after its name, the machine's
  label as its subtitle, "Runs on <label>, <address>" as the glyph's
  tooltip (the card's and the modal header's; just "Runs on <label>" when
  the label already carries the address), and no Terminal button. Everything else on the card and in its modal works: the
  conversation (fetched with `history` when the card's state, state time
  or snippet moves, and at most every 3 s while it is working, since tool
  calls change none of those), reply with attachments, answer
  buttons, compact, rename, model, effort and mode.
- Every session command in the app (`send_reply`, `answer_question`,
  `compact_session`, `rename_session`, `set_session_option`,
  `cycle_session_mode`, `session_history`) looks the session id up first: a
  local id runs as today, a remote id becomes a `command` to its assistant,
  and the command's `result` is the Tauri command's result. The page does
  not change for that. A live local session wins over a remote card with
  the same id: the board shows the local card only, and commands for that
  id run locally.
- The "+" and resume dialogs gain a Machine picker, "This Mac" first, then
  each connected assistant. A remote machine's folders come from its last
  `board`; start and resume forward to it. The picker shows even when this
  Mac has no projects directory: the setup hint ("Set a projects directory
  in Settings first.") belongs to "This Mac" and goes while another
  machine is chosen. An assistant with no folders gets its own hint: "Set
  a projects directory in Settings on <label>."
- A snapshot older than thirty seconds greys the machine's cards (a
  `stale` flag on `Card`); after five minutes without one they disappear.
  A disconnect greys them at once. A session on an expired board is not
  routed anywhere either.
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
  remote machines; a chosen folder is split into project and machine only
  when `machine` is the label of a connected assistant, so a local folder
  that happens to be named like that stays local. Voice and notifications use the label.
- Nothing on an assistant's first board is announced when this run of the
  main holds no board for it: right after pairing, and on its first
  connection after the main starts. The server decides that under its lock
  (no board held for the id) and hands the board to
  `Notify::board_seeded` before it joins the boards; the app seeds its
  notifier with it (`Notifier::seed`). Later changes are announced. A
  reconnect within the same run is not seeded: its first board is compared
  with the one the main holds, so an ask or a finished turn that began
  while the assistant was away is announced when it comes back.

### The assistant

- Shows only its own sessions, exactly as today. The main's cards never
  appear on an assistant.
- Runs commands from the main through the same functions its own buttons
  use, and answers each with a `result`. A reply's attachments are saved
  only after the session is found, and deleted again if a later one or
  the reply itself fails.
- Looks the main's host up on a helper thread and gives up after five
  seconds ("Could not find <host>: the lookup took more than 5 s."), then
  backs off as for any failed connection.
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
- Two assistants with the same name are shown as `name (address)`, both
  of them.
- A board the main cannot decode (a newer Maya's) is noted beside the
  assistant; the last good board is kept and counts as just received, so
  its cards stay and do not grey while the note explains why.
- A removed assistant is sent `bye { reason: "removed" }`, its cards are
  dropped, and it cannot reconnect; on its side the status shows "Removed
  by the main Maya; pair again." The shorter list is saved first; if that
  fails, Remove shows the error and nothing changes. A save when an
  assistant connects or disconnects (its address, when it was last seen)
  that fails is only logged.
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
- `src-tauri/src/model.rs` — `machine`, `machine_address`, `stale` on
  `Card`.
- `src-tauri/src/config.rs` — `PairedAssistant` with `address` and
  `last_seen`.
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
  reused nonce; merge order, stale and expiry; routing by session id (and
  not to an expired board); labels by address; re-pairing by address;
  handshake slots per address; `Notifier::seed`; machine-qualified spoken lines and summary lines; the
  `name on machine` matcher.
- Integration (one process, localhost): a server and a client pair, the
  client sends a board, the server merges it, a `reply` command round-trips
  to a `result`, a wrong HMAC is refused, a removal disconnects, an
  assistant refuses a main that cannot prove the code or the token, the
  same address pairing again keeps its entry, a third handshake from one
  address is refused, a client gone right after `pair` leaves no entry,
  nothing on the first board after pairing is announced, and what began
  while an assistant was away is announced when it reconnects.
- End to end on two Macs: pair, see the remote cards, reply and answer from
  the main, start a session on the assistant from the main, hear the main
  announce a remote decision, and confirm the assistant stays silent.

## Out of scope

- Discovery of the main (mDNS/Bonjour); the host is typed.
- Encryption on the wire.
- Terminal access to remote sessions.
- The main's Pull Requests tab showing an assistant's pull requests.
- Assistants of assistants.
