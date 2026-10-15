//! The phone's main, with the page and the Android service behind traits:
//! the config, the server, the notifier diff and the alert state. The Tauri
//! layer (`lib.rs`, `commands.rs`) is thin on purpose, so this is what the
//! integration test drives.

use crate::alerts::{self, Action, Alerts, Posted, Switches};
use maya_core::config::{self, Config, NetworkRole, PairedAssistant};
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::identity::{self, Keypair};
use maya_core::net::merge::{self, RemoteBoard};
use maya_core::net::client::{check_pair_args, pair_with, reconnect, ClientHandle, ClientNotify, Executor, RequestLane, NOT_PAIRED, REMOVED, REQUEST_TIMEOUT};
use maya_core::net::protocol::{CommandKind, Request, CAP_FOLLOW, CAP_LEAD, CAP_MIRROR, DEFAULT_PORT};
use maya_core::net::routing::MachineInfo;
use maya_core::net::routing::{agents_reply, target_of, Target, ROUTE_TIMEOUT};
use maya_core::net::server::{send_command_with, start_with, Notify, ServerHandle};
use maya_core::net::{self, promote, AssistantLink, MirrorAccess, NetworkStatus};
use maya_core::notify::Notifier;
use maya_core::store::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// What the hub tells the outside: the page's events and the service.
pub trait Sink: Send + Sync {
    fn sessions(&self, cards: &[Card]);
    fn network(&self, status: &NetworkStatus);
    fn service_start(&self, line: &str);
    fn service_stop(&self);
}

/// The config and where it is saved.
pub struct Settings {
    pub path: PathBuf,
    pub config: Config,
}

/// Where the config lives under the app's data directory.
pub fn settings_path(maya_dir: &Path) -> PathBuf {
    maya_dir.join("config.json")
}

/// Loads the config, filling a blank name with the device's so the main's
/// `welcome` never says "localhost".
pub fn load_settings(maya_dir: &Path, model: &str) -> Settings {
    let path = settings_path(maya_dir);
    let mut config = config::load(&path);
    if config.network.name.trim().is_empty() {
        config.network.name = alerts::default_name(model);
    }
    Settings { path, config }
}

/// The main's board while this phone has joined it.
#[derive(Default, Clone)]
pub struct MirrorView {
    pub cards: Vec<Card>,
    pub machines: Vec<MachineInfo>,
}

/// What this phone says it can do: follow a moved main, become the main,
/// and mirror the main's board.
fn phone_caps() -> Vec<String> {
    vec![CAP_FOLLOW.into(), CAP_LEAD.into(), CAP_MIRROR.into()]
}

pub struct Hub {
    /// `<app data dir>/maya`: the config, the log and saved attachments.
    pub maya_dir: PathBuf,
    /// Serializes `refresh`, so a stale snapshot never overtakes a newer one.
    /// Lock order: `refresh` first, alone (its callers hold no other lock);
    /// then `server`, then the server's own mutex, then `settings`;
    /// `notifier`, `posted`, `client`, `link` and `mirror` are taken alone,
    /// never while holding another hub lock. `stop_client` joins the client
    /// thread, which may be in a callback that calls `refresh`, so it holds
    /// no hub lock while it joins. `lane` has its own lock, never held here.
    refresh: Mutex<()>,
    pub settings: Mutex<Settings>,
    pub server: Mutex<Option<ServerHandle>>,
    /// Why the server is not running (the port is taken…); `None` once it runs.
    pub main_error: Mutex<Option<String>>,
    /// The client that listens to the main while this phone is an assistant.
    pub client: Mutex<Option<ClientHandle>>,
    /// How that client stands with the main.
    pub link: Mutex<AssistantLink>,
    /// The joined main's last board; empty otherwise.
    pub mirror: Mutex<MirrorView>,
    /// Where the page's commands go while joined.
    pub lane: Arc<RequestLane>,
    /// This phone's key pair, read from `identity.key` once and kept.
    identity: Mutex<Option<Arc<Keypair>>>,
    /// The first mirror of the current link was seeded into the notifier.
    /// A reconnect keeps it, so an ask that began while away still notifies.
    mirror_seeded: AtomicBool,
    pub notifier: Mutex<Notifier>,
    pub posted: Mutex<Posted>,
    pub alerts: Arc<dyn Alerts>,
    pub sink: Arc<dyn Sink>,
    /// The time boards are merged and routed at: the real clock, or a
    /// test's, so expiry can be checked without waiting five minutes.
    now: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl Hub {
    pub fn new(maya_dir: PathBuf, settings: Settings, alerts: Arc<dyn Alerts>, sink: Arc<dyn Sink>) -> Arc<Hub> {
        Hub::with_clock(maya_dir, settings, alerts, sink, now_ms)
    }

    /// `new` with the clock boards are merged at.
    pub fn with_clock(maya_dir: PathBuf, settings: Settings, alerts: Arc<dyn Alerts>, sink: Arc<dyn Sink>, now: impl Fn() -> u64 + Send + Sync + 'static) -> Arc<Hub> {
        Arc::new(Hub { maya_dir, settings: Mutex::new(settings), server: Mutex::new(None), main_error: Mutex::new(None), client: Mutex::new(None), link: Mutex::new(AssistantLink::default()), refresh: Mutex::new(()), mirror: Mutex::new(MirrorView::default()), lane: Arc::new(RequestLane::default()), identity: Mutex::new(None), mirror_seeded: AtomicBool::new(false), notifier: Mutex::new(Notifier::default()), posted: Mutex::new(Posted::default()), alerts, sink, now: Box::new(now) })
    }

    fn handle(&self) -> Option<ServerHandle> {
        self.server.lock().unwrap().clone()
    }

    /// The assistants' last snapshots; none while the server is down.
    pub fn boards(&self) -> Vec<RemoteBoard> {
        self.handle().map(|s| s.boards()).unwrap_or_default()
    }

    /// The board: the joined main's mirror (greyed while the link is down),
    /// or every connected assistant's cards; never local ones.
    pub fn cards(&self) -> Vec<Card> {
        if self.joined() {
            let connected = self.link.lock().unwrap().connected;
            let mut cards = self.mirror.lock().unwrap().cards.clone();
            for c in &mut cards {
                c.stale |= !connected;
            }
            return cards;
        }
        merge::merged(vec![], &self.boards(), (self.now)())
    }

    /// This phone listens to a main (joined, or after handing its role away).
    pub fn joined(&self) -> bool {
        self.client.lock().unwrap().is_some()
    }

    /// Sends a request to the joined main and waits for its answer.
    pub fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        {
            let link = self.link.lock().unwrap();
            if link.mirror == Some(MirrorAccess::Unsupported) {
                let main = link.main_name.clone().unwrap_or_else(|| "The main Maya".into());
                return Err(format!("{main} runs an older Maya that cannot share its sessions. Update it."));
            }
        }
        self.lane.request(machine.map(str::to_string), request, REQUEST_TIMEOUT)
    }

    /// The session whose notification has this id, if its card is on the board.
    pub fn session_for_notification(&self, id: i32) -> Option<String> {
        self.cards().into_iter().map(|c| c.session_id).find(|s| alerts::notification_id(s) == id)
    }

    /// `session_for_notification`, waiting up to `timeout` for the card.
    pub fn wait_for_notification(&self, id: i32, timeout: Duration) -> Option<String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(s) = self.session_for_notification(id) {
                return Some(s);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Repaints the page and posts or clears notifications for what changed.
    /// Runs on every new board and on a timer, so a board that went quiet
    /// greys and then drops (with its notifications) without a new frame.
    pub fn refresh(&self) {
        let _serial = self.refresh.lock().unwrap();
        let cards = self.cards();
        // A Maya that mirrors this phone sees what the page shows.
        if let Some(s) = self.handle() {
            s.mirror(cards.clone());
        }
        let (fresh, finished) = {
            let mut n = self.notifier.lock().unwrap();
            (n.take_new(&cards), n.take_finished(&cards))
        };
        let switches = {
            let s = self.settings.lock().unwrap();
            Switches { awaiting: s.config.notify_on_awaiting, completed: s.config.notify_on_completed }
        };
        let actions = alerts::plan(&cards, &fresh, &finished, &mut self.posted.lock().unwrap(), switches);
        for a in actions {
            match a {
                Action::Post(p) => self.alerts.post(&p),
                Action::Clear(id) => self.alerts.clear(id),
            }
        }
        self.sink.sessions(&cards);
    }

    /// The status as the Network screen shows it: the server's, or the
    /// paired list offline with why the server is down.
    pub fn status(&self) -> NetworkStatus {
        if self.client.lock().unwrap().is_some() {
            return self.with_pair_again(NetworkStatus { role: NetworkRole::Assistant, assistant: self.link.lock().unwrap().clone(), ..Default::default() });
        }
        match self.handle() {
            Some(s) => self.with_pair_again(s.status()),
            None => {
                let paired = self.settings.lock().unwrap().config.network.assistants.clone();
                self.with_pair_again(NetworkStatus { role: NetworkRole::Off, code: None, assistants: net::paired_offline(&paired), main_error: self.main_error.lock().unwrap().clone(), ..Default::default() })
            }
        }
    }

    /// The core never fills `pair_again`; it is the config's, so every
    /// status that reaches the page goes through here.
    fn with_pair_again(&self, mut status: NetworkStatus) -> NetworkStatus {
        status.pair_again = self.settings.lock().unwrap().config.network.pair_again;
        status
    }

    /// This phone's key pair, from `identity.key` beside its config.
    pub fn identity(&self) -> Result<Arc<Keypair>, String> {
        let mut held = self.identity.lock().unwrap();
        if let Some(k) = held.as_ref() {
            return Ok(k.clone());
        }
        let k = Arc::new(identity::load_or_create(&self.maya_dir.join(identity::FILE))?);
        *held = Some(k.clone());
        Ok(k)
    }

    /// Drops every pairing and makes a fresh key: the server and the link
    /// stop and the role goes Off. Nothing stops or changes unless both the
    /// config and the key file are written.
    pub fn new_key(&self) -> Result<NetworkStatus, String> {
        let before = self.settings.lock().unwrap().config.clone();
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.assistants.clear();
            c.network.main_key.clear();
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.pair_again = false;
        })?;
        let mut held = self.identity.lock().unwrap();
        match identity::replace(&self.maya_dir.join(identity::FILE)) {
            Ok(k) => *held = Some(Arc::new(k)),
            Err(e) => {
                drop(held);
                let _ = self.update_config(|c| *c = before);
                return Err(e);
            }
        }
        drop(held);
        self.stop_server();
        self.stop_client();
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
    }

    /// Changes the config with `f` and saves it; the config in memory
    /// changes only once the save succeeded.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config, String> {
        let mut s = self.settings.lock().unwrap();
        let mut next = s.config.clone();
        f(&mut next);
        config::save(&s.path, &next).map_err(|e| format!("Could not save the settings: {e}"))?;
        s.config = next.clone();
        Ok(next)
    }

    /// Saves the paired list changed by `f`; a new pairing (`new`) also
    /// clears `pair_again`, in the same write.
    fn save_assistants(&self, new: bool, f: impl FnOnce(&mut Vec<PairedAssistant>)) -> Result<(), String> {
        let mut skipped = false;
        let saved = self.update_config(|c| {
            skipped = !change_paired(c, f);
            if new {
                c.network.pair_again = false;
            }
        })
        .map(|_| ());
        if skipped {
            log::line("network", "not saving the paired assistants: this phone is an assistant now");
        }
        saved.map_err(|e| {
            log::line("network", format!("could not save the paired assistants: {e}"));
            format!("Could not save the paired assistants: {e}")
        })
    }

    /// Starts the server on the configured port, replacing a running one,
    /// and the foreground service with it. When the port cannot be bound
    /// the error is kept for the Network screen and no service starts.
    pub fn start_server(self: &Arc<Self>) -> Result<(), String> {
        self.stop_server();
        let (port, view) = {
            let s = self.settings.lock().unwrap();
            (s.config.listen_port(), s.config.network.clone())
        };
        let notify: Arc<dyn Notify> = Arc::new(HubNotify(Arc::downgrade(self)));
        let key = match self.identity() {
            Ok(k) => k,
            Err(e) => {
                *self.main_error.lock().unwrap() = Some(e.clone());
                log::line("network", format!("server not started: {e}"));
                self.sink.network(&self.status());
                return Err(e);
            }
        };
        let mut started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port, key.clone());
        for _ in 0..5 {
            if started.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
            started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port, key.clone());
        }
        match started {
            Ok(handle) => {
                // The user pressed Make main on this phone, so the entry it hands itself may mirror.
                handle.set_own_mirror(true);
                let status = self.with_pair_again(handle.status());
                *self.server.lock().unwrap() = Some(handle);
                *self.main_error.lock().unwrap() = None;
                log::line("network", format!("server listening on port {port}"));
                self.sink.service_start(&alerts::service_line(status.assistants.len(), 0));
                self.sink.network(&status);
                Ok(())
            }
            Err(e) => {
                *self.main_error.lock().unwrap() = Some(e.clone());
                log::line("network", format!("server not started: {e}"));
                self.sink.network(&self.status());
                Err(e)
            }
        }
    }

    /// Stops the server and the service; the board empties.
    pub fn stop_server(&self) {
        let running = self.server.lock().unwrap().take();
        if let Some(s) = running {
            s.stop();
            self.sink.service_stop();
        }
        self.refresh();
        self.sink.network(&self.status());
    }

    /// Listens to the main in the config and mirrors its board; the keep-awake
    /// service runs meanwhile. That main can make this phone the main.
    pub fn start_client(self: &Arc<Self>) {
        // The old client and the last link's state go before the new client
        // starts, so its first mirror is never wiped behind it. A new link
        // seeds the notifier afresh and shows none of the last one's cards.
        let old = self.client.lock().unwrap().take();
        if let Some(old) = old {
            old.stop();
        }
        *self.link.lock().unwrap() = AssistantLink { retrying: true, ..Default::default() };
        self.mirror_seeded.store(false, Ordering::SeqCst);
        *self.mirror.lock().unwrap() = MirrorView::default();
        let key = match self.identity() {
            Ok(k) => k,
            Err(e) => {
                // With no client running, `status()` shows the Off branch,
                // which carries `main_error`: the page sees why.
                self.link.lock().unwrap().error = Some(e.clone());
                *self.main_error.lock().unwrap() = Some(e);
                self.sink.network(&self.status());
                return;
            }
        };
        let weak = Arc::downgrade(self);
        let handle = ClientHandle::spawn(move |stop| {
            let notify: Arc<dyn ClientNotify> = Arc::new(HubClientNotify { hub: weak.clone(), stop: stop.clone() });
            let config_of = move || weak.upgrade().map(|h| h.settings.lock().unwrap().config.network.clone()).unwrap_or_default();
            reconnect(&config_of, key, Arc::new(NoSessions), notify, &stop);
        });
        *self.client.lock().unwrap() = Some(handle);
        *self.main_error.lock().unwrap() = None;
        // A mirror that landed before the handle was in shows now.
        self.refresh();
        self.sink.service_start("Reconnecting…");
        self.sink.network(&self.status());
    }

    /// Stops listening: the role goes Off and the link is forgotten.
    pub fn stop_listening(&self) -> Result<NetworkStatus, String> {
        // Saved first: if the save fails, the phone keeps listening as before.
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.main_key.clear();
        })?;
        self.stop_client();
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
    }

    /// Pairs with a running main and listens to it. A phone that was the
    /// main stops its server first and gets it back if the pairing fails.
    pub fn join(self: &Arc<Self>, host: &str, port: u16, name: &str, code: &str) -> Result<NetworkStatus, String> {
        let (host, code) = check_pair_args(host, code)?;
        let port = if port == 0 { DEFAULT_PORT } else { port };
        let name = match name.trim() {
            "" => self.settings.lock().unwrap().config.network.name.clone(),
            n => n.to_string(),
        };
        let identity = self.identity()?;
        let was_main = self.handle().is_some();
        let was_joined = self.joined();
        self.stop_server();
        self.stop_client();
        let paired = pair_with(Arc::new(NoSessions), identity, &host, port, &name, &code, phone_caps()).and_then(|(main, main_key)| {
            self.update_config(|c| {
                c.network.role = NetworkRole::Assistant;
                c.network.main_host = host.clone();
                c.network.main_port = port;
                c.network.name = name.clone();
                c.network.main_key = main_key;
                c.network.pair_again = false;
            })
            .map(|_| main)
        });
        match paired {
            Ok(main) => {
                log::line("network", format!("joined {main}"));
                self.start_client();
                Ok(self.status())
            }
            Err(e) => {
                if was_main {
                    let _ = self.start_server();
                }
                // The config is unchanged, so this reconnects to the main it had.
                if was_joined {
                    self.start_client();
                }
                Err(e)
            }
        }
    }

    /// Stops listening to the main without changing the config; the board
    /// and the link are forgotten. Holds no hub lock while it joins the
    /// client thread, which may be inside a callback that refreshes.
    pub fn stop_client(&self) {
        let client = self.client.lock().unwrap().take();
        if let Some(c) = client {
            c.stop_and_join();
            self.sink.service_stop();
        }
        *self.link.lock().unwrap() = AssistantLink::default();
        *self.mirror.lock().unwrap() = MirrorView::default();
        self.refresh();
    }

    /// Makes a connected assistant the main, then listens to it.
    pub fn promote(self: &Arc<Self>, id: &str) -> Result<NetworkStatus, String> {
        let server = self.handle().ok_or("Start the server first.")?;
        let out = server.promote(id)?;
        self.update_config(|c| promote::as_assistant_of(c, &out.host, out.port, &out.main_key)).map_err(|e| format!("Could not save this Maya as an assistant: {}.", e.trim_end_matches('.')))?;
        self.stop_server();
        self.start_client();
        Ok(self.status())
    }

    /// Opens pairing, or regenerates the code when it is already open.
    pub fn pairing_code(&self) -> Result<NetworkStatus, String> {
        let Some(server) = self.handle() else {
            return Err(self.main_error.lock().unwrap().clone().unwrap_or_else(|| "Start the server first.".into()));
        };
        server.open_pairing(now_ms())?;
        Ok(server.status())
    }

    /// Forgets a paired assistant; a connected one is told and closed.
    pub fn remove_assistant(&self, id: &str) -> Result<NetworkStatus, String> {
        match self.handle() {
            Some(s) => s.remove_assistant(id)?,
            None => {
                self.save_assistants(false, |list| list.retain(|a| a.id != id))?;
                log::line("network", format!("{id}: removed"));
            }
        }
        Ok(self.status())
    }

    /// A mirroring Maya's request, as a main: every session and machine is
    /// an assistant's; this phone itself runs nothing.
    pub fn serve_request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        let to_data = |v: serde_json::Result<Value>| v.map(Some).map_err(|e| e.to_string());
        match request {
            Request::ListDirs => to_data(serde_json::to_value(machine.map(|m| merge::dirs_of(&self.boards(), m)).unwrap_or_default())),
            Request::ListAgents => to_data(serde_json::to_value(agents_reply(false, machine.and_then(|m| merge::agents_of(&self.boards(), m)), Vec::new))),
            Request::Command(kind) => match target_of(&kind) {
                Target::Session(session) => self.send(&session, |_| kind),
                Target::Machine => self.send_to(machine.ok_or("This phone runs no sessions.")?, kind),
            },
        }
    }

    /// Lets a paired Maya see and control every session, or stops it.
    pub fn set_mirror(&self, id: &str, on: bool) -> Result<NetworkStatus, String> {
        match self.handle() {
            Some(s) => s.set_mirror(id, on)?,
            None => self.save_assistants(false, |list| {
                if let Some(a) = list.iter_mut().find(|a| a.id == id) {
                    a.mirror = on;
                }
            })?,
        }
        Ok(self.status())
    }

    /// Sends a session command to the machine that lists the session. There
    /// are no local sessions: a session nobody lists is no longer running.
    pub fn send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String> {
        if self.joined() {
            return self.request(None, Request::Command(kind(session_id.to_string())));
        }
        let machine = merge::route(&[], &self.boards(), session_id, (self.now)()).ok_or("Session is no longer running.")?;
        self.send_to(&machine, kind(session_id.to_string()))
    }

    /// Sends a command to a machine by its label (start, resume, listings).
    pub fn send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String> {
        if self.joined() {
            return self.request(Some(machine), Request::Command(kind));
        }
        let shared = self.handle().map(|s| s.shared.clone()).ok_or("The server is stopped.")?;
        send_command_with(&shared, machine, kind, ROUTE_TIMEOUT)
    }
}

/// Changes the paired list with `f`, unless the config says assistant: an
/// assistant has no list, and a late save from the server that just handed
/// its role away would put the old one back. Whether `f` ran.
fn change_paired(c: &mut Config, f: impl FnOnce(&mut Vec<PairedAssistant>)) -> bool {
    if c.network.role == NetworkRole::Assistant {
        return false;
    }
    f(&mut c.network.assistants);
    true
}

/// The server's line to the hub. `paired` and `paired_list_changed` run
/// with the server's mutex held and take only `settings`; the others run
/// on the server's threads with no lock held.
struct HubNotify(Weak<Hub>);

impl HubNotify {
    fn with(&self, f: impl FnOnce(&Hub)) {
        if let Some(hub) = self.0.upgrade() {
            f(&hub);
        }
    }
}

impl Notify for HubNotify {
    fn request(&self, machine: Option<&str>, request: Request) -> Result<Option<Value>, String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.serve_request(machine, request)
    }

    fn board_changed(&self) {
        self.with(|h| h.refresh());
    }

    fn board_seeded(&self, cards: &[Card]) {
        self.with(|h| h.notifier.lock().unwrap().seed(cards));
    }

    fn status_changed(&self, status: NetworkStatus) {
        self.with(|h| {
            let connected = status.assistants.iter().filter(|a| a.connected).count();
            h.alerts.service_line(&alerts::service_line(status.assistants.len(), connected));
            h.sink.network(&h.with_pair_again(status));
        });
    }

    fn paired(&self, assistant: &PairedAssistant) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(true, |list| match list.iter_mut().find(|a| a.id == assistant.id) {
            Some(a) => *a = assistant.clone(),
            None => list.push(assistant.clone()),
        })
    }

    fn paired_list_changed(&self, assistants: &[PairedAssistant]) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(false, |list| *list = assistants.to_vec())
    }
}

/// The phone while it listens: no sessions, so every command but
/// `promote` (which the client opens itself) is refused.
pub(crate) struct NoSessions;

impl Executor for NoSessions {
    fn execute(&self, _: CommandKind) -> Result<Option<Value>, String> {
        Err("This phone runs no sessions.".into())
    }
    fn board(&self) -> (Vec<Card>, Vec<String>) {
        (vec![], vec![])
    }
}

/// The client's line to the hub.
struct HubClientNotify {
    hub: Weak<Hub>,
    stop: Arc<AtomicBool>,
}

impl HubClientNotify {
    /// Repaints the board, so its cards grey or come back with the link.
    fn refresh(&self) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        if let Some(h) = self.hub.upgrade() {
            h.refresh();
        }
    }

    fn set(&self, f: impl FnOnce(&mut AssistantLink)) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        let line = {
            let mut l = h.link.lock().unwrap();
            f(&mut l);
            match (&l.main_name, l.connected) {
                (Some(m), true) => format!("Connected to {m}"),
                _ => "Reconnecting…".to_string(),
            }
        };
        h.alerts.service_line(&line);
        h.sink.network(&h.status());
    }
}

impl ClientNotify for HubClientNotify {
    fn paired(&self, _: &str) {}
    fn connected(&self, main_name: &str) {
        let name = main_name.to_string();
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false, mirror: None });
        self.refresh();
    }
    fn disconnected(&self, error: &str) {
        let retrying = error != NOT_PAIRED && error != REMOVED;
        self.set(|l| {
            l.connected = false;
            l.error = Some(error.to_string());
            l.retrying = retrying;
        });
        self.refresh();
    }
    fn removed(&self) {
        self.set(|l| {
            l.connected = false;
            l.error = Some(REMOVED.into());
            l.retrying = false;
        });
        self.refresh();
    }
    fn caps(&self) -> Vec<String> {
        phone_caps()
    }
    fn mirror_offer(&self, offered: Option<bool>) {
        let Some(h) = self.hub.upgrade() else { return };
        let access = match offered {
            Some(true) => MirrorAccess::Allowed,
            Some(false) => MirrorAccess::Refused,
            None => MirrorAccess::Unsupported,
        };
        if access != MirrorAccess::Allowed {
            *h.mirror.lock().unwrap() = MirrorView::default();
        }
        self.set(|l| l.mirror = Some(access));
        h.refresh();
    }
    fn mirrored(&self, allowed: bool, cards: Vec<Card>, machines: Vec<MachineInfo>) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        if !allowed {
            *h.mirror.lock().unwrap() = MirrorView::default();
            self.set(|l| l.mirror = Some(MirrorAccess::Refused));
            h.refresh();
            return;
        }
        // What already waits when the link comes up is not news.
        if !h.mirror_seeded.swap(true, Ordering::SeqCst) {
            h.notifier.lock().unwrap().seed(&cards);
        }
        *h.mirror.lock().unwrap() = MirrorView { cards, machines };
        self.set(|l| l.mirror = Some(MirrorAccess::Allowed));
        h.refresh();
    }
    fn lane(&self) -> Option<Arc<RequestLane>> {
        self.hub.upgrade().map(|h| h.lane.clone())
    }
    /// Saves the config as a main with the list and starts the server; if
    /// it cannot listen, the old config goes back. The client ends by
    /// itself once the answer is out, so it is dropped here, not stopped.
    fn promoted(&self, list: Vec<PairedAssistant>) -> Result<u16, String> {
        let h = self.hub.upgrade().ok_or("the app is gone")?;
        let handed = list.len().saturating_sub(1);
        let by = h.link.lock().unwrap().main_name.clone().unwrap_or_else(|| "the main Maya".into());
        let before = h.settings.lock().unwrap().config.clone();
        h.update_config(|c| promote::as_main(c, list))?;
        if let Err(e) = h.start_server() {
            let _ = h.update_config(|c| *c = before);
            return Err(e);
        }
        h.client.lock().unwrap().take();
        *h.mirror.lock().unwrap() = MirrorView::default();
        *h.link.lock().unwrap() = AssistantLink::default();
        log::line("network", format!("promoted to main by {by}; {handed} assistants handed over"));
        let port = h.handle().map(|s| s.port()).ok_or("The server is stopped.")?;
        h.sink.network(&h.status());
        Ok(port)
    }
    fn moved(&self, host: &str, port: u16, key: &str) -> Result<(), String> {
        let h = self.hub.upgrade().ok_or("the app is gone")?;
        h.update_config(|c| {
            c.network.main_host = host.into();
            c.network.main_port = port;
            c.network.main_key = key.into();
        })
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::net::client::Executor;

    #[test]
    fn a_late_paired_list_change_leaves_an_assistant_config_empty() {
        let late = PairedAssistant { id: "a1".into(), name: "a1".into(), hostname: "h".into(), platform: "macos".into(), key: "k".into(), address: "10.0.0.5".into(), last_seen: None, mirror: false };
        let mut config = Config::default();
        config.network.role = NetworkRole::Assistant;
        assert!(!change_paired(&mut config, |l| *l = vec![late.clone()]));
        assert!(config.network.assistants.is_empty());
        config.network.role = NetworkRole::Main;
        assert!(change_paired(&mut config, |l| *l = vec![late]));
        assert_eq!(config.network.assistants.len(), 1);
    }

    #[test]
    fn a_listening_phone_has_no_sessions_and_runs_no_commands() {
        let e = NoSessions;
        assert_eq!(e.board(), (vec![], vec![]));
        assert_eq!(e.execute(CommandKind::Compact { session: "s".into() }).unwrap_err(), "This phone runs no sessions.");
    }
}
