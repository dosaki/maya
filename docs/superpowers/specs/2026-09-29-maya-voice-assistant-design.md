# Maya: voice assistant with the "Maya" wake word — design

Date: 2026-09-29
Status: draft for review
Builds on: `2026-09-28-eye-session-board-design.md`, `2026-09-28-eye-reply-modal-design.md`

## Purpose

Talk to Maya. Say "Maya, what's waiting on me?" or "Maya, tell hexgrid to go
ahead" and she answers out loud and acts on the board. Listening is local and
always on while Maya runs; only the text of a command leaves the Mac, and only
to the model that interprets it.

This is the first Maya feature that needs judgement rather than rules. The
judgement lives in an on-demand model call per utterance, the same shape as
the "Let Claude choose" directory picker. Nothing runs in the background
except the listener, which is plain speech recognition.

## Findings (verified on this Mac)

- macOS on-device speech recognition (`SFSpeechRecognizer`) is available and
  supports continuous recognition in `en-GB`. Microphone permission is already
  granted; the speech permission is not yet requested and will prompt once.
- On-device requests are capped at about a minute, so the listener restarts
  its recognition request on a timer.
- The Mac has virtual audio devices installed (BlackHole). The system default
  input cannot be trusted; the listener must pick the built-in microphone by
  preference.
- Claude Code accepts a prompt on the command line in print mode with a JSON
  output format, so the interpreter can be a one-shot `claude -p` call.

## Behaviour

### Listening

- A small Swift helper, `maya-ear`, is a Tauri sidecar. It opens the
  preferred microphone (built-in first, then any non-virtual input, never
  BlackHole or "Remote Sound" devices), runs on-device continuous recognition,
  and writes one JSON line per event to stdout: `partial` and `final`
  transcripts, `state` changes (listening, paused, error), and a `level`
  reading a few times a second for the indicator.
- Maya starts the helper when "Listen for Maya" is on in Settings and stops
  it when it is off or Maya quits. Nothing listens when Maya is not running.
- While Maya speaks, listening pauses so she does not hear herself.

### Wake word

- A `final` segment containing "maya" (also "maia", "my a", "mya" as the
  recogniser tends to spell it) wakes her. The command is the text after the
  wake word to the end of the segment.
- If the wake word ends the segment ("Maya." then a pause), she says "Yes?"
  and takes the next `final` segment as the command, giving up after eight
  seconds of silence with no reply.
- The indicator in the top bar shows grey while idle, amber while she is
  waiting for a command or a confirmation, blue while she thinks, and a
  muted icon when listening is off. Clicking it toggles listening.

### Understanding

- The command text, the board summary (each session's name, harness, state,
  project and any open ask), and the last six voice exchanges go to
  `claude -p` with Haiku and JSON output, under a fixed system prompt that
  lists the allowed actions.
- The model returns `{ "say": "...", "action": {...} | null, "confirm": bool }`
  where `action` is one of:
  - `report` — nothing to do, the answer is in `say`
  - `reply { session, text }` — send text to a session (inbox or typed)
  - `answer { session, option }` — pick an option on an open question
  - `focus { session }` — bring the session's terminal forward
  - `compact { session }`
  - `resume { dir, session }` and `start { dir, prompt }`
- Maya validates the action before doing anything: the session must be on
  the board, the option must exist, the folder must be a project folder.
  Anything invalid becomes a spoken "I couldn't find a session called …".

### Confirmation

- Actions that send text, answer a question, start or resume a session are
  read back first: "Telling hexgrid: go ahead and push. Yes?" The next
  utterance within ten seconds decides, without the wake word: "yes", "go
  ahead", "do it" confirm; "no", "cancel", "stop" abort; anything else is
  treated as a new command. Report, focus and compact run at once.
- The voice panel shows the same pending action with Yes and No buttons.

### Speaking

- Replies use the existing voice path (Samantha or ElevenLabs). A reply to
  something the user said is spoken even under a Focus mode, since the user
  addressed her; only unsolicited announcements stay silent.

### Privacy and control

- Audio never leaves the Mac. Recognised text is kept in memory only, and
  only the wake-word command text and the board summary go to the model.
- Settings: "Listen for 'Maya'" (off by default), the microphone to use
  (preferred device by default), and the interpreter model (Haiku by default).
- A voice panel, opened from the indicator, lists what she heard, what she
  said, and any pending confirmation.

## Components

- `ear/main.swift` — the sidecar. Built by `pnpm ear:build` into
  `src-tauri/binaries/maya-ear-<target>` and by the release workflow for both
  architectures.
- `src-tauri/src/ear.rs` — starts and stops the sidecar through the shell
  plugin, parses its lines, and emits `voice` events to the page.
- `src-tauri/src/wake.rs` — wake-word extraction and the confirmation state
  machine, pure functions over transcript segments.
- `src-tauri/src/interpreter.rs` — builds the prompt, runs `claude -p`,
  validates the returned action against the board.
- `src-tauri/src/lib.rs` — `voice_listen(on)`, `voice_confirm(yes)`,
  `voice_history` commands; action execution reuses the existing commands.
- `src/voice.ts` — the indicator and the panel.

## Testing

- Unit: wake-word extraction across the recogniser's spellings and positions;
  the confirmation state machine, including the timeout and "anything else is
  a new command"; action validation against a fake board; sidecar line
  parsing; microphone preference ordering; the interpreter prompt shape.
- Sidecar alone: run `maya-ear` in a terminal and read recognised text.
- End to end: "Maya, what's waiting on me?" answered aloud; "Maya, tell
  voice-test to go ahead" read back, confirmed with "yes", and delivered.

## Out of scope

- Custom wake words and languages other than English.
- Listening while Maya is not running, and a menu-bar-only mode.
- Interrupting her mid-sentence.
- Voices other than the existing built-in and ElevenLabs options.
