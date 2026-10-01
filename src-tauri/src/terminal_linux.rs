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

impl LinuxTerminal {
    pub fn with_tmux(tmux: Tmux) -> Self {
        Self { tmux, windows: Mutex::new(HashSet::new()) }
    }

    pub fn remember(&self, label: &str) {
        self.windows.lock().unwrap().insert(label.to_string());
    }

    pub fn opened(&self, label: &str) -> bool {
        self.windows.lock().unwrap().contains(label)
    }

    fn open_window(&self, label: &str) -> Result<(), String> {
        let (bin, args) = terminal_command(&std::env::var("PATH").unwrap_or_default(), label)?;
        let mut cmd = Command::new(bin);
        maya_core::launch::scrub_appimage_env(&mut cmd);
        let mut child = cmd
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not open a terminal: {e}"))?;
        // gnome-terminal's client exits at once (the server owns the
        // window); reaping it keeps it from lingering as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        self.remember(label);
        Ok(())
    }

    /// wmctrl raises the window by title on an X11 session. Under GNOME on
    /// Wayland gnome-terminal is a native Wayland window, so wmctrl fails
    /// there and Focus opens a fresh window attached to the session instead.
    fn activate(&self, label: &str) -> bool {
        self.opened(label)
            && Command::new("wmctrl")
                .args(["-a", label])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
    }
}

impl Terminal for LinuxTerminal {
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        self.tmux.open(command, cwd, label)?;
        self.open_window(label)?;
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
