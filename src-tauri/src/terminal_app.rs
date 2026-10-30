//! Opening Terminal.app windows: the AppleScript comes from the core, the
//! activation afterwards needs AppKit, so these live in the app.

use crate::terminal_iterm::ItermApp;
use maya_core::config::NewSessionTerminal;
use maya_core::launch::applescript_run;
use maya_core::terminal::Terminal;
use std::path::Path;
use std::process::Command;

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
/// (key code 53) through System Events once Terminal is frontmost (it answers
/// "not front" and presses nothing if it never is): `do script` always ends with a
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
        tell application "System Events"
          repeat 20 times
            if frontmost of application process "Terminal" then exit repeat
            delay 0.05
          end repeat
          if not (frontmost of application process "Terminal") then return "not front"
          key code 53
        end tell
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#
    )
}

/// What osascript says when Maya may not send keys (-1719, -25211, 1002).
fn needs_accessibility(stderr: &str) -> bool {
    ["-1719", "-25211", "not allowed assistive access", "not allowed to send keystrokes", "(1002)"].iter().any(|m| stderr.contains(m))
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

/// The app's one terminal, borrowed by every local action. A session may live
/// in iTerm2 or in Terminal.app, and its tty says which, so typing and focus
/// ask each app in turn: iTerm2 first, because it answers only when it is
/// already running, while a script for Terminal starts it. New windows open
/// where Settings says.
pub struct MacTerminal {
    iterm: &'static dyn Terminal,
    terminal: &'static dyn Terminal,
    /// Where new windows open, read on each use so a change in Settings counts at once.
    choice: fn() -> NewSessionTerminal,
}

pub static TERMINAL: MacTerminal = MacTerminal { iterm: &ItermApp, terminal: &TerminalApp, choice: crate::native::route::configured_choice };

impl MacTerminal {
    /// The apps to ask about a tty, in order.
    fn apps(&self) -> [&dyn Terminal; 2] {
        [self.iterm, self.terminal]
    }
}

/// Asks each app in turn and returns the first success. An error that says
/// keys already went in (a typed count above 0) ends the search, so a reply is
/// never typed twice; any other error moves on to the next app. When no app has
/// the tty, the last error comes back.
fn first_hit<T>(apps: &[&dyn Terminal], ask: impl Fn(&dyn Terminal) -> Result<T, (usize, String)>) -> Result<T, (usize, String)> {
    let mut last = (0, "No terminal window found.".to_string());
    for app in apps {
        match ask(*app) {
            Ok(done) => return Ok(done),
            Err((typed, why)) if typed > 0 => return Err((typed, why)),
            Err(err) => last = err,
        }
    }
    Err(last)
}

impl Terminal for MacTerminal {
    fn open(&self, command: &str, cwd: &Path, label: &str) -> Result<Option<String>, String> {
        let app = match (self.choice)() {
            NewSessionTerminal::Iterm => self.iterm,
            // Native is taken before the call reaches here; this arm only
            // keeps the match exhaustive.
            NewSessionTerminal::Terminal | NewSessionTerminal::Native => self.terminal,
        };
        app.open(command, cwd, label)
    }
    /// A line is one command: it either went in or it did not, so any error
    /// means "not here" and the next app is tried.
    fn type_line(&self, tty: &str, text: &str) -> Result<(), String> {
        first_hit(&self.apps(),|app| app.type_line(tty, text).map_err(|why| (0, why))).map_err(|(_, why)| why)
    }
    /// Like a line: one key press either happened or did not, so any error
    /// means "not here" and the next app is tried.
    fn press_keys(&self, tty: &str, keys: &str) -> Result<(), String> {
        first_hit(&self.apps(), |app| app.press_keys(tty, keys).map_err(|why| (0, why))).map_err(|(_, why)| why)
    }
    fn type_reply(&self, tty: &str, lines: &[String]) -> Result<(), (usize, String)> {
        first_hit(&self.apps(),|app| app.type_reply(tty, lines))
    }
    fn focus(&self, tty: &str) -> Result<(), String> {
        first_hit(&self.apps(),|app| app.focus(tty).map_err(|why| (0, why))).map_err(|(_, why)| why)
    }
    fn name_for_tty(&self, _tty: &str) -> Option<String> {
        None
    }
}

/// Brings the terminal tab hosting `pid` to the front, in whichever app has it.
pub fn focus_pid(pid: i32) -> Result<(), String> {
    TERMINAL.focus(&maya_core::tty::tty_for_pid(pid)?)
}

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
        match String::from_utf8_lossy(&out.stdout).trim() {
            "ok" => Ok(()),
            "not front" => Err("Terminal did not come to the front, so Esc was not pressed.".into()),
            _ => Err(format!("No Terminal tab found for {tty}.")),
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

    /// A stand-in app: answers every call with `result` and counts the calls.
    struct Stub {
        result: Result<(), (usize, String)>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl Stub {
        fn new(result: Result<(), (usize, String)>) -> &'static Stub {
            Box::leak(Box::new(Stub { result, calls: Default::default() }))
        }
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn answer(&self) -> Result<(), (usize, String)> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.result.clone()
        }
    }

    impl Terminal for Stub {
        fn open(&self, _command: &str, _cwd: &Path, _label: &str) -> Result<Option<String>, String> {
            self.answer().map(|()| None).map_err(|(_, why)| why)
        }
        fn type_line(&self, _tty: &str, _text: &str) -> Result<(), String> {
            self.answer().map_err(|(_, why)| why)
        }
        fn press_keys(&self, _tty: &str, _keys: &str) -> Result<(), String> {
            self.answer().map_err(|(_, why)| why)
        }
        fn type_reply(&self, _tty: &str, _lines: &[String]) -> Result<(), (usize, String)> {
            self.answer()
        }
        fn focus(&self, _tty: &str) -> Result<(), String> {
            self.answer().map_err(|(_, why)| why)
        }
        fn name_for_tty(&self, _tty: &str) -> Option<String> {
            None
        }
    }

    fn missing() -> &'static Stub {
        Stub::new(Err((0, "no such tab".into())))
    }

    /// Two stand-in apps, iTerm2 first and Terminal second, as `TERMINAL` has them.
    fn mac(apps: &[&'static Stub; 2]) -> MacTerminal {
        mac_choosing(apps, || NewSessionTerminal::Terminal)
    }

    fn mac_choosing(apps: &[&'static Stub; 2], choice: fn() -> NewSessionTerminal) -> MacTerminal {
        MacTerminal { iterm: apps[0], terminal: apps[1], choice }
    }

    #[test]
    fn new_sessions_open_in_the_chosen_app() {
        let (iterm, terminal) = (Stub::new(Ok(())), Stub::new(Ok(())));
        mac_choosing(&[iterm, terminal], || NewSessionTerminal::Iterm).open("claude", Path::new("/p"), "x").unwrap();
        assert_eq!((iterm.calls(), terminal.calls()), (1, 0));
        mac_choosing(&[iterm, terminal], || NewSessionTerminal::Terminal).open("claude", Path::new("/p"), "x").unwrap();
        assert_eq!((iterm.calls(), terminal.calls()), (1, 1));
    }

    #[test]
    fn typing_goes_to_the_app_that_has_the_tty() {
        let (first, second) = (missing(), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).type_reply("/dev/ttys001", &["hi".to_string()]), Ok(()));
        assert_eq!((first.calls(), second.calls()), (1, 1));
    }

    #[test]
    fn the_first_app_that_has_the_tty_is_the_only_one_asked() {
        let (first, second) = (Stub::new(Ok(())), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).type_reply("/dev/ttys001", &["hi".to_string()]), Ok(()));
        assert_eq!((first.calls(), second.calls()), (1, 0));
    }

    #[test]
    fn a_partly_typed_reply_is_not_tried_on_the_next_app() {
        let (first, second) = (Stub::new(Err((1, "osascript failed".into()))), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).type_reply("/dev/ttys001", &["a".to_string(), "b".to_string()]), Err((1, "osascript failed".to_string())));
        assert_eq!(second.calls(), 0);
    }

    #[test]
    fn not_found_in_both_apps_reports_nothing_typed() {
        let result = mac(&[missing(), missing()]).type_reply("/dev/ttys001", &["hi".to_string()]);
        assert!(matches!(result, Err((0, _))), "{result:?}");
    }

    #[test]
    fn a_line_is_typed_into_the_app_that_has_the_tty() {
        let (first, second) = (missing(), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).type_line("/dev/ttys001", "/compact"), Ok(()));
        assert_eq!((first.calls(), second.calls()), (1, 1));
    }

    #[test]
    fn focus_tries_the_next_app_when_a_tab_is_missing() {
        let (first, second) = (missing(), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).focus("/dev/ttys001"), Ok(()));
        assert_eq!((first.calls(), second.calls()), (1, 1));
    }

    #[test]
    fn focus_reports_the_last_reason_when_no_app_has_the_tty() {
        let last = Stub::new(Err((0, "no Terminal tab".into())));
        assert_eq!(mac(&[missing(), last]).focus("/dev/ttys001"), Err("no Terminal tab".to_string()));
    }

    #[test]
    fn escape_is_pressed_in_the_app_that_has_the_tty() {
        let (first, second) = (missing(), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).press_keys("/dev/ttys001", maya_core::answer::ESC), Ok(()));
        assert_eq!((first.calls(), second.calls()), (1, 1));
    }

    #[test]
    fn escape_goes_only_to_the_first_app_that_has_the_tty() {
        let (first, second) = (Stub::new(Ok(())), Stub::new(Ok(())));
        assert_eq!(mac(&[first, second]).press_keys("/dev/ttys001", maya_core::answer::ESC), Ok(()));
        assert_eq!((first.calls(), second.calls()), (1, 0));
    }

    #[test]
    fn escape_reports_the_last_reason_when_no_app_has_the_tty() {
        let last = Stub::new(Err((0, "No Terminal tab found".into())));
        assert_eq!(mac(&[missing(), last]).press_keys("/dev/ttys001", maya_core::answer::ESC), Err("No Terminal tab found".to_string()));
    }

    #[test]
    fn focusing_a_dead_process_is_an_error_not_a_panic() {
        let err = focus_pid(2_000_000_000).unwrap_err();
        assert!(err.to_lowercase().contains("tty") || err.to_lowercase().contains("process"), "{err}");
    }

    #[test]
    fn escape_script_targets_the_tab_and_presses_key_code_53() {
        let s = applescript_press_escape("/dev/ttys004");
        assert!(s.contains("tty of t is \"/dev/ttys004\""), "{s}");
        assert!(s.contains("key code 53"), "{s}");
        assert!(!s.contains("do script"), "{s}");
        assert!(s.find("frontmost of application process").unwrap() < s.find("key code 53").unwrap(), "{s}");
    }

    #[test]
    fn missing_permission_is_recognised_in_osascripts_messages() {
        for e in [
            "execution error: osascript is not allowed to send keystrokes. (1002)",
            "System Events got an error: osascript is not allowed assistive access. (-25211)",
            "execution error: Not authorized (-1719)",
        ] {
            assert!(needs_accessibility(e), "{e}");
        }
        assert!(!needs_accessibility("execution error: Can't get window 1. (-1728)"));
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
