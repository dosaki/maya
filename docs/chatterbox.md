# A Chatterbox voice for Maya

Maya can speak through a speech server you run yourself. This guide sets up
the small Chatterbox server in `chatterbox/`, and points Maya at it.

## What you get

Maya speaks each line with the first voice that works: ElevenLabs, then the
voice server, then the built-in voice. Any OpenAI-compatible speech server
works as the voice server. `chatterbox/server.py` is one you can run with a
single command.

The phone keeps Android's voice. It does not use the server.

## Run it natively (recommended on a Mac)

Install [uv](https://docs.astral.sh/uv/), then start the server.

From a checkout of this repository:

    uv run chatterbox/server.py

Without one:

    curl -O https://raw.githubusercontent.com/dosaki/maya/main/chatterbox/server.py
    uv run server.py

- **First start.** It downloads the model, which took about 5.5 minutes on
  the Mac we tried it on. Later starts load it in 14 to 23 seconds.
- **Memory.** Plan for about 6.6 GB.
- **Flags.** `--host` (default `127.0.0.1`), `--port` (`8004`), `--device`
  (`cpu`, `mps` or `cuda`; `cpu`) and `--voices` (a folder; the `voices/`
  folder next to the script).
- **CPU is the default** because it beat the Apple GPU. Seconds to speak a
  whole line, on an M5 Pro with 64 GB:

  | Line | Apple GPU (`mps`) | CPU | Podman container |
  |---|---|---|---|
  | "Maya here. Is my voice ok?" | 4.6 | 2.2-2.5 | 5.7 |
  | "Claude in maya is asking you something." | 3.3 | 1.9-2.0 | 5.4 |
  | "Codex finished in maya core." | 2.5 | 1.7-1.9 | 4.5 |
  | Two sentences (about 6.5 s of speech) | 6.0 | 4.3-4.8 | 11.1 |

- **NVIDIA.** Use `--device cuda`.

## Start it at login (macOS)

Find where `uv` is with `which uv`. Save this as
`~/Library/LaunchAgents/com.dosaki.maya.chatterbox.plist`, with your own
`uv` path and the full path of `server.py`:

    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
      <key>Label</key><string>com.dosaki.maya.chatterbox</string>
      <key>ProgramArguments</key>
      <array>
        <string>/opt/homebrew/bin/uv</string>
        <string>run</string>
        <string>/Users/you/maya/chatterbox/server.py</string>
      </array>
      <key>RunAtLoad</key><true/>
      <key>KeepAlive</key><true/>
      <key>StandardOutPath</key><string>/Users/you/Library/Logs/maya-chatterbox.log</string>
      <key>StandardErrorPath</key><string>/Users/you/Library/Logs/maya-chatterbox.log</string>
    </dict>
    </plist>

Then load it:

    launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.dosaki.maya.chatterbox.plist

To stop it: `launchctl bootout gui/$(id -u) ~/Library/LaunchAgents/com.dosaki.maya.chatterbox.plist`.

If you add an API key under `EnvironmentVariables` in this file, it is
stored in plain text there.

## Run it with Docker or Podman

    docker compose -f chatterbox/compose.yaml up -d

Use `podman compose` the same way.

- **Memory.** Give the VM at least 10 GB. A 6 GB VM was killed while it
  loaded the model. For Podman: `podman machine set --memory 12288`.
- **Speed.** On a Mac the container is about 2.5 times slower than native
  (see the table).
- **Port.** The compose file binds `127.0.0.1:8004`. To let other machines
  in, change it to `"8004:8004"` and set `CHATTERBOX_API_KEY`.
- **Files.** `./voices` is mounted into the container, and a named volume
  keeps the downloaded model, so a restart does not fetch it again.
- **NVIDIA.** Uncomment the `deploy` block and the `command` line at the
  end of the service in `compose.yaml`.

## Add a voice

Put a clip of 10 seconds or more (`.wav`, `.mp3` or `.flac`) in
`chatterbox/voices/`. The file name, with its extension, is the voice name:
`alice.wav` is the voice `alice.wav`. The server reads the folder on every
request, so you do not restart it.

A clip of 5 seconds or less is refused ("Audio prompt must be longer than 5
seconds!"). `default` is always there: it is Chatterbox's built-in voice.

## Point Maya at it

1. Open Settings, then Voice.
2. Tick **Use a voice server (OpenAI-compatible)**.
3. Set **Server address** to `http://localhost:8004`.
4. Pick the **Voice**. It is a dropdown when the server lists its voices,
   otherwise a text box. Blank means the server's default voice.
5. Leave **Model** and **API key** empty unless your server needs them. The
   key is stored in the OS key store.
6. Click **Try the voice**. It says which voices failed and which one
   spoke.

Settings shows the order: "Maya speaks with the first voice that works:
ElevenLabs, then the voice server, then <built-in voice> (built in)."
ElevenLabs has its own **Use ElevenLabs** checkbox.

## Use it from another machine

On the machine that runs the server, set a key and listen on every
address:

    CHATTERBOX_API_KEY=choose-a-long-secret uv run chatterbox/server.py --host 0.0.0.0

With Docker, change the port to `"8004:8004"` and set `CHATTERBOX_API_KEY`
in the environment that runs `docker compose`. Requests then need
`Authorization: Bearer <key>`.

On the other machine, set **Server address** to
`http://<the server machine's address>:8004` and paste the key into **API
key**.

Traffic is plain HTTP. Keep it on a network you trust.

When the server cannot be reached, Maya uses the next voice and skips the
server for 5 minutes, or until you change its settings.

## What to expect

- About 2 seconds before a short new line is heard. A line Maya has said
  before plays at once, from her cache.
- No streaming: a new line waits for the whole synthesis.
- Every clip carries Resemble's inaudible watermark.

## Troubleshooting

- **`'NoneType' object is not callable`** when the model loads: the
  watermarker needs `setuptools<80`. `server.py` and the Dockerfile pin it.
  Pin it too in any environment of your own.
- **`libgomp.so.1` not found** in a custom image: install `libgomp1`
  (`apt-get install libgomp1`). The Dockerfile does.
- **The container dies while loading:** the VM is out of memory. Raise it
  to 10 GB or more.
- **Maya keeps using another voice:** click **Try the voice**. It names the
  voice that failed and why. After a failure Maya skips the server for 5
  minutes.
