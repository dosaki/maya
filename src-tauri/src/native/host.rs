//! The native host: pseudo-terminals Maya runs sessions in. Each terminal's
//! output is read on its own thread into a `Screen` and a pending batch,
//! and one thread sends every batch to the terminal's viewers.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
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

/// How many of its last lines an unseen terminal tells.
const LAST_LINES: usize = 3;

/// A terminal that ended unseen: no card ever showed its session and no
/// viewer was open on it. The folder it is named by and its last lines
/// that are not blank.
pub struct Unseen {
    pub term: TermId,
    pub folder: PathBuf,
    pub lines: Vec<String>,
}

/// Told each terminal that ends unseen.
pub type UnseenHook = Box<dyn Fn(Unseen) + Send + Sync>;

/// Whether a terminal's end is told: no card showed its session (`carded`),
/// no viewer shows its final screen, and Maya is not quitting.
pub fn tell_unseen(carded: bool, viewers: usize, quitting: bool) -> bool {
    !carded && viewers == 0 && !quitting
}

/// Where unseen ends are told, and whether Maya is ending every terminal.
#[derive(Default)]
struct Ends {
    unseen: OnceLock<UnseenHook>,
    quitting: AtomicBool,
}

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

/// How many times opening a pseudo-terminal is tried. On macOS it fails
/// now and then while another thread forks.
const OPEN_TRIES: u32 = 5;

/// Runs `f` until it succeeds, up to `OPEN_TRIES` times, with `pause` given
/// 10 ms more after each failure; the last error when every try failed.
fn retrying<T, E>(mut pause: impl FnMut(Duration), mut f: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let mut tries = 1;
    loop {
        match f() {
            Err(_) if tries < OPEN_TRIES => {
                pause(Duration::from_millis(10 * u64::from(tries)));
                tries += 1;
            }
            done => return done,
        }
    }
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
    /// The folder it is named by: where it started, or the folder its
    /// session works in when its command makes that (a review's worktree).
    folder: PathBuf,
    /// Set once a card showed its session.
    carded: AtomicBool,
    #[cfg(unix)]
    tty: Option<String>,
    /// None once the pseudo console is closed (Windows).
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Shared with the terminal's input thread, which types what is queued.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Input waiting for the input thread, in the order it was queued.
    input: Sender<Vec<u8>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// The program until it is reaped: while it is here, `root` cannot
    /// belong to another process.
    #[cfg(unix)]
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    /// Set while `kill_all` needs the program unreaped, so that its process
    /// group stays its own until the kill.
    #[cfg(unix)]
    reap_held: AtomicBool,
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

/// What wakes the sender: output waiting to go, or the host gone.
#[derive(Default)]
struct Wake {
    state: Mutex<WakeState>,
    rung: Condvar,
    /// Set when there is no sender thread: each reader sends its own output.
    direct: AtomicBool,
    /// How many batches the sender has sent.
    rounds: AtomicU64,
}

#[derive(Default)]
struct WakeState {
    due: bool,
    closed: bool,
}

impl Wake {
    /// Output is waiting: the terminal's reader just fed some, and `term`
    /// sends it at once when there is no sender.
    fn ring(&self, term: &Term) {
        if self.direct.load(Ordering::Relaxed) {
            term.send();
            return;
        }
        lock(&self.state).due = true;
        self.rung.notify_one();
    }

    fn close(&self) {
        lock(&self.state).closed = true;
        self.rung.notify_one();
    }

    /// Blocks until output is waiting; false once the host is gone.
    fn wait(&self) -> bool {
        let mut state = lock(&self.state);
        while !state.due && !state.closed {
            state = self.rung.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        state.due = false;
        !state.closed
    }
}

/// The pseudo-terminals Maya hosts.
pub struct NativeHost {
    terms: Arc<Terms>,
    wake: Arc<Wake>,
    ends: Arc<Ends>,
    next_term: AtomicU64,
    next_viewer: AtomicU64,
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        self.wake.close();
    }
}

impl NativeHost {
    /// A host that sends output in batches gathered over `batch`.
    pub fn new(batch: Duration) -> NativeHost {
        let terms: Arc<Terms> = Arc::default();
        let wake: Arc<Wake> = Arc::default();
        let (weak, w) = (Arc::downgrade(&terms), wake.clone());
        if let Err(e) = std::thread::Builder::new().name("native-flush".into()).spawn(move || flush(weak, w, batch)) {
            maya_core::log::line("native", format!("could not start the output sender ({e}): each terminal sends its own output"));
            wake.direct.store(true, Ordering::Relaxed);
        }
        NativeHost { terms, wake, ends: Arc::default(), next_term: AtomicU64::new(1), next_viewer: AtomicU64::new(1) }
    }

    /// Starts `command` in a login shell in a new terminal, in `cwd`.
    pub fn spawn(&self, command: &str, cwd: &Path) -> Result<TermId, String> {
        self.spawn_for_folder(command, cwd, cwd)
    }

    /// `spawn` for a session whose own folder, `folder`, is made by
    /// `command` (a review's worktree): the terminal is named by `folder`.
    pub fn spawn_for_folder(&self, command: &str, cwd: &Path, folder: &Path) -> Result<TermId, String> {
        #[cfg(windows)]
        maya_core::launch::git_bash().ok_or("Git for Windows is not installed; Maya needs its bash to start sessions (https://git-scm.com/download/win).")?;
        self.spawn_argv(&shell_argv(command), cwd, folder)
    }

    fn spawn_argv(&self, argv: &[String], cwd: &Path, folder: &Path) -> Result<TermId, String> {
        let size = PtySize { rows: DEFAULT_ROWS, cols: DEFAULT_COLS, pixel_width: 0, pixel_height: 0 };
        let pair = retrying(std::thread::sleep, || native_pty_system().openpty(size)).map_err(|e| format!("Could not open a terminal: {e}"))?;
        let mut cmd = CommandBuilder::from_argv(argv.iter().map(Into::into).collect());
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // Inside an AppImage, its variables and the entries of `PATH`,
        // `LD_LIBRARY_PATH`, `XDG_*` and the rest inside its mount go.
        #[cfg(target_os = "linux")]
        for (k, v) in maya_core::launch::appimage_env_edits() {
            match v {
                Some(v) => cmd.env(k, v),
                None => cmd.env_remove(k),
            }
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("Could not start the session: {e}"))?;
        drop(pair.slave);
        let Some(root) = child.process_id().and_then(|p| i32::try_from(p).ok()) else {
            let _ = child.kill();
            return Err("Could not start the session: it has no process id.".into());
        };
        let reader = pair.master.try_clone_reader().map_err(|e| format!("Could not read the terminal: {e}"))?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| format!("Could not write to the terminal: {e}"))?));
        let (input, queued) = channel();
        let term = Arc::new(Term {
            root,
            folder: folder.to_path_buf(),
            carded: AtomicBool::new(false),
            #[cfg(unix)]
            tty: pair.master.tty_name().map(|p| p.display().to_string()),
            master: Mutex::new(Some(pair.master)),
            writer: writer.clone(),
            input,
            killer: Mutex::new(child.clone_killer()),
            #[cfg(unix)]
            child: Mutex::new(Some(child)),
            #[cfg(unix)]
            reap_held: AtomicBool::new(false),
            out: Mutex::new(Out { screen: Screen::new(DEFAULT_ROWS, DEFAULT_COLS), pending: Vec::new(), viewers: Vec::new(), ended: false }),
            sending: Mutex::new(()),
        });
        let id = self.next_term.fetch_add(1, Ordering::Relaxed);
        lock(&self.terms).insert(id, term.clone());
        let weak = Arc::downgrade(&self.terms);
        let (wake, ends) = (self.wake.clone(), self.ends.clone());
        #[cfg(unix)]
        let started = std::thread::Builder::new().name(format!("native-{id}")).spawn(move || {
            read(id, term.clone(), reader, weak, &wake, &ends);
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
            std::thread::Builder::new().name(format!("native-{id}")).spawn(move || read(id, term, reader, weak, &wake, &ends))
        };
        let started = started.and_then(|_| std::thread::Builder::new().name(format!("native-{id}-input")).spawn(move || type_queued(&queued, &writer)));
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

    /// Queues `bytes` to be typed into the terminal after what was queued
    /// before, and returns at once: a program that is not reading its input
    /// holds up only the terminal's input thread.
    pub fn enqueue(&self, term: TermId, bytes: &[u8]) -> Result<(), String> {
        self.get(term)?.input.send(bytes.to_vec()).map_err(|_| "That session's terminal has closed.".into())
    }

    /// Types `bytes` into the terminal, and returns once they are in.
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

    /// Notes that a card showed the session of each terminal whose
    /// program's pid is in `roots`: its end is then not told.
    pub fn mark_carded(&self, roots: &[i32]) {
        for (_, t) in self.all() {
            if roots.contains(&t.root) {
                t.carded.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Where to tell each terminal that ends unseen; the first one set stays.
    pub fn on_unseen_end(&self, hook: UnseenHook) {
        let _ = self.ends.unseen.set(hook);
    }

    /// The folder of every terminal no card has shown yet: its session is
    /// still starting, so the board does not show it.
    pub fn starting(&self) -> Vec<PathBuf> {
        self.all().into_iter().filter(|(_, t)| !t.carded.load(Ordering::Relaxed)).map(|(_, t)| t.folder.clone()).collect()
    }

    pub fn term_for_root(&self, root_pid: i32) -> Option<TermId> {
        self.all().into_iter().find(|(_, t)| t.root == root_pid).map(|(id, _)| id)
    }

    /// The pid of every hosted terminal's program.
    pub fn root_pids(&self) -> Vec<i32> {
        self.all().into_iter().map(|(_, t)| t.root).collect()
    }

    /// Ends every terminal's program, for when Maya quits. On Unix each
    /// process group gets a hangup, and every group with a process still
    /// running after one shared grace period is killed, even one whose
    /// leader ended on the hangup.
    pub fn kill_all(&self) {
        // Their ends are Maya's own doing: none is told.
        self.ends.quitting.store(true, Ordering::SeqCst);
        let all: Vec<Arc<Term>> = self.all().into_iter().map(|(_, t)| t).collect();
        // No leader is reaped until the kill: an unreaped leader, even a
        // zombie, keeps its pid, so its group cannot be another's.
        #[cfg(unix)]
        for t in &all {
            t.reap_held.store(true, Ordering::SeqCst);
        }
        for t in &all {
            hang_up(t);
        }
        #[cfg(unix)]
        {
            let end = Instant::now() + GRACE;
            while Instant::now() < end && !all.iter().all(|t| settled(t)) {
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
            for t in &all {
                t.reap_held.store(false, Ordering::SeqCst);
            }
        }
    }

    fn kill(&self, term: TermId) {
        if let Ok(t) = self.get(term) {
            hang_up(&t);
        }
    }

    #[cfg(all(test, unix))]
    fn flush_rounds(&self) -> u64 {
        self.wake.rounds.load(Ordering::Relaxed)
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

/// Whether nothing in the terminal's process group runs any more. Its
/// terminal closes once no process has it open; on macOS that also happens
/// when the group's leader ends, so the group is asked as well (a zombie
/// leader does not count there).
#[cfg(unix)]
fn settled(t: &Term) -> bool {
    if !lock(&t.out).ended {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: signal 0 only checks; the leader is unreaped, so the group is its own.
        unsafe { libc::kill(-t.root, 0) != 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Reaps the terminal's program if it has ended; true once it is reaped.
/// While `kill_all` holds reaping, false.
#[cfg(unix)]
fn reaped(t: &Term) -> bool {
    let mut child = lock(&t.child);
    if t.reap_held.load(Ordering::SeqCst) {
        return false;
    }
    if let Some(c) = child.as_mut() {
        if !matches!(c.try_wait(), Ok(None)) {
            *child = None;
        }
    }
    child.is_none()
}

/// A terminal's input thread: types each queued input in turn, until the
/// terminal is dropped or cannot be typed into.
fn type_queued(queued: &Receiver<Vec<u8>>, writer: &Mutex<Box<dyn Write + Send>>) {
    for bytes in queued {
        let mut w = lock(writer);
        if w.write_all(&bytes).and_then(|()| w.flush()).is_err() {
            return;
        }
    }
}

/// The host-wide sender: once output is waiting, what gathers over `batch`
/// goes to each terminal's viewers; while output flows, a batch goes every
/// `batch`. It sleeps while nothing is printed, and ends with the host.
fn flush(terms: Weak<Terms>, wake: Arc<Wake>, batch: Duration) {
    while wake.wait() {
        std::thread::sleep(batch);
        wake.rounds.fetch_add(1, Ordering::Relaxed);
        let Some(terms) = terms.upgrade() else { return };
        let all: Vec<Arc<Term>> = lock(&terms).values().cloned().collect();
        drop(terms);
        for t in all {
            t.send();
        }
    }
}

/// A terminal's reader: feeds what the program prints into the screen and
/// the pending batch. At the end it sends the rest and `Exit`, drops the
/// terminal, and tells its end if it ended unseen.
fn read(id: TermId, term: Arc<Term>, mut reader: Box<dyn Read + Send>, terms: Weak<Terms>, wake: &Wake, ends: &Ends) {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                lock(&term.out).feed(&buf[..n]);
                wake.ring(&term);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let sending = lock(&term.sending);
    let (delivery, lines) = {
        let mut out = lock(&term.out);
        out.ended = true;
        let delivery = out.take();
        out.viewers.clear();
        (delivery, out.screen.last_lines(LAST_LINES))
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
    drop(sending);
    if tell_unseen(term.carded.load(Ordering::Relaxed), delivery.len() - dead.len(), ends.quitting.load(Ordering::SeqCst)) {
        if let Some(hook) = ends.unseen.get() {
            hook(Unseen { term: id, folder: term.folder.clone(), lines });
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
        host.spawn_argv(&["/bin/sh".into(), "-c".into(), script.into()], Path::new("/"), Path::new("/")).unwrap()
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
    fn opening_a_terminal_is_tried_again_a_few_times() {
        let mut calls = 0;
        let mut pauses = Vec::new();
        let got: Result<u32, &str> = retrying(
            |d| pauses.push(d),
            || {
                calls += 1;
                if calls < 3 { Err("busy") } else { Ok(calls) }
            },
        );
        assert_eq!(got, Ok(3));
        assert_eq!(pauses, [Duration::from_millis(10), Duration::from_millis(20)]);

        let (mut calls, mut pauses) = (0, 0);
        let got: Result<(), &str> = retrying(
            |_| pauses += 1,
            || {
                calls += 1;
                Err("busy")
            },
        );
        assert_eq!((got, calls, pauses), (Err("busy"), OPEN_TRIES, OPEN_TRIES - 1));
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

    /// Starts a terminal that copies every byte typed into it, untouched,
    /// to `file`, and returns it once it is ready.
    fn recorder(host: &NativeHost, file: &Path) -> TermId {
        let term = sh(host, &format!("stty raw -echo; printf ready; cat > '{}'", file.display()));
        let (seen, sink) = collector();
        let at = host.attach(term, 0, sink).unwrap();
        if !within_2s(|| has(&at.snapshot, "ready") || has(&seen.lock().unwrap().bytes, "ready")) {
            host.kill_all();
            panic!("the terminal never got ready");
        }
        term
    }

    #[test]
    fn queued_input_reaches_the_program_in_order() {
        let host = NativeHost::new(BATCH);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("typed");
        let term = recorder(&host, &file);
        let want: String = (0..300).map(|i| format!("{i:04},")).collect();
        for i in 0..300 {
            host.enqueue(term, format!("{i:04},").as_bytes()).unwrap();
        }
        let got = within_2s(|| std::fs::read_to_string(&file).is_ok_and(|s| s == want));
        let typed = std::fs::read_to_string(&file).unwrap_or_default();
        host.kill_all();
        assert!(got, "typed {} bytes, out of order or short", typed.len());
    }

    #[test]
    fn queuing_input_does_not_wait_for_the_program_to_read_it() {
        let host = NativeHost::new(BATCH);
        // sleep never reads, so the terminal's input fills up.
        let term = sh(&host, "sleep 30");
        let t = Instant::now();
        let queued = (0..64).map(|_| host.enqueue(term, &[b'x'; 16 * 1024])).collect::<Result<Vec<()>, String>>();
        let took = t.elapsed();
        host.kill_all();
        assert_eq!(queued, Ok(vec![(); 64]));
        assert!(took < Duration::from_millis(200), "queuing 1 MB took {took:?}");
        assert!(host.enqueue(term + 1000, b"x").is_err(), "no such terminal");
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
    fn the_sender_sleeps_while_nothing_is_printed() {
        let host = NativeHost::new(BATCH);
        let term = sh(&host, "stty -echo; cat");
        let (seen, sink) = collector();
        host.attach(term, 0, sink).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let before = host.flush_rounds();
        std::thread::sleep(Duration::from_millis(300));
        let idle = host.flush_rounds() - before;
        host.write(term, b"ping\r").unwrap();
        let got = within_2s(|| has(&seen.lock().unwrap().bytes, "ping"));
        let busy = host.flush_rounds() - before;
        host.kill_all();
        assert_eq!(idle, 0, "the sender woke {idle} times with nothing to send");
        assert!(got && busy > 0, "output still reaches the viewer");
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
    fn a_terminal_is_starting_until_a_card_shows_it() {
        let host = NativeHost::new(BATCH);
        sh(&host, "sleep 30");
        assert_eq!(host.starting(), [PathBuf::from("/")]);
        host.mark_carded(&host.root_pids());
        assert!(host.starting().is_empty());
        host.kill_all();
    }

    #[test]
    fn an_end_is_told_only_without_a_card_a_viewer_or_a_quit() {
        assert!(tell_unseen(false, 0, false));
        assert!(!tell_unseen(true, 0, false), "its card showed it");
        assert!(!tell_unseen(false, 1, false), "an open viewer shows the final screen");
        assert!(!tell_unseen(false, 0, true), "Maya is quitting");
    }

    /// A host that collects the ends it tells.
    fn telling() -> (NativeHost, Arc<Mutex<Vec<Unseen>>>) {
        let host = NativeHost::new(BATCH);
        let told = Arc::new(Mutex::new(Vec::new()));
        let t = told.clone();
        host.on_unseen_end(Box::new(move |u| t.lock().unwrap().push(u)));
        (host, told)
    }

    #[test]
    fn a_terminal_that_ends_before_its_card_tells_its_folder_and_last_lines() {
        let (host, told) = telling();
        let dir = tempfile::tempdir().unwrap();
        let term = host.spawn_argv(&["/bin/sh".into(), "-c".into(), "printf 'one\\n\\ntwo\\nthree\\nfour\\n'; exit 3".into()], dir.path(), dir.path()).unwrap();
        assert!(within_2s(|| !told.lock().unwrap().is_empty()), "the end was not told");
        let told = told.lock().unwrap();
        assert_eq!(told.len(), 1);
        assert_eq!((told[0].term, told[0].folder.as_path(), told[0].lines.as_slice()), (term, dir.path(), ["two", "three", "four"].map(String::from).as_slice()));
    }

    #[test]
    fn a_terminal_started_for_another_folder_is_named_by_it() {
        let (host, told) = telling();
        let made = Path::new("/work/bedrock-451");
        let term = host.spawn_argv(&["/bin/sh".into(), "-c".into(), "read l".into()], Path::new("/"), made).unwrap();
        assert_eq!(host.starting(), [made]);
        host.write(term, b"\r").unwrap();
        assert!(within_2s(|| !told.lock().unwrap().is_empty()), "the end was not told");
        assert_eq!(told.lock().unwrap()[0].folder, made);
    }

    #[test]
    fn a_terminal_seen_on_a_card_or_in_a_viewer_or_ended_by_a_quit_ends_quietly() {
        let (host, told) = telling();
        sh(&host, "sleep 0.3");
        host.mark_carded(&host.root_pids());
        let viewed = sh(&host, "sleep 0.3");
        host.attach(viewed, 0, Box::new(|_| true)).unwrap();
        assert!(within_2s(|| host.root_pids().is_empty()));
        sh(&host, "sleep 30");
        host.kill_all();
        assert!(within_2s(|| host.root_pids().is_empty()));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(told.lock().unwrap().len(), 0);
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
        for _ in 0..3 {
            sh(&host, "trap '' HUP; sleep 30");
        }
        let roots = host.root_pids();
        // Let the shells set their traps.
        std::thread::sleep(Duration::from_millis(200));
        let t = Instant::now();
        host.kill_all();
        let took = t.elapsed();
        assert!(within_2s(|| roots.iter().all(|&p| !alive(p))), "a program that ignores SIGHUP outlived kill_all");
        // One grace period each would take 1200 ms.
        assert!(took < Duration::from_millis(1000), "kill_all took {took:?}: the grace period is not shared");
    }

    #[test]
    fn kill_all_kills_what_ignores_the_hangup_after_its_leader_ended() {
        let host = NativeHost::new(BATCH);
        // The shell ends on the hangup; the sleep it started ignores it and keeps the tty.
        let term = sh(&host, "(trap '' HUP; exec sleep 30) & echo pid=$!; wait");
        let (seen, sink) = collector();
        let at = host.attach(term, 0, sink).unwrap();
        let pid = |bytes: &[u8]| {
            let text = String::from_utf8_lossy(bytes).into_owned();
            text.split("pid=").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()?.parse::<i32>().ok())
        };
        let mut member = None;
        within_2s(|| {
            member = pid(&at.snapshot).or_else(|| pid(&seen.lock().unwrap().bytes));
            member.is_some()
        });
        let Some(member) = member else {
            host.kill_all();
            panic!("the shell never said its sleep's pid");
        };
        // Let the sleep set its trap.
        std::thread::sleep(Duration::from_millis(200));
        host.kill_all();
        let gone = within_2s(|| !alive(member));
        if !gone {
            // SAFETY: the test's own sleep, still running.
            unsafe { libc::kill(member, libc::SIGKILL) };
        }
        assert!(gone, "a program that ignores SIGHUP outlived its leader and kill_all");
    }
}
