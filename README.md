# Maya

**Manage All Your Agents.** A macOS desktop board for your local Claude Code
sessions. Every running session is a card in one of four columns: Idle,
Working, Awaiting Decision, Completed. From a card you can jump to its
Terminal tab, read the conversation and reply, or answer the question it is
asking with one click. The Idle column's "+" starts a new session in one of
your project folders, or lets Claude pick the folder from your prompt.

## Install

macOS only for now (Apple Silicon and Intel). One command:

    curl -fsSL https://raw.githubusercontent.com/dosaki/maya/main/install.sh | sh

It downloads the latest release, puts `Maya.app` in `/Applications` (or
`~/Applications` when that is not writable), and clears the quarantine flag,
since the builds are not signed or notarised. Then `open -a Maya`.

Prefer a manual install? Grab the `.dmg` for your architecture from the
[latest release](https://github.com/dosaki/maya/releases/latest), drag Maya
to Applications, and the first time right-click it and choose Open.

To update, run the same command again. Set `MAYA_INSTALL_DIR` to install
somewhere else.

Every push to `main` builds and publishes a release. Linux and Windows builds
will follow once the terminal integration is portable; today the board is
macOS only because it drives Terminal through AppleScript and AppKit.

## How it works

- Reads the session registry Claude Code keeps at `~/.claude/sessions/`.
- Optionally installs a Claude Code hook (Settings → Install hook) that appends
  hook payloads to `~/.claude/maya/events.jsonl`. This is what makes permission
  prompts and finished turns show precisely. The hook never blocks and prints
  nothing; a backup of `settings.json` is taken before every change.
- Reads the tail of each session transcript for the card snippet and the
  conversation history.
- Replies go through the session's documented inbox socket; answers to
  questions are typed into the session's Terminal tab.
- New sessions open in a new Terminal window; "Let Claude choose" runs a
  short headless Haiku call to match the prompt to a folder.

Settings and data live in `~/.claude/maya/` (`config.json`, `events.jsonl`,
`hook.sh`, short-lived `prompts/`).

## Develop

    pnpm install
    pnpm tauri dev      # run
    pnpm test           # frontend tests
    cd src-tauri && cargo test   # Rust tests

Requires Rust (rustup), Node 20+, pnpm, and `jq` on PATH for the hook.

## Build

    pnpm tauri build

The app bundle lands in `src-tauri/target/release/bundle/`.
