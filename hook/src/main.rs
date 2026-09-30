//! Claude Code runs this for every hooked event with the payload on stdin.
//! It appends one line to `~/.claude/maya/events.jsonl` and, whatever
//! happens, prints nothing and exits 0 so it never blocks a session.
use std::io::Read;

fn main() {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let _ = maya_core::hook_install::append_record(&home.join(".claude").join("maya"), &payload, now);
}
