//! The usage collectors: what `Store::usage_due` hands out, run with the
//! store's lock released, the answers filed back on the store.

use std::io::Read;

/// Runs `program args` with stdin closed, returns stdout on exit 0 within
/// `timeout`; kills it and returns None otherwise.
pub fn run_with_timeout(program: &str, args: &[&str], timeout: std::time::Duration) -> Option<String> {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + timeout;
    let mut child = crate::command(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread of its own: a full pipe would otherwise stall the child,
    // and a grandchild holding the pipe open must not hold up the answer.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = tx.send(stdout.read_to_string(&mut out).ok().map(|_| out));
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => break None,
        }
    };
    if !status.is_some_and(|s| s.success()) {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok().flatten()
}

/// How often the collectors' loop asks the store what is due.
pub const COLLECT_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
const GROK_USAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Takes what is due from `store`, runs each collector with the lock
/// released, and files the answers. True when anything ran, so the board
/// repaints.
pub fn collect(store: &std::sync::Mutex<crate::store::Store>) -> bool {
    let due = store.lock().unwrap().usage_due(crate::now_ms());
    let mut learned = false;
    if !due.grok.is_empty() {
        let grok = crate::launch::find_binary(crate::model::Harness::Grok).ok();
        for id in &due.grok {
            let usage = grok.as_ref().and_then(|b| run_with_timeout(&b.to_string_lossy(), &["usage", id], GROK_USAGE_TIMEOUT)).and_then(|json| crate::grok::parse_usage(&json));
            store.lock().unwrap().set_grok_usage(id, usage, crate::now_ms());
        }
        learned = true;
    }
    learned
}

/// What the collectors should run now.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Due {
    pub antigravity: bool,
    pub kiro: bool,
    /// Grok session ids whose events file changed since their last run.
    pub grok: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn run_with_timeout_returns_output_and_kills_a_hanger() {
        use std::time::{Duration, Instant};
        assert_eq!(run_with_timeout("sh", &["-c", "echo hi"], Duration::from_secs(5)).as_deref(), Some("hi\n"));
        assert_eq!(run_with_timeout("sh", &["-c", "echo no; exit 3"], Duration::from_secs(5)), None, "a failed exit is no answer");
        let start = Instant::now();
        assert_eq!(run_with_timeout("sh", &["-c", "sleep 5"], Duration::from_millis(200)), None);
        assert!(start.elapsed() < Duration::from_secs(1), "took {:?}", start.elapsed());
    }
}
