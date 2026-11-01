//! The assistant Maya's client: one WebSocket to the main, opened by this
//! side, reconnecting with backoff.
//!
//! Threads: the app's `start` spawns one connection thread that loops
//! `run_once` through `reconnect`, waiting `backoff_ms` between failures. `run_once` owns the socket, and
//! its read timeout (`TICK`, `LIVE_TICK` while a live terminal streams on the
//! link) is the loop's tick: each turn sends the results
//! finished commands queued, a board when one is due, a ping every ten
//! seconds, then tries one read. Each command runs on its own short-lived
//! thread and queues its `result` on the connection's channel; a result that
//! finishes after its connection closed is dropped. The stop flag ends the
//! connection thread within a tick (within the connect timeout while a TCP
//! connect is in flight).
//!
//! Locks: the `RequestLane` holds a mutex briefly, never across a send that
//! blocks or a callback. The app's adapters (`net_app`) take `network` or
//! `store` one at a time, never one while holding the other.

use super::identity::{decode_key, encode_key, Keypair};
use super::live::{self, FrameSink, LiveTerms, TermEvent, TermFrame, TermView};
use super::protocol::{decode_down, encode, Attachment, CommandKind, Down, Request, Up, DEFAULT_PORT, MAX_FRAME, PRE_AUTH_FRAME, PROTOCOL};
use super::secure::{self, Channel, ChannelError, Fail, Got};
use super::routing::MachineInfo;
use crate::config::{NetworkConfig, NetworkRole};
use crate::log;
use crate::model::Card;
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::WebSocket;

/// How long one read waits; the connection loop's tick.
const TICK: Duration = Duration::from_millis(100);
/// A main silent this long is gone (it answers every ping), and the
/// handshake, pairing included, must finish within it.
const SILENCE: Duration = Duration::from_secs(15);
const PING_EVERY: Duration = Duration::from_secs(10);
/// Boards go out at most once a second.
const BOARD_GAP: Duration = Duration::from_secs(1);
/// And at least this often, changed or not, so the main's copy (stale after
/// 30 s) stays fresh.
const BOARD_REFRESH: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long looking up the main's name may take.
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
/// A connection that lasted this long starts the backoff over.
const STEADY: Duration = Duration::from_secs(60);

/// What a request reads when no connection to the main is up.
pub const NOT_CONNECTED: &str = "Not connected to the main Maya.";
/// What a waiting request reads when its connection drops.
pub const LOST: &str = "Lost the connection to the main Maya.";
/// How long a request waits: the main itself waits up to 30 s on an assistant.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);

pub const REMOVED: &str = "Removed by the main Maya; pair again.";
/// What the user reads when the main refuses a pairing code.
pub const WRONG_CODE: &str = "Wrong or expired pairing code.";
pub const NOT_PAIRED: &str = "Not paired with a main Maya yet; pair in Settings.";
pub const KEY_CHANGED: &str = "The main Maya's key has changed. If you reset it, pair again; otherwise someone may be impersonating it.";
pub const OLD_MAIN: &str = "The main runs an older Maya. Update it.";
pub const NO_ANSWER: &str = "The main Maya did not answer. If it runs an older Maya, update it.";
pub const PAIRING_FAILED: &str = "The pairing did not complete; try again with a new code.";
/// What the user reads for a refusal whose reason this Maya does not know:
/// refusals are unauthenticated, so their text is never shown.
pub const REFUSED: &str = "The main Maya refused the connection.";
const TOO_MANY_CODES: &str = "Too many wrong codes; wait five minutes and try again.";
/// Why a connection ended when the main moved; `reconnect` tries the new host at once.
pub const MOVED: &str = "The main Maya moved to another machine.";
pub const CANNOT_LEAD: &str = "This Maya cannot become a main.";
pub const CANNOT_FOLLOW: &str = "This Maya cannot follow the main to another machine.";
/// How long the client waits before trying again after `KEY_CHANGED`.
const UNPROVEN_WAIT_MS: u64 = 30_000;

/// Runs the main's commands on this Maya and describes its board.
pub trait Executor: Send + Sync {
    fn execute(&self, kind: CommandKind) -> Result<Option<Value>, String>;
    /// This Maya's own cards and project folders.
    fn board(&self) -> (Vec<Card>, Vec<String>);
    /// True once after the board may have changed (see `push_board`).
    fn board_requested(&self) -> bool {
        false
    }
    /// The agents this machine can start, for the main's New session modal.
    fn agents(&self) -> Option<Vec<crate::agents::AgentInfo>> {
        None
    }
    /// What was running on this machine when its Maya last stopped, for
    /// the main's banner; `None` from a Maya that does not remember.
    fn remembered(&self) -> Option<Vec<crate::remembered::Remembered>> {
        None
    }
    /// The terminals this Maya streams to the main; `None` from a Maya with none.
    fn live_terms(&self) -> Option<Arc<dyn LiveTerms>> {
        None
    }
}

/// What the client tells the app.
pub trait ClientNotify: Send + Sync {
    /// The main's public key, learned at pairing (pairing only).
    fn paired(&self, main_key: &str);
    fn connected(&self, main_name: &str);
    fn disconnected(&self, error: &str);
    fn removed(&self);
    /// What this Maya can do, sent in `hello` (`CAP_FOLLOW`, `CAP_LEAD`).
    fn caps(&self) -> Vec<String> {
        vec![]
    }
    /// The main handed this Maya its paired list: become the main with it.
    /// `Ok(port)` once its server listens; on `Err` nothing changed. Runs on
    /// a command thread; the client ends by itself once the answer is out,
    /// so this must not stop or join the client.
    /// `summary`: what the old main's Maya did while this one's brain was
    /// dormant; none from a phone or an older main.
    fn promoted(&self, _assistants: Vec<crate::config::PairedAssistant>, _summary: Option<String>) -> Result<u16, String> {
        Err(CANNOT_LEAD.into())
    }
    /// When this Maya's brain went dormant, sent in `hello`; None when it
    /// never ran one.
    fn brain_dormant_since(&self) -> Option<u64> {
        None
    }
    /// The main is now at `host:port` with the public key `key`: save both.
    /// Runs on the connection thread.
    fn moved(&self, _host: &str, _port: u16, _key: &str) -> Result<(), String> {
        Err(CANNOT_FOLLOW.into())
    }
    /// What the main said at `welcome` about mirroring (`Some(true)`: it
    /// will; `Some(false)`: its switch is off; `None`: it did not say).
    /// Called right after `connected`.
    fn mirror_offer(&self, _offered: Option<bool>) {}
    /// What the main said it can do at `welcome` (`CAP_VOICE`); empty for
    /// an older main. Called right after `mirror_offer`.
    fn main_caps(&self, _caps: &[String]) {}
    /// A turn of the main's Chat (`Down::Chat`), for a phone that asked to
    /// mirror it. Runs on the client thread: do not wait on the main here.
    fn chat_turn(&self, _turn: Value) {}
    /// The main's brain status (`Down::Brain`), sent with its chat turns.
    /// Runs on the client thread, like `chat_turn`.
    fn brain_status(&self, _status: Value) {}
    /// The word the main listens for (`None` from an older main), with
    /// each board it shares, before `mirrored_remembered`.
    fn main_wake_word(&self, _word: Option<&str>) {}
    /// The main's "was running" rows, just before `mirrored` with its board.
    fn mirrored_remembered(&self, _rows: Vec<crate::remembered::RememberedRow>) {}
    /// A board from the main, or `allowed: false` once its switch went off.
    fn mirrored(&self, _allowed: bool, _cards: Vec<Card>, _machines: Vec<MachineInfo>) {}
    /// Where this Maya's requests to the main go out; `None` sends none.
    fn lane(&self) -> Option<Arc<RequestLane>> {
        None
    }
    /// True while this Maya views a live terminal through the main: its link
    /// then ticks at `LIVE_TICK`.
    fn live(&self) -> bool {
        false
    }
}

/// Requests to the main from this Maya's page, matched to their answers by
/// id. Each connection attaches its outbox while it runs; when it ends,
/// every request still waiting fails with `LOST`.
#[derive(Default)]
pub struct RequestLane {
    inner: Mutex<Lane>,
}

#[derive(Default)]
struct Lane {
    out: Option<Sender<Up>>,
    /// Which connection attached `out`, so an older one cannot detach a newer.
    generation: u64,
    next: u64,
    waiting: HashMap<u64, Sender<Result<Option<Value>, String>>>,
}

impl RequestLane {
    /// Sends `request` to the main and waits up to `timeout` for its answer.
    pub fn request(&self, machine: Option<String>, request: Request, timeout: Duration) -> Result<Option<Value>, String> {
        let (id, rx) = {
            let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let out = l.out.clone().ok_or(NOT_CONNECTED)?;
            l.next += 1;
            let id = l.next;
            let (tx, rx) = channel();
            l.waiting.insert(id, tx);
            if out.send(Up::Request { id, machine, request }).is_err() {
                l.waiting.remove(&id);
                return Err(NOT_CONNECTED.into());
            }
            (id, rx)
        };
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.inner.lock().unwrap_or_else(|e| e.into_inner()).waiting.remove(&id);
                Err("The main Maya did not answer in time.".into())
            }
        }
    }

    fn attach(&self, out: Sender<Up>) -> u64 {
        let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        l.generation += 1;
        l.out = Some(out);
        l.generation
    }

    fn detach(&self, generation: u64) {
        let mut l = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if l.generation != generation {
            return;
        }
        l.out = None;
        for (_, w) in l.waiting.drain() {
            let _ = w.send(Err(LOST.into()));
        }
    }

    fn resolve(&self, id: u64, result: Result<Option<Value>, String>) {
        let waiter = self.inner.lock().unwrap_or_else(|e| e.into_inner()).waiting.remove(&id);
        if let Some(w) = waiter {
            let _ = w.send(result);
        }
    }
}

/// Detaches the lane when the connection's pump ends, however it ends.
struct Attached(Option<(Arc<RequestLane>, u64)>);

impl Drop for Attached {
    fn drop(&mut self) {
        if let Some((lane, generation)) = &self.0 {
            lane.detach(*generation);
        }
    }
}

/// What the main reads when it asks a Maya with no live terminals for one.
pub(crate) const NO_TERMINALS: &str = "This Maya has no live terminals.";
/// What the main reads for keys to a terminal this Maya does not stream to it.
pub(crate) const NO_SUCH_TERMINAL: &str = "This terminal is not open here.";
/// What the main reads for keys that are not base64.
pub(crate) const UNREADABLE_KEYS: &str = "The keys could not be read.";

/// The terminals one connection streams to the main, by the main's `sub`.
/// Every view still open is detached when the pump ends, however it ends.
struct Streams {
    terms: Option<Arc<dyn LiveTerms>>,
    subs: HashMap<u64, TermView>,
}

impl Streams {
    /// Streams `session` to the main as `sub`, in place of whatever `sub` streamed before.
    fn attach(&mut self, sub: u64, session: &str, tx: &Sender<Up>) {
        self.detach(sub);
        let Some(terms) = &self.terms else {
            return term_error(tx, sub, NO_TERMINALS.into());
        };
        let out = tx.clone();
        // The host calls it from its own threads, holding its locks: it only
        // queues, and is false once the pump has ended.
        let sink: FrameSink = Box::new(move |frame| out.send(Up::TermFrame { sub, frame }).is_ok());
        match live::open(&**terms, session, sink) {
            Ok(view) => {
                log::line("network", format!("terminal {sub} attached"));
                self.subs.insert(sub, view);
            }
            Err(e) => {
                log::line("network", format!("terminal {sub} not attached: {e}"));
                term_error(tx, sub, e);
            }
        }
    }

    /// Types `data`, base64, into `sub`'s terminal. A failure ends the view:
    /// its `Error` frame is the last the main hears of it.
    fn input(&mut self, sub: u64, data: &str, tx: &Sender<Up>) {
        let (Some(terms), Some(view)) = (&self.terms, self.subs.get(&sub)) else {
            return term_error(tx, sub, NO_SUCH_TERMINAL.into());
        };
        let typed = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|_| UNREADABLE_KEYS.to_string()).and_then(|bytes| terms.input(view, &bytes));
        if let Err(e) = typed {
            log::line("network", format!("terminal {sub} ended: {e}"));
            term_error(tx, sub, e);
            self.detach(sub);
        }
    }

    fn detach(&mut self, sub: u64) {
        if let (Some(terms), Some(view)) = (&self.terms, self.subs.remove(&sub)) {
            terms.detach(&view);
            log::line("network", format!("terminal {sub} detached"));
        }
    }
}

impl Drop for Streams {
    fn drop(&mut self) {
        if let Some(terms) = &self.terms {
            for (_, view) in self.subs.drain() {
                terms.detach(&view);
            }
        }
    }
}

/// Tells the main `sub` cannot be shown; nothing follows it.
fn term_error(tx: &Sender<Up>, sub: u64, message: String) {
    let _ = tx.send(Up::TermFrame { sub, frame: TermFrame { seq: 0, event: TermEvent::Error { message } } });
}

/// 1, 2, 4, 8 and 16 seconds, then every 30.
pub fn backoff_ms(attempt: u32) -> u64 {
    if attempt < 5 {
        1_000 << attempt
    } else {
        30_000
    }
}

/// The name this assistant goes by: the configured one, else the hostname.
pub fn display_name(name: &str, hostname: &str) -> String {
    match name.trim() {
        "" => hostname.to_string(),
        n => n.to_string(),
    }
}

/// The host and code a pairing form gave, checked: a host with no spaces
/// or slashes, and six digits (spaces dropped).
pub fn check_pair_args(host: &str, code: &str) -> Result<(String, String), String> {
    let host = host.trim();
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '/') {
        return Err("Type the main Maya's host name or address.".into());
    }
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("The pairing code has six digits.".into());
    }
    Ok((host.to_string(), code))
}

const ATTACHED: &str = "Attached file: ";

/// The reply text with each `Attached file: <path on the main>` line
/// pointing at the copy saved here.
pub fn rewrite_attachments(text: &str, saved: &[(String, PathBuf)]) -> String {
    text.split('\n')
        .map(|line| {
            let local = line.strip_prefix(ATTACHED).and_then(|p| saved.iter().find(|(orig, _)| orig == p.trim()));
            match local {
                Some((_, path)) => format!("{ATTACHED}{}", path.display()),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs a reply from the main that carries attachments. Nothing is written
/// unless `session_exists`; the files are saved under `maya_dir`, `send`
/// gets the text pointing at them, and on any failure the files saved so
/// far are deleted again.
pub fn reply_with_attachments<T>(maya_dir: &Path, session_exists: bool, text: &str, attachments: Vec<Attachment>, now_ms: u64, send: impl FnOnce(String) -> Result<T, String>) -> Result<T, String> {
    if !session_exists {
        return Err("Session is no longer running.".into());
    }
    let mut saved: Vec<(String, PathBuf)> = Vec::with_capacity(attachments.len());
    let out = save_attachments(maya_dir, attachments, now_ms, &mut saved).and_then(|()| send(rewrite_attachments(text, &saved)));
    if out.is_err() {
        for (_, path) in &saved {
            let _ = std::fs::remove_file(path);
        }
    }
    out
}

fn save_attachments(maya_dir: &Path, attachments: Vec<Attachment>, now_ms: u64, saved: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
    for a in attachments {
        let bytes = base64::engine::general_purpose::STANDARD.decode(a.bytes.as_bytes()).map_err(|_| format!("The attachment {} could not be read.", clean(&a.name)))?;
        let path = crate::attachments::save(maya_dir, &a.name, &bytes, now_ms)?;
        saved.push((a.name, path));
    }
    Ok(())
}

/// Text from the main, stripped of control characters and kept short.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(64).collect()
}

/// What the user reads for a main's `bye` reason.
fn bye_message(reason: &str) -> String {
    match reason {
        "removed" => REMOVED.into(),
        "wrong or expired pairing code" => WRONG_CODE.into(),
        "too many attempts" => TOO_MANY_CODES.into(),
        "unknown key" => KEY_CHANGED.into(),
        "protocol" => "The main Maya runs a different version of Maya.".into(),
        "could not save the pairing" => "The main could not save the pairing.".into(),
        other => format!("The main Maya closed the connection ({}).", clean(other)),
    }
}

/// What the user reads when the handshake did not complete. A refusal is
/// unauthenticated, so only the reasons this Maya knows become their own message.
fn fail_message(f: &Fail, pairing: bool) -> String {
    match f {
        Fail::OldPeer => OLD_MAIN.into(),
        Fail::Refused(reason) if reason == secure::WRONG_CODE_REASON => WRONG_CODE.into(),
        Fail::Refused(reason) if reason == "too many attempts" => TOO_MANY_CODES.into(),
        Fail::Refused(reason) if reason == secure::UNKNOWN_KEY_REASON => KEY_CHANGED.into(),
        Fail::Refused(_) => REFUSED.into(),
        Fail::Crypto if pairing => PAIRING_FAILED.into(),
        Fail::Crypto => KEY_CHANGED.into(),
        Fail::Io(e) if e == "stopped" => "stopped".into(),
        Fail::Io(_) | Fail::Malformed => NO_ANSWER.into(),
    }
}

type Ws = WebSocket<TcpStream>;
type Chan = Channel<TcpStream>;

fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Ordering::SeqCst)
}

fn port_of(config: &NetworkConfig) -> u16 {
    if config.main_port == 0 {
        DEFAULT_PORT
    } else {
        config.main_port
    }
}

/// Looks `host` up, giving up after `DNS_TIMEOUT`.
fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let name = host.to_string();
    resolve_with(host, DNS_TIMEOUT, move || (name.as_str(), port).to_socket_addrs().map(|a| a.collect()))
}

/// Runs `lookup` on a helper thread and waits at most `timeout` for it: the
/// system resolver has no deadline of its own, and a stuck lookup would
/// otherwise hold the connection thread, and a stop, as long as it likes.
/// A lookup that times out finishes on its own and is ignored.
fn resolve_with(host: &str, timeout: Duration, lookup: impl FnOnce() -> std::io::Result<Vec<SocketAddr>> + Send + 'static) -> Result<Vec<SocketAddr>, String> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("net-dns".into())
        .spawn(move || {
            let _ = tx.send(lookup());
        })
        .map_err(|e| format!("Could not find {host}: {e}"))?;
    match rx.recv_timeout(timeout) {
        Ok(Ok(addrs)) => Ok(addrs),
        Ok(Err(e)) => Err(format!("Could not find {host}: {e}")),
        Err(_) => Err(format!("Could not find {host}: the lookup took more than {} s.", timeout.as_secs())),
    }
}

/// Opens the TCP connection and the WebSocket handshake, by `deadline`.
fn connect(config: &NetworkConfig, stop: &AtomicBool, deadline: Instant) -> Result<Ws, String> {
    let host = config.main_host.trim();
    if host.is_empty() {
        return Err("Type the main Maya's host first.".into());
    }
    let port = port_of(config);
    let addrs = resolve(host, port)?;
    let mut stream = None;
    let mut last = String::from("no address");
    for a in addrs {
        if stopped(stop) {
            return Err("stopped".into());
        }
        match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let stream = stream.ok_or_else(|| format!("Could not reach {host}:{port}: {last}"))?;
    let _ = stream.set_read_timeout(Some(TICK));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);
    // Non-blocking for the upgrade: tungstenite resumes it only after
    // WouldBlock, and a read timeout is TimedOut on Windows, which would end
    // an upgrade answered a read tick late. The timeouts apply once it is done.
    stream.set_nonblocking(true).map_err(|e| format!("Could not reach {host}:{port}: {e}"))?;
    let url = if host.contains(':') && !host.starts_with('[') { format!("ws://[{host}]:{port}/") } else { format!("ws://{host}:{port}/") };
    // Small until `welcome`, so whatever answers cannot make this Maya buffer more.
    let ws_config = WebSocketConfig::default().max_message_size(Some(PRE_AUTH_FRAME)).max_frame_size(Some(PRE_AUTH_FRAME));
    let mut attempt = tungstenite::client::client_with_config(url, stream, Some(ws_config));
    loop {
        match attempt {
            Ok((ws, _)) => {
                ws.get_ref().set_nonblocking(false).map_err(|e| format!("Could not reach {host}:{port}: {e}"))?;
                return Ok(ws);
            }
            Err(HandshakeError::Interrupted(mid)) => {
                if stopped(stop) || Instant::now() > deadline {
                    return Err(format!("{host}:{port} did not answer as a Maya."));
                }
                std::thread::sleep(secure::HANDSHAKE_POLL);
                attempt = mid.handshake();
            }
            Err(HandshakeError::Failure(e)) => return Err(format!("{host}:{port} did not answer as a Maya: {e}")),
        }
    }
}

fn send_up(ch: &mut Chan, up: &Up) -> Result<(), String> {
    ch.send(&encode(up)).map_err(|e| format!("Lost the connection to the main Maya: {e}"))
}

/// The next message from the main that parses, by `deadline`; garbage is logged and skipped.
fn read_down(ch: &mut Chan, stop: &AtomicBool, deadline: Instant) -> Result<Down, String> {
    loop {
        if stopped(stop) {
            return Err("stopped".into());
        }
        // Checked on every frame, not only on a quiet read, so pings cannot hold it open.
        if Instant::now() > deadline {
            return Err("The main Maya did not answer in time.".into());
        }
        match ch.read() {
            Ok(Got::Text(t)) => match decode_down(&t) {
                Ok(down) => return Ok(down),
                Err(e) => log::line("network", format!("main: ignored a message: {e}")),
            },
            Ok(Got::Closed) => return Err("The main Maya closed the connection.".into()),
            Ok(Got::Control | Got::Idle) => {}
            Err(ChannelError::Forged) => {
                log::line("network", format!("main: {}", secure::FORGED));
                return Err(secure::FORGED.into());
            }
            Err(ChannelError::Io(e)) => return Err(format!("Lost the connection to the main Maya: {e}")),
        }
    }
}

/// Closes the WebSocket and gives the main up to a second to agree.
fn finish(ws: &mut Ws) {
    let _ = ws.close(None);
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        match ws.read() {
            Ok(_) => {}
            Err(e) if secure::is_timeout(&e) => {}
            Err(_) => break,
        }
    }
}

/// One connection to the main: pairs with `code`, or connects with the
/// pinned main key, sends boards and runs commands until the link drops
/// (`Err`) or `stop` is set (`Ok`). The notifier hears of every failure
/// except one after a stop.
pub fn run_once(config: &NetworkConfig, identity: &Keypair, exec: Arc<dyn Executor>, notify: Arc<dyn ClientNotify>, stop: &AtomicBool, code: Option<&str>) -> Result<(), String> {
    match connection(config, identity, &exec, &notify, stop, code) {
        Ok(()) => Ok(()),
        Err(_) if stopped(stop) => Ok(()),
        Err(e) => {
            if e == REMOVED {
                notify.removed();
            } else {
                notify.disconnected(&e);
            }
            Err(e)
        }
    }
}

fn connection(config: &NetworkConfig, identity: &Keypair, exec: &Arc<dyn Executor>, notify: &Arc<dyn ClientNotify>, stop: &AtomicBool, code: Option<&str>) -> Result<(), String> {
    let main_key = match code {
        Some(_) => None,
        None => Some(decode_key(&config.main_key).map_err(|_| NOT_PAIRED.to_string())?),
    };
    let deadline = Instant::now() + SILENCE;
    let mut ws = connect(config, stop, deadline)?;
    let quit = || stopped(stop);
    let shook = match (code, main_key) {
        (Some(code), _) => secure::pair(&mut ws, identity, code, deadline, &quit).map(|(t, k)| (t, Some(k))),
        (None, Some(k)) => secure::connect(&mut ws, identity, &k, deadline, &quit).map(|t| (t, None)),
        (None, None) => unreachable!("a main key or a code"),
    };
    let (noise, learned) = match shook {
        Ok(done) => done,
        Err(f) => {
            finish(&mut ws);
            return Err(fail_message(&f, code.is_some()));
        }
    };
    let mut ch = Channel::new(ws, noise);
    let out = handshake(&mut ch, config, &**notify, stop, learned, deadline).and_then(|(main, mirror, caps)| {
        log::line("network", format!("connected to {main}"));
        notify.connected(&main);
        notify.mirror_offer(mirror);
        notify.main_caps(&caps);
        pump(&mut ch, exec, notify, stop)
    });
    finish(&mut ch.ws);
    out
}

/// `hello`, then `welcome`; returns the main's name, its mirror offer and its caps.
/// `learned` is the main's key from a pairing, kept only once welcomed.
fn handshake(ch: &mut Chan, config: &NetworkConfig, notify: &dyn ClientNotify, stop: &AtomicBool, learned: Option<[u8; 32]>, deadline: Instant) -> Result<(String, Option<bool>, Vec<String>), String> {
    let hostname = super::local_hostname();
    let hello = Up::Hello { protocol: PROTOCOL, app: env!("CARGO_PKG_VERSION").into(), name: display_name(&config.name, &hostname), hostname, platform: std::env::consts::OS.into(), caps: notify.caps(), brain_dormant_since: notify.brain_dormant_since() };
    send_up(ch, &hello)?;
    loop {
        match read_down(ch, stop, deadline)? {
            Down::Welcome { name, mirror, caps } => {
                // Commands carry attachments inline: allow what the protocol
                // allows, and the little sealing adds (`open_frame` caps the
                // text at `MAX_FRAME`).
                ch.ws.set_config(|c| {
                    c.max_message_size = Some(MAX_FRAME + MAX_FRAME / 1024);
                    c.max_frame_size = Some(MAX_FRAME + MAX_FRAME / 1024);
                });
                if let Some(key) = learned {
                    notify.paired(&encode_key(&key));
                }
                let name = clean(&name);
                return Ok((if name.is_empty() { "the main Maya".into() } else { name }, mirror, caps));
            }
            Down::Bye { reason } => return Err(bye_message(&reason)),
            other => log::line("network", format!("main: ignored {} before welcome", encode(&other).chars().take(40).collect::<String>())),
        }
    }
}

/// Runs a welcomed connection until it drops or `stop` is set.
fn pump(ch: &mut Chan, exec: &Arc<dyn Executor>, notify: &Arc<dyn ClientNotify>, stop: &AtomicBool) -> Result<(), String> {
    let (tx, rx) = channel::<Up>();
    let lane = notify.lane();
    let _attached = Attached(lane.as_ref().map(|l| (l.clone(), l.attach(tx.clone()))));
    // Set once this Maya has become the main: the client ends after it has written its answer.
    let leaving = Arc::new(AtomicBool::new(false));
    let mut last_read = Instant::now();
    let mut last_ping = Instant::now();
    // When the last board went out, and what it said.
    let mut last_board: Option<(Instant, String)> = None;
    // The first board goes out right after `welcome`.
    let mut board_wanted = true;
    let mut streams = Streams { terms: exec.live_terms(), subs: HashMap::new() };
    let mut tick = TICK;
    loop {
        if stopped(stop) {
            return Ok(());
        }
        // Read before draining, so a promote answer queued just before the
        // flag was set is written before the connection ends.
        let leave = leaving.load(Ordering::SeqCst);
        while let Ok(up) = rx.try_recv() {
            send_up(ch, &up)?;
        }
        if leave {
            log::line("network", "this Maya is the main now; the client ends");
            return Ok(());
        }
        // Checked on every turn: only sealed messages show the main is alive.
        if last_read.elapsed() > SILENCE {
            return Err("The main Maya stopped answering.".into());
        }
        board_wanted |= exec.board_requested();
        let since = last_board.as_ref().map(|(t, _)| t.elapsed());
        let refresh = since.map_or(true, |d| d >= BOARD_REFRESH);
        if since.map_or(true, |d| d >= BOARD_GAP) && (board_wanted || refresh) {
            board_wanted = false;
            let (cards, dirs) = exec.board();
            let frame = encode(&Up::Board { cards, dirs, agents: exec.agents(), remembered: exec.remembered() });
            if refresh || last_board.as_ref().map_or(true, |(_, f)| *f != frame) {
                ch.send(&frame).map_err(|e| format!("Lost the connection to the main Maya: {e}"))?;
                last_board = Some((Instant::now(), frame));
            }
        }
        if last_ping.elapsed() >= PING_EVERY {
            send_up(ch, &Up::Ping)?;
            last_ping = Instant::now();
        }
        // A live terminal on the link, either way, has its frames go out within `LIVE_TICK`.
        let want = live::tick_for(!streams.subs.is_empty() || notify.live(), TICK);
        if want != tick {
            let _ = ch.ws.get_ref().set_read_timeout(Some(want));
            tick = want;
        }
        match ch.read() {
            Ok(Got::Text(t)) => {
                last_read = Instant::now();
                match decode_down(&t) {
                    Ok(down) => handle(down, exec, notify, &lane, &tx, &leaving, &mut streams)?,
                    Err(e) => log::line("network", format!("main: ignored a message: {e}")),
                }
            }
            // The old main may close first once it has seen the answer.
            Ok(Got::Closed) if leaving.load(Ordering::SeqCst) => return Ok(()),
            Ok(Got::Closed) => return Err("The main Maya closed the connection.".into()),
            // Unauthenticated, so never proof the main is alive.
            Ok(Got::Control | Got::Idle) => {}
            Err(ChannelError::Forged) => {
                log::line("network", format!("main: {}", secure::FORGED));
                return Err(secure::FORGED.into());
            }
            Err(ChannelError::Io(_)) if leaving.load(Ordering::SeqCst) => return Ok(()),
            Err(ChannelError::Io(e)) => return Err(format!("Lost the connection to the main Maya: {e}")),
        }
    }
}

fn handle(down: Down, exec: &Arc<dyn Executor>, notify: &Arc<dyn ClientNotify>, lane: &Option<Arc<RequestLane>>, tx: &Sender<Up>, leaving: &Arc<AtomicBool>, streams: &mut Streams) -> Result<(), String> {
    match down {
        Down::Command { id, kind: CommandKind::Promote { list, summary } } => {
            log::line("network", format!("command {id} promote"));
            let (notify, out, leaving) = (notify.clone(), tx.clone(), leaving.clone());
            // On its own thread, as every command: becoming the main starts a server.
            let spawned = std::thread::Builder::new().name("net-promote".into()).spawn(move || {
                match notify.promoted(list, summary) {
                    Ok(port) => {
                        let _ = out.send(Up::Result { id, ok: true, error: None, data: Some(serde_json::json!({ "port": port })) });
                        leaving.store(true, Ordering::SeqCst);
                    }
                    Err(e) => {
                        log::line("network", format!("command {id} promote failed: {e}"));
                        let _ = out.send(Up::Result { id, ok: false, error: Some(e), data: None });
                    }
                }
            });
            if let Err(e) = spawned {
                let _ = tx.send(Up::Result { id, ok: false, error: Some(format!("no thread for the command: {e}")), data: None });
            }
        }
        Down::Command { id, kind } => {
            let name = super::server::kind_name(&kind);
            log::line("network", format!("command {id} {name}"));
            let (exec, out) = (exec.clone(), tx.clone());
            let spawned = std::thread::Builder::new().name("net-command".into()).spawn(move || {
                let up = match exec.execute(kind) {
                    Ok(data) => Up::Result { id, ok: true, error: None, data },
                    Err(e) => {
                        log::line("network", format!("command {id} {name} failed: {e}"));
                        Up::Result { id, ok: false, error: Some(e), data: None }
                    }
                };
                let _ = out.send(up);
            });
            if let Err(e) = spawned {
                let _ = tx.send(Up::Result { id, ok: false, error: Some(format!("no thread for the command: {e}")), data: None });
            }
        }
        Down::Bye { reason } => return Err(bye_message(&reason)),
        Down::Pong => {}
        Down::TermAttach { sub, session } => streams.attach(sub, &session, tx),
        Down::TermInput { sub, data } => streams.input(sub, &data, tx),
        Down::TermDetach { sub } => streams.detach(sub),
        Down::TermFrame { .. } => log::line("network", "main: terminal frame not handled yet"),
        Down::Move { host, port, key } => {
            let host = clean(&host);
            if decode_key(&key).is_err() {
                log::line("network", format!("the main moved to {host}:{port} with no usable key; not following"));
                return Err(format!("Could not follow the main Maya to {host}: {}", super::identity::BAD_KEY));
            }
            log::line("network", format!("the main moved to {host}:{port}"));
            notify.moved(&host, port, &key).map_err(|e| format!("Could not follow the main Maya to {host}: {e}"))?;
            return Err(MOVED.into());
        }
        Down::Mirror { allowed, cards, machines, remembered, wake_word } => {
            if allowed {
                notify.main_wake_word(wake_word.as_deref());
            }
            notify.mirrored_remembered(remembered);
            notify.mirrored(allowed, cards, machines);
        }
        Down::Response { id, ok, error, data } => match lane {
            Some(l) => l.resolve(id, if ok { Ok(data) } else { Err(error.unwrap_or_else(|| "The request failed.".into())) }),
            None => log::line("network", format!("main: response {id} with no request lane")),
        },
        Down::Welcome { .. } => log::line("network", "main: ignored a welcome after welcome"),
        Down::Chat { turn } => notify.chat_turn(turn),
        Down::Brain { status } => notify.brain_status(status),
    }
    Ok(())
}

/// The running client; `stop` ends its thread within a tick (within the
/// connect or lookup timeout while one is in flight).
#[derive(Clone)]
pub struct ClientHandle {
    stop: Arc<AtomicBool>,
    /// Set to ask the connection for a board (throttled there to one a second).
    pub board_due: Arc<AtomicBool>,
    thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl ClientHandle {
    /// Runs `body` on the client thread; it should return once its stop flag is set.
    pub fn spawn(body: impl FnOnce(Arc<AtomicBool>) + Send + 'static) -> ClientHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = match std::thread::Builder::new().name("net-client".into()).spawn(move || body(flag)) {
            Ok(t) => Some(t),
            Err(e) => {
                log::line("network", format!("no thread for the client: {e}"));
                None
            }
        };
        ClientHandle { stop, board_due: Arc::new(AtomicBool::new(false)), thread: Arc::new(Mutex::new(thread)) }
    }

    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            log::line("network", "client stopped");
        }
    }

    /// Stops the client and waits for its thread to end: by then its
    /// connection is closed, so the main has seen it go.
    pub fn stop_and_join(&self) {
        self.stop();
        let thread = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(t) = thread {
            let _ = t.join();
        }
    }
}

/// Pairs (`pair`) only once the running client, if any, has stopped and its
/// connection has closed. Otherwise the main would still count this machine
/// as connected and find the pairing's key on a connected entry.
pub fn pair_after_stopping<T>(running: Option<ClientHandle>, pair: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if let Some(c) = running {
        c.stop_and_join();
    }
    pair()
}

/// How long `reconnect` waits after a failure: none after a move, 30 s
/// after a main that could not prove it holds the pinned key, else the backoff.
fn retry_wait_ms(error: &str, attempt: u32) -> u64 {
    match error {
        MOVED => 0,
        KEY_CHANGED => UNPROVEN_WAIT_MS,
        _ => backoff_ms(attempt),
    }
}

/// Reconnects until `stop`, reading the config afresh each attempt.
pub fn reconnect(config_of: &dyn Fn() -> NetworkConfig, identity: Arc<Keypair>, exec: Arc<dyn Executor>, notify: Arc<dyn ClientNotify>, stop: &AtomicBool) {
    let mut attempt = 0u32;
    while !stopped(stop) {
        let config = config_of();
        log::line("network", format!("connecting to {}:{}", config.main_host.trim(), port_of(&config)));
        let began = Instant::now();
        match run_once(&config, &identity, exec.clone(), notify.clone(), stop, None) {
            Ok(()) => break,
            Err(e) if e == REMOVED || e == NOT_PAIRED => {
                log::line("network", format!("{e} Not reconnecting."));
                break;
            }
            Err(e) => {
                if began.elapsed() >= STEADY {
                    attempt = 0;
                }
                // Whatever answered at the main's address could not prove it
                // holds its key: do not hammer it.
                if e == MOVED {
                    attempt = 0;
                }
                let wait = retry_wait_ms(&e, attempt);
                if e != MOVED {
                    attempt = attempt.saturating_add(1);
                }
                log::line("network", format!("{e} Retrying in {} s.", wait / 1_000));
                let end = Instant::now() + Duration::from_millis(wait);
                while Instant::now() < end && !stopped(stop) {
                    std::thread::sleep(TICK);
                }
            }
        }
    }
}

/// The pairing handshake once. Ok((main_name, main_key)). One blocking
/// connection, at most `SILENCE` long once connected; it ends at
/// `welcome`, so no command reaches `exec`.
pub fn pair_with(exec: Arc<dyn Executor>, identity: Arc<Keypair>, host: &str, port: u16, name: &str, code: &str, caps: Vec<String>) -> Result<(String, String), String> {
    let config = NetworkConfig { role: NetworkRole::Assistant, main_host: host.trim().into(), main_port: port, name: name.trim().into(), ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(PairNotify { stop: stop.clone(), key: Mutex::new(None), main: Mutex::new(None), caps });
    log::line("network", format!("pairing with {}:{}", config.main_host, port_of(&config)));
    run_once(&config, &identity, exec, notify.clone(), &stop, Some(code)).map_err(|e| {
        log::line("network", format!("pairing failed: {e}"));
        e
    })?;
    let key = notify.key.lock().unwrap().take().ok_or("The main Maya did not pair this assistant.")?;
    let main = notify.main.lock().unwrap().take().unwrap_or_default();
    Ok((main, key))
}

/// Pairing's notifier: keeps the main's key and ends the connection at `welcome`.
struct PairNotify {
    stop: Arc<AtomicBool>,
    key: Mutex<Option<String>>,
    main: Mutex<Option<String>>,
    caps: Vec<String>,
}

impl ClientNotify for PairNotify {
    fn paired(&self, main_key: &str) {
        *self.key.lock().unwrap() = Some(main_key.into());
    }
    fn connected(&self, main_name: &str) {
        *self.main.lock().unwrap() = Some(main_name.into());
        self.stop.store(true, Ordering::SeqCst);
    }
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
    fn caps(&self) -> Vec<String> {
        self.caps.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::live::{EventSink, TermOpened};

    #[test]
    fn frames_from_the_main_are_small_until_welcome() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let main = std::thread::spawn(move || {
            let mut ws = tungstenite::accept(listener.accept().unwrap().0).unwrap();
            ws.send(tungstenite::Message::binary(vec![0u8; PRE_AUTH_FRAME + 1])).unwrap();
            ws
        });
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, ..Default::default() };
        let mut ws = connect(&config, &AtomicBool::new(false), Instant::now() + Duration::from_secs(5)).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        let err = loop {
            match ws.read() {
                Err(e) if secure::is_timeout(&e) && Instant::now() < end => {}
                other => break other,
            }
        };
        assert!(matches!(err, Err(tungstenite::Error::Capacity(_))), "{:?}", err.map(|m| m.len()));
        drop(main.join());
    }

    #[test]
    fn a_main_slower_than_a_read_tick_to_answer_the_upgrade_is_still_reached() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let main = std::thread::spawn(move || {
            let stream = listener.accept().unwrap().0;
            // The answer comes after the client's first read has timed out.
            std::thread::sleep(TICK * 3);
            tungstenite::accept(stream).map(drop)
        });
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, ..Default::default() };
        assert!(connect(&config, &AtomicBool::new(false), Instant::now() + Duration::from_secs(5)).is_ok(), "the upgrade completed");
        main.join().unwrap().unwrap();
    }

    #[test]
    fn a_move_retries_at_once() {
        assert_eq!(retry_wait_ms(MOVED, 4), 0);
        assert_eq!(retry_wait_ms(KEY_CHANGED, 0), 30_000);
        assert_eq!(retry_wait_ms("Lost the connection", 2), 4_000);
    }

    #[test]
    fn a_refusal_shows_only_the_messages_this_maya_knows() {
        let refused = |r: &str| fail_message(&Fail::Refused(r.into()), true);
        assert_eq!(refused(secure::WRONG_CODE_REASON), WRONG_CODE);
        assert_eq!(refused("too many attempts"), TOO_MANY_CODES);
        assert_eq!(refused(secure::UNKNOWN_KEY_REASON), KEY_CHANGED);
        assert_eq!(refused("removed"), REFUSED);
        assert_eq!(refused("Visit evil.example to fix this"), REFUSED, "an unknown reason is never echoed");
        assert_eq!(fail_message(&Fail::OldPeer, false), OLD_MAIN);
        assert_eq!(fail_message(&Fail::Crypto, true), PAIRING_FAILED);
        assert_eq!(fail_message(&Fail::Crypto, false), KEY_CHANGED);
        assert_eq!(fail_message(&Fail::Io("stopped".into()), false), "stopped");
        assert_eq!(fail_message(&Fail::Io("timed out".into()), false), NO_ANSWER);
        assert_eq!(fail_message(&Fail::Malformed, true), NO_ANSWER);
    }

    #[test]
    fn backoff_doubles_to_sixteen_seconds_then_every_thirty() {
        assert_eq!((0..7).map(backoff_ms).collect::<Vec<_>>(), [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
    }

    #[test]
    fn remote_reply_rewrites_attachment_paths() {
        let text = "look at this\nAttached file: /Users/main/.claude/maya/attachments/a.png";
        let out = rewrite_attachments(text, &[("/Users/main/.claude/maya/attachments/a.png".into(), std::path::PathBuf::from("/Users/asst/.claude/maya/attachments/1-a.png"))]);
        assert_eq!(out, "look at this\nAttached file: /Users/asst/.claude/maya/attachments/1-a.png");
    }

    #[test]
    fn attachments_are_saved_only_for_a_live_session_and_removed_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let files = || std::fs::read_dir(dir.path().join("attachments")).map(|d| d.count()).unwrap_or(0);
        let file = |name: &str, bytes: &str| Attachment { name: name.into(), bytes: bytes.into() };
        let good = || file("/main/a.png", "aGVsbG8=");
        // A session that is gone: nothing is written, nothing is sent.
        let out: Result<(), _> = reply_with_attachments(dir.path(), false, "hi", vec![good()], 1, |_| panic!("not sent"));
        assert_eq!(out, Err("Session is no longer running.".to_string()));
        assert_eq!(files(), 0);
        // The second attachment is unreadable: the first is deleted again.
        let out: Result<(), _> = reply_with_attachments(dir.path(), true, "hi", vec![good(), file("/main/b.png", "not base64!")], 2, |_| panic!("not sent"));
        assert_eq!(out, Err("The attachment /main/b.png could not be read.".to_string()));
        assert_eq!(files(), 0);
        // The reply itself fails: the saved files go too.
        let out: Result<(), _> = reply_with_attachments(dir.path(), true, "hi", vec![good()], 3, |_| Err("no inbox".into()));
        assert_eq!(out, Err("no inbox".to_string()));
        assert_eq!(files(), 0);
        // All good: the files stay and the text points at them.
        let mut sent = String::new();
        let out = reply_with_attachments(dir.path(), true, "look\nAttached file: /main/a.png", vec![good()], 4, |t| {
            sent = t;
            Ok(())
        });
        assert_eq!(out, Ok(()));
        assert_eq!(files(), 1);
        assert_eq!(sent, format!("look\nAttached file: {}", dir.path().join("attachments").join("4-a.png").display()));
    }

    #[test]
    fn pairing_waits_for_the_running_client_to_end() {
        let log = Arc::new(Mutex::new(Vec::<&str>::new()));
        let l = log.clone();
        let running = ClientHandle::spawn(move |stop| {
            while !stopped(&stop) {
                std::thread::sleep(Duration::from_millis(5));
            }
            // As `finish` does: the socket closes a moment after the stop.
            std::thread::sleep(Duration::from_millis(50));
            l.lock().unwrap().push("client ended");
        });
        let out = pair_after_stopping(Some(running), || {
            log.lock().unwrap().push("paired");
            Ok::<_, String>(7)
        });
        assert_eq!(out, Ok(7));
        assert_eq!(*log.lock().unwrap(), ["client ended", "paired"]);
        assert_eq!(pair_after_stopping(None, || Ok::<_, String>(1)), Ok(1), "no client running: it just pairs");
    }

    #[test]
    fn a_name_lookup_has_a_deadline() {
        let addrs = resolve("127.0.0.1", 4127).unwrap();
        assert_eq!(addrs, [SocketAddr::from(([127, 0, 0, 1], 4127))]);
        let began = Instant::now();
        let slow = resolve_with("desk.local", Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(500));
            Ok(vec![])
        });
        assert_eq!(slow, Err("Could not find desk.local: the lookup took more than 0 s.".to_string()));
        assert!(began.elapsed() < Duration::from_millis(400), "it does not wait for the lookup");
        let failed = resolve_with("nowhere", Duration::from_secs(1), || Err(std::io::Error::other("no such host")));
        assert_eq!(failed, Err("Could not find nowhere: no such host".to_string()));
    }

    #[test]
    fn chat_and_brain_frames_reach_the_app() {
        struct Quiet;
        impl Executor for Quiet {
            fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
                Ok(None)
            }
            fn board(&self) -> (Vec<Card>, Vec<String>) {
                (vec![], vec![])
            }
        }
        #[derive(Default)]
        struct Recorder(Mutex<Vec<(&'static str, Value)>>);
        impl ClientNotify for Recorder {
            fn paired(&self, _: &str) {}
            fn connected(&self, _: &str) {}
            fn disconnected(&self, _: &str) {}
            fn removed(&self) {}
            fn chat_turn(&self, turn: Value) {
                self.0.lock().unwrap().push(("chat", turn));
            }
            fn brain_status(&self, status: Value) {
                self.0.lock().unwrap().push(("brain", status));
            }
        }
        let recorder = Arc::new(Recorder::default());
        let (exec, notify): (Arc<dyn Executor>, Arc<dyn ClientNotify>) = (Arc::new(Quiet), recorder.clone());
        let (tx, _rx) = channel();
        let leaving = Arc::new(AtomicBool::new(false));
        let turn = serde_json::json!({"id": 3, "role": "assistant", "text": "Done."});
        let status = serde_json::json!({"dot": "working", "pending": null, "job": "x", "ready": true});
        for down in [Down::Chat { turn: turn.clone() }, Down::Brain { status: status.clone() }] {
            handle(down, &exec, &notify, &None, &tx, &leaving, &mut Streams { terms: None, subs: HashMap::new() }).unwrap();
        }
        assert_eq!(*recorder.0.lock().unwrap(), [("chat", turn), ("brain", status)]);
    }

    /// A host that records what the network asks of it and keeps the sink it is given.
    #[derive(Default)]
    struct FakeTerms {
        sink: Mutex<Option<EventSink>>,
        inputs: Mutex<Vec<(TermView, Vec<u8>)>>,
        detached: Mutex<Vec<TermView>>,
        fail: bool,
    }

    const VIEW: TermView = TermView { term: 1, viewer: 2 };

    impl LiveTerms for FakeTerms {
        fn attach(&self, _session: &str, sink: EventSink) -> Result<TermOpened, String> {
            if self.fail {
                return Err("This session is not running in Maya.".into());
            }
            *self.sink.lock().unwrap() = Some(sink);
            Ok(TermOpened { view: VIEW, snapshot: b"hi".to_vec(), rows: 40, cols: 120 })
        }
        fn input(&self, view: &TermView, data: &[u8]) -> Result<(), String> {
            self.inputs.lock().unwrap().push((*view, data.to_vec()));
            Ok(())
        }
        fn detach(&self, view: &TermView) {
            self.detached.lock().unwrap().push(*view);
        }
    }

    /// An assistant with no sessions, serving `terms` when it has them.
    struct Hosting(Option<Arc<FakeTerms>>);

    impl Executor for Hosting {
        fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
            Ok(None)
        }
        fn board(&self) -> (Vec<Card>, Vec<String>) {
            (vec![], vec![])
        }
        fn live_terms(&self) -> Option<Arc<dyn LiveTerms>> {
            self.0.clone().map(|t| t as Arc<dyn LiveTerms>)
        }
    }

    struct Silent;

    impl ClientNotify for Silent {
        fn paired(&self, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, _: &str) {}
        fn removed(&self) {}
    }

    /// A welcomed link over loopback: the assistant's end runs `pump` on a
    /// thread until `stop`; the main's end is returned for the test to drive.
    fn serving(terms: Option<Arc<FakeTerms>>, stop: Arc<AtomicBool>) -> (Chan, std::thread::JoinHandle<Result<(), String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let joiner = std::thread::spawn(move || {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(TICK)).unwrap();
            tungstenite::client::client(format!("ws://127.0.0.1:{port}/"), stream).unwrap().0
        });
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(TICK)).unwrap();
        let main = tungstenite::accept(stream).unwrap();
        let (a, m) = secure::transport_pair();
        let mut assistant = Channel::new(joiner.join().unwrap(), a);
        let (exec, notify): (Arc<dyn Executor>, Arc<dyn ClientNotify>) = (Arc::new(Hosting(terms)), Arc::new(Silent));
        let run = std::thread::spawn(move || pump(&mut assistant, &exec, &notify, &stop));
        (Channel::new(main, m), run)
    }

    fn send_down(main: &mut Chan, down: Down) {
        main.send(&encode(&down)).unwrap();
    }

    /// The next terminal frame the assistant sent; boards and pings are skipped.
    fn next_frame(main: &mut Chan) -> (u64, TermFrame) {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if let Ok(Got::Text(t)) = main.read() {
                if let Ok(Up::TermFrame { sub, frame }) = crate::net::protocol::decode_up(&t) {
                    return (sub, frame);
                }
            }
        }
        panic!("no terminal frame within 5 s");
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "{what} within 5 s");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn an_assistant_streams_a_live_terminal_to_the_main() {
        let terms = Arc::new(FakeTerms::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (mut main, run) = serving(Some(terms.clone()), stop.clone());
        send_down(&mut main, Down::TermAttach { sub: 5, session: "s1".into() });
        assert_eq!(next_frame(&mut main), (5, TermFrame { seq: 0, event: TermEvent::Attached { snapshot: "aGk=".into(), rows: 40, cols: 120 } }));
        assert!(terms.sink.lock().unwrap().as_ref().unwrap()(TermEvent::Output { data: "eA==".into() }));
        assert_eq!(next_frame(&mut main), (5, TermFrame { seq: 1, event: TermEvent::Output { data: "eA==".into() } }));
        send_down(&mut main, Down::TermInput { sub: 5, data: "aGk=".into() });
        wait_for("the keys", || !terms.inputs.lock().unwrap().is_empty());
        assert_eq!(*terms.inputs.lock().unwrap(), [(VIEW, b"hi".to_vec())]);
        send_down(&mut main, Down::TermDetach { sub: 5 });
        wait_for("the detach", || !terms.detached.lock().unwrap().is_empty());
        assert_eq!(*terms.detached.lock().unwrap(), [VIEW]);
        stop.store(true, Ordering::SeqCst);
        assert_eq!(run.join().unwrap(), Ok(()));
        assert_eq!(*terms.detached.lock().unwrap(), [VIEW], "a detached view is not detached again");
    }

    #[test]
    fn an_unknown_session_or_no_terminals_answers_with_an_error_frame() {
        let error = |message: &str| TermFrame { seq: 0, event: TermEvent::Error { message: message.into() } };
        let terms = Arc::new(FakeTerms { fail: true, ..Default::default() });
        let stop = Arc::new(AtomicBool::new(false));
        let (mut main, run) = serving(Some(terms.clone()), stop.clone());
        send_down(&mut main, Down::TermAttach { sub: 5, session: "gone".into() });
        assert_eq!(next_frame(&mut main), (5, error("This session is not running in Maya.")));
        // Nothing was stored for it, so its keys have nowhere to go.
        send_down(&mut main, Down::TermInput { sub: 5, data: "aGk=".into() });
        assert_eq!(next_frame(&mut main), (5, error(NO_SUCH_TERMINAL)));
        assert!(terms.inputs.lock().unwrap().is_empty());
        stop.store(true, Ordering::SeqCst);
        assert_eq!(run.join().unwrap(), Ok(()));
        assert!(terms.detached.lock().unwrap().is_empty(), "nothing was attached");

        let stop = Arc::new(AtomicBool::new(false));
        let (mut main, run) = serving(None, stop.clone());
        send_down(&mut main, Down::TermAttach { sub: 6, session: "s1".into() });
        assert_eq!(next_frame(&mut main), (6, error("This Maya has no live terminals.")));
        stop.store(true, Ordering::SeqCst);
        assert_eq!(run.join().unwrap(), Ok(()));
    }

    #[test]
    fn keys_that_do_not_decode_end_the_view() {
        let terms = Arc::new(FakeTerms::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (mut main, run) = serving(Some(terms.clone()), stop.clone());
        send_down(&mut main, Down::TermAttach { sub: 5, session: "s1".into() });
        assert_eq!(next_frame(&mut main).1.seq, 0);
        send_down(&mut main, Down::TermInput { sub: 5, data: "not base64!".into() });
        assert_eq!(next_frame(&mut main), (5, TermFrame { seq: 0, event: TermEvent::Error { message: UNREADABLE_KEYS.into() } }));
        wait_for("the detach", || !terms.detached.lock().unwrap().is_empty());
        assert!(terms.inputs.lock().unwrap().is_empty());
        stop.store(true, Ordering::SeqCst);
        assert_eq!(run.join().unwrap(), Ok(()));
        assert_eq!(*terms.detached.lock().unwrap(), [VIEW], "detached once, when the error went out");
    }

    #[test]
    fn attaching_a_sub_again_detaches_its_old_view_first() {
        let terms = Arc::new(FakeTerms::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (mut main, run) = serving(Some(terms.clone()), stop.clone());
        send_down(&mut main, Down::TermAttach { sub: 5, session: "s1".into() });
        assert_eq!(next_frame(&mut main).1.seq, 0);
        send_down(&mut main, Down::TermAttach { sub: 5, session: "s1".into() });
        assert_eq!(next_frame(&mut main).1.seq, 0);
        assert_eq!(*terms.detached.lock().unwrap(), [VIEW], "the old view went before the new one came");
        stop.store(true, Ordering::SeqCst);
        assert_eq!(run.join().unwrap(), Ok(()));
        assert_eq!(*terms.detached.lock().unwrap(), [VIEW, VIEW]);
    }

    #[test]
    fn the_pump_detaches_its_viewers_when_the_link_ends() {
        let terms = Arc::new(FakeTerms::default());
        let (mut main, run) = serving(Some(terms.clone()), Arc::new(AtomicBool::new(false)));
        send_down(&mut main, Down::TermAttach { sub: 5, session: "s1".into() });
        assert_eq!(next_frame(&mut main).1.seq, 0);
        drop(main);
        assert!(run.join().unwrap().is_err(), "the link dropped");
        assert_eq!(*terms.detached.lock().unwrap(), [VIEW]);
        // The pump has ended: the host hears its viewer is gone.
        assert!(!terms.sink.lock().unwrap().as_ref().unwrap()(TermEvent::Exit));
    }

    #[test]
    fn a_hello_carries_the_name_or_the_hostname() {
        assert_eq!(display_name("", "tiagos-mbp"), "tiagos-mbp");
        assert_eq!(display_name("  laptop ", "tiagos-mbp"), "laptop");
    }

    #[test]
    fn a_pairing_form_needs_a_host_and_six_digits() {
        assert_eq!(check_pair_args(" desk.local ", "483 921"), Ok(("desk.local".into(), "483921".into())));
        assert_eq!(check_pair_args("", "483921").unwrap_err(), "Type the main Maya's host name or address.");
        assert_eq!(check_pair_args("http://desk", "483921").unwrap_err(), "Type the main Maya's host name or address.");
        assert_eq!(check_pair_args("desk", "48392").unwrap_err(), "The pairing code has six digits.");
    }
}
