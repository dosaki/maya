use std::process::Command;
use std::sync::OnceLock;

pub use maya_core::tty::{tty_for_pid, tty_from_ps};

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Remembers the app handle so activation can run on Maya's main thread.
pub fn install_app_handle(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

/// Activates Terminal from inside Maya's own process, on the main thread:
/// Maya is the active app when the user clicks, so it may yield activation
/// to Terminal (macOS 14 cooperative activation). Requests from a helper
/// process such as `osascript` can be silently deferred. Returns false when
/// Terminal is not running or the request was not accepted.
pub fn activate_terminal_in_process() -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationOptions, NSRunningApplication};
    use objc2_foundation::ns_string;
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(ns_string!("com.apple.Terminal"));
    let Some(terminal) = apps.iter().next() else { return false };
    NSApplication::sharedApplication(mtm).yieldActivationToApplication(&terminal);
    terminal.activateWithOptions(NSApplicationActivationOptions::empty())
}

/// Runs the in-process activation on the main thread and waits for its answer.
fn activate_from_main_thread() -> Option<bool> {
    let app = APP.get()?;
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(activate_terminal_in_process());
    })
    .ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(2)).ok()
}

/// Selects the tab on `tty` and puts its window first in Terminal's stack,
/// without activating: AppleScript's `activate` raises every Terminal
/// window and often does nothing when the caller is not frontmost.
pub fn applescript_for(tty: &str) -> String {
    format!(
        r#"tell application "Terminal"
  repeat with w in windows
    repeat with t in tabs of w
      if tty of t is "{tty}" then
        set selected tab of w to t
        set index of w to 1
        return "ok"
      end if
    end repeat
  end repeat
end tell
return "not found""#
    )
}

/// JavaScript for Automation that brings Terminal forward the way a click on
/// one of its windows does: only the window at the top of its stack comes up,
/// the rest stay where they are (no `NSApplicationActivateAllWindows`).
pub fn activate_script() -> String {
    "ObjC.import('AppKit');\n\
     const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier('com.apple.Terminal');\n\
     if (apps.count > 0) apps.objectAtIndex(0).activateWithOptions($.NSApplicationActivateIgnoringOtherApps);"
        .to_string()
}

/// Brings Terminal forward with only its front window: from Maya's own
/// process when possible, else through a JavaScript for Automation helper.
pub fn activate_terminal() -> Result<(), String> {
    if activate_from_main_thread() == Some(true) {
        return Ok(());
    }
    activate_terminal_via_osascript()
}

pub fn activate_terminal_via_osascript() -> Result<(), String> {
    let out = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &activate_script()])
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("could not activate Terminal: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Brings the Terminal tab hosting `tty` to the front.
pub fn focus_tty(tty: &str) -> Result<(), String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(applescript_for(tty))
        .output()
        .map_err(|e| format!("could not run osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if result == "ok" {
        activate_terminal()
    } else {
        Err(format!("no Terminal tab found for {tty}"))
    }
}

/// Brings the Terminal tab hosting `pid` to the front.
pub fn focus_pid(pid: i32) -> Result<(), String> {
    focus_tty(&tty_for_pid(pid)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_targets_the_tty_without_activating() {
        let s = applescript_for("/dev/ttys021");
        assert!(s.contains("tell application \"Terminal\""));
        assert!(s.contains("if tty of t is \"/dev/ttys021\""));
        assert!(s.contains("set selected tab of w to t"));
        assert!(s.contains("set index of w to 1"));
        // AppleScript's `activate` raises every Terminal window and is flaky
        // from a background caller; activation is done separately via AppKit.
        assert!(!s.contains("activate"));
        assert!(s.contains("return \"not found\""));
    }

    #[test]
    fn activation_uses_appkit_without_the_all_windows_option() {
        let s = activate_script();
        assert!(s.contains("com.apple.Terminal"));
        assert!(s.contains("NSApplicationActivateIgnoringOtherApps"));
        assert!(!s.contains("NSApplicationActivateAllWindows"));
    }

    #[test]
    fn focusing_a_dead_pid_is_an_error_not_a_panic() {
        let err = focus_pid(2_000_000_000).unwrap_err();
        assert!(err.to_lowercase().contains("tty") || err.to_lowercase().contains("process"), "{err}");
    }
}
