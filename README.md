# Eye

A macOS desktop board for your local Claude Code sessions. Every running
session is a card in one of four columns: Awaiting Decision, Working,
Completed, Idle. Click a card to jump to its Terminal tab.

## How it works

- Reads the session registry Claude Code keeps at `~/.claude/sessions/`.
- Optionally installs a Claude Code hook (Settings → Install hook) that appends
  hook payloads to `~/.claude/eye/events.jsonl`. This is what makes permission
  prompts and finished turns show precisely. The hook never blocks and prints
  nothing; a backup of `settings.json` is taken before every change.
- Reads the tail of each session transcript for the card snippet.

## Develop

    pnpm install
    pnpm tauri dev      # run
    pnpm test           # frontend tests
    cd src-tauri && cargo test   # Rust tests

Requires Rust (rustup), Node 20+, pnpm, and `jq` on PATH for the hook.

## Build

    pnpm tauri build

The app bundle lands in `src-tauri/target/release/bundle/`.
