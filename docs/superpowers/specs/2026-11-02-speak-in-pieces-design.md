# Speak long replies in pieces

## Intent

A local voice model (the OpenAI-compatible voice server, e.g. chatterbox)
garbles a long text towards its end. Maya should speak a long reply as
several short clips: one per paragraph, and a very long paragraph split
again at a natural stopping point. While one clip plays, the next one is
made, so the gaps stay short.

## Goals

- A reply with several paragraphs is spoken one paragraph at a time.
- A paragraph longer than a limit is split at a natural stopping point.
- The next piece is synthesised while the current one plays.
- The ear stays paused from the first piece to the last.
- Chat, the voice panel and the history still show the reply as one message.

## Non-goals

- The phone: it speaks with Android's own voice and does not change.
- Streaming audio from the server while it is made: the model would still
  get the whole text, so the garbling would stay.
- A setting for the limit.

## Anti-goals

- The ear opening in the gaps between pieces, so Maya hears herself.
- The voice switching back and forth between pieces after a failure.
- A piece cut in the middle of a word.

## Approaches

- **A. One utterance made of pieces (picked).** `speech.rs` turns the
  markdown into a list of pieces; the `Utterance` carries the list; the
  speech thread plays piece *n* while a helper thread synthesises piece
  *n+1* into the voice cache. The remote speaker is split into "make the
  clip" and "play the clip". The speech hook runs once around the whole
  utterance. Picked because it is the only way that keeps the ear closed
  and the voice steady for the whole reply.
- **B. Split in `reply_aloud_as` and queue one utterance per piece**, with
  a separate cache warm-up call. Smaller, but the hook pauses and resumes
  the ear around each piece, and each piece picks its voice again.
- **C. Stream the audio.** Dropped: it does not fix the model drift.

## Decided

- Split at paragraph ends (as `from_markdown` sees them: paragraphs,
  headings, list items). Consecutive blocks are joined back into one piece
  while together they stay under the limit, so a short list or a heading
  goes with its paragraph, not as tiny clips of its own.
- A block over the limit is split at sentence ends; a sentence still over
  the limit at `;`, `:` or `,`; last, between words. Never inside a word.
- The voice fallback chain stays as it is today. Once a piece fails on a
  voice, that voice is skipped for the rest of the utterance (it falls to
  the next voice in the chain, and last to the built-in voice). The voice
  may change partway through a reply; the user accepted that.
- Every voice speaks in pieces: ElevenLabs, the voice server and the
  built-in voice. A reply that fits in one piece behaves exactly as today.
- Each piece is cached on its own, by its own text.
- The speech hook's `Starting` carries the whole text, so the listener
  still recognises any part of it heard back.
- The `SpeechReport` for the utterance names the voice that spoke the last
  piece and lists every failure.

## My calls (check these)

- The limit is 400 bytes, about 400 characters of English (roughly 25 seconds of speech), one
  constant in `core/src/speech.rs`.
- Only remote voices synthesise ahead; the built-in voice (`say`, SAPI,
  `spd-say`) speaks each piece in turn, since it has no synthesis step to
  overlap.
- `speak_and_wait` waits for the whole utterance.

## Left to the builder

- The exact split of the remote speaker into synthesise and play, and how
  the prefetch thread reports back.
- The locking of the down-marks (`DOWN`), which today is held across
  synthesis and playback on the assumption that the speech thread is its
  only user.
- What happens to a prefetch still running when its piece is no longer
  needed (it may finish into the cache; that is harmless).
