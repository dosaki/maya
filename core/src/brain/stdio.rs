//! A brain process spoken to in lines on stdin and stdout, whatever the
//! protocol: the process, a reader that stamps the output clock and feeds
//! the protocol, a ticker for what time alone ends, and the last bit of its
//! stderr for the reason it ended.
//!
//! The protocol, her stdin and the events share one lock, so the lines
//! written and the events sent keep their order. A protocol that panics
//! ends her session with `Exited` rather than poisoning that lock.

use std::io::{BufRead, BufReader, Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::driver::{Driver, Event, Launch};

/// How much of her stderr is kept for the reason her process ended.
pub const STDERR_TAIL_BYTES: usize = 2_048;
/// How long the end of her stdout waits for the end of her stderr.
const STDERR_WAIT: Duration = Duration::from_millis(500);

/// A protocol with no process in it: lines in, events and lines to write
/// out (one string may hold several lines, each newline-terminated).
pub trait Protocol: Send + 'static {
    /// Her agent's name, for the log and the reason her process ended.
    fn label(&self) -> &'static str;
    /// What to write as soon as the process runs.
    fn start(&mut self) -> Option<String> {
        None
    }
    fn line(&mut self, line: &str) -> (Vec<Event>, Option<String>);
    fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>);
    /// Stops her running turn (and whatever else the agent can stop).
    fn cancel(&mut self, now_ms: u64) -> (Vec<Event>, Option<String>);
    /// What time alone ends; `last_output_ms` is the output clock.
    fn tick(&mut self, _now_ms: u64, _last_output_ms: u64) -> Vec<Event> {
        vec![]
    }
    /// The line asking her agent to compact; `Err("unsupported")` where
    /// it compacts itself.
    fn compact(&mut self) -> Result<(Vec<Event>, String), String> {
        Err("unsupported".into())
    }
    fn background_subagents(&self) -> bool;
    fn assumed_session_id(&self) -> Option<String> {
        None
    }
}

/// The last `STDERR_TAIL_BYTES` of her stderr.
#[derive(Clone, Default)]
struct Tail(Arc<Mutex<Vec<u8>>>);

impl Tail {
    fn push(&self, bytes: &[u8]) {
        let mut t = self.0.lock().unwrap();
        t.extend_from_slice(bytes);
        let over = t.len().saturating_sub(STDERR_TAIL_BYTES);
        t.drain(..over);
    }

    /// `why`, with what she last wrote to stderr, on one line.
    fn reason(&self, why: String) -> String {
        let t = self.0.lock().unwrap();
        let text = String::from_utf8_lossy(&t);
        let said: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if said.is_empty() {
            why
        } else {
            format!("{why} (stderr: {})", said.join(" | "))
        }
    }
}

struct Shared<P> {
    protocol: P,
    stdin: Option<ChildStdin>,
    events: Sender<Event>,
    stderr: Tail,
    /// `Exited` went out: nothing follows it.
    done: bool,
}

impl<P: Protocol> Shared<P> {
    fn write(&mut self, line: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("the brain is shut down")?;
        stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).map_err(|e| format!("cannot write to the brain: {e}"))
    }

    /// Writes `line`, if any, logging a failure.
    fn write_logged(&mut self, line: Option<String>) {
        if let Some(line) = line {
            if let Err(e) = self.write(&line) {
                crate::log::line("brain", format!("{}: {e}", self.protocol.label()));
            }
        }
    }

    /// Runs `f` on the protocol. A panic in it ends her session with
    /// `Exited` (the protocol's state is not trusted again) and gives None;
    /// it is caught under the lock, so the lock is not poisoned.
    fn guarded<T>(&mut self, what: &str, f: impl FnOnce(&mut P) -> T) -> Option<T> {
        if self.done {
            return None;
        }
        match catch_unwind(AssertUnwindSafe(|| f(&mut self.protocol))) {
            Ok(t) => Some(t),
            Err(_) => {
                let why = format!("Maya could not handle {what} from {}", self.protocol.label());
                crate::log::line("brain", &why);
                self.emit(vec![Event::Exited(why)]);
                None
            }
        }
    }

    /// Sends `events` in order; an `Exited` among them ends her session:
    /// her stdin closes, the reason gets her stderr's tail, and nothing
    /// more is sent.
    fn emit(&mut self, events: Vec<Event>) {
        for e in events {
            if self.done {
                return;
            }
            let e = match e {
                Event::Exited(why) => {
                    self.done = true;
                    self.stdin = None;
                    Event::Exited(self.stderr.reason(why))
                }
                e => e,
            };
            let _ = self.events.send(e);
        }
    }
}

/// A brain process driven by `P`.
pub struct LineDriver<P: Protocol> {
    child: Child,
    shared: Arc<Mutex<Shared<P>>>,
    background: bool,
    assumed: Option<String>,
    /// When she last wrote a line: see `Driver::output_clock`.
    last_output: Arc<AtomicU64>,
}

/// Starts `l.binary` with `args` (and `env` on top of the clean
/// environment) in `l.cwd`, spoken to by `protocol`. Events arrive on
/// `events`; `Exited` is the last one.
pub fn spawn<P: Protocol>(l: &Launch, args: Vec<String>, env: &[(&str, &str)], mut protocol: P, events: Sender<Event>) -> Result<LineDriver<P>, String> {
    let label = protocol.label();
    std::fs::create_dir_all(&l.cwd).map_err(|e| format!("cannot create {}: {e}", l.cwd.display()))?;
    let mut cmd = crate::command(&l.binary);
    cmd.args(args)
        .current_dir(&l.cwd)
        .env_clear()
        .envs(crate::launch::clean_env(std::env::vars()))
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", l.binary.display()))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take();
    let tail = Tail::default();
    let (stderr_done, stderr_ended) = std::sync::mpsc::channel::<()>();
    if let Some(mut err) = stderr {
        let tail = tail.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.push(&buf[..n]);
            }
            let _ = stderr_done.send(());
        });
    }
    let first = protocol.start();
    let (background, assumed) = (protocol.background_subagents(), protocol.assumed_session_id());
    let shared = Arc::new(Mutex::new(Shared { protocol, stdin, events, stderr: tail, done: false }));
    shared.lock().unwrap().write_logged(first);
    let last_output = Arc::new(AtomicU64::new(0));
    let (stamp, reader_shared) = (last_output.clone(), shared.clone());
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // Any line shows she is alive, whether or not it is an event.
            stamp.store(crate::now_ms(), Ordering::SeqCst);
            let line = String::from_utf8_lossy(&buf).into_owned();
            let mut s = reader_shared.lock().unwrap();
            if s.done {
                return;
            }
            let Some((events, back)) = s.guarded("a line", |p| p.line(&line)) else { return };
            s.write_logged(back);
            s.emit(events);
        }
        // Her last words on stderr come with the reason.
        let _ = stderr_ended.recv_timeout(STDERR_WAIT);
        reader_shared.lock().unwrap().emit(vec![Event::Exited(format!("the {label} process ended"))]);
    });
    let (clock, ticker_shared) = (last_output.clone(), shared.clone());
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(200));
        let mut s = ticker_shared.lock().unwrap();
        if s.done {
            return;
        }
        let Some(events) = s.guarded("the time", |p| p.tick(crate::now_ms(), clock.load(Ordering::SeqCst))) else { return };
        s.emit(events);
    });
    Ok(LineDriver { child, shared, background, assumed, last_output })
}

impl<P: Protocol> LineDriver<P> {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl<P: Protocol> Driver for LineDriver<P> {
    fn send(&mut self, text: &str) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        if s.done {
            return Err("her session has ended".into());
        }
        let (events, line) = s.guarded("a message", |p| p.send(text)).ok_or("her session has ended")?;
        if let Some(line) = line {
            s.write(&line)?;
        }
        s.emit(events);
        Ok(())
    }

    fn interrupt(&mut self) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        let Some((events, line)) = s.guarded("a stop", |p| p.cancel(crate::now_ms())) else { return Ok(()) };
        let wrote = match line {
            Some(line) => s.write(&line),
            None => Ok(()),
        };
        s.emit(events);
        wrote
    }

    fn compact(&mut self) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        let (events, line) = s.guarded("a compaction", |p| p.compact()).ok_or("her session has ended")??;
        s.write(&line)?;
        s.emit(events);
        Ok(())
    }

    fn background_subagents(&self) -> bool {
        self.background
    }

    fn assumed_session_id(&self) -> Option<String> {
        self.assumed.clone()
    }

    fn output_clock(&self) -> Option<Arc<AtomicU64>> {
        Some(self.last_output.clone())
    }

    fn shutdown(&mut self) {
        self.shared.lock().unwrap().stdin = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Shutting down waits for the process (or kills and waits), so one that
/// exited on its own leaves no zombie behind once dropped.
impl<P: Protocol> Drop for LineDriver<P> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::model::Harness;

    #[test]
    fn the_stderr_tail_keeps_the_last_bytes_on_one_line() {
        let t = Tail::default();
        assert_eq!(t.reason("ended".into()), "ended", "nothing on stderr: the reason alone");
        t.push(&vec![b'x'; 3_000]);
        t.push(b"\nError: Not logged in\n\n  run codex login\n");
        let r = t.reason("the Codex process ended".into());
        assert!(r.starts_with("the Codex process ended (stderr: "), "{r}");
        assert!(r.ends_with("| Error: Not logged in | run codex login)"), "{r}");
        assert!(!r.contains('\n'));
        assert!(t.0.lock().unwrap().len() <= STDERR_TAIL_BYTES);
    }

    /// Echoes each message back as her reply, one line per turn.
    #[cfg(unix)]
    struct Echo;

    #[cfg(unix)]
    impl Protocol for Echo {
        fn label(&self) -> &'static str {
            "Echo"
        }
        fn start(&mut self) -> Option<String> {
            Some("hello\n".into())
        }
        fn line(&mut self, line: &str) -> (Vec<Event>, Option<String>) {
            assert_ne!(line.trim(), "boom", "a protocol bug");
            (vec![Event::TurnEnded { text: line.trim().into(), context_tokens: None }], None)
        }
        fn send(&mut self, text: &str) -> (Vec<Event>, Option<String>) {
            (vec![Event::TurnStarted], Some(format!("{text}\n")))
        }
        fn cancel(&mut self, _now_ms: u64) -> (Vec<Event>, Option<String>) {
            (vec![], None)
        }
        fn background_subagents(&self) -> bool {
            false
        }
    }

    #[cfg(unix)]
    fn fake(script: &str) -> (std::path::PathBuf, Launch) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("maya-fake-stdio-{}", crate::launch::new_session_uuid()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("agent");
        std::fs::write(&bin, format!("#!/bin/sh\n{script}")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let l = Launch { agent: Harness::Other, binary: bin, model: None, cwd: dir.join("work"), system_prompt: String::new(), resume: None, writable: vec![] };
        (dir, l)
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_fails_to_start_ends_with_its_stderr() {
        let (dir, l) = fake("read -r first\necho \"Error: Not logged in ($first)\" >&2\nexit 1\n");
        let (tx, rx) = std::sync::mpsc::channel();
        let d = spawn(&l, vec![], &[], Echo, tx).unwrap();
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Exited(why) => assert_eq!(why, "the Echo process ended (stderr: Error: Not logged in (hello))"),
            e => panic!("{e:?}"),
        }
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_protocol_that_panics_ends_her_session_without_poisoning_the_lock() {
        let (dir, l) = fake("read -r first\necho boom\ncat >/dev/null\n");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = spawn(&l, vec![], &[], Echo, tx).unwrap();
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Exited(why) => assert!(why.starts_with("Maya could not handle a line from Echo"), "{why}"),
            e => panic!("{e:?}"),
        }
        assert_eq!(d.send("hi"), Err("her session has ended".into()), "the lock still works");
        assert_eq!(d.interrupt(), Ok(()));
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err(), "Exited is the last event");
        drop(d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn lines_go_both_ways_and_the_environment_is_passed() {
        let (dir, l) = fake("read -r first\nread -r msg\necho \"$msg $MAYA_TEST_VAR\"\ncat >/dev/null\n");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut d = spawn(&l, vec![], &[("MAYA_TEST_VAR", "set")], Echo, tx).unwrap();
        assert!(!d.background_subagents());
        assert_eq!(d.compact(), Err("unsupported".into()));
        d.send("hi").unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnStarted);
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::TurnEnded { text: "hi set".into(), context_tokens: None });
        assert!(d.output_clock().unwrap().load(Ordering::SeqCst) > 0);
        d.shutdown();
        assert_eq!(rx.recv_timeout(wait).unwrap(), Event::Exited("the Echo process ended".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
