//! The main Maya's server: a WebSocket listener that pairs assistants, keeps
//! their board snapshots and sends them commands.
//!
//! Threads: one accept loop, and one thread per connection that owns its
//! socket. The socket's read timeout (`TICK`) is the loop's tick: each turn
//! writes the `Down` frames queued on the connection's channel, then tries
//! one read. A separate deadline (`SILENCE`) drops a peer that sends nothing,
//! and the stop flag ends every thread within a tick. Dropping a
//! connection's sender (a newer connection for the same assistant, or its
//! removal) ends that connection too.
//!
//! Locks: `Server` sits behind one mutex held only for bookkeeping. `Notify`
//! is never called with it held, so the Tauri adapter may take the app's own
//! locks (network, then server, then store) without a cycle.

use super::merge::{display_names, RemoteBoard};
use super::protocol::{decode_up, encode, mac_matches, new_id, new_nonce, new_token, Attempts, CommandKind, Down, NonceLog, PairingWindow, Up, MAX_FRAME, PROTOCOL};
use super::{AssistantStatus, NetworkStatus, PairingCode};
use crate::config::{NetworkConfig, NetworkRole, PairedAssistant};
use crate::log;
use crate::store::now_ms;
use serde_json::Value;
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

/// How long one read waits; the connection loop's tick.
const TICK: Duration = Duration::from_millis(100);
/// A peer silent this long is dropped (assistants ping every 10 s).
const SILENCE: Duration = Duration::from_secs(15);
/// Connections beyond this are closed at once.
const MAX_CONNECTIONS: usize = 32;
const DISCONNECTED: &str = "assistant disconnected";

/// What the server tells the app; the Tauri adapter lives in `lib.rs`.
pub trait Notify: Send + Sync {
    fn board_changed(&self);
    fn status_changed(&self, status: NetworkStatus);
    fn paired(&self, assistant: &PairedAssistant);
    fn paired_list_changed(&self, assistants: &[PairedAssistant]);
}

struct Conn {
    tx: Sender<Down>,
    serial: u64,
}

struct Pending {
    conn: u64,
    tx: Sender<Result<Option<Value>, String>>,
}

/// What an assistant said about itself in its last `hello`.
struct Peer {
    name: String,
    hostname: String,
    platform: String,
    last_seen: u64,
}

pub struct Server {
    /// Every paired assistant's last snapshot, `machine` set to its display name.
    pub boards: Vec<RemoteBoard>,
    pub pairing: Option<PairingWindow>,
    by_id: HashMap<String, RemoteBoard>,
    conns: HashMap<String, Conn>,
    pending: HashMap<u64, Pending>,
    next_id: u64,
    attempts: Attempts,
    nonces: NonceLog,
    paired: Vec<PairedAssistant>,
    peers: HashMap<String, Peer>,
}

impl Server {
    fn new(paired: Vec<PairedAssistant>) -> Server {
        Server { boards: vec![], pairing: None, by_id: HashMap::new(), conns: HashMap::new(), pending: HashMap::new(), next_id: 0, attempts: Attempts::default(), nonces: NonceLog::default(), paired, peers: HashMap::new() }
    }

    fn next(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn is_paired(&self, id: &str) -> bool {
        self.paired.iter().any(|p| p.id == id)
    }

    /// Each paired assistant's id and display name, `name (hostname)` when two share a name.
    fn labels(&self) -> Vec<(String, String)> {
        let pairs: Vec<(String, String)> = self.paired.iter().map(|p| self.peers.get(&p.id).map_or((p.name.clone(), p.hostname.clone()), |x| (x.name.clone(), x.hostname.clone()))).collect();
        self.paired.iter().map(|p| p.id.clone()).zip(display_names(&pairs)).collect()
    }

    fn label(&self, id: &str) -> String {
        self.labels().into_iter().find(|(i, _)| i == id).map(|(_, l)| l).unwrap_or_else(|| id.to_string())
    }

    fn id_for(&self, machine: &str) -> Option<String> {
        self.labels().into_iter().find(|(_, l)| l == machine).map(|(i, _)| i)
    }

    fn host_of(&self, id: &str) -> (String, String) {
        match (self.peers.get(id), self.paired.iter().find(|p| p.id == id)) {
            (Some(x), _) => (x.hostname.clone(), x.platform.clone()),
            (None, Some(p)) => (p.hostname.clone(), p.platform.clone()),
            _ => (String::new(), String::new()),
        }
    }

    fn touch(&mut self, id: &str) {
        if let Some(p) = self.peers.get_mut(id) {
            p.last_seen = now_ms();
        }
    }

    /// Rebuilds `boards` from the snapshots, relabelled and sorted by machine.
    fn sync_boards(&mut self) {
        let labels = self.labels();
        let mut boards: Vec<RemoteBoard> = labels
            .iter()
            .filter_map(|(id, label)| {
                self.by_id.get(id).map(|b| RemoteBoard { machine: label.clone(), ..b.clone() })
            })
            .collect();
        boards.sort_by(|a, b| a.machine.cmp(&b.machine));
        self.boards = boards;
    }

    fn fail_pending(&mut self, serial: u64) {
        self.pending.retain(|_, p| {
            if p.conn == serial {
                let _ = p.tx.send(Err(DISCONNECTED.into()));
                false
            } else {
                true
            }
        });
    }

    pub fn status(&self, now_ms: u64) -> NetworkStatus {
        let code = self.pairing.as_ref().filter(|w| now_ms <= w.expires_at_ms).map(|w| PairingCode { code: w.code.clone(), expires_at: w.expires_at_ms });
        let labels = self.labels();
        let assistants = self
            .paired
            .iter()
            .zip(labels)
            .map(|(p, (_, name))| {
                let (hostname, platform) = self.host_of(&p.id);
                AssistantStatus { id: p.id.clone(), name, hostname, platform, connected: self.conns.contains_key(&p.id), last_seen: self.peers.get(&p.id).map(|x| x.last_seen) }
            })
            .collect();
        NetworkStatus { role: NetworkRole::Main, code, assistants, assistant: Default::default() }
    }
}

/// The server's mutex, recovered if a thread panicked while holding it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Ctx {
    shared: Arc<Mutex<Server>>,
    notify: Arc<dyn Notify>,
    config_view: Arc<Mutex<NetworkConfig>>,
    stop: Arc<AtomicBool>,
    main_name: String,
    live: AtomicUsize,
}

impl Ctx {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn announce(&self) {
        if self.stopped() {
            return;
        }
        let status = lock(&self.shared).status(now_ms());
        self.notify.status_changed(status);
    }
}

#[derive(Clone)]
pub struct ServerHandle {
    pub shared: Arc<Mutex<Server>>,
    stop: Arc<AtomicBool>,
    port: u16,
    ctx: Arc<Ctx>,
}

impl ServerHandle {
    /// Opens a fresh pairing window (a new code even when one is open).
    pub fn open_pairing(&self, now_ms: u64) -> (String, u64) {
        let (code, expires, status) = {
            let mut s = lock(&self.shared);
            let w = PairingWindow::new(now_ms);
            let out = (w.code.clone(), w.expires_at_ms);
            s.pairing = Some(w);
            (out.0, out.1, s.status(now_ms))
        };
        log::line("network", "pairing open for five minutes");
        self.ctx.notify.status_changed(status);
        (code, expires)
    }

    /// Forgets an assistant: it is told `bye removed`, its cards go, and its
    /// token no longer opens a connection.
    pub fn remove_assistant(&self, id: &str) {
        let (list, label, status) = {
            let mut s = lock(&self.shared);
            let label = s.label(id);
            s.paired.retain(|p| p.id != id);
            if let Some(c) = s.conns.remove(id) {
                let _ = c.tx.send(Down::Bye { reason: "removed".into() });
            }
            s.by_id.remove(id);
            s.peers.remove(id);
            s.sync_boards();
            (s.paired.clone(), label, s.status(now_ms()))
        };
        lock(&self.ctx.config_view).assistants.retain(|p| p.id != id);
        log::line("network", format!("{label}: removed"));
        self.ctx.notify.paired_list_changed(&list);
        self.ctx.notify.status_changed(status);
        self.ctx.notify.board_changed();
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn status(&self) -> NetworkStatus {
        lock(&self.shared).status(now_ms())
    }

    pub fn boards(&self) -> Vec<RemoteBoard> {
        lock(&self.shared).boards.clone()
    }

    /// Stops accepting and closes every connection within a tick; does not wait.
    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            log::line("network", "server stopped");
        }
    }
}

/// Starts the server with the Tauri adapter and the store's paired list.
pub fn start(app: AppHandle, port: u16) -> Result<ServerHandle, String> {
    let view = app.state::<crate::AppState>().store.lock().unwrap().config.network.clone();
    start_with(Arc::new(crate::TauriNetNotify { app }), Arc::new(Mutex::new(view)), port)
}

/// Binds `port` on every interface (0 picks a free one) and starts accepting.
pub fn start_with(notify: Arc<dyn Notify>, config_view: Arc<Mutex<NetworkConfig>>, port: u16) -> Result<ServerHandle, String> {
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("Could not listen on port {port}: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let paired = lock(&config_view).assistants.clone();
    let shared = Arc::new(Mutex::new(Server::new(paired)));
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(Ctx { shared: shared.clone(), notify, config_view, stop: stop.clone(), main_name: super::local_hostname(), live: AtomicUsize::new(0) });
    let c = ctx.clone();
    std::thread::Builder::new().name("net-accept".into()).spawn(move || accept_loop(listener, c)).map_err(|e| e.to_string())?;
    log::line("network", format!("listening on port {port}"));
    Ok(ServerHandle { shared, stop, port, ctx })
}

fn accept_loop(listener: TcpListener, ctx: Arc<Ctx>) {
    while !ctx.stopped() {
        match listener.accept() {
            Ok((stream, addr)) => {
                if ctx.live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    log::line("network", format!("{addr}: refused, too many connections"));
                    continue;
                }
                ctx.live.fetch_add(1, Ordering::SeqCst);
                let c = ctx.clone();
                let spawned = std::thread::Builder::new().name("net-conn".into()).spawn(move || {
                    serve(&c, stream, addr);
                    c.live.fetch_sub(1, Ordering::SeqCst);
                });
                if let Err(e) = spawned {
                    ctx.live.fetch_sub(1, Ordering::SeqCst);
                    log::line("network", format!("{addr}: no thread for the connection: {e}"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(TICK),
            Err(e) => {
                log::line("network", format!("accept failed: {e}"));
                std::thread::sleep(TICK);
            }
        }
    }
}

type Ws = WebSocket<TcpStream>;

/// A connection that got through `hello` and pairing or authentication.
struct Link {
    id: String,
    serial: u64,
    label: String,
    rx: Receiver<Down>,
}

fn serve(ctx: &Ctx, stream: TcpStream, addr: SocketAddr) {
    // Accepted sockets inherit the listener's non-blocking flag on macOS.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(TICK));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);
    let mut ws = match upgrade(ctx, stream) {
        Ok(ws) => ws,
        Err(e) => {
            log::line("network", format!("{addr}: no WebSocket handshake: {e}"));
            return;
        }
    };
    let Some(link) = admit(ctx, &mut ws, addr) else {
        finish(&mut ws);
        return;
    };
    let reason = pump(ctx, &mut ws, &link);
    finish(&mut ws);
    leave(ctx, &link, &reason);
}

fn upgrade(ctx: &Ctx, stream: TcpStream) -> Result<Ws, String> {
    let config = WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME));
    let deadline = Instant::now() + SILENCE;
    let mut attempt = tungstenite::accept_with_config(stream, Some(config));
    loop {
        match attempt {
            Ok(ws) => return Ok(ws),
            Err(HandshakeError::Interrupted(mid)) => {
                if ctx.stopped() || Instant::now() > deadline {
                    return Err("timed out".into());
                }
                attempt = mid.handshake();
            }
            Err(HandshakeError::Failure(e)) => return Err(e.to_string()),
        }
    }
}

fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn send_down(ws: &mut Ws, down: &Down) -> Result<(), String> {
    ws.send(Message::text(encode(down))).map_err(|e| format!("write failed: {e}"))
}

fn bye(ws: &mut Ws, reason: &str) {
    let _ = send_down(ws, &Down::Bye { reason: reason.into() });
}

/// Closes the WebSocket and waits up to a second for the peer to agree, so
/// a `bye` just sent is read before the socket goes.
fn finish(ws: &mut Ws) {
    let _ = ws.close(None);
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        match ws.read() {
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {}
            Err(_) => break,
        }
    }
}

/// The next frame that parses; garbage is logged and skipped.
fn read_up(ctx: &Ctx, ws: &mut Ws, who: &str) -> Result<Up, String> {
    let deadline = Instant::now() + SILENCE;
    loop {
        if ctx.stopped() {
            return Err("the main stopped".into());
        }
        match ws.read() {
            Ok(Message::Text(t)) => match decode_up(&t) {
                Ok(up) => return Ok(up),
                Err(e) => log::line("network", format!("{who}: ignored a message: {e}")),
            },
            Ok(Message::Close(_)) => return Err("closed".into()),
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {
                if Instant::now() > deadline {
                    return Err("silent for 15 s".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Names from the peer, trimmed of control characters and kept short.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(64).collect()
}

fn admit(ctx: &Ctx, ws: &mut Ws, addr: SocketAddr) -> Option<Link> {
    let ip = addr.ip().to_string();
    let hello = match read_up(ctx, ws, &ip) {
        Ok(up) => up,
        Err(e) => {
            log::line("network", format!("{ip}: no hello: {e}"));
            return None;
        }
    };
    let Up::Hello { protocol, name, hostname, platform, id, .. } = hello else {
        log::line("network", format!("{ip}: first message was not a hello"));
        bye(ws, "protocol");
        return None;
    };
    if protocol != PROTOCOL {
        log::line("network", format!("{ip}: protocol {protocol}, expected {PROTOCOL}"));
        bye(ws, "protocol");
        return None;
    }
    let (hostname, platform) = (clean(&hostname), clean(&platform));
    let name = Some(clean(&name)).filter(|n| !n.is_empty()).or_else(|| Some(hostname.clone()).filter(|h| !h.is_empty())).unwrap_or_else(|| "assistant".into());
    let id = match id {
        Some(id) => {
            authenticate(ctx, ws, &id, &name)?;
            id
        }
        None => pair(ctx, ws, &ip, &name, &hostname, &platform)?,
    };
    let (tx, rx) = channel();
    let registered = {
        let mut s = lock(&ctx.shared);
        if s.is_paired(&id) {
            let serial = s.next();
            s.peers.insert(id.clone(), Peer { name, hostname, platform, last_seen: now_ms() });
            // A newer connection replaces an older one; dropping its sender ends it.
            s.conns.insert(id.clone(), Conn { tx, serial });
            if let Some(b) = s.by_id.get_mut(&id) {
                b.connected = true;
            }
            s.sync_boards();
            Some((serial, s.label(&id)))
        } else {
            None
        }
    };
    let Some((serial, label)) = registered else {
        bye(ws, "removed");
        return None;
    };
    let link = Link { id, serial, label, rx };
    if let Err(e) = send_down(ws, &Down::Welcome { name: ctx.main_name.clone() }) {
        leave(ctx, &link, &e);
        return None;
    }
    log::line("network", format!("{}: connected from {ip}", link.label));
    ctx.announce();
    ctx.notify.board_changed();
    Some(link)
}

fn authenticate(ctx: &Ctx, ws: &mut Ws, id: &str, name: &str) -> Option<()> {
    let token = lock(&ctx.shared).paired.iter().find(|p| p.id == id).map(|p| p.token.clone());
    let Some(token) = token else {
        // Unknown ids are removed ones (or paired with another main): pairing again is the fix.
        log::line("network", format!("{name}: unknown assistant; told it was removed"));
        bye(ws, "removed");
        return None;
    };
    let nonce = new_nonce();
    send_down(ws, &Down::Challenge { nonce: nonce.clone() }).ok()?;
    let ok = match read_up(ctx, ws, name) {
        Ok(Up::Auth { mac }) => mac_matches(&token, &nonce, &mac) && lock(&ctx.shared).nonces.first_use(&nonce, now_ms()),
        Ok(_) => false,
        Err(e) => {
            log::line("network", format!("{name}: no answer to the challenge: {e}"));
            return None;
        }
    };
    if !ok {
        log::line("network", format!("{name}: authentication failed"));
        bye(ws, "authentication failed");
        return None;
    }
    Some(())
}

fn pair(ctx: &Ctx, ws: &mut Ws, ip: &str, name: &str, hostname: &str, platform: &str) -> Option<String> {
    let code = match read_up(ctx, ws, ip) {
        Ok(Up::Pair { code }) => code,
        Ok(_) => {
            bye(ws, "protocol");
            return None;
        }
        Err(e) => {
            log::line("network", format!("{ip}: no pairing code: {e}"));
            return None;
        }
    };
    let now = now_ms();
    let outcome = {
        let mut s = lock(&ctx.shared);
        if s.attempts.locked(ip, now) {
            Err("too many attempts")
        } else if !s.pairing.as_ref().is_some_and(|w| w.accepts(&code, now)) {
            s.attempts.failed(ip, now);
            Err("wrong or expired pairing code")
        } else {
            let a = PairedAssistant { id: new_id(), name: name.into(), hostname: hostname.into(), platform: platform.into(), token: new_token() };
            s.paired.push(a.clone());
            // One code, one assistant.
            s.pairing = None;
            Ok(a)
        }
    };
    match outcome {
        Err(reason) => {
            log::line("network", format!("{ip}: pairing refused: {reason}"));
            bye(ws, reason);
            None
        }
        Ok(a) => {
            lock(&ctx.config_view).assistants.push(a.clone());
            ctx.notify.paired(&a);
            log::line("network", format!("{name}: paired from {ip}"));
            send_down(ws, &Down::Paired { id: a.id.clone(), token: a.token.clone() }).ok()?;
            Some(a.id)
        }
    }
}

/// Runs a registered connection until it closes; returns why.
fn pump(ctx: &Ctx, ws: &mut Ws, link: &Link) -> String {
    let mut deadline = Instant::now() + SILENCE;
    loop {
        if ctx.stopped() {
            return "the main stopped".into();
        }
        loop {
            match link.rx.try_recv() {
                Ok(down) => {
                    let last = matches!(down, Down::Bye { .. });
                    if let Err(e) = send_down(ws, &down) {
                        return e;
                    }
                    if last {
                        return "removed".into();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return "replaced by a newer connection".into(),
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                deadline = Instant::now() + SILENCE;
                match decode_up(&t) {
                    Ok(up) => {
                        if let Err(e) = handle(ctx, ws, link, up) {
                            return e;
                        }
                    }
                    Err(e) => log::line("network", format!("{}: ignored a message: {e}", link.label)),
                }
            }
            Ok(Message::Close(_)) => return "closed by the assistant".into(),
            Ok(_) => deadline = Instant::now() + SILENCE,
            Err(e) if is_timeout(&e) => {
                if Instant::now() > deadline {
                    return "silent for 15 s".into();
                }
            }
            Err(e) => return e.to_string(),
        }
    }
}

fn handle(ctx: &Ctx, ws: &mut Ws, link: &Link, up: Up) -> Result<(), String> {
    match up {
        Up::Board { cards, dirs } => {
            let current = {
                let mut s = lock(&ctx.shared);
                let current = s.conns.get(&link.id).is_some_and(|c| c.serial == link.serial);
                if current {
                    s.touch(&link.id);
                    let (hostname, platform) = s.host_of(&link.id);
                    let now = now_ms();
                    s.by_id.insert(link.id.clone(), RemoteBoard { machine: String::new(), hostname, platform, cards, dirs, received_at: now, connected: true });
                    s.sync_boards();
                }
                current
            };
            if !current {
                return Err("replaced by a newer connection".into());
            }
            ctx.notify.board_changed();
        }
        Up::Result { id, ok, error, data } => {
            let waiter = {
                let mut s = lock(&ctx.shared);
                s.touch(&link.id);
                match s.pending.get(&id) {
                    Some(p) if p.conn == link.serial => s.pending.remove(&id),
                    _ => None,
                }
            };
            match waiter {
                Some(p) => {
                    let _ = p.tx.send(if ok { Ok(data) } else { Err(error.unwrap_or_else(|| "The command failed.".into())) });
                }
                None => log::line("network", format!("{}: result for unknown or expired command {id}", link.label)),
            }
        }
        Up::Ping => {
            lock(&ctx.shared).touch(&link.id);
            send_down(ws, &Down::Pong)?;
        }
        Up::Hello { .. } | Up::Pair { .. } | Up::Auth { .. } => log::line("network", format!("{}: ignored a handshake message after welcome", link.label)),
    }
    Ok(())
}

/// Forgets a closed connection: its board greys, its commands fail.
fn leave(ctx: &Ctx, link: &Link, reason: &str) {
    let current = {
        let mut s = lock(&ctx.shared);
        let current = s.conns.get(&link.id).is_some_and(|c| c.serial == link.serial);
        if current {
            s.conns.remove(&link.id);
            if let Some(b) = s.by_id.get_mut(&link.id) {
                b.connected = false;
            }
            s.sync_boards();
        }
        s.fail_pending(link.serial);
        current
    };
    log::line("network", format!("{}: disconnected ({reason})", link.label));
    if current && !ctx.stopped() {
        ctx.announce();
        ctx.notify.board_changed();
    }
}

fn kind_name(kind: &CommandKind) -> &'static str {
    match kind {
        CommandKind::Reply { .. } => "reply",
        CommandKind::Answer { .. } => "answer",
        CommandKind::Compact { .. } => "compact",
        CommandKind::Rename { .. } => "rename",
        CommandKind::SetOption { .. } => "set_option",
        CommandKind::CycleMode { .. } => "cycle_mode",
        CommandKind::Start { .. } => "start",
        CommandKind::Resume { .. } => "resume",
        CommandKind::ListResumable { .. } => "list_resumable",
        CommandKind::History { .. } => "history",
    }
}

/// Sends `kind` to the assistant shown as `machine` and waits for its result.
pub fn send_command_with(shared: &Arc<Mutex<Server>>, machine: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String> {
    let name = kind_name(&kind);
    let (rx, id) = {
        let mut s = lock(shared);
        let conn = s.id_for(machine).and_then(|id| s.conns.get(&id).map(|c| (c.tx.clone(), c.serial)));
        let Some((tx, serial)) = conn else {
            return Err(format!("{machine} is not connected"));
        };
        let id = s.next();
        let (rtx, rrx) = channel();
        s.pending.insert(id, Pending { conn: serial, tx: rtx });
        if tx.send(Down::Command { id, kind }).is_err() {
            s.pending.remove(&id);
            return Err(format!("{machine} is not connected"));
        }
        (rrx, id)
    };
    log::line("network", format!("{machine}: command {id} {name}"));
    let out = match rx.recv_timeout(timeout) {
        Ok(r) => r,
        Err(RecvTimeoutError::Timeout) => {
            lock(shared).pending.remove(&id);
            Err(format!("{machine} did not answer in time"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(DISCONNECTED.into()),
    };
    if let Err(e) = &out {
        log::line("network", format!("{machine}: command {id} {name} failed: {e}"));
    }
    out
}

/// `send_command_with` against the running server; an error when this Maya is not the main.
pub fn send_command(app: &AppHandle, machine: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String> {
    let shared = app.state::<crate::AppState>().network.lock().unwrap().server.as_ref().map(|s| s.shared.clone());
    let shared = shared.ok_or_else(|| format!("{machine} is not connected"))?;
    send_command_with(&shared, machine, kind, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NetworkConfig;
    use crate::model::{Card, Harness, State};
    use crate::net::protocol::{decode_down, encode, mac, CommandKind, Down, Up, PROTOCOL};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    pub(crate) type Client = WebSocket<MaybeTlsStream<TcpStream>>;

    /// Counts what the server tells the app.
    #[derive(Default)]
    pub(crate) struct Counter {
        pub boards: AtomicUsize,
        pub statuses: AtomicUsize,
        pub paired: AtomicUsize,
        pub lists: AtomicUsize,
    }

    impl Notify for Counter {
        fn board_changed(&self) {
            self.boards.fetch_add(1, Ordering::SeqCst);
        }
        fn status_changed(&self, _: NetworkStatus) {
            self.statuses.fetch_add(1, Ordering::SeqCst);
        }
        fn paired(&self, _: &crate::config::PairedAssistant) {
            self.paired.fetch_add(1, Ordering::SeqCst);
        }
        fn paired_list_changed(&self, _: &[crate::config::PairedAssistant]) {
            self.lists.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub(crate) fn test_server_with(counter: Arc<Counter>) -> (ServerHandle, u16) {
        let handle = start_with(counter, Arc::new(Mutex::new(NetworkConfig::default())), 0).unwrap();
        let port = handle.port();
        (handle, port)
    }

    pub(crate) fn test_server() -> (ServerHandle, u16) {
        test_server_with(Arc::new(Counter::default()))
    }

    fn hello(ws: &mut Client, name: &str, id: Option<&str>) {
        send(ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), id: id.map(str::to_string) });
    }

    /// Pairs a new assistant; returns its open socket, id and token.
    pub(crate) fn pair_client(handle: &ServerHandle, port: u16, name: &str) -> (Client, String, String) {
        let (code, _) = handle.open_pairing(crate::store::now_ms());
        let mut ws = connect(port);
        hello(&mut ws, name, None);
        send(&mut ws, &Up::Pair { code });
        let Down::Paired { id, token } = recv(&mut ws) else { panic!("expected paired") };
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        (ws, id, token)
    }

    pub(crate) fn connect(port: u16) -> Client {
        let (ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/")).unwrap();
        if let MaybeTlsStream::Plain(s) = ws.get_ref() {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        }
        ws
    }

    pub(crate) fn send(ws: &mut Client, up: &Up) {
        ws.send(Message::text(encode(up))).unwrap();
    }

    pub(crate) fn recv(ws: &mut Client) -> Down {
        loop {
            match ws.read().unwrap() {
                Message::Text(t) => return decode_down(&t).unwrap(),
                _ => continue,
            }
        }
    }

    pub(crate) fn wait_until(mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "condition not met within 5 s");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn card(id: &str, name: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: name.into(), cwd: "/x/p".into(), state: State::Idle, state_since: 0, snippet: "".into(), awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, stale: false }
    }

    #[test]
    fn pairs_snapshots_and_round_trips_a_command() {
        let (handle, port) = test_server();
        let (code, _) = handle.open_pairing(crate::store::now_ms());
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), id: None });
        send(&mut ws, &Up::Pair { code });
        let Down::Paired { id, token } = recv(&mut ws) else { panic!("expected paired") };
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        send(&mut ws, &Up::Board { cards: vec![card("r1", "remote")], dirs: vec!["proj".into()] });
        wait_until(|| handle.shared.lock().unwrap().boards.iter().any(|b| b.machine == "laptop" && b.cards.len() == 1));
        // A command from the main reaches the client and its result comes back.
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "laptop", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)));
        let Down::Command { id: cmd, kind } = recv(&mut ws) else { panic!("expected command") };
        assert_eq!(kind, CommandKind::Compact { session: "r1".into() });
        send(&mut ws, &Up::Result { id: cmd, ok: true, error: None, data: None });
        assert!(t.join().unwrap().is_ok());
        // Reconnect with the token: challenge-response, no code.
        drop(ws);
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), id: Some(id.clone()) });
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        send(&mut ws, &Up::Auth { mac: mac(&token, &nonce) });
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        handle.stop();
    }

    #[test]
    fn a_wrong_mac_or_code_is_refused_and_a_command_to_a_disconnected_assistant_fails_fast() {
        let (handle, port) = test_server();
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "x".into(), hostname: "h".into(), platform: "macos".into(), id: Some("nobody".into()) });
        assert!(matches!(recv(&mut ws), Down::Bye { .. }), "unknown assistant");
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "x".into(), hostname: "h".into(), platform: "macos".into(), id: None });
        send(&mut ws, &Up::Pair { code: "000000".into() });
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason.contains("pairing code")));
        let err = send_command_with(&handle.shared, "ghost", CommandKind::Compact { session: "r".into() }, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("not connected"), "{err}");
        handle.stop();
    }

    #[test]
    fn garbage_disconnects_wrong_macs_lockouts_removal_and_stop() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut ws, id, token) = pair_client(&handle, port, "desk");
        assert_eq!(counter.paired.load(Ordering::SeqCst), 1);
        assert!(handle.status().code.is_none(), "a code pairs one assistant");
        // Garbage is skipped; the next good frame still lands.
        ws.send(Message::text("not json")).unwrap();
        ws.send(Message::text(r#"{"type":"dance"}"#)).unwrap();
        ws.send(Message::binary(vec![1, 2, 3])).unwrap();
        send(&mut ws, &Up::Ping);
        assert_eq!(recv(&mut ws), Down::Pong);
        send(&mut ws, &Up::Board { cards: vec![card("r1", "x")], dirs: vec![] });
        wait_until(|| handle.boards().iter().any(|b| b.machine == "desk" && b.connected));
        assert!(counter.boards.load(Ordering::SeqCst) >= 1);
        // A command in flight fails when its assistant goes away, and the board greys.
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "desk", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)));
        assert!(matches!(recv(&mut ws), Down::Command { .. }));
        drop(ws);
        let err = t.join().unwrap().unwrap_err();
        assert!(err.contains("disconnected"), "{err}");
        wait_until(|| handle.boards().iter().any(|b| b.machine == "desk" && !b.connected));
        assert!(!handle.status().assistants[0].connected);
        // A wrong MAC is refused.
        let mut bad = connect(port);
        hello(&mut bad, "desk", Some(&id));
        let Down::Challenge { .. } = recv(&mut bad) else { panic!("expected challenge") };
        send(&mut bad, &Up::Auth { mac: "00".repeat(32) });
        assert!(matches!(recv(&mut bad), Down::Bye { reason } if reason == "authentication failed"));
        // Three wrong codes lock the address out, even with the right code after.
        for _ in 0..3 {
            let mut w = connect(port);
            hello(&mut w, "x", None);
            send(&mut w, &Up::Pair { code: "999999x".into() });
            assert!(matches!(recv(&mut w), Down::Bye { .. }));
        }
        let (code, _) = handle.open_pairing(crate::store::now_ms());
        let mut w = connect(port);
        hello(&mut w, "x", None);
        send(&mut w, &Up::Pair { code });
        assert!(matches!(recv(&mut w), Down::Bye { reason } if reason == "too many attempts"));
        // Removal says bye, drops the cards, and the token no longer opens a connection.
        let mut ws = connect(port);
        hello(&mut ws, "desk", Some(&id));
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        send(&mut ws, &Up::Auth { mac: mac(&token, &nonce) });
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        handle.remove_assistant(&id);
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == "removed"));
        assert!(handle.boards().is_empty());
        assert!(handle.status().assistants.is_empty());
        assert_eq!(counter.lists.load(Ordering::SeqCst), 1);
        let mut again = connect(port);
        hello(&mut again, "desk", Some(&id));
        assert!(matches!(recv(&mut again), Down::Bye { reason } if reason == "removed"));
        // Stop closes live connections and frees the port.
        let (mut live, _, _) = {
            // The lockout is per address, so pair through a fresh server.
            let (h2, p2) = test_server();
            let c = pair_client(&h2, p2, "other");
            h2.stop();
            wait_until(|| TcpStream::connect(("127.0.0.1", p2)).is_err());
            c
        };
        let closed = loop {
            match live.read() {
                Ok(Message::Close(_)) => break true,
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break false,
                Err(_) => break true,
            }
        };
        assert!(closed, "the connection closes when the server stops");
        handle.stop();
        wait_until(|| TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
