//! The main Maya's server: a WebSocket listener that pairs assistants, keeps
//! their board snapshots and sends them commands.
//!
//! Threads: one accept loop, and one thread per connection that owns its
//! socket. The socket's read timeout (`TICK`) is the loop's tick: each turn
//! writes the `Down` frames queued on the connection's channel, then tries
//! one read. A separate deadline (`SILENCE`) drops a peer that sends nothing,
//! and the stop flag ends every thread within a tick. Before `welcome` a
//! connection has `PRE_AUTH_DEADLINE` in all, and at most `MAX_PRE_AUTH`
//! such connections are open at once, `MAX_PRE_AUTH_PER_ADDRESS` of them
//! from any one address. Dropping a
//! connection's sender (a newer connection for the same assistant, or its
//! removal) ends that connection too.
//!
//! Locks: `Server` sits behind one mutex held only for bookkeeping. `Notify`
//! is never called with it held, so the Tauri adapter may take the app's own
//! locks (network, then server, then store) without a cycle, with one
//! exception: the saves, `Notify::paired` and `Notify::paired_list_changed`,
//! are made with it held, so they land in the order the list changed and an
//! older snapshot can never overwrite a newer one. Their adapter takes only
//! `store`, which comes after the server's mutex in that order (`AppState`
//! in lib.rs), and nothing takes the server's mutex while holding `store`.

use super::merge::{display_names, RemoteBoard};
use super::protocol::{decode_up, encode, mac, mac_matches, new_id, new_nonce, new_token, Attempts, CommandKind, Down, NonceLog, PairingWindow, Up, MAX_FRAME, PROTOCOL};
use super::{AssistantStatus, NetworkStatus, PairingCode};
use crate::config::{NetworkConfig, NetworkRole, PairedAssistant};
use crate::log;
use crate::store::now_ms;
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
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
/// Frames before `welcome` are small; a stranger cannot make the main buffer more.
const PRE_AUTH_FRAME: usize = 64 * 1024;
/// Connections beyond this are closed at once.
const MAX_CONNECTIONS: usize = 32;
/// Connections not yet through `welcome` beyond this are closed at once, so
/// strangers cannot hold every slot.
const MAX_PRE_AUTH: usize = 8;
/// And beyond this from one address, so one stranger cannot take them all.
const MAX_PRE_AUTH_PER_ADDRESS: usize = 2;
/// A connection must get through `welcome` within this, from accept.
const PRE_AUTH_DEADLINE: Duration = Duration::from_secs(10);
/// What `fail_pending` answers; `send_command_with` names the machine.
const DISCONNECTED: &str = "assistant disconnected";

/// What the server tells the app; the Tauri adapter lives in `lib.rs`.
/// `paired` and `paired_list_changed` are called with the server's mutex
/// held (see the module docs) and must not take it; the others never are.
pub trait Notify: Send + Sync {
    fn board_changed(&self);
    /// An assistant's first board when this run of the main holds none for
    /// it (it just paired, or the main just started), before it joins the
    /// boards: nothing on it is announced. Later changes are.
    fn board_seeded(&self, cards: &[crate::model::Card]);
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

/// What the main knows of a connected (or once connected) assistant beyond
/// its paired entry, which holds its name, hostname, platform and address.
struct Peer {
    /// The Maya version it runs.
    app: String,
    last_seen: u64,
    /// Its last board failed to decode; the one before it is kept.
    unreadable_board: bool,
}

pub(crate) const UNREADABLE_BOARD: &str = "sent a board this version cannot read; update Maya on both machines";

impl Peer {
    /// What the assistants list says beside its name, if anything.
    fn note(&self) -> Option<String> {
        let ours = env!("CARGO_PKG_VERSION");
        let mut parts = vec![];
        if !self.app.is_empty() && self.app != ours {
            parts.push(format!("runs Maya {}; this Mac runs {ours}", self.app));
        }
        if self.unreadable_board {
            parts.push(UNREADABLE_BOARD.to_string());
        }
        Some(parts.join("; ")).filter(|n| !n.is_empty())
    }
}

/// Why a frame that did not decode matters: a board this version cannot
/// read (a newer Maya's session state…) is worth telling the user about;
/// any other bad frame is only logged.
fn unreadable_board(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|v| v.get("type").and_then(Value::as_str) == Some("board"))
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

    /// Each paired assistant's id and label: its name, with its address
    /// when another entry has the same name.
    fn labels(&self) -> Vec<(String, String)> {
        let entries: Vec<(String, String)> = self.paired.iter().map(|p| (p.name.clone(), p.address.clone())).collect();
        self.paired.iter().zip(display_names(&entries)).map(|(p, label)| (p.id.clone(), label)).collect()
    }

    /// Records a pairing from `address`. A disconnected entry already paired
    /// from that address is the same machine pairing again: it keeps its id,
    /// gets a new token and the names just given, and its old board goes.
    /// Otherwise the pairing is a new entry. Returns the entry, and what it
    /// was before when it already existed (for `undo_pairing`).
    fn record_pairing(&mut self, address: &str, name: &str, hostname: &str, platform: &str) -> (PairedAssistant, Option<PairedAssistant>) {
        let again = self.paired.iter().position(|p| p.address == address && !self.conns.contains_key(&p.id));
        let Some(i) = again else {
            let a = PairedAssistant { id: new_id(), name: name.into(), hostname: hostname.into(), platform: platform.into(), token: new_token(), address: address.into(), last_seen: None };
            self.paired.push(a.clone());
            return (a, None);
        };
        let previous = self.paired[i].clone();
        let p = &mut self.paired[i];
        p.name = name.into();
        p.hostname = hostname.into();
        p.platform = platform.into();
        p.token = new_token();
        let a = p.clone();
        self.by_id.remove(&a.id);
        self.peers.remove(&a.id);
        self.sync_boards();
        (a, Some(previous))
    }

    /// Takes back a pairing the assistant never completed: a new entry goes,
    /// an existing one gets its old token and names back.
    fn undo_pairing(&mut self, id: &str, previous: Option<PairedAssistant>) {
        match previous {
            Some(p) => {
                if let Some(e) = self.paired.iter_mut().find(|e| e.id == id) {
                    *e = p;
                }
            }
            None => {
                self.paired.retain(|e| e.id != id);
                self.by_id.remove(id);
            }
        }
        self.peers.remove(id);
        self.sync_boards();
    }

    /// Brings a paired entry up to date as its machine connects: the address
    /// it connected from, the names it just gave, and when that was.
    fn refresh_entry(&mut self, id: &str, address: &str, name: &str, hostname: &str, platform: &str, now_ms: u64) {
        if let Some(p) = self.paired.iter_mut().find(|p| p.id == id) {
            p.address = address.into();
            p.name = name.into();
            p.hostname = hostname.into();
            p.platform = platform.into();
            p.last_seen = Some(now_ms);
        }
    }

    fn label(&self, id: &str) -> String {
        self.labels().into_iter().find(|(i, _)| i == id).map(|(_, l)| l).unwrap_or_else(|| id.to_string())
    }

    /// The assistant shown as `machine`; a connected one wins should labels ever tie.
    fn id_for(&self, machine: &str) -> Option<String> {
        let ids: Vec<String> = self.labels().into_iter().filter(|(_, l)| l == machine).map(|(i, _)| i).collect();
        ids.iter().find(|i| self.conns.contains_key(*i)).or(ids.first()).cloned()
    }

    fn touch(&mut self, id: &str) {
        if let Some(p) = self.peers.get_mut(id) {
            p.last_seen = now_ms();
        }
    }

    /// Rebuilds `boards` from the snapshots, relabelled, with each machine's
    /// hostname, platform and address, and sorted by machine.
    fn sync_boards(&mut self) {
        let labels = self.labels();
        let mut boards: Vec<RemoteBoard> = self
            .paired
            .iter()
            .zip(labels)
            .filter_map(|(p, (id, label))| {
                self.by_id.get(&id).map(|b| RemoteBoard { machine: label, hostname: p.hostname.clone(), platform: p.platform.clone(), address: p.address.clone(), ..b.clone() })
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
                let peer = self.peers.get(&p.id);
                AssistantStatus { id: p.id.clone(), name, hostname: p.hostname.clone(), platform: p.platform.clone(), address: p.address.clone(), connected: self.conns.contains_key(&p.id), last_seen: peer.map(|x| x.last_seen).or(p.last_seen), note: peer.and_then(Peer::note) }
            })
            .collect();
        NetworkStatus { role: NetworkRole::Main, code, assistants, ..Default::default() }
    }
}

/// The server's mutex, recovered if a thread panicked while holding it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Ctx {
    shared: Arc<Mutex<Server>>,
    notify: Arc<dyn Notify>,
    stop: Arc<AtomicBool>,
    main_name: String,
    live: AtomicUsize,
    /// Connections accepted and not yet through `welcome`.
    pre_auth: Mutex<HandshakeSlots>,
}

/// Connections not yet through `welcome`, in all and per peer address.
#[derive(Default)]
struct HandshakeSlots {
    total: usize,
    by_address: HashMap<IpAddr, usize>,
}

impl HandshakeSlots {
    /// Takes a slot for a connection from `ip`, or says why there is none.
    fn take(&mut self, ip: IpAddr) -> Result<(), &'static str> {
        if self.total >= MAX_PRE_AUTH {
            return Err("too many connections still in their handshake");
        }
        let n = self.by_address.get(&ip).copied().unwrap_or(0);
        if n >= MAX_PRE_AUTH_PER_ADDRESS {
            return Err("too many connections from this address still in their handshake");
        }
        self.by_address.insert(ip, n + 1);
        self.total += 1;
        Ok(())
    }

    fn release(&mut self, ip: IpAddr) {
        self.total = self.total.saturating_sub(1);
        if let Some(n) = self.by_address.get_mut(&ip) {
            *n -= 1;
            if *n == 0 {
                self.by_address.remove(&ip);
            }
        }
    }
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
        let (label, status) = {
            let mut s = lock(&self.shared);
            let label = s.label(id);
            s.paired.retain(|p| p.id != id);
            if let Some(c) = s.conns.remove(id) {
                let _ = c.tx.send(Down::Bye { reason: "removed".into() });
            }
            s.by_id.remove(id);
            s.peers.remove(id);
            s.sync_boards();
            self.ctx.notify.paired_list_changed(&s.paired);
            (label, s.status(now_ms()))
        };
        log::line("network", format!("{label}: removed"));
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
/// `config_view` supplies the paired assistants at start; from then on the
/// server's own list is the truth and `Notify` reports changes to it.
/// What assistants see as the main's name in `welcome`: the configured
/// name, or this computer's hostname when it is blank.
pub fn main_name_of(c: &NetworkConfig) -> String {
    let n = c.name.trim();
    if n.is_empty() { super::local_hostname() } else { n.to_string() }
}

pub fn start_with(notify: Arc<dyn Notify>, config_view: Arc<Mutex<NetworkConfig>>, port: u16) -> Result<ServerHandle, String> {
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("Could not listen on port {port}: {e}. Choose another port."))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let paired = lock(&config_view).assistants.clone();
    let shared = Arc::new(Mutex::new(Server::new(paired)));
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(Ctx { shared: shared.clone(), notify, stop: stop.clone(), main_name: main_name_of(&config_view.lock().unwrap()), live: AtomicUsize::new(0), pre_auth: Mutex::new(HandshakeSlots::default()) });
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
                // Refused before the WebSocket upgrade: dropping the stream closes it.
                if let Err(why) = lock(&ctx.pre_auth).take(addr.ip()) {
                    log::line("network", format!("{addr}: refused, {why}"));
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
                    lock(&ctx.pre_auth).release(addr.ip());
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
    /// No board has arrived on this connection yet.
    first_board: std::cell::Cell<bool>,
}

/// Counts a connection as pre-auth until dropped (`welcome` sent, or it failed).
struct PreAuth<'a>(&'a Ctx, IpAddr);

impl Drop for PreAuth<'_> {
    fn drop(&mut self) {
        lock(&self.0.pre_auth).release(self.1);
    }
}

fn serve(ctx: &Ctx, stream: TcpStream, addr: SocketAddr) {
    let pre_auth = PreAuth(ctx, addr.ip());
    let deadline = Instant::now() + PRE_AUTH_DEADLINE;
    // Accepted sockets inherit the listener's non-blocking flag on macOS.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(TICK));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);
    let mut ws = match upgrade(ctx, stream, deadline) {
        Ok(ws) => ws,
        Err(e) => {
            log::line("network", format!("{addr}: no WebSocket handshake: {e}"));
            return;
        }
    };
    let admitted = admit(ctx, &mut ws, addr, deadline);
    drop(pre_auth);
    let Some(link) = admitted else {
        finish(&mut ws);
        return;
    };
    let reason = pump(ctx, &mut ws, &link);
    // Forgotten before the close completes: an assistant that waits for its
    // close to be answered (a client stopped for "Pair again") finds the
    // main already counting it as gone.
    leave(ctx, &link, &reason);
    finish(&mut ws);
}

fn upgrade(ctx: &Ctx, stream: TcpStream, deadline: Instant) -> Result<Ws, String> {
    let config = WebSocketConfig::default().max_message_size(Some(PRE_AUTH_FRAME)).max_frame_size(Some(PRE_AUTH_FRAME));
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

/// Logs a frame that is neither text nor a control frame, and skips it.
fn skip_frame(who: &str, m: &Message) {
    match m {
        Message::Ping(_) | Message::Pong(_) | Message::Text(_) | Message::Close(_) => {}
        Message::Binary(b) => log::line("network", format!("{who}: ignored a binary frame ({} bytes)", b.len())),
        Message::Frame(_) => log::line("network", format!("{who}: ignored a raw frame")),
    }
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

/// The next frame that parses, by `deadline`; garbage is logged and skipped.
fn read_up(ctx: &Ctx, ws: &mut Ws, who: &str, deadline: Instant) -> Result<Up, String> {
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
            Ok(other) => skip_frame(who, &other),
            Err(e) if is_timeout(&e) => {
                if Instant::now() > deadline {
                    return Err("no answer within 10 s of connecting".into());
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

fn admit(ctx: &Ctx, ws: &mut Ws, addr: SocketAddr, deadline: Instant) -> Option<Link> {
    let ip = addr.ip().to_string();
    let hello = match read_up(ctx, ws, &ip, deadline) {
        Ok(up) => up,
        Err(e) => {
            log::line("network", format!("{ip}: no hello: {e}"));
            return None;
        }
    };
    let Up::Hello { protocol, app, name, hostname, platform, id } = hello else {
        log::line("network", format!("{ip}: first message was not a hello"));
        bye(ws, "protocol");
        return None;
    };
    if protocol != PROTOCOL {
        log::line("network", format!("{ip}: protocol {protocol}, expected {PROTOCOL}"));
        bye(ws, "protocol");
        return None;
    }
    let (hostname, platform, app) = (clean(&hostname), clean(&platform), clean(&app));
    if app != env!("CARGO_PKG_VERSION") {
        log::line("network", format!("{ip}: runs Maya {app}; this Mac runs {}", env!("CARGO_PKG_VERSION")));
    }
    let name = Some(clean(&name)).filter(|n| !n.is_empty()).or_else(|| Some(hostname.clone()).filter(|h| !h.is_empty())).unwrap_or_else(|| "assistant".into());
    // `proof` answers the assistant's own nonce: under the token when it
    // authenticates, under the pairing code when it pairs.
    // A fresh pairing is only saved once `welcome` is out.
    let (id, proof, pairing) = match id {
        Some(id) => {
            let proof = authenticate(ctx, ws, &id, &name, deadline)?;
            (id, proof, None)
        }
        None => {
            let (p, proof) = pair(ctx, ws, &ip, &name, &hostname, &platform, deadline)?;
            (p.entry.id.clone(), proof, Some(p))
        }
    };
    let (tx, rx) = channel();
    let registered = {
        let mut s = lock(&ctx.shared);
        if s.is_paired(&id) {
            let serial = s.next();
            // The entry follows the machine: its address, and the names it just gave.
            let now = now_ms();
            s.refresh_entry(&id, &ip, &name, &hostname, &platform, now);
            s.peers.insert(id.clone(), Peer { app, last_seen: now, unreadable_board: false });
            // A newer connection replaces an older one; dropping its sender ends it.
            s.conns.insert(id.clone(), Conn { tx, serial });
            if let Some(b) = s.by_id.get_mut(&id) {
                b.connected = true;
            }
            s.sync_boards();
            // Saved here, under the lock; a fresh pairing is saved after
            // `welcome`, below. A stopped server's list is no longer the truth.
            if pairing.is_none() && !ctx.stopped() {
                ctx.notify.paired_list_changed(&s.paired);
            }
            Some((serial, s.label(&id)))
        } else {
            None
        }
    };
    let Some((serial, label)) = registered else {
        bye(ws, "removed");
        return None;
    };
    let link = Link { id, serial, label, rx, first_board: std::cell::Cell::new(true) };
    // Authenticated: boards and results may now be large.
    ws.set_config(|c| {
        c.max_message_size = Some(MAX_FRAME);
        c.max_frame_size = Some(MAX_FRAME);
    });
    if let Err(e) = send_down(ws, &Down::Welcome { name: ctx.main_name.clone(), mac: proof }) {
        // Undone first, so `leave` never saves the pairing it takes back.
        if let Some(p) = pairing {
            p.undo(ctx, &e);
        }
        leave(ctx, &link, &e);
        return None;
    }
    if pairing.is_some() {
        let s = lock(&ctx.shared);
        if let Some(entry) = s.paired.iter().find(|p| p.id == link.id) {
            ctx.notify.paired(entry);
        }
    }
    log::line("network", format!("{}: connected from {ip}", link.label));
    ctx.announce();
    ctx.notify.board_changed();
    Some(link)
}

/// Challenge-response for a paired assistant; returns the main's proof for
/// `welcome`: the MAC of the nonce the assistant sent with its answer.
fn authenticate(ctx: &Ctx, ws: &mut Ws, id: &str, name: &str, deadline: Instant) -> Option<String> {
    let token = lock(&ctx.shared).paired.iter().find(|p| p.id == id).map(|p| p.token.clone());
    let Some(token) = token else {
        // Unknown ids are removed ones (or paired with another main): pairing again is the fix.
        log::line("network", format!("{name}: unknown assistant; told it was removed"));
        bye(ws, "removed");
        return None;
    };
    let nonce = new_nonce();
    send_down(ws, &Down::Challenge { nonce: nonce.clone() }).ok()?;
    let (ok, theirs) = match read_up(ctx, ws, name, deadline) {
        Ok(Up::Auth { mac, nonce: theirs }) => (mac_matches(&token, &nonce, &mac) && !theirs.is_empty() && lock(&ctx.shared).nonces.first_use(&nonce, now_ms()), theirs),
        Ok(_) => (false, String::new()),
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
    Some(mac(&token, &theirs))
}

/// A pairing recorded but not yet saved: it is saved once `welcome` is out,
/// and undone if `paired` or `welcome` cannot be sent, so an assistant
/// that never got its token leaves no entry behind.
struct NewPairing {
    entry: PairedAssistant,
    previous: Option<PairedAssistant>,
}

impl NewPairing {
    fn undo(self, ctx: &Ctx, why: &str) {
        log::line("network", format!("{}: pairing not completed ({why}); nothing kept", self.entry.name));
        lock(&ctx.shared).undo_pairing(&self.entry.id, self.previous);
        ctx.announce();
    }
}

/// Pairs a new assistant, or the same machine again; returns the pairing
/// and the main's proof for `welcome`: the MAC, under the code, of the
/// nonce the assistant sent with it.
fn pair(ctx: &Ctx, ws: &mut Ws, ip: &str, name: &str, hostname: &str, platform: &str, deadline: Instant) -> Option<(NewPairing, String)> {
    let (code, nonce) = match read_up(ctx, ws, ip, deadline) {
        Ok(Up::Pair { code, nonce }) => (code, nonce),
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
            let pairing = s.record_pairing(ip, name, hostname, platform);
            // One code, one assistant.
            s.pairing = None;
            Ok(pairing)
        }
    };
    match outcome {
        Err(reason) => {
            log::line("network", format!("{ip}: pairing refused: {reason}"));
            bye(ws, reason);
            None
        }
        Ok((entry, previous)) => {
            let again = previous.is_some();
            let pairing = NewPairing { entry, previous };
            if let Err(e) = send_down(ws, &Down::Paired { id: pairing.entry.id.clone(), token: pairing.entry.token.clone() }) {
                pairing.undo(ctx, &e);
                return None;
            }
            log::line("network", format!("{name}: {} from {ip}", if again { "paired again (same entry, new key)" } else { "paired" }));
            Some((pairing, mac(code.trim(), &nonce)))
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
                    Err(e) if unreadable_board(&t) => {
                        // Keep the last good board; say why it no longer updates.
                        log::line("network", format!("{}: could not read its board: {e}", link.label));
                        keep_board_fresh(ctx, link);
                        mark_board(ctx, link, true);
                    }
                    Err(e) => log::line("network", format!("{}: ignored a message: {e}", link.label)),
                }
            }
            Ok(Message::Close(_)) => return "closed by the assistant".into(),
            Ok(other) => {
                deadline = Instant::now() + SILENCE;
                skip_frame(&link.label, &other);
            }
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
            // The first board of a connection is seeded, not announced, only
            // when this run of the main holds no board for the assistant: it
            // just paired, or the main just started. A reconnect's first board
            // is compared with the held one, so what began while it was away
            // is announced. Decided under the lock, seeded outside it (no
            // `Notify` call with `shared` held) and before the board is
            // visible, so no refresh in between announces it. Only this
            // connection's thread inserts this id's board.
            if link.first_board.replace(false) && !lock(&ctx.shared).by_id.contains_key(&link.id) {
                ctx.notify.board_seeded(&cards);
            }
            let current = {
                let mut s = lock(&ctx.shared);
                let current = s.conns.get(&link.id).is_some_and(|c| c.serial == link.serial);
                if current {
                    s.touch(&link.id);
                    // `sync_boards` fills in the label, hostname, platform and address.
                    s.by_id.insert(link.id.clone(), RemoteBoard { machine: String::new(), hostname: String::new(), platform: String::new(), address: String::new(), cards, dirs, received_at: now_ms(), connected: true });
                    s.sync_boards();
                }
                current
            };
            if !current {
                return Err("replaced by a newer connection".into());
            }
            mark_board(ctx, link, false);
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

/// An unreadable board still says the assistant is alive: the kept board
/// counts as just received, so its cards neither grey nor expire.
fn keep_board_fresh(ctx: &Ctx, link: &Link) {
    let kept = {
        let mut s = lock(&ctx.shared);
        let current = s.conns.get(&link.id).is_some_and(|c| c.serial == link.serial);
        s.touch(&link.id);
        match s.by_id.get_mut(&link.id) {
            Some(b) if current => {
                b.received_at = now_ms();
                s.sync_boards();
                true
            }
            _ => false,
        }
    };
    if kept {
        ctx.notify.board_changed();
    }
}

/// Records whether the assistant's last board was unreadable; the status
/// goes out only when that changed.
fn mark_board(ctx: &Ctx, link: &Link, unreadable: bool) {
    let changed = {
        let mut s = lock(&ctx.shared);
        let current = s.conns.get(&link.id).is_some_and(|c| c.serial == link.serial);
        match s.peers.get_mut(&link.id) {
            Some(p) if current && p.unreadable_board != unreadable => {
                p.unreadable_board = unreadable;
                true
            }
            _ => false,
        }
    };
    if changed {
        ctx.announce();
    }
}

/// Forgets a closed connection: its board greys, its commands fail, and
/// when it was last seen is saved.
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
            let seen = s.peers.get(&link.id).map_or_else(now_ms, |p| p.last_seen);
            if let Some(p) = s.paired.iter_mut().find(|p| p.id == link.id) {
                p.last_seen = Some(seen);
            }
            // Saved under the lock; a stopped server's list is no longer
            // the truth (a new one may be running).
            if !ctx.stopped() {
                ctx.notify.paired_list_changed(&s.paired);
            }
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

pub(crate) fn kind_name(kind: &CommandKind) -> &'static str {
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
        Ok(Err(e)) if e == DISCONNECTED => Err(format!("{machine} disconnected before answering.")),
        Ok(r) => r,
        Err(RecvTimeoutError::Timeout) => {
            lock(shared).pending.remove(&id);
            Err(format!("{machine} did not answer in time"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(format!("{machine} disconnected before answering.")),
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
    use crate::config::{NetworkConfig, NetworkRole};
    use crate::model::{Card, Harness, State};
    use crate::net::client;
    use std::sync::atomic::AtomicBool;
    use crate::net::protocol::{decode_down, encode, mac, CommandKind, Down, Up, PROTOCOL};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    pub(crate) type Client = WebSocket<MaybeTlsStream<TcpStream>>;

    /// Counts what the server tells the app, and announces what the app
    /// would: the notifier runs over the merged boards on every change.
    #[derive(Default)]
    pub(crate) struct Counter {
        pub boards: AtomicUsize,
        pub statuses: AtomicUsize,
        pub paired: AtomicUsize,
        /// Session ids announced (asks and finished turns), in order.
        pub announced: Mutex<Vec<String>>,
        /// The paired list as last saved.
        pub saved: Mutex<Vec<crate::config::PairedAssistant>>,
        /// Saves made without the server's lock held (there should be none).
        pub saves_outside_lock: AtomicUsize,
        notifier: Mutex<crate::notify::Notifier>,
        server: std::sync::OnceLock<Arc<Mutex<Server>>>,
    }

    impl Notify for Counter {
        fn board_changed(&self) {
            self.boards.fetch_add(1, Ordering::SeqCst);
            let Some(shared) = self.server.get() else { return };
            let boards = shared.lock().unwrap().boards.clone();
            let cards = super::super::merge::merged(vec![], &boards, now_ms());
            let mut n = self.notifier.lock().unwrap();
            let fresh: Vec<Card> = n.take_new(&cards).into_iter().chain(n.take_finished(&cards)).collect();
            self.announced.lock().unwrap().extend(fresh.into_iter().map(|c| c.session_id));
        }
        fn board_seeded(&self, cards: &[Card]) {
            self.notifier.lock().unwrap().seed(cards);
        }
        fn status_changed(&self, _: NetworkStatus) {
            self.statuses.fetch_add(1, Ordering::SeqCst);
        }
        fn paired(&self, assistant: &crate::config::PairedAssistant) {
            self.check_under_lock();
            self.paired.fetch_add(1, Ordering::SeqCst);
            let mut saved = self.saved.lock().unwrap();
            match saved.iter_mut().find(|a| a.id == assistant.id) {
                Some(a) => *a = assistant.clone(),
                None => saved.push(assistant.clone()),
            }
        }
        fn paired_list_changed(&self, list: &[crate::config::PairedAssistant]) {
            self.check_under_lock();
            *self.saved.lock().unwrap() = list.to_vec();
        }
    }

    impl Counter {
        /// Saves must happen with the server's lock held, so they land in the
        /// order the list changed. A save made without it finds the lock free.
        fn check_under_lock(&self) {
            if self.server.get().is_some_and(|s| s.try_lock().is_ok()) {
                self.saves_outside_lock.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    pub(crate) fn test_server_with(counter: Arc<Counter>) -> (ServerHandle, u16) {
        let handle = start_with(counter.clone(), Arc::new(Mutex::new(NetworkConfig::default())), 0).unwrap();
        let _ = counter.server.set(handle.shared.clone());
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
        let nonce = crate::net::protocol::new_nonce();
        send(&mut ws, &Up::Pair { code: code.clone(), nonce: nonce.clone() });
        let Down::Paired { id, token } = recv(&mut ws) else { panic!("expected paired") };
        let Down::Welcome { mac: proof, .. } = recv(&mut ws) else { panic!("expected welcome") };
        assert!(crate::net::protocol::mac_matches(&code, &nonce, &proof), "the main proves it knows the code");
        (ws, id, token)
    }

    /// Answers the main's challenge with a nonce of this side's own, and
    /// checks the `welcome` proves the main holds the same token.
    pub(crate) fn answer(ws: &mut Client, token: &str, nonce: &str) {
        let ours = crate::net::protocol::new_nonce();
        send(ws, &Up::Auth { mac: mac(token, nonce), nonce: ours.clone() });
        let Down::Welcome { mac: proof, .. } = recv(ws) else { panic!("expected welcome") };
        assert!(crate::net::protocol::mac_matches(token, &ours, &proof), "the main proves it holds the token");
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
        Card { session_id: id.into(), pid: 1, name: name.into(), cwd: "/x/p".into(), state: State::Idle, state_since: 0, snippet: "".into(), awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, stale: false }
    }

    #[test]
    fn pairs_snapshots_and_round_trips_a_command() {
        let (handle, port) = test_server();
        let (code, _) = handle.open_pairing(crate::store::now_ms());
        let mut ws = connect(port);
        send(&mut ws, &Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "laptop".into(), hostname: "h".into(), platform: "macos".into(), id: None });
        send(&mut ws, &Up::Pair { code, nonce: "n".into() });
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
        answer(&mut ws, &token, &nonce);
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
        send(&mut ws, &Up::Pair { code: "000000".into(), nonce: "n".into() });
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
        // Saved once `welcome` is out, which the client may read first.
        wait_until(|| counter.paired.load(Ordering::SeqCst) == 1);
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
        assert_eq!(err, "desk disconnected before answering.");
        wait_until(|| handle.boards().iter().any(|b| b.machine == "desk" && !b.connected));
        assert!(!handle.status().assistants[0].connected);
        // A wrong MAC is refused.
        let mut bad = connect(port);
        hello(&mut bad, "desk", Some(&id));
        let Down::Challenge { .. } = recv(&mut bad) else { panic!("expected challenge") };
        send(&mut bad, &Up::Auth { mac: "00".repeat(32), nonce: crate::net::protocol::new_nonce() });
        assert!(matches!(recv(&mut bad), Down::Bye { reason } if reason == "authentication failed"));
        // Three wrong codes lock the address out, even with the right code after.
        for _ in 0..3 {
            let mut w = connect(port);
            hello(&mut w, "x", None);
            send(&mut w, &Up::Pair { code: "999999x".into(), nonce: "n".into() });
            assert!(matches!(recv(&mut w), Down::Bye { .. }));
        }
        let (code, _) = handle.open_pairing(crate::store::now_ms());
        let mut w = connect(port);
        hello(&mut w, "x", None);
        send(&mut w, &Up::Pair { code, nonce: "n".into() });
        assert!(matches!(recv(&mut w), Down::Bye { reason } if reason == "too many attempts"));
        // Removal says bye, drops the cards, and the token no longer opens a connection.
        let mut ws = connect(port);
        hello(&mut ws, "desk", Some(&id));
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        answer(&mut ws, &token, &nonce);
        handle.remove_assistant(&id);
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == "removed"));
        assert!(handle.boards().is_empty());
        assert!(handle.status().assistants.is_empty());
        assert!(counter.saved.lock().unwrap().is_empty(), "the removal is saved");
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

    #[test]
    fn a_machine_pairing_again_from_its_address_keeps_its_entry_with_a_new_token() {
        let (handle, port) = test_server();
        let (mut old, old_id, old_token) = pair_client(&handle, port, "laptop");
        send(&mut old, &Up::Board { cards: vec![card("r1", "x")], dirs: vec![] });
        wait_until(|| handle.boards().iter().any(|b| b.machine == "laptop" && b.address == "127.0.0.1"));
        drop(old);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        // Its config was reset: it pairs again from the same address.
        let (mut new, new_id, new_token) = pair_client(&handle, port, "laptop");
        assert_eq!(new_id, old_id, "the same machine keeps its entry");
        assert_ne!(new_token, old_token, "with a new token");
        let status = handle.status();
        assert_eq!(status.assistants.len(), 1);
        assert_eq!((status.assistants[0].name.as_str(), status.assistants[0].address.as_str()), ("laptop", "127.0.0.1"));
        assert!(handle.boards().iter().all(|b| !b.cards.iter().any(|c| c.session_id == "r1")), "the old snapshot is dropped at once");
        send(&mut new, &Up::Board { cards: vec![card("r2", "x")], dirs: vec![] });
        wait_until(|| handle.boards().iter().any(|b| b.machine == "laptop" && b.connected && b.cards[0].session_id == "r2"));
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "laptop", CommandKind::Compact { session: "r2".into() }, Duration::from_secs(5)));
        let Down::Command { id, .. } = recv(&mut new) else { panic!("expected command") };
        send(&mut new, &Up::Result { id, ok: true, error: None, data: None });
        assert!(t.join().unwrap().is_ok());
        // The old token no longer opens a connection.
        let mut stale = connect(port);
        hello(&mut stale, "laptop", Some(&old_id));
        let Down::Challenge { nonce } = recv(&mut stale) else { panic!("expected challenge") };
        send(&mut stale, &Up::Auth { mac: mac(&old_token, &nonce), nonce: crate::net::protocol::new_nonce() });
        assert!(matches!(recv(&mut stale), Down::Bye { reason } if reason == "authentication failed"));
        // While it is connected, another pairing from its address is another machine.
        let (_twin, twin_id, _) = pair_client(&handle, port, "desk");
        assert_ne!(twin_id, new_id);
        assert_eq!(handle.status().assistants.len(), 2);
        handle.stop();
    }

    #[test]
    fn pairings_are_told_apart_by_address_and_equal_names_carry_it() {
        let mut s = Server::new(vec![]);
        let (first, previous) = s.record_pairing("192.168.55.70", "Gnowee", "TKC-0176", "macos");
        assert!(previous.is_none());
        s.by_id.insert(first.id.clone(), RemoteBoard { machine: String::new(), hostname: String::new(), platform: String::new(), address: String::new(), cards: vec![card("r1", "x")], dirs: vec![], received_at: 0, connected: false });
        let (second, previous) = s.record_pairing("192.168.55.70", "Gnowee", "TKC-0176", "macos");
        assert_eq!(previous.as_ref(), Some(&first), "the same address, disconnected: the same machine");
        assert_eq!(second.id, first.id);
        assert_ne!(second.token, first.token);
        assert_eq!(s.paired.len(), 1);
        assert!(s.by_id.is_empty(), "its old board is dropped");
        // Another address with the same name is another machine.
        let (other, previous) = s.record_pairing("192.168.55.71", "Gnowee", "TKC-0199", "macos");
        assert!(previous.is_none());
        assert_ne!(other.id, first.id);
        let names: Vec<(String, String)> = s.status(0).assistants.into_iter().map(|a| (a.name, a.address)).collect();
        assert_eq!(names, [("Gnowee (192.168.55.70)".to_string(), "192.168.55.70".to_string()), ("Gnowee (192.168.55.71)".to_string(), "192.168.55.71".to_string())]);
        // A connected entry is never taken over by a new pairing.
        s.conns.insert(other.id.clone(), Conn { tx: channel().0, serial: 1 });
        let (third, previous) = s.record_pairing("192.168.55.71", "Gnowee", "TKC-0199", "macos");
        assert!(previous.is_none());
        assert_eq!(s.paired.len(), 3);
        assert_ne!(third.id, other.id);
        // Authentication brings the entry up to date: a new address, a new name.
        s.refresh_entry(&first.id, "192.168.55.80", "Gnowee", "TKC-0176", "macos", 7);
        assert_eq!((s.paired[0].address.as_str(), s.paired[0].last_seen), ("192.168.55.80", Some(7)));
        // A pairing the assistant never completed is taken back.
        s.undo_pairing(&third.id, None);
        assert_eq!(s.paired.len(), 2);
        let (again, previous) = s.record_pairing("192.168.55.80", "Renamed", "TKC-0176", "macos");
        s.undo_pairing(&again.id, previous);
        assert_eq!(s.paired[0].name, "Gnowee", "an entry paired again gets its old self back");
        assert_eq!(s.paired[0].token, second.token);
    }

    #[test]
    fn a_first_board_is_quiet_but_a_reconnect_announces_what_began_while_away() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let state = |id: &str, state: State, since: u64| Card { state, state_since: since, ..card(id, "x") };
        let (mut ws, id, token) = pair_client(&handle, port, "desk");
        // Open when it paired: an ask and a finished turn. Nothing is announced.
        send(&mut ws, &Up::Board { cards: vec![state("r1", State::Awaiting, 100), state("r2", State::Completed, 100)], dirs: vec![] });
        wait_until(|| handle.boards().iter().any(|b| b.cards.len() == 2));
        // A new ask after that is announced, and only it.
        send(&mut ws, &Up::Board { cards: vec![state("r1", State::Awaiting, 100), state("r2", State::Completed, 100), state("r3", State::Awaiting, 200)], dirs: vec![] });
        wait_until(|| !counter.announced.lock().unwrap().is_empty());
        assert_eq!(*counter.announced.lock().unwrap(), ["r3"]);
        // It drops; while it is away r1 asks something new and r2 finishes again.
        drop(ws);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        let mut ws = connect(port);
        hello(&mut ws, "desk", Some(&id));
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        answer(&mut ws, &token, &nonce);
        // The main still holds its board, so the reconnect's first board is news.
        send(&mut ws, &Up::Board { cards: vec![state("r1", State::Awaiting, 300), state("r2", State::Completed, 300), state("r3", State::Awaiting, 200)], dirs: vec![] });
        wait_until(|| counter.announced.lock().unwrap().len() == 3);
        assert_eq!(*counter.announced.lock().unwrap(), ["r3", "r1", "r2"], "what began while away is announced; r3 is not repeated");
        handle.stop();
    }

    #[test]
    fn a_main_started_afresh_is_quiet_about_a_known_assistants_first_board() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (ws, id, token) = pair_client(&handle, port, "desk");
        drop(ws);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        handle.stop();
        // A new run of the main: the assistant is paired but no board is held yet.
        let saved = handle.shared.lock().unwrap().paired.clone();
        let counter = Arc::new(Counter::default());
        let restarted = start_with(counter.clone(), Arc::new(Mutex::new(NetworkConfig { assistants: saved, ..Default::default() })), 0).unwrap();
        let _ = counter.server.set(restarted.shared.clone());
        let mut ws = connect(restarted.port());
        hello(&mut ws, "desk", Some(&id));
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        answer(&mut ws, &token, &nonce);
        send(&mut ws, &Up::Board { cards: vec![Card { state: State::Awaiting, state_since: 100, ..card("r1", "x") }], dirs: vec![] });
        wait_until(|| restarted.boards().iter().any(|b| b.cards.len() == 1));
        send(&mut ws, &Up::Board { cards: vec![Card { state: State::Awaiting, state_since: 100, ..card("r1", "x") }, Card { state: State::Awaiting, state_since: 200, ..card("r2", "x") }], dirs: vec![] });
        wait_until(|| !counter.announced.lock().unwrap().is_empty());
        assert_eq!(*counter.announced.lock().unwrap(), ["r2"]);
        restarted.stop();
    }

    /// Closes `ws` with a reset rather than a FIN, so the main's next write fails.
    fn reset(ws: Client) {
        use std::os::fd::AsRawFd;
        let MaybeTlsStream::Plain(s) = ws.get_ref() else { panic!("plain socket") };
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: a valid socket, and a linger struct of the size given.
        let rc = unsafe { libc::setsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_LINGER, &linger as *const _ as *const libc::c_void, std::mem::size_of::<libc::linger>() as libc::socklen_t) };
        assert_eq!(rc, 0);
        drop(ws);
    }

    #[test]
    fn an_assistant_gone_right_after_sending_pair_leaves_no_entry() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (code, _) = handle.open_pairing(now_ms());
        let mut ws = connect(port);
        // `hello` and `pair` in one write, then the socket goes.
        ws.write(Message::text(encode(&Up::Hello { protocol: PROTOCOL, app: "t".into(), name: "ghost".into(), hostname: "h".into(), platform: "macos".into(), id: None }))).unwrap();
        ws.write(Message::text(encode(&Up::Pair { code, nonce: "n".into() }))).unwrap();
        ws.flush().unwrap();
        reset(ws);
        wait_until(|| handle.ctx.live.load(Ordering::SeqCst) == 0);
        assert!(handle.status().assistants.is_empty(), "no paired entry is left behind");
        assert_eq!(counter.paired.load(Ordering::SeqCst), 0, "nothing was saved");
        assert!(handle.status().code.is_none(), "the main read the code and paired, then took the pairing back");
        handle.stop();
    }

    #[test]
    fn the_paired_list_is_saved_under_the_server_lock_so_no_older_snapshot_wins() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let reconnect = |id: &str, token: &str| {
            let mut ws = connect(port);
            hello(&mut ws, "desk", Some(id));
            let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
            answer(&mut ws, token, &nonce);
            ws
        };
        // Every kind of save: a pairing, a disconnect, a connect, a removal.
        let (desk, id, token) = pair_client(&handle, port, "desk");
        let (_twin, twin, _) = pair_client(&handle, port, "twin");
        wait_until(|| counter.paired.load(Ordering::SeqCst) == 2);
        drop(desk);
        wait_until(|| counter.saved.lock().unwrap().iter().any(|a| a.id == id && a.last_seen.is_some()) && !handle.status().assistants[0].connected);
        let desk = reconnect(&id, &token);
        handle.remove_assistant(&twin);
        drop(desk);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        // Each save was made with the lock held, so saves land in the order
        // the list changed and the last one is the current list.
        assert_eq!(counter.saves_outside_lock.load(Ordering::SeqCst), 0, "every save is made under the server lock");
        let ids = |l: &[crate::config::PairedAssistant]| l.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        wait_until(|| ids(&counter.saved.lock().unwrap()) == ids(&handle.shared.lock().unwrap().paired));
        assert_eq!(ids(&counter.saved.lock().unwrap()), [id]);
        handle.stop();
    }

    #[test]
    fn when_an_assistant_was_last_seen_is_saved_and_shown_after_a_restart() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (ws, id, _) = pair_client(&handle, port, "desk");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected && counter.saved.lock().unwrap().iter().any(|p| p.id == id && p.last_seen.is_some()));
        let saved = counter.saved.lock().unwrap().clone();
        assert_eq!(handle.status().assistants[0].last_seen, saved[0].last_seen);
        handle.stop();
        let restarted = start_with(Arc::new(Counter::default()), Arc::new(Mutex::new(NetworkConfig { assistants: saved.clone(), ..Default::default() })), 0).unwrap();
        let status = restarted.status();
        assert!(!status.assistants[0].connected);
        assert_eq!(status.assistants[0].last_seen, saved[0].last_seen, "a restarted main still says when it last saw it");
        restarted.stop();
    }

    #[test]
    fn a_version_mismatch_and_an_unreadable_board_are_noted_and_the_last_good_board_kept() {
        let (handle, port) = test_server();
        // The test clients say they run Maya "t".
        let (mut ws, id, _) = pair_client(&handle, port, "desk");
        let note = |handle: &ServerHandle| handle.status().assistants.iter().find(|a| a.id == id).unwrap().note.clone();
        let ours = env!("CARGO_PKG_VERSION");
        assert_eq!(note(&handle), Some(format!("runs Maya t; this Mac runs {ours}")));
        send(&mut ws, &Up::Board { cards: vec![card("r1", "x")], dirs: vec![] });
        wait_until(|| handle.boards().iter().any(|b| b.cards.len() == 1));
        // A newer Maya's board: a harness this version does not know.
        let mut future = serde_json::to_value(Up::Board { cards: vec![card("r2", "y")], dirs: vec![] }).unwrap();
        future["cards"][0]["harness"] = "future-harness".into();
        ws.send(Message::text(future.to_string())).unwrap();
        wait_until(|| note(&handle).is_some_and(|n| n.ends_with(UNREADABLE_BOARD)));
        assert_eq!(note(&handle).unwrap(), format!("runs Maya t; this Mac runs {ours}; {UNREADABLE_BOARD}"));
        assert_eq!(handle.boards()[0].cards[0].session_id, "r1", "the last good board stays");
        // It stays fresh too: the assistant is alive, only its board is unreadable.
        {
            let mut s = handle.shared.lock().unwrap();
            s.by_id.get_mut(&id).unwrap().received_at = 1;
            s.sync_boards();
        }
        ws.send(Message::text(future.to_string())).unwrap();
        wait_until(|| handle.boards()[0].received_at > 1);
        let cards = super::super::merge::merged(vec![], &handle.boards(), now_ms());
        assert_eq!(cards.len(), 1);
        assert!(!cards[0].stale, "the kept cards stay un-stale while the note explains why they do not change");
        assert!(note(&handle).unwrap().ends_with(UNREADABLE_BOARD));
        // A readable board clears the note.
        send(&mut ws, &Up::Board { cards: vec![card("r3", "z")], dirs: vec![] });
        wait_until(|| note(&handle).is_some_and(|n| !n.contains(UNREADABLE_BOARD)));
        // The same version says nothing.
        let (code, _) = handle.open_pairing(now_ms());
        let mut same = connect(port);
        send(&mut same, &Up::Hello { protocol: PROTOCOL, app: ours.into(), name: "twin".into(), hostname: "h2".into(), platform: "macos".into(), id: None });
        send(&mut same, &Up::Pair { code, nonce: "n".into() });
        let Down::Paired { id: twin, .. } = recv(&mut same) else { panic!("expected paired") };
        assert!(matches!(recv(&mut same), Down::Welcome { .. }));
        assert_eq!(handle.status().assistants.iter().find(|a| a.id == twin).unwrap().note, None);
        handle.stop();
    }

    #[test]
    fn only_an_unreadable_board_is_worth_a_note() {
        assert!(unreadable_board(r#"{"type":"board","cards":[{"harness":"future"}],"dirs":[]}"#));
        assert!(!unreadable_board(r#"{"type":"dance"}"#));
        assert!(!unreadable_board("not json"));
    }

    #[test]
    fn strangers_in_their_handshake_cannot_take_every_slot() {
        let (handle, port) = test_server();
        let (desk, id, token) = pair_client(&handle, port, "desk");
        let in_handshake = |handle: &ServerHandle| handle.ctx.pre_auth.lock().unwrap().total;
        wait_until(|| in_handshake(&handle) == 0);
        // Two sockets from this address that never even start the WebSocket handshake.
        let strangers: Vec<TcpStream> = (0..MAX_PRE_AUTH_PER_ADDRESS).map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
        wait_until(|| in_handshake(&handle) == MAX_PRE_AUTH_PER_ADDRESS);
        assert!(tungstenite::connect(format!("ws://127.0.0.1:{port}/")).is_err(), "a third from the same address is closed at once");
        assert!(handle.status().assistants[0].connected, "an assistant through welcome holds no slot");
        drop(strangers);
        wait_until(|| in_handshake(&handle) == 0);
        // The assistant reconnects with its token, and is through welcome.
        drop(desk);
        wait_until(|| !handle.status().assistants[0].connected);
        let mut ws = connect(port);
        hello(&mut ws, "desk", Some(&id));
        let Down::Challenge { nonce } = recv(&mut ws) else { panic!("expected challenge") };
        answer(&mut ws, &token, &nonce);
        wait_until(|| in_handshake(&handle) == 0);
        assert_eq!(handle.ctx.live.load(Ordering::SeqCst), 1);
        handle.stop();
    }

    #[test]
    fn handshake_slots_cap_each_address_and_all_of_them() {
        let ip = |n: u8| IpAddr::from([10, 0, 0, n]);
        let mut slots = HandshakeSlots::default();
        assert!(slots.take(ip(1)).is_ok() && slots.take(ip(1)).is_ok());
        assert!(slots.take(ip(1)).is_err(), "a third from one address");
        for n in 2..=(MAX_PRE_AUTH as u8 - 1) {
            assert!(slots.take(ip(n)).is_ok());
        }
        assert_eq!(slots.total, MAX_PRE_AUTH);
        assert!(slots.take(ip(100)).is_err(), "a ninth from anywhere");
        slots.release(ip(1));
        assert!(slots.take(ip(100)).is_ok(), "a freed slot is free for anyone");
        for n in [1, 100] {
            slots.release(ip(n));
        }
        for n in 2..=(MAX_PRE_AUTH as u8 - 1) {
            slots.release(ip(n));
        }
        assert_eq!(slots.total, 0);
        assert!(slots.by_address.is_empty(), "addresses with no slot are forgotten");
    }

    #[test]
    fn a_busy_port_is_an_error_that_names_the_port() {
        let taken = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let Err(err) = start_with(Arc::new(Counter::default()), Arc::new(Mutex::new(NetworkConfig::default())), port) else { panic!("the port is taken") };
        assert!(err.starts_with(&format!("Could not listen on port {port}: ")) && err.ends_with("Choose another port."), "{err}");
    }

    #[test]
    fn frames_are_small_until_welcome_and_large_after() {
        let (handle, port) = test_server();
        let mut ws = connect(port);
        ws.send(Message::text("x".repeat(PRE_AUTH_FRAME + 1))).unwrap();
        let refused = loop {
            match ws.read() {
                Ok(Message::Text(_)) => break false,
                Ok(Message::Close(_)) => break true,
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break false,
                Err(_) => break true,
            }
        };
        assert!(refused, "an oversized frame before hello closes the connection");
        let (mut ws, _, _) = pair_client(&handle, port, "desk");
        let mut big = card("r1", "x");
        big.snippet = "y".repeat(4 * PRE_AUTH_FRAME);
        send(&mut ws, &Up::Board { cards: vec![big], dirs: vec![] });
        wait_until(|| handle.boards().iter().any(|b| b.cards.len() == 1));
        handle.stop();
    }

    /// The client's executor: records each command and serves one card.
    #[derive(Default)]
    struct FakeExec {
        kinds: Mutex<Vec<CommandKind>>,
    }

    impl client::Executor for FakeExec {
        fn execute(&self, kind: CommandKind) -> Result<Option<Value>, String> {
            self.kinds.lock().unwrap().push(kind);
            Ok(None)
        }
        fn board(&self) -> (Vec<Card>, Vec<String>) {
            (vec![card("r1", "remote")], vec!["proj".into()])
        }
    }

    /// What the client tells the app.
    #[derive(Default)]
    struct FakeClientNotify {
        main: Mutex<Option<String>>,
        creds: Mutex<Option<(String, String)>>,
        errors: Mutex<Vec<String>>,
        removed: AtomicUsize,
    }

    impl client::ClientNotify for FakeClientNotify {
        fn paired(&self, id: &str, token: &str) {
            *self.creds.lock().unwrap() = Some((id.into(), token.into()));
        }
        fn connected(&self, main_name: &str) {
            *self.main.lock().unwrap() = Some(main_name.into());
        }
        fn disconnected(&self, error: &str) {
            self.errors.lock().unwrap().push(error.into());
        }
        fn removed(&self) {
            self.removed.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A stranger at the main's address: answers one connection with
    /// `welcome` (after a challenge when `challenge`) carrying `proof`, then
    /// sends a command. Returns its port.
    fn fake_main(challenge: bool, proof: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let read = |ws: &mut WebSocket<TcpStream>| loop {
                if let Message::Text(t) = ws.read().unwrap() {
                    return crate::net::protocol::decode_up(&t).unwrap();
                }
            };
            assert!(matches!(read(&mut ws), Up::Hello { .. }));
            if challenge {
                ws.send(Message::text(encode(&Down::Challenge { nonce: "n".into() }))).unwrap();
                assert!(matches!(read(&mut ws), Up::Auth { .. }));
            }
            let _ = ws.send(Message::text(encode(&Down::Welcome { name: "evil".into(), mac: proof.into() })));
            let _ = ws.send(Message::text(encode(&Down::Command { id: 1, kind: CommandKind::Compact { session: "r1".into() } })));
            let _ = ws.read();
        });
        port
    }

    /// A stranger that takes any pairing code: answers `pair` with an id and
    /// a token, then `welcome` carrying `proof`. Returns its port.
    fn fake_pairing_main(proof: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let read = |ws: &mut WebSocket<TcpStream>| loop {
                if let Message::Text(t) = ws.read().unwrap() {
                    return crate::net::protocol::decode_up(&t).unwrap();
                }
            };
            assert!(matches!(read(&mut ws), Up::Hello { .. }));
            assert!(matches!(read(&mut ws), Up::Pair { nonce, .. } if !nonce.is_empty()), "the assistant sends a nonce with its code");
            let _ = ws.send(Message::text(encode(&Down::Paired { id: "evil-id".into(), token: "evil-token".into() })));
            let _ = ws.send(Message::text(encode(&Down::Welcome { name: "evil".into(), mac: proof.into() })));
            let _ = ws.read();
        });
        port
    }

    #[test]
    fn the_client_keeps_no_credentials_from_a_main_that_cannot_prove_it_knows_the_code() {
        for proof in ["00", ""] {
            let port = fake_pairing_main(proof);
            let exec = Arc::new(FakeExec::default());
            let notify = Arc::new(FakeClientNotify::default());
            let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, ..Default::default() };
            let out = client::run_once(&config, exec.clone(), notify.clone(), &AtomicBool::new(false), Some("123456"));
            assert_eq!(out, Err(client::PAIRING_UNPROVEN.to_string()), "proof {proof:?}");
            assert!(notify.creds.lock().unwrap().is_none(), "nothing is saved");
            assert!(notify.main.lock().unwrap().is_none(), "never reported as connected");
            assert_eq!(*notify.errors.lock().unwrap(), [client::PAIRING_UNPROVEN]);
        }
    }

    #[test]
    fn the_client_refuses_a_main_that_cannot_prove_it_holds_the_token() {
        for (challenge, proof) in [(true, "00"), (true, ""), (false, "")] {
            let port = fake_main(challenge, proof);
            let exec = Arc::new(FakeExec::default());
            let notify = Arc::new(FakeClientNotify::default());
            let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, assistant_id: "a1".into(), token: crate::net::protocol::new_token(), ..Default::default() };
            let out = client::run_once(&config, exec.clone(), notify.clone(), &AtomicBool::new(false), None);
            assert_eq!(out, Err(client::MAIN_UNPROVEN.to_string()), "challenge {challenge}, proof {proof:?}");
            assert!(exec.kinds.lock().unwrap().is_empty(), "no command from an unproven main runs");
            assert!(notify.main.lock().unwrap().is_none(), "never reported as connected");
            assert_eq!(*notify.errors.lock().unwrap(), [client::MAIN_UNPROVEN]);
        }
    }

    #[test]
    fn pair_again_stops_the_running_client_first_and_keeps_the_entry() {
        let (handle, port) = test_server();
        let exec = Arc::new(FakeExec::default());
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "Gnowee".into(), ..Default::default() };
        // Pairing on its own connection, ended once the credentials are kept.
        let pair = |notify: Arc<FakeClientNotify>| {
            let (code, _) = handle.open_pairing(now_ms());
            let stop = Arc::new(AtomicBool::new(false));
            let (c, e, n, s) = (config.clone(), exec.clone(), notify.clone(), stop.clone());
            let t = std::thread::spawn(move || client::run_once(&c, e, n, &s, Some(&code)));
            wait_until(|| notify.creds.lock().unwrap().is_some());
            stop.store(true, Ordering::SeqCst);
            assert_eq!(t.join().unwrap(), Ok(()));
            notify.creds.lock().unwrap().clone().unwrap()
        };
        let (id, token) = pair(Arc::new(FakeClientNotify::default()));
        // The client runs with those credentials and is connected as that entry.
        let linked = NetworkConfig { assistant_id: id.clone(), token: token.clone(), ..config.clone() };
        let (e, n) = (exec.clone(), Arc::new(FakeClientNotify::default()));
        let running = client::ClientHandle::spawn(move |stop| {
            let _ = client::run_once(&linked, e, n, &stop, None);
        });
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        // "Pair again": the running client is stopped and gone before the pair is sent.
        let (again, new_token) = client::pair_after_stopping(Some(running), || Ok::<_, String>(pair(Arc::new(FakeClientNotify::default())))).unwrap();
        assert_eq!(again, id, "the same machine keeps its entry");
        assert_ne!(new_token, token);
        let status = handle.status();
        assert_eq!(status.assistants.len(), 1, "no twin");
        assert_eq!(status.assistants[0].name, "Gnowee");
        handle.stop();
    }

    #[test]
    fn the_client_pairs_sends_its_board_runs_a_command_reconnects_and_is_removed() {
        let (handle, port) = test_server();
        let exec = Arc::new(FakeExec::default());
        let notify = Arc::new(FakeClientNotify::default());
        let stop = Arc::new(AtomicBool::new(false));
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), ..Default::default() };
        let run = |config: NetworkConfig, code: Option<String>| {
            let (e, n, s) = (exec.clone(), notify.clone(), stop.clone());
            std::thread::spawn(move || client::run_once(&config, e, n, &s, code.as_deref()))
        };
        // No pairing window is open, so any code is wrong.
        let err = run(config.clone(), Some("123456".into())).join().unwrap().unwrap_err();
        assert_eq!(err, "Wrong or expired pairing code.");
        let (code, _) = handle.open_pairing(now_ms());
        let t = run(config.clone(), Some(code));
        wait_until(|| handle.boards().iter().any(|b| b.machine == "laptop" && b.cards.len() == 1 && b.dirs == ["proj"]));
        assert_eq!(notify.main.lock().unwrap().clone(), Some(crate::net::local_hostname()));
        let mut named = NetworkConfig::default();
        named.name = "  desk  ".into();
        assert_eq!(main_name_of(&named), "desk", "a configured name is what assistants see");
        assert_eq!(main_name_of(&NetworkConfig::default()), crate::net::local_hostname());
        let (id, token) = notify.creds.lock().unwrap().clone().expect("the client was paired");
        // A command from the main runs on the client's executor and its result comes back.
        let out = send_command_with(&handle.shared, "laptop", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5));
        assert_eq!(out, Ok(None));
        assert_eq!(*exec.kinds.lock().unwrap(), [CommandKind::Compact { session: "r1".into() }]);
        stop.store(true, Ordering::SeqCst);
        assert_eq!(t.join().unwrap(), Ok(()), "a stop ends the connection cleanly");
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        // With the token it reconnects by challenge-response; removal ends it for good.
        stop.store(false, Ordering::SeqCst);
        let t = run(NetworkConfig { assistant_id: id.clone(), token, ..config }, None);
        wait_until(|| handle.status().assistants.iter().any(|a| a.connected));
        handle.remove_assistant(&id);
        assert_eq!(t.join().unwrap().unwrap_err(), client::REMOVED);
        assert_eq!(notify.removed.load(Ordering::SeqCst), 1);
        handle.stop();
    }
}
