# The phone speaks and listens — design

Date: 2026-10-16
Status: proposed
Builds on: `2026-10-07-maya-android-companion-design.md`,
`2026-10-14-phone-joins-a-main-design.md`

## Purpose

A phone that has joined a desktop main can talk to Maya as the desktop
can: the user speaks a command, the main's agent works out what it means,
and the phone speaks the answer. While the app is on screen the phone also
announces board events aloud. The microphone is used only while the app
is on screen.

Success: with the phone joined to the Mac and the app open, the user taps
the mic and says "answer the first one with Postgres". The phone reads
back "Answer maya settings on Yhi with Postgres?", the user says "yes",
the answer reaches the session and the phone says "Sent.". With the wake
word on, "Maya, what needs me?" works without a tap. When a session on
the laptop asks for a decision while the app is open, the phone says
"<name> on <machine> needs a decision". As soon as the app leaves the
screen, the phone stops listening and speaking.

In this document **M** is the desktop main and **P** the phone that has
joined it.

## Decisions

- The main interprets. P turns speech into text and sends it to M, whose
  agent (Maya's agent, as on the desktop) interprets it. P speaks the
  reply. Voice commands work only while P has joined a desktop main.
- P announces board events only while the app is on screen. In the
  background it keeps to notifications.
- Tap-to-talk is always available. A setting adds the "Maya" wake word
  while the app is on screen.
- P recognises speech with Android's `SpeechRecognizer` and speaks with
  Android's `TextToSpeech` (approach A). Whisper on the phone (approach B)
  stays possible later as a swap of the recogniser alone.

## Findings

- Interpreting is split, and only half of it is reusable.
  `maya_core::interpreter::run` (text in, `Reply{say, action, confirm}`
  out, no side effects), `validate` and `needs_confirm` are reusable. The
  glue is private to the desktop listener in
  `src-tauri/src/listener.rs`. `interpret` (300-426) gathers the board
  (`merged_cards`), projects and machines, PRs, history and the agent,
  speaks the reply, and stores the pending confirmation in the global
  `VoiceState`. `execute_action` (168-233) runs a validated action through
  the same Tauri commands the page uses. `on_heard` does nothing unless
  the desktop is listening. A phone conversation cannot use this as it
  is: it would share the desktop's history and pending confirmation.
- The wake word is text matching, not a detector.
  `src-tauri/src/wake.rs::extract` matches "maya" (and three variants) in
  each final transcript. `wake::Flow` is a pure state machine: idle,
  awaiting a command (8 s), awaiting a yes or no (10 s), "Maya …" replacing
  a pending action, the follow-up window after a reply that asks a
  question, and ignoring lines Maya speaks herself. It has no I/O, so it
  can run on the phone unchanged.
- The confirmation reads back the validated action (`spoken_for`,
  `listener.rs:949-966`), never the model's `say`. `needs_confirm` decides
  which actions need a yes (reply, answer, start, resume, review); the
  model's `confirm` flag does not.
- The link already carries requests with replies. A joined phone sends
  `Up::Request{id, machine, request}` and gets `Down::Response{id, ok,
  error, data}` (`core/src/net/protocol.rs`). The main serves them only
  when the mirror switch ("Sees and controls all sessions") is on
  (`server.rs:1070-1105`), through `net_app::serve_request` on a desktop
  main. The phone waits 40 s (`REQUEST_TIMEOUT`, `client.rs:60`); the
  interpreter's own limit is 25 s.
- An older main does not answer an unknown request: `decode_up` fails and
  the server only logs it (`server.rs:1004`). The phone would wait 40 s
  and then report a timeout. The phone must know beforehand whether the
  main can interpret. `Welcome` carries only `name` and `mirror`.
- Android has nothing for voice. The manifest has no `RECORD_AUDIO`.
  `notify::speak_line` is a no-op on Android (`core/src/notify.rs:254-256`).
  No code tracks whether the app is on screen, although
  `androidx.lifecycle:lifecycle-process` is already a dependency
  (`app/build.gradle.kts:96`). Kotlin joins Rust only through Tauri
  plugins, as `KeepAlivePlugin` does (`mobile/src/android.rs:49-73`).
- The phone already posts notifications for the mirrored board
  (`hub.refresh` → `alerts::plan`, `mobile/src/alerts.rs:113-140`). The
  desktop's spoken line is `notify::spoken_line` (`notify.rs:69-80`).
- `ear-rs` builds only for Windows and Linux, and Android cannot run a
  sidecar process. Whisper on the phone would mean linking whisper.cpp
  into `maya-mobile` through the NDK; that is out of scope here.

## Design

### Shared code (`core`)

- `wake.rs` moves from `src-tauri/src` to `core/src`, with its tests. The
  desktop listener imports it from `maya_core`. Behaviour does not change.
- Protocol (`core/src/net/protocol.rs`):
  - `Request::Interpret{command: String, history: Vec<Turn>}`, where a
    `Turn` is `{who: "user" | "maya", text}`. The reply's `data` is
    `{say: String, pending: Option<{say: String, action: Value}>}`.
  - `Request::Execute{action: Value}`. The reply's `data` is
    `{say: String}`.
  - `Welcome` gains `caps: Vec<String>` with `#[serde(default)]`. A main
    that can interpret sends `"voice"`. A main whose agent CLI cannot be
    found leaves it out. An older main sends no `caps` at all, which the
    phone reads as no voice. `PROTOCOL` stays 2: the field is optional and
    an older phone ignores it.

### The desktop main (`src-tauri`)

- The work in `listener::interpret` splits in two:
  - `interpret_text(app, command, history) -> Result<Interpreted, String>`
    gathers the context (merged cards, project dirs and machines, PRs,
    the agent and model), runs `interpreter::run`, validates the action,
    and returns `Interpreted{say, pending: Option<Pending>}`. A pending
    action is one `needs_confirm` marks, and its `say` is
    `spoken_for(&action)`. It speaks nothing and stores nothing.
  - The desktop listener keeps everything else: its history, its
    `Flow`, speaking the reply, and the listener generation checks. It
    calls `interpret_text` and acts on the result as today.
- `execute_action` becomes callable from `net_app` and returns the line
  to speak instead of speaking it. The desktop listener speaks that line.
- `net_app::serve_request` handles the two new requests. The server's
  mirror check already applies to every request, so a phone without the
  switch is refused before they get here.
  - `Interpret` calls `interpret_text` with the phone's history.
  - `Execute` validates the action again against the current board
    (`interpreter::validate`) before `execute_action`. An action whose ask
    has changed, whose session has ended or whose machine has gone fails
    with the reason as `error`.
- The two requests leave the desktop's own conversation alone: they use
  neither `VoiceState.history` nor its pending confirmation, and nothing
  is spoken on the desktop.
- `Welcome.caps` includes `"voice"` when `launch::find_binary` finds the
  configured agent.

### The phone, Rust side (`mobile/src`)

A new `voice` module owns:

- the phone's `wake::Flow`, its history (at most 12 turns, the same
  shape the desktop sends to the interpreter), and whether the app is on
  screen;
- the conversation: `Interpret` and `Execute` through
  `hub.request(None, …)`, then speaking `say`;
- speaking board events (below);
- a generation counter, bumped whenever the app leaves the screen or the
  conversation resets, so a late answer is dropped.

The recogniser and the voice sit behind a `Voice` trait (start and stop
listening, speak, stop speaking), so the module is tested on the host
with a fake. The Android build implements it with the plugin.

### The phone, Android side

A new Tauri plugin, `VoicePlugin.kt`, registered next to `KeepAlivePlugin`:

- `SpeechRecognizer`. Where the phone has an on-device recogniser (API
  31+, `isOnDeviceRecognitionAvailable`), it uses
  `createOnDeviceSpeechRecognizer`; otherwise the default one. Partial
  and final results, errors and the end of speech arrive as plugin
  events.
- `TextToSpeech`. It speaks a line and reports when it has finished, or
  that it failed.
- On-screen state from `ProcessLifecycleOwner` (`ON_START` / `ON_STOP`),
  as plugin events.
- The quiet state for events: the ringer mode, and
  `NotificationManager.currentInterruptionFilter` for Do Not Disturb.

`RECORD_AUDIO` goes in the manifest and is asked for at runtime the first
time the mic is used. Nothing listens off screen, so no
`FOREGROUND_SERVICE_MICROPHONE` and no change to the keep-alive service.
The hand-edited files list in `docs/DEVELOPING.md` gains the plugin.

### The phone page (`src/mobile`)

- A mic button on the board. Tapping it again while listening stops it.
- A line under the board's strip showing what was heard (partial text,
  then the final) and what Maya said, with *Thinking…* while the main
  interprets.
- Yes and No buttons while a confirmation is pending.
- A **Voice** card on the Settings tab: **Speak** and **Wake word "Maya"
  (experimental)**, with the hints below.

## Flows

### Tap-to-talk

1. The user taps the mic. `Flow` moves straight to awaiting a command:
   the tap shows intent, so there is no wake word and no "Yes?".
2. The recogniser listens for one utterance and shows partial text. The
   final text goes out as `Interpret{command, history}`, and the line
   shows *Thinking…*.
3. The reply:
   - With no pending action (a report, focus, a question back), the phone
     speaks `say`. When it ends in "?", the mic reopens for a follow-up
     without a tap, as on the desktop.
   - With a pending action, the phone speaks its read-back, shows Yes and
     No, and once the read-back ends listens 10 s for a spoken yes or no.
4. Yes sends `Execute{action}`, and the phone speaks the result. No, or
   no answer within 10 s, gets "OK." and nothing runs.

Both turns of each exchange join the history.

### Wake word

While the setting is on, the app is on screen and the on-device
recogniser is available, the recogniser runs in a loop and restarts after
each result or error. Each final transcript goes through `wake::extract`:
"Maya" alone gets "Yes?", then the tap flow from step 2; "Maya, answer the
first one" goes straight to `Interpret`. The loop pauses while the phone
speaks, using the voice's finished event and `Flow`'s ignore-line, so it
does not hear itself.

### Leaving the screen

The phone stops listening and speaking at once, drops a pending
confirmation, and bumps the generation, so the answer to an `Interpret`
still in flight is ignored. An `Execute` already sent completes: the user
confirmed it. Coming back on screen restarts the wake word loop when it
is on, and nothing else.

### Board events

With Speak on and the app on screen, when `hub.refresh` posts a new
"needs a decision" or "is finished", the phone also speaks
`notify::spoken_line` for it. The notification is still posted, so the
shade keeps it, but on a new channel "spoken" (low importance: no sound,
no vibration, no heads-up), so no sound clashes with the speech. The
notification plugin's `silent()` works only on iOS, and on Android the
sound belongs to the channel, so the quiet channel is the way to post
without one. An event the phone does not speak (quiet rules, Speak off)
keeps its usual channel. Events wait while a conversation is under
way and are spoken once it is idle. Events are not spoken when the phone
is on silent or vibrate or in Do Not Disturb. Replies to something the
user just asked are always spoken.

### When voice is not offered

| Case | The phone shows |
|---|---|
| The phone is its own main | No mic; the Voice card says "Voice needs a desktop main." |
| The main has no `voice` in `caps` | The mic disabled: "Update <main> to talk to Maya." |
| The mirror switch is off | The existing "not allowed" card instead of the board. |

## Errors

| Case | Behaviour |
|---|---|
| Mic permission refused | The mic shows "Microphone access is off" with a button to open the app's settings; the wake word setting turns off. |
| No recogniser at all | The mic is disabled with the reason. |
| Only an online recogniser | Tap-to-talk works, with a hint that Google's service may send the audio online. The wake word needs the on-device recogniser, so an always-open mic never streams audio off the phone. |
| The wake word loop keeps failing | Backs off from 1 s to 16 s, then stops after 5 failures in a row: "Wake word stopped: <reason>". |
| The main does not answer, or the link drops | "<main> didn't answer in time." or "Lost the connection to <main>." The conversation resets. |
| `Execute` refused (the ask changed, the session ended, the switch went off) | The phone speaks the main's reason. |
| No text-to-speech voice | Replies show as text only, with a hint to install a voice. |

## Settings

- **Speak** reuses the config's `speak_notifications`, on by default. It
  covers board events on screen; replies are always spoken.
- **Wake word "Maya" (experimental)** is a new `wake_word` field, off by
  default.
- The phone's `set_config` (`mobile/src/commands.rs:233-242`) learns both.

## Testing

- `core`: `wake`'s tests move with it. Protocol round trips for
  `Interpret`, `Execute` and their replies, and `Welcome` with and without
  `caps`, so an old main's welcome still parses.
- Desktop main: `interpret_text` against a stub agent. `serve_request`
  serving `Interpret` and `Execute`, and `Execute` failing cleanly on a
  stale action. The desktop listener's tests keep passing on the split.
- Phone Rust, on the host with a fake `Voice`: tap flow, wake flow; yes,
  no and timeout; leaving the screen drops a pending action, stops
  speaking and ignores a late answer; events wait for a conversation and
  respect the quiet rules; the not-offered states.
- Phone page (vitest): the mic button's states, the heard and said line,
  Yes and No, the Voice card.
- By hand, on the phone joined to the Mac, since the recogniser and the
  voice have no automated tests: tap-to-talk with a report and with a
  confirmed answer; the wake word; leaving the screen mid-confirmation;
  an event spoken on screen and not on silent; permission refused.

## Out of scope

- Listening or speaking while the app is not on screen.
- Voice when the phone is its own main.
- The ElevenLabs voice on the phone.
- Whisper on the phone (approach B), kept as a later swap of the
  recogniser alone.
- Keeping the desktop and the phone from announcing the same event.

## Release

A `feat`: a minor bump from `main`'s version at the time. The desktop
main and the phone ship in the same release.
