# Developing Maya

Maya is a Tauri 2 app: a Rust backend in `src-tauri/`, a vanilla TypeScript
frontend in `src/` (vitest), and a Swift listener sidecar in `ear/`.

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
    cd src-tauri && cargo test   # Rust tests

`pnpm ear:build` fetches whisper.cpp's prebuilt framework (53 MB, once, into
`vendor/`, checked against a pinned sha256) and compiles `ear/main.swift`
into `src-tauri/binaries/maya-ear-<target>`. That binary is not in git, and
`pnpm tauri dev`, `pnpm tauri build` and `cargo test` all fail until it
exists, because Tauri bundles it as a sidecar.

    pnpm tauri build --bundles app

puts `Maya.app` in `src-tauri/target/release/bundle/macos/`. The voice
features only work from a bundle: the sidecar, the whisper framework and
the microphone and speech permissions are all tied to it.

## Layout

- `src-tauri/src/` — `store.rs` (session cache), `state.rs` (card states),
  `watcher.rs`, `answer.rs` and `launch.rs` (Terminal automation),
  `inbox.rs` (Claude Code's session socket), `foreign.rs` with `codex.rs`,
  `antigravity.rs`, `grok.rs` (other agents), `reviews.rs` and `pr.rs`
  (GitHub), `notify.rs` and `voice.rs` (speech output, ElevenLabs),
  `ear.rs`, `wake.rs`, `interpreter.rs`, `listener.rs`, `models.rs` (the
  voice assistant), `log.rs` (the Debug tab and `maya.log`), and `net/`
  with `protocol.rs` (message types, encode/decode, pairing and HMAC),
  `server.rs` (the main: listener, threads per assistant, pairing,
  command dispatch), `client.rs` (the assistant: connection, backoff,
  board sending, command execution), and `merge.rs` (merging local and
  remote cards, stale and expiry rules, routing).
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
`package.json` and `Cargo.toml` fails the run.

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
