//! Claude Code runs this for every hooked event with the payload on stdin.
//! It appends one line to `~/.claude/maya/events.jsonl`, keeps the
//! session's inbox token where Maya's replies find it, and, whatever
//! happens, prints nothing and exits 0 so it never blocks a session.
use maya_core::hook_install::{append_record, event_name, record_token, TOKEN_ONLY_EVENTS};
use std::io::Read;

fn main() {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let maya_dir = home.join(".claude").join("maya");
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
