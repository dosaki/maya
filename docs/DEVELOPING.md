# Developing Maya

Maya is a Tauri 2 app: a Rust backend in a Cargo workspace (`core/`,
`src-tauri/`, `cli/`, `hook/`, `ear-rs/`, and `mobile/` for the Android
app), a vanilla TypeScript frontend in `src/` (vitest), and a listener
sidecar: Swift in `ear/` on macOS, Rust in `ear-rs/` on Windows and Linux.

## Requirements

- Rust (rustup), Node 20+, pnpm
- the Xcode Command Line Tools (`xcode-select --install`) for `swiftc`
- `jq` on PATH for the Claude Code hook

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

## Maya's brain

With Claude Code as the brain, Maya keeps one long-running `claude -p`
session (`core/src/brain/`). Her files are in
`<claude_dir>/maya/brain/`:

- `record.json`: the saved session, to resume or start fresh, and when
  her brain went dormant (`dormantSince`) on an assistant.
- `chat.json`: the conversation shared by voice and the Chat tab.
- `watches.json`: sessions and pull requests she asked to hear about.

The modules: `driver` (what every resident brain looks like), `claude`
(the Claude Code driver, stream-json over stdin/stdout), `acp` (Grok and
Kiro, over ACP), `codex_drv` (Codex, `codex app-server`), `agy`
(Antigravity, stream-json; a stop sends SIGINT and relaunches on the
saved conversation), `opencode_drv` (OpenCode, HTTP and SSE on the user's
own OpenCode server), `session` (queue, busy replies, stop, compaction; no
process in it), `protocol` (her fenced `maya` action blocks), `prompt`,
`notes` (board changes since her last turn), `chat`, `record`, `watch` and
`handover` (the summary for a Maya made main). Claude, ACP, Codex and agy
share `stdio` (spawn, line reader, restart and kill), and ACP and Codex
also share `jsonrpc`; they differ only in a small protocol parser. Actions run after a 5 second
window unless the user says stop or corrects.

Every brain is resident on the main machine; the quick one-shot
interpreter with the same window is only the fallback (her session is
down, or the machine is an assistant). Every Maya keeps her own brain: on
an assistant machine it is dormant (no process runs, nothing is reset),
Chat is read-only there, and her card in Debug mode is greyed as
"Dormant". An assistant's `hello` says when its brain went dormant. Make
main asks the outgoing main's brain for a handover summary (60 s at most;
otherwise a digest of her Chat since then), sent
with `promote`; the new main resumes her own session and reads the summary
as her first turn. Settings → Debug (off by default) shows the Debug tab
and her card.

Limits per brain:

- Maya never approves a permission prompt on the agent's behalf. Kiro
  (`--trust-all-tools`) and OpenCode (allow-all ruleset) approve
  everything, by the user's choice; Grok and Codex use their own auto-mode
  checks, and what they escalate is declined. Antigravity's own auto mode
  refuses what it will not run: in headless mode nothing reaches Maya.
- Codex and Kiro have no background subagents, so a long job makes her
  say "I'm in the middle of ..., I'll do that next."
- Grok: ACP cannot list or stop background subagents, so "stop" does not
  reach them and they do not show as working.
- Antigravity sends no TaskStarted/TaskEnded, so its subagents never show
  as working. A stop while she is idle does nothing; a stop during a turn
  sends SIGINT and relaunches on the saved conversation, which also ends
  any background subagent with no report.
- Codex runs workspace-write with no network (its closest-to-auto mode)
  and every escalation is declined, so network jobs (`gh`, `git fetch`)
  fail with the Codex brain.
- OpenCode: her session lives on the user's own OpenCode server (hidden
  from the board) and runs with an allow-everything ruleset.
- Compaction: only the Claude brain is compacted by Maya; the others
  compact themselves.

When her session stops on its own, the quick interpreter takes what she
held and Maya starts her again after 30 s, then 60 s, 120 s and 300 s
while it keeps failing. A turn still running 10 s after a stop, or
silent for 10 minutes, restarts her the same way.

## Usage and limits

The session window's header shows three things for a session: its context
(how full the model's window is), its account's usage limits (windows
with a percentage used and a reset time) and its cost so far. The same
fields sit on the session card (`usage`, `limits`, `status_line`), and
`core/src/collect.rs` fills them. The collectors run off the store lock,
from the list `Store::usage_due` returns, in the app's `poll_usage`
thread and in the CLI's, every 5 s. A probe that fails keeps the last
good reading and logs at most once an hour. Opening a window nudges a
stale reading.

Where each agent's numbers come from:

- Claude Code: limits, context and cost come from the status line
  wrapper below. Limits come from the recording on the machine that
  learned its `rate_limits` last, and every Claude card shares them: a
  status line that re-renders on its refresh interval with unchanged
  numbers is not a new reading. Cost is the recording's
  `total_cost_usd`; with no recording it is estimated from the
  transcript (main conversation only, labelled "est."). The transcript
  totals (`TranscriptTotals` in `core/src/usage.rs`) are incremental,
  at most 8 MB per refresh, so a long transcript catches up over a few
  ticks. `rate_limits` only reaches Pro and Max accounts: with an API
  key the window shows "no limits".
- Codex: both windows and the tokens come from the `token_count` lines
  of the rollout file (`turn_context` names the model). Limits are
  shared across Codex cards. Cost is estimated.
- Kiro: credits per session come from the metering in `<id>.json`.
  Account limits (one `month` window) come from a probe over
  `kiro-cli acp` every 10 min: `_kiro.dev/commands/execute` with
  `{"command":"usage","args":{}}`, the structured `result.data` first
  and the card text as a fallback. This route is unofficial; it was
  checked on Kiro 2.34 and may change. The probe removes only the
  empty session it made.
- Antigravity: one window per model group (`Gemini 7d`,
  `Claude+GPT 7d`) from `agy -p "/usage" --output-format json`, every
  5 min. No per-session cost.
- Grok: cost and tokens from `grok usage <session-id>`, at most once a
  minute and only when the events file changed. No limits ("limits not
  shared").
- OpenCode: tokens and cost from its server's session record. No limits.

The status line wrapper (`core/src/statusline.rs`) is installed from
Settings → Sessions ("Install usage status line" / "Remove usage status
line") or with `maya statusline install|remove|status`. Install writes
`~/.claude/maya/maya-statusline` (POSIX sh and `jq`; on Windows
`maya-hook.exe statusline`) as `statusLine.command` in the user-level
settings file that holds the winning entry (`settings.local.json`, else
`settings.json`). It refuses when the managed settings file holds a
status line, and Settings names the blocked file. Project-level
`.claude/settings.json` status lines are not checked: a session under
such a project records nothing and shows "limits not read yet". The
previous command is saved in `~/.claude/maya/statusline-inner` and runs
unchanged. Each render writes `~/.claude/maya/usage-<session_id>.json`
with the limits, cost, context, model and the rendered line. Maya scans
that folder once per refresh and parses only the files that changed.
Once an hour the scan deletes the usage files not written for 8 days.
The account label is `oauthAccount.emailAddress` from `~/.claude.json`.

The terminal look's bottom line is the recorded status line, drawn with
its ANSI colours; without one it is Maya's own line with the limits and
cost appended.

Prices are list prices per model for the Claude and Codex families in
`core/src/pricing.rs`, fetched on 2026-10-29, each row with its own
long-context threshold. Cost is kept in integer micro-units. Update the
table and its date when the vendors change prices.

## Git integration

The Pull requests tab and each card's PR badge talk to GitHub's and
GitLab's APIs directly (`core/src/forge/`); `gh` is not needed. Settings
→ Git switches each on or off and says where its token comes from: a
pasted token, kept in the OS key store (`maya-github`, `maya-gitlab`), or
a variable read from the login shell (`forge::env`). GitLab's URL defaults
to `https://gitlab.com`. The first start after 0.24 imports `gh auth
token` once, when `gh` is logged in. Review clones run `git clone` over
SSH (the default) or HTTPS, chosen per provider in Settings → Git, with the
user's own git credentials. For an OpenCode review the core runs the git
steps itself, with no terminal, before the session starts
(`reviews::run_git_steps`, with `GIT_TERMINAL_PROMPT=0`): a credential
that needs a prompt fails there, and git's message shows in Maya.

The Deployments tab lists GitHub Actions runs that wait for the user's
approval, with Approve and Reject buttons, and deployment runs queued
behind another run's deployment ("Waiting on <sha> <title>"). When Maya
cannot find the run it waits on (a concurrency group shared by several
workflows), it says "Waiting on another deployment". Only a main or a
standalone Maya checks, every two minutes; on an assistant the Deployments
tab says "Deployments are checked on the main." The check covers the repos the
token can see that were pushed to in the last 7 days. Of those, only repos
with an environment that requires reviewers are asked for runs, and that
answer, a refusal of the token included, is kept for 1 hour per repo, or
until the Git settings change. GitHub only; GitLab is not covered.
Reject rejects the pending deployment, so the run fails at that step; it
does not cancel the whole run. Approve approves every environment in that
run that the user may approve, and only if they are the ones the row showed;
if a check changed them, the click is refused. The token needs `repo` (classic), or, for a
fine-grained token, **Actions: read**, **Deployments: read and write** and
**Metadata: read**.

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
- **Inbox.** The fallback for replies: `actions::send_reply` types a
  Claude Code reply into the terminal (`answer::reply_lines`) and uses the
  inbox only when nothing reached the terminal. `core/src/inbox.rs`: a named pipe, opened with the auth line
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
  the `thread-writer-locks/<id>.lock` it holds. A `kiro-cli-chat` agent
  pid maps to its session through the lock file beside the session, and
  to its console through the TUI that is its parent
  (`foreign::kiro_sessions`).
- **Ear.** `ear-rs/`: the Swift ear's protocol over WASAPI and whisper.cpp,
  with the VAD ported case for case. `cargo run -p maya-ear --release
  --example transcribe -- <model.bin> <file.wav>` runs a WAV through it.

The `windows` job of the build workflow (see [CI](#ci)) runs these tests
and builds the installers.

## Linux

Requirements (Ubuntu 24.04 or later): Rust (rustup), Node 22+, pnpm, and

    sudo apt install build-essential pkg-config libssl-dev cmake clang \
      libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
      librsvg2-dev patchelf libasound2-dev tmux libnotify-bin \
      speech-dispatcher pulseaudio-utils libsecret-tools wmctrl

Then:

    pnpm install
    pnpm ear:build      # maya-ear and maya-hook into src-tauri/binaries/
    pnpm tauri dev
    pnpm test
    pnpm ear:test       # the Rust ear's tests
    cargo test --workspace

    pnpm tauri build --bundles deb,appimage

puts the `.deb` and the AppImage in `target/release/bundle/deb/` and
`appimage/` (`tauri.linux.conf.json` adds the sidecars, the bundle targets
and the `.deb`'s dependencies).

What is different on Linux, and where:

- **Terminal.** `src-tauri/src/terminal_linux.rs`: a session Maya starts
  or resumes runs in a tmux session `maya-<8 hex>`
  (`core/src/terminal_tmux.rs`, shared with the CLI), shown in
  `gnome-terminal --title <label> -- tmux attach -t <label>` (else
  `ptyxis --new-window`, else `x-terminal-emulator`; an emulator that
  exits non-zero within 1.5 s is reported). Typing goes through `tmux send-keys` to the pane
  whose tty is the session's; a session outside tmux refuses typing with
  `NOT_IN_TMUX` and its card has no Terminal button (`src/card.ts`). Focus
  raises the window by title with `wmctrl -a` (X11, XWayland); GNOME on
  Wayland lets no other process raise a window, so there it opens a fresh
  window attached to the same tmux session. `lib.rs`'s
  `fill_terminal_names` gives each card its tmux session's name.
- **Inbox.** The fallback for replies, as on Windows. As on macOS, the session's Unix socket (`core/src/inbox.rs`),
  its owner checked with `SO_PEERCRED`; no token.
- **Hook.** `core/src/hook_install.rs`: the hook is `hook/`'s `maya-hook`,
  bundled beside the app (as `maya-hook` or Tauri's
  `maya-hook-<triple>`), copied to `~/.claude/maya/maya-hook` (0755) by
  Install Claude hook, so the app needs no `jq`. The Codex hook buttons
  use it too (`core/src/codex_hooks.rs`). The CLI, a single binary, still
  installs the `jq` script.
- **Notifications, voice, quiet.** `core/src/notify.rs`: banners with
  `notify-send`, the sound with `canberra-gtk-play`, the built-in voice
  with `spd-say -w` (else `espeak-ng`), and GNOME's Do not disturb from
  `gsettings get org.gnome.desktop.notifications show-banners`. The
  ElevenLabs key is in GNOME Keyring through `secret-tool`, and playback
  is `paplay`, else `ffplay` (`src-tauri/src/voice.rs`); `aplay` cannot
  decode MP3.
- **Other agents.** As on macOS: `ps` and `lsof`.
- **Ear.** `ear-rs/`, as on Windows: the microphone through cpal's ALSA
  host (PipeWire and PulseAudio show as ALSA devices), whisper.cpp built
  in. Settings offers only *Built-in (Whisper)* (`src/platform.ts`).
- **Page.** `src/platform.ts`'s `isLinux()`: "This computer", Ctrl+Enter,
  "speech-dispatcher", "GNOME Keyring".

This is developed from a Mac, so `scripts/linux-vm.sh` drives an Ubuntu
24.04 VM through [Multipass](https://multipass.run) to build, test and
run it:

    sh scripts/linux-vm.sh create    # once: launch the VM and install the toolchain (~15 min)
    sh scripts/linux-vm.sh sync      # copy the working tree to ~/maya in the VM
    sh scripts/linux-vm.sh run 'cargo test --workspace'
    sh scripts/linux-vm.sh shell     # an interactive shell in the VM
    sh scripts/linux-vm.sh destroy   # tear the VM down

`create` launches `maya-ubuntu` (4 CPUs, 8 GB, 40 GB) and installs the
Rust and Node toolchains plus the GTK/WebKit/tmux packages the app and its
tests need, and `xvfb`, `weston` and ImageMagick to run it without a
display. `sync` copies the working tree to `~/maya` in the VM, leaving
out `target/`, `node_modules/`, `dist*/`, `vendor/`, `.superpowers/` and
`src-tauri/binaries/`, then runs `pnpm install` there; re-run it after
local changes before `run` (it replaces `~/maya`, so the next build starts
from scratch). `run '<command>'` runs a shell line in `~/maya` in the VM
with `~/.cargo/bin` on `PATH`. `MAYA_VM` overrides the VM name if more
than one is needed. To see the app, build the AppImage in the VM and run
the `linux-app` job's smoke test there (below), then copy the screenshot
out with `multipass transfer maya-ubuntu:<path> .`.

The `linux-app` job of the build workflow (see [CI](#ci)) runs the whole
workspace's tests, builds the `.deb` and the AppImage on an x86_64 and an
arm64 runner, and smoke-tests the AppImage: under `xvfb-run`, from a path
with a space in it and with an empty `HOME`, it must log `started` to
`~/.claude/maya/maya.log` within 60 s and still be running 3 s later. The
screen is kept as the `smoke-<arch>` run artifact, to see that the board
painted. The VM cannot prove sound, the microphone or GNOME's own banners;
those are the hand checks the README lists.

Beyond `cargo test --workspace`, four checks exercise the parts a click
cannot reach headless. They need the sidecars rebuilt after every `sync`
(`sh scripts/build-ear.sh aarch64-unknown-linux-gnu`) and the AppImage
built once (`pnpm tauri build --bundles appimage`):

- **A headless Wayland session.** `weston --backend=headless-backend.so
  --socket=<name> --width=1280 --height=800 &`, with `XDG_RUNTIME_DIR` set
  to a writable directory, then the AppImage with `WAYLAND_DISPLAY=<name>
  GDK_BACKEND=wayland`. It painted and logged `started` after about 30 s,
  the same as under X11. `weston-screenshooter` could not capture it on
  this VM's weston 13.0.0 (`screenshot_create_shm_buffer: Assertion
  'width > 0' failed`, aborting the client, even with `--width`/`--height`
  set on the backend); the fallback is `GDK_BACKEND=x11` under
  `xvfb-run -a` with `import -window root` — the `linux-app` smoke test's
  own recipe — which does capture the board.
- **A session round trip without clicking.** Clicking a card is not
  scriptable headless, so the same code the buttons use is driven from the
  CLI: `mkdir -p ~/proj/demo && cargo run -p maya-cli -- config
  projects-dir ~/proj && cargo run -p maya-cli -- start --dir demo
  --prompt "hello"` (the target directory must exist before `config
  projects-dir`, which checks it) exercises `Tmux::open` and the label the
  same way the app's Start button does. `tmux ls` lists `maya-<hex>`, and
  `tmux capture-pane -p` shows the pane left open with `claude: not
  found` (the VM has no `claude` binary) rather than closing — the
  `exec $SHELL` fallback the tmux integration test covers; typing into the
  pane is that test's job, not this one's.
- **The hook install.** The Linux-gated test,
  `cargo test -p maya-core hook_install::`, is the check for what a click
  on Install Claude hook cannot run headless. Extracting the AppImage
  (`--appimage-extract`) and listing `squashfs-root/usr/bin` shows `maya`,
  `maya-hook` and `maya-ear` side by side, which is what `hook_source_in`
  expects next to the running binary.
- **Notifications, focus and speech.** `core/examples/linux_probe.rs`
  builds one Awaiting card named "collector" and calls `notify::notify`,
  `notify::focus_active` and `notify::speak_and_wait`, the way the Tauri
  commands do, from a terminal: `cargo run -p maya-core --example
  linux_probe`. Run under `dbus-run-session` with a `dbus-monitor` on the
  session bus filtering `interface=org.freedesktop.Notifications`, no
  `Notify` call appears: `notify-send` only gets as far as
  `GetServerInformation` and gives up, because `dbus-run-session` starts a
  bare session bus with no notification daemon on it
  (`notify-send` by hand confirms the reason: it exits 1 with
  `GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown`). `notify()`
  swallows that error and does not panic. `focus_active` reads `false`
  (no GNOME schema in the VM). `speak_and_wait` picks `spd-say -w`
  (installed) and does not panic either. On this VM `spd-say -w` never
  finishes: there is no PulseAudio server (`pactl info` refuses the
  connection) and no ALSA playback device (`/dev/snd` has only `seq` and
  `timer`, no card), so speech-dispatcher's output module has nothing to
  play to. The line's deadline (15 s plus 1 s per 12 characters) stops it,
  and `speak_and_wait` returns after about 15 s; the `spd-say -C` that
  follows hangs the same way and is killed after 2 s by the app (the probe
  exits sooner and leaves it behind; kill it by pid). That nothing is
  heard is a gap the VM cannot close: a developer with a sound card should
  expect the call to return once it has actually spoken.

## Android

The phone app is a second Tauri crate, `mobile/`, that depends on
`maya_core` and reuses the frontend's board, card, modal and dialogs from
`mobile/web/index.html` through `src/mobile/main.ts`. It always joins a
main, with a pairing code, and never runs a server. It mirrors that
main's board: the main sends it `mirror` frames and answers its
`request`s, as long as the main's "Sees and controls all sessions" switch
for it is on. Its Chat tab is the main's Chat, through the same `chat`
frames. A phone saved as a main by 0.34 opens with networking off. It runs
no sessions, so it needs no sidecar and no `jq`.

Requirements: `rustup` (Homebrew's `rust` has no Android targets), Java 17
or later (CI uses 21), and the Android SDK command-line tools (`brew
install --cask android-commandlinetools`, or Android Studio's SDK Manager)
with:

    sdkmanager "platform-tools" "platforms;android-37.0" "build-tools;36.0.0" "ndk;27.2.12479018"
    rustup target add aarch64-linux-android x86_64-linux-android
    export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools   # or ~/Library/Android/sdk
    export NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"

Then:

    pnpm install
    pnpm mobile:dev            # on the connected phone or the running emulator
    pnpm mobile:build          # a release APK for arm64, debug-signed without a keystore
    pnpm mobile:build --debug
    pnpm mobile:desktop        # the phone's app in a desktop window, for quick page work
    pnpm test                  # the page, src/mobile included
    cargo test -p maya-mobile  # the hub against a core client on localhost

The APK lands in `mobile/gen/android/app/build/outputs/apk/universal/`.
`adb logcat -s RustStdoutStderr:V` shows what the Rust side prints to
stdout and stderr, panics included; Maya's own log is `maya/maya.log` in
the app's data directory, which `adb shell run-as com.dosaki.maya.mobile`
reaches on a debug build. Its network key is `maya/identity.key` in the
same directory. On the emulator the phone's address is not
reachable from the Mac, but the phone only joins: make the Mac the main
and join it from the emulator at `10.0.2.2`, port 4127.

What is where:

- **`mobile/src/hub.rs`** is the app with the page and Android behind
  traits: the config, the client handle that joins the main, the notifier
  diff and the alert state. `mobile/tests/round_trip.rs` drives it with a real core client.
- **`mobile/src/commands.rs`** is the page's commands, by the desktop's
  names, every session command sent to the joined main through
  `Hub::request`.
- **`mobile/src/alerts.rs`** decides which notifications a board change
  posts and clears, the whole text each shows pulled down, and its
  buttons: a single question's first choices (three at most, Android's
  limit) and its own-words row, and a Reply box on a prose ask or a
  finished turn. **`mobile/src/android.rs`** is the `keepalive` plugin
  bridge and the Android notifications, with host no-ops. A press goes
  from `SessionNotificationReceiver` through the channel `watch_buttons`
  hands the plugin to `Hub::answer_pressed`, which sends it as the card
  would and puts the notification back with why when that fails.
- **`mobile/gen/android/`** is the generated project, committed, plus
  `KeepAliveService.kt` (the foreground service), `KeepAlivePlugin.kt`,
  `SessionNotifications.kt` (the sessions' notifications, their buttons
  and the receiver for a press), `VoicePlugin.kt` (the recogniser and the
  voice), `MainActivity.kt`'s `onNewIntent` (it keeps the tapped
  notification's intent) and `res/drawable/ic_stat_maya.xml` (the
  status-bar icon). Everything else in it is Tauri's; regenerate with
  `pnpm mobile android init` only if you must, then restore those six
  files, the manifest's permissions (`RECORD_AUDIO` included), its
  `<queries>` for the speech recogniser and text-to-speech services
  (without them Android 11+ hides both from the app), its service and its
  receiver, `build.gradle.kts`'s signing block, and the
  launcher icon with `cp -R mobile/icons/android/. mobile/gen/android/app/src/main/res/`
  (`sh scripts/check-android-icons.sh`, run in CI, fails without it). (Run
  the Tauri CLI through `pnpm mobile …` from the repository root; `pnpm
  exec tauri` does not work from inside `mobile/`.)
- The Android job of the build workflow (see [CI](#ci)) runs the tests
  and builds the APK; `release.yml` attaches it as `Maya_<version>.apk`.
  With `ANDROID_KEYSTORE` (a `.jks` as base64), `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` set as repository secrets
  the APK is release-signed; without them it is debug-signed. The job
  writes them to `mobile/gen/android/keystore.properties` (ignored by
  git), which `build.gradle.kts` reads; put the same file there to sign a
  local build.
  The secrets are set from the release after 0.21.0, so every release
  APK since is signed with the same key and installs over the one before.
  Keep a copy of the keystore and its password outside GitHub: secrets
  cannot be read back, and a new key means every phone uninstalls once
  more, which forgets the pairings. 0.21.0 and earlier had no secrets, so
  each was signed with a new debug key.

## The CLI

`cli/` builds on its own, with no Node toolchain and no `pnpm ear:build`:

    cargo build -p maya-cli --release        # target/release/maya-cli
    cargo test -p maya-core -p maya-cli

`maya-cli` depends only on `maya_core`, so it builds without GTK or
WebKit, and statically for musl. The build workflow's `linux` job builds
`maya-cli` natively for its musl target (`musl-tools`, no
cross-compilation container), once per architecture on an x86_64 runner
and an `ubuntu-24.04-arm` runner. Their tests run in the `linux-app`
job's `cargo test --workspace` on Ubuntu (with tmux installed), the
guard that keeps `maya_core` free of macOS-only code.

## CI

Three workflows in `.github/workflows/`:

- `build.yml` tests and builds on Linux (x86_64 and aarch64: the CLI in
  `linux`, the app in `linux-app`), Windows, macOS (aarch64 and
  x86_64, one `macos` job each, both on Apple Silicon runners; the
  tests run in the aarch64 one) and Android (the host-side tests and an
  arm64 APK, in `android`), the jobs running in parallel, and keeps
  what each built as run artifacts (`maya-linux-<arch>`,
  `maya-linux-app-<arch>`, `maya-windows`, `maya-macos-<arch>`,
  `maya-android`; and
  `smoke-<arch>`, the Linux smoke test's screenshot, which is not
  released). It is only ever called by the other two.
- `ci.yml` ("CI") runs on every pull request to `main`: `build.yml`
  unsigned, and a check that the version files agree. Nothing is
  published, so a pull request shows everything building before the merge.
- `release.yml` ("Release") runs on every push to `main`: `build.yml`
  again without the tests (the pull request ran them; the Linux smoke
  test still runs), signing the macOS apps when this push releases, and
  then, only once every platform has built, a `release` job that
  publishes `v<version>` with all the artifacts, the `.sig` files and `latest.json`. It releases when
  `scripts/release-version.sh` finds no release for the version yet; a
  push whose version is released only builds, and a `docs` commit skips
  the run.

## Layout

- `Cargo.toml` — the workspace: shared version, dependencies and release profile.
- `core/` — `maya_core`, the platform-neutral core: no Tauri, no AppKit.
- `src-tauri/` — `maya`, the app: Tauri commands, voice, focus and dock; depends on `maya_core`.
- `hook/` — `maya-hook`, the Claude Code hook on Windows and Linux.
- `ear-rs/` — `maya-ear` for Windows and Linux: `vad.rs`, `resample.rs` and the
  microphone choice in the library, the audio and whisper.cpp in `main.rs`.
- `cli/` — `maya_cli`, binary `maya-cli`: the headless assistant for
  SSH boxes and containers (`pair`, `run`, `status`, `hooks`, `config`, `start`);
  depends on `maya_core`, no Tauri.
- `mobile/` — `maya-mobile`, the Android app: `hub.rs`, `commands.rs`,
  `alerts.rs`, `android.rs`, `tests/round_trip.rs`, and `gen/android/`.
- `core/src/` — `store.rs` (session cache), `state.rs` (card states),
  `watcher.rs`, `answer.rs` and `launch.rs` (Terminal automation),
  `terminal_tmux.rs` (tmux sessions, for the CLI and the Linux app),
  `inbox.rs` (Claude Code's session socket), `foreign.rs` with `codex.rs`,
  `antigravity.rs`, `grok.rs`, `kiro.rs` (other agents), `opencode.rs` (OpenCode's
  server client, over `ureq`), `reviews.rs` and `pr.rs`
  (GitHub), `worktree.rs` (a new session's own worktree and branch), `notify.rs` (notifications and speech output), `tty.rs`,
  `interpreter.rs`, `log.rs` (the Debug tab and `maya.log`),
  `transcript.rs` and `diff.rs` (a session's turns for the viewer: tool output, edit diffs),
  `remembered.rs` (the sessions running when Maya stopped, offered back after a restart), `voice_server.rs` (the OpenAI-compatible speech client), and `net/`
  with `protocol.rs` (message types, encode/decode, the pairing window),
  `identity.rs` (this Maya's key pair), `secure.rs` (SPAKE2 pairing, the
  Noise handshakes and the encrypted `Channel`),
  `server.rs` (the main: listener, threads per assistant, pairing,
  command dispatch), `client.rs` (the assistant: connection, backoff,
  board sending), `merge.rs` (merging local and remote cards, stale
  and expiry rules, routing), and `promote.rs` (handing the main role to
  an assistant: the list, its seal, the config rewrites).
- `src-tauri/src/` — `lib.rs` (Tauri commands), `net_app.rs` (the network's
  Tauri side: starting server and client, command execution),
  `terminal_app.rs` (Terminal windows, and the choice between Terminal and
  iTerm2 for typing, focus and new windows), `terminal_iterm.rs` (the iTerm2
  side: AppleScript builders and `ItermApp`), `terminal_win.rs` and
  `terminal_linux.rs` (their Windows and Linux counterparts), `focus.rs` and `dock.rs`
  (AppKit), `keystore.rs` (the OS key store), `voice.rs` (ElevenLabs and the voice server: cache and playback),
  `ear.rs`, `wake.rs`,
  `listener.rs`, `models.rs` (the voice assistant), and
  `remembered_app.rs` (resuming and forgetting what was running, for the banner and the voice; the heartbeat).
- `chatterbox/` — a small OpenAI-compatible speech server (`server.py`, a `Dockerfile`, `compose.yaml` and a `voices/` folder) for Maya's voice server; see [chatterbox.md](chatterbox.md).
- `cli/src/` — `main.rs` and `args.rs` (subcommand parsing), `commands.rs`
  (`pair`, `status`, `hooks`, `config`, `start`), `run_cmd.rs` (`run`, the headless
  loop), `executor.rs` (running the main's commands), `notify.rs` (writes the CLI's own status
  file on connect, disconnect and removal — the CLI never notifies, speaks
  or listens like the app), and `status_file.rs`
  (`~/.claude/maya/cli-status.json`, read by `maya status`).
- `src/` — `main.ts`, `board.ts`, `card.ts`, `modal.ts` with `toolturn.ts` (a tool call's output and diff in the viewer), `reviews.ts`,
  `settings.ts`, `release-notes.ts`, `voice.ts`, `debug.ts`, `tabs.ts`,
  `remembered.ts` (the Sessions banner offering back what ran before a restart, desktop and phone).
- `src/mobile/` — the phone's page: `main.ts`, `strip.ts` (the column
  strip and the one-to-four-column layout), `network.ts` (Settings: the
  list of groups and each group's page, with `settings-history.ts` so
  Back returns to the list), `setup.ts`,
  `notify-tap.ts`, `mobile.css`; `src/machines.ts` is the dialogs'
  machine switch both pages share.
- `ear/` — `main.swift` (audio capture, the System and Whisper engines),
  `vad.swift` (voice activity detection), `vadtest.swift`.
- `docs/superpowers/specs/` — the design documents behind each feature.

## Releasing

Releases follow semver. The version in `src-tauri/tauri.conf.json` is the
release version; bump it everywhere, rewrite `RELEASE_NOTES.md` for it,
commit and push to `main`:

    pnpm version:set 0.2.0      # or: sh scripts/set-version.sh 0.2.0
    $EDITOR RELEASE_NOTES.md    # "# 0.2.0", then ## New and ## Fixed
    git commit -am "chore(release): 0.2.0"
    git push

The Release workflow publishes `v0.2.0` for every platform. Its notes lead
with `RELEASE_NOTES.md` under "Release Notes", then the install text, then
the list GitHub generates from the merged pull requests
(`scripts/release-notes.sh` assembles them; `scripts/test-release-notes.sh`
tests it in CI). The app shows the same file in Settings → Release Notes,
built in at build time (`src/release-notes.ts`), so it is always the
notes of the running version. A push whose version already has
a release only builds; a mismatch between `src-tauri/tauri.conf.json`,
`mobile/tauri.conf.json`, `package.json` and `Cargo.toml`, or a
`RELEASE_NOTES.md` for another version, fails the run,
a pull request's too (see [CI](#ci)); `pnpm version:set` keeps all of
them in step, the phone app's included. Pull requests bump the version themselves: see the rule in
[CLAUDE.md](../CLAUDE.md).

## Updates

The desktop app updates itself from `latest.json`, which every release
publishes beside its assets (`scripts/updater-manifest.sh`, tested by
`scripts/test-updater-manifest.sh` in CI). `src-tauri/src/updater.rs`
checks it through `tauri-plugin-updater` on macOS, Windows and the
AppImage, and through `maya_core::update` on a `.deb`, an MSI install and
the phone, which only show a Download link. Which one is Tauri's stamped
bundle type (`tauri::utils::platform::bundle_type()`). `src/update.ts` is the bar on the board and the
checks (at start, then every 6 hours); a debug build never checks.

Each update is signed with the key in the `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets, and the app checks it against
`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. A release build
turns on `createUpdaterArtifacts` and writes the `.sig` files; a pull
request's does not. A release with any platform's `.sig` missing fails.
Losing the private key means every installed Maya must be reinstalled by
hand once with a new one.

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
