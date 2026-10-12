# Promote an assistant to main — design

Date: 2026-10-12
Status: implemented
Builds on: `2026-09-30-maya-remote-assistants-design.md`,
`2026-10-07-maya-android-companion-design.md`

## Purpose

A main Maya can make one of its assistants the main. Every connected Maya,
the old main included, then assists the new main, without pairing again.
The phone, which can only be a main, stays linked as a listener with no
sessions, so the new main can make it the main again later.

Success: on the Mac (main) the user presses Make main beside the laptop;
seconds later the laptop's board shows the Mac's, the laptop's and every
other assistant's sessions, and nothing on any machine asked for a pairing
code.

In this document **M** is the main that promotes, **G** the assistant it
promotes, and **B** any other assistant of M.

## Findings

- An assistant's id and token are what the main checks at every
  connection, and the handshake is mutual: the main proves it holds the
  token too (`server.rs`, `client.rs`). A main that holds an assistant's
  token can therefore take it over without pairing; the assistant only has
  to dial a different host.
- Tokens cross the wire once today, in plain text, in `paired`. Handing
  the whole paired list to G in plain text would let one captured frame
  give away every machine's token, so the handover is sealed with a key
  only G can derive.
- `maya-cli` is only an assistant, and the phone only a main. Neither can
  do everything, so each Maya says what it can do in `hello`.
- On the desktop a role change stops the client and joins its thread
  (`NetChange::StopAssistant`). G receives `promote` on that thread, so it
  must answer first and stop its client from another thread.

## Behaviour

### Capabilities

- `hello` gains `caps`, a list of strings; a Maya that sends none (an older
  one) has none.
  - `follow`: it understands `move` and follows the main to a new host.
  - `lead`: it can become a main when sent `promote`.
- The desktop app sends `follow` and `lead`, the phone `follow` and `lead`,
  `maya-cli` `follow`.
- The main keeps each connected assistant's caps; `AssistantStatus` gains
  `canLead` and `canFollow` (false while it is not connected).

### The handover

1. **Confirm on M.** Make main on G opens a confirmation under G's row
   (inline, as the Network section has no dialogs): "Make <G> the main
   Maya? Every connected Maya will assist <G>, and so will this <Mac|PC|
   phone>." It then lists, when there are any:
   - connected Mayas without `follow`: "<name> cannot follow and stays
     with no main; pair it with <G> afterwards.";
   - paired Mayas that are offline: "<name> is offline; when it is back,
     set its Host to <G's address>. It does not need to pair again."
   Buttons: Make main, Cancel. The lines are worked out on the page
   (`src/promote.ts`) from the network status.
2. **M builds the list.** It closes the pairing window. The list is every
   paired entry except G, plus a new entry for M itself: a new id and
   token, M's name, hostname and platform, its address left empty (G
   records it at M's first authentication).
3. **M sends `promote` to G.** A command whose payload is that list,
   sealed (see Sealing). G opens it, saves its config as Main with the
   list as its paired assistants and its own `port`, and drops
   `mainHost`, `mainPort`, `assistantId` and `token`. It starts its
   server.
   - If the seal does not open, the save fails or the server cannot
     listen, G puts its old config back (the server stopped, the client
     still running) and answers `ok: false` with the error. M shows
     "<G> could not become the main: <error>." and changes nothing more;
     its pairing window stays closed until the user opens it again.
   - Otherwise G answers `ok: true` with `data: { "port": <port> }`.
     After the result is sent it stops its client from another thread.
4. **M moves the others.** M sends every other connected assistant with
   `follow` `move { host, port }`: `host` is G's address as M sees it,
   `port` the one G returned. Each saves `mainHost` and `mainPort`, keeps
   its id and token, and reconnects at once (no backoff for this one
   reconnect). M then closes their connections.
5. **M becomes an assistant.** M saves its config as Assistant to G (host
   and port as in `move`, the id and token of the entry it made in step
   2), with no paired list, stops its server and starts its client. On
   the desktop the existing assistant rules apply: notifications and
   speech are suppressed, and "Listen for 'Maya'" is off and disabled.
   It also turns "Listen for 'Maya'" off, as pairing does. On the phone
   the Network screen shows Listening (below).
   - If M's save fails, M stays main with its paired list and server,
     shows "Could not save this Maya as an assistant: <error>.", and the
     assistants already moved to G stay with G.
6. **G greets everyone without noise.** G holds no board for anyone when
   they connect, so the existing seeding rule holds: nothing on their
   first board is announced.

- **G does not answer within 30 s.** M stays main and moves nobody, and
  says "<G> did not answer; it may have become a main on its own. Check
  it before trying again." This is the only case where two mains can
  exist; no assistant has been told to move, so all of them stay with M,
  and G is a main with nobody connected, put right in G's Settings.
- **G disconnects after `ok`, before M moves the others.** Expected (it
  stopped its client); the move goes on.
- **A B cannot reach G** (a firewall): it shows "Reconnecting…" against
  G's address, as for any main that is down. M dials G in step 5 too, so
  the same failure shows on M at once.
- **An older B** (no `follow`) is not sent `move`. When M's server stops
  it shows "Reconnecting…" against M until it is paired with G.
- **Only one handover at a time**: Make main is refused with "A handover
  is already running." while one is.

### Sealing

- Key: HKDF-SHA256 (extract and expand over `hmac`/`sha2`) of G's token,
  salt empty, info `maya promote v1`, 32 bytes.
- Cipher: ChaCha20-Poly1305 with a random 12-byte nonce; `sealed` is the
  base64 of nonce followed by ciphertext. The plaintext is the JSON
  `{ "assistants": [PairedAssistant…] }`.
- A seal that does not open (another token, a changed byte) is refused
  with "The handover could not be opened." Only `promote` is sealed;
  `move` carries a host and a port, which are not secret.

### What the user sees

*On a main, desktop or phone:*

- Each paired assistant that is connected and can `lead` gets a Make
  main button beside Remove. While a handover runs the Network section's
  buttons are disabled; a failure shows where Remove's errors show.

*On G (desktop):*

- The Network section shows the main's view by itself: "Act as main
  Maya" ticked, the paired list, port and name. Notifications and speech
  follow the saved role, so they come back. "Listen for 'Maya'" is
  enabled again but stays off (pairing turned it off) until the user
  turns it on.
- The Debug tab logs, under `network`, "promoted to main by <M>; <n>
  assistants handed over".

*On a B that moved, and on M once it is an assistant:*

- The status line shows "Reconnecting…", then "Connected to <G>". Host
  in Settings shows G's address. Nothing to click.

*On the phone, after it was M:*

- The Network screen has a third state beside the server running and
  stopped: "Listening to <G>" (or "Reconnecting…" with the last error),
  and one button, Stop listening.
- The board is empty and posts no notifications. The keep-awake service
  keeps running while it listens.
- Stop listening sets the phone to Off and forgets the id and token. G
  then shows it as disconnected; the user removes it there.
- Every command but `promote` is answered "This phone runs no
  sessions." When G promotes the phone, the phone starts its server with
  the list it was given and shows its usual main screen.

## Protocol

Additive; the protocol stays at version 1.

- Up: `hello { …, caps? }`.
- Down: `move { host, port }`.
- `command` kind `promote { sealed }`; its `result` is `ok: true, data:
  { port }` or `ok: false, error`.
- A Maya that cannot decode `move` (an older one) logs and ignores it;
  M sends `move` only to assistants with `follow` anyway.

## Components

- `core/src/net/protocol.rs`: `caps` on `Hello`, `Down::Move`,
  `CommandKind::Promote`, `seal` and `open`.
- `core/src/net/promote.rs` (new, pure): M's own entry, the handover
  list, sealing it, and the config rewrites `as_main(config, list)` and
  `as_assistant_of(config, host, port, entry)`.
- `src/promote.ts` (new, pure): the confirmation's lines, shared by the
  desktop and the phone.
- `core/src/net/server.rs`: caps per connection, `canLead` and
  `canFollow` in the status, `move` ending a connection as `bye` does,
  `ServerHandle::promote(id, now)` for steps 2–4, returning what M saves
  in step 5; one handover at a time.
- `core/src/net/client.rs`: `Promote` is caught before the executor and
  opened with the client's token; `ClientNotify` gains `promoted(list) ->
  Result<u16, String>` (default: "This Maya cannot become a main.") and
  `moved(host, port)`; `Move` saves through `moved` and reconnects at
  once. The caps the client sends come from its caller.
- `src-tauri/src/net_app.rs`: `promoted` (save, start the server, put the
  old config back if either fails, stop the client on another thread
  after the result is sent) and `moved`; caps `follow`, `lead`.
- `src-tauri/src/lib.rs`: `network_promote(id)`.
- `mobile/src/hub.rs`: a client beside the server (start, stop, the
  Listening status), an executor with an empty board, `promoted` and
  `moved`; `mobile/src/commands.rs`: `network_promote` and `listen_stop`.
- `cli/`: `moved` saves the config; caps `follow`.
- `src/settings.ts` (Make main, the confirmation, `canLead`) and
  `src/mobile/network.ts` (Make main, the Listening screen).
- `docs/DEVELOPING.md` and the README's network section.
- Crate: `chacha20poly1305`.

## Testing

- Unit:
  - seal and open round-trip; another token or one changed byte is
    refused;
  - the handover list leaves G out and adds M's entry with a new id and
    token;
  - the confirmation's lines: those left behind and the offline ones
    (vitest);
  - `as_main` and `as_assistant_of`;
  - a `hello` without `caps` decodes with none; `move` and `promote`
    encode and decode.
- Integration (core, localhost):
  - M's server with clients G and B: promote; G listens with B's entry;
    B moves and authenticates with its old token; M connects to G as an
    assistant; nothing on the first boards is announced;
  - G's port is taken: `ok: false`, G is still an assistant of M, nobody
    moves;
  - G never answers: M stays main, nobody moves;
  - an assistant without `follow` is not sent `move`;
  - a second handover while one runs is refused.
- `mobile/tests/round_trip.rs`: the phone promotes a core client and
  goes to Listening; that client promotes the phone back, and the phone
  runs its server with the list.
- vitest: Make main only for connected assistants with `lead`; the
  confirmation; the phone's Listening screen and Stop listening.
- By hand, with the user: the Mac, the laptop and the phone. Promote the
  laptop; see every session on the laptop's board; promote the phone
  from the laptop; promote the Mac from the phone.

## Out of scope

- The phone mirroring the new main's board while it listens.
- Telling offline assistants about the move; they are set by hand.
- An assistant taking the main role on its own (only the main gives it
  away).
- Encryption of the rest of the traffic.
