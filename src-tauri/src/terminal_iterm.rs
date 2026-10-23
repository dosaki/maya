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
/// command ends. If iTerm2 is not running this starts it. A window iTerm2
/// restores at start is left alone, never reused: it can hold a live program,
/// such as a running agent, and typing the command into it would send the line
/// to that program. So a cold start can leave one window more than before.
pub fn applescript_open(cmd: &str) -> String {
    let cmd = applescript_keys(cmd);
    format!(
        r#"tell application id "{BUNDLE_ID}"
  set newWindow to (create window with default profile)
  tell current session of newWindow to write text {cmd}
end tell"#
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

/// What the run of a reply's script means. A script that failed may have typed
/// some of the reply, so that counts every line. But macOS refusing Maya access
/// to iTerm2 (-1743) and iTerm2 going away (-600) fail before anything is
/// typed, and counting those would stop Terminal.app from being asked, so a
/// reply to a Terminal.app session would fail too.
fn reply_outcome(ran: Result<String, String>, lines: usize, tty: &str) -> Result<(), (usize, String)> {
    match ran {
        Ok(out) if out == "ok" => Ok(()),
        Ok(_) => Err((0, format!("No iTerm2 session found for {tty}."))),
        Err(why) if why.contains("(-1743)") || why.contains("(-600)") => Err((0, why)),
        Err(why) => Err((lines, why)),
    }
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
    /// The keys as given and no Enter: one raw write. Unlike Terminal.app, whose
    /// `do script` cannot send a bare Esc, this needs no Accessibility permission.
    fn press_keys(&self, tty: &str, keys: &str) -> Result<(), String> {
        match run(applescript_type(tty, keys, false))?.as_str() {
            "ok" => Ok(()),
            _ => Err(format!("No iTerm2 session found for {tty}.")),
        }
    }
    /// One write for the whole reply. A script that ran but failed may have
    /// typed some of it, so only a missing session counts as nothing typed.
    fn type_reply(&self, tty: &str, lines: &[String]) -> Result<(), (usize, String)> {
        reply_outcome(run(applescript_type(tty, &reply_keys(lines), false)), lines.len(), tty)
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
    fn open_always_makes_its_own_window_and_never_types_into_one_that_exists() {
        let s = applescript_open("claude");
        assert_eq!(s.matches("create window with default profile").count(), 1, "{s}");
        assert!(s.contains("write text \"claude\""), "{s}");
        // A window iTerm2 restores at start can hold a live program, such as a
        // running agent; typing the command into it would send it as a message.
        assert!(!s.contains("current window"), "{s}");
        assert!(!s.contains("window 1"), "{s}");
        assert!(!s.contains("activate"), "{s}");
    }

    #[test]
    fn a_reply_that_was_sent_is_ok_and_a_missing_session_means_nothing_typed() {
        assert_eq!(reply_outcome(Ok("ok".into()), 2, "/dev/ttys021"), Ok(()));
        assert_eq!(reply_outcome(Ok("not found".into()), 2, "/dev/ttys021"), Err((0, "No iTerm2 session found for /dev/ttys021.".to_string())));
    }

    #[test]
    fn a_refused_automation_permission_means_nothing_was_typed() {
        let refused = Err("osascript failed: execution error: Not authorized to send Apple events to iTerm. (-1743)".to_string());
        assert!(matches!(reply_outcome(refused, 3, "/dev/ttys021"), Err((0, _))));
    }

    #[test]
    fn an_app_that_went_away_means_nothing_was_typed() {
        let gone = Err("osascript failed: iTerm got an error: Application isn't running. (-600)".to_string());
        assert!(matches!(reply_outcome(gone, 3, "/dev/ttys021"), Err((0, _))));
    }

    #[test]
    fn any_other_script_failure_may_have_typed_some_of_the_reply() {
        let failed = Err("osascript failed: something broke (-2741)".to_string());
        assert!(matches!(reply_outcome(failed, 3, "/dev/ttys021"), Err((3, _))));
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
    fn escape_is_written_raw_with_no_enter() {
        // `press_keys` is `applescript_type` with `newline` off: the bare key and nothing after it.
        let s = applescript_type("/dev/ttys021", maya_core::answer::ESC, false);
        assert!(s.contains("write text (character id 27) newline false"), "{s}");
        assert!(!s.contains("activate"), "{s}");
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
