//! Claude Code runs this for every hooked event with the payload on stdin.
//! It appends one line to `~/.claude/maya/events.jsonl`, keeps the
//! session's inbox token where Maya's replies find it, and, whatever
//! happens, prints nothing and exits 0 so it never blocks a session.
//!
//! As `maya-hook statusline` it is Claude Code's status line on Windows:
//! it runs the user's own status line command, records the payload's usage
//! and limits for Maya, and prints that command's output unchanged.
use maya_core::hook_install::{append_record, event_name, record_token, TOKEN_ONLY_EVENTS};
use maya_core::statusline::{recorded_from, write_recorded, INNER_FILE};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

/// How long the user's own status line command may take before its
/// output is dropped.
const INNER_CAP: Duration = Duration::from_secs(5);

fn main() {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let maya_dir = home.join(".claude").join("maya");
    if std::env::args().nth(1).as_deref() == Some("statusline") {
        statusline(&maya_dir, &payload);
        return;
    }
    if std::env::args().any(|a| a == "--codex") {
        let now = maya_core::now_ms();
        let _ = maya_core::hook_install::append_record_named(&maya_dir, "codex-events.jsonl", &payload, now);
        return;
    }
    let event = event_name(&payload).unwrap_or_default();
    let socket = std::env::var("CLAUDE_CODE_MESSAGING_SOCKET").ok();
    let token = std::env::var("CLAUDE_CODE_MESSAGING_TOKEN").ok();
    let _ = record_token(&maya_dir, &event, socket.as_deref(), token.as_deref());
    if TOKEN_ONLY_EVENTS.contains(&event.as_str()) {
        return;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let _ = append_record(&maya_dir, &payload, now);
}

/// Runs the inner command, records the payload, prints what the command
/// printed. Nothing here may fail the status line.
fn statusline(maya_dir: &Path, payload: &str) {
    let inner = std::fs::read_to_string(maya_dir.join(INNER_FILE)).unwrap_or_default();
    let out = if inner.trim().is_empty() { Vec::new() } else { run_inner(&inner, payload) };
    if let Ok(v) = serde_json::from_str(payload) {
        let line = String::from_utf8_lossy(&out);
        if let Some(r) = recorded_from(&v, line.trim_end_matches('\n'), maya_core::now_ms()) {
            let _ = write_recorded(maya_dir, &r);
        }
    }
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&out);
    let _ = stdout.flush();
}

/// `sh -c <inner>` with the payload on stdin; its stdout, or nothing when
/// it cannot start or outlasts `INNER_CAP`.
fn run_inner(inner: &str, payload: &str) -> Vec<u8> {
    use std::process::Stdio;
    let Ok(mut child) = maya_core::command("sh").arg("-c").arg(inner).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn() else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let payload = payload.to_owned();
        std::thread::spawn(move || {
            let _ = stdin.write_all(payload.as_bytes());
        });
    }
    let Some(mut stdout) = child.stdout.take() else { return Vec::new() };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    match rx.recv_timeout(INNER_CAP) {
        // Not waited for: a command that closed its stdout but runs on
        // must not hold the status line up; the hook exits right after.
        Ok(out) => out,
        Err(_) => {
            let _ = child.kill();
            Vec::new()
        }
    }
}
