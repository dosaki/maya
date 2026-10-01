//! Linux: sessions live in tmux and show in a terminal window attached to
//! them. Keys go through tmux; focus raises the window Maya opened on an X11
//! session (wmctrl), else (GNOME on Wayland) opens a fresh one.
use maya_core::launch::find_on_path;
use maya_core::terminal::Terminal;
use maya_core::terminal_tmux::{Tmux, NOT_IN_TMUX};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const NO_EMULATOR: &str = "No terminal emulator found: install gnome-terminal.";

/// How long a terminal that fails at once (Ptyxis given flags it rejects,
/// a broken install) is watched before Maya counts the window as opened.
const EARLY_EXIT_WINDOW: Duration = Duration::from_millis(1500);

/// The emulator and its arguments for a window titled `label` attached to
/// the tmux session `label`: GNOME Terminal, else Ptyxis (Ubuntu 25.04 and
/// later, where `x-terminal-emulator` is Ptyxis and rejects `-T`/`-e`),
/// else `x-terminal-emulator`.
pub fn terminal_command(path: &str, label: &str) -> Result<(PathBuf, Vec<String>), String> {
    if let Some(gt) = find_on_path(path, "gnome-terminal") {
        return Ok((gt, ["--title", label, "--", "tmux", "attach", "-t", label].map(String::from).to_vec()));
    }
    if let Some(p) = find_on_path(path, "ptyxis") {
        return Ok((p, ["--new-window", "--title", label, "--", "tmux", "attach", "-t", label].map(String::from).to_vec()));
    }
    if let Some(xte) = find_on_path(path, "x-terminal-emulator") {
        return Ok((xte, ["-T", label, "-e", "tmux", "attach", "-t", label].map(String::from).to_vec()));
    }
    Err(NO_EMULATOR.into())
}

pub struct LinuxTerminal {
    tmux: Tmux,
    windows: Mutex<HashSet<String>>,
    /// Where the emulator and `wmctrl` are looked for; None is `$PATH`.
    path: Option<String>,
}

/// The app's one terminal, borrowed by every local action.
pub static TERMINAL: std::sync::LazyLock<LinuxTerminal> = std::sync::LazyLock::new(|| LinuxTerminal::with_tmux(Tmux::default()));

impl LinuxTerminal {
    pub fn with_tmux(tmux: Tmux) -> Self {
        Self { tmux, windows: Mutex::new(HashSet::new()), path: None }
    }

    #[cfg(test)]
    fn on_path(mut self, path: &str) -> Self {
        self.path = Some(path.to_string());
        self
    }

    fn search_path(&self) -> String {
        self.path.clone().unwrap_or_else(|| std::env::var("PATH").unwrap_or_default())
    }

    pub fn remember(&self, label: &str) {
        self.windows.lock().unwrap().insert(label.to_string());
    }

    pub fn opened(&self, label: &str) -> bool {
        self.windows.lock().unwrap().contains(label)
    }

    fn open_window(&self, label: &str) -> Result<(), String> {
        let (bin, args) = terminal_command(&self.search_path(), label)?;
        self.spawn_window(&bin, &args, label)
    }

    /// Starts the emulator and watches it briefly: one that exits at once
    /// with an error opened nothing, and saying so beats a Start that seems
    /// to do nothing. gnome-terminal's client exits 0 at once (its server
    /// owns the window); an emulator that owns its window keeps running.
    fn spawn_window(&self, bin: &Path, args: &[String], label: &str) -> Result<(), String> {
        let mut cmd = Command::new(bin);
        maya_core::launch::scrub_appimage_env(&mut cmd);
        let mut child = cmd
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not open a terminal: {e}"))?;
        let name = bin.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let deadline = Instant::now() + EARLY_EXIT_WINDOW;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => return Err(format!("The terminal ({name}) exited at once: {status}")),
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    // Still running: reaping it later keeps it from
                    // lingering as a zombie.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    break;
                }
            }
        }
        self.remember(label);
        Ok(())
    }

    /// wmctrl raises the window by title on an X11 session. Under GNOME on
    /// Wayland gnome-terminal is a native Wayland window, so wmctrl fails
    /// there and Focus opens a fresh window attached to the session instead.
    fn activate(&self, label: &str) -> bool {
        self.opened(label)
            && find_on_path(&self.search_path(), "wmctrl").is_some_and(|wmctrl| {
                Command::new(wmctrl)
                    .args(["-a", label])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
    }
}

impl Terminal for LinuxTerminal {
    /// The emulator is found first, so no tmux session is left behind
    /// when there is none.
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        let (bin, args) = terminal_command(&self.search_path(), label)?;
        self.tmux.open(command, cwd, label)?;
        self.spawn_window(&bin, &args, label)?;
        Ok(Some(label.to_string()))
    }

    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        self.tmux.type_line(tty, text)
    }

    fn focus(&self, tty: &str) -> Result<(), String> {
        let label = self.tmux.name_for_tty(tty).ok_or(NOT_IN_TMUX)?;
        if self.activate(&label) {
            return Ok(());
        }
        self.open_window(&label)
    }

    fn name_for_tty(&self, tty: &str) -> Option<String> {
        self.tmux.name_for_tty(tty)
    }

    fn names_for_ttys(&self, ttys: &[String]) -> HashMap<String, String> {
        self.tmux.names_for_ttys(ttys)
    }
}

/// The review flow: `cmd` in a tmux session of its own, in the home folder.
pub fn open_terminal_with(cmd: &str) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let label = format!("maya-review-{}", &maya_core::actions::tmux_label()[5..]);
    TERMINAL.open(cmd, &home, &label).map(|_| ())
}

/// Brings forward the terminal of the session running as `pid`.
pub fn focus_pid(pid: i32) -> Result<(), String> {
    TERMINAL.focus(&maya_core::tty::tty_for_pid(pid)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(d: &Path, name: &str) {
        let p = d.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn script(d: &Path, name: &str, body: &str) {
        let p = d.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn gnome_terminal_first_then_ptyxis_then_x_terminal_emulator_then_an_error() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().to_string_lossy().into_owned();
        assert_eq!(terminal_command(&path, "maya-1a2b3c4d").unwrap_err(), "No terminal emulator found: install gnome-terminal.");
        exe(d.path(), "x-terminal-emulator");
        let (bin, args) = terminal_command(&path, "maya-1a2b3c4d").unwrap();
        assert!(bin.ends_with("x-terminal-emulator"));
        assert_eq!(args, ["-T", "maya-1a2b3c4d", "-e", "tmux", "attach", "-t", "maya-1a2b3c4d"].map(String::from).to_vec());
        // Ubuntu 25.04 and later: x-terminal-emulator is Ptyxis, which
        // rejects -T and -e, so Ptyxis is asked for by name first.
        exe(d.path(), "ptyxis");
        let (bin, args) = terminal_command(&path, "maya-1a2b3c4d").unwrap();
        assert!(bin.ends_with("ptyxis"));
        assert_eq!(args, ["--new-window", "--title", "maya-1a2b3c4d", "--", "tmux", "attach", "-t", "maya-1a2b3c4d"].map(String::from).to_vec());
        exe(d.path(), "gnome-terminal");
        let (bin, args) = terminal_command(&path, "maya-1a2b3c4d").unwrap();
        assert!(bin.ends_with("gnome-terminal"));
        assert_eq!(args, ["--title", "maya-1a2b3c4d", "--", "tmux", "attach", "-t", "maya-1a2b3c4d"].map(String::from).to_vec());
    }

    #[test]
    fn a_terminal_that_exits_at_once_with_an_error_is_reported() {
        let d = tempfile::tempdir().unwrap();
        script(d.path(), "gnome-terminal", "exit 3");
        let t = LinuxTerminal::with_tmux(Tmux { binary: "/nonexistent/tmux".into() }).on_path(&d.path().to_string_lossy());
        assert_eq!(t.open_window("maya-1a2b3c4d"), Err("The terminal (gnome-terminal) exited at once: exit status: 3".into()));
        assert!(!t.opened("maya-1a2b3c4d"));
    }

    #[test]
    fn a_terminal_that_exits_cleanly_or_keeps_running_counts_as_opened() {
        let d = tempfile::tempdir().unwrap();
        // gnome-terminal's client hands the window to its server and exits 0.
        script(d.path(), "gnome-terminal", "exit 0");
        let t = LinuxTerminal::with_tmux(Tmux { binary: "/nonexistent/tmux".into() }).on_path(&d.path().to_string_lossy());
        assert_eq!(t.open_window("maya-1a2b3c4d"), Ok(()));
        assert!(t.opened("maya-1a2b3c4d"));
        // An emulator that owns its window keeps running.
        let e = tempfile::tempdir().unwrap();
        script(e.path(), "x-terminal-emulator", "exec sleep 5");
        let t = LinuxTerminal::with_tmux(Tmux { binary: "/nonexistent/tmux".into() }).on_path(&e.path().to_string_lossy());
        let started = std::time::Instant::now();
        assert_eq!(t.open_window("maya-1a2b3c4d"), Ok(()));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert!(t.opened("maya-1a2b3c4d"));
    }

    #[test]
    fn without_an_emulator_open_fails_before_any_tmux_session_is_made() {
        let d = tempfile::tempdir().unwrap();
        let t = LinuxTerminal::with_tmux(Tmux { binary: "/nonexistent/tmux".into() }).on_path(&d.path().to_string_lossy());
        // A bogus tmux would fail with "tmux is not installed"; the emulator
        // error shows tmux was never asked.
        assert_eq!(t.open("true", Path::new("/"), "maya-1a2b3c4d"), Err(NO_EMULATOR.into()));
    }

    #[test]
    fn focus_outside_tmux_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let t = LinuxTerminal::with_tmux(Tmux { binary: "/nonexistent/tmux".into() }).on_path(&d.path().to_string_lossy());
        assert_eq!(t.focus("/dev/pts/7"), Err(NOT_IN_TMUX.into()));
    }

    #[test]
    fn focus_opens_a_fresh_window_when_the_remembered_one_cannot_be_raised() {
        let d = tempfile::tempdir().unwrap();
        let marker = d.path().join("opened");
        script(d.path(), "tmux", "printf '/dev/pts/7\\tmaya-1a2b3c4d:0.0\\tmaya-1a2b3c4d\\n'");
        script(d.path(), "wmctrl", &format!("echo \"$@\" > '{}'; exit 1", d.path().join("wmctrl-args").display()));
        script(d.path(), "gnome-terminal", &format!("echo \"$@\" > '{}'", marker.display()));
        let t = LinuxTerminal::with_tmux(Tmux { binary: d.path().join("tmux") }).on_path(&d.path().to_string_lossy());
        t.remember("maya-1a2b3c4d");
        assert_eq!(t.focus("/dev/pts/7"), Ok(()));
        assert_eq!(std::fs::read_to_string(d.path().join("wmctrl-args")).unwrap().trim(), "-a maya-1a2b3c4d", "raising was tried first");
        assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "--title maya-1a2b3c4d -- tmux attach -t maya-1a2b3c4d");
    }

    #[test]
    fn focus_stops_when_the_remembered_window_is_raised() {
        let d = tempfile::tempdir().unwrap();
        let marker = d.path().join("opened");
        script(d.path(), "tmux", "printf '/dev/pts/7\\tmaya-1a2b3c4d:0.0\\tmaya-1a2b3c4d\\n'");
        script(d.path(), "wmctrl", "exit 0");
        script(d.path(), "gnome-terminal", &format!("touch '{}'", marker.display()));
        let t = LinuxTerminal::with_tmux(Tmux { binary: d.path().join("tmux") }).on_path(&d.path().to_string_lossy());
        t.remember("maya-1a2b3c4d");
        assert_eq!(t.focus("/dev/pts/7"), Ok(()));
        assert!(!marker.exists());
    }

    #[test]
    fn typing_outside_tmux_is_refused_with_the_shared_message() {
        let t = LinuxTerminal::with_tmux(maya_core::terminal_tmux::Tmux { binary: "/nonexistent/tmux".into() });
        assert_eq!(t.type_line("/dev/pts/9", "x"), Err(maya_core::terminal_tmux::NOT_IN_TMUX.into()));
        assert_eq!(t.name_for_tty("/dev/pts/9"), None);
    }
}
