# The phone is always a mirror, and gets Chat — design

Date: 2026-10-26
Status: draft
Builds on: `2026-10-14-phone-joins-a-main-design.md`,
`2026-10-16-phone-voice-design.md`,
`2026-10-21-maya-brain-assistant-design.md`

## Purpose

The phone has no Chat. On the desktop, Chat is the user's typed
conversation with Maya's brain. On the phone, the only way to reach her is
by voice. The user wants the same Chat on the phone.

A phone that is the main has no brain, because it runs no agents. To give
it one, it would have to borrow a brain from one of its assistants, and
that brain would need the phone's whole board. The user chose the simpler
model: **the phone is never the main.** It always joins a main computer
and mirrors it. Maya's brain is therefore always the main computer's, and
the phone's Chat is that brain's Chat.

Success: on a phone joined to the Mac, the Chat tab shows the same
conversation as the Mac's Chat tab, including turns that are spoken on the
phone or the Mac while her session runs. A message typed on the phone reaches Maya. Her reply
comes up on both screens within a second. The 5-second Cancel and Stop on
the phone work as they do on the Mac. The phone's first screen offers only
Join.

In this document, **M** is the main computer and **P** is a phone that
joins it.

## Findings

- The desktop Chat (`src/chat.ts`, `initChat`) uses the commands
  `chat_history`, `chat_send`, `chat_seen`, `brain_status`,
  `window_cancel` and `brain_stop`, and the events `chat` (one
  `ChatTurn`) and `brain` (`BrainStatus`). `chat.ts` has no desktop-only
  parts.
- Each of the two events is emitted from one place:
  `listener.rs:108` (`chat`) and `brain_app::emit_status` (`brain`).
- The conversation is `chat.json` on M, at most `chat::KEEP` (500) turns.
- P's spoken requests (`Request::Interpret` and `Request::Execute`) already
  go to M's brain while her session is ready, and their turns already go
  into M's Chat with `via: "phone"`. While her session is down
  (`serve_voice`'s fallback to the one-shot interpreter), they do not
  reach Chat, and only failures of `Execute` are added to it. `Via::Phone` is tied to that spoken path: a waiter in
  `brain_app` holds the reply so P can speak it.
- `Down::Mirror` carries the cards, the machines, the remembered rows and
  the wake word. It carries no chat. The server sends it on a tick, and no
  more often than `MIRROR_GAP`. To send a frame to one link at once, the
  server uses `conns[id].tx` (see `set_mirror`).
- The main tells a mirroring Maya what it can do in `Down::Welcome.caps`.
  `CAP_VOICE` is the precedent for adding a capability.
- Code that makes the phone a main:
  - `mobile/src/hub.rs`: `start_server`, `stop_server`, `pairing_code`,
    `remove_assistant`, `set_mirror`, `promote`, `serve_request`, the
    `NoSessions` executor and the main side of `status`;
  - `mobile/src/commands.rs`: the commands that call these;
  - `src/mobile/setup.ts`: the "Be the main" and "Join a main" choice, and
    the "server is stopped" card;
  - `src/mobile/main.ts`: `paintHint` ("Pair an assistant…");
  - `src/mobile/network.ts`: the main's view of the Network tab;
  - `phone_caps()` sends `CAP_LEAD`;
  - the keep-alive notification text ("Main for 2 assistants, 1
    connected").
- Code on the desktop that only exists for a phone that becomes the main:
  - the Make main button on a phone's row (`src/settings.ts:1450` shows it
    for any connected row with `canLead`, and a phone sends `lead`);
  - `HandoverTo.phone` and `Server::set_own_mirror`
    (`core/src/net/server.rs`), with `ctx.own_mirror`.

## Behaviour

### The phone never runs a server

- The first screen offers only Join: the name, address, port and code
  fields with the Join button. The "Be the main" and "Join a main" choice
  goes.
- The Network tab of a joined P shows the main it is joined to, the
  connection state, Keep Maya awake, the notification and voice settings,
  and Leave (`stop_listening`). It shows no pairing code, no list of
  assistants and no Start or Stop for a server.
- After Leave, or when P has never joined, the board shows the Join card
  in place of "The server is stopped".
- `phone_caps()` sends `follow`, `mirror` and `chat`, and no longer
  `lead`.
- The keep-alive notification describes the main only ("Joined to desk",
  "Reconnecting to desk…").
- The phone-as-main code listed in Findings is removed. Nothing in the
  phone still starts `net::server`.

### Upgrading a phone that is the main

The user chose to set this up by hand.

- When P starts with a saved `network.role` of `Main`, it sets the role to
  `Off`, clears the paired assistants from its config, and logs one line
  ("this phone is no longer a main: join a main computer"). Then it shows
  the Join screen.
- The computers that were paired to P keep trying to reach it until the
  user changes them. The README says what to do: on one computer choose
  Main, then join the phone and the other computers to it.
- There is no automatic handover, and no prompt to pick a computer.

### The desktop

- A row that wants to mirror (`wantsMirror || mirror`) never shows Make
  main, even when it sends `lead`. Older phones still send `lead`.
- `Server::promote` refuses a peer that asked to mirror ("A phone cannot
  be the main."), so an older page or the CLI cannot do it either.
- `HandoverTo.phone`, `set_own_mirror` and `ctx.own_mirror` are removed.
  So is the "none for a phone" branch of the handover summary.
- A desktop that is the assistant of an older phone's main keeps working
  as it does now. Nothing is done to cut it off.

### Chat on the wire

A new capability, `CAP_CHAT` (`"chat"`), goes both ways: M sends it in
`Welcome.caps`, and P sends it in `Hello.caps` (`phone_caps()`).

Two new `Down` frames go only to links that asked to mirror, that sent
`chat`, and that the main's switch allows (`wants_mirror && wants_chat &&
may_mirror`). An older phone would log "ignored a message: bad message"
for every frame it cannot decode, so it gets none. They are sent through
the link's `tx` as soon as they happen, not on the mirror tick:

- `Down::Chat { turn }`: one new turn, the same JSON as the page's `chat`
  event.
- `Down::Brain { status }`: the status, the same JSON as the page's `brain`
  event.

The desktop app sends both from the same two places where it emits the
page events. The server gets a method that sends a frame to every link
that is allowed (for example, `Server::to_mirrors(Down)`).

Five new `Request` variants. Each one is answered as the desktop command
of the same name would answer it:

| Request | Answer |
|---|---|
| `ChatHistory` | `{ turns, status }`: the last `KEEP` turns and the status now |
| `ChatSend { text }` | the same as `chat_send` (it cancels a pending batch, then calls `ask(…, Via::Chat)`) |
| `ChatSeen` | the same as `chat_seen` |
| `WindowCancel` | the same as `window_cancel` |
| `BrainStop` | the same as `brain_stop` |

The main refuses these requests as it refuses every other request when its
switch for P is off.

`PROTOCOL` stays at 2: the change only adds frames and requests. An older
phone does not send `chat`, so it gets none of the new frames. A new phone joined to an older main does not
see `CAP_CHAT` and never sends these requests.

### Chat on the phone

- The tab bar is Sessions, then **Chat** with its dot, then the spacer,
  then Debug and Network.
- `src/mobile/main.ts` calls the same `initChat()`. The phone's crate
  serves `chat_history`, `brain_status`, `chat_send`, `chat_seen`,
  `window_cancel` and `brain_stop` by forwarding the requests to M. It
  emits `chat` and `brain` events when it gets `Down::Chat` and
  `Down::Brain`. `chat_history` and `brain_status` both come from the
  `ChatHistory` answer, which the hub keeps. The page code stays shared.
- On every `connected` from a main with `CAP_CHAT`, the hub asks for
  `ChatHistory` again and gives the page the whole list. Turns missed
  while P was disconnected therefore come back. `initChat` needs a way to
  take a whole new list (a `chat-reset` event, or a call that replaces
  the turns).
- When the main's switch for P is off, the Chat tab is hidden. When the
  main has no `CAP_CHAT`, the Chat pane shows "Update {main} to chat from
  the phone." in place of the input, which is the existing dormant note.
- While P is disconnected, Chat keeps what it has and shows
  "Reconnecting to {main}…" in place of the input.
- The composer has a Send button (see Builder's choices).

## Approaches

The user picked the approach for how the conversation reaches the phone:

1. **M pushes the chat as it happens (picked).** Two small new frames,
   `chat` and `brain`. P asks for the history once on connect. P's typing,
   Cancel and Stop are `request`s. Chat stays live and costs nothing when
   it is idle. The cost is two new frame types.
2. **The chat goes inside `Down::Mirror`.** There is no new frame type,
   but the whole conversation would go out on every board change, and a
   mirror would have to go out on every chat turn. Rejected as wasteful.
3. **P polls M** for the history and status every few seconds. This is the
   least code on M, but Chat lags, the countdown jumps, and polling uses
   the battery. Rejected because it does not feel like the desktop.

Earlier, the user also rejected a phone-main that borrows a brain from an
assistant. That would need the borrowed brain to see and act on the whole
pool through the phone.

## Goals, non-goals, anti-goals

Goals:
- The phone's Chat is M's Chat, live, with Send, Cancel and Stop.
- The phone is never a main. The phone and the desktop have no code that
  makes it one.

Non-goals:
- Moving a phone that is the main over to the new model automatically.
- Attachments in Chat (the desktop Chat has none).
- Chat in `maya-cli`.
- Putting the phone's spoken turns into Chat while M's session is down.
  The fallback path stays as it is (see Findings).
- Chat on a desktop assistant. Its Chat stays read-only and dormant.

Anti-goals:
- A second conversation on the phone that drifts from M's.
- A Chat on the phone that shows text the main's switch hides.
- Breaking a phone that is joined to an older main, or an older phone that
  is joined to a newer main.

## Builder's choices (made here; change them if the code argues otherwise)

- **Typed messages from the phone use `Via::Chat`**, not `Via::Phone`.
  `Via::Phone` is tied to the spoken reply waiter. Typed messages from the
  phone therefore show `via: "chat"`, the same as typed messages on the
  desktop.
- **`chat_seen` on the phone marks M's Chat as seen.** There is one
  conversation and one reader, so reading it on either screen clears the
  dot on both.
- **The countdown uses M's `until` as it comes.** Phones and computers on
  the same network keep their clocks within a second of each other.
- **The phone's composer has a Send button**, and its keys work as the
  phone's reply box's do (`composerKey`, the user's chosen keys).
  `renderChat` has no Send button today, so it gets one that the phone
  turns on.
- The `BrainStatus` shape can move into `maya_core` so that the protocol
  can name it, or the frames can carry it as `Value`. Choose whichever
  keeps the tests simpler.

## Left to the builder

- Where the new server method sits, and how its tests drive a link.
- Whether `initChat` takes a "replace all turns" event or exposes a
  function.
- The exact wording of the log line and the notification.
- The order in which to remove the phone-main code, as long as every
  commit builds and its tests pass.

## Tests

- `core`: the new frames and requests round-trip. `Chat` and `Brain` go
  only to links that are mirroring and allowed, and they stop when the
  switch goes off. `promote` refuses a link that is mirroring. `Welcome`
  carries `chat` when the app says so. A mirroring link without `chat` in
  its `hello` gets no `Chat` or `Brain` frame.
- `src-tauri`: `serve_request` answers each chat request the way its
  command does. Rows that want to mirror get no Make main.
- `mobile`: in `round_trip.rs`, a joined hub gets the history on connect
  and again after a reconnect, forwards `chat_send`, and turns
  `Down::Chat` and `Down::Brain` into events. A saved `Main` role loads as
  `Off` with no assistants.
- Page: the phone's first screen has no "Be the main". The Chat tab is
  hidden when the main's switch is off, and shows the update note without
  `CAP_CHAT`.
- By hand: the success scenario in Purpose, on a real phone and the Mac.

## Docs and release

- README: rewrite the Android section so that the phone always joins a
  main. Add a note for phones that were the main, and mention Chat.
- `docs/DEVELOPING.md`: rewrite the Android paragraph ("It is a main, or
  it joins a main…") and the `hub.rs` description.
- Version: this removes a mode that users have, so it is a breaking
  change. Before 1.0 that means a minor bump from `main`'s version (0.34.0
  today, so 0.35.0), set with `sh scripts/set-version.sh` in a separate
  `chore(release)` commit.
