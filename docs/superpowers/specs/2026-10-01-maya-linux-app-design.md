# Maya on Linux (Ubuntu, GNOME, Wayland) — design

Date: 2026-10-01
Status: implemented
Builds on: `2026-09-30-maya-cli-design.md` (the workspace and the
`Terminal` seam) and the Windows port (PR #8: `ear-rs`, `maya-hook`, the
platform gates, `build.yml`).

## Purpose

Maya's desktop app runs on Linux with the same features as on macOS and
Windows: the board, replies and answers, starting and resuming sessions in
a terminal, notifications and spoken announcements with the built-in or
ElevenLabs voice, Do Not Disturb awareness, the voice assistant (wake word,
Whisper recognition), other agents' sessions, pull requests, and the main
and assistant roles over the network.

Target: Ubuntu with GNOME on Wayland (24.04 and later). X11 sessions work
too; where they differ it is said. Other distributions get the AppImage.

Decisions taken during design:

- Sessions Maya starts run in **tmux**, opened in a terminal window; the
  Terminal button activates the window Maya opened and otherwise opens a
  fresh attached one (the user's option 2).
- Voice uses the **Rust ear** from the Windows port, un-gated for Linux.
- The Claude Code hook is the bundled **`maya-hook`** binary, as on
  Windows, so the app needs no `jq`.
- Packaging: **`.deb` and AppImage**, x86_64 and aarch64, built natively
  in CI; `install.sh` installs the AppImage.
- The machine picker says **"This computer"** on Linux; macOS and Windows
  keep their words.

## Behaviour

### Sessions and the terminal

- **Start and resume** create a tmux session `maya-<8 hex>` (the CLI's
  label and command line) and open a terminal window attached to it:
  `gnome-terminal --title <label> -- tmux attach -t <label>` when
  `gnome-terminal` is on the PATH, else `x-terminal-emulator -e tmux attach
  -t <label>`, else the error `No terminal emulator found: install
  gnome-terminal.` The pane stays after `claude` exits (the CLI's `exec
  $SHELL` rule).
- **Typing** (answers, slash commands, Shift+Tab, replies to agents without
  an inbox) goes through tmux `send-keys` to the pane whose tty is the
  session's, exactly as the CLI does. A session outside tmux (started by
  hand in a plain terminal) is on the board, takes replies through its
  inbox, and refuses typing with `This session is not in tmux; only
  replies reach it.`
- **Terminal button**: for a card whose session is in tmux, Maya looks up
  the window it opened for that label; if it is still open it activates it
  (on Wayland through an activation token the app requests from GTK, on
  X11 through the same GTK call, which raises directly); if that fails or
  there is no record it opens a fresh attached window, which the desktop
  focuses because Maya launched it. A session outside tmux has no Terminal
  button (as remote cards today).
- **Focus by pid** (the review cards) resolves the pid's tty, then the tmux
  pane, then the same rule.

### Notifications and speech

- Banners through `notify-send --app-name Maya --icon <bundled icon>`
  with the title, subtitle and body the macOS banner has; `sound` plays
  the desktop's message sound with `canberra-gtk-play -i message` when
  present.
- **Do Not Disturb**: `gsettings get org.gnome.desktop.notifications
  show-banners` returning `false` means quiet; anything else (not GNOME,
  unreadable) means not quiet.
- **Built-in voice**: `spd-say -w <line>` (speech-dispatcher, installed
  with GNOME); when `spd-say` is missing, `espeak-ng <line>`; when both are
  missing, speech is skipped and the Debug log says so once. With no
  audio sink both wait forever, so each line has a deadline of 15 s plus
  1 s per 12 characters: past it the synthesiser is killed, `spd-say -C`
  cancels what it queued, and the Debug log says `<bin> did not finish in
  <N> s; stopped`.
- **ElevenLabs**: the key lives in GNOME Keyring through `secret-tool
  store --label "Maya ElevenLabs" service maya account elevenlabs` and
  `secret-tool lookup service maya account elevenlabs`; playback through
  `paplay <file>`, else `aplay`, else `ffplay -nodisp -autoexit`; with none
  of them the Settings "Try" button shows `No audio player found: install
  pulseaudio-utils.`
- The Settings page names the store "GNOME Keyring" and the built-in
  voice "speech-dispatcher's voice".

### Voice assistant

- The sidecar is `maya-ear` from `ear-rs/`, built for Linux: microphone
  through `cpal`'s ALSA host (PipeWire and PulseAudio expose ALSA devices),
  whisper.cpp through `whisper-rs`, the same JSON lines and arguments.
  `--device <name>` picks a microphone from the names the ear lists.
- Settings offers one recogniser, "Built-in (Whisper, runs on this
  computer)"; models download and are stored as on the other platforms.
- The wake word, the conversation flow and the interpreter are unchanged.

### Hooks and other agents

- `Install Claude hook` copies `maya-hook` from beside the app binary (the
  AppImage's and the `.deb`'s layout both put sidecars next to `maya`) into
  `~/.claude/maya/maya-hook` (0755) and points every hook at it, as the
  Windows branch does with `maya-hook.exe`. `Remove` strips it. The Codex
  hook buttons do the same with `--codex`.
- Codex and Antigravity are found by `ps`/`lsof` as on macOS; Grok through
  its registry. `lsof` missing means those sessions are not listed, with
  one Debug line.

### Networking, pull requests, URLs

- Unchanged: the main and assistant roles, the LAN protocol, `gh` for
  pull requests. URLs open with `xdg-open`.

### Page

- `platform.ts` gains `isLinux()` (WebKitGTK's user agent contains
  `Linux` and not `Android`): `thisComputer()` → "This computer",
  `sendShortcut()` → "Ctrl+Enter", `builtinVoiceName()` →
  "speech-dispatcher", `secretStore()` → "GNOME Keyring",
  `recognizerOptions()` → Whisper only.

### Packaging, install, release

- `src-tauri/tauri.linux.conf.json`: bundle targets `deb` and `appimage`,
  `externalBin` `binaries/maya-ear` and `binaries/maya-hook`, the `.deb`
  depending on `libwebkit2gtk-4.1-0`, `libgtk-3-0`, `tmux`,
  `libnotify-bin`, `speech-dispatcher` (recommends: `gnome-terminal`,
  `pulseaudio-utils`, `lsof`).
- Assets per architecture: `Maya_<version>_amd64.deb`,
  `Maya_<version>_arm64.deb`, `Maya_<version>_amd64.AppImage`,
  `Maya_<version>_arm64.AppImage`, uploaded as `maya-linux-app-<arch>`
  and merged into the release by the existing release job.
- `install.sh` on Linux downloads the AppImage for the architecture to
  `~/.local/bin/maya-app` (or `$MAYA_INSTALL_DIR`), marks it executable,
  writes `~/.local/share/applications/maya.desktop` with the bundled icon,
  and prints how to start it. It never needs root; the `.deb` is for
  people who prefer `apt`.
- The README gains a Linux section (install, requirements, what differs).

## Architecture

- **`core/src/terminal_tmux.rs`** (moved from `cli/src/tmux.rs`, unchanged
  API): `Tmux`, `new_session_args`, `send_keys_args`, `pane_for_tty`,
  `NOT_IN_TMUX`. The CLI re-exports it.
- **`src-tauri/src/terminal_linux.rs`**: `LinuxTerminal` implements
  `Terminal`:
  - `open(command, cwd, label)`: `Tmux::open`, then `open_window(label)`
    (the emulator command line above, built by a pure
    `terminal_command(emulator, label) -> Vec<String>`), records
    `label → launched pid` in a `Mutex<HashMap<String, Window>>`, returns
    `Some(label)`;
  - `type_line`, `name_for_tty`, `names_for_ttys`: delegate to `Tmux`;
  - `focus(tty)`: pane → label → recorded window → `activate(window)`;
    on failure `open_window(label)`; no pane → `Err(NOT_IN_TMUX)`.
  - `activate(window)`: GNOME Terminal is one server process, so an
    activation token minted by Maya cannot be handed to an existing
    window, and GNOME Shell exposes no activation call to third-party
    processes. So `activate` runs `wmctrl -a <title>` (works on X11 and
    for XWayland terminals) and reports failure otherwise; the caller then
    opens a fresh attached window. The record of launched windows is kept
    so a real Wayland activation path can be added later without changing
    callers. On pure Wayland the Terminal button therefore opens a new
    attached window each time; the README says so.
  - `open_terminal_with(cmd)` for the PR review flow: a tmux session
    `maya-review-<8 hex>` running `cmd`, opened the same way.
- **`core/src/notify.rs`**: `cfg(target_os = "linux")` branches for
  `notify`, `say_builtin`, `voice()` (speech-dispatcher's default voice
  name from `spd-say -L` first line, cached), `focus_active`; the macOS
  branches become `cfg(target_os = "macos")`. Pure helpers:
  `notify_send_args`, `dnd_from_gsettings(&str) -> bool`.
- **`src-tauri/src/voice.rs`**: `cfg(target_os = "linux")` `store_key`,
  `load_key` (`secret-tool`), `play` (`paplay`/`aplay`/`ffplay`), with
  pure `secret_tool_args`.
- **`src-tauri/src/lib.rs`**: `open_in_browser` with `xdg-open`; `term`
  alias to `terminal_linux` on Linux; no `dock`/`focus` on Linux.
- **AppImage environment**: the AppImage's runtime (`APPDIR`, `APPIMAGE`,
  `ARGV0`, `OWD`) and linuxdeploy's GTK hook (`GDK_BACKEND`, `GTK_THEME`,
  `GTK_PATH`, `GTK_EXE_PREFIX`, `GTK_DATA_PREFIX`, `GIO_EXTRA_MODULES`,
  `GSETTINGS_SCHEMA_DIR`, `GDK_PIXBUF_MODULE_FILE`, `GI_TYPELIB_PATH`, and
  `$APPDIR/usr/share` first in `XDG_DATA_DIRS`) point into the mount.
  `launch::scrub_appimage_env(&mut Command)` removes them (and the mount's
  `XDG_DATA_DIRS` entries) when `APPIMAGE` is set, for the processes that
  outlive Maya: every `tmux` command (the first starts the server), the
  terminal window, and `xdg-open`.
- **`core/src/hook_install.rs`**: the Windows `install_with` becomes
  `cfg(any(windows, target_os = "linux"))` with `HOOK_EXE` = `maya-hook`
  on Linux and the 0755 mode set; `install_to` on Linux finds the binary
  beside `current_exe()` (`maya-hook` or `maya-hook-<triple>` as Tauri
  names sidecars; check both). macOS keeps `hook.sh`.
- **`ear-rs/`**: the `cfg(windows)` gates become
  `cfg(any(windows, target_os = "linux"))`; `Cargo.toml` target
  dependencies likewise; `main.rs`'s module is renamed `ear`; the
  non-Linux-non-Windows stub stays for macOS. `scripts/build-ear.sh`
  handles `*-unknown-linux-gnu`: `GGML_NATIVE=OFF cargo build --release
  -p maya-ear -p maya-hook` (whisper.cpp otherwise tunes for the build
  machine's CPU, and CI's arm64 runner has SVE2; the Windows branch sets it
  too, and CI's cache key changes with it because whisper-rs-sys does not
  rebuild on that variable) and copies both as `src-tauri/binaries/<name>-<triple>`.
  `scripts/test-ear.sh` runs `cargo test -p maya-ear` on Linux.
- **`src/platform.ts`**: `isLinux()` and the five functions above;
  `settings.ts` hints use `thisComputer()` where they say "this PC".
- **CI**: `build.yml` gains `linux-app` (matrix x86_64/aarch64 on the
  pinned `ubuntu-24.04` and `ubuntu-24.04-arm` runners, never
  `ubuntu-latest`: a bundle built on a newer Ubuntu needs its newer glibc
  and does not run on 24.04): apt installs `libwebkit2gtk-4.1-dev libgtk-3-dev
  libayatana-appindicator3-dev librsvg2-dev patchelf tmux libnotify-bin
  speech-dispatcher pulseaudio-utils`, `pnpm install`, `sh
  scripts/build-ear.sh <triple>`, `pnpm test`, `cargo test --workspace`
  (the app's tests included), `pnpm tauri build --bundles deb,appimage`,
  copy to `dist/`, upload `maya-linux-app-<arch>`. The existing `linux`
  (CLI) job stays. The release job needs no change.
- **Docs**: README Linux section; DEVELOPING "Linux" section mirroring
  "Windows"; `install.sh` Linux branch.

## Error handling

- Every missing tool has a named error or a one-line Debug note: terminal
  emulator, `tmux`, `notify-send`, `spd-say`/`espeak-ng`, `secret-tool`,
  audio player, `lsof`.
- The Terminal button never fails silently: activation failure falls back
  to opening a window; no pane gives the not-in-tmux message as a toast.
- The hook install reports the exact path it could not copy to.

## Testing

- Pure helpers unit-tested on every platform (they take strings): terminal
  command lines, `send-keys` sequences (already), `notify_send_args`,
  `dnd_from_gsettings`, `secret_tool_args`, player selection, hook install
  path selection, `isLinux()` and the page wording (vitest).
- `ear-rs` tests run on the Linux CI legs.
- A tmux integration test for `LinuxTerminal::type_line`/`name_for_tty`
  on the Linux runners (tmux is installed there), skipped elsewhere.
- The whole workspace's tests on the Linux app runners (first time the
  app crate's own tests run on Linux).
- **CI smoke test** (the `linux-app` job, after the bundle is built):
  under `xvfb-run` the AppImage is started with `--appimage-extract-and-run`
  and a temporary `HOME`; the job waits up to 60 s for
  `~/.claude/maya/maya.log` to contain `started`, takes a screenshot with
  `import -window root` (ImageMagick) uploaded as an artifact, sends
  `SIGTERM`, and fails if the process died early or never logged. This
  proves the WebKitGTK build launches and paints.
- **A VM on the developer's Mac** (no Ubuntu machine exists): `multipass
  launch --name maya-ubuntu` (arm64 Ubuntu 24.04), `ubuntu-desktop-minimal`
  plus `tmux gnome-terminal speech-dispatcher pulseaudio-utils`, the
  arm64 AppImage copied in. Driven over SSH: a GNOME session on a
  headless Wayland compositor (`weston --backend=headless` is enough for
  launch, typing and tmux; `gnome-session` under `weston` or a nested
  `mutter --wayland --headless` for the focus path) with screenshots by
  `grim`. What a VM cannot prove: real speakers and microphone (the ear
  runs against a null ALSA device), and GNOME's own notification popups
  (the `notify-send` call is verified by `dbus-monitor`).
- Hand checks for whoever first runs it on real Ubuntu hardware, listed in
  the README "Linux" section: hear a notification and the voice, say
  "Maya, what's waiting on me?", click Terminal on a card.

## Out of scope

Wayland window activation beyond the fallback (until GNOME exposes a
usable API to third-party processes), the CLI's `jq` replacement, Flatpak
or Snap packaging, non-GNOME desktops' Do Not Disturb.
