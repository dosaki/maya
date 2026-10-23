//! Typing into iTerm2 sessions. The AppleScript comes from builders, so it can
//! be tested as text, and a thin runner hands it to `osascript`. This is the
//! iTerm2 counterpart of `terminal_app.rs`.

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
    fn open(&self, _command: &str, _cwd: &Path, _label: &str) -> Result<Option<String>, String> {
        Err("not yet".into())
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
    fn focus(&self, _tty: &str) -> Result<(), String> {
        Err("not yet".into())
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
