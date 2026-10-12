//! The phone's main, with the page and the Android service behind traits:
//! the config, the server, the notifier diff and the alert state. The Tauri
//! layer (`lib.rs`, `commands.rs`) is thin on purpose, so this is what the
//! integration test drives.

use crate::alerts::{self, Action, Alerts, Posted, Switches};
use maya_core::config::{self, Config, NetworkRole, PairedAssistant};
use maya_core::log;
use maya_core::model::Card;
use maya_core::net::merge::{self, RemoteBoard};
use maya_core::net::client::{reconnect, ClientHandle, ClientNotify, Executor, NOT_PAIRED, REMOVED};
use maya_core::net::protocol::{CommandKind, CAP_FOLLOW, CAP_LEAD};
use maya_core::net::routing::ROUTE_TIMEOUT;
use maya_core::net::server::{send_command_with, start_with, Notify, ServerHandle};
use maya_core::net::{self, promote, AssistantLink, NetworkStatus};
use maya_core::notify::Notifier;
use maya_core::store::now_ms;
use serde_json::Value;
use std::path::{Path, PathBuf};
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

pub struct Hub {
    /// `<app data dir>/maya`: the config, the log and saved attachments.
    pub maya_dir: PathBuf,
    /// Serializes `refresh`, so a stale snapshot never overtakes a newer one.
    /// Lock order: `refresh` first, alone (its callers hold no other lock);
    /// then `server`, then the server's own mutex, then `settings`;
    /// `notifier`, `posted`, `client` and `link` are taken alone, never
    /// while holding another hub lock.
    refresh: Mutex<()>,
    pub settings: Mutex<Settings>,
    pub server: Mutex<Option<ServerHandle>>,
    /// Why the server is not running (the port is taken…); `None` once it runs.
    pub main_error: Mutex<Option<String>>,
    /// The client that listens to the main while this phone is an assistant.
    pub client: Mutex<Option<ClientHandle>>,
    /// How that client stands with the main.
    pub link: Mutex<AssistantLink>,
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
        Arc::new(Hub { maya_dir, settings: Mutex::new(settings), server: Mutex::new(None), main_error: Mutex::new(None), client: Mutex::new(None), link: Mutex::new(AssistantLink::default()), refresh: Mutex::new(()), notifier: Mutex::new(Notifier::default()), posted: Mutex::new(Posted::default()), alerts, sink, now: Box::new(now) })
    }

    fn handle(&self) -> Option<ServerHandle> {
        self.server.lock().unwrap().clone()
    }

    /// The assistants' last snapshots; none while the server is down.
    pub fn boards(&self) -> Vec<RemoteBoard> {
        self.handle().map(|s| s.boards()).unwrap_or_default()
    }

    /// The board: every connected assistant's cards, no local ones.
    pub fn cards(&self) -> Vec<Card> {
        merge::merged(vec![], &self.boards(), (self.now)())
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
            return NetworkStatus { role: NetworkRole::Assistant, assistant: self.link.lock().unwrap().clone(), ..Default::default() };
        }
        match self.handle() {
            Some(s) => s.status(),
            None => {
                let paired = self.settings.lock().unwrap().config.network.assistants.clone();
                NetworkStatus { role: NetworkRole::Off, code: None, assistants: net::paired_offline(&paired), main_error: self.main_error.lock().unwrap().clone(), ..Default::default() }
            }
        }
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

    fn save_assistants(&self, f: impl FnOnce(&mut Vec<PairedAssistant>)) -> Result<(), String> {
        let mut skipped = false;
        let saved = self.update_config(|c| skipped = !change_paired(c, f)).map(|_| ());
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
        let mut started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port);
        for _ in 0..5 {
            if started.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
            started = start_with(notify.clone(), Arc::new(Mutex::new(view.clone())), port);
        }
        match started {
            Ok(handle) => {
                let status = handle.status();
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

    /// Listens to the main in the config: an assistant with no sessions, so
    /// that main can make this phone the main again. The keep-awake service runs meanwhile.
    pub fn start_client(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let handle = ClientHandle::spawn(move |stop| {
            let notify: Arc<dyn ClientNotify> = Arc::new(HubClientNotify { hub: weak.clone(), stop: stop.clone() });
            let config_of = move || weak.upgrade().map(|h| h.settings.lock().unwrap().config.network.clone()).unwrap_or_default();
            reconnect(&config_of, Arc::new(NoSessions), notify, &stop);
        });
        *self.link.lock().unwrap() = AssistantLink { retrying: true, ..Default::default() };
        if let Some(old) = self.client.lock().unwrap().replace(handle) {
            old.stop();
        }
        self.sink.service_start("Reconnecting…");
        self.sink.network(&self.status());
    }

    /// Stops listening: the role goes Off and the link is forgotten.
    pub fn stop_listening(&self) -> Result<NetworkStatus, String> {
        let client = self.client.lock().unwrap().take();
        if let Some(c) = client {
            c.stop();
            self.sink.service_stop();
        }
        *self.link.lock().unwrap() = AssistantLink::default();
        self.update_config(|c| {
            c.network.role = NetworkRole::Off;
            c.network.main_host.clear();
            c.network.main_port = 0;
            c.network.assistant_id.clear();
            c.network.token.clear();
        })?;
        let status = self.status();
        self.sink.network(&status);
        Ok(status)
    }

    /// Makes a connected assistant the main, then listens to it.
    pub fn promote(self: &Arc<Self>, id: &str) -> Result<NetworkStatus, String> {
        let server = self.handle().ok_or("Start the server first.")?;
        let out = server.promote(id)?;
        self.update_config(|c| promote::as_assistant_of(c, &out.host, out.port, &out.me)).map_err(|e| format!("Could not save this Maya as an assistant: {}.", e.trim_end_matches('.')))?;
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
                self.save_assistants(|list| list.retain(|a| a.id != id))?;
                log::line("network", format!("{id}: removed"));
            }
        }
        Ok(self.status())
    }

    /// Sends a session command to the machine that lists the session. There
    /// are no local sessions: a session nobody lists is no longer running.
    pub fn send(&self, session_id: &str, kind: impl FnOnce(String) -> CommandKind) -> Result<Option<Value>, String> {
        let machine = merge::route(&[], &self.boards(), session_id, (self.now)()).ok_or("Session is no longer running.")?;
        self.send_to(&machine, kind(session_id.to_string()))
    }

    /// Sends a command to a machine by its label (start, resume, listings).
    pub fn send_to(&self, machine: &str, kind: CommandKind) -> Result<Option<Value>, String> {
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
            h.sink.network(&status);
        });
    }

    fn paired(&self, assistant: &PairedAssistant) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(|list| match list.iter_mut().find(|a| a.id == assistant.id) {
            Some(a) => *a = assistant.clone(),
            None => list.push(assistant.clone()),
        })
    }

    fn paired_list_changed(&self, assistants: &[PairedAssistant]) -> Result<(), String> {
        let Some(h) = self.0.upgrade() else { return Err("the app is gone".into()) };
        h.save_assistants(|list| *list = assistants.to_vec())
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
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl HubClientNotify {
    fn set(&self, f: impl FnOnce(&mut AssistantLink)) {
        if self.stop.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(h) = self.hub.upgrade() else { return };
        let line = {
            let mut l = h.link.lock().unwrap();
            f(&mut l);
            match (&l.main_name, l.connected) {
                (Some(m), true) => format!("Listening to {m}"),
                _ => "Reconnecting…".to_string(),
            }
        };
        h.alerts.service_line(&line);
        h.sink.network(&h.status());
    }
}

impl ClientNotify for HubClientNotify {
    fn paired(&self, _: &str, _: &str) {}
    fn connected(&self, main_name: &str) {
        let name = main_name.to_string();
        self.set(|l| *l = AssistantLink { connected: true, main_name: Some(name), error: None, retrying: false });
    }
    fn disconnected(&self, error: &str) {
        let retrying = error != NOT_PAIRED && error != REMOVED;
        self.set(|l| {
            l.connected = false;
            l.error = Some(error.to_string());
            l.retrying = retrying;
        });
    }
    fn removed(&self) {
        self.set(|l| {
            l.connected = false;
            l.error = Some(REMOVED.into());
            l.retrying = false;
        });
    }
    fn caps(&self) -> Vec<String> {
        vec![CAP_FOLLOW.into(), CAP_LEAD.into()]
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
        *h.link.lock().unwrap() = AssistantLink::default();
        log::line("network", format!("promoted to main by {by}; {handed} assistants handed over"));
        let port = h.handle().map(|s| s.port()).ok_or("The server is stopped.")?;
        h.sink.network(&h.status());
        Ok(port)
    }
    fn moved(&self, host: &str, port: u16) -> Result<(), String> {
        let h = self.hub.upgrade().ok_or("the app is gone")?;
        h.update_config(|c| {
            c.network.main_host = host.into();
            c.network.main_port = port;
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
        let late = PairedAssistant { id: "a1".into(), name: "a1".into(), hostname: "h".into(), platform: "macos".into(), token: "t".into(), address: "10.0.0.5".into(), last_seen: None };
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
