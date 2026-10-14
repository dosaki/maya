# The phone joins a main — design

Date: 2026-10-14
Status: implemented
Builds on: `2026-09-30-maya-remote-assistants-design.md`,
`2026-10-07-maya-android-companion-design.md`,
`2026-10-12-promote-assistant-design.md`

## Purpose

A phone can pair with a main Maya that is already running, such as the Mac,
instead of being the main itself. Once it has joined, it shows the main's
whole board, gets the same notifications and acts on any session. It is
also in the pool, so the main can make it the main later with Make main.

Success: on a fresh phone the user picks Join a main, types the Mac's
address and the pairing code the Mac shows, and seconds later the phone's
board shows every session the Mac's board shows: the Mac's own and the
laptop's. A reply typed on the phone reaches a session on the laptop.
Make main on the Mac then turns the phone into the main without another
pairing code.

In this document **M** is the main, **P** the phone that joins it and
**A** any other assistant of M.

## Findings

- A fresh phone has no way in. The phone pairs assistants to itself as
  their main, and becomes a listener only by handing its role away with
  Make main. It has no `network_pair` command, its first-run overlay offers
  only Start, and its Network screen has no host or code fields. The core's
  `client::pair_with` is generic over the executor and works on the phone
  unchanged.
- A listening phone shows nothing. Boards travel only up (`Up::Board`);
  `Down` carries nothing about sessions. While the phone listens, its board
  is empty and it posts no notifications.
- All the routing a phone needs already exists on the main. The desktop
  main's Tauri commands (`send_reply`, `answer_question`, `start_session`
  and the rest in `src-tauri/src/lib.rs`) find a session's machine
  (`remote_machine_of`), then act locally or send a `CommandKind` to the
  assistant. A phone main does the same in `mobile/src/hub.rs`, where every
  session is remote.
- An assistant cannot act on other machines' sessions today. A Maya that
  mirrors can, so the main must grant this on purpose (see Permission).
- `server_start` on the phone starts the server without stopping a running
  client, so a listening phone that presses Start runs both.
- The phone's `hello` reports its platform as `android`, which neither page
  maps to a label.

## Behaviour

### Capability and permission

- `hello.caps` gains `mirror`: the Maya wants the main's board and will
  send requests. The phone sends `follow`, `lead` and `mirror`. The desktop
  app and `maya-cli` do not send `mirror`.
- `PairedAssistant` gains `mirror: bool` (serde default false): this Maya
  may see and control every session in the pool.
  - It is set to true at pairing when the pairing `hello` carries `mirror`.
    The user typed the code on that device, which is the consent.
  - It is false otherwise, and false for every entry saved before this
    change.
  - It travels in the handover list, so a new main honours it.
  - The entry a main makes for itself when it hands its role away
    (`promote::own_entry`) has `mirror` true when that main is a phone.
    The user pressed Make main on that phone.
- `AssistantStatus` gains `mirror` (the permission) and `wantsMirror`
  (connected, and its `hello` carried `mirror`).
- The main's Network list shows a switch, "Sees and controls all
  sessions", on each entry that has `wantsMirror` or `mirror`. Turning it
  on or off saves the paired list. Turning it off stops the mirror to that
  Maya at once and refuses its later requests. Turning it on sends a
  mirror at once.

### Mirror (main → phone)

- `Down::Mirror { allowed, cards, machines }`. With `allowed: false`
  the lists are empty: the main tells the phone it may not mirror.
  - `cards` is the main's own board exactly as its page shows it: its
    local cards and every assistant's, stale ones included. The main's
    local cards are sent with `machine` set to the main's name, since on
    the phone they are remote. No card belongs to P itself.
  - `machines` is the main first (`connected: true`, its hostname and
    platform), then its paired assistants as `list_machines` returns them,
    without P.
- `Welcome` gains `mirror: bool` (serde default false) for a Maya whose
  `hello` carried `mirror`: true when the main will mirror to it.
- The main sends a mirror to each connected Maya whose `mirror` is on and
  whose `hello` carried `mirror`:
  - right after its `welcome`;
  - whenever the main's board changes, at most once every 300 ms (a
    trailing send carries the last change);
  - when the Network status changes `machines`;
  - when its switch is turned on.
- When the switch is turned off, the main sends that Maya `allowed:
  false` once.
- A main with no Maya to mirror to builds no mirror.

### Requests (phone → main)

- `Up::Request { id, machine?, kind }` and
  `Down::Response { id, ok, error?, data? }`. `id` is the phone's, unique
  per connection.
- `kind` is a `RequestKind`: every `CommandKind` except `Promote`, plus
  `ListDirs` and `ListAgents`. A `Promote` is refused with "That cannot be
  requested."
- Session kinds (`Reply`, `Answer`, `Compact`, `Close`, `Rename`,
  `SetOption`, `CycleMode`, `Slash`, `History`) carry no machine: the main
  finds the session's owner the way its page does. `Start`, `Resume`,
  `ListResumable`, `ListDirs` and `ListAgents` carry the machine's name.
  The main's own name means the main.
- The server takes each request off the connection's reader and calls a
  new `Notify::request(machine, kind) -> Result<Option<Value>, String>` on
  a worker thread, without the server's lock, then sends the `response`. A
  routed command can take `ROUTE_TIMEOUT` (30 s), and other traffic on the
  connection carries on meanwhile.
- `data` is what the main's page gets for the same action: the reply's
  `Route`, the history's turns, the start's `StartResult`, the resumable
  list, the directories, the `AgentsReply`.
- Attachments in a `Reply` arrive as `Attachment` bytes. For a session on
  another assistant the main passes them on as they are. For a local
  session it writes them where an assistant writes received attachments
  today, and replies with those paths.
- Refusals:
  - mirror not allowed: "<main> has not allowed this Maya to control its
    sessions.";
  - an unknown session or machine, or an assistant that is offline: the
    same errors the main's page shows.

### On the desktop main

- The bodies of the Tauri commands that route move into plain functions
  over `&AppHandle` (`reply`, `answer`, `history`, `start`, `resume`,
  `list_resumable`, `list_dirs`, `list_agents`, and the session options).
  The Tauri commands and `Notify::request` both call them. Behaviour on
  the Mac's own page does not change.
- `Notify::request` on the desktop adapter (`net_app.rs`) matches the kind
  and calls those functions.
- The board-changed path that emits `sessions` to the page also tells the
  server to mirror.

### On a phone main

- `Notify::request` on the hub forwards the kind to the session's or
  machine's assistant, as the hub's own commands do. That lets a second
  phone join a phone main.

### The phone

*Join a main.*

- The first-run overlay offers two choices:
  - **Be the main**: today's Name, Port and Start.
  - **Join a main**: Name, Host, Port (default 4127) and Pairing code, with
    the line "Open Settings → Network on the main, choose Pair an
    assistant, and type the code it shows."
- The Network screen shows the same Join form whenever the phone is Off,
  beside Start server.
- Join calls a new command, `network_pair { host, port, name, code }`:
  - it checks the host and that the code has 6 digits;
  - it stops any server or client;
  - it pairs through `client::pair_with` with caps `follow`, `lead` and
    `mirror`;
  - it saves `role: Assistant`, `mainHost`, `mainPort`, `name`,
    `assistantId` and `token`, and starts the client and the keep-awake
    service.
- Errors (a wrong code, too many tries, no answer, a bad host) show under
  the form, worded as on the desktop.

*While joined.*

- The hub keeps the latest mirror. `list_sessions` returns its cards and
  `list_machines` its machines.
- Each page command the phone runs as a main goes up as a `request`
  instead, and its result or error reaches the page in the same shape:
  `session_history`, `send_reply`, `answer_question`,
  `set_session_option`, `cycle_session_mode`, `send_slash_command`,
  `rename_session`, `compact_session`, `close_session`,
  `list_project_dirs`, `list_agents`, `list_resumable_sessions`,
  `resume_session` and `start_session`.
- `send_reply` reads the attachment files and sends their bytes under the
  existing 20 MB limits (`remote_attachments`).
- `open_pr` opens the card's own PR link on the phone.
- Notifications are planned from the mirrored cards by the same rules and
  switches as on a main. The first mirror after each connect seeds the
  alerts without announcing them, so a reconnect does not repeat what is
  already waiting.
- The executor the client runs stays `NoSessions`: the phone runs nothing,
  and every command from the main except `Promote` is answered "This phone
  runs no sessions."

*When the main is unreachable.*

- The last mirror's cards stay, marked stale, with their buttons disabled
  as on a main's stale cards. The board is not announced again when the
  next mirror replaces it.
- The Network line says "Reconnecting…" with the last error.

*When the main does not allow the mirror.*

- The board shows one card in place of the columns: "<main> has not
  allowed this phone to see its sessions. Turn on 'Sees and controls all
  sessions' for <phone> in <main>'s Settings → Network." No notifications
  are posted.
- The phone knows this from `welcome.mirror: false` at connect and from
  `mirror { allowed: false }` when the switch is turned off later.

*The Network screen while joined.*

- "Connected to <main>" or "Reconnecting…", the host and port, and one
  button, **Leave**: today's Stop listening, renamed. It forgets the id and
  token, sets the phone to Off and clears the mirror.

*With Make main.*

- A phone that hands its role away lands in this joined state, mirroring
  the new main, instead of an empty board.
- When the main makes the phone the main, the phone drops the mirror, as
  it stops the client, and runs its server as it does today.
- A main moving to a new host sends `move` to the phone with the others.

*Fixes along the way.*

- `server_start` on the phone stops the client first.
- Both pages label the platform `android` as "Android".

## Protocol

Additive; the protocol stays at version 1.

- Up: `hello.caps` may hold `mirror`; `request { id, machine?, kind }`.
- Down: `welcome.mirror?`; `mirror { allowed, cards, machines }`;
  `response { id, ok, error?, data? }`.
- A main that does not know `mirror` (an older one) ignores the cap,
  sends no mirror and answers no request. The phone then shows the
  not-allowed card, worded for that case: "<main> runs an older Maya that
  cannot share its sessions. Update it." The phone tells the two apart by
  `welcome.mirror` being absent.

## Components

- `core/src/config.rs`: `mirror` on `PairedAssistant`.
- `core/src/net/protocol.rs`: the `mirror` cap, `Up::Request`,
  `RequestKind`, `Down::Mirror`, `Down::Response`, `Welcome.mirror`.
- `core/src/net/server.rs`: set `mirror` at pairing; `mirror` and
  `wantsMirror` in the status; `set_mirror(id, on)`; sending mirrors
  (debounced) with `ServerHandle::board_changed()`; taking requests to
  `Notify::request` on a worker thread.
- `core/src/net/client.rs`: `ClientNotify` gains `mirrored(allowed, cards,
  machines)` (default: ignore); `ClientHandle::request(machine, kind,
  timeout)`, matching each `response` to its waiting caller; the welcome's
  `mirror` reaches the notify.
- `core/src/net/promote.rs`: `own_entry` takes whether this Maya is a
  phone and sets `mirror`.
- `src-tauri/src/lib.rs`, `src-tauri/src/net_app.rs`: the routing bodies as
  shared functions; `Notify::request`; mirror on board change;
  `network_set_mirror(id, on)`.
- `mobile/src/hub.rs`: the mirror held beside the boards; `cards()` and
  `machines()` from it while joined; `network_pair`; `request` routing for
  the page's commands; `Notify::request` as a main; `server_start` stopping
  the client. `mobile/src/commands.rs`: the commands above route through
  the hub; `network_pair`, `network_set_mirror`.
- `mobile/src/alerts.rs`: seeding on the first mirror after a connect.
- `src/mobile/setup.ts`: Be the main or Join a main.
- `src/mobile/network.ts`: the Join form when Off; Connected to and Leave
  while joined; the per-Maya switch when it is the main.
- `src/mobile/main.ts`: the not-allowed card in place of the columns.
- `src/settings.ts`: the per-Maya switch.
- The `android` label in both pages.
- `docs/DEVELOPING.md`, the README's network and phone sections, and the
  `mobile/src/lib.rs` and `hub.rs` doc comments that say the phone is only
  ever a main.

## Testing

- Unit (core):
  - `request`, `response`, `mirror` and `welcome.mirror` encode and decode;
    a `hello` and a `welcome` without them decode as before; `RequestKind`
    has no `Promote`;
  - pairing with `mirror` in the `hello` saves `mirror: true`, without it
    `false`; an old config without the field loads as `false`;
  - `own_entry` sets `mirror` for a phone only;
  - the mirror's cards label the main's local cards with its name and
    leave out the receiver; `machines` puts the main first and leaves out
    the receiver.
- Integration (core, localhost): a main with assistant A and mirroring
  client P.
  - P receives a mirror with A's cards after its welcome and after A's
    board changes;
  - P replies to a session on A, with an attachment, and A receives it;
  - P starts a session on the main through a test `Notify::request`;
  - with the switch off, P's request is refused and P receives
    `allowed: false`; turning it on sends a mirror;
  - a request that waits on A does not hold up P's pings or mirrors.
- `mobile/tests/round_trip.rs`:
  - the phone joins a core main with a code and shows its cards;
  - it replies through the main;
  - the main promotes the phone, which runs its server with the list;
  - the phone promotes the core client back and mirrors it;
  - Leave sets it to Off.
- vitest: the Be the main or Join a main overlay and its errors; the Join
  form on the Network screen; Connected to and Leave; the not-allowed and
  older-main cards; stale cards while reconnecting; the per-Maya switch on
  the desktop and the phone; the Android label.
- By hand, with the user: the phone joins the Mac and replies to a session
  on the Mac and one on the laptop; a notification arrives on the phone;
  the switch off and on; Make main on the phone from the Mac; Make main on
  the Mac from the phone.

## Out of scope

- Desktop assistants or `maya-cli` mirroring the main's board.
- Reaching the main from outside the local network.
- Encrypting traffic other than the handover.
- A phone starting sessions on itself.
