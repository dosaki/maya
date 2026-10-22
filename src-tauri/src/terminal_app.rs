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

/// Types a reply's `lines` into the Terminal tab on `tty` in one script, so
/// the reply goes in as one burst. `do script` always ends with its own
/// carriage return, so each line ends in ESC to make that Alt+Enter
/// (`answer::NEW_LINE`), and the last in `ESC [ I`, the report of the
/// terminal gaining focus, so the carriage return after it reaches Claude
/// Code on its own and sends the reply.
pub fn applescript_type_reply(tty: &str, lines: &[String]) -> String {
    let last = lines.len().saturating_sub(1);
    let typed: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| format!("do script (\"{}\" & (character id 27){}) in t", applescript_string(l), if i < last { "" } else { " & \"[I\"" }))
        .collect();
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        {}
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#,
        typed.join("\n        ")
    )
}

/// Brings the Terminal tab on `tty` and its window forward, then presses Esc
/// (key code 53) through System Events: `do script` always ends with a
/// carriage return, so it cannot send a bare Esc.
pub fn applescript_press_escape(tty: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        set selected of t to true
        set index of w to 1
        activate
        tell application "System Events" to key code 53
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#
    )
}

/// What osascript says when Maya may not send keys (-1719, -25211).
fn needs_accessibility(stderr: &str) -> bool {
    stderr.contains("-1719") || stderr.contains("-25211") || stderr.contains("not allowed assistive access")
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
    /// Only Esc: through System Events, which needs Accessibility permission.
    fn press_keys(&self, tty: &str, keys: &str) -> Result<(), String> {
        if keys != maya_core::answer::ESC {
            return Err("Only Esc can be pressed in Terminal.app.".into());
        }
        let out = Command::new("osascript").arg("-e").arg(applescript_press_escape(tty)).output().map_err(|e| format!("could not run osascript: {e}"))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
            if needs_accessibility(&stderr) {
                return Err("Maya needs Accessibility permission to press Esc in Terminal (System Settings → Privacy & Security → Accessibility).".into());
            }
            return Err(format!("osascript failed: {}", stderr.trim()));
        }
        if String::from_utf8_lossy(&out.stdout).trim() == "ok" {
            Ok(())
        } else {
            Err(format!("No Terminal tab found for {tty}."))
        }
    }
    /// One script for the whole reply. A script that ran but failed may have
    /// typed some lines, so only a missing tab or osascript counts as none.
    fn type_reply(&self, tty: &str, lines: &[String]) -> Result<(), (usize, String)> {
        let out = Command::new("osascript").arg("-e").arg(applescript_type_reply(tty, lines)).output().map_err(|e| (0, format!("could not run osascript: {e}")))?;
        if !out.status.success() {
            return Err((lines.len(), format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        if String::from_utf8_lossy(&out.stdout).trim() == "ok" {
            Ok(())
        } else {
            Err((0, format!("No Terminal tab found for {tty}.")))
        }
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
    fn escape_script_targets_the_tab_and_presses_key_code_53() {
        let s = applescript_press_escape("/dev/ttys004");
        assert!(s.contains("tty of t is \"/dev/ttys004\""), "{s}");
        assert!(s.contains("key code 53"), "{s}");
        assert!(!s.contains("do script"), "{s}");
    }

    #[test]
    fn only_escape_can_be_pressed_in_terminal_app() {
        assert_eq!(TerminalApp.press_keys("/dev/ttys004", "x"), Err("Only Esc can be pressed in Terminal.app.".into()));
    }

    #[test]
    fn terminal_app_has_no_names() {
        assert_eq!(TerminalApp.name_for_tty("/dev/ttys001"), None);
    }

    #[test]
    fn a_reply_is_typed_into_the_tab_in_one_script() {
        let s = applescript_type_reply("/dev/ttys021", &["push it\\ ".to_string(), "say \"hi\"".to_string()]);
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(
            s.contains("do script (\"push it\\\\ \" & (character id 27)) in t\n        do script (\"say \\\"hi\\\"\" & (character id 27) & \"[I\") in t\n        return \"ok\""),
            "{s}"
        );
        assert!(s.ends_with("return \"not found\""));
    }
}
