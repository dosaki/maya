//! Opening Terminal.app windows: the AppleScript comes from the core, the
//! activation afterwards needs AppKit, so these live in the app.

use maya_core::launch::{applescript_run, session_command, LaunchOptions};
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

pub fn open_terminal(target: &Path, prompt_file: &Path, opts: &LaunchOptions) -> Result<(), String> {
    open_terminal_with(&session_command(target, prompt_file, opts))
}
