# Maya: built-in speech recognition with Whisper — design

Date: 2026-09-29
Status: draft for review
Builds on: `2026-09-29-maya-voice-assistant-design.md`

## Purpose

Let Maya listen on Macs where Apple's speech recognition cannot run. The
voice assistant relies on `SFSpeechRecognizer`, which macOS gates behind the
Dictation setting. A management profile that sets `allowDictation` to false
(as on the developer's own Mac) locks that switch, and every recognition
request then fails with `kLSRErrorDomain 201`.

The fix is a second recogniser inside the same sidecar: whisper.cpp, running
on this Mac with a model the user downloads once. A setting chooses between
the two. Apple's stays the default.

## Findings (verified on this Mac, 2026-09-29)

- whisper.cpp v1.9.2 publishes `whisper-v1.9.2-xcframework.zip` (53 MB,
  sha256 `af74fed13ea7f2d5ca2a39d9f58ec177713fafd7cab63aef4e27b79f3ceca80b`).
  Its `macos-arm64_x86_64` slice is a universal dynamic `whisper.framework`
  with a module map, minimum macOS 13.3, Metal embedded. The sidecar links
  it with `swiftc -F … -framework whisper` and no cmake.
- With the 60 MB `ggml-base.en-q5_1` model the framework transcribed a 3.6 s
  clip ("Maya, what's waiting on me? Tell hexgrid to go ahead") correctly in
  0.52 s cold and 0.03 s warm on an M5 Pro, using Metal. The first model
  load took 7 s.
- Whisper writes "hexgrid" as "Hex Grid"; the interpreter's loose session
  matching already ignores case and spacing.
- Hugging Face serves the models with a size and sha256 in their LFS
  pointers (see Models).

## Behaviour

### Choosing the recogniser

- Settings › Voice assistant gains "Speech recognition" with two choices:
  **System (Apple)**, the default, and **Built-in (Whisper)**. Config field
  `recognizer`: `"system"` | `"builtin"`, default `"system"`.
- Changing it while listening restarts the listener with the new engine.
- The Dictation-locked message ends with "…or switch Speech recognition to
  Built-in."

### Models

- With Built-in selected, a "Model" picker lists:

  | id | file | size | note |
  |----|------|------|------|
  | `tiny.en` | `ggml-tiny.en.bin` | 78 MB | fastest, least accurate |
  | `base.en-q5_1` | `ggml-base.en-q5_1.bin` | 60 MB | recommended (default) |
  | `base.en` | `ggml-base.en.bin` | 148 MB | base, unquantised |
  | `small.en-q5_1` | `ggml-small.en-q5_1.bin` | 190 MB | most accurate, slower |

  Checksums (sha256): tiny.en
  `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f`;
  base.en-q5_1
  `4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f`;
  base.en
  `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002`;
  small.en-q5_1
  `bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30`.
  Sizes are the exact byte counts 77704715, 59721011, 147964211, 190098681.
- Config field `whisperModel` holds the id, default `base.en-q5_1`.
- Models live in `~/.claude/maya/models/<file>`. Each entry in the picker
  shows "downloaded" or its size; a "Download" button fetches the chosen one
  from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/<file>`
  with a progress bar, to a `.part` file, verified against the sha256 before
  it is renamed into place. A failed or mismatched download is deleted and
  reported. A "Remove" button deletes a downloaded model that is not the
  one in use.
- That download is the only network access the Built-in engine makes.
  Audio never leaves the Mac.
- The listener refuses to start Built-in without the chosen model's file and
  says "Download the <name> model in Settings first." under the toggle.

### The Whisper engine in the sidecar

- One sidecar binary, `maya-ear`, with `--engine system|whisper` and
  `--model <path>`. Audio capture, device choice, the level meter, pause and
  resume, and the JSON line protocol are shared; only recognition differs.
- Whisper needs 16 kHz mono float samples: the tap's buffers are converted
  with `AVAudioConverter`.
- Utterances are cut by an energy voice-activity detector over 20 ms frames:
  speech starts when three consecutive frames exceed the speech threshold,
  and ends after 0.8 s under it, or at 12 s. A 0.6 s pre-roll before the
  start is kept so the first syllable is not lost. Thresholds adapt to the
  room: the noise floor is the running minimum of frame energy, and speech
  is the floor plus a fixed margin.
- While an utterance is open, every 1.5 s the audio so far is transcribed
  and emitted as `partial`. When it closes, the whole utterance is
  transcribed and emitted as `final`; an empty or blank result emits
  nothing. Whisper's own hallucinated fillers on silence ("Thank you.",
  "[BLANK_AUDIO]") are dropped by a small deny list.
- Transcription runs on one background queue so audio capture never waits.
  Greedy sampling, English only, no timestamps, `single_segment` on, four
  threads, Metal when available.
- The sidecar emits `state: loading` while the model loads, then
  `listening`. A model that fails to load emits `state: error` with the
  reason and exits.

### Build and bundle

- `scripts/build-ear.sh` fetches the pinned xcframework zip into `vendor/`
  (git-ignored) when missing, checks its sha256, and links the sidecar
  against `vendor/whisper.xcframework/macos-arm64_x86_64` with rpaths
  `@executable_path/../Frameworks` (inside Maya.app) and the vendor
  directory (for `tauri dev`).
- `tauri.conf.json` lists the framework under `bundle.macOS.frameworks` so
  it lands in `Maya.app/Contents/Frameworks/whisper.framework`.
- The release workflow runs the same script for both architectures; the
  framework is universal, so one copy serves both bundles.

### Debug tab

- Model load time, each utterance's length and transcription time, and
  dropped fillers are logged under `ear`, so slow or wrong recognition can be
  read off the log.

## Components

- `ear/main.swift` — argument parsing, shared capture, the `System` engine
  (today's code) and the `Whisper` engine; the VAD as a small pure type.
- `scripts/build-ear.sh` — xcframework fetch, checksum, link flags.
- `src-tauri/src/models.rs` — the model table, paths, download with progress
  events (`voice-model`), checksum, remove.
- `src-tauri/src/config.rs` — `recognizer`, `whisper_model`.
- `src-tauri/src/ear.rs` — passes `--engine` and `--model`.
- `src-tauri/src/listener.rs` — refuses Built-in without a model; restarts on
  recogniser change.
- `src/settings.ts` — the recogniser select, the model picker with
  download/remove and progress.
- `.github/workflows/release.yml`, `README.md`.

## Testing

- Unit (Swift, in a `--selftest`-style harness or pure functions):
  the VAD state machine over synthetic frame energies (start after three
  loud frames, end after 0.8 s quiet, 12 s cap, pre-roll length); the
  filler deny list.
- Unit (Rust): model table lookups, target paths, checksum verification of
  a temp file, a `.part` that fails verification is removed, the listener's
  missing-model message, config defaults.
- Unit (vitest): the select and picker render, download button and progress,
  Remove disabled for the model in use.
- Sidecar alone: `maya-ear --engine whisper --model <path>` in a terminal
  prints partials and a final for a spoken sentence.
- End to end on this Mac: Built-in selected, model downloaded, "Maya, what's
  waiting on me?" answered aloud; a read-back confirmed with "yes".

## Out of scope

- Languages other than English, and multilingual models.
- Speaker identification, and a trained wake-word model.
- Downloading models from anywhere but Hugging Face, or user-supplied model
  files.
- Falling back to Built-in automatically when System fails.
