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
//! in the app's lib.rs), and nothing takes the server's mutex while holding `store`.

use super::merge::{display_names, RemoteBoard};
use super::identity::{encode_key, Keypair};
use super::protocol::{decode_up, encode, new_id, Attempts, CommandKind, Down, PairingWindow, Request, Up, CAP_CHAT, CAP_FOLLOW, CAP_LEAD, CAP_MIRROR, CAP_WORKTREE, MAX_FRAME, PRE_AUTH_FRAME, PROTOCOL};
use super::secure::{self, Channel, ChannelError, Fail, Got};
use super::routing::MachineInfo;
use super::{AssistantStatus, NetworkStatus, PairingCode};
use crate::config::{NetworkConfig, NetworkRole, PairedAssistant};
use crate::log;
use crate::model::Card;
use crate::remembered::{Key, RememberedRow};
use crate::store::now_ms;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

/// How long one read waits; the connection loop's tick.
const TICK: Duration = Duration::from_millis(100);
/// Mirrors go out at most this often per connection; the last change still goes.
const MIRROR_GAP: Duration = Duration::from_millis(300);
/// A peer silent this long is dropped (assistants ping every 10 s).
const SILENCE: Duration = Duration::from_secs(15);
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
/// The `bye` reason when the main could not save a pairing; the assistant
/// keeps nothing and says "The main could not save the pairing."
pub(crate) const PAIRING_NOT_SAVED: &str = "could not save the pairing";

/// What the server tells the app; the Tauri adapter lives in the app's `lib.rs`.
/// `paired` and `paired_list_changed` are called with the server's mutex
/// held (see the module docs) and must not take it; the others never are.
/// Both saves say whether they reached the disk: a pairing or a removal
/// that could not be saved is not made; a refresh on connect or disconnect
/// (address, last seen) that could not be saved is only logged.
pub trait Notify: Send + Sync {
    fn board_changed(&self);
    /// An assistant's first board when this run of the main holds none for
    /// it (it just paired, or the main just started), before it joins the
    /// boards: nothing on it is announced. Later changes are.
    fn board_seeded(&self, cards: &[crate::model::Card]);
    fn status_changed(&self, status: NetworkStatus);
    fn paired(&self, assistant: &PairedAssistant) -> Result<(), String>;
    fn paired_list_changed(&self, assistants: &[PairedAssistant]) -> Result<(), String>;
    /// A mirroring Maya's request, run on a worker thread with no lock held.
    /// `machine` is `None` for this main itself. The result is what this
    /// main's own page would get for the same action.
    fn request(&self, _machine: Option<&str>, _request: Request) -> Result<Option<Value>, String> {
        Err("This Maya cannot answer requests.".into())
    }
    /// What this main offers a mirroring Maya, sent in `welcome`.
    fn main_caps(&self) -> Vec<String> {
        vec![]
    }
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
    /// What it can do (`CAP_FOLLOW`, `CAP_LEAD`); empty for an older Maya.
    caps: Vec<String>,
    last_seen: u64,
    /// Its last board failed to decode; the one before it is kept.
    unreadable_board: bool,
    /// When its brain went dormant, as its `hello` said; None when it never
    /// ran one (or is an older Maya). Kept in memory only.
    brain_dormant_since: Option<u64>,
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
    /// The last code took its three tries without a pairing.
    code_used_up: bool,
    paired: Vec<PairedAssistant>,
    peers: HashMap<String, Peer>,
    /// A handover to a new main is running; no second one may start.
    handover: bool,
    /// Pairings recorded but not yet registered as connections: a handover
    /// would hand them over without moving them, so it waits for none.
    pairings_in_flight: HashSet<String>,
    /// The main's last board for mirroring Mayas: one board and the serial it was taken at.
    mirror: Option<MirrorBoard>,
    mirror_serial: u64,
    /// The word this main listens for, sent with its board; `None` sends none.
    wake_word: Option<String>,
    /// This main's name, so an assistant with the same name gets a label of
    /// its own (the main's own name means the main in a request).
    main_name: String,
}

/// The main's board as its page shows it; local cards have no `machine`.
struct MirrorBoard {
    serial: u64,
    cards: Vec<Card>,
    remembered: Vec<RememberedRow>,
}

impl Server {
    fn new(paired: Vec<PairedAssistant>, main_name: String) -> Server {
        Server { boards: vec![], pairing: None, by_id: HashMap::new(), conns: HashMap::new(), pending: HashMap::new(), next_id: 0, attempts: Attempts::default(), code_used_up: false, paired, peers: HashMap::new(), handover: false, pairings_in_flight: HashSet::new(), mirror: None, mirror_serial: 0, wake_word: None, main_name }
    }

    /// The entry `id` lets its Maya mirror.
    fn may_mirror(&self, id: &str) -> bool {
        self.paired.iter().any(|p| p.id == id && p.mirror)
    }

    /// The machines a mirroring Maya can pick: this main first, then every
    /// paired assistant but `receiver`.
    fn mirror_machines(&self, main_name: &str, receiver: &str) -> Vec<MachineInfo> {
        let me = MachineInfo { name: main_name.into(), hostname: super::local_hostname(), platform: std::env::consts::OS.into(), connected: true };
        let others = self.paired.iter().zip(self.labels()).filter(|(p, _)| p.id != receiver).map(|(p, (id, label))| MachineInfo { name: label, hostname: p.hostname.clone(), platform: p.platform.clone(), connected: self.conns.contains_key(&id) });
        std::iter::once(me).chain(others).collect()
    }

    /// Makes the next tick send the board again to every mirroring Maya.
    fn bump_mirror(&mut self) {
        if let Some(board) = self.mirror.take() {
            self.mirror_serial += 1;
            self.mirror = Some(MirrorBoard { serial: self.mirror_serial, cards: board.cards, remembered: board.remembered });
        }
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
        // The main joins the comparison but not the result: an assistant
        // named like it carries its address too.
        let entries: Vec<(String, String)> = std::iter::once((self.main_name.clone(), String::new())).chain(self.paired.iter().map(|p| (p.name.clone(), p.address.clone()))).collect();
        self.paired.iter().zip(display_names(&entries).into_iter().skip(1)).map(|(p, label)| (p.id.clone(), label)).collect()
    }

    /// Records a pairing by the Maya with `key`. An entry with that key is
    /// the same machine pairing again: it keeps its id, takes the names
    /// just given, and its old board goes. Otherwise it is a new entry.
    /// Returns the entry, and what it was before (for `undo_pairing`).
    fn record_pairing(&mut self, key: &str, address: &str, name: &str, hostname: &str, platform: &str, mirror: bool) -> (PairedAssistant, Option<PairedAssistant>) {
        let again = self.paired.iter().position(|p| p.key == key);
        let Some(i) = again else {
            let a = PairedAssistant { id: new_id(), name: name.into(), hostname: hostname.into(), platform: platform.into(), key: key.into(), address: address.into(), last_seen: None, mirror };
            self.paired.push(a.clone());
            self.pairings_in_flight.insert(a.id.clone());
            return (a, None);
        };
        let previous = self.paired[i].clone();
        let p = &mut self.paired[i];
        p.name = name.into();
        p.hostname = hostname.into();
        p.platform = platform.into();
        p.address = address.into();
        p.mirror = mirror;
        let a = p.clone();
        self.pairings_in_flight.insert(a.id.clone());
        self.by_id.remove(&a.id);
        self.peers.remove(&a.id);
        self.sync_boards();
        (a, Some(previous))
    }

    /// Takes back a pairing the assistant never completed: a new entry goes,
    /// an existing one gets its old names back.
    fn undo_pairing(&mut self, id: &str, previous: Option<PairedAssistant>) {
        self.pairings_in_flight.remove(id);
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
                self.by_id.get(&id).map(|b| RemoteBoard { machine: label, hostname: p.hostname.clone(), platform: p.platform.clone(), address: p.address.clone(), worktrees: self.peers.get(&id).is_some_and(|p| p.caps.iter().any(|c| c == CAP_WORKTREE)), ..b.clone() })
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
                let connected = self.conns.contains_key(&p.id);
                let can = |cap: &str| connected && peer.is_some_and(|x| x.caps.iter().any(|c| c == cap));
                AssistantStatus { id: p.id.clone(), name, hostname: p.hostname.clone(), platform: p.platform.clone(), address: p.address.clone(), connected, last_seen: peer.map(|x| x.last_seen).or(p.last_seen), note: peer.and_then(Peer::note), can_lead: can(CAP_LEAD), can_follow: can(CAP_FOLLOW), mirror: p.mirror, wants_mirror: can(CAP_MIRROR) }
            })
            .collect();
        NetworkStatus { role: NetworkRole::Main, code, assistants, addresses: super::local_addresses(), code_used_up: self.code_used_up, ..Default::default() }
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
    /// This main's key pair: assistants pin its public half.
    identity: Arc<Keypair>,
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
    pub fn open_pairing(&self, now_ms: u64) -> Result<(String, u64), String> {
        let (code, expires, status) = {
            let mut s = lock(&self.shared);
            if s.handover {
                log::line("network", "pairing stays closed during a handover");
                return Err("A handover is already running.".into());
            }
            let w = PairingWindow::new(now_ms);
            let out = (w.code.clone(), w.expires_at_ms);
            s.code_used_up = false;
            s.pairing = Some(w);
            (out.0, out.1, s.status(now_ms))
        };
        log::line("network", "pairing open for five minutes");
        self.ctx.notify.status_changed(status);
        Ok((code, expires))
    }

    /// Forgets an assistant: it is told `bye removed`, its cards go, and its
    /// key no longer opens a connection. The shorter list is saved first;
    /// if that fails nothing changes and the error comes back.
    pub fn remove_assistant(&self, id: &str) -> Result<(), String> {
        let (label, status) = {
            let mut s = lock(&self.shared);
            let label = s.label(id);
            let after: Vec<PairedAssistant> = s.paired.iter().filter(|p| p.id != id).cloned().collect();
            self.ctx.notify.paired_list_changed(&after)?;
            s.paired = after;
            if let Some(c) = s.conns.remove(id) {
                let _ = c.tx.send(Down::Bye { reason: "removed".into() });
            }
            s.by_id.remove(id);
            s.peers.remove(id);
            s.sync_boards();
            (label, s.status(now_ms()))
        };
        log::line("network", format!("{label}: removed"));
        self.ctx.notify.status_changed(status);
        self.ctx.notify.board_changed();
        Ok(())
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// This main's public key, as its assistants pin it.
    pub fn public_key(&self) -> [u8; 32] {
        self.ctx.identity.public
    }

    pub fn status(&self) -> NetworkStatus {
        lock(&self.shared).status(now_ms())
    }

    pub fn boards(&self) -> Vec<RemoteBoard> {
        lock(&self.shared).boards.clone()
    }

    /// The main's board as its page shows it (local cards with no machine),
    /// for every mirroring Maya; each connection sends it on its next tick.
    pub fn mirror(&self, cards: Vec<Card>) {
        let mut s = lock(&self.shared);
        s.mirror_serial += 1;
        let remembered = s.mirror.as_ref().map(|b| b.remembered.clone()).unwrap_or_default();
        s.mirror = Some(MirrorBoard { serial: s.mirror_serial, cards, remembered });
    }

    /// Sends a Chat frame to every connected Maya that mirrors, asked for
    /// Chat and is allowed; the rest hear nothing.
    pub fn to_mirrors(&self, frame: Down) {
        let s = lock(&self.shared);
        for (id, conn) in &s.conns {
            let asked = s.peers.get(id).is_some_and(|p| p.caps.iter().any(|c| c == CAP_MIRROR) && p.caps.iter().any(|c| c == CAP_CHAT));
            if asked && s.may_mirror(id) {
                let _ = conn.tx.send(frame.clone());
            }
        }
    }

    /// The word this main listens for, which mirroring Mayas answer to as
    /// well; a change goes out with the board on the next tick.
    pub fn mirror_wake_word(&self, word: Option<String>) {
        let mut s = lock(&self.shared);
        if s.wake_word != word {
            s.wake_word = word;
            s.bump_mirror();
        }
    }

    /// The merged "was running" list for mirroring Mayas; it goes out with
    /// the board on the next tick when it changed.
    pub fn mirror_remembered(&self, rows: Vec<RememberedRow>) {
        let mut s = lock(&self.shared);
        match &s.mirror {
            Some(b) if b.remembered == rows => return,
            None if rows.is_empty() => return,
            _ => {}
        }
        s.mirror_serial += 1;
        let cards = s.mirror.as_ref().map(|b| b.cards.clone()).unwrap_or_default();
        s.mirror = Some(MirrorBoard { serial: s.mirror_serial, cards, remembered: rows });
    }

    /// Forgets `keys` from the list the assistant shown as `machine` last
    /// sent, so a Dismiss clears the banner even for one that is away. It
    /// sends its own list again when it reconnects. Whether that assistant
    /// is connected now, so a `Forget` can follow.
    pub fn drop_remembered(&self, machine: &str, keys: &[Key]) -> bool {
        let mut s = lock(&self.shared);
        let Some(id) = s.id_for(machine) else { return false };
        if let Some(list) = s.by_id.get_mut(&id).and_then(|b| b.remembered.as_mut()) {
            list.retain(|e| !keys.contains(&e.key()));
        }
        s.sync_boards();
        s.conns.contains_key(&id)
    }

    /// Lets the Maya `id` see and control every session, or stops it. Saved
    /// first; if that fails nothing changes. Off tells a connected Maya at
    /// once; on sends it the board again.
    pub fn set_mirror(&self, id: &str, on: bool) -> Result<(), String> {
        let (label, status) = {
            let mut s = lock(&self.shared);
            let mut after = s.paired.clone();
            let entry = after.iter_mut().find(|p| p.id == id).ok_or("That Maya is no longer paired.")?;
            entry.mirror = on;
            self.ctx.notify.paired_list_changed(&after)?;
            s.paired = after;
            if on {
                s.bump_mirror();
            } else if let Some(c) = s.conns.get(id) {
                let _ = c.tx.send(Down::Mirror { allowed: false, cards: vec![], machines: vec![], remembered: vec![], wake_word: None });
            }
            (s.label(id), s.status(now_ms()))
        };
        log::line("network", format!("{label}: {}", if on { "may see and control every session" } else { "no longer sees the sessions" }));
        self.ctx.notify.status_changed(status);
        // The app refreshes and mirrors the board (it may hold none yet).
        if on {
            self.ctx.notify.board_changed();
        }
        Ok(())
    }

    /// Stops accepting and closes every connection within a tick; does not wait.
    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            log::line("network", "server stopped");
        }
    }
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

pub fn start_with(notify: Arc<dyn Notify>, config_view: Arc<Mutex<NetworkConfig>>, port: u16, identity: Arc<Keypair>) -> Result<ServerHandle, String> {
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("Could not listen on port {port}: {e}. Choose another port."))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (paired, main_name) = {
        let c = lock(&config_view);
        (c.assistants.clone(), main_name_of(&c))
    };
    let shared = Arc::new(Mutex::new(Server::new(paired, main_name.clone())));
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = Arc::new(Ctx { shared: shared.clone(), notify, stop: stop.clone(), main_name, identity, live: AtomicUsize::new(0), pre_auth: Mutex::new(HandshakeSlots::default()) });
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
type Chan = Channel<TcpStream>;

/// A connection that got through its handshake and `hello`.
struct Link {
    id: String,
    serial: u64,
    label: String,
    rx: Receiver<Down>,
    /// No board has arrived on this connection yet.
    first_board: std::cell::Cell<bool>,
    /// Its `hello` asked for the main's board.
    wants_mirror: bool,
    /// The serial of the last mirror sent on this connection, and when.
    mirror_sent: std::cell::Cell<u64>,
    mirror_at: std::cell::Cell<Option<Instant>>,
}

/// Counts a connection as pre-auth until dropped (`welcome` sent, or it failed).
struct PreAuth<'a>(&'a Ctx, IpAddr);

impl Drop for PreAuth<'_> {
    fn drop(&mut self) {
        lock(&self.0.pre_auth).release(self.1);
    }
}

/// One try at the open code for a pairing that is starting from `ip`, or
/// the reason to refuse it.
fn take_try(ctx: &Ctx, ip: &str) -> Result<String, String> {
    let mut s = lock(&ctx.shared);
    let now = now_ms();
    if s.attempts.locked(ip, now) {
        return Err("too many attempts".into());
    }
    s.pairing.as_mut().and_then(|w| w.take_try(now).then(|| w.code.clone())).ok_or_else(|| secure::WRONG_CODE_REASON.into())
}

/// A pairing that took a try did not reach `welcome`: the address counts a
/// failure, and the code goes once its tries are gone. A code another
/// pairing already used is left alone.
fn pairing_failed(ctx: &Ctx, ip: &str) {
    {
        let mut s = lock(&ctx.shared);
        let Some(tries_left) = s.pairing.as_ref().map(|w| w.tries_left) else { return };
        s.attempts.failed(ip, now_ms());
        if tries_left == 0 {
            s.pairing = None;
            s.code_used_up = true;
            log::line("network", "pairing code used up after 3 wrong tries");
        }
    }
    ctx.announce();
}

fn serve(ctx: &Ctx, stream: TcpStream, addr: SocketAddr) {
    let pre_auth = PreAuth(ctx, addr.ip());
    let deadline = Instant::now() + PRE_AUTH_DEADLINE;
    // Read once `upgrade` makes the socket blocking again.
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
    let ip = addr.ip().to_string();
    let mut took_try = false;
    let shook = secure::accept(&mut ws, &ctx.identity, deadline, &|| ctx.stopped(), &mut || {
        let code = take_try(ctx, &ip);
        took_try = code.is_ok();
        code
    });
    let (noise, accepted) = match shook {
        Ok(done) => done,
        Err(f) => {
            if f == Fail::OldPeer {
                // A protocol 1 Maya reads this as "runs a different version".
                let _ = ws.send(Message::text(encode(&Down::Bye { reason: "protocol".into() })));
            }
            if took_try {
                pairing_failed(ctx, &ip);
            }
            log::line("network", format!("{ip}: handshake failed: {f:?}"));
            drop(pre_auth);
            finish_ws(&mut ws);
            return;
        }
    };
    let mut ch = Channel::new(ws, noise);
    let admitted = admit(ctx, &mut ch, addr, &accepted, deadline);
    if admitted.is_none() && took_try {
        pairing_failed(ctx, &ip);
    }
    drop(pre_auth);
    let Some(link) = admitted else {
        finish_ws(&mut ch.ws);
        return;
    };
    let reason = pump(ctx, &mut ch, &link);
    // Forgotten before the close completes: an assistant that waits for its
    // close to be answered (a client stopped for "Pair again") finds the
    // main already counting it as gone.
    leave(ctx, &link, &reason);
    finish_ws(&mut ch.ws);
}

/// The WebSocket upgrade, on a non-blocking socket: tungstenite resumes it
/// only after WouldBlock, and a read timeout is TimedOut on Windows, which
/// would end an upgrade whose request is a read tick late.
fn upgrade(ctx: &Ctx, stream: TcpStream, deadline: Instant) -> Result<Ws, String> {
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default().max_message_size(Some(PRE_AUTH_FRAME)).max_frame_size(Some(PRE_AUTH_FRAME));
    let mut attempt = tungstenite::accept_with_config(stream, Some(config));
    loop {
        match attempt {
            Ok(ws) => {
                ws.get_ref().set_nonblocking(false).map_err(|e| e.to_string())?;
                return Ok(ws);
            }
            Err(HandshakeError::Interrupted(mid)) => {
                if ctx.stopped() || Instant::now() > deadline {
                    return Err("timed out".into());
                }
                std::thread::sleep(secure::HANDSHAKE_POLL);
                attempt = mid.handshake();
            }
            Err(HandshakeError::Failure(e)) => return Err(e.to_string()),
        }
    }
}

fn send_down(ch: &mut Chan, down: &Down) -> Result<(), String> {
    ch.send(&encode(down)).map_err(|e| format!("write failed: {e}"))
}

fn bye(ch: &mut Chan, reason: &str) {
    let _ = send_down(ch, &Down::Bye { reason: reason.into() });
}

/// Closes the WebSocket and waits up to a second for the peer to agree, so
/// a `bye` just sent is read before the socket goes.
fn finish_ws(ws: &mut Ws) {
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

/// The next message that parses, by `deadline`; garbage is logged and skipped.
fn read_up(ctx: &Ctx, ch: &mut Chan, who: &str, deadline: Instant) -> Result<Up, String> {
    loop {
        if ctx.stopped() {
            return Err("the main stopped".into());
        }
        // Checked on every frame, not only on a quiet read, so pings cannot hold it open.
        if Instant::now() > deadline {
            return Err("no answer within 10 s of connecting".into());
        }
        match ch.read() {
            Ok(Got::Text(t)) => match decode_up(&t) {
                Ok(up) => return Ok(up),
                Err(e) => log::line("network", format!("{who}: ignored a message: {e}")),
            },
            Ok(Got::Closed) => return Err("closed".into()),
            Ok(Got::Control | Got::Idle) => {}
            Err(ChannelError::Forged) => {
                log::line("network", format!("{who}: {}", secure::FORGED));
                return Err(secure::FORGED.into());
            }
            Err(ChannelError::Io(e)) => return Err(e),
        }
    }
}

/// Names from the peer, trimmed of control characters and kept short.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(64).collect()
}

fn admit(ctx: &Ctx, ch: &mut Chan, addr: SocketAddr, accepted: &secure::Accepted, deadline: Instant) -> Option<Link> {
    let ip = addr.ip().to_string();
    let hello = match read_up(ctx, ch, &ip, deadline) {
        Ok(up) => up,
        Err(e) => {
            log::line("network", format!("{ip}: no hello: {e}"));
            return None;
        }
    };
    let Up::Hello { protocol, app, name, hostname, platform, caps, brain_dormant_since } = hello else {
        log::line("network", format!("{ip}: first message was not a hello"));
        bye(ch, "protocol");
        return None;
    };
    if protocol != PROTOCOL {
        log::line("network", format!("{ip}: protocol {protocol}, expected {PROTOCOL}"));
        bye(ch, "protocol");
        return None;
    }
    let (hostname, platform, app) = (clean(&hostname), clean(&platform), clean(&app));
    if app != env!("CARGO_PKG_VERSION") {
        log::line("network", format!("{ip}: runs Maya {app}; this Mac runs {}", env!("CARGO_PKG_VERSION")));
    }
    let name = Some(clean(&name)).filter(|n| !n.is_empty()).or_else(|| Some(hostname.clone()).filter(|h| !h.is_empty())).unwrap_or_else(|| "assistant".into());
    let wants_mirror = caps.iter().any(|c| c == CAP_MIRROR);
    let key = encode_key(&accepted.key);
    let (id, pairing) = if accepted.paired {
        let p = pair(ctx, ch, &key, &ip, &name, &hostname, &platform, wants_mirror)?;
        (p.entry.id.clone(), Some(p))
    } else {
        let known = lock(&ctx.shared).paired.iter().find(|p| p.key == key).map(|p| p.id.clone());
        let Some(id) = known else {
            // A key this main does not know: removed, or paired with another main.
            log::line("network", format!("{name}: unknown key; told it was removed"));
            bye(ch, "removed");
            return None;
        };
        (id, None)
    };
    let (tx, rx) = channel();
    // `Err(None)`: no longer paired; `Err(Some(e))`: a fresh pairing that
    // could not be saved.
    let registered: Result<(u64, String, Option<bool>), Option<String>> = {
        let mut s = lock(&ctx.shared);
        // Registered or refused below: either way no longer in flight.
        s.pairings_in_flight.remove(&id);
        if s.is_paired(&id) {
            // The entry follows the machine: its address, and the names it just gave.
            let now = now_ms();
            s.refresh_entry(&id, &ip, &name, &hostname, &platform, now);
            // Saved here, under the lock, before `welcome`: a fresh pairing
            // that cannot be saved is answered with `bye` instead, so neither
            // side keeps it. A refresh that cannot be saved is only logged,
            // and a stopped server's list is no longer the truth.
            let saved = match &pairing {
                Some(_) => s.paired.iter().find(|p| p.id == id).map_or(Ok(()), |entry| ctx.notify.paired(entry)),
                None if !ctx.stopped() => {
                    let _ = ctx.notify.paired_list_changed(&s.paired);
                    Ok(())
                }
                None => Ok(()),
            };
            match saved {
                Ok(()) => {
                    let serial = s.next();
                    s.peers.insert(id.clone(), Peer { app, caps: caps.into_iter().map(|c| clean(&c)).collect(), last_seen: now, unreadable_board: false, brain_dormant_since });
                    // A newer connection replaces an older one; dropping its sender ends it.
                    s.conns.insert(id.clone(), Conn { tx, serial });
                    if let Some(b) = s.by_id.get_mut(&id) {
                        b.connected = true;
                    }
                    s.sync_boards();
                    let offer = wants_mirror.then(|| s.may_mirror(&id));
                    Ok((serial, s.label(&id), offer))
                }
                Err(e) => Err(Some(e)),
            }
        } else {
            Err(None)
        }
    };
    let (serial, label, offer) = match registered {
        Ok(r) => r,
        Err(None) => {
            bye(ch, "removed");
            return None;
        }
        Err(Some(e)) => {
            if let Some(p) = pairing {
                p.undo(ctx, &format!("could not save it: {e}"));
            }
            bye(ch, PAIRING_NOT_SAVED);
            return None;
        }
    };
    let link = Link { id, serial, label, rx, first_board: std::cell::Cell::new(true), wants_mirror, mirror_sent: std::cell::Cell::new(0), mirror_at: std::cell::Cell::new(None) };
    // Authenticated: boards and results may now be large. Sealing adds a
    // little to each frame, which `open_frame` caps at `MAX_FRAME`.
    ch.ws.set_config(|c| {
        c.max_message_size = Some(MAX_FRAME + MAX_FRAME / 1024);
        c.max_frame_size = Some(MAX_FRAME + MAX_FRAME / 1024);
    });
    if let Err(e) = send_down(ch, &Down::Welcome { name: ctx.main_name.clone(), mirror: offer, caps: ctx.notify.main_caps() }) {
        // Undone first, so the list `leave` saves no longer holds the
        // pairing it takes back.
        if let Some(p) = pairing {
            p.undo(ctx, &e);
        }
        leave(ctx, &link, &e);
        return None;
    }
    log::line("network", format!("{}: connected from {ip}", link.label));
    ctx.announce();
    ctx.notify.board_changed();
    Some(link)
}

/// A pairing recorded but not yet saved: it is saved just before `welcome`,
/// and undone if the save fails or `welcome` cannot be sent, so a Maya that
/// was never welcomed leaves no entry behind.
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

/// Records the pairing of a Maya that proved the code, unless the code
/// was used in the meantime (one code, one machine).
#[allow(clippy::too_many_arguments)]
fn pair(ctx: &Ctx, ch: &mut Chan, key: &str, ip: &str, name: &str, hostname: &str, platform: &str, mirror: bool) -> Option<NewPairing> {
    let outcome = {
        let mut s = lock(&ctx.shared);
        if s.pairing.is_none() {
            None
        } else {
            let pairing = s.record_pairing(key, ip, name, hostname, platform, mirror);
            s.pairing = None;
            Some(pairing)
        }
    };
    let Some((entry, previous)) = outcome else {
        log::line("network", format!("{ip}: pairing refused: the code was used meanwhile"));
        bye(ch, secure::WRONG_CODE_REASON);
        return None;
    };
    log::line("network", format!("{name}: {} from {ip}", if previous.is_some() { "paired again (same key)" } else { "paired" }));
    Some(NewPairing { entry, previous })
}

/// Why a connection ends when the main no longer counts it: a newer
/// connection replaced it, or the assistant was removed (`bye` queued).
const NOT_CURRENT: &str = "replaced by a newer connection";

/// Writes the frames queued for this connection, up to a `bye` or `move`. `Some(why)`
/// when the connection ends: the `bye` went out, a write failed, or the
/// sender is gone (a newer connection replaced this one).
fn send_queued(ch: &mut Chan, link: &Link) -> Option<String> {
    loop {
        match link.rx.try_recv() {
            Ok(down) => {
                let last = matches!(down, Down::Bye { .. } | Down::Move { .. });
                let why = if matches!(down, Down::Move { .. }) { "moved" } else { "removed" };
                if let Err(e) = send_down(ch, &down) {
                    return Some(e);
                }
                if last {
                    return Some(why.into());
                }
            }
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => return Some(NOT_CURRENT.into()),
        }
    }
}

/// The mirror this connection is owed, if any: a newer board than the last
/// one sent, no sooner than `MIRROR_GAP` after it, to a Maya allowed it.
fn mirror_due(ctx: &Ctx, link: &Link) -> Option<Down> {
    if !link.wants_mirror || link.mirror_at.get().is_some_and(|t| t.elapsed() < MIRROR_GAP) {
        return None;
    }
    let s = lock(&ctx.shared);
    let board = s.mirror.as_ref().filter(|b| b.serial != link.mirror_sent.get())?;
    if !s.may_mirror(&link.id) {
        return None;
    }
    let cards = board
        .cards
        .iter()
        .filter(|c| c.machine.as_deref() != Some(link.label.as_str()))
        .map(|c| {
            let mut c = c.clone();
            if c.machine.is_none() {
                c.machine = Some(ctx.main_name.clone());
                c.machine_platform = Some(std::env::consts::OS.into());
            }
            c
        })
        .collect();
    let remembered = board
        .remembered
        .iter()
        .cloned()
        .map(|mut r| {
            r.machine.get_or_insert_with(|| ctx.main_name.clone());
            r
        })
        .collect();
    link.mirror_sent.set(board.serial);
    link.mirror_at.set(Some(Instant::now()));
    Some(Down::Mirror { allowed: true, cards, machines: s.mirror_machines(&ctx.main_name, &link.id), remembered, wake_word: s.wake_word.clone() })
}

/// Runs a registered connection until it closes; returns why.
fn pump(ctx: &Ctx, ch: &mut Chan, link: &Link) -> String {
    let mut deadline = Instant::now() + SILENCE;
    loop {
        if ctx.stopped() {
            return "the main stopped".into();
        }
        // Checked on every turn, so pings cannot keep a silent peer connected.
        if Instant::now() > deadline {
            return "silent for 15 s".into();
        }
        if let Some(why) = send_queued(ch, link) {
            return why;
        }
        if let Some(mirror) = mirror_due(ctx, link) {
            if let Err(e) = send_down(ch, &mirror) {
                return e;
            }
        }
        match ch.read() {
            Ok(Got::Text(t)) => {
                deadline = Instant::now() + SILENCE;
                match decode_up(&t) {
                    Ok(up) => match handle(ctx, ch, link, up) {
                        Ok(()) => {}
                        // Removed while this frame was in flight: the `bye`
                        // `remove_assistant` queued (under the same lock
                        // `handle` checked) goes out before the close.
                        Err(e) if e == NOT_CURRENT => return send_queued(ch, link).unwrap_or(e),
                        Err(e) => return e,
                    },
                    Err(e) if unreadable_board(&t) => {
                        // Keep the last good board; say why it no longer updates.
                        log::line("network", format!("{}: could not read its board: {e}", link.label));
                        keep_board_fresh(ctx, link);
                        mark_board(ctx, link, true);
                    }
                    Err(e) => log::line("network", format!("{}: ignored a message: {e}", link.label)),
                }
            }
            Ok(Got::Closed) => return "closed by the assistant".into(),
            // Unauthenticated, so never proof the assistant is alive.
            Ok(Got::Control | Got::Idle) => {}
            Err(ChannelError::Forged) => {
                log::line("network", format!("{}: {}", link.label, secure::FORGED));
                return secure::FORGED.into();
            }
            Err(ChannelError::Io(e)) => return e,
        }
    }
}

fn handle(ctx: &Ctx, ch: &mut Chan, link: &Link, up: Up) -> Result<(), String> {
    match up {
        Up::Board { cards, dirs, agents, remembered } => {
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
                    s.by_id.insert(link.id.clone(), RemoteBoard { machine: String::new(), hostname: String::new(), platform: String::new(), address: String::new(), cards, dirs, agents, remembered, worktrees: false, received_at: now_ms(), connected: true });
                    s.sync_boards();
                }
                current
            };
            if !current {
                return Err(NOT_CURRENT.into());
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
            send_down(ch, &Down::Pong)?;
        }
        Up::Request { id, machine, request } => {
            let (allowed, tx) = {
                let mut s = lock(&ctx.shared);
                s.touch(&link.id);
                let tx = s.conns.get(&link.id).filter(|c| c.serial == link.serial).map(|c| c.tx.clone());
                (link.wants_mirror && s.may_mirror(&link.id), tx)
            };
            let Some(tx) = tx else { return Err(NOT_CURRENT.into()) };
            let refusal = if !allowed {
                Some(format!("{} has not allowed this Maya to control its sessions.", ctx.main_name))
            } else if matches!(request, Request::Command(CommandKind::Promote { .. })) {
                Some("That cannot be requested.".to_string())
            } else {
                None
            };
            if let Some(error) = refusal {
                send_down(ch, &Down::Response { id, ok: false, error: Some(error), data: None })?;
                return Ok(());
            }
            // The main's own name, like none, means the main.
            let machine = machine.map(|m| clean(&m)).filter(|m| !m.is_empty() && *m != ctx.main_name);
            let (notify, label) = (ctx.notify.clone(), link.label.clone());
            let spawned = std::thread::Builder::new().name("net-request".into()).spawn(move || {
                let down = match notify.request(machine.as_deref(), request) {
                    Ok(data) => Down::Response { id, ok: true, error: None, data },
                    Err(e) => {
                        log::line("network", format!("{label}: request {id} failed: {e}"));
                        Down::Response { id, ok: false, error: Some(e), data: None }
                    }
                };
                let _ = tx.send(down);
            });
            if let Err(e) = spawned {
                send_down(ch, &Down::Response { id, ok: false, error: Some(format!("no thread for the request: {e}")), data: None })?;
            }
        }
        Up::Hello { .. } => log::line("network", format!("{}: ignored a hello after welcome", link.label)),
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
                let _ = ctx.notify.paired_list_changed(&s.paired);
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
        CommandKind::Stop { .. } => "stop",
        CommandKind::Pick { .. } => "pick",
        CommandKind::Compact { .. } => "compact",
        CommandKind::Close { .. } => "close",
        CommandKind::Forget { .. } => "forget",
        CommandKind::Rename { .. } => "rename",
        CommandKind::SetOption { .. } => "set_option",
        CommandKind::CycleMode { .. } => "cycle_mode",
        CommandKind::Slash { .. } => "slash",
        CommandKind::Start { .. } => "start",
        CommandKind::Resume { .. } => "resume",
        CommandKind::ListResumable { .. } => "list_resumable",
        CommandKind::History { .. } => "history",
        CommandKind::Promote { .. } => "promote",
    }
}

/// The end of the error when an assistant does not answer a command in time.
pub(crate) const NO_ANSWER: &str = "did not answer in time";
const LEFT_BEFORE_ANSWER: &str = "disconnected before answering.";
/// How long the new main has to answer `promote`.
const PROMOTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the old main waits for moved connections to close.
const MOVE_WAIT: Duration = Duration::from_secs(3);

/// Sends `kind` to the assistant shown as `machine` and waits for its result.
pub fn send_command_with(shared: &Arc<Mutex<Server>>, machine: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String> {
    let Some(id) = lock(shared).id_for(machine) else {
        return Err(format!("{machine} is not connected"));
    };
    send_command_to(shared, &id, kind, timeout)
}

/// Sends `kind` to the assistant `id` and waits for its result.
pub fn send_command_to(shared: &Arc<Mutex<Server>>, id: &str, kind: CommandKind, timeout: Duration) -> Result<Option<Value>, String> {
    let name = kind_name(&kind);
    let (rx, cmd, machine) = {
        let mut s = lock(shared);
        let machine = s.label(id);
        let conn = s.conns.get(id).map(|c| (c.tx.clone(), c.serial));
        let Some((tx, serial)) = conn else {
            return Err(format!("{machine} is not connected"));
        };
        let cmd = s.next();
        let (rtx, rrx) = channel();
        s.pending.insert(cmd, Pending { conn: serial, tx: rtx });
        if tx.send(Down::Command { id: cmd, kind }).is_err() {
            s.pending.remove(&cmd);
            return Err(format!("{machine} is not connected"));
        }
        (rrx, cmd, machine)
    };
    log::line("network", format!("{machine}: command {cmd} {name}"));
    let out = match rx.recv_timeout(timeout) {
        Ok(Err(e)) if e == DISCONNECTED => Err(format!("{machine} {LEFT_BEFORE_ANSWER}")),
        Ok(r) => r,
        Err(RecvTimeoutError::Timeout) => {
            lock(shared).pending.remove(&cmd);
            Err(format!("{machine} {NO_ANSWER}"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(format!("{machine} {LEFT_BEFORE_ANSWER}")),
    };
    if let Err(e) = &out {
        log::line("network", format!("{machine}: command {cmd} {name} failed: {e}"));
    }
    out
}

/// What the old main needs once its assistant became the main: where the
/// new main is, its key, and who followed.
#[derive(Debug, Clone, PartialEq)]
pub struct Promotion {
    pub host: String,
    pub port: u16,
    /// The new main's public key, for the old main to pin.
    pub main_key: String,
    pub moved: Vec<String>,
    pub left_behind: Vec<String>,
}

/// Clears `handover` when the handover ends, however it ends.
struct HandoverTurn<'a>(&'a Arc<Mutex<Server>>);

impl Drop for HandoverTurn<'_> {
    fn drop(&mut self) {
        lock(self.0).handover = false;
    }
}

/// The Maya a handover goes to, for its summary.
#[derive(Clone, Debug, PartialEq)]
pub struct HandoverTo {
    pub label: String,
    /// When its brain went dormant; None when it never ran one.
    pub brain_dormant_since: Option<u64>,
}

impl ServerHandle {
    /// Makes the assistant `id` the main: closes pairing, sends it the paired
    /// list with this main's own new entry, and once
    /// it answers with its port, sends `move` to every other connected
    /// assistant that can follow and waits for those connections to close.
    /// The caller then saves this Maya as an assistant of `host:port`.
    pub fn promote(&self, id: &str) -> Result<Promotion, String> {
        self.promote_with_timeout(id, PROMOTE_TIMEOUT)
    }

    /// `promote`, with a handover summary from `summary`: called once the
    /// handover is checked and begun, with no lock held, before `promote`
    /// goes out. It must bound its own wait.
    pub fn promote_with_summary(&self, id: &str, summary: impl FnOnce(&HandoverTo) -> Option<String>) -> Result<Promotion, String> {
        self.promote_inner(id, PROMOTE_TIMEOUT, summary)
    }

    pub fn promote_with_timeout(&self, id: &str, timeout: Duration) -> Result<Promotion, String> {
        self.promote_inner(id, timeout, |_| None)
    }

    fn promote_inner(&self, id: &str, timeout: Duration, summary: impl FnOnce(&HandoverTo) -> Option<String>) -> Result<Promotion, String> {
        let (label, host, main_key, list, to) = {
            let mut s = lock(&self.shared);
            if s.handover {
                return Err("A handover is already running.".into());
            }
            let label = s.label(id);
            let entry = s.paired.iter().find(|p| p.id == id).cloned().ok_or_else(|| format!("{label} is no longer paired."))?;
            if crate::net::identity::decode_key(&entry.key).is_err() {
                return Err(format!("{label} has no key; pair it again."));
            }
            if !s.conns.contains_key(id) {
                return Err(format!("{label} is not connected."));
            }
            // A phone only mirrors a main; an older one may still say it can lead.
            if entry.mirror || s.peers.get(id).is_some_and(|p| p.caps.iter().any(|c| c == CAP_MIRROR)) {
                return Err("A phone cannot be the main.".into());
            }
            if !s.peers.get(id).is_some_and(|p| p.caps.iter().any(|c| c == CAP_LEAD)) {
                return Err(format!("{label} cannot become a main."));
            }
            if !s.pairings_in_flight.is_empty() {
                return Err("A Maya is pairing right now; try again in a moment.".into());
            }
            s.handover = true;
            s.pairing = None;
            let me = super::promote::own_entry(&self.ctx.main_name, &self.ctx.identity.public_b64());
            let list = super::promote::handover_list(&s.paired, id, &me);
            let peer = s.peers.get(id);
            let to = HandoverTo { label: label.clone(), brain_dormant_since: peer.and_then(|p| p.brain_dormant_since) };
            (label, entry.address, entry.key, list, to)
        };
        let _turn = HandoverTurn(&self.shared);
        self.ctx.announce();
        let summary = summary(&to);
        log::line("network", format!("{label}: handing over the main role ({} assistants{})", list.len(), if summary.is_some() { ", with a summary" } else { "" }));
        let data = send_command_to(&self.shared, id, CommandKind::Promote { list, summary }, timeout).map_err(|e| {
            // A dropped connection leaves the same doubt as silence: G may already be a main.
            if e.ends_with(NO_ANSWER) || e.ends_with(LEFT_BEFORE_ANSWER) {
                format!("{label} did not answer; it may have become a main on its own. Check it before trying again.")
            } else {
                format!("{label} could not become the main: {}.", e.trim_end_matches('.'))
            }
        })?;
        let port = data.as_ref().and_then(|d| d.get("port")).and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).ok_or_else(|| format!("{label} could not become the main: it did not say its port."))?;
        let (mut moved, mut left_behind, mut moving) = (vec![], vec![], vec![]);
        {
            let s = lock(&self.shared);
            for (other, conn) in s.conns.iter().filter(|(other, _)| other.as_str() != id) {
                let follows = s.peers.get(other).is_some_and(|p| p.caps.iter().any(|c| c == CAP_FOLLOW));
                if follows && conn.tx.send(Down::Move { host: host.clone(), port, key: main_key.clone() }).is_ok() {
                    moved.push(s.label(other));
                    moving.push(other.clone());
                } else {
                    left_behind.push(s.label(other));
                }
            }
        }
        // `move` ends each connection once written; the server must not stop before.
        let end = Instant::now() + MOVE_WAIT;
        while Instant::now() < end && moving.iter().any(|m| lock(&self.shared).conns.contains_key(m)) {
            std::thread::sleep(TICK);
        }
        moved.sort();
        left_behind.sort();
        log::line("network", format!("{label} is the main now at {host}:{port}; moved {}", if moved.is_empty() { "nobody".to_string() } else { moved.join(", ") }));
        Ok(Promotion { host, port, main_key, moved, left_behind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{NetworkConfig, NetworkRole};
    use crate::model::{Card, Harness, State};
    use crate::net::client;
    use std::sync::atomic::AtomicBool;
    use crate::net::identity::Keypair;
    use crate::net::protocol::{decode_down, encode, CommandKind, Down, Request, Up, CAP_VOICE, PROTOCOL};
    use crate::net::secure::{self, Channel, Got};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    pub(crate) type Client = Channel<MaybeTlsStream<TcpStream>>;
    pub(crate) type RawWs = WebSocket<MaybeTlsStream<TcpStream>>;

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
        /// Every save fails while set (a full disk, a read-only config).
        pub fail_saves: AtomicBool,
        /// What `main_caps` answers (`CAP_VOICE` for a main that interprets).
        pub caps: Mutex<Vec<String>>,
        /// Requests a mirroring Maya made, with the machine asked.
        pub requests: Mutex<Vec<(Option<String>, Request)>>,
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
            let fresh: Vec<Card> = n.take_new(&cards).into_iter().chain(n.take_finished(&cards, &HashSet::new())).collect();
            self.announced.lock().unwrap().extend(fresh.into_iter().map(|c| c.session_id));
        }
        fn board_seeded(&self, cards: &[Card]) {
            self.notifier.lock().unwrap().seed(cards);
        }
        fn status_changed(&self, _: NetworkStatus) {
            self.statuses.fetch_add(1, Ordering::SeqCst);
        }
        fn paired(&self, assistant: &crate::config::PairedAssistant) -> Result<(), String> {
            self.check_under_lock();
            if self.fail_saves.load(Ordering::SeqCst) {
                return Err("disk full".into());
            }
            self.paired.fetch_add(1, Ordering::SeqCst);
            let mut saved = self.saved.lock().unwrap();
            match saved.iter_mut().find(|a| a.id == assistant.id) {
                Some(a) => *a = assistant.clone(),
                None => saved.push(assistant.clone()),
            }
            Ok(())
        }
        fn paired_list_changed(&self, list: &[crate::config::PairedAssistant]) -> Result<(), String> {
            self.check_under_lock();
            if self.fail_saves.load(Ordering::SeqCst) {
                return Err("disk full".into());
            }
            *self.saved.lock().unwrap() = list.to_vec();
            Ok(())
        }
        fn main_caps(&self) -> Vec<String> {
            self.caps.lock().unwrap().clone()
        }
        fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
            if let Request::Command(CommandKind::Compact { session }) = &request {
                if session == "slow" {
                    std::thread::sleep(Duration::from_millis(800));
                }
            }
            let answer = serde_json::json!(format!("{request:?}"));
            self.requests.lock().unwrap().push((machine.map(str::to_string), request));
            Ok(Some(answer))
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
        let handle = start_with(counter.clone(), Arc::new(Mutex::new(NetworkConfig::default())), 0, Arc::new(Keypair::generate())).unwrap();
        let _ = counter.server.set(handle.shared.clone());
        let port = handle.port();
        (handle, port)
    }

    pub(crate) fn test_server() -> (ServerHandle, u16) {
        test_server_with(Arc::new(Counter::default()))
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    /// A WebSocket to the test main, with a 5 s read timeout.
    pub(crate) fn connect(port: u16) -> RawWs {
        let (ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/")).unwrap();
        if let MaybeTlsStream::Plain(s) = ws.get_ref() {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        }
        ws
    }

    #[test]
    fn welcome_names_what_the_main_can_do() {
        let counter = Arc::new(Counter::default());
        *counter.caps.lock().unwrap() = vec![CAP_VOICE.into()];
        let (handle, port) = test_server_with(counter.clone());
        let (ch, id, me) = pair_client(&handle, port, "Pixel");
        drop(ch);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ch = connect_as(&handle, port, &me, "Pixel", &["mirror"]);
        let Down::Welcome { caps, .. } = recv(&mut ch) else { panic!("expected welcome") };
        assert_eq!(caps, vec![CAP_VOICE.to_string()]);
        handle.stop();
    }

    pub(crate) fn hello_up(name: &str, caps: &[&str]) -> Up {
        Up::Hello { protocol: PROTOCOL, app: "t".into(), name: name.into(), hostname: "h".into(), platform: "macos".into(), caps: caps.iter().map(|c| c.to_string()).collect(), brain_dormant_since: None }
    }

    /// Pairs a new Maya (key `me`) whose `hello` carries `caps`; returns its
    /// open channel, its entry id and the welcome's mirror offer.
    pub(crate) fn pair_as(handle: &ServerHandle, port: u16, me: &Keypair, name: &str, caps: &[&str]) -> (Client, String, Option<bool>) {
        let (code, _) = handle.open_pairing(crate::store::now_ms()).unwrap();
        let mut ws = connect(port);
        let (noise, main_key) = secure::pair(&mut ws, me, &code, soon(), &|| false).unwrap();
        assert_eq!(main_key, handle.public_key(), "the joiner learns the main's key");
        let mut ch = Channel::new(ws, noise);
        send(&mut ch, &hello_up(name, caps));
        let Down::Welcome { mirror, .. } = recv(&mut ch) else { panic!("expected welcome") };
        let key = crate::net::identity::encode_key(&me.public);
        let id = lock(&handle.shared).paired.iter().find(|p| p.key == key).map(|p| p.id.clone()).expect("an entry with this key");
        (ch, id, mirror)
    }

    /// Pairs a new Maya with a fresh key; returns its channel, id, key and offer.
    pub(crate) fn pair_client_caps(handle: &ServerHandle, port: u16, name: &str, caps: &[&str]) -> (Client, String, Keypair, Option<bool>) {
        let me = Keypair::generate();
        let (ch, id, offer) = pair_as(handle, port, &me, name, caps);
        (ch, id, me, offer)
    }

    pub(crate) fn pair_client(handle: &ServerHandle, port: u16, name: &str) -> (Client, String, Keypair) {
        let (ch, id, me, _) = pair_client_caps(handle, port, name, &[]);
        (ch, id, me)
    }

    /// Connects a paired Maya with its key and says `hello`; returns the
    /// channel before `welcome` is read.
    pub(crate) fn connect_as(handle: &ServerHandle, port: u16, me: &Keypair, name: &str, caps: &[&str]) -> Client {
        let mut ws = connect(port);
        let noise = secure::connect(&mut ws, me, &handle.public_key(), soon(), &|| false).unwrap();
        let mut ch = Channel::new(ws, noise);
        send(&mut ch, &hello_up(name, caps));
        ch
    }

    /// Pairs, then reconnects with caps; returns the welcomed channel, id and key.
    fn pair_with_caps(handle: &ServerHandle, port: u16, name: &str, caps: &[&str]) -> (Client, String, Keypair) {
        let (ch, id, me) = pair_client(handle, port, name);
        drop(ch);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ch = connect_as(handle, port, &me, name, caps);
        assert!(matches!(recv(&mut ch), Down::Welcome { .. }));
        (ch, id, me)
    }

    pub(crate) fn send(ch: &mut Client, up: &Up) {
        ch.send(&encode(up)).unwrap();
    }

    pub(crate) fn recv(ch: &mut Client) -> Down {
        let end = soon();
        loop {
            match ch.read().unwrap() {
                Got::Text(t) => return decode_down(&t).unwrap(),
                Got::Closed => panic!("closed"),
                _ if Instant::now() > end => panic!("nothing within 5 s"),
                _ => {}
            }
        }
    }

    #[test]
    fn a_maya_that_asks_to_mirror_is_allowed_at_pairing_and_told_so_in_welcome() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (_phone, phone_id, _, offer) = pair_client_caps(&handle, port, "Pixel", &["follow", "lead", "mirror"]);
        assert_eq!(offer, Some(true));
        let (_laptop, laptop_id, _, offer) = pair_client_caps(&handle, port, "laptop", &["follow", "lead"]);
        assert_eq!(offer, None, "a Maya that did not ask is told nothing");
        wait_until(|| handle.status().assistants.iter().filter(|a| a.connected).count() == 2);
        let status = handle.status();
        let phone = status.assistants.iter().find(|a| a.id == phone_id).unwrap();
        assert!(phone.mirror && phone.wants_mirror);
        let laptop = status.assistants.iter().find(|a| a.id == laptop_id).unwrap();
        assert!(!laptop.mirror && !laptop.wants_mirror);
        assert!(counter.saved.lock().unwrap().iter().any(|a| a.id == phone_id && a.mirror), "the permission is saved");
        handle.stop();
    }

    #[test]
    fn an_assistant_named_like_the_main_carries_its_address_so_requests_reach_it() {
        let counter = Arc::new(Counter::default());
        let handle = start_with(counter.clone(), Arc::new(Mutex::new(NetworkConfig { name: "desk".into(), ..Default::default() })), 0, Arc::new(Keypair::generate())).unwrap();
        let _ = counter.server.set(handle.shared.clone());
        let port = handle.port();
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        let (_twin, twin_id, _) = pair_client(&handle, port, "desk");
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == twin_id && a.connected));
        assert_eq!(handle.status().assistants.iter().find(|a| a.id == twin_id).unwrap().name, "desk (127.0.0.1)");
        handle.mirror(vec![]);
        let Down::Mirror { machines, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        let names: Vec<&str> = machines.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["desk", "desk (127.0.0.1)"]);
        send(&mut phone, &Up::Request { id: 1, machine: Some("desk (127.0.0.1)".into()), request: Request::ListDirs });
        assert!(matches!(recv(&mut phone), Down::Response { id: 1, ok: true, .. }));
        assert_eq!(counter.requests.lock().unwrap()[0].0.as_deref(), Some("desk (127.0.0.1)"), "the twin is not taken for the main");
        handle.stop();
    }

    #[test]
    fn the_main_mirrors_its_board_with_its_own_cards_named_and_the_receiver_left_out() {
        let (handle, port) = test_server();
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        let (_laptop, _, _) = pair_client(&handle, port, "laptop");
        wait_until(|| handle.status().assistants.iter().filter(|a| a.connected).count() == 2);
        let mut remote = card("r1", "fix");
        remote.machine = Some("laptop".into());
        handle.mirror(vec![card("l1", "local"), remote]);
        let Down::Mirror { allowed, cards, machines, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        let main = main_name_of(&NetworkConfig::default());
        assert!(allowed);
        assert_eq!(cards[0].machine.as_deref(), Some(main.as_str()), "the main's own cards carry its name");
        assert_eq!(cards[1].machine.as_deref(), Some("laptop"));
        assert_eq!(machines[0].name, main);
        assert!(machines[0].connected);
        assert!(machines.iter().any(|m| m.name == "laptop"));
        assert!(!machines.iter().any(|m| m.name == "Pixel"), "the receiver is not a machine to pick");
        handle.stop();
    }

    #[test]
    fn chat_frames_reach_only_allowed_mirrors_that_asked_for_chat() {
        let (handle, port) = test_server();
        let (mut phone, phone_id, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror", "chat"]);
        let (mut old, _, _, _) = pair_client_caps(&handle, port, "Old", &["mirror"]);
        let (mut laptop, _, _, _) = pair_client_caps(&handle, port, "laptop", &["lead"]);
        wait_until(|| handle.status().assistants.iter().filter(|a| a.connected).count() == 3);
        handle.to_mirrors(Down::Chat { turn: serde_json::json!({"text": "a"}) });
        let turn = loop {
            match recv(&mut phone) {
                Down::Mirror { .. } => continue,
                other => break other,
            }
        };
        assert_eq!(turn, Down::Chat { turn: serde_json::json!({"text": "a"}) });
        for quiet in [&mut old, &mut laptop] {
            let end = Instant::now() + Duration::from_millis(300);
            while Instant::now() < end {
                match quiet.read() {
                    Ok(Got::Text(t)) => assert!(!matches!(decode_down(&t).unwrap(), Down::Chat { .. }), "no chat for this Maya"),
                    Ok(Got::Closed) => panic!("closed"),
                    _ => {}
                }
            }
        }
        handle.set_mirror(&phone_id, false).unwrap();
        handle.to_mirrors(Down::Chat { turn: serde_json::json!({"text": "b"}) });
        let end = Instant::now() + Duration::from_millis(300);
        let mut seen = vec![];
        while Instant::now() < end {
            if let Ok(Got::Text(t)) = phone.read() {
                seen.push(decode_down(&t).unwrap());
            }
        }
        assert!(seen.iter().any(|d| matches!(d, Down::Mirror { allowed: false, .. })), "{seen:?}");
        assert!(!seen.iter().any(|d| matches!(d, Down::Chat { .. })), "{seen:?}");
        handle.stop();
    }

    #[test]
    fn the_main_sends_its_wake_word_with_the_board_and_again_when_it_changes() {
        let (handle, port) = test_server();
        let (mut phone, id, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        handle.mirror_wake_word(Some("Jarvis".into()));
        handle.mirror(vec![]);
        let Down::Mirror { wake_word, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        assert_eq!(wake_word.as_deref(), Some("Jarvis"));
        handle.mirror_wake_word(Some("Jarvis".into()));
        handle.mirror_wake_word(Some("Friday".into()));
        let Down::Mirror { wake_word, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        assert_eq!(wake_word.as_deref(), Some("Friday"), "a new word goes out without a board change");
        handle.stop();
    }

    #[test]
    fn turning_the_switch_off_tells_the_maya_and_refuses_its_requests_and_on_mirrors_again() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut phone, id, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        handle.set_mirror(&id, false).unwrap();
        assert!(!counter.saved.lock().unwrap().iter().find(|a| a.id == id).unwrap().mirror, "the switch is saved");
        assert_eq!(recv(&mut phone), Down::Mirror { allowed: false, cards: vec![], machines: vec![], remembered: vec![], wake_word: None });
        send(&mut phone, &Up::Request { id: 1, machine: None, request: Request::Command(CommandKind::Compact { session: "s1".into() }) });
        let Down::Response { id: 1, ok: false, error: Some(e), .. } = recv(&mut phone) else { panic!("expected a refusal") };
        assert!(e.ends_with("has not allowed this Maya to control its sessions."), "{e}");
        assert!(counter.requests.lock().unwrap().is_empty());
        handle.mirror(vec![card("l1", "local")]);
        handle.set_mirror(&id, true).unwrap();
        assert!(matches!(recv(&mut phone), Down::Mirror { allowed: true, .. }));
        assert_eq!(handle.set_mirror("nobody", true).unwrap_err(), "That Maya is no longer paired.");
        handle.stop();
    }

    #[test]
    fn a_maya_that_reconnects_without_the_mirror_cap_gets_no_board_and_no_requests() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (ws, id, key, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        assert!(handle.status().assistants.iter().any(|a| a.id == id && a.mirror), "the permission is still saved");
        let mut ws = connect_as(&handle, port, &key, "Pixel", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        send(&mut ws, &Up::Request { id: 1, machine: None, request: Request::ListDirs });
        let Down::Response { id: 1, ok: false, error: Some(e), .. } = recv(&mut ws) else { panic!("expected a refusal") };
        assert!(e.ends_with("has not allowed this Maya to control its sessions."), "{e}");
        assert!(counter.requests.lock().unwrap().is_empty());
        handle.mirror(vec![card("l1", "local")]);
        send(&mut ws, &Up::Ping);
        assert_eq!(recv(&mut ws), Down::Pong, "no board came before the pong");
        handle.stop();
    }

    #[test]
    fn an_allowed_maya_s_requests_reach_the_app_off_the_connection_thread() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        let main = main_name_of(&NetworkConfig::default());
        send(&mut phone, &Up::Request { id: 1, machine: None, request: Request::Command(CommandKind::Compact { session: "slow".into() }) });
        send(&mut phone, &Up::Ping);
        assert_eq!(recv(&mut phone), Down::Pong, "a slow request does not hold up the connection");
        assert_eq!(recv(&mut phone), Down::Response { id: 1, ok: true, error: None, data: Some(serde_json::json!(format!("{:?}", Request::Command(CommandKind::Compact { session: "slow".into() })))) });
        send(&mut phone, &Up::Request { id: 2, machine: Some(main.clone()), request: Request::ListDirs });
        assert!(matches!(recv(&mut phone), Down::Response { id: 2, ok: true, .. }));
        send(&mut phone, &Up::Request { id: 3, machine: Some("laptop".into()), request: Request::ListAgents });
        assert!(matches!(recv(&mut phone), Down::Response { id: 3, ok: true, .. }));
        let seen: Vec<Option<String>> = counter.requests.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
        assert_eq!(seen, [None, None, Some("laptop".to_string())], "the main's own name means the main");
        send(&mut phone, &Up::Request { id: 4, machine: None, request: Request::Command(CommandKind::Promote { list: vec![], summary: None }) });
        assert_eq!(recv(&mut phone), Down::Response { id: 4, ok: false, error: Some("That cannot be requested.".into()), data: None });
        handle.stop();
    }

    #[test]
    fn promote_refuses_a_maya_that_mirrors() {
        let (handle, port) = test_server();
        let (_phone, id, _, _) = pair_client_caps(&handle, port, "Pixel", &["follow", "lead", "mirror"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        assert_eq!(handle.promote(&id).err().as_deref(), Some("A phone cannot be the main."));
        let a = handle.status().assistants.into_iter().find(|a| a.id == id).expect("still paired");
        assert!(a.connected, "nothing was handed over");
        handle.stop();
    }

    #[test]
    fn promote_asks_for_a_summary_with_the_assistant_s_dormant_time_and_sends_it() {
        let (handle, port) = test_server();
        let (ch, g_id, me) = pair_client(&handle, port, "laptop");
        drop(ch);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == g_id && a.connected));
        let mut ws = connect(port);
        let noise = secure::connect(&mut ws, &me, &handle.public_key(), soon(), &|| false).unwrap();
        let mut g = Channel::new(ws, noise);
        let Up::Hello { protocol, app, name, hostname, platform, caps, .. } = hello_up("laptop", &["follow", "lead"]) else { panic!() };
        send(&mut g, &Up::Hello { protocol, app, name, hostname, platform, caps, brain_dormant_since: Some(77) });
        assert!(matches!(recv(&mut g), Down::Welcome { .. }));
        let h = handle.clone();
        let asked = Arc::new(Mutex::new(None));
        let seen = asked.clone();
        let t = std::thread::spawn(move || {
            h.promote_with_summary(&g_id, |to| {
                *seen.lock().unwrap() = Some(to.clone());
                Some("User asked for #52.".into())
            })
        });
        let Down::Command { id: cmd, kind: CommandKind::Promote { summary, .. } } = recv(&mut g) else { panic!("expected promote") };
        assert_eq!(summary.as_deref(), Some("User asked for #52."));
        assert_eq!(*asked.lock().unwrap(), Some(HandoverTo { label: "laptop".into(), brain_dormant_since: Some(77) }));
        send(&mut g, &Up::Result { id: cmd, ok: true, error: None, data: Some(serde_json::json!({ "port": 5001 })) });
        assert!(t.join().unwrap().is_ok());
        handle.stop();
    }

    #[test]
    fn caps_show_as_can_lead_and_can_follow_while_connected() {
        let (handle, port) = test_server();
        let (ws, id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let a = handle.status().assistants.into_iter().find(|a| a.id == id).unwrap();
        assert!(a.can_lead && a.can_follow);
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!((json["canLead"].as_bool(), json["canFollow"].as_bool()), (Some(true), Some(true)));
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let a = &handle.status().assistants[0];
        assert!(!a.can_lead && !a.can_follow, "a disconnected Maya can do nothing");
        handle.stop();
    }

    #[test]
    fn promote_hands_the_keys_over_moves_followers_with_the_new_key_and_names_the_rest() {
        let (handle, port) = test_server();
        let (mut g, g_id, g_key) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let (mut b, b_id, b_key) = pair_with_caps(&handle, port, "mini", &["follow"]);
        let (_old, old_id, _) = pair_with_caps(&handle, port, "old", &[]);
        handle.open_pairing(crate::store::now_ms()).unwrap();
        let h = handle.clone();
        let t = std::thread::spawn(move || h.promote(&g_id));
        let Down::Command { id: cmd, kind: CommandKind::Promote { list, .. } } = recv(&mut g) else { panic!("expected promote") };
        assert!(handle.status().code.is_none(), "pairing closes for the handover");
        let key_of = |k: &Keypair| crate::net::identity::encode_key(&k.public);
        assert!(list.iter().all(|p| !p.key.is_empty()), "every entry carries its key");
        assert!(!encode(&list).contains("token"), "nothing secret is handed over");
        assert!(list.iter().any(|p| p.id == b_id && p.key == key_of(&b_key)), "B keeps its key");
        assert!(list.iter().any(|p| p.id == old_id));
        assert!(!list.iter().any(|p| p.key == key_of(&g_key)), "the new main is not in its own list");
        assert_eq!(list.last().unwrap().key, crate::net::identity::encode_key(&handle.public_key()), "the old main connects with its own key");
        send(&mut g, &Up::Result { id: cmd, ok: true, error: None, data: Some(serde_json::json!({ "port": 5001 })) });
        assert_eq!(recv(&mut b), Down::Move { host: "127.0.0.1".into(), port: 5001, key: key_of(&g_key) });
        let out = t.join().unwrap().unwrap();
        assert_eq!((out.host.as_str(), out.port), ("127.0.0.1", 5001));
        assert_eq!(out.main_key, key_of(&g_key), "the old main pins the new main's key");
        assert_eq!(out.moved, ["mini"]);
        assert_eq!(out.left_behind, ["old"], "a Maya without follow is not moved");
        assert!(!handle.status().assistants.iter().any(|a| a.id == b_id && a.connected), "a moved connection is closed");
        handle.stop();
    }

    #[test]
    fn promote_refuses_an_entry_without_a_key() {
        let (handle, port) = test_server();
        let (_g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        handle.shared.lock().unwrap().paired.iter_mut().for_each(|p| p.key.clear());
        assert_eq!(handle.promote(&g_id).unwrap_err(), "laptop has no key; pair it again.");
        assert!(!lock(&handle.shared).handover);
        handle.stop();
    }

    #[test]
    fn promote_refuses_a_second_handover_and_one_that_cannot_lead() {
        let (handle, port) = test_server();
        let (mut g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let (_c, c_id, _) = pair_with_caps(&handle, port, "cli", &["follow"]);
        assert_eq!(handle.promote(&c_id).unwrap_err(), "cli cannot become a main.");
        let h = handle.clone();
        let first = std::thread::spawn(move || h.promote(&g_id));
        let Down::Command { id: cmd, .. } = recv(&mut g) else { panic!("expected promote") };
        assert_eq!(handle.promote(&c_id).unwrap_err(), "A handover is already running.");
        send(&mut g, &Up::Result { id: cmd, ok: false, error: Some("Could not listen on port 4127: in use".into()), data: None });
        assert_eq!(first.join().unwrap().unwrap_err(), "laptop could not become the main: Could not listen on port 4127: in use.");
        assert!(handle.status().assistants.iter().all(|a| a.connected), "a failed handover moves nobody");
        handle.stop();
    }

    #[test]
    fn a_promote_nobody_answers_says_so_and_moves_nobody() {
        let (handle, port) = test_server();
        let (mut g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let (_b, _, _) = pair_with_caps(&handle, port, "mini", &["follow"]);
        let err = handle.promote_with_timeout(&g_id, Duration::from_millis(300)).unwrap_err();
        assert_eq!(err, "laptop did not answer; it may have become a main on its own. Check it before trying again.");
        assert!(matches!(recv(&mut g), Down::Command { kind: CommandKind::Promote { .. }, .. }));
        assert!(handle.status().assistants.iter().all(|a| a.connected));
        handle.stop();
    }

    #[test]
    fn a_promote_whose_assistant_disconnects_reads_like_one_nobody_answers() {
        let (handle, port) = test_server();
        let (mut g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let h = handle.clone();
        let t = std::thread::spawn(move || h.promote_with_timeout(&g_id, Duration::from_secs(5)));
        assert!(matches!(recv(&mut g), Down::Command { kind: CommandKind::Promote { .. }, .. }));
        drop(g);
        assert_eq!(t.join().unwrap().unwrap_err(), "laptop did not answer; it may have become a main on its own. Check it before trying again.");
        handle.stop();
    }

    #[test]
    fn an_assistant_error_reaches_the_caller() {
        // What an older Maya sends back for a command it cannot decode.
        let (handle, port) = test_server();
        let (mut g, _, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "laptop", CommandKind::Stop { session: "r1".into() }, Duration::from_secs(5)));
        let Down::Command { id: cmd, kind: CommandKind::Stop { .. } } = recv(&mut g) else { panic!("expected stop") };
        send(&mut g, &Up::Result { id: cmd, ok: false, error: Some("unknown variant `stop`".into()), data: None });
        assert_eq!(t.join().unwrap(), Err("unknown variant `stop`".to_string()));
        handle.stop();
    }

    #[test]
    fn pairing_stays_closed_during_a_handover() {
        let (handle, port) = test_server();
        let (mut g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        let h = handle.clone();
        let t = std::thread::spawn(move || h.promote(&g_id));
        let Down::Command { id: cmd, .. } = recv(&mut g) else { panic!("expected promote") };
        assert_eq!(handle.open_pairing(crate::store::now_ms()).unwrap_err(), "A handover is already running.");
        assert!(handle.status().code.is_none());
        send(&mut g, &Up::Result { id: cmd, ok: false, error: Some("no".into()), data: None });
        t.join().unwrap().unwrap_err();
        assert!(handle.open_pairing(crate::store::now_ms()).is_ok(), "pairing opens again once the handover ended");
        handle.stop();
    }

    #[test]
    fn a_handover_waits_for_a_pairing_that_is_still_in_flight() {
        let (handle, port) = test_server();
        let (_g, g_id, _) = pair_with_caps(&handle, port, "laptop", &["follow", "lead"]);
        assert!(lock(&handle.shared).pairings_in_flight.is_empty(), "a finished pairing is no longer in flight");
        // A machine past `record_pairing` but not yet registered: it would be
        // handed over without being moved.
        let (entry, previous) = lock(&handle.shared).record_pairing("a2V5", "10.0.0.9", "mini", "h", "macos", false);
        assert_eq!(handle.promote(&g_id).unwrap_err(), "A Maya is pairing right now; try again in a moment.");
        assert!(!lock(&handle.shared).handover, "a refused handover leaves no turn behind");
        lock(&handle.shared).undo_pairing(&entry.id, previous);
        assert!(lock(&handle.shared).pairings_in_flight.is_empty());
        handle.stop();
    }

    pub(crate) fn wait_until(mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "condition not met within 5 s");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub(crate) fn card(id: &str, name: &str) -> Card {
        Card { session_id: id.into(), pid: 1, name: name.into(), cwd: "/x/p".into(), state: State::Idle, state_since: 0, snippet: "".into(), awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None, machine_address: None, machine_platform: None, terminal: None, stale: false, model: None, branch: None, brain: false, shell: false }
    }

    #[test]
    fn pairs_snapshots_and_round_trips_a_command() {
        let (handle, port) = test_server();
        let (mut ws, id, key) = pair_client(&handle, port, "laptop");
        send(&mut ws, &Up::Board { cards: vec![card("r1", "remote")], dirs: vec!["proj".into()], agents: Some(vec![crate::agents::claude()]), remembered: None });
        wait_until(|| handle.shared.lock().unwrap().boards.iter().any(|b| b.machine == "laptop" && b.cards.len() == 1));
        assert_eq!(handle.boards().iter().find(|b| b.machine == "laptop").unwrap().agents.as_ref().unwrap().len(), 1);
        // A command from the main reaches the client and its result comes back.
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "laptop", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)));
        let Down::Command { id: cmd, kind } = recv(&mut ws) else { panic!("expected command") };
        assert_eq!(kind, CommandKind::Compact { session: "r1".into() });
        send(&mut ws, &Up::Result { id: cmd, ok: true, error: None, data: None });
        assert!(t.join().unwrap().is_ok());
        // Reconnect with the key: IK, no code, the same entry.
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let mut ws = connect_as(&handle, port, &key, "laptop", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        assert_eq!(handle.status().assistants.len(), 1);
        handle.stop();
    }

    #[test]
    fn a_board_says_whether_its_maya_takes_worktrees() {
        let (handle, port) = test_server();
        let (mut new, _, _, _) = pair_client_caps(&handle, port, "new", &[CAP_WORKTREE]);
        let (mut old, _, _, _) = pair_client_caps(&handle, port, "old", &[]);
        send(&mut new, &Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: None });
        send(&mut old, &Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: None });
        wait_until(|| handle.boards().len() == 2);
        let boards = handle.boards();
        assert!(boards.iter().find(|b| b.machine == "new").unwrap().worktrees);
        assert!(!boards.iter().find(|b| b.machine == "old").unwrap().worktrees);
        handle.stop();
    }

    #[test]
    fn an_unknown_key_is_told_it_was_removed_and_a_wrong_code_is_refused() {
        let (handle, port) = test_server();
        let mut ws = connect_as(&handle, port, &Keypair::generate(), "x", &[]);
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == "removed"), "unknown key");
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let mut raw = connect(port);
        assert_eq!(secure::pair(&mut raw, &Keypair::generate(), wrong, soon(), &|| false).unwrap_err(), secure::Fail::Refused(secure::WRONG_CODE_REASON.into()));
        let err = send_command_with(&handle.shared, "ghost", CommandKind::Compact { session: "r".into() }, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("not connected"), "{err}");
        handle.stop();
    }

    /// One pairing attempt with `code`; true when it was welcomed.
    fn try_code(port: u16, code: &str) -> bool {
        let mut raw = connect(port);
        let Ok((noise, _)) = secure::pair(&mut raw, &Keypair::generate(), code, soon(), &|| false) else { return false };
        let mut ch = Channel::new(raw, noise);
        send(&mut ch, &hello_up("guesser", &[]));
        let end = soon();
        loop {
            match ch.read() {
                Ok(Got::Text(t)) => return matches!(decode_down(&t), Ok(Down::Welcome { .. })),
                Ok(Got::Idle) if Instant::now() < end => {}
                _ => return false,
            }
        }
    }

    #[test]
    fn a_code_is_used_up_after_three_wrong_tries_from_any_address() {
        let (handle, port) = test_server();
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..3 {
            assert!(!try_code(port, wrong));
        }
        wait_until(|| handle.status().code_used_up);
        assert!(handle.status().code.is_none());
        // The address's lockout goes, so the refusal is the used-up code's.
        lock(&handle.shared).attempts = Attempts::default();
        assert_eq!(secure::pair(&mut connect(port), &Keypair::generate(), &code, soon(), &|| false).unwrap_err(), secure::Fail::Refused(secure::WRONG_CODE_REASON.into()), "the right code no longer works");
        handle.open_pairing(now_ms()).unwrap();
        assert!(!handle.status().code_used_up, "a new code clears it");
        handle.stop();
    }

    #[test]
    fn an_abandoned_pairing_uses_a_try() {
        let (handle, port) = test_server();
        handle.open_pairing(now_ms()).unwrap();
        for _ in 0..3 {
            let mut raw = connect(port);
            // SPAKE2's first message, then gone.
            let (_, ours) = spake2::Spake2::<spake2::Ed25519Group>::start_a(&spake2::Password::new(b"123456"), &spake2::Identity::new(b"maya-join"), &spake2::Identity::new(b"maya-main"));
            let mut first = vec![secure::MODE_PAIR];
            first.extend(ours);
            raw.send(Message::binary(first)).unwrap();
            let _ = raw.read();
            drop(raw);
        }
        wait_until(|| handle.status().code_used_up);
        handle.stop();
    }

    #[test]
    fn two_joiners_with_the_right_code_pair_one_machine() {
        let (handle, port) = test_server();
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let mut a = connect(port);
        let mut b = connect(port);
        let (na, _) = secure::pair(&mut a, &Keypair::generate(), &code, soon(), &|| false).unwrap();
        let (nb, _) = secure::pair(&mut b, &Keypair::generate(), &code, soon(), &|| false).unwrap();
        let (mut a, mut b) = (Channel::new(a, na), Channel::new(b, nb));
        send(&mut a, &hello_up("first", &[]));
        assert!(matches!(recv(&mut a), Down::Welcome { .. }));
        send(&mut b, &hello_up("second", &[]));
        assert!(matches!(recv(&mut b), Down::Bye { reason } if reason == secure::WRONG_CODE_REASON));
        assert_eq!(handle.status().assistants.len(), 1);
        handle.stop();
    }

    #[test]
    fn a_protocol_1_maya_is_answered_in_plain_text_that_it_can_show() {
        let (handle, port) = test_server();
        let mut raw = connect(port);
        raw.send(Message::text(r#"{"type":"hello","protocol":1,"app":"0.21.0","name":"old","hostname":"h","platform":"macos"}"#)).unwrap();
        let reason = loop {
            match raw.read().unwrap() {
                Message::Text(t) => break decode_down(&t).unwrap(),
                _ => continue,
            }
        };
        assert_eq!(reason, Down::Bye { reason: "protocol".into() });
        handle.stop();
    }

    #[test]
    fn garbage_disconnects_lockouts_removal_and_stop() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut ws, id, key) = pair_client(&handle, port, "desk");
        // Saved once `welcome` is out, which the client may read first.
        wait_until(|| counter.paired.load(Ordering::SeqCst) == 1);
        assert!(handle.status().code.is_none(), "a code pairs one assistant");
        // Sealed garbage is skipped; the next good message still lands.
        ws.send("not json").unwrap();
        ws.send(r#"{"type":"dance"}"#).unwrap();
        send(&mut ws, &Up::Ping);
        assert_eq!(recv(&mut ws), Down::Pong);
        send(&mut ws, &Up::Board { cards: vec![card("r1", "x")], dirs: vec![], agents: None, remembered: None });
        wait_until(|| handle.boards().iter().any(|b| b.machine == "desk" && b.connected));
        assert!(counter.boards.load(Ordering::SeqCst) >= 1);
        // A command in flight fails when its assistant goes away, and the
        // board greys: a frame that is not sealed ends the connection.
        let shared = handle.shared.clone();
        let t = std::thread::spawn(move || send_command_with(&shared, "desk", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5)));
        assert!(matches!(recv(&mut ws), Down::Command { .. }));
        ws.ws.send(Message::binary(vec![1, 2, 3])).unwrap();
        let err = t.join().unwrap().unwrap_err();
        assert_eq!(err, "desk disconnected before answering.");
        wait_until(|| handle.boards().iter().any(|b| b.machine == "desk" && !b.connected));
        assert!(!handle.status().assistants[0].connected);
        drop(ws);
        // Three wrong tries lock the address out, even with a new code after.
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..3 {
            assert!(!try_code(port, wrong));
        }
        wait_until(|| handle.status().code_used_up);
        let (new_code, _) = handle.open_pairing(now_ms()).unwrap();
        assert_eq!(secure::pair(&mut connect(port), &Keypair::generate(), &new_code, soon(), &|| false).unwrap_err(), secure::Fail::Refused("too many attempts".into()));
        // Removal says bye, drops the cards, and the key no longer opens a connection.
        let mut ws = connect_as(&handle, port, &key, "desk", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        handle.remove_assistant(&id).unwrap();
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == "removed"));
        assert!(handle.boards().is_empty());
        assert!(handle.status().assistants.is_empty());
        assert!(counter.saved.lock().unwrap().is_empty(), "the removal is saved");
        let mut again = connect_as(&handle, port, &key, "desk", &[]);
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
                Ok(Got::Closed) | Err(_) => break true,
                Ok(Got::Idle) => break false,
                Ok(_) => continue,
            }
        };
        assert!(closed, "the connection closes when the server stops");
        handle.stop();
        wait_until(|| TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn a_maya_pairing_again_with_its_key_keeps_its_entry() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let me = Keypair::generate();
        let (ws, first, _) = pair_as(&handle, port, &me, "desk", &[]);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
        let (_ws, again, _) = pair_as(&handle, port, &me, "desk2", &[]);
        assert_eq!(again, first, "the same key: the same entry");
        assert_eq!(handle.status().assistants.len(), 1);
        assert_eq!(handle.status().assistants[0].name, "desk2", "names follow the machine");
        handle.stop();
    }

    #[test]
    fn two_mayas_at_one_address_are_two_entries_and_equal_names_carry_it() {
        let (handle, port) = test_server();
        let (_a, a, _) = pair_client(&handle, port, "laptop");
        let (_b, b, _) = pair_client(&handle, port, "laptop");
        assert_ne!(a, b);
        let names: Vec<String> = handle.status().assistants.iter().map(|s| s.name.clone()).collect();
        assert!(names.iter().all(|n| n.contains("127.0.0.1")), "{names:?}");
        handle.stop();
    }

    #[test]
    fn a_pairing_finds_the_machine_by_its_key_and_an_undo_gives_the_entry_back() {
        let mut s = Server::new(vec![], "desk".into());
        let (first, previous) = s.record_pairing("a2V5", "192.168.55.70", "Gnowee", "TKC-0176", "macos", false);
        assert!(previous.is_none());
        s.by_id.insert(first.id.clone(), RemoteBoard { machine: String::new(), hostname: String::new(), platform: String::new(), address: String::new(), cards: vec![card("r1", "x")], dirs: vec![], agents: None, remembered: None, worktrees: false, received_at: 0, connected: false });
        // The same key from another address: the same machine, at its new address.
        let (second, previous) = s.record_pairing("a2V5", "192.168.55.80", "Gnowee", "TKC-0176", "macos", false);
        assert_eq!(previous.as_ref(), Some(&first));
        assert_eq!(second.id, first.id);
        assert_eq!(second.address, "192.168.55.80");
        assert_eq!(s.paired.len(), 1);
        assert!(s.by_id.is_empty(), "its old board is dropped");
        // Another key from the same address is another machine.
        let (other, previous) = s.record_pairing("b3RoZXI=", "192.168.55.80", "Gnowee", "TKC-0199", "macos", false);
        assert!(previous.is_none());
        assert_ne!(other.id, first.id);
        let names: Vec<String> = s.status(0).assistants.into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["Gnowee (192.168.55.80)", "Gnowee (192.168.55.80)"]);
        // A connection brings the entry up to date: a new address, a new name.
        s.refresh_entry(&first.id, "192.168.55.81", "Gnowee", "TKC-0176", "macos", 7);
        assert_eq!((s.paired[0].address.as_str(), s.paired[0].last_seen), ("192.168.55.81", Some(7)));
        // A pairing the Maya never completed is taken back.
        s.undo_pairing(&other.id, None);
        assert_eq!(s.paired.len(), 1);
        let (again, previous) = s.record_pairing("a2V5", "192.168.55.90", "Renamed", "TKC-0176", "macos", false);
        s.undo_pairing(&again.id, previous);
        assert_eq!((s.paired[0].name.as_str(), s.paired[0].address.as_str()), ("Gnowee", "192.168.55.81"), "an entry paired again gets its old self back");
    }

    #[test]
    fn a_first_board_is_quiet_but_a_reconnect_announces_what_began_while_away() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let state = |id: &str, state: State, since: u64| Card { state, state_since: since, ..card(id, "x") };
        let (mut ws, _, key) = pair_client(&handle, port, "desk");
        // Open when it paired: an ask and a finished turn. Nothing is announced.
        send(&mut ws, &Up::Board { cards: vec![state("r1", State::Awaiting, 100), state("r2", State::Completed, 100)], dirs: vec![], agents: None, remembered: None });
        wait_until(|| handle.boards().iter().any(|b| b.cards.len() == 2));
        // A new ask after that is announced, and only it.
        send(&mut ws, &Up::Board { cards: vec![state("r1", State::Awaiting, 100), state("r2", State::Completed, 100), state("r3", State::Awaiting, 200)], dirs: vec![], agents: None, remembered: None });
        wait_until(|| !counter.announced.lock().unwrap().is_empty());
        assert_eq!(*counter.announced.lock().unwrap(), ["r3"]);
        // It drops; while it is away r1 asks something new and r2 finishes again.
        drop(ws);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        let mut ws = connect_as(&handle, port, &key, "desk", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        // The main still holds its board, so the reconnect's first board is news.
        send(&mut ws, &Up::Board { cards: vec![state("r1", State::Awaiting, 300), state("r2", State::Completed, 300), state("r3", State::Awaiting, 200)], dirs: vec![], agents: None, remembered: None });
        wait_until(|| counter.announced.lock().unwrap().len() == 3);
        assert_eq!(*counter.announced.lock().unwrap(), ["r3", "r1", "r2"], "what began while away is announced; r3 is not repeated");
        handle.stop();
    }

    #[test]
    fn a_main_started_afresh_is_quiet_about_a_known_assistants_first_board() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (ws, _, key) = pair_client(&handle, port, "desk");
        drop(ws);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        handle.stop();
        // A new run of the main, with its key: the assistant is paired but no board is held yet.
        let saved = handle.shared.lock().unwrap().paired.clone();
        let counter = Arc::new(Counter::default());
        let restarted = start_with(counter.clone(), Arc::new(Mutex::new(NetworkConfig { assistants: saved, ..Default::default() })), 0, handle.ctx.identity.clone()).unwrap();
        let _ = counter.server.set(restarted.shared.clone());
        let mut ws = connect_as(&restarted, restarted.port(), &key, "desk", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
        send(&mut ws, &Up::Board { cards: vec![Card { state: State::Awaiting, state_since: 100, ..card("r1", "x") }], dirs: vec![], agents: None, remembered: None });
        wait_until(|| restarted.boards().iter().any(|b| b.cards.len() == 1));
        send(&mut ws, &Up::Board { cards: vec![Card { state: State::Awaiting, state_since: 100, ..card("r1", "x") }, Card { state: State::Awaiting, state_since: 200, ..card("r2", "x") }], dirs: vec![], agents: None, remembered: None });
        wait_until(|| !counter.announced.lock().unwrap().is_empty());
        assert_eq!(*counter.announced.lock().unwrap(), ["r2"]);
        restarted.stop();
    }

    /// Closes `ws` with a reset rather than a FIN, so the main's next write fails.
    #[cfg(windows)]
    fn reset(ws: Client) {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{setsockopt, LINGER, SOL_SOCKET, SO_LINGER};
        let MaybeTlsStream::Plain(s) = ws.ws.get_ref() else { panic!("plain socket") };
        let linger = LINGER { l_onoff: 1, l_linger: 0 };
        // SAFETY: a valid socket, and a linger struct of the size given.
        let rc = unsafe { setsockopt(s.as_raw_socket() as usize, SOL_SOCKET, SO_LINGER, &linger as *const _ as *const u8, std::mem::size_of::<LINGER>() as i32) };
        assert_eq!(rc, 0);
        drop(ws);
    }

    /// Closes `ws` with a reset rather than a FIN, so the main's next write fails.
    #[cfg(unix)]
    fn reset(ws: Client) {
        use std::os::fd::AsRawFd;
        let MaybeTlsStream::Plain(s) = ws.ws.get_ref() else { panic!("plain socket") };
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: a valid socket, and a linger struct of the size given.
        let rc = unsafe { libc::setsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_LINGER, &linger as *const _ as *const libc::c_void, std::mem::size_of::<libc::linger>() as libc::socklen_t) };
        assert_eq!(rc, 0);
        drop(ws);
    }

    #[test]
    fn an_assistant_gone_right_after_its_hello_leaves_no_entry() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let mut raw = connect(port);
        let (noise, _) = secure::pair(&mut raw, &Keypair::generate(), &code, soon(), &|| false).unwrap();
        let mut ch = Channel::new(raw, noise);
        // `hello`, then the socket goes.
        ch.write(&encode(&hello_up("ghost", &[]))).unwrap();
        ch.flush().unwrap();
        reset(ch);
        wait_until(|| handle.ctx.live.load(Ordering::SeqCst) == 0);
        let status = handle.status();
        assert!(status.assistants.iter().all(|a| !a.connected), "a Maya that is gone is never left connected");
        // Only macOS reliably fails the main's one `welcome` write after the
        // reset. Linux may take that write into the socket buffer (the error
        // shows on a later write) and keep a disconnected entry, which the
        // same key pairing again reuses; Windows may discard the `hello`
        // unread, leaving the code open. Both are as safe.
        if cfg!(target_os = "macos") {
            assert!(status.assistants.is_empty(), "no paired entry is left behind");
            // Saved just before `welcome`, then taken back when `welcome` could not be sent.
            assert!(counter.saved.lock().unwrap().is_empty(), "the saved list does not keep it");
            assert!(status.code.is_none(), "the main read the hello and paired, then took the pairing back");
        }
        handle.stop();
    }

    #[test]
    fn the_paired_list_is_saved_under_the_server_lock_so_no_older_snapshot_wins() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let reconnect = |key: &Keypair| {
            let mut ws = connect_as(&handle, port, key, "desk", &[]);
            assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
            ws
        };
        // Every kind of save: a pairing, a disconnect, a connect, a removal.
        let (desk, id, key) = pair_client(&handle, port, "desk");
        let (_twin, twin, _) = pair_client(&handle, port, "twin");
        wait_until(|| counter.paired.load(Ordering::SeqCst) == 2);
        drop(desk);
        wait_until(|| counter.saved.lock().unwrap().iter().any(|a| a.id == id && a.last_seen.is_some()) && !handle.status().assistants[0].connected);
        let desk = reconnect(&key);
        handle.remove_assistant(&twin).unwrap();
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
        let restarted = start_with(Arc::new(Counter::default()), Arc::new(Mutex::new(NetworkConfig { assistants: saved.clone(), ..Default::default() })), 0, Arc::new(Keypair::generate())).unwrap();
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
        send(&mut ws, &Up::Board { cards: vec![card("r1", "x")], dirs: vec![], agents: None, remembered: None });
        wait_until(|| handle.boards().iter().any(|b| b.cards.len() == 1));
        // A newer Maya's board: a card state this version does not know. (An
        // unknown harness is readable since 0.11.0: it decodes to Other.)
        let mut future = serde_json::to_value(Up::Board { cards: vec![card("r2", "y")], dirs: vec![], agents: None, remembered: None }).unwrap();
        future["cards"][0]["state"] = "future-state".into();
        ws.send(&future.to_string()).unwrap();
        wait_until(|| note(&handle).is_some_and(|n| n.ends_with(UNREADABLE_BOARD)));
        assert_eq!(note(&handle).unwrap(), format!("runs Maya t; this Mac runs {ours}; {UNREADABLE_BOARD}"));
        assert_eq!(handle.boards()[0].cards[0].session_id, "r1", "the last good board stays");
        // It stays fresh too: the assistant is alive, only its board is unreadable.
        {
            let mut s = handle.shared.lock().unwrap();
            s.by_id.get_mut(&id).unwrap().received_at = 1;
            s.sync_boards();
        }
        ws.send(&future.to_string()).unwrap();
        wait_until(|| handle.boards()[0].received_at > 1);
        let cards = super::super::merge::merged(vec![], &handle.boards(), now_ms());
        assert_eq!(cards.len(), 1);
        assert!(!cards[0].stale, "the kept cards stay un-stale while the note explains why they do not change");
        assert!(note(&handle).unwrap().ends_with(UNREADABLE_BOARD));
        // A readable board clears the note.
        send(&mut ws, &Up::Board { cards: vec![card("r3", "z")], dirs: vec![], agents: None, remembered: None });
        wait_until(|| note(&handle).is_some_and(|n| !n.contains(UNREADABLE_BOARD)));
        // The same version says nothing.
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let twin_key = Keypair::generate();
        let mut raw = connect(port);
        let (noise, _) = secure::pair(&mut raw, &twin_key, &code, soon(), &|| false).unwrap();
        let mut same = Channel::new(raw, noise);
        send(&mut same, &Up::Hello { protocol: PROTOCOL, app: ours.into(), name: "twin".into(), hostname: "h2".into(), platform: "macos".into(), caps: vec![], brain_dormant_since: None });
        assert!(matches!(recv(&mut same), Down::Welcome { .. }));
        let twin = lock(&handle.shared).paired.iter().find(|p| p.key == crate::net::identity::encode_key(&twin_key.public)).unwrap().id.clone();
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
        let (desk, _, key) = pair_client(&handle, port, "desk");
        let in_handshake = |handle: &ServerHandle| handle.ctx.pre_auth.lock().unwrap().total;
        wait_until(|| in_handshake(&handle) == 0);
        // Two sockets from this address that never even start the WebSocket handshake.
        let strangers: Vec<TcpStream> = (0..MAX_PRE_AUTH_PER_ADDRESS).map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
        wait_until(|| in_handshake(&handle) == MAX_PRE_AUTH_PER_ADDRESS);
        assert!(tungstenite::connect(format!("ws://127.0.0.1:{port}/")).is_err(), "a third from the same address is closed at once");
        assert!(handle.status().assistants[0].connected, "an assistant through welcome holds no slot");
        drop(strangers);
        wait_until(|| in_handshake(&handle) == 0);
        // The assistant reconnects with its key, and is through welcome.
        drop(desk);
        wait_until(|| !handle.status().assistants[0].connected);
        let mut ws = connect_as(&handle, port, &key, "desk", &[]);
        assert!(matches!(recv(&mut ws), Down::Welcome { .. }));
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
        let Err(err) = start_with(Arc::new(Counter::default()), Arc::new(Mutex::new(NetworkConfig::default())), port, Arc::new(Keypair::generate())) else { panic!("the port is taken") };
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
        send(&mut ws, &Up::Board { cards: vec![big], dirs: vec![], agents: None, remembered: None });
        wait_until(|| handle.boards().iter().any(|b| b.cards.len() == 1));
        handle.stop();
    }

    /// The client's executor: records each command and serves one card.
    #[derive(Default)]
    pub(crate) struct FakeExec {
        pub(crate) kinds: Mutex<Vec<CommandKind>>,
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
    pub(crate) struct FakeClientNotify {
        main: Mutex<Option<String>>,
        /// The main's key, as pairing learned it.
        key: Mutex<Option<String>>,
        errors: Mutex<Vec<String>>,
        removed: AtomicUsize,
    }

    impl client::ClientNotify for FakeClientNotify {
        fn paired(&self, main_key: &str) {
            *self.key.lock().unwrap() = Some(main_key.into());
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

    /// A client-side notifier that records `moved` and answers `promoted`.
    #[derive(Default)]
    struct Leader {
        caps: Vec<String>,
        moved: Mutex<Option<(String, u16, String)>>,
        promoted: Mutex<Option<Vec<crate::config::PairedAssistant>>>,
        promote_result: Mutex<Option<Result<u16, String>>>,
        disconnects: Mutex<Vec<String>>,
    }

    impl client::ClientNotify for Leader {
        fn paired(&self, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, e: &str) {
            self.disconnects.lock().unwrap().push(e.into());
        }
        fn removed(&self) {}
        fn caps(&self) -> Vec<String> {
            self.caps.clone()
        }
        fn promoted(&self, list: Vec<crate::config::PairedAssistant>, _summary: Option<String>) -> Result<u16, String> {
            *self.promoted.lock().unwrap() = Some(list);
            self.promote_result.lock().unwrap().clone().unwrap_or(Ok(6000))
        }
        fn moved(&self, host: &str, port: u16, key: &str) -> Result<(), String> {
            *self.moved.lock().unwrap() = Some((host.into(), port, key.into()));
            Ok(())
        }
    }

    struct NoSessions;
    impl client::Executor for NoSessions {
        fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
            Err("no".into())
        }
        fn board(&self) -> (Vec<Card>, Vec<String>) {
            (vec![], vec![])
        }
    }

    pub(crate) fn assistant_config(port: u16, main: &ServerHandle) -> NetworkConfig {
        NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, main_key: crate::net::identity::encode_key(&main.public_key()), ..Default::default() }
    }

    #[test]
    fn a_client_says_its_caps_opens_promote_answers_with_its_port_and_ends() {
        let (handle, port) = test_server();
        let (ws, id, key) = pair_client(&handle, port, "laptop");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let notify = Arc::new(Leader { caps: vec!["follow".into(), "lead".into()], ..Default::default() });
        let (config, n, stop, k) = (assistant_config(port, &handle), notify.clone(), Arc::new(AtomicBool::new(false)), key.clone());
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, &k, Arc::new(NoSessions), n, &s, None));
        wait_until(|| handle.status().assistants[0].can_lead);
        let out = handle.promote(&id).unwrap();
        assert_eq!(out.port, 6000, "the port the new main gave");
        assert_eq!(out.main_key, crate::net::identity::encode_key(&key.public), "the promoted Maya's own key");
        assert_eq!(notify.promoted.lock().unwrap().as_ref().unwrap().last().unwrap().key, crate::net::identity::encode_key(&handle.public_key()));
        assert_eq!(run.join().unwrap(), Ok(()), "a promoted client ends without an error");
        assert!(notify.disconnects.lock().unwrap().is_empty());
        handle.stop();
    }

    #[test]
    fn a_client_that_cannot_become_the_main_says_why_and_stays() {
        let (handle, port) = test_server();
        let (ws, id, key) = pair_client(&handle, port, "laptop");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let notify = Arc::new(Leader { caps: vec!["follow".into(), "lead".into()], promote_result: Mutex::new(Some(Err("Could not listen on port 6000: in use".into()))), ..Default::default() });
        let (config, n, stop) = (NetworkConfig { name: "laptop".into(), ..assistant_config(port, &handle) }, notify.clone(), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, &key, Arc::new(NoSessions), n, &s, None));
        wait_until(|| handle.status().assistants[0].can_lead);
        assert_eq!(handle.promote(&id).unwrap_err(), "laptop could not become the main: Could not listen on port 6000: in use.");
        assert!(handle.status().assistants[0].connected, "still an assistant of this main");
        stop.store(true, Ordering::SeqCst);
        let _ = run.join();
        handle.stop();
    }

    #[test]
    fn a_client_told_to_move_saves_the_new_host_and_reconnects_at_once() {
        let (handle, port) = test_server();
        let (ws, id, key) = pair_client(&handle, port, "mini");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let notify = Arc::new(Leader { caps: vec!["follow".into()], ..Default::default() });
        let (config, n, stop) = (assistant_config(port, &handle), notify.clone(), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, &key, Arc::new(NoSessions), n, &s, None));
        wait_until(|| handle.status().assistants[0].connected);
        let new_key = Keypair::generate().public_b64();
        lock(&handle.shared).conns.get(&id).unwrap().tx.send(Down::Move { host: "10.0.0.9".into(), port: 5001, key: new_key.clone() }).unwrap();
        assert_eq!(run.join().unwrap(), Err(client::MOVED.into()));
        assert_eq!(*notify.moved.lock().unwrap(), Some(("10.0.0.9".into(), 5001, new_key)));
        handle.stop();
    }

    #[test]
    fn an_older_client_without_caps_cannot_be_promoted() {
        let (handle, port) = test_server();
        let (ws, _, key) = pair_client(&handle, port, "old");
        drop(ws);
        wait_until(|| !handle.status().assistants[0].connected);
        let (config, stop) = (assistant_config(port, &handle), Arc::new(AtomicBool::new(false)));
        let s = stop.clone();
        let run = std::thread::spawn(move || client::run_once(&config, &key, Arc::new(NoSessions), Arc::new(Leader::default()), &s, None));
        wait_until(|| handle.status().assistants[0].connected);
        assert!(!handle.status().assistants[0].can_lead && !handle.status().assistants[0].can_follow);
        stop.store(true, Ordering::SeqCst);
        let _ = run.join();
        handle.stop();
    }

    /// A stranger at the main's address: takes the first frame, answers it
    /// with `answer`, then sends a plain-text command. Returns its port.
    fn fake_main(answer: fn(&mut WebSocket<TcpStream>)) -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let _ = ws.read();
            answer(&mut ws);
            let _ = ws.send(Message::text(encode(&Down::Command { id: 1, kind: CommandKind::Compact { session: "r1".into() } })));
            let _ = ws.read();
        });
        port
    }

    #[test]
    fn the_client_keeps_nothing_from_a_main_that_does_not_know_the_code() {
        // It answers SPAKE2 with its own guess, then garbage where the
        // handshake goes on.
        let port = fake_main(|ws| {
            let (_, ours) = spake2::Spake2::<spake2::Ed25519Group>::start_b(&spake2::Password::new(b"999999"), &spake2::Identity::new(b"maya-join"), &spake2::Identity::new(b"maya-main"));
            let mut b = vec![secure::KIND_MESSAGE];
            b.extend(ours);
            let _ = ws.send(Message::binary(b));
            let _ = ws.read();
            let mut garbage = vec![secure::KIND_MESSAGE];
            garbage.extend([7u8; 96]);
            let _ = ws.send(Message::binary(garbage));
        });
        let exec = Arc::new(FakeExec::default());
        let notify = Arc::new(FakeClientNotify::default());
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), exec.clone(), notify.clone(), &AtomicBool::new(false), Some("123456"));
        assert_eq!(out, Err(client::PAIRING_FAILED.to_string()));
        assert!(notify.key.lock().unwrap().is_none(), "nothing is saved");
        assert!(notify.main.lock().unwrap().is_none(), "never reported as connected");
        assert!(exec.kinds.lock().unwrap().is_empty());
    }

    #[test]
    fn the_client_refuses_a_main_that_does_not_hold_the_pinned_key() {
        let port = fake_main(|ws| {
            let mut garbage = vec![secure::KIND_MESSAGE];
            garbage.extend([7u8; 48]);
            let _ = ws.send(Message::binary(garbage));
        });
        let exec = Arc::new(FakeExec::default());
        let notify = Arc::new(FakeClientNotify::default());
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, main_key: crate::net::identity::encode_key(&Keypair::generate().public), ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), exec.clone(), notify.clone(), &AtomicBool::new(false), None);
        assert_eq!(out, Err(client::KEY_CHANGED.to_string()));
        assert!(exec.kinds.lock().unwrap().is_empty(), "no command from an impostor runs");
        assert!(notify.main.lock().unwrap().is_none());
        assert_eq!(*notify.errors.lock().unwrap(), [client::KEY_CHANGED]);
    }

    #[test]
    fn the_client_says_the_main_is_older_when_it_speaks_plain_text() {
        let port = fake_main(|ws| {
            let _ = ws.send(Message::text(r#"{"type":"challenge","nonce":"n"}"#));
        });
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, main_key: crate::net::identity::encode_key(&Keypair::generate().public), ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), Arc::new(FakeExec::default()), Arc::new(FakeClientNotify::default()), &AtomicBool::new(false), None);
        assert_eq!(out, Err(client::OLD_MAIN.to_string()));
    }

    #[test]
    fn a_move_with_a_bad_key_is_not_followed() {
        let (handle, port) = test_server();
        let me = Keypair::generate();
        let (ws, id, _) = pair_as(&handle, port, &me, "laptop", &["follow"]);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.connected));
        let leader = Arc::new(Leader { caps: vec!["follow".into()], ..Default::default() });
        let stop = Arc::new(AtomicBool::new(false));
        let config = assistant_config(port, &handle);
        let (l, s, k) = (leader.clone(), stop.clone(), me.clone());
        let run = std::thread::spawn(move || client::run_once(&config, &k, Arc::new(FakeExec::default()), l, &s, None));
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let tx = handle.shared.lock().unwrap().conns.get(&id).unwrap().tx.clone();
        tx.send(Down::Move { host: "10.0.0.9".into(), port: 4127, key: "not a key".into() }).unwrap();
        let out = run.join().unwrap();
        assert!(out.unwrap_err().starts_with("Could not follow the main Maya to 10.0.0.9"));
        assert!(leader.moved.lock().unwrap().is_none(), "the old main is kept");
        stop.store(true, Ordering::SeqCst);
        handle.stop();
    }

    #[test]
    fn a_pairing_the_main_cannot_save_is_undone_and_the_assistant_keeps_nothing() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        counter.fail_saves.store(true, Ordering::SeqCst);
        // On the wire: the handshake, then `bye` instead of `welcome`.
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let mut raw = connect(port);
        let (noise, _) = secure::pair(&mut raw, &Keypair::generate(), &code, soon(), &|| false).unwrap();
        let mut ws = Channel::new(raw, noise);
        send(&mut ws, &hello_up("desk", &[]));
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == PAIRING_NOT_SAVED));
        wait_until(|| handle.ctx.live.load(Ordering::SeqCst) == 0);
        assert!(handle.status().assistants.is_empty(), "no entry on the main");
        assert!(counter.saved.lock().unwrap().is_empty());
        // The real client reports it and keeps no key.
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let notify = Arc::new(FakeClientNotify::default());
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "desk".into(), ..Default::default() };
        let out = client::run_once(&config, &Keypair::generate(), Arc::new(FakeExec::default()), notify.clone(), &AtomicBool::new(false), Some(&code));
        assert_eq!(out, Err("The main could not save the pairing.".to_string()));
        assert!(notify.key.lock().unwrap().is_none(), "nothing is stored on the assistant");
        assert!(notify.main.lock().unwrap().is_none(), "never reported as connected");
        wait_until(|| handle.ctx.live.load(Ordering::SeqCst) == 0);
        assert!(handle.status().assistants.is_empty());
        // A machine pairing again whose save fails keeps its old entry.
        counter.fail_saves.store(false, Ordering::SeqCst);
        let me = Keypair::generate();
        let (first, id, _) = pair_as(&handle, port, &me, "desk", &[]);
        wait_until(|| counter.paired.load(Ordering::SeqCst) == 1);
        drop(first);
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        counter.fail_saves.store(true, Ordering::SeqCst);
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let mut raw = connect(port);
        let (noise, _) = secure::pair(&mut raw, &me, &code, soon(), &|| false).unwrap();
        let mut ws = Channel::new(raw, noise);
        send(&mut ws, &hello_up("renamed", &[]));
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == PAIRING_NOT_SAVED));
        wait_until(|| handle.ctx.live.load(Ordering::SeqCst) == 0);
        let kept = handle.shared.lock().unwrap().paired.clone();
        let key = crate::net::identity::encode_key(&me.public);
        assert_eq!(kept.iter().map(|a| (a.id.as_str(), a.name.as_str(), a.key.as_str())).collect::<Vec<_>>(), [(id.as_str(), "desk", key.as_str())]);
        handle.stop();
    }

    #[test]
    fn a_removal_the_main_cannot_save_changes_nothing() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (mut ws, id, _) = pair_client(&handle, port, "desk");
        send(&mut ws, &Up::Board { cards: vec![card("r1", "remote")], dirs: vec![], agents: None, remembered: None });
        wait_until(|| !handle.boards().is_empty());
        counter.fail_saves.store(true, Ordering::SeqCst);
        let err = handle.remove_assistant(&id).unwrap_err();
        assert!(err.contains("disk full"), "{err}");
        let status = handle.status();
        assert_eq!(status.assistants.len(), 1, "still paired");
        assert!(status.assistants[0].connected, "still connected: no bye went out");
        assert_eq!(handle.boards().len(), 1, "its cards stay");
        counter.fail_saves.store(false, Ordering::SeqCst);
        assert_eq!(handle.remove_assistant(&id), Ok(()));
        assert!(matches!(recv(&mut ws), Down::Bye { reason } if reason == "removed"));
        assert!(handle.status().assistants.is_empty());
        assert!(counter.saved.lock().unwrap().is_empty());
        handle.stop();
    }

    #[test]
    fn pair_again_stops_the_running_client_first_and_keeps_the_entry() {
        let (handle, port) = test_server();
        let exec = Arc::new(FakeExec::default());
        let me = Keypair::generate();
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "Gnowee".into(), ..Default::default() };
        let entry_of = |handle: &ServerHandle| lock(&handle.shared).paired.iter().find(|p| p.key == me.public_b64()).map(|p| p.id.clone());
        // Pairing on its own connection, ended once the main's key is kept.
        let pair = |notify: Arc<FakeClientNotify>| {
            let (code, _) = handle.open_pairing(now_ms()).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let (c, k, e, n, s) = (config.clone(), me.clone(), exec.clone(), notify.clone(), stop.clone());
            let t = std::thread::spawn(move || client::run_once(&c, &k, e, n, &s, Some(&code)));
            wait_until(|| notify.key.lock().unwrap().is_some());
            stop.store(true, Ordering::SeqCst);
            assert_eq!(t.join().unwrap(), Ok(()));
            notify.key.lock().unwrap().clone().unwrap()
        };
        let main_key = pair(Arc::new(FakeClientNotify::default()));
        assert_eq!(main_key, crate::net::identity::encode_key(&handle.public_key()));
        let id = entry_of(&handle).expect("paired");
        // The client runs with that key pinned and is connected as that entry.
        let linked = NetworkConfig { main_key, ..config.clone() };
        let (k, e, n) = (me.clone(), exec.clone(), Arc::new(FakeClientNotify::default()));
        let running = client::ClientHandle::spawn(move |stop| {
            let _ = client::run_once(&linked, &k, e, n, &stop, None);
        });
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        // "Pair again": the running client is stopped and gone before the pairing starts.
        client::pair_after_stopping(Some(running), || Ok::<_, String>(pair(Arc::new(FakeClientNotify::default())))).unwrap();
        assert_eq!(entry_of(&handle), Some(id), "the same machine keeps its entry");
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
        let me = Keypair::generate();
        let run = |config: NetworkConfig, code: Option<String>| {
            let (k, e, n, s) = (me.clone(), exec.clone(), notify.clone(), stop.clone());
            std::thread::spawn(move || client::run_once(&config, &k, e, n, &s, code.as_deref()))
        };
        // No pairing window is open, so any code is wrong.
        let err = run(config.clone(), Some("123456".into())).join().unwrap().unwrap_err();
        assert_eq!(err, "Wrong or expired pairing code.");
        let (code, _) = handle.open_pairing(now_ms()).unwrap();
        let t = run(config.clone(), Some(code));
        wait_until(|| handle.boards().iter().any(|b| b.machine == "laptop" && b.cards.len() == 1 && b.dirs == ["proj"]));
        assert_eq!(notify.main.lock().unwrap().clone(), Some(crate::net::local_hostname()));
        let mut named = NetworkConfig::default();
        named.name = "  desk  ".into();
        assert_eq!(main_name_of(&named), "desk", "a configured name is what assistants see");
        assert_eq!(main_name_of(&NetworkConfig::default()), crate::net::local_hostname());
        let main_key = notify.key.lock().unwrap().clone().expect("the client was paired");
        let id = lock(&handle.shared).paired.iter().find(|p| p.key == me.public_b64()).unwrap().id.clone();
        // A command from the main runs on the client's executor and its result comes back.
        let out = send_command_with(&handle.shared, "laptop", CommandKind::Compact { session: "r1".into() }, Duration::from_secs(5));
        assert_eq!(out, Ok(None));
        assert_eq!(*exec.kinds.lock().unwrap(), [CommandKind::Compact { session: "r1".into() }]);
        stop.store(true, Ordering::SeqCst);
        assert_eq!(t.join().unwrap(), Ok(()), "a stop ends the connection cleanly");
        wait_until(|| handle.status().assistants.iter().all(|a| !a.connected));
        // With the main's key pinned it reconnects by IK; removal ends it for good.
        stop.store(false, Ordering::SeqCst);
        let t = run(NetworkConfig { main_key, ..config }, None);
        wait_until(|| handle.status().assistants.iter().any(|a| a.connected));
        handle.remove_assistant(&id).unwrap();
        assert_eq!(t.join().unwrap().unwrap_err(), client::REMOVED);
        assert_eq!(notify.removed.load(Ordering::SeqCst), 1);
        handle.stop();
    }
    /// The client's executor that holds its first board until told: the
    /// connection is welcomed and the main's pump is reading when it goes.
    struct GatedExec {
        gate: Mutex<Receiver<()>>,
        holding: AtomicBool,
    }

    impl client::Executor for GatedExec {
        fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
            Ok(None)
        }
        fn board(&self) -> (Vec<Card>, Vec<String>) {
            self.holding.store(true, Ordering::SeqCst);
            let _ = self.gate.lock().unwrap().recv_timeout(Duration::from_secs(5));
            (vec![card("r1", "remote")], vec![])
        }
    }

    #[test]
    fn a_websocket_handshake_slower_than_a_read_tick_still_completes() {
        let (handle, port) = test_server();
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        // The request comes after the main's first read has timed out.
        std::thread::sleep(TICK * 3);
        assert!(tungstenite::client(format!("ws://127.0.0.1:{port}/"), stream).is_ok(), "the main answered the upgrade");
        handle.stop();
    }

    #[test]
    fn a_client_removed_while_its_board_is_on_the_way_is_told_it_was_removed() {
        let (handle, port) = test_server();
        let config = NetworkConfig { role: NetworkRole::Assistant, main_host: "127.0.0.1".into(), main_port: port, name: "laptop".into(), ..Default::default() };
        for round in 0..25 {
            let (code, _) = handle.open_pairing(now_ms()).unwrap();
            let (go, gate) = channel();
            let exec = Arc::new(GatedExec { gate: Mutex::new(gate), holding: AtomicBool::new(false) });
            let notify = Arc::new(FakeClientNotify::default());
            let me = Keypair::generate();
            let (c, k, e, n) = (config.clone(), me.clone(), exec.clone(), notify.clone());
            let t = std::thread::spawn(move || client::run_once(&c, &k, e, n, &AtomicBool::new(false), Some(&code)));
            // Welcomed, its first board not yet sent.
            wait_until(|| exec.holding.load(Ordering::SeqCst));
            assert!(notify.key.lock().unwrap().is_some(), "the client was paired");
            let id = lock(&handle.shared).paired.iter().find(|p| p.key == me.public_b64()).unwrap().id.clone();
            handle.remove_assistant(&id).unwrap();
            // The board reaches a main that no longer counts the connection.
            go.send(()).unwrap();
            assert_eq!(t.join().unwrap(), Err(client::REMOVED.to_string()), "round {round}");
            assert_eq!(notify.removed.load(Ordering::SeqCst), 1, "round {round}");
            wait_until(|| handle.ctx.live.load(Ordering::SeqCst) == 0);
        }
        handle.stop();
    }

    /// A Maya that plays either side: its client hands `promoted` a real
    /// server, and `moved` rewrites the config `reconnect` reads.
    struct Mobile {
        config: Arc<Mutex<NetworkConfig>>,
        /// Its key pair, for its client and, once promoted, its server.
        identity: Arc<Keypair>,
        server: Mutex<Option<ServerHandle>>,
        counter: Arc<Counter>,
        /// Fails `promoted` as a taken port would.
        port_taken: bool,
    }

    impl client::ClientNotify for Mobile {
        fn paired(&self, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, _: &str) {}
        fn removed(&self) {}
        fn caps(&self) -> Vec<String> {
            vec!["follow".into(), "lead".into()]
        }
        fn promoted(&self, list: Vec<crate::config::PairedAssistant>, _summary: Option<String>) -> Result<u16, String> {
            if self.port_taken {
                return Err("Could not listen on port 4127: Address already in use. Choose another port.".into());
            }
            let mut c = crate::config::Config::default();
            c.network = self.config.lock().unwrap().clone();
            crate::net::promote::as_main(&mut c, list);
            let view = Arc::new(Mutex::new(c.network.clone()));
            let handle = start_with(self.counter.clone(), view, 0, self.identity.clone())?;
            let _ = self.counter.server.set(handle.shared.clone());
            *self.config.lock().unwrap() = c.network;
            let port = handle.port();
            *self.server.lock().unwrap() = Some(handle);
            Ok(port)
        }
        fn moved(&self, host: &str, port: u16, key: &str) -> Result<(), String> {
            let mut c = self.config.lock().unwrap();
            c.main_host = host.into();
            c.main_port = port;
            c.main_key = key.into();
            Ok(())
        }
    }

    /// Pairs a `Mobile` with `handle` and runs its client until `stop`;
    /// returns it, its entry id, its stop flag and its thread.
    fn run_mobile(handle: &ServerHandle, port: u16, name: &str, port_taken: bool) -> (Arc<Mobile>, String, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
        let (ws, id, key) = pair_client(handle, port, name);
        drop(ws);
        wait_until(|| !handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        let config = Arc::new(Mutex::new(NetworkConfig { name: name.into(), ..assistant_config(port, handle) }));
        let identity = Arc::new(key);
        let m = Arc::new(Mobile { config: config.clone(), identity: identity.clone(), server: Mutex::new(None), counter: Arc::new(Counter::default()), port_taken });
        let stop = Arc::new(AtomicBool::new(false));
        let (n, s) = (m.clone(), stop.clone());
        let t = std::thread::spawn(move || client::reconnect(&move || config.lock().unwrap().clone(), identity, Arc::new(NoSessions), n, &s));
        wait_until(|| handle.status().assistants.iter().any(|a| a.id == id && a.connected));
        (m, id, stop, t)
    }

    #[test]
    fn a_promoted_assistant_becomes_the_main_and_everyone_follows_it_with_their_own_keys() {
        let (handle, port) = test_server();
        let (g, g_id, g_stop, g_thread) = run_mobile(&handle, port, "laptop", false);
        let (b, _, b_stop, b_thread) = run_mobile(&handle, port, "mini", false);
        let out = handle.promote(&g_id).unwrap();
        g_thread.join().unwrap();
        let new_main = g.server.lock().unwrap().clone().expect("the laptop runs a server");
        assert_eq!(out.port, new_main.port());
        assert_eq!(out.moved, ["mini"]);
        assert_eq!(out.main_key, crate::net::identity::encode_key(&new_main.public_key()), "the new main serves with the key it paired with");
        // B reconnects to the laptop by itself, with the key it always had, pinning the laptop's.
        wait_until(|| new_main.status().assistants.iter().any(|a| a.name == "mini" && a.connected));
        assert_eq!(b.config.lock().unwrap().main_port, out.port);
        assert_eq!(b.config.lock().unwrap().main_key, out.main_key);
        // The old main joins as an assistant with its own key.
        let mut old = crate::config::Config::default();
        crate::net::promote::as_assistant_of(&mut old, &out.host, out.port, &out.main_key);
        let old_key = handle.ctx.identity.clone();
        let me = lock(&new_main.shared).paired.iter().find(|p| p.key == old_key.public_b64()).map(|p| p.id.clone()).expect("the old main's entry");
        handle.stop();
        let (m_stop, m_cfg) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(old.network)));
        let s = m_stop.clone();
        let m_thread = std::thread::spawn(move || client::reconnect(&move || m_cfg.lock().unwrap().clone(), old_key, Arc::new(NoSessions), Arc::new(Leader::default()), &s));
        wait_until(|| new_main.status().assistants.iter().any(|a| a.id == me && a.connected));
        assert!(g.counter.announced.lock().unwrap().is_empty(), "nothing on the first boards is announced");
        for s in [&b_stop, &m_stop, &g_stop] {
            s.store(true, Ordering::SeqCst);
        }
        let _ = (b_thread.join(), m_thread.join());
        new_main.stop();
    }

    #[test]
    fn an_assistant_whose_port_is_taken_stays_an_assistant_and_nobody_moves() {
        let (handle, port) = test_server();
        let (g, g_id, g_stop, g_thread) = run_mobile(&handle, port, "laptop", true);
        let (_b, _, b_stop, b_thread) = run_mobile(&handle, port, "mini", false);
        let err = handle.promote(&g_id).unwrap_err();
        assert!(err.starts_with("laptop could not become the main: Could not listen on port 4127"), "{err}");
        assert!(g.server.lock().unwrap().is_none());
        assert!(handle.status().assistants.iter().all(|a| a.connected), "both still assist this main");
        for s in [&g_stop, &b_stop] {
            s.store(true, Ordering::SeqCst);
        }
        let _ = (g_thread.join(), b_thread.join());
        handle.stop();
    }

    #[derive(Default)]
    struct Mirrorer {
        lane: Arc<client::RequestLane>,
        offers: Mutex<Vec<Option<bool>>>,
        mirrors: Mutex<Vec<(bool, Vec<Card>)>>,
    }

    impl client::ClientNotify for Mirrorer {
        fn paired(&self, _: &str) {}
        fn connected(&self, _: &str) {}
        fn disconnected(&self, _: &str) {}
        fn removed(&self) {}
        fn caps(&self) -> Vec<String> {
            vec!["mirror".into()]
        }
        fn mirror_offer(&self, offered: Option<bool>) {
            self.offers.lock().unwrap().push(offered);
        }
        fn mirrored(&self, allowed: bool, cards: Vec<Card>, _: Vec<MachineInfo>) {
            self.mirrors.lock().unwrap().push((allowed, cards));
        }
        fn lane(&self) -> Option<Arc<client::RequestLane>> {
            Some(self.lane.clone())
        }
    }

    /// Pairs a mirroring client with `handle` and runs it until `stop`.
    fn run_mirrorer(handle: &ServerHandle, port: u16) -> (Arc<Mirrorer>, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
        let (code, _) = handle.open_pairing(crate::store::now_ms()).unwrap();
        let me = Arc::new(Keypair::generate());
        let (_, main_key) = client::pair_with(Arc::new(NoSessions), me.clone(), "127.0.0.1", port, "Pixel", &code, vec!["mirror".into()]).unwrap();
        assert_eq!(main_key, crate::net::identity::encode_key(&handle.public_key()));
        let m = Arc::new(Mirrorer::default());
        let stop = Arc::new(AtomicBool::new(false));
        let config = assistant_config(port, handle);
        let (n, s) = (m.clone(), stop.clone());
        let t = std::thread::spawn(move || client::reconnect(&move || config.clone(), me, Arc::new(NoSessions), n, &s));
        wait_until(|| m.offers.lock().unwrap().as_slice() == [Some(true)]);
        (m, stop, t)
    }

    #[test]
    fn a_mirroring_client_hears_the_offer_gets_the_board_and_its_requests_answered() {
        let counter = Arc::new(Counter::default());
        let (handle, port) = test_server_with(counter.clone());
        let (m, stop, t) = run_mirrorer(&handle, port);
        handle.mirror(vec![card("l1", "local")]);
        wait_until(|| m.mirrors.lock().unwrap().iter().any(|(ok, cards)| *ok && cards.iter().any(|c| c.session_id == "l1")));
        let lane = m.lane.clone();
        let slow = {
            let l = lane.clone();
            std::thread::spawn(move || l.request(None, Request::Command(CommandKind::Compact { session: "slow".into() }), Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(lane.request(Some("laptop".into()), Request::ListAgents, Duration::from_secs(5)), Ok(Some(serde_json::json!(format!("{:?}", Request::ListAgents)))));
        let slow_answer = format!("{:?}", Request::Command(CommandKind::Compact { session: "slow".into() }));
        assert_eq!(slow.join().unwrap(), Ok(Some(serde_json::json!(slow_answer))), "each answer reaches its own caller");
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
        assert_eq!(lane.request(None, Request::ListDirs, Duration::from_secs(1)).unwrap_err(), client::NOT_CONNECTED);
        handle.stop();
    }

    #[test]
    fn a_request_waiting_when_the_main_goes_away_fails_at_once() {
        let (handle, port) = test_server();
        let (m, stop, t) = run_mirrorer(&handle, port);
        let lane = m.lane.clone();
        let began = Instant::now();
        let waiting = std::thread::spawn(move || lane.request(None, Request::Command(CommandKind::Compact { session: "slow".into() }), Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(100));
        handle.stop();
        assert_eq!(waiting.join().unwrap().unwrap_err(), client::LOST);
        assert!(began.elapsed() < Duration::from_secs(5), "not left to time out");
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
    }

    fn was(id: &str) -> crate::remembered::Remembered {
        crate::remembered::Remembered { agent: crate::model::Harness::ClaudeCode, session_id: id.into(), dir: "/Users/x/dev/calibre".into(), name: format!("{id}-name"), first_seen_ms: 1, last_seen_ms: 2, gone_at_ms: None, offered: true, offered_at_ms: None, error: None }
    }

    #[test]
    fn an_assistant_s_remembered_list_is_kept_while_it_is_away_and_a_dismiss_drops_it() {
        let (handle, port) = test_server();
        let (mut ws, _, _) = pair_client(&handle, port, "desk");
        send(&mut ws, &Up::Board { cards: vec![], dirs: vec![], agents: None, remembered: Some(vec![was("r9"), was("r8")]) });
        wait_until(|| handle.boards().iter().any(|b| b.remembered.as_ref().is_some_and(|l| l.len() == 2)));
        assert!(handle.drop_remembered("desk", &[was("r8").key()]), "connected: the Forget can follow");
        drop(ws);
        wait_until(|| handle.boards().iter().any(|b| !b.connected));
        assert_eq!(handle.boards()[0].remembered.as_ref().map(Vec::len), Some(1), "kept while it is away");
        assert!(!handle.drop_remembered("desk", &[was("r9").key()]), "away: nothing to send it to");
        assert_eq!(handle.boards()[0].remembered.as_ref().map(Vec::len), Some(0));
        assert!(!handle.drop_remembered("nobody", &[was("r9").key()]));
        handle.stop();
    }

    #[test]
    fn the_mirror_carries_the_remembered_rows_with_the_main_s_own_named() {
        let (handle, port) = test_server();
        let (mut phone, _, _, _) = pair_client_caps(&handle, port, "Pixel", &["mirror"]);
        wait_until(|| handle.status().assistants.iter().any(|a| a.connected));
        handle.mirror(vec![]);
        let Down::Mirror { remembered, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        assert!(remembered.is_empty());
        let local = crate::remembered::RememberedRow { machine: None, waiting: false, entry: was("l1") };
        let away = crate::remembered::RememberedRow { machine: Some("laptop".into()), waiting: true, entry: was("r1") };
        handle.mirror_remembered(vec![local, away]);
        let Down::Mirror { remembered, cards, .. } = recv(&mut phone) else { panic!("expected a mirror") };
        let main = main_name_of(&NetworkConfig::default());
        assert!(cards.is_empty(), "the board it already had");
        assert_eq!(remembered[0].machine.as_deref(), Some(main.as_str()), "the main's own rows carry its name");
        assert_eq!((remembered[1].machine.as_deref(), remembered[1].waiting), (Some("laptop"), true));
        handle.stop();
    }

    mod tamper {
        include!("tamper_tests.rs");
    }
}
