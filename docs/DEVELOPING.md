# Developing Maya

Maya is a Tauri 2 app: a Rust backend in a Cargo workspace (`core/`,
`src-tauri/`, `cli/`, `hook/` and `ear-rs/`), a vanilla TypeScript frontend
in `src/` (vitest), and a listener sidecar: Swift in `ear/` on macOS, Rust
in `ear-rs/` on Windows.

## Requirements

- Rust (rustup), Node 20+, pnpm
- the Xcode Command Line Tools (`xcode-select --install`) for `swiftc`
- `jq` on PATH for the Claude Code hook, `gh` for the Pull Requests tab

## Build and run

    pnpm install
    pnpm ear:build      # build the voice listener sidecar (once, and after ear/ changes)
    pnpm tauri dev      # run
    pnpm test           # frontend tests
    pnpm ear:test       # sidecar tests
    cargo test --workspace   # Rust tests, from the repo root

`pnpm ear:build` fetches whisper.cpp's prebuilt framework (53 MB, once, into
`vendor/`, checked against a pinned sha256) and compiles `ear/main.swift`
into `src-tauri/binaries/maya-ear-<target>`. That binary is not in git, and
`pnpm tauri dev`, `pnpm tauri build` and `cargo test` all fail until it
exists, because Tauri bundles it as a sidecar.

    pnpm tauri build --bundles app

puts `Maya.app` in `target/release/bundle/macos/` (with `--target <triple>`,
`target/<triple>/release/bundle/macos/Maya.app`). The voice
features only work from a bundle: the sidecar, the whisper framework and
the microphone and speech permissions are all tied to it.

## Windows

Requirements: Rust (rustup, the MSVC toolchain), the Visual Studio C++
build tools, CMake (whisper.cpp is built from source), Node 22+, pnpm, and
Git for Windows. Then, from Git Bash or PowerShell:

    pnpm install
    pnpm ear:build      # maya-ear and maya-hook into src-tauri/binaries/
    pnpm tauri dev
    pnpm test
    pnpm ear:test       # the Rust ear's tests
    cargo test -p maya-core -p maya-hook -p maya-ear -p maya

    pnpm tauri build --bundles nsis,msi

puts the installers in `target/release/bundle/nsis/` and `msi/`
(`tauri.windows.conf.json` adds the Windows sidecars and bundle targets).
In PowerShell quote the list, `--bundles "nsis,msi"`, or it becomes two
arguments.

What is different on Windows, and where:

- **Terminal.** `src-tauri/src/terminal_win.rs`: new sessions open in a
  Windows Terminal window of their own running Git Bash, which runs the same
  shell lines as macOS. A session's "tty" is `console:<pid>`;
  `core/src/win_console.rs` attaches to that console to type key events
  into it and to find the window showing it.
- **Inbox.** `core/src/inbox.rs`: a named pipe, opened with the auth line
  `{"type":"auth","token":…}` after checking the pipe's server is the
  session. The token is the session's `CLAUDE_CODE_MESSAGING_TOKEN`, which
  Claude Code exports only to the session's hooks and Bash commands; the
  hook keeps it in `~/.claude/maya/inbox/<pipe name>.token`. (The
  `<pid>.<hex>.key` beside the registry record is not that token: a
  connection opened with it is accepted, then dropped without a word.)
  `cargo run -p maya-core --example inbox_probe` lists live sessions, and
  with `<pid> <message>` sends one.
- **Hook.** Claude Code runs hooks through Git Bash, which has no `jq`, so
  the hook is `hook/`'s `maya-hook.exe`, copied into `~/.claude/maya/` by
  Install Claude hook. It also hooks SessionStart, only to record the token.
- **Notifications, voice, quiet.** `core/src/notify_win.rs`: toasts, SAPI,
  and Do not disturb. The ElevenLabs key is in Credential Manager
  (`src-tauri/src/voice.rs`).
- **Other agents.** `core/src/win_process.rs` lists processes (ToolHelp)
  and names the processes holding a file open (the Restart Manager).
  `foreign::discover_from_files` maps an `agy` pid to its conversation
  through its own `log/cli-*.log`, and a `codex` pid to its thread through
  the `thread-writer-locks/<id>.lock` it holds.
- **Ear.** `ear-rs/`: the Swift ear's protocol over WASAPI and whisper.cpp,
  with the VAD ported case for case. `cargo run -p maya-ear --release
  --example transcribe -- <model.bin> <file.wav>` runs a WAV through it.

The release workflow's `windows` job runs these tests on every push and
pull request, and on a release push builds the installers and hands them
to the `macos` job as the `windows-installers` artifact.

## The CLI

`cli/` builds on its own, with no Node toolchain and no `pnpm ear:build`:

    cargo build -p maya-cli --release        # target/release/maya-cli
    cargo test -p maya-core -p maya-cli

`maya-cli` depends only on `maya_core`, so unlike the rest of the workspace
it also builds and tests on Linux. The release workflow's `linux` job runs
first, on every pull request to `main` and every push to it: it runs
`cargo test -p maya-core -p maya-cli` on Ubuntu (with tmux installed) —
the guard that keeps `maya_core` free of macOS-only code. On a release
push it also cross-compiles `maya-cli` for `x86_64-unknown-linux-musl`
and `aarch64-unknown-linux-musl` with `cross` 0.2.5 (needs Docker) and
hands both binaries to the `macos` job as the `linux-binaries` artifact.
The `macos` job runs on pushes only and after `linux` succeeds; it tests
the whole workspace, builds the app and the macOS binaries, and creates
the release with every asset at once, so a failed Linux build publishes
nothing. Both jobs decide whether to release with
`scripts/release-version.sh`.

## Layout

- `Cargo.toml` — the workspace: shared version, dependencies and release profile.
- `core/` — `maya_core`, the platform-neutral core: no Tauri, no AppKit.
- `src-tauri/` — `maya`, the app: Tauri commands, voice, focus and dock; depends on `maya_core`.
- `hook/` — `maya-hook`, the Claude Code hook on Windows.
- `ear-rs/` — `maya-ear` for Windows: `vad.rs`, `resample.rs` and the
  microphone choice in the library, the audio and whisper.cpp in `main.rs`.
- `cli/` — `maya_cli`, binary `maya-cli`: the headless assistant for
  SSH boxes and containers (`pair`, `run`, `status`, `hooks`, `config`, `start`);
  depends on `maya_core`, no Tauri.
- `core/src/` — `store.rs` (session cache), `state.rs` (card states),
  `watcher.rs`, `answer.rs` and `launch.rs` (Terminal automation),
  `inbox.rs` (Claude Code's session socket), `foreign.rs` with `codex.rs`,
  `antigravity.rs`, `grok.rs` (other agents), `reviews.rs` and `pr.rs`
  (GitHub), `notify.rs` (notifications and speech output), `tty.rs`,
  `interpreter.rs`, `log.rs` (the Debug tab and `maya.log`), and `net/`
  with `protocol.rs` (message types, encode/decode, pairing and HMAC),
  `server.rs` (the main: listener, threads per assistant, pairing,
  command dispatch), `client.rs` (the assistant: connection, backoff,
  board sending), and `merge.rs` (merging local and remote cards, stale
  and expiry rules, routing).
- `src-tauri/src/` — `lib.rs` (Tauri commands), `net_app.rs` (the network's
  Tauri side: starting server and client, command execution),
  `terminal_app.rs` (opening Terminal windows), `focus.rs` and `dock.rs`
  (AppKit), `voice.rs` (ElevenLabs), and `ear.rs`, `wake.rs`,
  `listener.rs`, `models.rs` (the voice assistant).
- `cli/src/` — `main.rs` and `args.rs` (subcommand parsing), `commands.rs`
  (`pair`, `status`, `hooks`, `config`, `start`), `run_cmd.rs` (`run`, the headless
  loop), `executor.rs` (running the main's commands), `tmux.rs` (named
  sessions for start and resume), `notify.rs` (writes the CLI's own status
  file on connect, disconnect and removal — the CLI never notifies, speaks
  or listens like the app), and `status_file.rs`
  (`~/.claude/maya/cli-status.json`, read by `maya status`).
- `src/` — `main.ts`, `board.ts`, `card.ts`, `modal.ts`, `reviews.ts`,
  `settings.ts`, `voice.ts`, `debug.ts`, `tabs.ts`.
- `ear/` — `main.swift` (audio capture, the System and Whisper engines),
  `vad.swift` (voice activity detection), `vadtest.swift`.
- `docs/superpowers/specs/` — the design documents behind each feature.

## Releasing

Releases follow semver. The version in `src-tauri/tauri.conf.json` is the
release version; bump it everywhere, commit and push to `main`:

    pnpm version:set 0.2.0      # or: sh scripts/set-version.sh 0.2.0
    git commit -am "chore: release 0.2.0"
    git push

The workflow publishes `v0.2.0` for both architectures, with notes generated
from the merged pull requests and commits. A push whose version already has
a release only runs the tests; a mismatch between `tauri.conf.json`,
`package.json` and `Cargo.toml` fails the run. A pull request runs the
Linux and Windows tests; the macOS job runs on pushes to `main` only.
Pull requests bump the version themselves: see the rule in
[CLAUDE.md](../CLAUDE.md).

## Signing

Without a signing identity the bundle is ad-hoc signed, and macOS treats
every rebuild as a new app: the microphone and speech-recognition prompts
come back each time. Sign with a stable identity to keep the grants:

    APPLE_SIGNING_IDENTITY="Maya Development" pnpm tauri build --bundles app

Any code-signing certificate works for your own Mac. Without Xcode or the
paid developer program, make a self-signed one: Keychain Access ›
Certificate Assistant › Create a Certificate, type Code Signing, then mark
it Always Trust for code signing. `security find-identity -v -p codesigning`
prints the name to use. Tauri signs the app, the listener sidecar and the
whisper framework with it; `src-tauri/Entitlements.plist` disables library
validation so the sidecar can load the framework under a signature with no
Team ID, and grants audio input under the hardened runtime.

To sign release builds in CI, export the certificate with its private key
as a `.p12` with legacy algorithms (macOS's `security import` rejects the
OpenSSL 3 defaults):

    openssl pkcs12 -export -legacy -in cert.pem -inkey key.pem -name "Maya Development" -out maya.p12

and add three repository secrets: `APPLE_CERTIFICATE` (the `.p12` as
base64 without line breaks, `base64 -i maya.p12 | tr -d '\n'`),
`APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY` (the certificate
name). The workflow imports the certificate into its own keychain, marks it
trusted for code signing and signs with it; without the secrets the bundles
stay ad-hoc. A self-signed certificate only satisfies Macs that trust it;
distributing to other people needs a Developer ID and notarization.
