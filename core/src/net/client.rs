//! The assistant Maya's client: one WebSocket to the main, opened by this
//! side, reconnecting with backoff.
//!
//! Threads: the app's `start` spawns one connection thread that loops
//! `run_once` through `reconnect`, waiting `backoff_ms` between failures. `run_once` owns the socket, and
//! its read timeout (`TICK`) is the loop's tick: each turn sends the results
//! finished commands queued, a board when one is due, a ping every ten
//! seconds, then tries one read. Each command runs on its own short-lived
//! thread and queues its `result` on the connection's channel; a result that
//! finishes after its connection closed is dropped. The stop flag ends the
//! connection thread within a tick (within the connect timeout while a TCP
//! connect is in flight).
//!
//! Locks: none of its own. The app's adapters (`net_app`) take `network` or
//! `store` one at a time, never one while holding the other.

use super::protocol::{decode_down, encode, mac, mac_matches, new_nonce, Attachment, CommandKind, Down, Up, DEFAULT_PORT, MAX_FRAME, PROTOCOL};
use crate::config::{NetworkConfig, NetworkRole};
use crate::log;
use crate::model::Card;
use base64::Engine;
use serde_json::Value;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

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

pub const REMOVED: &str = "Removed by the main Maya; pair again.";
/// What the user reads when the main refuses a pairing code.
pub const WRONG_CODE: &str = "Wrong or expired pairing code.";
pub const NOT_PAIRED: &str = "Not paired with a main Maya yet; pair in Settings.";
pub const MAIN_UNPROVEN: &str = "The main Maya failed to prove it holds the pairing token.";
pub const PAIRING_UNPROVEN: &str = "The main Maya failed to prove it knows the pairing code.";
/// How long the client waits before trying again after `MAIN_UNPROVEN`.
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
}

/// What the client tells the app.
pub trait ClientNotify: Send + Sync {
    /// The main minted this assistant's id and token (pairing only).
    fn paired(&self, id: &str, token: &str);
    fn connected(&self, main_name: &str);
    fn disconnected(&self, error: &str);
    fn removed(&self);
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
        "too many attempts" => "Too many wrong codes; wait five minutes and try again.".into(),
        "authentication failed" => "The main Maya did not accept this assistant's key; pair again.".into(),
        "protocol" => "The main Maya runs a different version of Maya.".into(),
        "could not save the pairing" => "The main could not save the pairing.".into(),
        other => format!("The main Maya closed the connection ({}).", clean(other)),
    }
}

type Ws = WebSocket<TcpStream>;

fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

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
    let url = if host.contains(':') && !host.starts_with('[') { format!("ws://[{host}]:{port}/") } else { format!("ws://{host}:{port}/") };
    // Commands carry attachments inline: allow what the protocol allows.
    let ws_config = WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME));
    let mut attempt = tungstenite::client::client_with_config(url, stream, Some(ws_config));
    loop {
        match attempt {
            Ok((ws, _)) => return Ok(ws),
            Err(HandshakeError::Interrupted(mid)) => {
                if stopped(stop) || Instant::now() > deadline {
                    return Err(format!("{host}:{port} did not answer as a Maya."));
                }
                attempt = mid.handshake();
            }
            Err(HandshakeError::Failure(e)) => return Err(format!("{host}:{port} did not answer as a Maya: {e}")),
        }
    }
}

fn send_up(ws: &mut Ws, up: &Up) -> Result<(), String> {
    ws.send(Message::text(encode(up))).map_err(|e| format!("Lost the connection to the main Maya: {e}"))
}

/// The next frame from the main that parses, by `deadline`; garbage is logged and skipped.
fn read_down(ws: &mut Ws, stop: &AtomicBool, deadline: Instant) -> Result<Down, String> {
    loop {
        if stopped(stop) {
            return Err("stopped".into());
        }
        match ws.read() {
            Ok(Message::Text(t)) => match decode_down(&t) {
                Ok(down) => return Ok(down),
                Err(e) => log::line("network", format!("main: ignored a message: {e}")),
            },
            Ok(Message::Close(_)) => return Err("The main Maya closed the connection.".into()),
            Ok(_) => {}
            Err(e) if is_timeout(&e) => {
                if Instant::now() > deadline {
                    return Err("The main Maya did not answer in time.".into());
                }
            }
            Err(e) => return Err(format!("Lost the connection to the main Maya: {e}")),
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
            Err(e) if is_timeout(&e) => {}
            Err(_) => break,
        }
    }
}

/// One connection to the main: pairs with `code` (the token is then empty)
/// or authenticates with the token, sends boards and runs commands until the
/// link drops (`Err`) or `stop` is set (`Ok`). The notifier hears of every
/// failure except one after a stop.
pub fn run_once(config: &NetworkConfig, exec: Arc<dyn Executor>, notify: Arc<dyn ClientNotify>, stop: &AtomicBool, code: Option<&str>) -> Result<(), String> {
    match connection(config, &exec, &*notify, stop, code) {
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

fn connection(config: &NetworkConfig, exec: &Arc<dyn Executor>, notify: &dyn ClientNotify, stop: &AtomicBool, code: Option<&str>) -> Result<(), String> {
    if code.is_none() && (config.assistant_id.is_empty() || config.token.is_empty()) {
        return Err(NOT_PAIRED.into());
    }
    let deadline = Instant::now() + SILENCE;
    let mut ws = connect(config, stop, deadline)?;
    let out = handshake(&mut ws, config, notify, stop, code, deadline).and_then(|main| {
        log::line("network", format!("connected to {main}"));
        notify.connected(&main);
        pump(&mut ws, exec, stop)
    });
    finish(&mut ws);
    out
}

/// `hello`, then pairing or challenge-response, up to `welcome`; returns the main's name.
fn handshake(ws: &mut Ws, config: &NetworkConfig, notify: &dyn ClientNotify, stop: &AtomicBool, code: Option<&str>, deadline: Instant) -> Result<String, String> {
    let hostname = super::local_hostname();
    let id = if code.is_some() { None } else { Some(config.assistant_id.clone()) };
    let hello = Up::Hello { protocol: PROTOCOL, app: env!("CARGO_PKG_VERSION").into(), name: display_name(&config.name, &hostname), hostname, platform: std::env::consts::OS.into(), id };
    send_up(ws, &hello)?;
    let code = code.map(str::trim);
    // The nonce this side sent with `pair` or `auth`; the main must answer it
    // in `welcome`, under the code or the token.
    let mut ours: Option<String> = None;
    if let Some(code) = code {
        let mine = new_nonce();
        send_up(ws, &Up::Pair { code: code.to_string(), nonce: mine.clone() })?;
        ours = Some(mine);
    }
    // The id and token from `paired`, kept until the main proves it knows the code.
    let mut creds: Option<(String, String)> = None;
    loop {
        match read_down(ws, stop, deadline)? {
            Down::Challenge { nonce } if code.is_none() => {
                let mine = new_nonce();
                send_up(ws, &Up::Auth { mac: mac(&config.token, &nonce), nonce: mine.clone() })?;
                ours = Some(mine);
            }
            Down::Paired { id, token } if code.is_some() => {
                if id.is_empty() || token.is_empty() {
                    return Err("The main Maya sent an empty key.".into());
                }
                creds = Some((id, token));
            }
            Down::Welcome { name, mac: proof } => {
                let proven = |key: &str| ours.as_ref().is_some_and(|n| mac_matches(key, n, &proof));
                match code {
                    Some(code) => {
                        let Some((id, token)) = creds.take() else {
                            return Err("The main Maya did not pair this assistant.".into());
                        };
                        // Whatever answered must know the code just typed;
                        // only then are the credentials worth keeping.
                        if !proven(code) {
                            log::line("network", "the main did not prove it knows the pairing code");
                            return Err(PAIRING_UNPROVEN.into());
                        }
                        notify.paired(&id, &token);
                    }
                    // With a stored token, the main proves it holds it too.
                    None if !proven(&config.token) => {
                        log::line("network", "the main did not prove it holds the token");
                        return Err(MAIN_UNPROVEN.into());
                    }
                    None => {}
                }
                let name = clean(&name);
                return Ok(if name.is_empty() { "the main Maya".into() } else { name });
            }
            Down::Bye { reason } => return Err(bye_message(&reason)),
            other => log::line("network", format!("main: ignored {} before welcome", encode(&other).chars().take(40).collect::<String>())),
        }
    }
}

/// Runs a welcomed connection until it drops or `stop` is set.
fn pump(ws: &mut Ws, exec: &Arc<dyn Executor>, stop: &AtomicBool) -> Result<(), String> {
    let (tx, rx) = channel::<Up>();
    let mut last_read = Instant::now();
    let mut last_ping = Instant::now();
    // When the last board went out, and what it said.
    let mut last_board: Option<(Instant, String)> = None;
    // The first board goes out right after `welcome`.
    let mut board_wanted = true;
    loop {
        if stopped(stop) {
            return Ok(());
        }
        while let Ok(up) = rx.try_recv() {
            send_up(ws, &up)?;
        }
        board_wanted |= exec.board_requested();
        let since = last_board.as_ref().map(|(t, _)| t.elapsed());
        let refresh = since.map_or(true, |d| d >= BOARD_REFRESH);
        if since.map_or(true, |d| d >= BOARD_GAP) && (board_wanted || refresh) {
            board_wanted = false;
            let (cards, dirs) = exec.board();
            let frame = encode(&Up::Board { cards, dirs, agents: exec.agents() });
            if refresh || last_board.as_ref().map_or(true, |(_, f)| *f != frame) {
                ws.send(Message::text(frame.clone())).map_err(|e| format!("Lost the connection to the main Maya: {e}"))?;
                last_board = Some((Instant::now(), frame));
            }
        }
        if last_ping.elapsed() >= PING_EVERY {
            send_up(ws, &Up::Ping)?;
            last_ping = Instant::now();
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                last_read = Instant::now();
                match decode_down(&t) {
                    Ok(down) => handle(down, exec, &tx)?,
                    Err(e) => log::line("network", format!("main: ignored a message: {e}")),
                }
            }
            Ok(Message::Close(_)) => return Err("The main Maya closed the connection.".into()),
            Ok(_) => last_read = Instant::now(),
            Err(e) if is_timeout(&e) => {
                if last_read.elapsed() > SILENCE {
                    return Err("The main Maya stopped answering.".into());
                }
            }
            Err(e) => return Err(format!("Lost the connection to the main Maya: {e}")),
        }
    }
}

fn handle(down: Down, exec: &Arc<dyn Executor>, tx: &Sender<Up>) -> Result<(), String> {
    match down {
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
        Down::Challenge { .. } | Down::Welcome { .. } | Down::Paired { .. } => log::line("network", "main: ignored a handshake message after welcome"),
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
/// as connected and take the new pairing for a second machine at the same
/// address.
pub fn pair_after_stopping<T>(running: Option<ClientHandle>, pair: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if let Some(c) = running {
        c.stop_and_join();
    }
    pair()
}

/// Reconnects until `stop`, reading the config afresh each attempt.
pub fn reconnect(config_of: &dyn Fn() -> NetworkConfig, exec: Arc<dyn Executor>, notify: Arc<dyn ClientNotify>, stop: &AtomicBool) {
    let mut attempt = 0u32;
    while !stopped(stop) {
        let config = config_of();
        log::line("network", format!("connecting to {}:{}", config.main_host.trim(), port_of(&config)));
        let began = Instant::now();
        match run_once(&config, exec.clone(), notify.clone(), stop, None) {
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
                // holds the token: do not hammer it.
                let wait = if e == MAIN_UNPROVEN { UNPROVEN_WAIT_MS } else { backoff_ms(attempt) };
                attempt = attempt.saturating_add(1);
                log::line("network", format!("{e} Retrying in {} s.", wait / 1_000));
                let end = Instant::now() + Duration::from_millis(wait);
                while Instant::now() < end && !stopped(stop) {
                    std::thread::sleep(TICK);
                }
            }
        }
    }
}

/// The pairing handshake once. Ok((main_name, id, token)). One blocking
/// connection, at most `SILENCE` long once connected; it ends at `welcome`,
/// so no command reaches `exec`.
pub fn pair_with(exec: Arc<dyn Executor>, host: &str, port: u16, name: &str, code: &str) -> Result<(String, String, String), String> {
    let config = NetworkConfig { role: NetworkRole::Assistant, main_host: host.trim().into(), main_port: port, name: name.trim().into(), ..Default::default() };
    let stop = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(PairNotify { stop: stop.clone(), creds: Mutex::new(None), main: Mutex::new(None) });
    log::line("network", format!("pairing with {}:{}", config.main_host, port_of(&config)));
    run_once(&config, exec, notify.clone(), &stop, Some(code)).map_err(|e| {
        log::line("network", format!("pairing failed: {e}"));
        e
    })?;
    let (id, token) = notify.creds.lock().unwrap().take().ok_or("The main Maya did not pair this assistant.")?;
    let main = notify.main.lock().unwrap().take().unwrap_or_default();
    Ok((main, id, token))
}

/// Pairing's notifier: keeps the credentials and ends the connection at `welcome`.
struct PairNotify {
    stop: Arc<AtomicBool>,
    creds: Mutex<Option<(String, String)>>,
    main: Mutex<Option<String>>,
}

impl ClientNotify for PairNotify {
    fn paired(&self, id: &str, token: &str) {
        *self.creds.lock().unwrap() = Some((id.into(), token.into()));
    }
    fn connected(&self, main_name: &str) {
        *self.main.lock().unwrap() = Some(main_name.into());
        self.stop.store(true, Ordering::SeqCst);
    }
    fn disconnected(&self, _: &str) {}
    fn removed(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_hello_carries_the_name_or_the_hostname() {
        assert_eq!(display_name("", "tiagos-mbp"), "tiagos-mbp");
        assert_eq!(display_name("  laptop ", "tiagos-mbp"), "laptop");
    }
}
