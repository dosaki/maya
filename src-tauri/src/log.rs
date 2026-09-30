//! Maya's own log: what she heard, decided, said and ran, plus errors from
//! the pollers. Kept in memory for the Debug tab (the last `CAPACITY` lines)
//! and written to a file that starts afresh on every launch.

use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Lines kept for the Debug tab.
pub const CAPACITY: usize = 500;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Line {
    /// Unix time in milliseconds.
    pub at: u64,
    pub source: String,
    pub text: String,
}

struct Log {
    lines: std::collections::VecDeque<Line>,
    file: Option<File>,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);
static EMIT: OnceLock<Box<dyn Fn(&Line) + Send + Sync>> = OnceLock::new();

/// Starts a fresh log file at `path` (truncating the last launch's); lines
/// before this call are kept in memory only.
pub fn init(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let file = File::create(path).map_err(|e| format!("could not create {}: {e}", path.display()))?;
    let mut guard = LOG.lock().unwrap();
    let log = guard.get_or_insert_with(|| Log { lines: Default::default(), file: None });
    log.file = Some(file);
    Ok(())
}

/// Installs the function that pushes each new line to the page. Once.
pub fn install_emitter(f: impl Fn(&Line) + Send + Sync + 'static) {
    let _ = EMIT.set(Box::new(f));
}

/// Records one line under `source` ("ear", "wake", "interpreter", "action",
/// "speech", "listener", "app"). Never blocks on the page or the disk failing.
pub fn line(source: &str, text: impl Into<String>) {
    let l = Line { at: now_ms(), source: source.to_string(), text: text.into() };
    {
        let mut guard = LOG.lock().unwrap();
        let log = guard.get_or_insert_with(|| Log { lines: Default::default(), file: None });
        if log.lines.len() >= CAPACITY {
            log.lines.pop_front();
        }
        log.lines.push_back(l.clone());
        if let Some(f) = log.file.as_mut() {
            let _ = writeln!(f, "{}", file_line(&l));
        }
    }
    if let Some(emit) = EMIT.get() {
        emit(&l);
    }
}

/// The lines kept in memory, oldest first.
pub fn lines() -> Vec<Line> {
    LOG.lock().unwrap().as_ref().map(|l| l.lines.iter().cloned().collect()).unwrap_or_default()
}

/// Forgets the lines shown on the page; the file keeps them.
pub fn clear() {
    if let Some(l) = LOG.lock().unwrap().as_mut() {
        l.lines.clear();
    }
}

/// `HH:MM:SS.mmm source: text`, local time; later lines of a multi-line text
/// are indented so the file stays one record per line-start.
pub fn file_line(l: &Line) -> String {
    let text = l.text.replace('\n', "\n    ");
    format!("{} {}: {}", clock(l.at), l.source, text)
}

/// Local wall-clock `HH:MM:SS.mmm` for a unix millisecond timestamp.
pub fn clock(at_ms: u64) -> String {
    let secs = (at_ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:02}:{:02}:{:02}.{:03}", tm.tm_hour, tm.tm_min, tm.tm_sec, at_ms % 1000)
}

/// The first `max` characters of `s`, with a marker when it was cut.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}… [{} more chars]", s.chars().count() - max)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The log is one global: tests that touch it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn keeps_the_last_lines_in_order_and_writes_them_to_the_file() {
        let _s = SERIAL.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("maya.log");
        init(&path).unwrap();
        clear();
        line("ear", "heard: Maya what's waiting");
        line("interpreter", "reply: {\"say\":\"Nothing.\"}\nsecond line");
        // Other tests' threads may log meanwhile; look at this test's lines only.
        let kept: Vec<Line> = lines().into_iter().filter(|l| l.source == "ear" || l.source == "interpreter").collect();
        let tail = &kept[kept.len() - 2..];
        assert_eq!(tail[0].source, "ear");
        assert_eq!(tail[1].text, "reply: {\"say\":\"Nothing.\"}\nsecond line");
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains(" ear: heard: Maya what's waiting\n"), "{on_disk}");
        assert!(on_disk.contains("second line"), "{on_disk}");
        assert!(on_disk.lines().any(|l| l.starts_with("    second line")), "continuation lines are indented: {on_disk}");
    }

    #[test]
    fn a_fresh_init_truncates_the_previous_file() {
        let _s = SERIAL.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("maya.log");
        init(&path).unwrap();
        line("app", "first launch");
        init(&path).unwrap();
        line("app", "second launch");
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("first launch"), "{on_disk}");
        assert!(on_disk.contains("second launch"), "{on_disk}");
    }

    #[test]
    fn the_ring_drops_the_oldest_past_capacity() {
        let _s = SERIAL.lock().unwrap();
        clear();
        for i in 0..(CAPACITY + 3) {
            line("ring-test", format!("n{i}"));
        }
        let kept = lines();
        assert_eq!(kept.len(), CAPACITY);
        // Other tests' threads may log meanwhile, pushing out a few more of ours.
        let ours: Vec<&str> = kept.iter().filter(|l| l.source == "ring-test").map(|l| l.text.as_str()).collect();
        let last = format!("n{}", CAPACITY + 2);
        assert_eq!(ours.last().copied(), Some(last.as_str()));
        assert!(!ours.iter().any(|t| ["n0", "n1", "n2"].contains(t)), "the oldest are dropped");
        assert!(ours.len() > CAPACITY - 50, "only stray lines displace ours: {}", ours.len());
    }

    #[test]
    fn clips_long_text_and_says_how_much_was_cut() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdefghij", 4), "abcd… [6 more chars]");
    }

    #[test]
    fn formats_the_clock_with_milliseconds() {
        let s = clock(1_700_000_000_123);
        assert_eq!(s.len(), 12, "{s}");
        assert!(s.ends_with(".123"), "{s}");
    }
}
