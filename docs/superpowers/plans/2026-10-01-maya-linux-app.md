# Maya on Linux Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Maya's desktop app runs on Ubuntu (GNOME, Wayland) with the same features as on macOS and Windows, voice included, shipped as `.deb` and AppImage for x86_64 and aarch64.

**Architecture:** Linux slots into the platform gates the Windows port created. The CLI's tmux terminal moves into core and backs a `LinuxTerminal` that opens attached GNOME Terminal windows; `notify.rs` and `voice.rs` gain Linux branches (`notify-send`, `spd-say`, `gsettings`, `secret-tool`, `paplay`); the Rust ear and the `maya-hook` binary are un-gated for Linux and bundled as sidecars; `build.yml` gains a `linux-app` job that builds, smoke-tests under a virtual display and uploads the bundles. Because this Mac cannot compile the Tauri app for Linux, Task 1 provides an Ubuntu VM through Multipass with a `scripts/linux-vm.sh` helper that syncs the working tree and runs commands there.

**Tech Stack:** Rust 2021, Tauri 2 on WebKitGTK, tmux, GNOME tools (`notify-send`, `spd-say`, `gsettings`, `secret-tool`, `wmctrl`), `cpal` + `whisper-rs`, GitHub Actions (`ubuntu-latest`, `ubuntu-24.04-arm`), Multipass.

**Spec:** `docs/superpowers/specs/2026-10-01-maya-linux-app-design.md`

## Global Constraints

- Platform gates: Linux code is `#[cfg(target_os = "linux")]`; what Linux shares with Windows is `#[cfg(any(windows, target_os = "linux"))]`; macOS-only code that today says `cfg(unix)` becomes `cfg(target_os = "macos")` where Linux gets its own branch. macOS and Windows behaviour is unchanged; their suites stay green.
- Linux work is built and tested in the VM through `sh scripts/linux-vm.sh run '<command>'` (after `sync`); pure helpers are also tested on macOS.
- Messages (verbatim): `No terminal emulator found: install gnome-terminal.`, `This session is not in tmux; only replies reach it.`, `No audio player found: install pulseaudio-utils.`; tmux labels `maya-<8 hex>` and `maya-review-<8 hex>`.
- Linux tool commands: `notify-send --app-name Maya [--icon <path>] <title> <body>`; `gsettings get org.gnome.desktop.notifications show-banners`; `spd-say -w <line>` then `espeak-ng <line>`; `secret-tool store --label "Maya ElevenLabs" service maya account elevenlabs` (key on stdin) and `secret-tool lookup service maya account elevenlabs`; players in order `paplay`, `aplay`, `ffplay -nodisp -autoexit`; `xdg-open <url>`; `wmctrl -a <title>`.
- Hook on Linux: binary `maya-hook`, marker `.claude/maya/maya-hook`, mode 0755; macOS keeps `hook.sh`.
- Page on Linux: "This computer", "Ctrl+Enter", built-in voice "speech-dispatcher", store "GNOME Keyring", recogniser Whisper only.
- Bundles: `Maya_<version>_amd64.deb`, `Maya_<version>_arm64.deb`, `Maya_<version>_amd64.AppImage`, `Maya_<version>_arm64.AppImage`; artifacts `maya-linux-app-<arch>`.
- Per `CLAUDE.md`, this PR bumps the version (a `feat`: minor) in its own `chore(release): <version>` commit as the last task.
- Conventional commits, one concern per commit, trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`; never commit to main, never push.
- Commands on the Mac: `cargo` needs `PATH="$HOME/.cargo/bin:$PATH"`, from the repo root; `pnpm exec vitest run`, `pnpm exec tsc --noEmit`, `sh scripts/test-ear.sh`.

## Review Focus

1. A session started by hand in GNOME Terminal (not in tmux): it shows on the board, replies reach it through the inbox, and the card has no Terminal button; typing gives `This session is not in tmux; only replies reach it.` (Task 4 tests `LinuxTerminal::type_line` on a tty with no pane; Task 5 tests the page hides the button when `terminal` is absent and `machine` is absent on Linux).
2. `gnome-terminal` missing but `x-terminal-emulator` present: the fallback command line is used; neither present: the named error (Task 4 tests `terminal_command`).
3. Not on GNOME (`gsettings` missing or the key absent): not quiet, no error (Task 2 tests `dnd_from_gsettings` on garbage and empty input).
4. Neither `spd-say` nor `espeak-ng` installed: announcements still notify, speech is skipped, one Debug line (Task 2 tests the chooser with an empty PATH).
5. The AppImage run from a path with spaces, and from a non-GNOME session (XFCE on X11): launches, `xdg-open` works, Terminal falls back to `x-terminal-emulator` (Task 6's smoke test runs the AppImage from `/tmp/maya smoke/`; Task 4's command builder is path-agnostic).

---

### Task 1: The Ubuntu VM and `scripts/linux-vm.sh`

**Files:**
- Create: `scripts/linux-vm.sh`
- Modify: `docs/DEVELOPING.md` (a "Linux" section stub: how to create and use the VM)

**Interfaces:**
- Produces: `sh scripts/linux-vm.sh create | sync | run '<command>' | shell | destroy`. `create` launches `maya-ubuntu` (Ubuntu 24.04, 4 CPUs, 8 GB, 40 GB) and installs the toolchain; `sync` copies the working tree (without `target/`, `node_modules/`, `dist*/`, `vendor/`, `.superpowers/`) to `~/maya` in the VM; `run` executes a shell line in `~/maya` there with `~/.cargo/bin` on the PATH; `shell` opens a shell.

- [ ] **Step 1: Write the script**

```sh
#!/bin/sh
# An Ubuntu VM (Multipass) for building and testing Maya's Linux port from
# a Mac. Usage: sh scripts/linux-vm.sh create|sync|run '<cmd>'|shell|destroy
set -eu
cd "$(dirname "$0")/.."
VM="${MAYA_VM:-maya-ubuntu}"
PKGS="build-essential curl git pkg-config libssl-dev cmake clang libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf tmux libnotify-bin speech-dispatcher pulseaudio-utils libasound2-dev xvfb weston imagemagick wmctrl libsecret-tools file"
case "${1:-}" in
  create)
    multipass launch 24.04 --name "$VM" --cpus 4 --memory 8G --disk 40G
    multipass exec "$VM" -- sudo apt-get update
    multipass exec "$VM" -- sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y $PKGS
    multipass exec "$VM" -- sh -c 'curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal'
    multipass exec "$VM" -- sh -c 'curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash - && sudo apt-get install -y nodejs && sudo corepack enable && corepack prepare pnpm@latest --activate'
    multipass exec "$VM" -- mkdir -p maya
    echo "created $VM; now: sh scripts/linux-vm.sh sync" ;;
  sync)
    tar --exclude=./target --exclude=./node_modules --exclude='./dist*' --exclude=./vendor --exclude=./.superpowers --exclude=./src-tauri/binaries -cf - . \
      | multipass exec "$VM" -- sh -c 'rm -rf maya && mkdir maya && tar -xf - -C maya'
    multipass exec "$VM" -- sh -c 'cd maya && pnpm install --frozen-lockfile >/dev/null'
    echo "synced to $VM:~/maya" ;;
  run)
    multipass exec "$VM" -- sh -lc "export PATH=\$HOME/.cargo/bin:\$PATH; cd maya && $2" ;;
  shell)
    multipass shell "$VM" ;;
  destroy)
    multipass delete "$VM" && multipass purge ;;
  *) echo "usage: sh scripts/linux-vm.sh create|sync|run '<cmd>'|shell|destroy" >&2; exit 2 ;;
esac
```

- [ ] **Step 2: Create the VM and check the baseline**

Run: `sh scripts/linux-vm.sh create` (10–15 minutes), then `sh scripts/linux-vm.sh sync`, then `sh scripts/linux-vm.sh run 'cargo test -p maya-core -p maya-cli 2>&1 | tail -n 5'`.
Expected: the core and CLI tests pass in the VM (the same counts CI shows: core 271, cli 38 + 2 integration with tmux present). If `multipass launch` fails for lack of an image, run `multipass find` and use the listed 24.04 alias.

- [ ] **Step 3: Confirm the app crate does not yet build on Linux**

Run: `sh scripts/linux-vm.sh run 'cargo check -p maya 2>&1 | tail -n 15'`.
Expected: errors in `src-tauri/src/lib.rs` about the missing `term` alias (no `cfg(target_os = "linux")` branch) and `open_in_browser`. Record the list in the report; Task 4 removes them.

- [ ] **Step 4: Document and commit**

In `docs/DEVELOPING.md` add a "## Linux" section after "## Windows": the VM commands above, the note that the app crate only builds on Linux, and that CI's `linux-app` job (Task 6) is the authority.

```bash
git add scripts/linux-vm.sh docs/DEVELOPING.md
git commit -m "chore(dev): an Ubuntu VM through Multipass for building the Linux port from a Mac

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Core on Linux: tmux in core, the hook binary, notifications and speech

**Files:**
- Move: `cli/src/tmux.rs` → `core/src/terminal_tmux.rs` (`git mv`), with `maya_core::` paths becoming `crate::`
- Modify: `core/src/lib.rs` (`pub mod terminal_tmux;`), `cli/src/lib.rs` (`pub use maya_core::terminal_tmux as tmux;` so `maya_cli::tmux::Tmux` keeps working), `cli/src/commands.rs`, `cli/src/run_cmd.rs`, `cli/src/executor.rs`, `cli/tests/tmux_integration.rs` (imports), `core/src/hook_install.rs`, `core/src/notify.rs`
- Test: `core/src/notify.rs`, `core/src/hook_install.rs`, `core/src/terminal_tmux.rs` (moved tests)

**Interfaces:**
- Produces: `maya_core::terminal_tmux::{Tmux, NOT_IN_TMUX, new_session_args, send_keys_args, pane_for_tty, LIST_PANES_FORMAT}` (unchanged API).
- Produces in `notify.rs` (Linux): `pub fn notify_send_args(icon: Option<&Path>, title: &str, subtitle: &str, body: &str) -> Vec<String>`; `pub fn dnd_from_gsettings(out: &str) -> bool`; `pub fn speech_command(path: &str) -> Option<(&'static str, Vec<String>)>` where `path` is the `PATH` string and the result is `("spd-say", ["-w"])` or `("espeak-ng", [])`; `pub fn app_icon_path() -> Option<PathBuf>` (`$APPDIR/usr/share/icons/hicolor/128x128/apps/maya.png` when `APPDIR` is set, else `/usr/share/icons/hicolor/128x128/apps/maya.png`, only if the file exists).
- Produces in `hook_install.rs` (Linux): `HOOK_MARKER = ".claude/maya/maya-hook"`, `HOOK_EXE = "maya-hook"`; `pub fn hook_source_in(dir: &Path) -> Option<PathBuf>` returns `dir/maya-hook` when present, else the first file in `dir` whose name starts with `maya-hook-` (a dev build keeps Tauri's triple suffix); `install_to` uses it on the app binary's folder; `install_with(claude_dir, source)` is `cfg(any(windows, target_os = "linux"))` and sets 0755 on unix.

- [ ] **Step 1: Move the tmux module**

```bash
git mv cli/src/tmux.rs core/src/terminal_tmux.rs
```

In `core/src/terminal_tmux.rs` replace `use maya_core::terminal::Terminal;` with `use crate::terminal::Terminal;`, `maya_core::launch::shell_single_quote` with `crate::launch::shell_single_quote`, `maya_core::net::local_hostname` with `crate::net::local_hostname` (tests too). Add `pub mod terminal_tmux;` to `core/src/lib.rs` (alphabetical, after `terminal`). In `cli/src/lib.rs` replace `pub mod tmux;` with `pub use maya_core::terminal_tmux as tmux;`. The CLI's other files keep `crate::tmux::Tmux`.

Run: `cargo test -p maya-core -p maya-cli` on the Mac (and `sh scripts/linux-vm.sh sync && sh scripts/linux-vm.sh run 'cargo test -p maya-core -p maya-cli'`). Expected: all pass, the tmux tests now count under `maya-core`.

- [ ] **Step 2: Failing tests for the notification helpers**

In `core/src/notify.rs`, inside `mod tests`:

```rust
#[test]
fn notify_send_args_carry_app_name_icon_title_and_two_line_body() {
    let args = notify_send_args(Some(Path::new("/usr/share/icons/hicolor/128x128/apps/maya.png")), "collector", "needs a decision", "Allow Bash?");
    assert_eq!(args, ["--app-name", "Maya", "--icon", "/usr/share/icons/hicolor/128x128/apps/maya.png", "--", "collector", "needs a decision\nAllow Bash?"].map(String::from).to_vec());
    assert_eq!(notify_send_args(None, "a", "", "b"), ["--app-name", "Maya", "--", "a", "b"].map(String::from).to_vec());
}

#[test]
fn dnd_is_only_an_explicit_false_from_gsettings() {
    assert!(dnd_from_gsettings("false\n"));
    assert!(!dnd_from_gsettings("true\n"));
    assert!(!dnd_from_gsettings(""));
    assert!(!dnd_from_gsettings("No such schema"));
}

#[test]
fn speech_prefers_spd_say_then_espeak_then_nothing() {
    let d = tempfile::tempdir().unwrap();
    let exe = |name: &str| {
        let p = d.path().join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    assert_eq!(speech_command(""), None);
    exe("espeak-ng");
    assert_eq!(speech_command(&d.path().to_string_lossy()).map(|c| c.0), Some("espeak-ng"));
    exe("spd-say");
    assert_eq!(speech_command(&d.path().to_string_lossy()), Some(("spd-say", vec!["-w".to_string()])));
}
```

(These helpers are `cfg(any(test, target_os = "linux"))` so the tests run on the Mac too; `tempfile` is already a dev-dependency.)

- [ ] **Step 3: Run to verify they fail** — `cargo test -p maya-core notify::` FAILS to compile.

- [ ] **Step 4: Implement the Linux branches**

In `notify.rs`:

```rust
#[cfg(any(test, target_os = "linux"))]
pub fn notify_send_args(icon: Option<&Path>, title: &str, subtitle: &str, body: &str) -> Vec<String> {
    let mut a = vec!["--app-name".to_string(), "Maya".to_string()];
    if let Some(i) = icon {
        a.extend(["--icon".to_string(), i.to_string_lossy().into_owned()]);
    }
    let text = if subtitle.is_empty() { body.to_string() } else if body.is_empty() { subtitle.to_string() } else { format!("{subtitle}\n{body}") };
    a.extend(["--".to_string(), title.to_string(), text]);
    a
}

#[cfg(any(test, target_os = "linux"))]
pub fn dnd_from_gsettings(out: &str) -> bool {
    out.trim() == "false"
}

#[cfg(any(test, target_os = "linux"))]
pub fn speech_command(path: &str) -> Option<(&'static str, Vec<String>)> {
    if crate::launch::find_on_path(path, "spd-say").is_some() {
        return Some(("spd-say", vec!["-w".to_string()]));
    }
    crate::launch::find_on_path(path, "espeak-ng").map(|_| ("espeak-ng", Vec::new()))
}

#[cfg(target_os = "linux")]
pub fn app_icon_path() -> Option<PathBuf> {
    let rel = "usr/share/icons/hicolor/128x128/apps/maya.png";
    let p = match std::env::var("APPDIR") { Ok(d) => Path::new(&d).join(rel), Err(_) => Path::new("/").join(rel) };
    p.is_file().then_some(p)
}

#[cfg(target_os = "linux")]
pub fn focus_active() -> bool {
    Command::new("gsettings").args(["get", "org.gnome.desktop.notifications", "show-banners"]).stdin(Stdio::null()).stderr(Stdio::null()).output()
        .map(|o| dnd_from_gsettings(&String::from_utf8_lossy(&o.stdout))).unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn say_builtin(line: &str) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    match speech_command(&std::env::var("PATH").unwrap_or_default()) {
        Some((bin, args)) => { let _ = Command::new(bin).args(&args).arg(line).stdin(Stdio::null()).status(); }
        None => WARNED.call_once(|| crate::log::line("notify", "no speech synthesiser: install speech-dispatcher or espeak-ng")),
    }
}

#[cfg(target_os = "linux")]
pub fn notify(card: &Card, sound: bool) {
    let icon = app_icon_path();
    let _ = Command::new("notify-send").args(notify_send_args(icon.as_deref(), &card.name, &subtitle_for(card), &body_for(card))).stdin(Stdio::null()).output();
    if sound {
        let _ = Command::new("canberra-gtk-play").args(["-i", "message"]).stdin(Stdio::null()).status();
    }
}
```

Then narrow the existing macOS-only items from `cfg(unix)` to `cfg(target_os = "macos")`: `VOICE`, `voice()`, `focus_active` (the plutil one), `say_builtin` (the `say` one), `notify` (the osascript one), and the `use std::process::{Command, Stdio}` becomes `cfg(unix)` still (both use it). `voice()` is referenced only by the macOS `say_builtin`.

- [ ] **Step 5: Run to verify they pass** — `cargo test -p maya-core notify::` on the Mac; then `sh scripts/linux-vm.sh sync && sh scripts/linux-vm.sh run 'cargo test -p maya-core'` in the VM. Expected: PASS on both.

- [ ] **Step 6: Failing test for the Linux hook install**

In `core/src/hook_install.rs` tests (Linux-gated test, plus a path-selection test that runs everywhere):

```rust
#[test]
fn hook_source_prefers_the_plain_name_then_a_triple_suffixed_one() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(hook_source_in(d.path()), None);
    std::fs::write(d.path().join("maya-hook-x86_64-unknown-linux-gnu"), b"x").unwrap();
    assert_eq!(hook_source_in(d.path()).unwrap().file_name().unwrap(), "maya-hook-x86_64-unknown-linux-gnu");
    std::fs::write(d.path().join("maya-hook"), b"x").unwrap();
    assert_eq!(hook_source_in(d.path()).unwrap().file_name().unwrap(), "maya-hook");
}

#[cfg(target_os = "linux")]
#[test]
fn install_with_copies_the_binary_executable_and_points_hooks_at_it() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("settings.json"), "{}").unwrap();
    let src = d.path().join("src-hook");
    std::fs::write(&src, b"#!/bin/sh\n").unwrap();
    install_with(d.path(), &src).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let installed = d.path().join("maya").join("maya-hook");
    assert_eq!(std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(status(d.path()).unwrap());
    assert!(std::fs::read_to_string(d.path().join("settings.json")).unwrap().contains(".claude/maya/maya-hook\""));
}
```

- [ ] **Step 7: Implement**

```rust
#[cfg(target_os = "linux")]
pub const HOOK_MARKER: &str = ".claude/maya/maya-hook";
#[cfg(windows)]
pub const HOOK_EXE: &str = "maya-hook.exe";
#[cfg(not(windows))]
pub const HOOK_EXE: &str = "maya-hook";

/// The hook binary next to Maya's own: Tauri strips the target triple from
/// a bundled sidecar's name, a dev build keeps it.
pub fn hook_source_in(dir: &Path) -> Option<PathBuf> {
    let plain = dir.join(HOOK_EXE);
    if plain.is_file() { return Some(plain); }
    let prefix = format!("{HOOK_EXE}-");
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.starts_with(&prefix)) && p.is_file())
}
```

`install_to` on Linux (`cfg(target_os = "linux")`): `let exe = current_exe()?; let source = hook_source_in(exe.parent()?).ok_or("Maya's hook binary (maya-hook) is not next to the app")?; install_with(claude_dir, &source)`. `install_with` becomes `cfg(any(windows, target_os = "linux"))` and after writing adds, under `cfg(unix)`, `set_permissions(0o755)`. The macOS `install_to` (`hook.sh`) becomes `cfg(target_os = "macos")`; `HOOK_MARKER`'s `cfg(unix)` arm becomes `cfg(target_os = "macos")`; `TOKEN_ONLY_EVENTS` on Linux = the Windows list (`SessionStart`), since the hook binary records the token the same way (check `record_token` is portable; it is plain file I/O).

- [ ] **Step 8: Verify and commit** — `cargo test -p maya-core` on the Mac and in the VM; `cargo test -p maya-cli` on both; `cargo build --workspace` on the Mac warning-free.

```bash
git add -A core cli
git commit -m "feat(core): Linux notifications, speech, hook install, and tmux in core

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: The Rust ear on Linux

**Files:**
- Modify: `ear-rs/Cargo.toml`, `ear-rs/src/main.rs`, `scripts/build-ear.sh`, `scripts/test-ear.sh`
- Test: `ear-rs` tests in the VM; `maya-ear --selftest` in the VM

- [ ] **Step 1: Un-gate**

`ear-rs/Cargo.toml`: `[target.'cfg(any(windows, target_os = "linux"))'.dependencies]` for `cpal` and `whisper-rs`; description "Maya's ear on Windows and Linux: …". `ear-rs/src/main.rs`: every `#[cfg(windows)]` becomes `#[cfg(any(windows, target_os = "linux"))]`, the stub `#[cfg(not(any(windows, target_os = "linux")))]` with the message "This ear runs on Windows and Linux; macOS has the Swift one in ear/."; rename `mod windows_ear` to `mod ear` and the doc comment's first line to "Maya's ear on Windows and Linux: the microphone through cpal (WASAPI, ALSA), …". Any Windows-only API inside the module (grep `windows_sys`, `os::windows`) gets its own `cfg(windows)` with a Linux equivalent or a no-op; the report lists each.

`scripts/build-ear.sh`: add a case

```sh
  *-unknown-linux-gnu)
    mkdir -p src-tauri/binaries
    cargo build --release --target "$target" -p maya-ear -p maya-hook
    for bin in maya-ear maya-hook; do
      cp "target/$target/release/$bin" "src-tauri/binaries/$bin-$target"
      echo "built src-tauri/binaries/$bin-$target"
    done
    exit 0 ;;
```

`scripts/test-ear.sh`: `Linux) exec cargo test -p maya-ear ;;` in the `uname -s` case.

- [ ] **Step 2: Build and test in the VM**

Run: `sh scripts/linux-vm.sh sync && sh scripts/linux-vm.sh run 'sh scripts/test-ear.sh && sh scripts/build-ear.sh aarch64-unknown-linux-gnu && ls -la src-tauri/binaries'`.
Expected: the ear's 14 tests pass; both sidecars built (`whisper-rs` compiles whisper.cpp with cmake and clang, several minutes the first time). Then `sh scripts/linux-vm.sh run './src-tauri/binaries/maya-ear-aarch64-unknown-linux-gnu --selftest; echo exit $?'`: with no audio device the ear must exit 4 ("no microphone") and print a `devices` line, not crash. If `--selftest` is not an argument the ear accepts, run it with `--engine whisper --model /nonexistent` and expect exit 7.

- [ ] **Step 3: macOS unaffected** — on the Mac `cargo test -p maya-ear` (the stub crate) and `sh scripts/test-ear.sh` (Swift) still pass; `cargo build --workspace` warning-free.

- [ ] **Step 4: Commit**

```bash
git add ear-rs scripts/build-ear.sh scripts/test-ear.sh
git commit -m "feat(ear): the Rust ear listens on Linux too

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: The app on Linux: terminal, voice key and playback, browser, bundle config

**Files:**
- Create: `src-tauri/src/terminal_linux.rs`, `src-tauri/tauri.linux.conf.json`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/voice.rs`, `src-tauri/Cargo.toml` (no new deps expected), `core/src/actions.rs` (review label helper if needed)
- Test: `src-tauri/src/terminal_linux.rs`, `src-tauri/src/voice.rs`; the app crate's tests in the VM

**Interfaces:**
- Produces: `terminal_linux::{LinuxTerminal, TERMINAL, open_terminal_with, focus_pid, terminal_command}`; `pub fn terminal_command(path: &str, label: &str) -> Result<(PathBuf, Vec<String>), String>`.
- Consumes: `maya_core::terminal_tmux::Tmux`, `maya_core::actions::tmux_label()`, `maya_core::launch::find_on_path`, `maya_core::tty::tty_for_pid`.

- [ ] **Step 1: Failing tests**

`src-tauri/src/terminal_linux.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn exe(d: &Path, name: &str) { let p = d.join(name); std::fs::write(&p, "#!/bin/sh\n").unwrap(); use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap(); }

    #[test]
    fn gnome_terminal_first_then_x_terminal_emulator_then_an_error() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().to_string_lossy().into_owned();
        assert_eq!(terminal_command(&path, "maya-1a2b3c4d").unwrap_err(), "No terminal emulator found: install gnome-terminal.");
        exe(d.path(), "x-terminal-emulator");
        let (bin, args) = terminal_command(&path, "maya-1a2b3c4d").unwrap();
        assert!(bin.ends_with("x-terminal-emulator"));
        assert_eq!(args, ["-T", "maya-1a2b3c4d", "-e", "tmux", "attach", "-t", "maya-1a2b3c4d"].map(String::from).to_vec());
        exe(d.path(), "gnome-terminal");
        let (bin, args) = terminal_command(&path, "maya-1a2b3c4d").unwrap();
        assert!(bin.ends_with("gnome-terminal"));
        assert_eq!(args, ["--title", "maya-1a2b3c4d", "--", "tmux", "attach", "-t", "maya-1a2b3c4d"].map(String::from).to_vec());
    }

    #[test]
    fn typing_outside_tmux_is_refused_with_the_shared_message() {
        let t = LinuxTerminal::with_tmux(maya_core::terminal_tmux::Tmux { binary: "/nonexistent/tmux".into() });
        assert_eq!(t.type_line("/dev/pts/9", "x"), Err(maya_core::terminal_tmux::NOT_IN_TMUX.into()));
        assert_eq!(t.name_for_tty("/dev/pts/9"), None);
    }

    #[test]
    fn opened_windows_are_remembered_by_label() {
        let t = LinuxTerminal::with_tmux(maya_core::terminal_tmux::Tmux { binary: "/nonexistent/tmux".into() });
        t.remember("maya-1a2b3c4d");
        assert!(t.opened("maya-1a2b3c4d"));
        assert!(!t.opened("maya-ffffffff"));
    }
}
```

`src-tauri/src/voice.rs` (Linux-gated, `cfg(any(test, target_os = "linux"))` for the pure helper):

```rust
#[test]
fn secret_tool_args_name_the_maya_service_and_account() {
    assert_eq!(secret_tool_store_args(), ["store", "--label", "Maya ElevenLabs", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
    assert_eq!(secret_tool_lookup_args(), ["lookup", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
}

#[test]
fn player_is_paplay_then_aplay_then_ffplay() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().to_string_lossy().into_owned();
    let exe = |name: &str| { let p = d.path().join(name); std::fs::write(&p, "#!/bin/sh\n").unwrap(); use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap(); };
    assert_eq!(player_command(""), None);
    exe("ffplay");
    assert_eq!(player_command(&path), Some(("ffplay", vec!["-nodisp".to_string(), "-autoexit".to_string()])));
    exe("aplay");
    assert_eq!(player_command(&path).map(|c| c.0), Some("aplay"));
    exe("paplay");
    assert_eq!(player_command(&path).map(|c| c.0), Some("paplay"));
}
```

(`play` with no player found returns `Err("No audio player found: install pulseaudio-utils.")`; `tempfile` is a dev-dependency of the app crate already, check `src-tauri/Cargo.toml` and add it if not.)

- [ ] **Step 2: Run to verify they fail** — on the Mac these modules are not compiled; run them in the VM: `sh scripts/linux-vm.sh sync && sh scripts/linux-vm.sh run 'cargo test -p maya terminal_linux:: voice::'` FAILS to compile (the modules do not exist / lib.rs has no Linux `term`).

- [ ] **Step 3: Implement `terminal_linux.rs`**

```rust
//! Linux: sessions live in tmux and show in a terminal window attached to
//! them. Keys go through tmux; focus raises the window Maya opened when the
//! desktop lets it (wmctrl on X11 and XWayland), else opens a fresh one.
use maya_core::launch::find_on_path;
use maya_core::terminal::Terminal;
use maya_core::terminal_tmux::{Tmux, NOT_IN_TMUX};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub const NO_EMULATOR: &str = "No terminal emulator found: install gnome-terminal.";

/// The emulator and its arguments for a window titled `label` attached to
/// the tmux session `label`.
pub fn terminal_command(path: &str, label: &str) -> Result<(PathBuf, Vec<String>), String> {
    if let Some(gt) = find_on_path(path, "gnome-terminal") {
        return Ok((gt, ["--title", label, "--", "tmux", "attach", "-t", label].map(String::from).to_vec()));
    }
    if let Some(xte) = find_on_path(path, "x-terminal-emulator") {
        return Ok((xte, ["-T", label, "-e", "tmux", "attach", "-t", label].map(String::from).to_vec()));
    }
    Err(NO_EMULATOR.into())
}

pub struct LinuxTerminal {
    tmux: Tmux,
    windows: Mutex<HashSet<String>>,
}

/// The app's one terminal, borrowed by every local action.
pub static TERMINAL: std::sync::LazyLock<LinuxTerminal> = std::sync::LazyLock::new(|| LinuxTerminal::with_tmux(Tmux::default()));
```

(`LazyLock` is stable since Rust 1.80; CI and the VM use `stable`. `lib.rs` reaches it as `&*term::TERMINAL`; see the `terminal()` helper below.)

```rust
impl LinuxTerminal {
    pub fn with_tmux(tmux: Tmux) -> Self { Self { tmux, windows: Mutex::new(HashSet::new()) } }
    pub fn remember(&self, label: &str) { self.windows.lock().unwrap().insert(label.to_string()); }
    pub fn opened(&self, label: &str) -> bool { self.windows.lock().unwrap().contains(label) }

    fn open_window(&self, label: &str) -> Result<(), String> {
        let (bin, args) = terminal_command(&std::env::var("PATH").unwrap_or_default(), label)?;
        Command::new(bin).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("could not open a terminal: {e}"))?;
        self.remember(label);
        Ok(())
    }

    /// wmctrl raises by title on X11 and for XWayland windows; on pure
    /// Wayland it fails and the caller opens a fresh window instead.
    fn activate(&self, label: &str) -> bool {
        self.opened(label) && Command::new("wmctrl").args(["-a", label]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }
}

impl Terminal for LinuxTerminal {
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        self.tmux.open(command, cwd, label)?;
        self.open_window(label)?;
        Ok(Some(label.to_string()))
    }
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> { self.tmux.type_line(tty, text) }
    fn focus(&self, tty: &str) -> Result<(), String> {
        let label = self.tmux.name_for_tty(tty).ok_or(NOT_IN_TMUX)?;
        if self.activate(&label) { return Ok(()); }
        self.open_window(&label)
    }
    fn name_for_tty(&self, tty: &str) -> Option<String> { self.tmux.name_for_tty(tty) }
    fn names_for_ttys(&self, ttys: &[String]) -> std::collections::HashMap<String, String> { self.tmux.names_for_ttys(ttys) }
}

/// The review flow: `cmd` in a tmux session of its own, in the home folder.
pub fn open_terminal_with(cmd: &str) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let label = format!("maya-review-{}", &maya_core::actions::tmux_label()[5..]);
    TERMINAL.open(cmd, &home, &label).map(|_| ())
}

pub fn focus_pid(pid: i32) -> Result<(), String> {
    TERMINAL.focus(&maya_core::tty::tty_for_pid(pid)?)
}
```

(`dirs` is a dependency of the app crate already through core? Check `src-tauri/Cargo.toml`; add `dirs.workspace = true` if not.) In `lib.rs`: `#[cfg(target_os = "linux")] pub mod terminal_linux;`, `#[cfg(target_os = "linux")] pub(crate) use terminal_linux as term;`, and a Linux `open_in_browser` running `xdg-open` with the same success check as macOS. `local()` uses `&*term::TERMINAL` on Linux (and keeps `&term::TERMINAL` elsewhere; write a tiny `fn terminal() -> &'static dyn Terminal` per platform to avoid the `*` difference).

- [ ] **Step 4: Implement `voice.rs` Linux**

```rust
#[cfg(any(test, target_os = "linux"))]
pub fn secret_tool_store_args() -> Vec<String> { ["store", "--label", "Maya ElevenLabs", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec() }
#[cfg(any(test, target_os = "linux"))]
pub fn secret_tool_lookup_args() -> Vec<String> { ["lookup", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec() }
#[cfg(any(test, target_os = "linux"))]
pub fn player_command(path: &str) -> Option<(&'static str, Vec<String>)> {
    for (bin, args) in [("paplay", vec![]), ("aplay", vec![]), ("ffplay", vec!["-nodisp".to_string(), "-autoexit".to_string()])] {
        if maya_core::launch::find_on_path(path, bin).is_some() { return Some((bin, args)); }
    }
    None
}
#[cfg(target_os = "linux")]
pub fn store_key(key: &str) -> Result<(), String> {
    use std::io::Write;
    let key = key.trim();
    if key.is_empty() {
        return Err("The key is empty.".into());
    }
    let mut child = maya_core::command("secret-tool").args(secret_tool_store_args()).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
    child.stdin.take().ok_or("no stdin for secret-tool")?.write_all(key.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(format!("GNOME Keyring refused the key: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}
#[cfg(target_os = "linux")]
pub fn load_key() -> Option<String> {
    let out = maya_core::command("secret-tool").args(secret_tool_lookup_args()).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() { return None; }
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!key.is_empty()).then_some(key)
}
#[cfg(target_os = "linux")]
pub fn play(path: &Path) -> Result<(), String> {
    let (bin, args) = player_command(&std::env::var("PATH").unwrap_or_default()).ok_or("No audio player found: install pulseaudio-utils.")?;
    let ok = maya_core::command(bin).args(args).arg(path).stdin(Stdio::null()).status().map_err(|e| format!("could not run {bin}: {e}"))?;
    if ok.success() { Ok(()) } else { Err(format!("{bin} failed")) }
}
```

- [ ] **Step 4b: Local cards carry their tmux name on Linux**

The page must know which local sessions are in tmux (the Terminal button is shown only for those, Task 5). In `src-tauri/src/lib.rs` `refresh_and_emit`, after `store.refresh(now_ms())` and before the merge, on Linux only:

```rust
#[cfg(target_os = "linux")]
fn fill_terminal_names(store: &mut Store, cards: &mut [model::Card]) {
    static TTYS: Mutex<Option<std::collections::HashMap<i32, String>>> = Mutex::new(None);
    let mut cache = TTYS.lock().unwrap();
    let cache = cache.get_or_insert_with(Default::default);
    let live: std::collections::HashSet<i32> = cards.iter().map(|c| c.pid).collect();
    cache.retain(|pid, _| live.contains(pid));
    let mut ttys = Vec::with_capacity(cards.len());
    for c in cards.iter() {
        let known = store.session(&c.session_id).and_then(|s| s.tty).or_else(|| store.foreign(&c.session_id).and_then(|f| f.tty));
        let tty = known.or_else(|| cache.get(&c.pid).cloned()).or_else(|| tty::tty_for_pid(c.pid).ok());
        if let Some(t) = &tty { cache.insert(c.pid, t.clone()); }
        ttys.push(tty);
    }
    let wanted: Vec<String> = ttys.iter().flatten().cloned().collect();
    let names = term::TERMINAL.names_for_ttys(&wanted);
    for (c, tty) in cards.iter_mut().zip(ttys) {
        c.terminal = tty.and_then(|t| names.get(&t).cloned());
    }
}
```

called inside the `store` lock block of `refresh_and_emit` and in `list_sessions` (whichever builds the cards the page gets; find both call sites of `store.refresh`). This mirrors the CLI executor's `board()`; a later clean-up can move both into core. Test: in the VM, the tmux integration test from Task 2 plus a unit test of the cache pruning if it is factored into a pure function `prune_ttys(cache, live_pids)`.

- [ ] **Step 5: `tauri.linux.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "bundle": {
    "targets": ["deb", "appimage"],
    "externalBin": ["binaries/maya-ear", "binaries/maya-hook"],
    "linux": {
      "deb": {
        "depends": ["libwebkit2gtk-4.1-0", "libgtk-3-0", "tmux", "libnotify-bin", "speech-dispatcher"],
        "recommends": ["gnome-terminal", "pulseaudio-utils", "lsof", "wmctrl", "libsecret-tools"]
      }
    }
  }
}
```

(Check the Tauri 2 schema for `recommends`; if absent, drop that key and list the tools in the README instead.)

- [ ] **Step 6: Build and test in the VM**

Run: `sh scripts/linux-vm.sh sync && sh scripts/linux-vm.sh run 'sh scripts/build-ear.sh aarch64-unknown-linux-gnu && cargo test -p maya && pnpm tauri build --bundles deb,appimage 2>&1 | tail -n 6 && ls target/release/bundle/deb target/release/bundle/appimage'`.
Expected: the app's tests pass on Linux; a `.deb` and an `.AppImage` exist. Then launch under a virtual display to see it paint: `sh scripts/linux-vm.sh run 'HOME=$(mktemp -d) xvfb-run -a sh -c "./target/release/bundle/appimage/*.AppImage --appimage-extract-and-run & sleep 15; import -window root /tmp/maya.png; kill %1"; echo done'`, then `multipass transfer maya-ubuntu:/tmp/maya.png /tmp/maya-linux.png` and Read the PNG: the board must be visible.

- [ ] **Step 7: macOS unaffected and commit** — on the Mac: `cargo test --workspace`, `cargo build --workspace` warning-free, `pnpm exec tsc --noEmit`.

```bash
git add src-tauri core
git commit -m "feat(app): Maya runs on Linux: tmux terminal windows, GNOME Keyring, notifications and speech

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: The page on Linux

**Files:**
- Modify: `src/platform.ts`, `src/settings.ts` (the "this PC"/"this Mac" hint uses `thisComputer()`)
- Test: `src/platform.test.ts`, `src/settings.test.ts`

- [ ] **Step 1: Failing tests**

```ts
it("tells Linux apart from Android and from the other desktops", () => {
  withUserAgent("Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15", () => {
    expect(isLinux()).toBe(true);
    expect(thisComputer()).toBe("This computer");
    expect(sendShortcut()).toBe("Ctrl+Enter");
    expect(builtinVoiceName()).toBe("speech-dispatcher");
    expect(secretStore()).toBe("GNOME Keyring");
    expect(recognizerOptions().map(([v]) => v)).toEqual(["builtin"]);
  });
  withUserAgent("Mozilla/5.0 (Linux; Android 14) …", () => expect(isLinux()).toBe(false));
});
```

(`withUserAgent` is whatever `platform.test.ts` already uses to stub `navigator.userAgent`; reuse it.) In `settings.test.ts`: the model hint reads "it stays on this computer." under the Linux user agent. In `card.test.ts`: under the Linux user agent, a local card with `terminal: "maya-1a2b3c4d"` renders the Terminal button and a local card without `terminal` does not (on the macOS and Windows user agents both render it, as today); the button's tooltip on Linux reads `Open the terminal attached to maya-1a2b3c4d`. Find where `card.ts` renders the Terminal button (grep `data-action="terminal"` or the button text) and gate it with `!isLinux() || card.terminal`.

- [ ] **Step 2: Run to verify they fail** — `pnpm exec vitest run src/platform.test.ts` FAILS (`isLinux` missing).

- [ ] **Step 3: Implement**

```ts
export function isLinux(): boolean {
  return /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);
}
```

and in each existing function a Linux branch first: `thisComputer` → "This computer"; `sendShortcut` → "Ctrl+Enter"; `builtinVoiceName` → "speech-dispatcher"; `secretStore` → "GNOME Keyring"; `recognizerOptions` → `[["builtin", "Built-in (Whisper, runs on this computer)"]]`. In `settings.ts` the hint becomes `` `Download the model once; it stays on ${thisComputer().toLowerCase()}.` `` (gives "this mac", so keep the Mac/PC words by mapping: Linux "this computer", Windows "this PC", macOS "this Mac" via a new `thisComputerLower()` in `platform.ts`).

- [ ] **Step 4: Verify and commit** — `pnpm exec vitest run`, `pnpm exec tsc --noEmit`.

```bash
git add src
git commit -m "feat(ui): the page knows it runs on Linux

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: CI: the `linux-app` job with a smoke test; install script; docs; version

**Files:**
- Modify: `.github/workflows/build.yml`, `install.sh`, `README.md`, `docs/DEVELOPING.md`, the spec's `Status:` line
- Version: `sh scripts/set-version.sh <main's minor + 1>.0` as the final commit

- [ ] **Step 1: The job**

Add to `build.yml` after the `linux` job:

```yaml
  # The desktop app for Linux: the whole workspace's tests, the two
  # sidecars, a .deb and an AppImage per architecture, and a launch under a
  # virtual display that must paint the board.
  linux-app:
    name: Linux app (${{ matrix.arch }})
    runs-on: ${{ matrix.runner }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - arch: x86_64
            runner: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            deb: amd64
          - arch: aarch64
            runner: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            deb: arm64
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with:
          version: 11
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
      - uses: dtolnay/rust-toolchain@stable
      - uses: swatinem/rust-cache@v2
        with:
          workspaces: .
      - name: Install the GTK, audio and desktop tools
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf cmake clang libasound2-dev tmux libnotify-bin speech-dispatcher pulseaudio-utils xvfb imagemagick
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
      - name: Build the sidecars
        run: sh scripts/build-ear.sh ${{ matrix.target }}
      - name: Test
        run: |
          pnpm test
          sh scripts/test-ear.sh
          cargo test --workspace
      - name: Build the bundles
        run: |
          set -euo pipefail
          version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
          pnpm tauri build --bundles deb,appimage
          mkdir -p dist
          cp target/release/bundle/deb/*.deb "dist/Maya_${version}_${{ matrix.deb }}.deb"
          cp target/release/bundle/appimage/*.AppImage "dist/Maya_${version}_${{ matrix.deb }}.AppImage"
          chmod +x dist/*.AppImage
          ls -la dist
      - name: Smoke test under a virtual display
        run: |
          set -euo pipefail
          mkdir -p "/tmp/maya smoke" && cp dist/*.AppImage "/tmp/maya smoke/Maya.AppImage"
          export HOME="$(mktemp -d)"
          xvfb-run -a sh -c '"/tmp/maya smoke/Maya.AppImage" --appimage-extract-and-run & pid=$!
            for i in $(seq 1 40); do grep -q "started" "$HOME/.claude/maya/maya.log" 2>/dev/null && break; sleep 0.5; done
            sleep 3; import -window root smoke-${{ matrix.arch }}.png
            kill -0 $pid || { echo "Maya exited early"; exit 1; }
            grep -q "started" "$HOME/.claude/maya/maya.log" || { echo "Maya never logged started"; cat "$HOME/.claude/maya/maya.log" || true; exit 1; }
            kill $pid'
      - uses: actions/upload-artifact@v4
        with:
          name: maya-linux-app-${{ matrix.arch }}
          path: dist/*
          if-no-files-found: error
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: smoke-${{ matrix.arch }}
          path: smoke-${{ matrix.arch }}.png
          if-no-files-found: ignore
```

Validate: `ruby -ryaml -e "YAML.load_file('.github/workflows/build.yml')"`. The release job's `pattern: maya-*` already picks `maya-linux-app-*` up. Rehearse the smoke script in the VM before committing (same commands, with `sh scripts/linux-vm.sh run`).

- [ ] **Step 2: `install.sh` Linux branch**

Replace the `Linux) fail …` case with a branch that: picks `amd64`/`arm64` from `uname -m`, finds the `.AppImage` URL in the latest release JSON (same `asset_url` approach matching `_<arch>\.AppImage$`), downloads to `${MAYA_INSTALL_DIR:-$HOME/.local/bin}/maya-app`, `chmod +x`, writes `~/.local/share/applications/maya.desktop`:

```
[Desktop Entry]
Type=Application
Name=Maya
Exec=<dest>/maya-app
Icon=maya
Categories=Development;
```

and prints `Installed Maya to <dest>/maya-app` and `Requirements: tmux, libnotify-bin, speech-dispatcher (sudo apt install tmux libnotify-bin speech-dispatcher)`. Test by running `sh install.sh --print-url` in the VM (prints the AppImage URL for arm64) once a release with the asset exists; before that, test the branch's URL filter with a saved JSON.

- [ ] **Step 3: Docs**

README: a "### Linux" subsection under "Install" (the one-liner, requirements, what differs: tmux windows, the Terminal button opening a new attached window on Wayland, GNOME Keyring, Whisper only, hand checks for real hardware: hear a notification and the voice, say "Maya, what's waiting on me?", click Terminal). DEVELOPING: complete the "## Linux" section (what is different and where, mirroring "## Windows"; the VM; the `linux-app` job and its smoke test). Spec `Status: implemented`.

- [ ] **Step 4: Commits and the version bump**

```bash
git add .github/workflows/build.yml
git commit -m "ci: build, smoke-test and release Maya for Linux on both architectures

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
git add install.sh README.md docs
git commit -m "docs(linux): install and use Maya on Ubuntu

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
sh scripts/set-version.sh 0.3.0   # a feat: the minor after main's version; if main moved past 0.2.x, use its minor + 1
git add package.json src-tauri/tauri.conf.json Cargo.toml Cargo.lock
git commit -m "chore(release): 0.3.0

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Verification in the VM

**Files:**
- Modify: `docs/DEVELOPING.md` (the "Linux" section gains the VM verification recipe used here)
- No product code; the report records what was verified.

- [ ] **Step 1: A headless Wayland session with a terminal**

In the VM: `weston --backend=headless --socket=maya-wl &` then with `WAYLAND_DISPLAY=maya-wl` and `XDG_RUNTIME_DIR` set: start the AppImage; wait for `started` in the log; screenshot through weston's screenshooter (`weston-screenshooter` writes `wayland-screenshot-*.png`) and Read it.

- [ ] **Step 2: Session round trip without clicking**

Clicking in the app is not scriptable headless, so drive the same code the buttons use: start a `claude`-stand-in session in tmux through the CLI's path — `sh scripts/linux-vm.sh run 'cargo run -p maya-cli -- config projects-dir ~/proj && mkdir -p ~/proj/demo && cargo run -p maya-cli -- start --dir demo --prompt "hello"'` — which exercises `Tmux::open` and the label; then `tmux ls` shows `maya-…`; `tmux send-keys` typing is covered by the tmux integration test. Record the outputs.

- [ ] **Step 3: Hook install**

The Tauri command cannot be clicked headless; the Linux-gated test `install_with_copies_the_binary_executable_and_points_hooks_at_it` (Task 2) is the check, run in the VM: `sh scripts/linux-vm.sh run 'cargo test -p maya-core hook_install::'`. Record its output. Also confirm `hook_source_in` finds the sidecar next to the AppImage's binary: `sh scripts/linux-vm.sh run './target/release/bundle/appimage/*.AppImage --appimage-extract >/dev/null && ls squashfs-root/usr/bin'` must list `maya` and `maya-hook` (and `maya-ear`).

- [ ] **Step 4: Notifications, DND, speech, keyring calls**

A small committed example, `core/examples/linux_probe.rs` (`#[cfg(target_os = "linux")]`, a stub `main` elsewhere), builds one Awaiting card named "collector" and calls `notify::notify(&card, false)`, prints `notify::focus_active()`, and speaks one line through `notify::speak_and_wait`. In the VM: `sh scripts/linux-vm.sh run 'dbus-run-session -- sh -c "dbus-monitor --session \"interface=org.freedesktop.Notifications\" > /tmp/dbus.log & sleep 1; cargo run -p maya-core --example linux_probe; sleep 1; grep -c Notify /tmp/dbus.log"'`. Expected: at least one `Notify` member in the log and the probe printing `focus_active: false` (no GNOME schema in the VM) and the speech command it chose or the "no speech synthesiser" line. Record.

- [ ] **Step 5: Commit the recipe**

```bash
git add docs/DEVELOPING.md core/examples/linux_probe.rs
git commit -m "docs(linux): how the port is verified in the VM

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
