//! The usage collectors: what `Store::usage_due` hands out, run with the
//! store's lock released, the answers filed back on the store.

use crate::model::Harness;
use crate::store::Store;
use crate::usage::AccountLimits;
use std::io::Read;
use std::sync::Mutex;

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
const AGY_USAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Takes what is due from `store`, runs each collector with the lock
/// released, and files the answers. True when anything ran, so the board
/// repaints.
pub fn collect(store: &Mutex<Store>) -> bool {
    let due = store.lock().unwrap().usage_due(crate::now_ms());
    let mut learned = false;
    if !due.grok.is_empty() {
        let grok = crate::launch::find_binary(Harness::Grok).ok();
        for id in &due.grok {
            let usage = grok.as_ref().and_then(|b| run_with_timeout(&b.to_string_lossy(), &["usage", id], GROK_USAGE_TIMEOUT)).and_then(|json| crate::grok::parse_usage(&json));
            store.lock().unwrap().set_grok_usage(id, usage, crate::now_ms());
        }
        learned = true;
    }
    if due.antigravity {
        file_limits(store, Harness::Antigravity, antigravity_limits());
        learned = true;
    }
    learned
}

/// `agy -p /usage --output-format json`, parsed.
fn antigravity_limits() -> Result<AccountLimits, String> {
    let agy = crate::launch::find_binary(Harness::Antigravity)?;
    let json = run_with_timeout(&agy.to_string_lossy(), &["-p", "/usage", "--output-format", "json"], AGY_USAGE_TIMEOUT).ok_or("agy -p /usage failed or timed out")?;
    crate::antigravity::parse_usage(&json, crate::now_ms()).ok_or_else(|| "agy -p /usage gave no limits".to_string())
}

/// Files a limits reading on the store. A failure keeps the last good one
/// and is logged at most once an hour per harness.
fn file_limits(store: &Mutex<Store>, h: Harness, read: Result<AccountLimits, String>) {
    let now = crate::now_ms();
    let failed = match read {
        Ok(limits) => {
            store.lock().unwrap().set_limits(h, Some(limits), now);
            None
        }
        Err(why) => store.lock().unwrap().limits_failed(h, now).then_some(why),
    };
    if let Some(why) = failed {
        crate::log::line("usage", format!("{h:?} limits not read: {why}"));
    }
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

    #[test]
    fn a_failed_read_keeps_the_last_limits_and_logs_once() {
        use crate::usage::LimitWindow;
        let t = tempfile::tempdir().unwrap();
        let session = crate::foreign::ForeignSession { harness: Harness::Antigravity, pid: 7, tty: Some("ttys007".into()), session_id: "a1".into(), cwd: "/x".into(), name: "a".into(), transcript_path: t.path().join("a1.jsonl") };
        let store = Mutex::new(Store::new(t.path().join("claude")).with_alive(|_| true).with_foreign(t.path().join("codex"), t.path().join("agy"), vec![session]));
        let limits = AccountLimits { account: None, windows: vec![LimitWindow { label: "Gemini 7d".into(), percent: 12, resets_at: None }], read_at: 1 };
        file_limits(&store, Harness::Antigravity, Ok(limits.clone()));
        let why = format!("probe {} timed out", t.path().display());
        file_limits(&store, Harness::Antigravity, Err(why.clone()));
        file_limits(&store, Harness::Antigravity, Err(why.clone()));
        assert_eq!(store.lock().unwrap().refresh(crate::now_ms())[0].limits, Some(limits));
        let logged = crate::log::lines().iter().filter(|l| l.source == "usage" && l.text.contains(&why)).count();
        assert_eq!(logged, 1);
    }
}
