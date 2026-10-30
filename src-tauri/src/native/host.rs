//! The native host: pseudo-terminals Maya runs sessions in. Each terminal's
//! output is read on its own thread into a `Screen` and a pending batch,
//! and one thread sends every batch to the terminal's viewers.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

#[cfg(unix)]
use portable_pty::Child;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use super::screen::Screen;

pub type TermId = u64;

pub const DEFAULT_ROWS: u16 = 40;
pub const DEFAULT_COLS: u16 = 120;
/// The smallest size a terminal is resized to.
const MIN_ROWS: u16 = 5;
const MIN_COLS: u16 = 20;
/// How long output gathers before it goes to the viewers.
pub const BATCH: Duration = Duration::from_millis(16);
/// How long programs get to end on a hangup before they are killed.
#[cfg(unix)]
const GRACE: Duration = Duration::from_millis(400);

pub enum Frame {
    Output(Vec<u8>),
    /// The program ended; no frame follows.
    Exit,
}

/// A viewer's sink; returning false detaches it.
pub type Sink = Box<dyn Fn(Frame) -> bool + Send>;

/// A viewer just attached: the screen to show it first, at the terminal's size.
pub struct Attached {
    pub term: TermId,
    pub viewer: u64,
    pub snapshot: Vec<u8>,
    pub rows: u16,
    pub cols: u16,
}

/// The command line that runs `command` in a login shell, so the user's
/// `PATH` is set even when Maya was opened from the Dock. The shell exits
/// with the command.
pub fn shell_argv(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let bash = maya_core::launch::git_bash().map(|p| p.display().to_string()).unwrap_or_else(|| "bash.exe".into());
        vec![bash, "-l".into(), "-c".into(), command.into()]
    }
    #[cfg(not(windows))]
    {
        login_argv(std::env::var("SHELL").ok().as_deref(), command)
    }
}

/// `shell_argv` with `$SHELL` given: used when it is an absolute path that
/// exists, else zsh on macOS and bash elsewhere.
#[cfg(not(windows))]
fn login_argv(shell: Option<&str>, command: &str) -> Vec<String> {
    let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
    let shell = shell.filter(|s| Path::new(s).is_absolute() && Path::new(s).exists()).unwrap_or(fallback);
    vec![shell.into(), "-l".into(), "-c".into(), command.into()]
}

/// A lock that a panicking sink cannot poison for everyone else.
fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Viewer {
    id: u64,
    sink: Arc<Mutex<Sink>>,
    /// Bytes at the start of the pending batch that its snapshot already showed.
    seen: usize,
}

/// One batch going out: each viewer with the bytes it gets.
type Delivery = Vec<(u64, Arc<Mutex<Sink>>, Vec<u8>)>;

/// What a terminal printed: the screen and the batch not yet sent.
struct Out {
    screen: Screen,
    pending: Vec<u8>,
    viewers: Vec<Viewer>,
    ended: bool,
}

impl Out {
    fn feed(&mut self, bytes: &[u8]) {
        self.screen.feed(bytes);
        self.pending.extend_from_slice(bytes);
    }

    /// Adds a viewer and returns what it is shown first. The pending bytes
    /// are in the snapshot already, so it is sent only what comes after.
    fn attach(&mut self, id: u64, scrollback: usize, sink: Sink) -> (Vec<u8>, u16, u16) {
        let snapshot = self.screen.snapshot(scrollback);
        let (rows, cols) = self.screen.size();
        self.viewers.push(Viewer { id, sink: Arc::new(Mutex::new(sink)), seen: self.pending.len() });
        (snapshot, rows, cols)
    }

    /// Takes the pending batch, cut for each viewer.
    fn take(&mut self) -> Delivery {
        let bytes = std::mem::take(&mut self.pending);
        self.viewers
            .iter_mut()
            .map(|v| (v.id, v.sink.clone(), bytes[std::mem::take(&mut v.seen).min(bytes.len())..].to_vec()))
            .collect()
    }
}

/// Sends each viewer its bytes, and returns the viewers whose sink said no.
fn deliver(delivery: &Delivery) -> Vec<u64> {
    delivery.iter().filter(|(_, sink, bytes)| !bytes.is_empty() && !(lock(sink))(Frame::Output(bytes.clone()))).map(|(id, _, _)| *id).collect()
}

struct Term {
    /// The program's pid; on Unix also its process group.
    root: i32,
    #[cfg(unix)]
    tty: Option<String>,
    /// None once the pseudo console is closed (Windows).
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// The program until it is reaped: while it is here, `root` cannot
    /// belong to another process.
    #[cfg(unix)]
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    out: Mutex<Out>,
    /// Held while frames go out, so each viewer gets them in order. Taken
    /// before `out`, and never needed by a sink.
    sending: Mutex<()>,
}

impl Term {
    /// Sends the pending batch. Sinks run with only `sending` held.
    fn send(&self) {
        let _sending = lock(&self.sending);
        let delivery = {
            let mut out = lock(&self.out);
            if out.pending.is_empty() {
                return;
            }
            out.take()
        };
        let dead = deliver(&delivery);
        if !dead.is_empty() {
            lock(&self.out).viewers.retain(|v| !dead.contains(&v.id));
        }
    }
}

type Terms = Mutex<HashMap<TermId, Arc<Term>>>;

/// The pseudo-terminals Maya hosts.
pub struct NativeHost {
    terms: Arc<Terms>,
    next_term: AtomicU64,
    next_viewer: AtomicU64,
}

impl NativeHost {
    /// A host that sends output every `batch`.
    pub fn new(batch: Duration) -> NativeHost {
        let terms: Arc<Terms> = Arc::default();
        let weak = Arc::downgrade(&terms);
        let _ = std::thread::Builder::new().name("native-flush".into()).spawn(move || flush(weak, batch));
        NativeHost { terms, next_term: AtomicU64::new(1), next_viewer: AtomicU64::new(1) }
    }

    /// Starts `command` in a login shell in a new terminal, in `cwd`.
    pub fn spawn(&self, command: &str, cwd: &Path) -> Result<TermId, String> {
        #[cfg(windows)]
        maya_core::launch::git_bash().ok_or("Git for Windows is not installed; Maya needs its bash to start sessions (https://git-scm.com/download/win).")?;
        self.spawn_argv(&shell_argv(command), cwd)
    }

    fn spawn_argv(&self, argv: &[String], cwd: &Path) -> Result<TermId, String> {
        let size = PtySize { rows: DEFAULT_ROWS, cols: DEFAULT_COLS, pixel_width: 0, pixel_height: 0 };
        let pair = native_pty_system().openpty(size).map_err(|e| format!("Could not open a terminal: {e}"))?;
        let mut cmd = CommandBuilder::from_argv(argv.iter().map(Into::into).collect());
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        #[cfg(target_os = "linux")]
        if std::env::var_os("APPIMAGE").is_some() {
            for var in maya_core::launch::APPIMAGE_VARS {
                cmd.env_remove(var);
            }
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("Could not start the session: {e}"))?;
        drop(pair.slave);
        let Some(root) = child.process_id().and_then(|p| i32::try_from(p).ok()) else {
            let _ = child.kill();
            return Err("Could not start the session: it has no process id.".into());
        };
        let reader = pair.master.try_clone_reader().map_err(|e| format!("Could not read the terminal: {e}"))?;
        let writer = pair.master.take_writer().map_err(|e| format!("Could not write to the terminal: {e}"))?;
        let term = Arc::new(Term {
            root,
            #[cfg(unix)]
            tty: pair.master.tty_name().map(|p| p.display().to_string()),
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(writer),
            killer: Mutex::new(child.clone_killer()),
            #[cfg(unix)]
            child: Mutex::new(Some(child)),
            out: Mutex::new(Out { screen: Screen::new(DEFAULT_ROWS, DEFAULT_COLS), pending: Vec::new(), viewers: Vec::new(), ended: false }),
            sending: Mutex::new(()),
        });
        let id = self.next_term.fetch_add(1, Ordering::Relaxed);
        lock(&self.terms).insert(id, term.clone());
        let weak = Arc::downgrade(&self.terms);
        #[cfg(unix)]
        let started = std::thread::Builder::new().name(format!("native-{id}")).spawn(move || {
            read(id, term.clone(), reader, weak);
            // The tty closes as the program ends; a straggler is polled.
            while !reaped(&term) {
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        #[cfg(windows)]
        let started = {
            // The pseudo console outlives its program, and the reader sees
            // no end until it closes: close it once the program ends.
            let waited = term.clone();
            std::thread::spawn(move || {
                let _ = child.wait();
                lock(&waited.master).take();
            });
            std::thread::Builder::new().name(format!("native-{id}")).spawn(move || read(id, term, reader, weak))
        };
        if let Err(e) = started {
            self.kill(id);
            return Err(format!("Could not start the session: {e}"));
        }
        Ok(id)
    }

    fn get(&self, term: TermId) -> Result<Arc<Term>, String> {
        lock(&self.terms).get(&term).cloned().ok_or_else(|| "That session's terminal has closed.".into())
    }

    fn all(&self) -> Vec<(TermId, Arc<Term>)> {
        lock(&self.terms).iter().map(|(id, t)| (*id, t.clone())).collect()
    }

    /// Types `bytes` into the terminal.
    pub fn write(&self, term: TermId, bytes: &[u8]) -> Result<(), String> {
        let t = self.get(term)?;
        let mut w = lock(&t.writer);
        w.write_all(bytes).and_then(|()| w.flush()).map_err(|e| format!("Could not type into the terminal: {e}"))
    }

    /// Resizes the terminal, to at least 5 rows by 20 columns.
    pub fn resize(&self, term: TermId, rows: u16, cols: u16) -> Result<(), String> {
        let t = self.get(term)?;
        let (rows, cols) = (rows.max(MIN_ROWS), cols.max(MIN_COLS));
        if let Some(m) = lock(&t.master).as_ref() {
            m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| format!("Could not resize the terminal: {e}"))?;
        }
        lock(&t.out).screen.resize(rows, cols);
        Ok(())
    }

    /// Adds a viewer: `sink` gets every byte the terminal prints after the
    /// returned snapshot, which shows up to `scrollback` lines of history.
    pub fn attach(&self, term: TermId, scrollback: usize, sink: Sink) -> Result<Attached, String> {
        let t = self.get(term)?;
        let viewer = self.next_viewer.fetch_add(1, Ordering::Relaxed);
        let mut out = lock(&t.out);
        if out.ended {
            return Err("That session's terminal has closed.".into());
        }
        let (snapshot, rows, cols) = out.attach(viewer, scrollback, sink);
        Ok(Attached { term, viewer, snapshot, rows, cols })
    }

    pub fn detach(&self, term: TermId, viewer: u64) {
        if let Ok(t) = self.get(term) {
            lock(&t.out).viewers.retain(|v| v.id != viewer);
        }
    }

    /// The terminal whose device is `tty`, with or without `/dev/`. On
    /// Windows `tty` is a console key, and the terminal is the one its
    /// process descends from.
    pub fn term_for_tty(&self, tty: &str) -> Option<TermId> {
        #[cfg(unix)]
        {
            let bare = |t: &str| t.strip_prefix("/dev/").unwrap_or(t).to_string();
            let want = bare(tty);
            if want.is_empty() {
                return None;
            }
            self.all().into_iter().find(|(_, t)| t.tty.as_deref().map(bare).as_ref() == Some(&want)).map(|(id, _)| id)
        }
        #[cfg(windows)]
        {
            let pid = i32::try_from(maya_core::win_console::console_pid(tty)?).ok()?;
            let tree: Vec<maya_core::foreign::Proc> = maya_core::win_process::tree()
                .into_iter()
                .map(|(pid, ppid)| maya_core::foreign::Proc { pid: pid as i32, ppid: ppid as i32, tty: None, command: String::new() })
                .collect();
            let root = maya_core::native::root_of(pid, &self.root_pids(), &tree)?;
            self.term_for_root(root)
        }
    }

    pub fn term_for_root(&self, root_pid: i32) -> Option<TermId> {
        self.all().into_iter().find(|(_, t)| t.root == root_pid).map(|(id, _)| id)
    }

    /// The pid of every hosted terminal's program.
    pub fn root_pids(&self) -> Vec<i32> {
        self.all().into_iter().map(|(_, t)| t.root).collect()
    }

    /// Ends every terminal's program, for when Maya quits. On Unix each
    /// gets a hangup, and the ones still running after one shared grace
    /// period are killed.
    pub fn kill_all(&self) {
        let all: Vec<Arc<Term>> = self.all().into_iter().map(|(_, t)| t).collect();
        for t in &all {
            hang_up(t);
        }
        #[cfg(unix)]
        {
            let end = Instant::now() + GRACE;
            while Instant::now() < end && !all.iter().all(|t| reaped(t)) {
                std::thread::sleep(Duration::from_millis(10));
            }
            for t in &all {
                let child = lock(&t.child);
                if child.is_some() {
                    // SAFETY: a signal to a process group whose leader is
                    // not reaped (the lock keeps it so), so not a reused pid.
                    unsafe {
                        libc::kill(-t.root, libc::SIGKILL);
                    }
                }
            }
        }
    }

    fn kill(&self, term: TermId) {
        if let Ok(t) = self.get(term) {
            hang_up(&t);
        }
    }

    #[cfg(all(test, unix))]
    fn viewers(&self, term: TermId) -> Vec<u64> {
        self.get(term).map(|t| lock(&t.out).viewers.iter().map(|v| v.id).collect()).unwrap_or_default()
    }

    #[cfg(all(test, unix))]
    pub(crate) fn tty(&self, term: TermId) -> Option<String> {
        self.get(term).ok()?.tty.clone()
    }
}

/// Asks a terminal's program to end.
fn hang_up(t: &Term) {
    // portable-pty makes the program a session leader, so its pid is also
    // its process group: the hangup reaches what it started.
    #[cfg(unix)]
    // SAFETY: a signal to a process group; `root` is always positive.
    unsafe {
        libc::kill(-t.root, libc::SIGHUP);
    }
    let _ = lock(&t.killer).kill();
    // Closing the pseudo console ends the processes in it.
    #[cfg(windows)]
    lock(&t.master).take();
}

/// Reaps the terminal's program if it has ended; true once it is reaped.
#[cfg(unix)]
fn reaped(t: &Term) -> bool {
    let mut child = lock(&t.child);
    if let Some(c) = child.as_mut() {
        if !matches!(c.try_wait(), Ok(None)) {
            *child = None;
        }
    }
    child.is_none()
}

/// The host-wide sender: every `batch`, each terminal's pending output goes
/// to its viewers. Ends with the host.
fn flush(terms: Weak<Terms>, batch: Duration) {
    loop {
        std::thread::sleep(batch);
        let Some(terms) = terms.upgrade() else { return };
        let all: Vec<Arc<Term>> = lock(&terms).values().cloned().collect();
        drop(terms);
        for t in all {
            t.send();
        }
    }
}

/// A terminal's reader: feeds what the program prints into the screen and
/// the pending batch. At the end it sends the rest and `Exit`, and drops
/// the terminal.
fn read(id: TermId, term: Arc<Term>, mut reader: Box<dyn Read + Send>, terms: Weak<Terms>) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => lock(&term.out).feed(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _sending = lock(&term.sending);
    let delivery = {
        let mut out = lock(&term.out);
        out.ended = true;
        let delivery = out.take();
        out.viewers.clear();
        delivery
    };
    if let Some(terms) = terms.upgrade() {
        lock(&terms).remove(&id);
    }
    let dead = deliver(&delivery);
    for (id, sink, _) in &delivery {
        if !dead.contains(id) {
            (lock(sink))(Frame::Exit);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::native::screen::SCROLLBACK;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Polls `done` every 10 ms for up to 2 s.
    fn within_2s(mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        done()
    }

    /// What a sink saw: the Output frames' bytes, how many, and whether Exit came.
    #[derive(Default)]
    struct Seen {
        bytes: Vec<u8>,
        frames: usize,
        exited: bool,
    }

    fn collector() -> (Arc<Mutex<Seen>>, Sink) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let s = seen.clone();
        let sink: Sink = Box::new(move |f| {
            let mut s = s.lock().unwrap();
            match f {
                Frame::Output(b) => {
                    s.bytes.extend(b);
                    s.frames += 1;
                }
                Frame::Exit => s.exited = true,
            }
            true
        });
        (seen, sink)
    }

    fn has(bytes: &[u8], needle: &str) -> bool {
        String::from_utf8_lossy(bytes).contains(needle)
    }

    /// Starts `script` in `/bin/sh` in a fresh PTY.
    fn sh(host: &NativeHost, script: &str) -> TermId {
        host.spawn_argv(&["/bin/sh".into(), "-c".into(), script.into()], Path::new("/")).unwrap()
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn the_command_runs_in_a_login_shell() {
        assert_eq!(login_argv(Some("/bin/sh"), "claude"), ["/bin/sh", "-l", "-c", "claude"]);
        let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
        assert_eq!(login_argv(None, "claude"), [fallback, "-l", "-c", "claude"]);
        assert_eq!(login_argv(Some("sh"), "claude"), [fallback, "-l", "-c", "claude"], "a relative SHELL is not trusted");
        assert_eq!(login_argv(Some("/no/such/shell"), "claude"), [fallback, "-l", "-c", "claude"], "nor one that does not exist");
        assert_eq!(shell_argv("claude")[1..], ["-l", "-c", "claude"]);
    }

    #[test]
    fn output_reaches_an_attached_viewer_and_the_snapshot() {
        let host = NativeHost::new(BATCH);
        // Through `spawn`, so the user's login shell runs it.
        let term = host.spawn("printf ready; cat", Path::new("/")).unwrap();
        let (seen, sink) = collector();
        let at = host.attach(term, SCROLLBACK, sink).unwrap();
        assert_eq!((at.term, at.rows, at.cols), (term, DEFAULT_ROWS, DEFAULT_COLS));
        let got = within_2s(|| has(&at.snapshot, "ready") || has(&seen.lock().unwrap().bytes, "ready"));
        host.kill_all();
        assert!(got, "neither the snapshot nor the stream showed the output");
    }

    #[test]
    fn written_bytes_reach_the_program() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "cat");
        let (seen, sink) = collector();
        host.attach(term, 0, sink).unwrap();
        host.write(term, b"ping\r").unwrap();
        let got = within_2s(|| has(&seen.lock().unwrap().bytes, "ping"));
        host.kill_all();
        assert!(got);
    }

    #[test]
    fn a_burst_of_output_arrives_in_few_frames() {
        let host = NativeHost::new(Duration::from_millis(50));
        let term = sh(&host, "i=0; while [ $i -lt 2000 ]; do printf x; i=$((i+1)); done; sleep 1");
        let (seen, sink) = collector();
        let at = host.attach(term, 0, sink).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let early = at.snapshot.iter().filter(|&&b| b == b'x').count();
        let got = within_2s(|| early + seen.lock().unwrap().bytes.iter().filter(|&&b| b == b'x').count() >= 2000);
        host.kill_all();
        let seen = seen.lock().unwrap();
        assert!(got, "only {} x arrived", early + seen.bytes.iter().filter(|&&b| b == b'x').count());
        assert!(seen.frames <= 40, "{} frames", seen.frames);
    }

    #[test]
    fn a_terminal_whose_program_exits_sends_exit_and_is_dropped() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "sleep 0.3; exit 0");
        let root = host.root_pids()[0];
        let (seen, sink) = collector();
        host.attach(term, 0, sink).unwrap();
        assert!(within_2s(|| seen.lock().unwrap().exited), "no Exit frame");
        assert_eq!(host.term_for_root(root), None);
        assert!(host.write(term, b"x").is_err());
        assert!(host.root_pids().is_empty());
    }

    #[test]
    fn a_viewer_that_attaches_mid_batch_gets_each_byte_once() {
        let mut out = Out { screen: Screen::new(5, 20), pending: Vec::new(), viewers: Vec::new(), ended: false };
        out.feed(b"one ");
        out.attach(1, 0, Box::new(|_| true));
        out.feed(b"two ");
        let (snapshot, _, _) = out.attach(2, 0, Box::new(|_| true));
        out.feed(b"three");
        let sent: Vec<(u64, Vec<u8>)> = out.take().into_iter().map(|(id, _, bytes)| (id, bytes)).collect();
        assert!(has(&snapshot, "one two"));
        assert_eq!(sent, [(1, b"two three".to_vec()), (2, b"three".to_vec())]);
        out.feed(b"four");
        let sent: Vec<(u64, Vec<u8>)> = out.take().into_iter().map(|(id, _, bytes)| (id, bytes)).collect();
        assert_eq!(sent, [(1, b"four".to_vec()), (2, b"four".to_vec())]);
    }

    #[test]
    fn a_tiny_resize_is_clamped() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "cat");
        assert!(host.resize(term, 0, 0).is_ok());
        let at = host.attach(term, 0, Box::new(|_| true)).unwrap();
        host.kill_all();
        assert_eq!((at.rows, at.cols), (5, 20));
    }

    #[test]
    fn a_sink_that_returns_false_is_detached() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "cat");
        let at = host.attach(term, 0, Box::new(|_| false)).unwrap();
        assert_eq!(host.viewers(term), vec![at.viewer]);
        host.write(term, b"hi\r").unwrap();
        let gone = within_2s(|| host.viewers(term).is_empty());
        host.kill_all();
        assert!(gone, "the viewer is still attached");
    }

    #[test]
    fn a_viewer_detaches() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "cat");
        let a = host.attach(term, 0, Box::new(|_| true)).unwrap();
        let b = host.attach(term, 0, Box::new(|_| true)).unwrap();
        host.detach(term, a.viewer);
        let left = host.viewers(term);
        host.kill_all();
        assert_eq!(left, vec![b.viewer]);
    }

    #[test]
    fn a_sink_may_call_back_into_the_host() {
        let host = Arc::new(NativeHost::new(BATCH));
        let term = sh(&host, "cat");
        let viewer = Arc::new(Mutex::new(None::<u64>));
        let (h, v) = (host.clone(), viewer.clone());
        let at = host
            .attach(
                term,
                0,
                Box::new(move |_| {
                    if let Some(id) = *v.lock().unwrap() {
                        h.detach(term, id);
                    }
                    true
                }),
            )
            .unwrap();
        *viewer.lock().unwrap() = Some(at.viewer);
        host.write(term, b"hi\r").unwrap();
        let gone = within_2s(|| host.viewers(term).is_empty());
        host.kill_all();
        assert!(gone, "a sink that detaches itself deadlocked or was kept");
    }

    #[test]
    fn the_terminal_is_found_by_its_tty_and_its_root() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "cat");
        let roots = host.root_pids();
        let tty = host.tty(term).unwrap();
        let bare = tty.strip_prefix("/dev/").unwrap().to_string();
        let found = (
            roots.first().and_then(|&r| host.term_for_root(r)),
            roots.first().and_then(|&r| host.term_for_root(r + 100_000)),
            host.term_for_tty(&format!("/dev/{bare}")),
            host.term_for_tty(&bare),
            host.term_for_tty("ttys999"),
        );
        host.kill_all();
        assert_eq!(roots.len(), 1);
        assert_eq!(found, (Some(term), None, Some(term), Some(term), None));
    }

    #[test]
    fn kill_all_ends_every_program() {
        let host = NativeHost::new(BATCH);
        sh(&host, "sleep 30");
        sh(&host, "sleep 30");
        let roots = host.root_pids();
        assert_eq!(roots.len(), 2);
        let t = Instant::now();
        host.kill_all();
        assert!(t.elapsed() < Duration::from_millis(300), "kill_all waited for programs that ended at once");
        assert!(within_2s(|| roots.iter().all(|&p| !alive(p))), "a program outlived kill_all");
    }

    #[test]
    fn kill_all_kills_a_program_that_ignores_the_hangup() {
        let host = NativeHost::new(BATCH);
        sh(&host, "trap '' HUP; sleep 30");
        sh(&host, "trap '' HUP; sleep 30");
        let roots = host.root_pids();
        // Let the shells set their traps.
        std::thread::sleep(Duration::from_millis(200));
        let t = Instant::now();
        host.kill_all();
        let took = t.elapsed();
        assert!(within_2s(|| roots.iter().all(|&p| !alive(p))), "a program that ignores SIGHUP outlived kill_all");
        assert!(took < Duration::from_millis(700), "kill_all took {took:?}: the grace period is not shared");
    }
}
