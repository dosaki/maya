# An OpenAI-compatible voice, and a Chatterbox server to go with it

Status: design, agreed with the user on 2026-10-23. Items marked
**(my call)** were decided by Claude while writing this up and were not
discussed; the user can overturn any of them.

## Intent

Maya speaks with the built-in voice (`say`, SAPI, speech-dispatcher) or with
ElevenLabs, one or the other. The user wants a third voice: one from a
speech server they run themselves, starting with Resemble AI's open-source
[Chatterbox](https://github.com/resemble-ai/chatterbox) running on their Mac.
And they want the voices to back each other up: **ElevenLabs first, then
the server, then the built-in voice**.

Rather than a Chatterbox-only option, Maya gets a **generic
"OpenAI-compatible server" voice**: anything that answers OpenAI's
`POST /v1/audio/speech` works (our Chatterbox server, other community
servers, OpenAI itself). Next to it, the repo ships **a small Chatterbox
server** of its own, runnable natively with one command or with Docker, and a
guide to set it up.

### How the user's machines use it

- The Chatterbox server runs **natively on this Mac**.
- Only the **main** speaks (`src-tauri/src/lib.rs:516-587`; an assistant
  stays quiet), and the voice settings are **per device and not synced**
  (`core/src/config.rs:107-115`; promotion carries no voice settings). So:
  - this Mac, as main or alone, points at `http://localhost:<port>`;
  - the laptop, as main or alone, points at this Mac's LAN address.
  Every scenario the user named (this Mac main, laptop main, either on its
  own) is covered by each device having its own server address.
- **The phone keeps Android's voice.** Desktop only.

### What the spike measured (2026-10-23, this Mac, M5 Pro, 64 GB)

Chatterbox Turbo from PyPI (`chatterbox-tts` 0.1.7, torch 2.6.0), four
Maya-style lines, time to synthesise the whole line:

| Line | Native, Apple GPU (MPS) | Native, CPU (9–12 threads) | Podman container, 9 CPUs |
|---|---|---|---|
| "Maya here. Is my voice ok?" | 4.6 s | 2.2–2.5 s | 5.7 s |
| "Claude in maya is asking you something." | 3.3 s | 1.9–2.0 s | 5.4 s |
| "Codex finished in maya core." | 2.5 s | 1.7–1.9 s | 4.5 s |
| two-sentence line (~6.5 s of speech) | 6.0 s | 4.3–4.8 s | 11.1 s |

- **CPU beats MPS** natively. The container is about **2.5× slower** than
  native CPU on a Mac.
- Model load ~14–23 s; peak memory **~6.6 GB** (a 6 GB Podman VM was
  OOM-killed loading it).
- Gotchas: `resemble-perth` (the watermarker) imports `pkg_resources`, so
  **`setuptools<80`** is required or model construction fails with
  `'NoneType' object is not callable`; the `python:3.11-slim` image needs
  **`libgomp1`**; the PyPI release has **no Nano model** (the README's
  `nano=True` is newer than 0.1.7). The open-source models do not stream, so
  a new line waits for the whole synthesis; every output carries Resemble's
  inaudible Perth watermark.

## Goals

### Maya: a chain of voices (desktop: macOS, Linux, Windows)

- **Fixed priority order** (the user's): **ElevenLabs → OpenAI-compatible
  server → built-in**. Each line is spoken by the first voice in that order
  that is switched on, fully set up, not marked down, and succeeds. The
  built-in voice is always last and always there.
- **Settings → Voice loses the single provider dropdown.** Instead it shows
  one section per remote voice, in chain order, each with an on/off switch
  and its own fields **(switch: my call)**. A switch is needed because the
  user may keep ElevenLabs set up but not want it used, which today is
  "provider: built in" with a voice id still stored.
  - **ElevenLabs**: the fields it has today (key in the key store, voice).
  - **OpenAI-compatible server**:
    - **Server address**, e.g. `http://localhost:8004`, or
      `http://<mac's address>:8004` on the laptop. Maya appends
      `/v1/audio/speech` and `/v1/audio/voices` itself; a trailing `/` or a
      trailing `/v1` typed by the user is tolerated **(my call)**.
    - **Voice**: a dropdown filled from `GET <address>/v1/audio/voices` when
      the server answers it; otherwise a free text field (OpenAI's own API
      has no voice list).
    - **Model**: optional free text, sent as `model`; blank means the field
      is left out or sent as a neutral default the builder picks (OpenAI
      needs it; our server ignores it).
    - **API key**: optional, stored in the OS key store as the ElevenLabs
      key is (`src-tauri/src/keystore.rs`), sent as
      `Authorization: Bearer <key>`.
  - The built-in voice is named as the last resort; the volume slider and
    "Try the voice" stay as they are.
- **Migration** **(my call)**: ElevenLabs is switched on when an existing
  config has `voiceProvider: "elevenlabs"`, off otherwise, so nobody's voice
  changes on upgrade. The server starts off. Whether `voiceProvider` is kept,
  replaced or only read once is left to the builder; an older Maya reading a
  newer config must not break (serde defaults).
- **Settings are per device**, stored in `Config`, not synced, not carried
  by promotion.
- **Speaking a line through the server**: `POST <address>/v1/audio/speech`
  with `{"model", "input", "voice", "response_format": "wav"}`; the body is
  cached in the existing voice cache (`<maya dir>/voice/`) keyed by address,
  model, voice and text, then played with the existing player (`afplay`,
  MCI, `paplay`/`ffplay`) at the existing volume.
- **A cached line plays from the cache** without touching the network, for
  either remote voice, even while that voice is marked down **(my call)**.
- **WAV** is requested **(my call)**: every player Maya uses plays it
  (`afplay`, `paplay`, `ffplay`, and MCI's `type mpegvideo` on Windows) and
  the server then needs no MP3 encoder. Cache files carry the matching
  extension; the Linux player comment about decoding MP3 is reworded.
- **Timeouts**: a short **connection** timeout, about 3 s, so an
  unreachable server (laptop away from home, Mac asleep) fails fast; a long
  **whole-request** timeout, about 60 s, because a long line took 11 s in
  the container on this Mac and slower hosts exist **(numbers: my call)**.
  ElevenLabs gets the same short connection timeout **(my call)**.
- **Remembering a voice is down** (the user chose this over timeouts
  alone): when a remote voice fails, Maya marks it down and skips it, going
  straight to the next voice in the chain, for **5 minutes** or until that
  voice's settings change, whichever comes first; then she tries it again.
  This applies to ElevenLabs and the server alike **(ElevenLabs: my call)**.
  The mark lives in memory only, per process.
- **"Try the voice"** ignores the down marks and walks the whole chain,
  without silently falling through: it reports each remote voice that
  failed and its error, and which voice finally spoke **(my call)**. A
  success clears that voice's down mark.
- Everything else stays as today: the one speech queue, the listener
  pausing around a line, volume, Focus/quiet hours.

### The Chatterbox server, in the repo (`chatterbox/`, my call)

- **`chatterbox/server.py`**: one Python file with PEP 723 inline metadata
  pinning its dependencies (`chatterbox-tts==0.1.7`, `setuptools<80`, a web
  framework and server of the builder's choice), so
  `uv run chatterbox/server.py` installs and starts it.
  - `POST /v1/audio/speech`: OpenAI request body (`model` ignored, `input`,
    `voice`, `response_format`); returns WAV. A request for a format it
    cannot produce is answered with WAV or a clear 400, builder's choice.
  - `GET /v1/audio/voices`: the voices it has, in the shape devnen's
    Chatterbox-TTS-Server uses for the same path,
    `{"status": "ok", "voices": ["<file name>", ...]}` (plain file names,
    extension included), so Maya's dropdown works with both **(my call)**.
    Maya's parser reads that shape.
  - **Voices**: `default` is Chatterbox's built-in voice; every audio clip in
    `chatterbox/voices/` (longer than 5 s; ~10 s recommended) is a voice
    named by its file name **with its extension** (e.g. `tiago.wav`), as
    devnen's server names them, so one name works on both. Each voice's
    conditionals are prepared once and kept in memory.
  - **Model**: Chatterbox Turbo (English). **Device**: CPU by default,
    because it beat MPS on this Mac; an option (flag or env var) selects
    `cuda` or `mps` **(my call)**.
  - **Network**: listens on `127.0.0.1` by default; a flag binds to all
    interfaces so the laptop can reach it. An optional API key (env var or
    flag) makes it require `Authorization: Bearer <key>`.
  - Port **8004** by default, the same as devnen's server **(my call)**.
- **`chatterbox/Dockerfile`** and **`chatterbox/compose.yaml`**: run the
  same `server.py` on CPU, with `libgomp1`, volumes for `voices/` and the
  model cache so models download once, and a commented NVIDIA GPU section.
- **Guide `docs/chatterbox.md`**, linked from the README's Voice section:
  - native setup on macOS and Linux with `uv` (the recommended route on a
    Mac), and a launchd agent so it runs at login **(my call)**;
  - Docker/Podman with the compose file, noting the ~2.5× slower start on a
    Mac and the ~6.6 GB memory need (raise the VM's memory);
  - adding a cloned voice;
  - pointing Maya at it, including the laptop at this Mac's address, the
    LAN flag and the API key;
  - the numbers above: expect ~2 s before a short new line, instant for a
    cached one;
  - the chain: with ElevenLabs on, the server only speaks when ElevenLabs
    fails.
- The README's Voice section is updated for the chain.

## Non-goals

- The phone speaking through the server. It keeps Android's TTS.
- Maya installing, starting, supervising or bundling Python, PyTorch or the
  models. The server is something the user runs.
- A user-chosen order for the chain, or more than one server.
- Streaming playback, multilingual or Nano models, the web UI for cloning
  voices that community servers offer.
- Syncing the voice settings between devices.
- Running the server in CI.

## Anti-goals

- Anyone's voice changing on upgrade.
- A down voice making Maya silent, or leaving her deaf and silent for long
  on every line (the listener pauses from `SpeechPhase::Starting`, before
  synthesis).
- The API key on a command line or in a log (`keystore.rs` rule: secrets go
  through stdin).

## Constraints

- Version bump: a `feat` → **0.31.0** from `main`'s 0.30.0 via
  `sh scripts/set-version.sh`, committed on its own (CLAUDE.md).
- Windows CI: unix-only tests gated with `cfg(unix)`; compare paths by
  component.
- Follow the existing ElevenLabs code's shape and idiom
  (`src-tauri/src/voice.rs`, `core/src/notify.rs`, `src/settings.ts`).
  How the `Utterance`'s ElevenLabs tuple becomes a chain of remote voices is
  left to the builder.
- HTTP: ElevenLabs uses `curl` subprocesses, other code uses `ureq`; the
  builder picks, keeping the key off the command line.

## Testing

- **Maya**: unit tests for the config round trip, defaults and migration,
  URL joining, the request body, parsing the voice list (including a server
  without one), the cache path, the chain order and which voices join it,
  down marks (skip, expiry after 5 minutes, cleared by a settings change and
  by a successful try), "Try the voice" reporting, and the Settings UI
  states (`src/settings.test.ts`, `src/settings-flow.test.ts`).
- **Server**: `python -m py_compile` in CI **(my call)**; no model download
  in CI.
- **Hand check** on this Mac: server natively, Maya on this Mac speaking
  through it with ElevenLabs off, then on with a bad key (falls to the
  server), then the server stopped (falls to built-in, then skipped); the
  laptop pointing at this Mac; then the compose file under Podman. Recorded
  as owed in memory if not done before merge.
