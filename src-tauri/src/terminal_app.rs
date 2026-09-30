//! Opening Terminal.app windows: the AppleScript comes from the core, the
//! activation afterwards needs AppKit, so these live in the app.

use maya_core::launch::applescript_run;
use maya_core::terminal::Terminal;
use std::path::Path;
use std::process::Command;

pub use crate::focus::focus_pid;

/// Runs `cmd` in a new Terminal window.
pub fn open_terminal_with(cmd: &str) -> Result<(), String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(applescript_run(cmd))
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    crate::focus::activate_terminal()
}

fn applescript_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Types `text` (plus Enter) into the Terminal tab on `tty` without activating Terminal.
pub fn applescript_type(tty: &str, text: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        do script "{}" in t
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#,
        applescript_string(text)
    )
}

pub fn type_into_tty(tty: &str, text: &str) -> Result<(), String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(applescript_type(tty, text))
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    if String::from_utf8_lossy(&out.stdout).trim() == "ok" {
        Ok(())
    } else {
        Err(format!("No Terminal tab found for {tty}."))
    }
}

/// Terminal.app: new windows through AppleScript, keys through `do script … in tab`.
pub struct TerminalApp;

/// The app's one terminal, borrowed by every local action.
pub static TERMINAL: TerminalApp = TerminalApp;

impl Terminal for TerminalApp {
    fn open(&self, command: &str, _cwd: &Path, _label: &str) -> Result<Option<String>, String> {
        open_terminal_with(command).map(|_| None)
    }
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        type_into_tty(tty, text)
    }
    fn focus(&self, tty: &str) -> Result<(), String> {
        crate::focus::focus_tty(tty)
    }
    fn name_for_tty(&self, _tty: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_types_without_activating_and_escapes_text() {
        let s = applescript_type("/dev/ttys021", "a\"b\\c");
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(s.contains("do script \"a\\\"b\\\\c\" in t"), "{s}");
        assert!(!s.contains("activate"));
        assert!(s.contains("return \"not found\""));
    }

    #[test]
    fn terminal_app_has_no_names() {
        assert_eq!(TerminalApp.name_for_tty("/dev/ttys001"), None);
    }
}
