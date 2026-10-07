# Maya for Android: a main Maya in your pocket — design

Date: 2026-10-07
Status: draft for review
Builds on: `2026-09-30-maya-remote-assistants-design.md`,
`2026-09-30-maya-cli-design.md`

## Purpose

Carry the board with you. The Android app is a **main** Maya and nothing
else: it runs no agents, has no sessions of its own, and watches and drives
the assistants paired with it. Your desktop Maya becomes one of those
assistants, as do any `maya-cli` boxes. From the phone you see every
session move between the four columns, answer a decision, reply, start or
resume a session on any assistant, and get a notification when a session
needs you with the phone in your pocket.

Decisions taken during design:

- **Topology.** The phone is the server; assistants pair with it and
  connect to it, exactly as they do with a desktop main. The alternative,
  the phone connecting to a desktop main as a remote view, is noted for the
  iPhone, where no foreground service exists (see Out of scope).
- **Stack.** Tauri 2 Android, in a new thin crate of this workspace that
  depends on `maya_core` and reuses the frontend's board, card, modal and
  dialog modules. The existing server, protocol, pairing, merge and routing
  code ship unchanged. Expo and native Kotlin were weighed and set aside:
  a phone that is a WebSocket *server* is foreign to React Native, and a
  native UI would duplicate the TypeScript board.
- **Alerts.** Android notifications through a foreground service. Speech
  comes later.
- **Actions.** Full card parity with the desktop's remote cards, plus the
  "+" and Resume dialogs.

## Behaviour

### Roles and first run

- The phone is only ever a main. There is no assistant role and no role
  menu.
- First start shows a setup screen: a Name for the phone (the device's
  model name by default, `ro.product.model`, or "Android" when that is
  blank), the Port (4127) and a Start button. Start begins the server and
  the foreground service (below), asks for notification permission on
  Android 13 and later, and moves to the board with the Network screen's
  pairing code showing.
- The server listens on every interface. The Network screen shows the
  phone's current Wi‑Fi (and any other non-loopback IPv4) addresses beside
  the pairing code, so the user can type host and code into an assistant's
  Settings. Pairing, tokens, the challenge-response handshake, the
  three-wrong-codes lockout and the handshake slots are the core server's,
  unchanged (`core/src/net/server.rs`).
- The config is the core's `Config`, saved with `config::save` at
  `<app data dir>/maya/config.json` (Tauri's `app_data_dir`); the phone
  never reads `~/.claude`. Only the network fields and the two notification
  switches matter; the rest keep their defaults.

### Network screen

A tab beside the board, as Settings is on the desktop:

- Name and Port. Changing either restarts the server; assistants reconnect
  on their own.
- The pairing code with when it expires and Regenerate, and the addresses.
- The paired assistants list as on the desktop: label, platform, address,
  "Connected" or "Last seen <time>", the version note when one applies, and
  Remove. The `network_status`, `network_pairing_code` and
  `network_remove_assistant` commands keep their desktop names and shapes,
  and the `network` event carries the core's `NetworkStatus`.
- Two switches: "Notify on Awaiting Decision" and "Notify on Completed"
  (the config's `notify_on_awaiting` and a new `notify_on_completed`,
  default on; the desktop ignores the second).
- "Keep Maya awake": opens Android's battery-optimisation exemption dialog
  for the app, with a line saying why ("Android may otherwise stop the
  server with the screen off.").
- Stop server, which also ends the foreground service. The board then says
  "The server is stopped." with a Start button.

A Debug tab shows the same log lines as the desktop's, the `network` ones
included, through `log_lines` and `log_clear`.

### Board

The four columns, Idle, Working, Awaiting Decision and Completed, with the
desktop's order rules, filled by `merge::merged` over the connected
assistants' boards with no local cards.

- **Portrait** shows one column at full width. A strip at the top names the
  four columns with their counts; tapping a name or swiping sideways moves
  to that column (the board is a horizontal scroll-snap container, one
  column per viewport width). The app opens on Awaiting Decision when it
  has cards, else on Working; a rotation keeps the current column in view.
- **Landscape** shows as many columns as fit at 200dp or more, up to four:
  four on most phones, three on narrow ones, four on tablets. Columns
  beyond that scroll sideways with snapping; the strip stays, shows every
  count, and marks the columns on screen. The count comes from the width
  alone (`Math.min(4, Math.max(1, Math.floor(width / 200)))`, with the
  gaps taken off), so the same rule gives one column in portrait.
- Age labels, stale greying after 30 s without a snapshot or on a
  disconnect, and disappearance after five minutes follow the core's
  constants. A board the phone cannot decode keeps the last good one and
  shows the note beside the assistant, as on the desktop.
- With no assistant paired the board shows "Pair an assistant to see its
  sessions." and a link to the Network screen; with assistants paired but
  none connected, each column is empty and the strip's counts are zero.

### Cards

Remote cards exactly as the desktop main shows them (`src/card.ts`
unchanged): the machine glyph, "<project> on <label>", the state bar, the
age, the answer buttons when a question is open, the PR chip, Compact and
Close, and no Terminal button. Labels follow `merge::display_names`:
`name (address)` for assistants sharing a name.

Tapping a card opens it full screen rather than in a centred modal: the
existing `modal.ts` with a phone layout (the panel fills the viewport, the
header carries a Back button). Everything in it works as on the desktop:

- the conversation, fetched with `history` as the desktop does (when the
  card's state, state time or snippet moves, and at most every 3 s while
  it is working);
- reply, with attachments chosen through the system file picker; the
  picked file is saved with `save_attachment` under `<app data
  dir>/maya/attachments/` and travels inline in the `reply` command, the
  same 20 MB cap;
- `/` and `!` lines, Compact, Close, Rename, model, effort and mode;
- the PR chip opens the pull request in the phone's browser; links in the
  conversation open there too.

Every action is the existing `command` to the assistant, found with
`merge::route` with an empty list of local ids, and the command's `result`
is the Tauri command's result. Errors show where the action was taken, as
on the desktop ("Session is no longer running.", "<label> is not
connected").

### Start and resume

The existing "+" and Resume dialogs (`src/newsession.ts`, `src/resume.ts`),
full screen. `list_machines` returns the connected assistants only, so the
Machine picker has no "This Mac"; it picks the first assistant by default.
Folders and agents come from the assistant's last `board`. With no
assistant connected the dialogs show "Pair an assistant first." in place
of the form and a link to the Network screen. An assistant with no folders
gets the desktop's hint, "Set a projects directory in Settings on
<label>."

### Staying alive

- The Rust server runs inside the app process, as it does on the desktop.
  When it starts, the app starts `KeepAliveService`, a Kotlin foreground
  service of type `connectedDevice`, whose only job is to keep the process
  alive: a persistent notification ("Maya — main for 2 assistants, 1
  connected", updated on every status change), a partial wake lock and a
  Wi‑Fi lock so the radio stays up with the screen off. Stop server stops
  it, and its notification goes.
- The manifest declares `FOREGROUND_SERVICE`,
  `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `CHANGE_WIFI_STATE` (the type's
  prerequisite), `WAKE_LOCK`, `INTERNET`, `ACCESS_NETWORK_STATE` and
  `POST_NOTIFICATIONS`.
- What survives what: screen off and the app in the background, the server
  runs on. Android killing the process regardless (swiped away from
  Recents, memory pressure): the server is gone until the app is next
  opened; the persistent notification disappearing is the sign, and the
  app starts the server again on open when the config says it was running.
  Assistants reconnect with their existing backoff as soon as it is back,
  and what began while the phone was away is announced on reconnect, under
  the desktop's rule (the first board after the app starts is seeded, a
  reconnect within the same run is compared).
- A changed Wi‑Fi address needs a hand: the Network screen shows the
  current addresses, and the assistants' Host field has to follow.

### Notifications

- On every `board_changed` the app merges the boards and runs the core's
  `Notifier` (`take_new`, `take_finished`), with the desktop's seeding:
  `board_seeded` feeds `Notifier::seed`, so nothing on an assistant's first
  board after pairing or after the app starts is announced.
- Each card found posts an Android notification through
  `tauri-plugin-notification`: title from the desktop's line ("hexgrid on
  maya-mini needs a decision", "hexgrid on maya-mini is finished"), body
  from `notify::body_for` (the question, or the snippet).
- Two channels: Decisions (high importance, sound, vibration) and Finished
  (default importance), each behind its switch.
- One notification per session: its id is a stable hash of the session id,
  so a new state replaces the old notification, and it is cancelled when
  the card leaves Awaiting Decision (or Completed, for the Finished one)
  or disappears from the board.
- Tapping one opens the app on that card (the notification's `extra`
  carries the session id; the page listens for the plugin's action event
  and opens the card); if the card is gone it opens the board on the
  column the app would open on.
- No mute switch and no speech: Android's Do Not Disturb covers quiet
  hours, and either channel can be silenced in system settings.

### Failure modes

- The port is taken: the Network screen shows the core's `main_error`
  under Port, and the service is not started.
- Notification permission refused: the server runs, the board works, and
  the Network screen says "Notifications are off for Maya in Android
  settings." beside the switches.
- The battery exemption refused: nothing changes; the line stays.
- A command times out (30 s, the core's) or the assistant is gone: the
  core's error shows where the action was taken.
- Malformed frames, unknown assistants and wrong codes are the core
  server's business and are logged and refused as on the desktop.

## Components

### New crate `mobile/`

Workspace member, package `maya-mobile`, Tauri identifier
`com.dosaki.maya.mobile`. Depends on `maya_core`, `tauri`,
`tauri-plugin-notification`, `tauri-plugin-dialog` (the file picker),
`tauri-plugin-opener` (links and PRs), `serde`, `serde_json`.

- `mobile/src/lib.rs` — the Tauri entry: `AppState` (the `Config` and its
  path, the `ServerHandle`, the `Notifier`, the merged cards), the
  `Notify` adapter (`board_changed` merges, diffs, emits `sessions`, posts
  and cancels notifications, and updates the service's notification;
  `board_seeded` seeds; `status_changed` emits `network`; `paired` and
  `paired_list_changed` save the config), start and stop of the server.
- `mobile/src/commands.rs` — the Tauri commands, the same names and
  argument shapes the page calls on the desktop: `list_sessions`,
  `session_history`, `send_reply`, `answer_question`, `set_session_option`,
  `cycle_session_mode`, `send_slash_command`, `rename_session`,
  `compact_session`, `close_session`, `save_attachment`, `open_url`,
  `open_pr`, `list_machines`, `list_project_dirs`, `list_agents`,
  `list_resumable_sessions`, `resume_session`, `start_session`,
  `network_status`, `network_pairing_code`, `network_remove_assistant`,
  `get_config`, `set_config`, `log_lines`, `log_clear`; and the phone's
  own: `server_start`, `server_stop`, `local_addresses`,
  `request_battery_exemption`, `notifications_allowed`. Every session
  command routes with `merge::route(&[], …)` and sends with
  `send_command_with`; there are no local branches.
- `mobile/src/android.rs` — the Android specifics: the device model, the
  interface addresses (`getifaddrs` through `libc`), and the JNI calls into
  the Kotlin side (`KeepAliveService.start/stop/update`, the battery
  exemption intent, the notification-permission check) through Tauri's
  Android plugin mechanism (`tauri::plugin::mobile`).
- `mobile/gen/android/` — the generated Android project, committed as
  Tauri expects, plus `KeepAliveService.kt`, the manifest entries for the
  service, its type and the permissions above, and the two notification
  channels created on start.
- `mobile/tauri.conf.json` — the workspace version, `frontendDist`
  pointing at the mobile build, identifier and icon; `mobile/build.rs`
  as Tauri's template.
- `scripts/set-version.sh` also edits `mobile/tauri.conf.json`, and the
  CI version check reads it.

### Core

One field: `notify_on_completed` on `Config` (default true, ignored by the
desktop until it wants it). Nothing else: `config::load`/`save` take a
path, the server, merge and routing are platform-neutral, and the notifier
and line builders are pure.

### Frontend

- `mobile.html` and `src/mobile/main.ts`, a second Vite entry
  (`vite.config.ts` builds both; the desktop bundle does not change). It
  wires the same events (`sessions`, `network`) and the same modules as
  `src/main.ts`, without reviews, voice, mute, first-run and settings.
- Reused unchanged: `board.ts`, `card.ts`, `modal.ts`, `answer.ts`,
  `actions.ts`, `newsession.ts`, `resume.ts`, `types.ts`, `markdown.ts`,
  `toast.ts`, `debug.ts`, `progress.ts`, `options.ts`, `clickguard.ts`,
  `harness.ts`, `icons.ts`, `format.ts`.
- New under `src/mobile/`: `strip.ts` (the column strip, the active and
  visible columns, the opening column, the column count for a width),
  `network.ts` (the Network screen), `setup.ts` (first run and the
  stopped-server state), `notify-tap.ts` (opening a card from a
  notification), `attach.ts` (the picker in place of drag-and-drop, behind
  `save_attachment`), and `mobile.css` layered over `styles.css`: the
  scroll-snap board, the strip, the full-screen card and dialogs, and touch
  sizes.
- `modal.ts`, `newsession.ts` and `resume.ts` mount themselves into the
  document and call the Tauri commands by name, so the mobile page reuses
  them as they are and the phone layout is CSS. Three desktop assumptions
  need a seam: the modal's Terminal button (hidden on remote cards
  already), the pickers' "This Mac" entry (`src/platform.ts`'s
  `thisComputer`, which the mobile entry turns off through a new
  `platform` switch so `list_machines` alone fills the picker), and the
  window drag-and-drop listener (which simply never fires on a phone).
  Each change is made in place, behind an option the desktop keeps its
  default for, and covered by the modules' existing tests.

### Build, CI and release

- `docs/DEVELOPING.md` gains an Android section: Android Studio's SDK and
  NDK (`ANDROID_HOME`, `NDK_HOME`), Java 17,
  `rustup target add aarch64-linux-android x86_64-linux-android`, then
  `pnpm mobile:dev` and `pnpm mobile:build` (wrappers over `tauri android
  dev` and `tauri android build --apk` run with `mobile/` as the Tauri
  directory). No sidecar, no `jq`.
- `build.yml` gains an `android` job: Ubuntu, Java 17, the SDK and NDK from
  the standard setup action, the two Rust targets, `cargo test -p
  maya-mobile`, `pnpm test`, then an arm64 APK. Signing follows the macOS
  pattern: with `ANDROID_KEYSTORE` (base64), `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` set the APK is
  release-signed; without them it is debug-signed, which installs on your
  own phone. The artifact is `maya-android`, and `release.yml`'s `release`
  job attaches `Maya_<version>.apk` beside the other bundles.
- The version is the workspace's; a pull request bumps it as CLAUDE.md
  says, and the phone and the assistants carry the same compatibility rule
  as two desktops ("both need 0.x or later").
- The README gets an Android section: download the APK from the release,
  allow installs from the browser, open Maya, Start, then on each assistant
  choose "Assistant to a main Maya" with the phone's address and code. The
  hand checks (below) are listed there, as the Linux ones are.

## Testing

- **Unit, Rust (`maya-mobile`, host-side):** routing with no local ids
  reaches the right machine and refuses an expired board; the `Notify`
  adapter's diff posts one request per new ask, none from a seeded board,
  replaces on a state change and cancels on a card leaving the column;
  stable notification ids per session; the default name falls back to
  "Android"; the service's status line for 0, 1 and n assistants.
- **Unit, TypeScript (vitest):** the column count for widths (360 → 1,
  640 → 3, 915 → 4, 1280 → 4); the strip's counts, active column and
  visible columns; the opening column with and without awaiting cards;
  the Network screen's rendering of the code, addresses, assistants list
  and switches; the dialogs' "Pair an assistant first." state; the
  picker's attachment entry producing the same model the drop zone does.
- **Integration, Rust, localhost:** the mobile crate's server started
  with its adapter, a core client pairing, sending a board, a `reply` from
  `send_reply` round-tripping to a `result`, and a later board with a new
  ask producing exactly one notification request and a seeded first board
  producing none.
- **By hand on a phone (listed in the README):** pair the desktop as an
  assistant; rotate between one and four columns and swipe in portrait;
  answer a decision from the phone; lock the phone, have a session ask,
  and receive the notification and open the card from it; start a session
  on the desktop from the phone; stop and start the server and watch the
  assistant reconnect.

## Out of scope

- **Speech.** Android text-to-speech of the core's `spoken_line` behind a
  third switch; one Kotlin call behind a command. Planned next.
- **iPhone.** `tauri ios init` on this crate gives the UI. iOS has no
  foreground service, so a listening server lives only while the app is
  on screen; the iPhone needs the other topology, the phone connecting to
  a desktop main as a remote view, which is a protocol extension on the
  desktop too. The connection layer is kept to `lib.rs` and
  `commands.rs` so that mode can be added without touching the page.
- The Pull Requests tab and reviews (they run `gh` on the main).
- Voice commands and the microphone.
- Discovery of the phone by assistants (mDNS); the address is typed.
- Encryption on the wire, as on the desktop.
- Play Store publishing; the APK is a release asset.
