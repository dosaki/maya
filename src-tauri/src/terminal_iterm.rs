//! Typing into iTerm2 sessions. The AppleScript comes from builders, so it can
//! be tested as text, and a thin runner hands it to `osascript`. This is the
//! iTerm2 counterpart of `terminal_app.rs`.

use crate::focus::activate_app;
use maya_core::answer::reply_keys;
use maya_core::launch::applescript_string;
use maya_core::terminal::Terminal;
use std::path::Path;
use std::process::Command;

/// iTerm2's bundle id. Scripts address the app by it, and ask whether it is
/// running first, so that typing never starts iTerm2.
pub const BUNDLE_ID: &str = "com.googlecode.iterm2";

/// An AppleScript expression for `keys`: text in quotes, and each control
/// character (Escape, Enter, a newline) as `(character id N)`, because a raw
/// control character cannot sit inside a line of script.
pub fn applescript_keys(keys: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut text = String::new();
    for c in keys.chars() {
        if c.is_control() {
            if !text.is_empty() {
                parts.push(format!("\"{}\"", applescript_string(&text)));
                text.clear();
            }
            parts.push(format!("(character id {})", c as u32));
        } else {
            text.push(c);
        }
    }
    if !text.is_empty() || parts.is_empty() {
        parts.push(format!("\"{}\"", applescript_string(&text)));
    }
    parts.join(" & ")
}

/// Writes `keys` into the iTerm2 session on `tty`, in any window, tab or split
/// pane, without activating iTerm2. With `newline` the write ends in Enter;
/// without it the keys go in untouched. Returns "ok", or "not found" when no
/// session has that tty or iTerm2 is not running.
pub fn applescript_type(tty: &str, keys: &str, newline: bool) -> String {
    let end = if newline { "" } else { " newline false" };
    format!(
        r#"if application id "{BUNDLE_ID}" is running then
  tell application id "{BUNDLE_ID}"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with s in sessions of t
          if tty of s is "{}" then
            tell s to write text {}{end}
            return "ok"
          end if
        end repeat
      end repeat
    end repeat
  end tell
end if
return "not found""#,
        applescript_string(tty),
        applescript_keys(keys)
    )
}

/// Opens a new iTerm2 window and types `cmd` into its shell, the way Terminal's
/// `do script` does: a `command` parameter would skip the login shell, so
/// `claude` would not be on the PATH, and the window would close when the
/// command ends. If iTerm2 is not running it is started and its own first
/// window is used, so a cold start leaves one window and not two.
pub fn applescript_open(cmd: &str) -> String {
    let cmd = applescript_keys(cmd);
    format!(
        r#"if application id "{BUNDLE_ID}" is running then
  tell application id "{BUNDLE_ID}"
    set newWindow to (create window with default profile)
    tell current session of newWindow to write text {cmd}
  end tell
else
  tell application id "{BUNDLE_ID}" to activate
  tell application id "{BUNDLE_ID}"
    set ready to false
    repeat 30 times
      if (count of windows) > 0 then
        if (count of tabs of window 1) > 0 then
          set ready to true
          exit repeat
        end if
      end if
      delay 0.1
    end repeat
    if not ready then create window with default profile
    tell current session of current window to write text {cmd}
  end tell
end if"#
    )
}

/// Selects the iTerm2 session on `tty`, its tab and its window, without
/// activating iTerm2 (activation is done separately). Window last: selecting
/// a window reorders `windows`, which would leave the references taken from it
/// pointing at another window.
pub fn applescript_focus(tty: &str) -> String {
    format!(
        r#"if application id "{BUNDLE_ID}" is running then
  tell application id "{BUNDLE_ID}"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with s in sessions of t
          if tty of s is "{}" then
            select s
            select t
            select w
            return "ok"
          end if
        end repeat
      end repeat
    end repeat
  end tell
end if
return "not found""#,
        applescript_string(tty)
    )
}

/// Runs `script` and returns what it printed.
fn run(script: String) -> Result<String, String> {
    let out = Command::new("osascript").arg("-e").arg(script).output().map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// iTerm2: keys reach a session by its tty, through AppleScript's `write text`.
pub struct ItermApp;

impl Terminal for ItermApp {
    fn open(&self, command: &str, _cwd: &Path, _label: &str) -> Result<Option<String>, String> {
        run(applescript_open(command))?;
        activate_app(BUNDLE_ID)?;
        Ok(None)
    }
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        match run(applescript_type(tty, text, true))?.as_str() {
            "ok" => Ok(()),
            _ => Err(format!("No iTerm2 session found for {tty}.")),
        }
    }
    /// One write for the whole reply. A script that ran but failed may have
    /// typed some of it, so only a missing session counts as nothing typed.
    fn type_reply(&self, tty: &str, lines: &[String]) -> Result<(), (usize, String)> {
        let out = run(applescript_type(tty, &reply_keys(lines), false)).map_err(|e| (lines.len(), e))?;
        if out == "ok" {
            Ok(())
        } else {
            Err((0, format!("No iTerm2 session found for {tty}.")))
        }
    }
    fn focus(&self, tty: &str) -> Result<(), String> {
        match run(applescript_focus(tty))?.as_str() {
            "ok" => activate_app(BUNDLE_ID),
            _ => Err(format!("no iTerm2 session found for {tty}")),
        }
    }
    fn name_for_tty(&self, _tty: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lookup_script_checks_iterm_is_running_first() {
        let s = applescript_type("/dev/ttys021", "hi", true);
        let running = s.find("if application id \"com.googlecode.iterm2\" is running").expect(&s);
        let tell = s.find("tell application id").expect(&s);
        assert!(running < tell, "{s}");
        assert!(s.ends_with("return \"not found\""), "{s}");
    }

    #[test]
    fn lookup_walks_every_window_tab_and_session() {
        let s = applescript_type("/dev/ttys021", "hi", true);
        assert!(s.contains("repeat with w in windows"), "{s}");
        assert!(s.contains("repeat with t in tabs of w"), "{s}");
        assert!(s.contains("repeat with s in sessions of t"), "{s}");
        assert!(s.contains("if tty of s is \"/dev/ttys021\""), "{s}");
    }

    #[test]
    fn open_types_the_command_into_a_new_window_shell() {
        let s = applescript_open("cd '/r' && claude");
        assert!(s.contains("create window with default profile"), "{s}");
        assert!(s.contains("write text \"cd '/r' && claude\""), "{s}");
        // A `command` parameter would skip the login shell, so `claude` would not be on the PATH.
        assert!(!s.contains("command \""), "{s}");
    }

    #[test]
    fn open_reuses_the_first_window_of_a_cold_start() {
        let s = applescript_open("claude");
        let (running, cold) = s.split_once("\nelse\n").expect(&s);
        assert!(!running.contains("activate"), "{s}");
        assert!(cold.contains("activate"), "{s}");
        assert!(cold.contains("repeat 30 times"), "{s}");
        assert!(cold.contains("(count of tabs of window 1) > 0"), "{s}");
        assert!(cold.contains("write text \"claude\""), "{s}");
        // Only when no window ever appears does a cold start make its own.
        assert!(cold.contains("if not ready then"), "{s}");
    }

    #[test]
    fn open_escapes_the_command() {
        let s = applescript_open("echo \"hi\" \\ there");
        assert!(s.contains("write text \"echo \\\"hi\\\" \\\\ there\""), "{s}");
    }

    #[test]
    fn focus_selects_session_then_tab_then_window_without_activating() {
        let s = applescript_focus("/dev/ttys021");
        assert!(s.contains("if tty of s is \"/dev/ttys021\""), "{s}");
        let session = s.find("select s").expect(&s);
        let tab = s.find("select t").expect(&s);
        let window = s.find("select w").expect(&s);
        // Selecting a window reorders `windows`, so it goes last.
        assert!(session < tab && tab < window, "{s}");
        assert!(!s.contains("activate"), "{s}");
        assert!(s.ends_with("return \"not found\""), "{s}");
    }

    #[test]
    fn focus_checks_iterm_is_running_first() {
        let s = applescript_focus("/dev/ttys021");
        assert!(s.find("is running").expect(&s) < s.find("tell application id").expect(&s), "{s}");
    }

    #[test]
    fn typing_never_activates() {
        assert!(!applescript_type("/dev/ttys021", "hi", true).contains("activate"));
        assert!(!applescript_type("/dev/ttys021", "hi", false).contains("activate"));
    }

    #[test]
    fn a_line_gets_its_own_enter_and_keys_do_not() {
        assert!(!applescript_type("/dev/ttys021", "/compact", true).contains("newline false"));
        assert!(applescript_type("/dev/ttys021", "/compact", false).contains("newline false"));
    }

    #[test]
    fn keys_with_control_characters_become_character_ids() {
        assert_eq!(
            applescript_keys("a\"b\\c\x1b\r\x1b[13u"),
            r#""a\"b\\c" & (character id 27) & (character id 13) & (character id 27) & "[13u""#
        );
    }

    #[test]
    fn a_reply_is_one_write_of_its_keys() {
        let keys = reply_keys(&["one".to_string(), "two".to_string()]);
        let s = applescript_type("/dev/ttys021", &keys, false);
        assert_eq!(s.matches("write text").count(), 1, "{s}");
        assert!(s.contains(r#""one" & (character id 27) & (character id 13) & "two" & (character id 27) & "[13u""#), "{s}");
    }
}
